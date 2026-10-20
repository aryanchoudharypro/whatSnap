use std::{
	sync::{
		Arc,
		atomic::{AtomicBool, AtomicU64, Ordering},
	},
	time::{SystemTime, UNIX_EPOCH},
};

use rodio::{DeviceSinkBuilder, MixerDeviceSink, Player};
use rodio_wsola::WsolaSourceExt;

use crate::{AppState, audio_devices, types::*};
static LAST_SOUND_TIME: AtomicU64 = AtomicU64::new(0);

// cpal/rodio don't otherwise tell a caller a stream died (device unplugged,
// invalidated across sleep, etc) - by default the error just goes to a log
// line and playback silently stops producing sound. Wiring this callback and
// flipping `on_error` is what actually lets `ensure_audio_stream` learn a
// stream needs reopening; without it, only a name-mismatch could trigger a
// reopen, which doesn't cover "same device name, session invalidated anyway".
pub(crate) fn open_output_stream(
	preferred_name: Option<&str>,
	on_error: Arc<AtomicBool>,
) -> Result<MixerDeviceSink, rodio::DeviceSinkError> {
	match audio_devices::resolve_output_device(preferred_name) {
		Some(device) => DeviceSinkBuilder::from_device(device)?
			.with_error_callback(move |err| {
				log_line(&format!("output stream error, marking dead for reopen: {:?}", err));
				on_error.store(true, Ordering::Relaxed);
			})
			.open_stream(),
		None => DeviceSinkBuilder::open_default_sink(),
	}
}

// A cached stream older than this is treated as untrustworthy and reopened
// even if the resolved device name hasn't changed and no stream error fired -
// see the doc comment on `AppState::audio_stream_opened_at`. Belt-and-braces
// alongside the error callback above, short enough that a sleep/hibernate
// cycle is always past it, long enough not to add a reopen to normal
// back-to-back playback.
const AUDIO_STREAM_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(20);

fn log_line(msg: &str) {
	use std::io::Write;
	let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
	let line = format!("[{}.{:03}] {}\n", now.as_secs(), now.subsec_millis(), msg);
	if let Ok(mut f) =
		std::fs::OpenOptions::new().create(true).append(true).open(crate::paths::log_dir().join("audio.log"))
	{
		let _ = f.write_all(line.as_bytes());
	}
}

pub(crate) fn ensure_audio_stream(s: &mut AppState) {
	let current_name = audio_devices::resolve_output_device_name(s.settings.output_device_name.as_deref());
	let stream_errored = s.audio_stream_failed.swap(false, Ordering::Relaxed);
	let fresh_enough = s.audio_stream_opened_at.map(|t| t.elapsed() < AUDIO_STREAM_MAX_AGE).unwrap_or(false);
	if s.audio_stream.is_some() && s.audio_stream_device_name == current_name && fresh_enough && !stream_errored {
		return;
	}
	if stream_errored {
		log_line("reopening output stream after a stream error");
	}
	match open_output_stream(s.settings.output_device_name.as_deref(), s.audio_stream_failed.clone()) {
		Ok(stream) => {
			s.audio_stream = Some(stream);
			s.audio_stream_device_name = current_name;
			s.audio_stream_opened_at = Some(std::time::Instant::now());
		}
		Err(e) => {
			log_line(&format!("Failed to (re)open output stream (device {:?}): {:?}", current_name, e));
			eprintln!("[AUDIO] Failed to (re)open output stream: {:?}", e);
			s.audio_stream = None;
			s.audio_stream_device_name = None;
			s.audio_stream_opened_at = None;
		}
	}
}

