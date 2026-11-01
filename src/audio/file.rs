//! Opening received audio files for playback.
//!
//! Voice notes are Ogg Opus and go through libopus (`audio::decode_opus`).
//! Everything else WhatsApp carries (MP3, AAC/M4A, WAV, FLAC, AMR...) is
//! decoded by Media Foundation, which Windows already has, on a thread of
//! its own that streams float samples to the player a few chunks ahead.

use std::{
	num::{NonZeroU16, NonZeroU32},
	sync::mpsc::{Receiver, SyncSender, sync_channel},
	time::Duration,
};

use rodio::Source;
use windows::{
	Win32::{
		Media::MediaFoundation::{
			IMFSample, IMFSourceReader, MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE,
			MF_MT_SUBTYPE, MF_PD_DURATION, MF_SOURCE_READER_ALL_STREAMS, MF_SOURCE_READER_FIRST_AUDIO_STREAM,
			MF_SOURCE_READER_MEDIASOURCE, MF_SOURCE_READERF_ENDOFSTREAM, MFAudioFormat_Float, MFCreateMediaType,
			MFCreateSourceReaderFromURL, MFMediaType_Audio,
		},
		System::{
			Com::{
				COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize,
				StructuredStorage::{PROPVARIANT, PropVariantToUInt64},
			},
			Variant::VT_I8,
		},
	},
	core::{GUID, HSTRING},
};

use crate::{audio::decode_opus, media::foundation};

/// Decoded chunks the decoding thread may run ahead of playback.
const CHUNKS_AHEAD: usize = 8;

/// The audio file at `path`, ready to play from `start` on.
pub(crate) fn open(path: &str, start: Duration) -> Option<Box<dyn Source + Send>> {
	if let Some(opus) = read_ogg_opus(path) {
		return Some(Box::new(decode_opus(&opus)?.skip_duration(start)));
	}
	MfSource::open(path, start).map(|s| Box::new(s) as Box<dyn Source + Send>)
}

/// How long the audio file at `path` plays for.
pub(crate) fn duration(path: &str) -> Option<Duration> {
	if let Some(opus) = read_ogg_opus(path) {
		return decode_opus(&opus)?.total_duration();
	}
	let _com = ComGuard::new();
	foundation::ensure_started();
	unsafe { open_reader(path).ok().and_then(|reader| reader_duration(&reader)) }
}

/// The file's bytes, if it's an Ogg Opus stream.
fn read_ogg_opus(path: &str) -> Option<Vec<u8>> {
	use std::io::Read;
	let mut head = [0u8; 36];
	std::fs::File::open(path).ok()?.read_exact(&mut head).ok()?;
	(&head[0..4] == b"OggS" && &head[28..36] == b"OpusHead").then(|| std::fs::read(path).ok())?
}

struct ComGuard(bool);

impl ComGuard {
	fn new() -> Self {
		ComGuard(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok())
	}
}

impl Drop for ComGuard {
	fn drop(&mut self) {
		if self.0 {
			unsafe { CoUninitialize() };
		}
	}
}

const AUDIO_STREAM: u32 = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;

/// A Source Reader on the file's first audio stream, decoding to float PCM.
unsafe fn open_reader(path: &str) -> windows::core::Result<IMFSourceReader> {
	unsafe {
		let reader = MFCreateSourceReaderFromURL(&HSTRING::from(path), None)?;
		reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
		reader.SetStreamSelection(AUDIO_STREAM, true)?;
		let float = MFCreateMediaType()?;
		float.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
		float.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float)?;
		reader.SetCurrentMediaType(AUDIO_STREAM, None, &float)?;
		Ok(reader)
	}
}

unsafe fn reader_duration(reader: &IMFSourceReader) -> Option<Duration> {
	unsafe {
		let value = reader.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION).ok()?;
		// In 100 ns units.
		let ticks = PropVariantToUInt64(&value).ok()?;
		Some(Duration::from_nanos(ticks.saturating_mul(100)))
	}
}

struct Format {
	channels: NonZeroU16,
	sample_rate: NonZeroU32,
	duration: Option<Duration>,
}

