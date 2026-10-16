//! GIFs go out the way WhatsApp sends them: as a short silent H.264 MP4 that
//! the receiving app loops (a video message with gifPlayback). WhatsApp Web
//! converts them the same way before sending (WAWebMediaAnalyzer, asGif);
//! phones don't animate a GIF sent as a photo. The frames are encoded with
//! Media Foundation's Sink Writer.

use std::io::BufReader;

use image::{AnimationDecoder, RgbaImage, codecs::gif::GifDecoder, imageops};
use windows::{
	Win32::{
		Media::MediaFoundation::{
			IMFAttributes, IMFByteStream, MF_MT_AVG_BITRATE, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE,
			MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MFCreateMediaType,
			MFCreateMemoryBuffer, MFCreateSample, MFCreateSinkWriterFromURL, MFMediaType_Video, MFVideoFormat_H264,
			MFVideoFormat_RGB32, MFVideoInterlace_Progressive,
		},
		System::Com::{COINIT_MULTITHREADED, CoInitializeEx},
	},
	core::HSTRING,
};

use crate::media_foundation::{self, pack_u64};

/// GIFs bigger than this are scaled down to it on their longest edge.
const MAX_EDGE: u32 = 640;
/// Long GIFs are cut off here rather than encoded whole.
const MAX_FRAMES: usize = 600;
/// Browsers (and WhatsApp) play frame delays under 20 ms as 100 ms.
const MIN_DELAY_MS: u32 = 20;
const DEFAULT_DELAY_MS: u32 = 100;
/// The frame rate the stream is labelled with; each frame carries its own
/// time, so this is only nominal.
const NOMINAL_FPS: u32 = 25;

/// Converts the GIF at `gif_path` into an MP4 at `mp4_path`, returning its
/// first frame as PNG bytes (for the message's preview).
pub(crate) fn gif_to_mp4(gif_path: &str, mp4_path: &str) -> anyhow::Result<Vec<u8>> {
	let decoder = GifDecoder::new(BufReader::new(std::fs::File::open(gif_path)?))?;
	let mut frames = Vec::new();
	for frame in decoder.into_frames().take(MAX_FRAMES) {
		let frame = frame?;
		let (numer, denom) = frame.delay().numer_denom_ms();
		let ms = numer.checked_div(denom).unwrap_or(DEFAULT_DELAY_MS);
		let ms = if ms < MIN_DELAY_MS { DEFAULT_DELAY_MS } else { ms };
		frames.push((frame.into_buffer(), ms));
	}
	let Some((first, _)) = frames.first() else {
		anyhow::bail!("GIF has no frames");
	};
	let (width, height) = encoded_size(first.width(), first.height());
	let frames: Vec<(RgbaImage, u32)> = frames
		.into_iter()
		.map(|(img, ms)| {
			let img = if img.width() != width || img.height() != height {
				imageops::resize(&img, width, height, imageops::FilterType::Triangle)
			} else {
				img
			};
			(img, ms)
		})
		.collect();

	let mut first_png = Vec::new();
	{
		use image::ImageEncoder;
		image::codecs::png::PngEncoder::new(&mut first_png).write_image(
			frames[0].0.as_raw(),
			width,
			height,
			image::ExtendedColorType::Rgba8,
		)?;
	}
	encode(&frames, width, height, mp4_path)?;
	Ok(first_png)
}

/// The size the video is encoded at: within MAX_EDGE, and even on both sides
/// as H.264 needs.
fn encoded_size(width: u32, height: u32) -> (u32, u32) {
	let (mut w, mut h) = (width.max(2), height.max(2));
	if w > MAX_EDGE || h > MAX_EDGE {
		if w >= h {
			h = (h * MAX_EDGE / w).max(2);
			w = MAX_EDGE;
		} else {
			w = (w * MAX_EDGE / h).max(2);
			h = MAX_EDGE;
		}
	}
	(w & !1, h & !1)
}

fn encode(frames: &[(RgbaImage, u32)], width: u32, height: u32, mp4_path: &str) -> anyhow::Result<()> {
	unsafe {
		// Runs on a worker thread of its own, which needs COM.
		let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
	}
	media_foundation::ensure_started();
	unsafe {
		let writer =
			MFCreateSinkWriterFromURL(&HSTRING::from(mp4_path), None::<&IMFByteStream>, None::<&IMFAttributes>)?;

		let out = MFCreateMediaType()?;
		out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
		out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
		// About 4 bits a pixel a second: plenty for GIF-sized pictures.
		out.SetUINT32(&MF_MT_AVG_BITRATE, (width * height * 4).clamp(200_000, 2_000_000))?;
		out.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
		out.SetUINT64(&MF_MT_FRAME_SIZE, pack_u64(width, height))?;
		out.SetUINT64(&MF_MT_FRAME_RATE, pack_u64(NOMINAL_FPS, 1))?;
		out.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack_u64(1, 1))?;
		let stream = writer.AddStream(&out)?;

		let input = MFCreateMediaType()?;
		input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
		input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
		input.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
		input.SetUINT64(&MF_MT_FRAME_SIZE, pack_u64(width, height))?;
		input.SetUINT64(&MF_MT_FRAME_RATE, pack_u64(NOMINAL_FPS, 1))?;
		input.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack_u64(1, 1))?;
		// A positive stride: rows are top-down, as the frames are.
		input.SetUINT32(&MF_MT_DEFAULT_STRIDE, width * 4)?;
		writer.SetInputMediaType(stream, &input, None::<&IMFAttributes>)?;

		writer.BeginWriting()?;
		let frame_bytes = width * height * 4;
		let mut time_100ns: i64 = 0;
		for (img, ms) in frames {
			let buffer = MFCreateMemoryBuffer(frame_bytes)?;
			let mut ptr: *mut u8 = std::ptr::null_mut();
			buffer.Lock(&mut ptr, None, None)?;
			let dst = std::slice::from_raw_parts_mut(ptr, frame_bytes as usize);
			for (d, s) in dst.chunks_exact_mut(4).zip(img.as_raw().chunks_exact(4)) {
				// RGBA to BGRX, over black where the GIF is transparent.
				let a = s[3] as u16;
				d[0] = (s[2] as u16 * a / 255) as u8;
				d[1] = (s[1] as u16 * a / 255) as u8;
				d[2] = (s[0] as u16 * a / 255) as u8;
				d[3] = 255;
			}
			buffer.Unlock()?;
			buffer.SetCurrentLength(frame_bytes)?;

			let sample = MFCreateSample()?;
			sample.AddBuffer(&buffer)?;
			let duration = *ms as i64 * 10_000;
			sample.SetSampleTime(time_100ns)?;
			sample.SetSampleDuration(duration)?;
			writer.WriteSample(stream, &sample)?;
			time_100ns += duration;
		}
		writer.Finalize()?;
	}
	Ok(())
}