pub(crate) fn play_notification_sound(
	sound: NotificationSound,
	sounds_enabled: bool,
	output_device_name: Option<String>,
) {
	if !sounds_enabled {
		return;
	}
	let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
	let last = LAST_SOUND_TIME.swap(now, Ordering::Relaxed);
	if now - last < 500 {
		return;
	}
	std::thread::spawn(move || {
		// COM is per-thread; without initializing it here, resolve_output_device's
		// CoCreateInstance(MMDeviceEnumerator) silently fails on this freshly
		// spawned thread (unlike the UI thread, which wx already initializes COM
		// on) and every notification sound falls back to cpal's plain Console-role
		// default, which doesn't follow a headset that only repoints the
		// Communications role.
		use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
		struct ComGuard(bool);
		impl Drop for ComGuard {
			fn drop(&mut self) {
				if self.0 {
					unsafe { CoUninitialize() };
				}
			}
		}
		let _com_guard = ComGuard(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok());
		// Honor the user's configured output device (and, for "System
		// Default", the Communications-role default - see
		// `audio_devices::resolve_output_device`) instead of always opening
		// whatever cpal calls the default sink, which used to send
		// notification sounds to a different device than calls/voice
		// messages whenever the two audio roles disagreed (e.g. headphones
		// plugged in).
		let stream_result = match audio_devices::resolve_output_device(output_device_name.as_deref()) {
			Some(device) => DeviceSinkBuilder::from_device(device).and_then(|b| b.open_stream()),
			None => DeviceSinkBuilder::open_default_sink(),
		};
		let Ok(stream) = stream_result else {
			return;
		};
		let sink = Player::connect_new(stream.mixer());
		if let Some(decoded) = decode_opus(sound.ogg()) {
			sink.append(decoded);
			sink.play();
			// Block this worker thread until playback finishes, so `stream` and
			// `sink` (and the device they hold open) stay alive for the duration.
			sink.sleep_until_end();
		}
	});
}

pub(crate) fn play_call_fail_sound(state: &mut AppState) {
	ensure_audio_stream(state);
	if let Some(ref stream_handle) = state.audio_stream {
		let sink = Player::connect_new(stream_handle.mixer());
		match decode_opus(include_bytes!("../sounds/whatsapp_windows_hangup_01_705kbps.ogg")) {
			Some(decoded) => {
				sink.append(decoded);
				sink.play();
				state.call_ringing_sink = Some(sink);
			}
			None => eprintln!("[AUDIO] Error decoding fail sound"),
		}
	} else {
		eprintln!("[AUDIO] No audio stream available");
	}
}

pub(crate) fn start_call_ringing_sound(state: &mut AppState, sound_bytes: &'static [u8]) {
	ensure_audio_stream(state);
	if let Some(ref stream_handle) = state.audio_stream {
		let sink = Player::connect_new(stream_handle.mixer());
		match decode_opus(sound_bytes) {
			Some(decoded) => {
				use rodio::Source;
				// A SamplesBuffer knows its length up front, which
				// repeat_infinite() needs to loop it.
				sink.append(decoded.repeat_infinite());
				sink.play();
				state.call_ringing_sink = Some(sink);
			}
			None => eprintln!("[AUDIO] Error decoding ringing sound"),
		}
	} else {
		eprintln!("[AUDIO] No audio stream available");
	}
}

/// Starts the ringing heard while calling someone: WhatsApp's own ringback
/// for the country of `self_phone` (see ringback.rs), looped until the call is
/// answered or ends. Stopped with `stop_call_ringing_sound` like the others.
pub(crate) fn start_call_ringback(state: &mut AppState, self_phone: &str) {
	use rodio::Source;
	ensure_audio_stream(state);
	let Some(ref stream_handle) = state.audio_stream else {
		eprintln!("[AUDIO] No audio stream available");
		return;
	};
	let sink = Player::connect_new(stream_handle.mixer());
	sink.append(crate::ringback::ringback_cycle(self_phone).repeat_infinite());
	sink.play();
	state.call_ringing_sink = Some(sink);
}

/// Plays WhatsApp's busy tone once (four beeps), for a call that ended
/// because the person was on another call. Stopped with
/// `stop_call_ringing_sound` like the others.
pub(crate) fn play_busy_tone(state: &mut AppState) {
	ensure_audio_stream(state);
	let Some(ref stream_handle) = state.audio_stream else {
		eprintln!("[AUDIO] No audio stream available");
		return;
	};
	let sink = Player::connect_new(stream_handle.mixer());
	sink.append(crate::ringback::busy_tone());
	sink.play();
	state.call_ringing_sink = Some(sink);
}

pub(crate) fn stop_call_ringing_sound(state: &mut AppState) {
	if let Some(sink) = state.call_ringing_sink.take() {
		sink.pause();
	}
	// The call keys go with the ringing: a call that stopped ringing has
	// nothing left for them to answer or reject.
	state.call_hotkeys = None;
}

/// Length of an audio file in whole seconds (0 if it can't be read).
pub(crate) fn audio_file_seconds(path: &str) -> u64 {
	crate::audio_file::duration(path).map_or(0, |d| d.as_secs_f64().round() as u64)
}

