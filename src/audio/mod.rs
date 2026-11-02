pub mod alert_tones;
pub mod device_watch;
pub mod devices;
pub mod file;
pub mod ogg;
pub mod ringback;
pub mod sound_file;
pub mod system_audio;
pub mod voice_dsp;
pub mod voice_recorder;
pub mod wav;

use std::{
	sync::{
		Arc,
		atomic::{AtomicBool, AtomicU64, Ordering},
	},
	time::{SystemTime, UNIX_EPOCH},
};

use rodio::{DeviceSinkBuilder, MixerDeviceSink, Player};
use rodio_wsola::WsolaSourceExt;

use crate::{AppState, types::*};
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
	match devices::resolve_output_device(preferred_name) {
		Some(device) => DeviceSinkBuilder::from_device(device)?
			.with_error_callback(move |err| {
				log_line(&format!("output stream error, marking dead for reopen: {:?}", err));
				on_error.store(true, Ordering::Relaxed);
			})
			.open_stream(),
		None => DeviceSinkBuilder::open_default_sink(),
	}
}

const AUDIO_STREAM_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(20);

fn log_line(msg: &str) {
	use std::io::Write;
	let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
	let line = format!("[{}.{:03}] {}\n", now.as_secs(), now.subsec_millis(), msg);
	if let Ok(mut f) =
		std::fs::OpenOptions::new().create(true).append(true).open(crate::platform::paths::log_dir().join("audio.log"))
	{
		let _ = f.write_all(line.as_bytes());
	}
}

pub(crate) fn ensure_audio_stream(s: &mut AppState) {
	let current_name = devices::resolve_output_device_name(s.settings.output_device_name.as_deref());
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
	if sounds_enabled {
		play_once(output_device_name, move |sink| {
			if let Some(decoded) = decode_opus(sound.ogg()) {
				sink.append(decoded);
			}
		});
	}
}

/// Plays a new message's tone (see [`whatsnap::TONE_DEFAULT`]): Default is
/// the usual background message sound, None nothing, Custom `file` (or
/// Default when it can't be played).
pub(crate) fn play_message_tone(tone: u8, file: &str, sounds_enabled: bool, output_device_name: Option<String>) {
	if sounds_enabled && let Some(fill) = message_tone(tone, file) {
		play_once(output_device_name, fill);
	}
}

/// Plays a tone picked in a tone list, stopping the one picked before.
pub(crate) fn preview_message_tone(tone: u8, file: &str, output_device_name: Option<String>) {
	match message_tone(tone, file) {
		Some(fill) => play_preview(output_device_name, fill),
		None => play_preview(output_device_name, |_| {}),
	}
}

fn message_tone(tone: u8, file: &str) -> Option<impl FnOnce(&Player) + Send + 'static> {
	if tone == whatsnap::TONE_NONE {
		return None;
	}
	let file = file.to_string();
	Some(move |sink: &Player| {
		use rodio::Source;
		if tone == whatsnap::TONE_CUSTOM
			&& let Some(custom) = sound_file::open(&file)
		{
			sink.append(custom.take_duration(sound_file::LONGEST_TONE));
		} else if let Some(alert) = alert_tones::alert_tone(tone) {
			sink.append(alert);
		} else if let Some(decoded) = decode_opus(NotificationSound::MessageBackground.ogg()) {
			sink.append(decoded);
		}
	})
}

/// Plays a ringtone (see [`whatsnap::RINGTONE_WHATSAPP`]) once, cut short,
/// for a ringtone list to preview, stopping the one picked before.
pub(crate) fn preview_ringtone(ringtone: &str, output_device_name: Option<String>) {
	use rodio::Source;
	let ringtone = ringtone.to_string();
	play_preview(output_device_name, move |sink| match ringtone.as_str() {
		whatsnap::RINGTONE_NONE => {}
		"" | whatsnap::RINGTONE_WHATSAPP => {
			if let Some(ring) = decode_opus(WHATSAPP_RINGTONE) {
				sink.append(ring.take_duration(sound_file::LONGEST_PREVIEW));
			}
		}
		path => {
			if let Some(ring) = sound_file::open(path) {
				sink.append(ring.take_duration(sound_file::LONGEST_PREVIEW));
			}
		}
	});
}

