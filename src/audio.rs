use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use rodio_wsola::WsolaSourceExt;
use crate::{AppState, audio_devices};
use crate::types::*;
use std::io::{Cursor, Read, Write};
static LAST_SOUND_TIME: AtomicU64 = AtomicU64::new(0);

pub(crate) fn open_output_stream(preferred_name: Option<&str>) -> Result<MixerDeviceSink, rodio::DeviceSinkError> {
	match audio_devices::resolve_output_device(preferred_name) {
		Some(device) => DeviceSinkBuilder::from_device(device).and_then(|b| b.open_stream()),
		None => DeviceSinkBuilder::open_default_sink(),
	}
}

pub(crate) fn ensure_audio_stream(s: &mut AppState) {
	let current_name = audio_devices::resolve_output_device_name(s.settings.output_device_name.as_deref());
	if s.audio_stream.is_some() && s.audio_stream_device_name == current_name {
		return;
	}
	match open_output_stream(s.settings.output_device_name.as_deref()) {
		Ok(stream) => {
			s.audio_stream = Some(stream);
			s.audio_stream_device_name = current_name;
		}
		Err(e) => {
			eprintln!("[AUDIO] Failed to (re)open output stream: {:?}", e);
			s.audio_stream = None;
			s.audio_stream_device_name = None;
		}
	}
}

pub(crate) fn play_notification_sound(sound: NotificationSound, sounds_enabled: bool) {
	if !sounds_enabled {
		return;
	}
	let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
	let last = LAST_SOUND_TIME.swap(now, Ordering::Relaxed);
	if now - last < 500 {
		return;
	}
	std::thread::spawn(move || {
		let Ok(stream) = DeviceSinkBuilder::open_default_sink() else {
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

pub(crate) fn stop_call_ringing_sound(state: &mut AppState) {
	if let Some(sink) = state.call_ringing_sink.take() {
		sink.pause();
	}
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