pub(crate) fn concat_wav_segments(segments: &[String], out_path: &str) -> crate::error::Result<()> {
	use crate::wav::{WavReader, WavWriter};
	let mut spec = None;
	let mut writer: Option<WavWriter> = None;
	for seg in segments {
		let reader = WavReader::open(seg)?;
		let seg_spec = reader.spec();
		if writer.is_none() {
			spec = Some(seg_spec);
			writer = Some(WavWriter::create(out_path, seg_spec)?);
		} else if spec != Some(seg_spec) {
			eprintln!("[LOG UI] Voice recording segment {} has a mismatched format, skipping it", seg);
			continue;
		}
		if let Some(ref mut w) = writer {
			for sample in reader {
				w.write_sample(sample?)?;
			}
		}
	}
	match writer {
		Some(w) => {
			w.finalize()?;
			Ok(())
		}
		None => Err(crate::error::err!("no valid voice recording segments to concatenate")),
	}
}

pub(crate) fn do_seek(stream_handle: &MixerDeviceSink, path: &str, target_secs: f32, speed: f32) -> Option<Player> {
	let source = crate::audio_file::open(path, std::time::Duration::from_secs_f32(target_secs.max(0.0)))?;
	let sink = Player::connect_new(stream_handle.mixer());
	sink.append(source.wsola(speed));
	sink.play();
	Some(sink)
}

/// Decodes an Ogg Opus stream (a voice note, or one of the built-in sounds).
pub(crate) fn decode_opus(data: &[u8]) -> Option<rodio::buffer::SamplesBuffer> {
	use opus::{Channels, Decoder};

	fn feed_packet(
		packet: &[u8],
		decoder: &mut Option<Decoder>,
		channels: &mut usize,
		pre_skip: &mut usize,
		skipped: &mut usize,
		samples: &mut Vec<f32>,
		pcm: &mut Vec<i16>,
	) {
		if packet.len() >= 8 && &packet[0..8] == b"OpusHead" {
			// Header packet, not audio - (re)establish the decoder from it.
			// Real chaining (a new pause/resume segment spliced in as its
			// own logical stream) shows up as more of these later in the
			// file; picking up its channel count/pre-skip here handles that
			// case too, not just the file's very first stream.
			*channels = if packet.len() >= 10 { (packet[9] as usize).max(1) } else { 1 };
			*pre_skip = if packet.len() >= 12 { u16::from_le_bytes([packet[10], packet[11]]) as usize } else { 0 };
			let opus_channels = if *channels >= 2 { Channels::Stereo } else { Channels::Mono };
			*decoder = Decoder::new(48000, opus_channels).ok();
			*skipped = 0;
			return;
		}
		if packet.len() >= 8 && &packet[0..8] == b"OpusTags" {
			return;
		}
		let Some(dec) = decoder.as_mut() else { return };
		let ch = (*channels).max(1);
		// Max frame size for opus is 120ms at 48kHz. One buffer serves every
		// packet of the file.
		pcm.resize(5760 * ch, 0);
		if let Ok(n) = dec.decode(packet, pcm, false) {
			let mut decoded = &pcm[..n * ch];
			if *skipped < *pre_skip {
				let to_skip = (*pre_skip - *skipped).min(n);
				decoded = &decoded[to_skip * ch..];
				*skipped += to_skip;
			}
			samples.extend(decoded.iter().map(|&s| s as f32 / 32768.0));
		}
	}

	let len = data.len();
	let mut pos: usize = 0;

	let mut decoder: Option<Decoder> = None;
	let mut channels: usize = 1;
	let mut pre_skip: usize = 0;
	let mut skipped: usize = 0;
	let mut samples: Vec<f32> = Vec::new();
	let mut pending: Vec<u8> = Vec::new(); // partial packet carried across a page boundary
	let mut pcm: Vec<i16> = Vec::new();

	while pos + 27 <= len {
		if &data[pos..pos + 4] != b"OggS" {
			// Lost sync (or not actually at a page boundary yet) - scan
			// forward for the next capture pattern rather than giving up.
			match data[pos + 1..].windows(4).position(|w| w == b"OggS") {
				Some(off) => {
					pos += 1 + off;
					continue;
				}
				None => break,
			}
		}
		let header = &data[pos..pos + 27];
		let flags = header[5];
		let segs = header[26] as usize;
		if pos + 27 + segs > len {
			break;
		}
		let lacing = &data[pos + 27..pos + 27 + segs];
		let body_len: usize = lacing.iter().map(|&b| b as usize).sum();
		let body_start = pos + 27 + segs;
		if body_start + body_len > len {
			break;
		}
		let body = &data[body_start..body_start + body_len];

		let starts_with_continued = flags & 0x01 != 0;
		if !starts_with_continued {
			pending.clear();
		}

		let mut offset = 0usize;
		let mut seg_run_start = 0usize;
		let mut first_packet_of_page = true;
		for (i, &seg_len) in lacing.iter().enumerate() {
			offset += seg_len as usize;
			let is_packet_end = seg_len < 255;
			let is_last_segment = i + 1 == lacing.len();
			if is_packet_end {
				let packet_bytes = &body[seg_run_start..offset];
				seg_run_start = offset;
				let is_continuation = first_packet_of_page && starts_with_continued;
				first_packet_of_page = false;
				if is_continuation {
					if pending.is_empty() {
						// No matching partial packet to complete this with
						// (e.g. we just resynced) - can't recover it, drop
						// it and move on to the next one.
						continue;
					}
					pending.extend_from_slice(packet_bytes);
					feed_packet(
						&pending,
						&mut decoder,
						&mut channels,
						&mut pre_skip,
						&mut skipped,
						&mut samples,
						&mut pcm,
					);
					pending.clear();
				} else {
					feed_packet(
						packet_bytes,
						&mut decoder,
						&mut channels,
						&mut pre_skip,
						&mut skipped,
						&mut samples,
						&mut pcm,
					);
				}
			} else if is_last_segment {
				// Page ends mid-packet - stash it for the next page to
				// (hopefully) complete.
				pending.extend_from_slice(&body[seg_run_start..offset]);
			}
		}

		pos = body_start + body_len;
	}

	if samples.is_empty() {
		return None;
	}

	Some(rodio::buffer::SamplesBuffer::new(
		std::num::NonZeroU16::new(channels as u16).unwrap_or(std::num::NonZeroU16::new(1).unwrap()),
		std::num::NonZeroU32::new(48000).unwrap(),
		samples,
	))
}