fn play_once(output_device_name: Option<String>, fill: impl FnOnce(&Player) + Send + 'static) {
	let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
	let last = LAST_SOUND_TIME.swap(now, Ordering::Relaxed);
	if now - last < 500 {
		return;
	}
	play_on_own_stream(output_device_name, None, fill);
}

/// Each preview gets the next number; a preview still playing stops once
/// it's no longer the latest, so arrowing through a list plays only the
/// sound now picked.
static PREVIEW: AtomicU64 = AtomicU64::new(0);

fn play_preview(output_device_name: Option<String>, fill: impl FnOnce(&Player) + Send + 'static) {
	let preview = PREVIEW.fetch_add(1, Ordering::Relaxed) + 1;
	play_on_own_stream(output_device_name, Some(preview), fill);
}

fn play_on_own_stream(
	output_device_name: Option<String>,
	preview: Option<u64>,
	fill: impl FnOnce(&Player) + Send + 'static,
) {
	std::thread::spawn(move || {
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
		let stream_result = match devices::resolve_output_device(output_device_name.as_deref()) {
			Some(device) => DeviceSinkBuilder::from_device(device).and_then(|b| b.open_stream()),
			None => DeviceSinkBuilder::open_default_sink(),
		};
		let Ok(stream) = stream_result else {
			return;
		};
		let sink = Player::connect_new(stream.mixer());
		fill(&sink);
		if sink.empty() {
			return;
		}
		sink.play();
		match preview {
			None => sink.sleep_until_end(),
			Some(preview) => {
				while !sink.empty() {
					if PREVIEW.load(Ordering::Relaxed) != preview {
						sink.stop();
						return;
					}
					std::thread::sleep(std::time::Duration::from_millis(20));
				}
			}
		}
	});
}