/// Plays a file Media Foundation decodes. The reader lives on its own thread
/// (COM objects stay on the thread that made them), which stops once this is
/// dropped and its channel closes.
struct MfSource {
	chunks: Receiver<Vec<f32>>,
	chunk: Vec<f32>,
	at: usize,
	format: Format,
}

impl MfSource {
	fn open(path: &str, start: Duration) -> Option<MfSource> {
		let (format_tx, format_rx) = sync_channel(1);
		let (chunk_tx, chunks) = sync_channel(CHUNKS_AHEAD);
		let path = path.to_string();
		std::thread::spawn(move || {
			let _com = ComGuard::new();
			foundation::ensure_started();
			unsafe { decode(&path, start, &format_tx, &chunk_tx) };
		});
		let format = format_rx.recv().ok()?;
		Some(MfSource { chunks, chunk: Vec::new(), at: 0, format })
	}
}

/// Opens `path`, reports its format through `format_tx` (dropping it
/// unsent if the file can't be decoded), then sends chunks until the end of
/// the file or until nobody's listening.
unsafe fn decode(path: &str, start: Duration, format_tx: &SyncSender<Format>, chunk_tx: &SyncSender<Vec<f32>>) {
	unsafe {
		let Ok(reader) = open_reader(path) else { return };
		let Ok(media_type) = reader.GetCurrentMediaType(AUDIO_STREAM) else { return };
		let channels = media_type.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).ok().and_then(|c| NonZeroU16::new(c as u16));
		let sample_rate = media_type.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND).ok().and_then(NonZeroU32::new);
		let (Some(channels), Some(sample_rate)) = (channels, sample_rate) else { return };
		let duration = reader_duration(&reader);
		if !start.is_zero() {
			// Positions are 100 ns units in a VT_I8.
			let mut position = PROPVARIANT::default();
			let inner = &mut *position.Anonymous.Anonymous;
			inner.vt = VT_I8;
			inner.Anonymous.hVal = (start.as_nanos() / 100).min(i64::MAX as u128) as i64;
			let _ = reader.SetCurrentPosition(&GUID::zeroed(), &position);
		}
		if format_tx.send(Format { channels, sample_rate, duration }).is_err() {
			return;
		}
		loop {
			let mut flags = 0u32;
			let mut sample: Option<IMFSample> = None;
			if reader.ReadSample(AUDIO_STREAM, 0, None, Some(&mut flags), None, Some(&mut sample)).is_err() {
				return;
			}
			if let Some(sample) = sample
				&& let Some(chunk) = sample_floats(&sample)
				&& !chunk.is_empty()
				&& chunk_tx.send(chunk).is_err()
			{
				return;
			}
			if flags & (MF_SOURCE_READERF_ENDOFSTREAM.0 as u32) != 0 {
				return;
			}
		}
	}
}

unsafe fn sample_floats(sample: &IMFSample) -> Option<Vec<f32>> {
	unsafe {
		let buffer = sample.ConvertToContiguousBuffer().ok()?;
		let mut ptr: *mut u8 = std::ptr::null_mut();
		let mut len = 0u32;
		buffer.Lock(&mut ptr, None, Some(&mut len)).ok()?;
		let floats = if ptr.is_null() {
			Vec::new()
		} else {
			// Media Foundation's buffers are aligned for any sample type.
			std::slice::from_raw_parts(ptr as *const f32, len as usize / 4).to_vec()
		};
		let _ = buffer.Unlock();
		Some(floats)
	}
}

impl Iterator for MfSource {
	type Item = f32;

	fn next(&mut self) -> Option<f32> {
		if self.at == self.chunk.len() {
			self.chunk = self.chunks.recv().ok()?;
			self.at = 0;
		}
		self.at += 1;
		Some(self.chunk[self.at - 1])
	}
}

impl Source for MfSource {
	fn current_span_len(&self) -> Option<usize> {
		// The format never changes mid-file (the reader converts everything
		// to the type it was opened with).
		None
	}

	fn channels(&self) -> rodio::ChannelCount {
		self.format.channels
	}

	fn sample_rate(&self) -> rodio::SampleRate {
		self.format.sample_rate
	}

	fn total_duration(&self) -> Option<Duration> {
		self.format.duration
	}
}