/// The 64 loudness bars WhatsApp draws on a voice message, 0-100 each - the
/// `waveform` field of the AudioMessage.
///
/// Built the way WhatsApp Web builds a recorded voice note's
/// (WAPttComposerWaveformRecorder): level in dB mapped from the -60..-10 dB
/// range onto 0..100, the loudest moment in each slice of the recording, the
/// slices then evened out to 64. `None` if the file can't be read or holds no
/// audio.
pub(crate) fn compute_voice_waveform(wav_path: &str) -> Option<Vec<u8>> {
	const BARS: usize = 64;
	const MIN_DB: f32 = -60.0;
	const MAX_DB: f32 = -10.0;

	let reader = crate::wav::WavReader::open(wav_path).ok()?;
	let spec = reader.spec();
	let channels = spec.channels.max(1) as usize;
	// 10 ms windows, first channel only (WhatsApp measures a mono stream).
	let window = (spec.sample_rate as usize / 100).max(1);
	let mut levels = Vec::new();
	let (mut sum, mut count) = (0.0f64, 0usize);
	for (i, sample) in reader.enumerate() {
		if i % channels != 0 {
			continue;
		}
		let s = sample.ok()? as f64 / 32768.0;
		sum += s * s;
		count += 1;
		if count == window {
			levels.push(10.0 * (sum / count as f64 + 1e-12).log10() as f32);
			(sum, count) = (0.0, 0);
		}
	}
	if count > 0 {
		levels.push(10.0 * (sum / count as f64 + 1e-12).log10() as f32);
	}
	if levels.is_empty() {
		return None;
	}
	let bars = (0..BARS)
		.map(|b| {
			let start = b * levels.len() / BARS;
			let end = ((b + 1) * levels.len() / BARS).max(start + 1).min(levels.len());
			let peak = levels[start.min(levels.len() - 1)..end].iter().cloned().fold(f32::MIN, f32::max);
			(((peak - MIN_DB) / (MAX_DB - MIN_DB)).clamp(0.0, 1.0) * 100.0) as u8
		})
		.collect();
	Some(bars)
}