pub(crate) fn play_call_fail_sound(state: &mut AppState) {
	ensure_audio_stream(state);
	if let Some(ref stream_handle) = state.audio_stream {
		let sink = Player::connect_new(stream_handle.mixer());
		match decode_opus(include_bytes!("../../sounds/whatsapp_windows_hangup_01_705kbps.ogg")) {
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

pub(crate) const WHATSAPP_RINGTONE: &[u8] = include_bytes!("../../sounds/ringtone_incoming_call.ogg");

/// Rings for an incoming call with `ringtone` (see
/// [`whatsnap::RINGTONE_WHATSAPP`]), looped, falling back to WhatsApp's own
/// when its file can't be played.
pub(crate) fn start_call_ringtone(state: &mut AppState, ringtone: &str) {
	match ringtone {
		whatsnap::RINGTONE_NONE => {}
		"" | whatsnap::RINGTONE_WHATSAPP => start_call_ringing_sound(state, WHATSAPP_RINGTONE),
		path => {
			ensure_audio_stream(state);
			let Some(ref stream_handle) = state.audio_stream else { return };
			match sound_file::Looped::open(path) {
				Some(ring) => {
					let sink = Player::connect_new(stream_handle.mixer());
					sink.append(ring);
					sink.play();
					state.call_ringing_sink = Some(sink);
				}
				None => {
					eprintln!("[AUDIO] Couldn't play the ringtone {path}, using WhatsApp's");
					start_call_ringing_sound(state, WHATSAPP_RINGTONE);
				}
			}
		}
	}
}

pub(crate) fn start_call_ringing_sound(state: &mut AppState, sound_bytes: &'static [u8]) {
	ensure_audio_stream(state);
	if let Some(ref stream_handle) = state.audio_stream {
		let sink = Player::connect_new(stream_handle.mixer());
		match decode_opus(sound_bytes) {
			Some(decoded) => {
				use rodio::Source;
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

pub(crate) fn start_call_ringback(state: &mut AppState, self_phone: &str) {
	use rodio::Source;
	ensure_audio_stream(state);
	let Some(ref stream_handle) = state.audio_stream else {
		eprintln!("[AUDIO] No audio stream available");
		return;
	};
	let sink = Player::connect_new(stream_handle.mixer());
	sink.append(crate::audio::ringback::ringback_cycle(self_phone).repeat_infinite());
	sink.play();
	state.call_ringing_sink = Some(sink);
}

pub(crate) fn play_busy_tone(state: &mut AppState) {
	ensure_audio_stream(state);
	let Some(ref stream_handle) = state.audio_stream else {
		eprintln!("[AUDIO] No audio stream available");
		return;
	};
	let sink = Player::connect_new(stream_handle.mixer());
	sink.append(crate::audio::ringback::busy_tone());
	sink.play();
	state.call_ringing_sink = Some(sink);
}

pub(crate) fn start_call_reconnect_tone(state: &mut AppState) {
	use rodio::Source;
	if state.call_reconnect_sink.is_some() {
		return;
	}
	ensure_audio_stream(state);
	let Some(ref stream_handle) = state.audio_stream else {
		eprintln!("[AUDIO] No audio stream available");
		return;
	};
	let sink = Player::connect_new(stream_handle.mixer());
	sink.append(crate::audio::ringback::reconnect_tone().repeat_infinite());
	sink.play();
	state.call_reconnect_sink = Some(sink);
}

pub(crate) fn stop_call_reconnect_tone(state: &mut AppState) {
	if let Some(sink) = state.call_reconnect_sink.take() {
		sink.pause();
	}
}

pub(crate) fn stop_call_ringing_sound(state: &mut AppState) {
	if let Some(sink) = state.call_ringing_sink.take() {
		sink.pause();
	}
	state.call_hotkeys = None;
}

pub(crate) fn audio_file_seconds(path: &str) -> u64 {
	crate::audio::file::duration(path).map_or(0, |d| d.as_secs_f64().round() as u64)
}

pub(crate) fn concat_wav_segments(segments: &[String], out_path: &str) -> crate::error::Result<()> {
	use crate::audio::wav::{WavReader, WavWriter};
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
	let source = crate::audio::file::open(path, std::time::Duration::from_secs_f32(target_secs.max(0.0)))?;
	let sink = Player::connect_new(stream_handle.mixer());
	sink.append(source.wsola(speed));
	sink.play();
	Some(sink)
}

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
	let mut pending: Vec<u8> = Vec::new();
	let mut pcm: Vec<i16> = Vec::new();

	while pos + 27 <= len {
		if &data[pos..pos + 4] != b"OggS" {
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

pub(crate) fn compute_voice_waveform(wav_path: &str) -> Option<Vec<u8>> {
	let reader = crate::audio::wav::WavReader::open(wav_path).ok()?;
	let spec = reader.spec();
	let channels = spec.channels.max(1) as usize;
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
	waveform_bars(&levels)
}

pub(crate) fn audio_file_waveform(path: &str) -> Option<Vec<u8>> {
	use rodio::Source;
	let source = crate::audio::file::open(path, std::time::Duration::ZERO)?;
	let channels = source.channels().get().max(1) as usize;
	let window = (source.sample_rate().get() as usize / 100).max(1);
	let mut levels = Vec::new();
	let (mut sum, mut count) = (0.0f64, 0usize);
	for (i, sample) in source.enumerate() {
		if i % channels != 0 {
			continue;
		}
		let s = sample as f64;
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
	waveform_bars(&levels)
}

fn waveform_bars(levels: &[f32]) -> Option<Vec<u8>> {
	const BARS: usize = 64;
	const MIN_DB: f32 = -60.0;
	const MAX_DB: f32 = -10.0;
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

pub(crate) fn convert_wav_to_opus_ogg(wav_path: &str, ogg_path: &str) -> crate::error::Result<()> {
	use std::{
		fs::File,
		io::{BufWriter, Write},
	};

	use opus::{Application, Channels, Encoder};

	use crate::audio::{
		ogg::{OggWriter, PacketEnd},
		wav::WavReader,
	};

	const FRAME: usize = 960;
	const PACKETS_PER_PAGE: usize = 50;

	let reader = WavReader::open(wav_path)?;
	let spec = reader.spec();
	if spec.sample_rate != 48000 {
		return Err(crate::error::err!("Sample rate must be 48000"));
	}
	// Recordings are stereo, but voice notes go out mono like WhatsApp's own:
	// iPhones refuse to play stereo voice notes.
	let ch = spec.channels.max(1) as usize;

	let mut encoder = Encoder::new(48000, Channels::Mono, Application::Voip)?;
	let pre_skip = encoder.get_lookahead()?.max(0) as usize;

	let mut writer = OggWriter::new(BufWriter::new(File::create(ogg_path)?), 12345);

	let mut head = Vec::with_capacity(19);
	head.extend_from_slice(b"OpusHead");
	head.push(1);
	head.push(1);
	head.extend_from_slice(&(pre_skip as u16).to_le_bytes());
	head.extend_from_slice(&48000_u32.to_le_bytes());
	head.extend_from_slice(&0_u16.to_le_bytes());
	head.push(0);
	writer.write_packet(&head, PacketEnd::EndPage, 0)?;

	let mut tags = Vec::with_capacity(24);
	tags.extend_from_slice(b"OpusTags");
	const VENDOR: &[u8] = b"whatSnap";
	tags.extend_from_slice(&(VENDOR.len() as u32).to_le_bytes());
	tags.extend_from_slice(VENDOR);
	tags.extend_from_slice(&0_u32.to_le_bytes());
	writer.write_packet(&tags, PacketEnd::EndPage, 0)?;

	let segments = |len: usize| len / 255 + 1;
	let mut samples = reader;
	let mut frame = vec![0i16; FRAME * ch];
	let mut mono = [0i16; FRAME];
	let mut out = [0u8; 1275];
	let mut held: Option<(Vec<u8>, u64)> = None;
	let (mut page_packets, mut page_segments) = (0, 0);
	let mut input_done = false;
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
		filled -= filled % ch;
		total += (filled / ch) as u64;
		let pad_now = pad.min(FRAME - filled / ch);
		pad -= pad_now;
		filled += pad_now * ch;
		if filled == 0 {
			break;
		}
		frame[filled..].fill(0);
		for (m, f) in mono.iter_mut().zip(frame.chunks_exact(ch)) {
			*m = (f.iter().map(|&s| s as i32).sum::<i32>() / ch as i32) as i16;
		}
		let len = encoder.encode(&mono, &mut out)?;
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
		let spec = crate::audio::wav::WavSpec { channels: 2, sample_rate: 48_000 };
		let mut w = crate::audio::wav::WavWriter::create(path.to_str().unwrap(), spec).unwrap();
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
		for total in [350_000u64, 96_000] {
			let dir = std::env::temp_dir();
			let wav = dir.join(format!("whantv_voice_test_{}_{total}.wav", std::process::id()));
			let ogg = wav.with_extension("ogg");
			let spec = crate::audio::wav::WavSpec { channels: 2, sample_rate: 48_000 };
			let mut w = crate::audio::wav::WavWriter::create(wav.to_str().unwrap(), spec).unwrap();
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
			assert_eq!(data[37], 1, "mono, or iPhones won't play it");
			let pre_skip = u16::from_le_bytes([data[38], data[39]]) as u64;
			let tags = &data[75..75 + data[74] as usize];
			assert_eq!(&tags[..8], b"OpusTags");
			let vendor = u32::from_le_bytes(tags[8..12].try_into().unwrap()) as usize;
			assert_eq!(tags.len(), 12 + vendor + 4, "vendor length doesn't match");
			assert_eq!(&tags[12 + vendor..], [0, 0, 0, 0], "comment count");

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
			assert!(decoded >= total, "{decoded} < {total}");
		}
	}
}
