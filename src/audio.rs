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
		let sink = Player::connect_new(&stream.mixer());
		let cursor = std::io::Cursor::new(sound.ogg());
		if let Ok(decoder) = rodio::Decoder::new(cursor) {
			sink.append(decoder);
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
		let sink = Player::connect_new(&stream_handle.mixer());
		let cursor = std::io::Cursor::new(include_bytes!("../sounds/whatsapp_windows_hangup_01_705kbps.ogg"));
		match rodio::Decoder::new(cursor) {
			Ok(decoder) => {
				sink.append(decoder);
				sink.play();
				state.call_ringing_sink = Some(sink);
			}
			Err(e) => eprintln!("[AUDIO] Error decoding fail sound: {:?}", e),
		}
	} else {
		eprintln!("[AUDIO] No audio stream available");
	}
}

pub(crate) fn start_call_ringing_sound(state: &mut AppState, sound_bytes: &'static [u8]) {
	ensure_audio_stream(state);
	if let Some(ref stream_handle) = state.audio_stream {
		let sink = Player::connect_new(&stream_handle.mixer());
		let cursor = std::io::Cursor::new(sound_bytes);
		match rodio::Decoder::new(cursor) {
			Ok(decoder) => {
				use rodio::Source;
				// Fully decode into a SamplesBuffer before looping: rodio's
				// symphonia-backed Decoder reports current_span_len() == 0 until
				// its internal buffer is first populated by a next() call, and
				// repeat_infinite()'s underlying buffered() checks that length
				// immediately at construction time - seeing 0, it treats the
				// decoder as already exhausted and Repeat plays silence forever.
				// SamplesBuffer reports its real length from the start, so
				// wrapping that instead avoids the false-exhausted state.
				let channels = decoder.channels();
				let sample_rate = decoder.sample_rate();
				let samples: Vec<f32> = decoder.collect();
				let buffer = rodio::buffer::SamplesBuffer::new(channels, sample_rate, samples);
				sink.append(buffer.repeat_infinite());
				sink.play();
				state.call_ringing_sink = Some(sink);
			}
			Err(e) => eprintln!("[AUDIO] Error decoding ringing sound: {:?}", e),
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
	let sink = Player::connect_new(&stream_handle.mixer());
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
	let sink = Player::connect_new(&stream_handle.mixer());
	sink.append(crate::ringback::busy_tone());
	sink.play();
	state.call_ringing_sink = Some(sink);
}

pub(crate) fn stop_call_ringing_sound(state: &mut AppState) {
	if let Some(sink) = state.call_ringing_sink.take() {
		sink.pause();
	}
}

/// Length of an audio file in whole seconds (0 if it can't be read): from the
/// container's own header when it has one, else by decoding it through.
pub(crate) fn audio_file_seconds(path: &str) -> u64 {
	use rodio::Source;
	let Ok(file) = std::fs::File::open(path) else {
		return 0;
	};
	if let Ok(decoder) = rodio::Decoder::new(std::io::BufReader::new(file)) {
		if let Some(d) = decoder.total_duration() {
			return d.as_secs_f64().round() as u64;
		}
		let (channels, rate) = (decoder.channels().get() as u64, decoder.sample_rate().get() as u64);
		let samples = decoder.count() as u64;
		return (samples / channels.max(1) + rate / 2) / rate.max(1);
	}
	decode_opus_file(path).and_then(|b| b.total_duration()).map(|d| d.as_secs_f64().round() as u64).unwrap_or(0)
}

pub(crate) fn concat_wav_segments(segments: &[String], out_path: &str) -> anyhow::Result<()> {
	use hound::{WavReader, WavWriter};
	let mut spec = None;
	let mut writer: Option<WavWriter<std::io::BufWriter<std::fs::File>>> = None;
	for seg in segments {
		let mut reader = WavReader::open(seg)?;
		let seg_spec = reader.spec();
		if writer.is_none() {
			spec = Some(seg_spec);
			writer = Some(WavWriter::create(out_path, seg_spec)?);
		} else if spec != Some(seg_spec) {
			eprintln!("[LOG UI] Voice recording segment {} has a mismatched format, skipping it", seg);
			continue;
		}
		if let Some(ref mut w) = writer {
			for sample in reader.samples::<i16>() {
				w.write_sample(sample?)?;
			}
		}
	}
	match writer {
		Some(w) => {
			w.finalize()?;
			Ok(())
		}
		None => Err(anyhow::anyhow!("no valid voice recording segments to concatenate")),
	}
}

pub(crate) fn do_seek(stream_handle: &MixerDeviceSink, path: &str, target_secs: f32, speed: f32) -> Option<Player> {
	use rodio::Source;
	let sink = Player::connect_new(&stream_handle.mixer());
	if let Ok(file) = std::fs::File::open(path) {
		if let Ok(decoder) = rodio::Decoder::new(std::io::BufReader::new(file)) {
			let source = decoder.skip_duration(std::time::Duration::from_secs_f32(target_secs.max(0.0))).wsola(speed);
			sink.append(source);
			sink.play();
			return Some(sink);
		} else if let Some(opus_buf) = decode_opus_file(path) {
			let source = opus_buf.skip_duration(std::time::Duration::from_secs_f32(target_secs.max(0.0))).wsola(speed);
			sink.append(source);
			sink.play();
			return Some(sink);
		}
	}
	None
}

pub(crate) fn decode_opus_file(path: &str) -> Option<rodio::buffer::SamplesBuffer> {
	use opus::{Channels, Decoder};

	fn feed_packet(
		packet: &[u8],
		decoder: &mut Option<Decoder>,
		channels: &mut usize,
		pre_skip: &mut usize,
		skipped: &mut usize,
		samples: &mut Vec<f32>,
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
		let mut pcm = vec![0i16; 5760 * (*channels).max(1)]; // Max frame size for opus is 120ms at 48kHz
		if let Ok(n) = dec.decode(packet, &mut pcm, false) {
			pcm.truncate(n * (*channels).max(1));
			let mut frame: Vec<f32> = pcm.into_iter().map(|s| (s as f32) / 32768.0).collect();
			if *skipped < *pre_skip {
				let frames_here = frame.len() / (*channels).max(1);
				let to_skip = (*pre_skip - *skipped).min(frames_here);
				frame.drain(0..to_skip * (*channels).max(1));
				*skipped += to_skip;
			}
			samples.extend(frame);
		}
	}

	let data = std::fs::read(path).ok()?;
	let len = data.len();
	let mut pos: usize = 0;

	let mut decoder: Option<Decoder> = None;
	let mut channels: usize = 1;
	let mut pre_skip: usize = 0;
	let mut skipped: usize = 0;
	let mut samples: Vec<f32> = Vec::new();
	let mut pending: Vec<u8> = Vec::new(); // partial packet carried across a page boundary

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
					let mut full = std::mem::take(&mut pending);
					full.extend_from_slice(packet_bytes);
					feed_packet(&full, &mut decoder, &mut channels, &mut pre_skip, &mut skipped, &mut samples);
				} else {
					feed_packet(packet_bytes, &mut decoder, &mut channels, &mut pre_skip, &mut skipped, &mut samples);
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

	let mut reader = hound::WavReader::open(wav_path).ok()?;
	let spec = reader.spec();
	let channels = spec.channels.max(1) as usize;
	// 10 ms windows, first channel only (WhatsApp measures a mono stream).
	let window = (spec.sample_rate as usize / 100).max(1);
	let mut levels = Vec::new();
	let (mut sum, mut count) = (0.0f64, 0usize);
	for (i, sample) in reader.samples::<i16>().enumerate() {
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

pub(crate) fn convert_wav_to_opus_ogg(wav_path: &str, ogg_path: &str) -> anyhow::Result<()> {
	use std::{fs::File, io::BufWriter};

	use hound::WavReader;
	use ogg::PacketWriter;
	use opus::{Application, Channels, Encoder};

	let mut reader = WavReader::open(wav_path)?;
	let spec = reader.spec();

	if spec.sample_rate != 48000 {
		return Err(anyhow::anyhow!("Sample rate must be 48000"));
	}
	let is_stereo = spec.channels == 2;
	let opus_channels = if is_stereo { Channels::Stereo } else { Channels::Mono };

	let mut encoder = Encoder::new(48000, opus_channels, Application::Voip)?;
	let out_file = File::create(ogg_path)?;
	let mut writer = PacketWriter::new(BufWriter::new(out_file));
	let serial = 12345;

	// OpusHead
	let mut head = Vec::new();
	head.extend_from_slice(b"OpusHead");
	head.push(1); // Version
	head.push(spec.channels as u8); // Channels
	head.extend_from_slice(&384_u16.to_le_bytes()); // Preskip
	head.extend_from_slice(&48000_u32.to_le_bytes()); // Input sample rate
	head.extend_from_slice(&0_u16.to_le_bytes()); // Gain
	head.push(0); // Channel mapping family
	writer.write_packet(head, serial, ogg::PacketWriteEndInfo::EndPage, 0)?;

	// OpusTags
	let mut tags = Vec::new();
	tags.extend_from_slice(b"OpusTags");
	tags.extend_from_slice(&8_u32.to_le_bytes()); // Vendor length
	tags.extend_from_slice(b"whaNTV  ");
	tags.extend_from_slice(&0_u32.to_le_bytes()); // User comments list length
	writer.write_packet(tags, serial, ogg::PacketWriteEndInfo::EndPage, 0)?;

	let mut granule = 0;
	let mut samples = Vec::new();
	let samples_per_frame = if is_stereo { 960 * 2 } else { 960 };

	for sample in reader.samples::<i16>() {
		samples.push(sample?);
		if samples.len() == samples_per_frame {
			let mut out = vec![0u8; 1024];
			let len = encoder.encode(&samples, &mut out)?;
			out.truncate(len);
			granule += 960;
			writer.write_packet(out, serial, ogg::PacketWriteEndInfo::NormalPacket, granule)?;
			samples.clear();
		}
	}

	if !samples.is_empty() {
		let remaining_frames = samples.len() / (if is_stereo { 2 } else { 1 });
		samples.resize(samples_per_frame, 0);
		let mut out = vec![0u8; 1024];
		let len = encoder.encode(&samples, &mut out)?;
		out.truncate(len);
		granule += remaining_frames as u64;
		writer.write_packet(out, serial, ogg::PacketWriteEndInfo::EndStream, granule)?;
	}

	Ok(())
}

#[cfg(test)]
mod waveform_tests {
	use super::compute_voice_waveform;

	#[test]
	fn waveform_is_64_bars_following_loudness() {
		let path = std::env::temp_dir().join(format!("whantv_waveform_test_{}.wav", std::process::id()));
		let spec = hound::WavSpec {
			channels: 2,
			sample_rate: 48_000,
			bits_per_sample: 16,
			sample_format: hound::SampleFormat::Int,
		};
		let mut w = hound::WavWriter::create(&path, spec).unwrap();
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
			let decoder = rodio::Decoder::new(std::io::Cursor::new(sound.ogg()));
			assert!(decoder.is_ok_and(|mut d| d.next().is_some()), "{:?} doesn't decode", sound as u8);
		}
	}
}