/// Encodes a recorded voice note laid out the way libopusenc and WhatsApp's
/// own voice notes are, since the phone's player is far stricter than the Ogg
/// spec: a page per second of audio, closed early rather than ever splitting
/// a packet across two pages, granule positions that count the encoder's
/// pre-skip, and a last page that always ends the stream. Four second pages
/// with packets spanning them made the phone call some notes broken.
pub(crate) fn convert_wav_to_opus_ogg(wav_path: &str, ogg_path: &str) -> crate::error::Result<()> {
	use std::{
		fs::File,
		io::{BufWriter, Write},
	};

	use opus::{Application, Channels, Encoder};

	use crate::{
		ogg::{OggWriter, PacketEnd},
		wav::WavReader,
	};

	const FRAME: usize = 960; // 20 ms at 48 kHz
	const PACKETS_PER_PAGE: usize = 50; // one second

	let reader = WavReader::open(wav_path)?;
	let spec = reader.spec();
	if spec.sample_rate != 48000 {
		return Err(crate::error::err!("Sample rate must be 48000"));
	}
	let is_stereo = spec.channels == 2;
	let ch = if is_stereo { 2 } else { 1 };

	let mut encoder =
		Encoder::new(48000, if is_stereo { Channels::Stereo } else { Channels::Mono }, Application::Voip)?;
	let pre_skip = encoder.get_lookahead()?.max(0) as usize;

	let mut writer = OggWriter::new(BufWriter::new(File::create(ogg_path)?), 12345);

	let mut head = Vec::with_capacity(19);
	head.extend_from_slice(b"OpusHead");
	head.push(1); // Version
	head.push(ch as u8); // Channels
	head.extend_from_slice(&(pre_skip as u16).to_le_bytes());
	head.extend_from_slice(&48000_u32.to_le_bytes()); // Input sample rate
	head.extend_from_slice(&0_u16.to_le_bytes()); // Gain
	head.push(0); // Channel mapping family
	writer.write_packet(&head, PacketEnd::EndPage, 0)?;

	let mut tags = Vec::with_capacity(24);
	tags.extend_from_slice(b"OpusTags");
	tags.extend_from_slice(&8_u32.to_le_bytes()); // Vendor length
	tags.extend_from_slice(b"whaNTV  ");
	tags.extend_from_slice(&0_u32.to_le_bytes()); // User comments list length
	writer.write_packet(&tags, PacketEnd::EndPage, 0)?;

	// Lacing values a packet takes up in a page.
	let segments = |len: usize| len / 255 + 1;
	let mut samples = reader;
	let mut frame = [0i16; FRAME * 2];
	let mut out = [0u8; 1275]; // the most one Opus frame can take
	// Each packet is held back until the next one exists, so it's known
	// whether the page has room for that one, and the last packet can go out
	// as the end of the stream.
	let mut held: Option<(Vec<u8>, u64)> = None;
	let (mut page_packets, mut page_segments) = (0, 0);
	let mut input_done = false;
	// Silence fed after the recording so its tail clears the encoder's
	// lookahead instead of being cut off.
	let mut pad = pre_skip;
	let mut total = 0u64;
	let mut encoded = 0u64;
	loop {
		let mut filled = 0;
		while filled < FRAME * ch && !input_done {
			match samples.next() {
				Some(s) => {
					frame[filled] = s?;
					filled += 1;
				}
				None => input_done = true,
			}
		}
		// A frame ending half way through a stereo pair drops that half.
		filled -= filled % ch;
		total += (filled / ch) as u64;
		let pad_now = pad.min(FRAME - filled / ch);
		pad -= pad_now;
		filled += pad_now * ch;
		if filled == 0 {
			break;
		}
		frame[filled..].fill(0);
		let len = encoder.encode(&frame[..FRAME * ch], &mut out)?;
		encoded += FRAME as u64;
		if let Some((packet, granule)) = held.take() {
			page_packets += 1;
			page_segments += segments(packet.len());
			let end = if page_packets == PACKETS_PER_PAGE || page_segments + segments(len) > 255 {
				(page_packets, page_segments) = (0, 0);
				PacketEnd::EndPage
			} else {
				PacketEnd::Normal
			};
			writer.write_packet(&packet, end, granule)?;
		}
		held = Some((out[..len].to_vec(), encoded));
		if input_done && pad == 0 {
			break;
		}
	}
	if total == 0 {
		return Err(crate::error::err!("no audio in the voice recording"));
	}
	if let Some((packet, _)) = held {
		// The last page's granule marks where the audio really ends.
		writer.write_packet(&packet, PacketEnd::EndStream, pre_skip as u64 + total)?;
	}
	writer.into_inner().flush()?;
	Ok(())
}

#[cfg(test)]
mod waveform_tests {
	use super::compute_voice_waveform;

	#[test]
	fn waveform_is_64_bars_following_loudness() {
		let path = std::env::temp_dir().join(format!("whantv_waveform_test_{}.wav", std::process::id()));
		let spec = crate::wav::WavSpec { channels: 2, sample_rate: 48_000 };
		let mut w = crate::wav::WavWriter::create(path.to_str().unwrap(), spec).unwrap();
		// 1 s silence, then 1 s of a loud tone.
		for i in 0..96_000 {
			let s = if i < 48_000 { 0.0 } else { 0.5 * (i as f32 * 0.05).sin() };
			let v = (s * 32767.0) as i16;
			w.write_sample(v).unwrap();
			w.write_sample(v).unwrap();
		}
		w.finalize().unwrap();
		let bars = compute_voice_waveform(path.to_str().unwrap()).unwrap();
		let _ = std::fs::remove_file(&path);
		assert_eq!(bars.len(), 64);
		assert!(bars[..31].iter().all(|&b| b == 0), "{bars:?}");
		assert!(bars[33..].iter().all(|&b| b == 100), "{bars:?}");
	}
}

#[cfg(test)]
mod sound_tests {
	use crate::NotificationSound;

	#[test]
	fn every_built_in_sound_decodes() {
		for sound in [
			NotificationSound::MessageCurrent,
			NotificationSound::MessageBackground,
			NotificationSound::MessageSent,
			NotificationSound::VoiceMessageSent,
			NotificationSound::VoiceSequenceNext,
			NotificationSound::VoiceSequenceEnd,
		] {
			let decoded = super::decode_opus(sound.ogg());
			assert!(decoded.is_some_and(|mut d| d.next().is_some()), "{:?} doesn't decode", sound as u8);
		}
	}
}

#[cfg(test)]
mod voice_note_tests {
	use super::{convert_wav_to_opus_ogg, decode_opus};

	#[test]
	fn voice_note_is_laid_out_like_whatsapps() {
		// One ending on a partial frame, one on a whole frame.
		for total in [350_000u64, 96_000] {
			let dir = std::env::temp_dir();
			let wav = dir.join(format!("whantv_voice_test_{}_{total}.wav", std::process::id()));
			let ogg = wav.with_extension("ogg");
			let spec = crate::wav::WavSpec { channels: 2, sample_rate: 48_000 };
			let mut w = crate::wav::WavWriter::create(wav.to_str().unwrap(), spec).unwrap();
			for i in 0..total {
				let v = (0.4 * (i as f32 * 0.03).sin() * 32767.0) as i16;
				w.write_sample(v).unwrap();
				w.write_sample(v / 2).unwrap();
			}
			w.finalize().unwrap();
			convert_wav_to_opus_ogg(wav.to_str().unwrap(), ogg.to_str().unwrap()).unwrap();
			let data = std::fs::read(&ogg).unwrap();
			let decoded = decode_opus(&data).unwrap().count() as u64;
			let _ = std::fs::remove_file(&wav);
			let _ = std::fs::remove_file(&ogg);

			assert_eq!(&data[28..36], b"OpusHead");
			assert_eq!(data[37], 2, "stereo, as recorded");
			let pre_skip = u16::from_le_bytes([data[38], data[39]]) as u64;

			let (mut pos, mut pages, mut last) = (0, 0, (0u8, 0i64));
			while pos < data.len() {
				assert_eq!(&data[pos..pos + 4], b"OggS");
				let flags = data[pos + 5];
				let granule = i64::from_le_bytes(data[pos + 6..pos + 14].try_into().unwrap());
				let segs = data[pos + 26] as usize;
				let lacing = &data[pos + 27..pos + 27 + segs];
				assert_eq!(flags & 1, 0, "page {pages} continues a packet");
				assert!(lacing.last().is_some_and(|&l| l < 255), "page {pages} ends mid-packet");
				assert!(lacing.iter().filter(|&&l| l < 255).count() <= 50, "page {pages} is over a second");
				last = (flags, granule);
				pos += 27 + segs + lacing.iter().map(|&l| l as usize).sum::<usize>();
				pages += 1;
			}
			assert_eq!(pos, data.len());
			assert_eq!(last.0 & 4, 4, "last page ends the stream");
			assert_eq!(last.1 as u64, pre_skip + total);
			// Every sample comes back once the decoder's pre-skip is dropped.
			assert!(decoded >= total * 2, "{decoded} < {}", total * 2);
		}
	}
}
