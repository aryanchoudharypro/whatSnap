//! Captures the local webcam, or the screen for a screen share, and encodes
//! H.264 via Media Foundation, shipping Annex-B access units to the backend's
//! per-call TCP video bridge (`backend/video_bridge.go`). A call has one video
//! stream, so the two sources take turns on it rather than running together. Mirrors `call_audio.rs`'s WASAPI pattern: a
//! dedicated thread owns its own COM/MF objects for its whole life and is
//! recreated wholesale (not fixed up in place) whenever the device changes
//! or is lost, rather than trying to hot-patch a live session.
//!
//! Target: 1280x720 @ 30fps, ~1.5 Mbps CBR - see the Phase 2 design
//! decisions in [[video_calls_work_status]].

use std::{
	io::Write,
	net::TcpStream,
	sync::{
		Arc, Mutex,
		atomic::{AtomicBool, Ordering},
	},
	time::{Duration, Instant},
};

use windows::{
	Win32::{
		Media::MediaFoundation::{
			CODECAPI_AVEncCommonMeanBitRate, CODECAPI_AVEncCommonRateControlMode, CODECAPI_AVEncVideoForceKeyFrame,
			ICodecAPI, IMFMediaType, IMFSample, IMFSourceReader, IMFTransform, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
			MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
			MF_E_TRANSFORM_NEED_MORE_INPUT, MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE,
			MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_MPEG2_PROFILE, MF_MT_SUBTYPE,
			MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, MF_SOURCE_READER_FIRST_VIDEO_STREAM,
			MF_SOURCE_READERF_ENDOFSTREAM, MFCreateAttributes, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample,
			MFCreateSourceReaderFromMediaSource, MFEnumDeviceSources, MFMediaType_Video,
			MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, MFT_MESSAGE_NOTIFY_END_OF_STREAM, MFT_MESSAGE_NOTIFY_END_STREAMING,
			MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_OUTPUT_DATA_BUFFER, MFVideoFormat_H264, MFVideoFormat_NV12,
			MFVideoInterlace_Progressive, eAVEncCommonRateControlMode_CBR, eAVEncH264VProfile_Base,
		},
		System::{
			Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize},
			Variant::VARIANT,
		},
	},
	core::{GUID, Interface},
};

use crate::{
	media_foundation, paths,
	screen_capture::{self, ScreenCapture, ShareTarget},
	ui_wake::UiCommandSender,
	video_devices,
};

const TARGET_WIDTH: u32 = 1280;
const TARGET_HEIGHT: u32 = 720;
const TARGET_FPS: u32 = 30;
const TARGET_BITRATE: u32 = 1_500_000;
/// A screen changes far less often than a camera picture, and the bits a
/// lower frame rate saves go into keeping text sharp.
const SCREEN_FPS: u32 = 15;

/// What a video session sends.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VideoSource {
	Camera,
	/// A monitor or a window, see `screen_capture.rs`.
	Screen(ShareTarget),
}

pub(crate) fn log_line(msg: &str) {
	let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
	let line = format!("[{}.{:03}] {}\n", now.as_secs(), now.subsec_millis(), msg);
	if let Ok(mut f) =
		std::fs::OpenOptions::new().create(true).append(true).open(paths::log_dir().join("call_video_send.log"))
	{
		let _ = f.write_all(line.as_bytes());
	}
}

/// Handle to a running capture/encode/send session. Dropping it stops
/// the background thread; the thread notices on its own timing, same as
/// `call_audio::CallAudioState`.
pub struct CallVideoSendState {
	stop: Arc<AtomicBool>,
	preferred_device: Arc<Mutex<Option<String>>>,
	force_keyframe: Arc<AtomicBool>,
}

impl CallVideoSendState {
	/// Switches the camera in place, without dropping the running session
	/// wholesale from the caller's point of view - the capture thread notices
	/// the change and reopens on the new device itself.
	pub fn set_device(&self, name: Option<String>) {
		*self.preferred_device.lock().unwrap() = name;
	}

	/// Asks the encoder to make its next access unit an IDR - wired to the
	/// backend's `call_video_keyframe_request` event (WhatsApp's PLI/FIR
	/// feedback, relayed from `OnVideoKeyframeRequest`).
	pub fn request_keyframe(&self) {
		self.force_keyframe.store(true, Ordering::Relaxed);
	}
}

impl Drop for CallVideoSendState {
	fn drop(&mut self) {
		self.stop.store(true, Ordering::Relaxed);
	}
}

/// Starts the capture/encode/send thread. Never fails synchronously: if the
/// very first attempt to open the source fails outright (no camera present,
/// permission denied, etc), the thread reports it once via
/// `UiCommand::CameraFailed` (or `ScreenShareFailed`) and exits without
/// retrying - there's nothing to recover into. `initial_device` only matters
/// for a camera. A camera that opens fine and is later lost (unplugged) does
/// retry, the same way `call_audio.rs` recovers from a lost audio device.
pub fn start_video_send(
	target_port: u16,
	source: VideoSource,
	initial_device: Option<String>,
	ui_tx: UiCommandSender,
) -> CallVideoSendState {
	let stop = Arc::new(AtomicBool::new(false));
	let preferred_device = Arc::new(Mutex::new(initial_device));
	let force_reopen = Arc::new(AtomicBool::new(false));
	let force_keyframe = Arc::new(AtomicBool::new(false));

	let thread_stop = Arc::clone(&stop);
	let thread_preferred = Arc::clone(&preferred_device);
	let thread_force_reopen = Arc::clone(&force_reopen);
	let thread_force_keyframe = Arc::clone(&force_keyframe);

	let spawned = std::thread::Builder::new().name("video send".into()).spawn(move || {
		run(target_port, source, thread_stop, thread_preferred, thread_force_reopen, thread_force_keyframe, ui_tx);
	});
	if let Err(e) = spawned {
		log_line(&format!("Couldn't start the video send thread: {e}"));
	}

	CallVideoSendState { stop, preferred_device, force_keyframe }
}

fn run(
	target_port: u16,
	source: VideoSource,
	stop: Arc<AtomicBool>,
	preferred_device: Arc<Mutex<Option<String>>>,
	force_reopen: Arc<AtomicBool>,
	force_keyframe: Arc<AtomicBool>,
	ui_tx: UiCommandSender,
) {
	unsafe {
		let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
	}
	media_foundation::ensure_started();

	let mut ever_opened = false;
	while !stop.load(Ordering::Relaxed) {
		let device_name = preferred_device.lock().unwrap().clone();
		force_reopen.store(false, Ordering::Relaxed);

		if let VideoSource::Screen(target) = source {
			log_line(&format!("Opening screen share session for {target:?}"));
		}
		match open_session(target_port, source, device_name.as_deref()) {
			Ok(mut session) => {
				if matches!(source, VideoSource::Screen(_)) {
					log_line("Screen share session open");
				}
				ever_opened = true;
				encode_loop(&mut session, &stop, &preferred_device, &force_reopen, &force_keyframe, &mut |black| {
					let _ = ui_tx.send(crate::UiCommand::ScreenShareBlack(black));
				});
				shutdown_session(&session);
				if let Capture::Screen(screen) = &session.capture
					&& screen.target_gone()
					&& !stop.load(Ordering::Relaxed)
				{
					let _ = ui_tx.send(crate::UiCommand::ScreenShareFailed("the window was closed".to_string()));
					break;
				}
			}
			Err(e) => {
				log_line(&format!("Failed to open {source:?} session: {e}"));
				if !ever_opened || matches!(source, VideoSource::Screen(ShareTarget::Window(_))) {
					let _ = ui_tx.send(match source {
						VideoSource::Camera => crate::UiCommand::CameraFailed(e.to_string()),
						VideoSource::Screen(_) => crate::UiCommand::ScreenShareFailed(e.to_string()),
					});
					break;
				}
			}
		}

		if !stop.load(Ordering::Relaxed) {
			std::thread::sleep(Duration::from_millis(500));
		}
	}

	unsafe {
		CoUninitialize();
	}
}

struct Session {
	capture: Capture,
	output: Output,
}

/// Where a session's pictures come from.
enum Capture {
	Camera { reader: IMFSourceReader, opened_symbolic_link: String },
	Screen(ScreenCapture),
}

/// The encoder and the socket its access units go out on.
struct Output {
	encoder: IMFTransform,
	codec_api: ICodecAPI,
	tcp: TcpStream,
}

fn pack(high: u32, low: u32) -> u64 {
	media_foundation::pack_u64(high, low)
}

/// Finds the `IMFActivate` for the camera identified by `symbolic_link`,
/// re-enumerating every time - Media Foundation hands out fresh activate
/// objects per call and they can't usefully be cached across sessions.
unsafe fn activate_camera_source(
	symbolic_link: &str,
) -> crate::error::Result<windows::Win32::Media::MediaFoundation::IMFMediaSource> {
	unsafe {
		let mut attributes: Option<windows::Win32::Media::MediaFoundation::IMFAttributes> = None;
		MFCreateAttributes(&mut attributes, 1)?;
		let attributes = attributes.ok_or_else(|| crate::error::err!("MFCreateAttributes returned no attributes"))?;
		attributes.SetGUID(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID)?;

		let mut activates: *mut Option<windows::Win32::Media::MediaFoundation::IMFActivate> = std::ptr::null_mut();
		let mut count: u32 = 0;
		MFEnumDeviceSources(&attributes, &mut activates, &mut count)?;
		if activates.is_null() {
			crate::error::bail!("no cameras available");
		}
		let slice = std::slice::from_raw_parts(activates, count as usize);
		let mut found = None;
		for entry in slice {
			let Some(activate) = entry else { continue };
			let mut ptr = windows::core::PWSTR::null();
			let mut len: u32 = 0;
			if activate
				.GetAllocatedString(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, &mut ptr, &mut len)
				.is_err()
				|| ptr.is_null()
			{
				continue;
			}
			let link = ptr.to_string().unwrap_or_default();
			windows::Win32::System::Com::CoTaskMemFree(Some(ptr.0 as *const core::ffi::c_void));
			if link == symbolic_link {
				found = Some(activate.clone());
				break;
			}
		}
		windows::Win32::System::Com::CoTaskMemFree(Some(activates as *const core::ffi::c_void));

		let Some(activate) = found else {
			crate::error::bail!("camera is no longer available");
		};
		Ok(activate.ActivateObject::<windows::Win32::Media::MediaFoundation::IMFMediaSource>()?)
	}
}

fn build_video_type(
	subtype: &GUID,
	fps: u32,
	extra: impl FnOnce(&IMFMediaType) -> crate::error::Result<()>,
) -> crate::error::Result<IMFMediaType> {
	unsafe {
		let mt = MFCreateMediaType()?;
		mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
		mt.SetGUID(&MF_MT_SUBTYPE, subtype)?;
		mt.SetUINT64(&MF_MT_FRAME_SIZE, pack(TARGET_WIDTH, TARGET_HEIGHT))?;
		mt.SetUINT64(&MF_MT_FRAME_RATE, pack(fps, 1))?;
		mt.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
		extra(&mt)?;
		Ok(mt)
	}
}

fn open_session(target_port: u16, source: VideoSource, device_name: Option<&str>) -> crate::error::Result<Session> {
	let capture = match source {
		VideoSource::Camera => open_camera(device_name)?,
		VideoSource::Screen(target) => Capture::Screen(ScreenCapture::new(target, TARGET_WIDTH, TARGET_HEIGHT)?),
	};
	let fps = match source {
		VideoSource::Camera => TARGET_FPS,
		VideoSource::Screen(_) => SCREEN_FPS,
	};
	Ok(Session { capture, output: open_output(target_port, fps)? })
}

fn open_camera(device_name: Option<&str>) -> crate::error::Result<Capture> {
	let Some(symbolic_link) = video_devices::resolve_camera_symbolic_link(device_name) else {
		crate::error::bail!("no camera available");
	};

	unsafe {
		let source = activate_camera_source(&symbolic_link)?;

		let mut reader_attrs: Option<windows::Win32::Media::MediaFoundation::IMFAttributes> = None;
		MFCreateAttributes(&mut reader_attrs, 1)?;
		let reader_attrs =
			reader_attrs.ok_or_else(|| crate::error::err!("MFCreateAttributes returned no attributes"))?;
		reader_attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)?;
		let reader = MFCreateSourceReaderFromMediaSource(&source, &reader_attrs)?;

		let capture_type = build_video_type(&MFVideoFormat_NV12, TARGET_FPS, |_| Ok(()))?;
		reader.SetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32, None, &capture_type)?;

		Ok(Capture::Camera { reader, opened_symbolic_link: symbolic_link })
	}
}

fn open_output(target_port: u16, fps: u32) -> crate::error::Result<Output> {
	unsafe {
		let encoder: IMFTransform =
			CoCreateInstance(&windows::Win32::Media::MediaFoundation::CLSID_MSH264EncoderMFT, None, CLSCTX_ALL)?;

		let output_type = build_video_type(&MFVideoFormat_H264, fps, |mt| {
			mt.SetUINT32(&MF_MT_AVG_BITRATE, TARGET_BITRATE)?;
			mt.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_Base.0 as u32)?;
			Ok(())
		})?;
		encoder.SetOutputType(0, &output_type, 0)?;

		let input_type = build_video_type(&MFVideoFormat_NV12, fps, |_| Ok(()))?;
		encoder.SetInputType(0, &input_type, 0)?;

		let codec_api: ICodecAPI = encoder.cast()?;
		let _ = codec_api
			.SetValue(&CODECAPI_AVEncCommonRateControlMode, &VARIANT::from(eAVEncCommonRateControlMode_CBR.0 as u32));
		let _ = codec_api.SetValue(&CODECAPI_AVEncCommonMeanBitRate, &VARIANT::from(TARGET_BITRATE));

		encoder.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
		encoder.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;

		let tcp = connect_with_retry(target_port)?;

		Ok(Output { encoder, codec_api, tcp })
	}
}

fn connect_with_retry(target_port: u16) -> crate::error::Result<TcpStream> {
	let mut last_err = None;
	for attempt in 0..5 {
		match TcpStream::connect(("127.0.0.1", target_port)) {
			Ok(stream) => {
				let _ = stream.set_nodelay(true);
				return Ok(stream);
			}
			Err(e) => {
				last_err = Some(e);
				if attempt < 4 {
					std::thread::sleep(Duration::from_millis(200));
				}
			}
		}
	}
	Err(last_err.unwrap().into())
}

fn shutdown_session(session: &Session) {
	unsafe {
		let _ = session.output.encoder.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
		let _ = session.output.encoder.ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
	}
}

fn encode_loop(
	session: &mut Session,
	stop: &Arc<AtomicBool>,
	preferred_device: &Arc<Mutex<Option<String>>>,
	force_reopen: &Arc<AtomicBool>,
	force_keyframe: &Arc<AtomicBool>,
	on_black_screen: &mut dyn FnMut(bool),
) {
	let Session { capture, output } = session;
	match capture {
		Capture::Camera { reader, opened_symbolic_link } => {
			camera_loop(reader, opened_symbolic_link, output, stop, preferred_device, force_reopen, force_keyframe);
		}
		Capture::Screen(screen) => screen_loop(screen, output, stop, force_keyframe, on_black_screen),
	}
}

fn camera_loop(
	reader: &IMFSourceReader,
	opened_symbolic_link: &str,
	output: &mut Output,
	stop: &Arc<AtomicBool>,
	preferred_device: &Arc<Mutex<Option<String>>>,
	force_reopen: &Arc<AtomicBool>,
	force_keyframe: &Arc<AtomicBool>,
) {
	let mut last_drift_check = Instant::now();

	loop {
		if stop.load(Ordering::Relaxed) || force_reopen.load(Ordering::Relaxed) {
			return;
		}

		// Only "System Default" needs live-follow: an explicitly-picked
		// device doesn't drift, it just eventually fails ReadSample below if
		// it's unplugged, which already triggers a reopen.
		if last_drift_check.elapsed() >= Duration::from_secs(3) {
			last_drift_check = Instant::now();
			let is_default = preferred_device.lock().unwrap().is_none();
			if is_default
				&& let Some(current_default) = video_devices::resolve_camera_symbolic_link(None)
				&& current_default != opened_symbolic_link
			{
				return;
			}
		}

		{
			let current_pref = preferred_device.lock().unwrap().clone();
			// Reopen if the explicit selection changed underneath us.
			let device_matches = match &current_pref {
				Some(name) => {
					video_devices::resolve_camera_symbolic_link(Some(name)).as_deref() == Some(opened_symbolic_link)
				}
				None => true,
			};
			if !device_matches {
				return;
			}
		}

		let mut stream_index = 0u32;
		let mut flags = 0u32;
		let mut timestamp = 0i64;
		let mut sample: Option<IMFSample> = None;
		let read_result = unsafe {
			reader.ReadSample(
				MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
				0,
				Some(&mut stream_index),
				Some(&mut flags),
				Some(&mut timestamp),
				Some(&mut sample),
			)
		};
		if let Err(e) = read_result {
			log_line(&format!("ReadSample error (reopening): {e}"));
			return;
		}
		if flags & (MF_SOURCE_READERF_ENDOFSTREAM.0 as u32) != 0 {
			log_line("Camera source reported end of stream (reopening)");
			return;
		}
		let Some(captured) = sample else { continue };

		if let Err(e) = encode(output, &captured, force_keyframe) {
			log_line(&format!("Encoder error (reopening): {e}"));
			return;
		}
	}
}

/// Sends the screen at `SCREEN_FPS`. Unlike a camera there's no device to
/// follow or lose; a frame that can't be grabbed (the lock screen or a UAC
/// prompt is up) is skipped and the viewers keep the last picture.
/// `on_black_screen` hears when the screen starts or stops coming out black.
fn screen_loop(
	screen: &mut ScreenCapture,
	output: &mut Output,
	stop: &Arc<AtomicBool>,
	force_keyframe: &Arc<AtomicBool>,
	on_black_screen: &mut dyn FnMut(bool),
) {
	let frame_interval = Duration::from_nanos(1_000_000_000 / SCREEN_FPS as u64);
	let frame_duration_100ns = 10_000_000 / SCREEN_FPS as i64;
	let frame_bytes = (TARGET_WIDTH * TARGET_HEIGHT * 3 / 2) as usize;
	let started = Instant::now();
	let mut next_frame = started;
	let mut grab_failing = false;
	// A screen that's black for a moment (a video fading out, a full-screen
	// switch) isn't worth interrupting for; one that stays black is.
	let black_frames_to_report = SCREEN_FPS * 2;
	let mut black_run = 0u32;
	let mut reported_black = false;
	let mut sent_one = false;

	while !stop.load(Ordering::Relaxed) {
		let now = Instant::now();
		if now < next_frame {
			std::thread::sleep(next_frame - now);
		}
		// Never queue up frames to catch up after a slow one.
		next_frame = (next_frame + frame_interval).max(Instant::now());

		if !sent_one {
			log_line("Grabbing the first screen frame");
		}
		let sample = match screen_sample(screen, frame_bytes) {
			Ok((sample, black)) => {
				grab_failing = false;
				black_run = if black { black_run + 1 } else { 0 };
				if black_run == black_frames_to_report && !reported_black {
					reported_black = true;
					on_black_screen(true);
				} else if !black && reported_black {
					reported_black = false;
					on_black_screen(false);
				}
				sample
			}
			Err(e) => {
				// The window is gone: the session ends and says so.
				if screen.target_gone() {
					return;
				}
				if !grab_failing {
					log_line(&format!("Screen grab failed (holding the last picture): {e}"));
					grab_failing = true;
				}
				continue;
			}
		};
		if !sent_one {
			log_line("First screen frame grabbed, encoding it");
		}
		let stamped = unsafe {
			sample
				.SetSampleTime(started.elapsed().as_nanos() as i64 / 100)
				.and_then(|_| sample.SetSampleDuration(frame_duration_100ns))
		};
		if let Err(e) = stamped.map_err(crate::error::Error::from).and_then(|_| encode(output, &sample, force_keyframe))
		{
			log_line(&format!("Encoder error (reopening): {e}"));
			return;
		}
		if !sent_one {
			sent_one = true;
			log_line("First screen frame encoded");
		}
	}
}

/// Grabs the screen straight into a new Media Foundation sample's buffer, and
/// says whether what it got was a black screen.
fn screen_sample(screen: &mut ScreenCapture, frame_bytes: usize) -> crate::error::Result<(IMFSample, bool)> {
	unsafe {
		let buffer = MFCreateMemoryBuffer(frame_bytes as u32)?;
		let mut ptr: *mut u8 = std::ptr::null_mut();
		buffer.Lock(&mut ptr, None, None)?;
		let frame = std::slice::from_raw_parts_mut(ptr, frame_bytes);
		let grabbed = screen.grab_nv12(frame);
		let black = grabbed.is_ok() && screen_capture::is_black(frame, TARGET_WIDTH as usize, TARGET_HEIGHT as usize);
		let _ = buffer.Unlock();
		grabbed?;
		buffer.SetCurrentLength(frame_bytes as u32)?;
		let sample = MFCreateSample()?;
		sample.AddBuffer(&buffer)?;
		Ok((sample, black))
	}
}

/// Runs one captured frame through the encoder and sends whatever comes out.
fn encode(output: &mut Output, captured: &IMFSample, force_keyframe: &Arc<AtomicBool>) -> crate::error::Result<()> {
	if force_keyframe.swap(false, Ordering::Relaxed) {
		let _ = unsafe { output.codec_api.SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &VARIANT::from(true)) };
	}
	unsafe { output.encoder.ProcessInput(0, captured, 0)? };
	drain_encoder_output(output)
}

fn drain_encoder_output(output: &mut Output) -> crate::error::Result<()> {
	loop {
		let stream_info = unsafe { output.encoder.GetOutputStreamInfo(0)? };
		let out_sample = unsafe { MFCreateSample()? };
		let buffer = unsafe { MFCreateMemoryBuffer(stream_info.cbSize.max(1 << 20))? };
		unsafe { out_sample.AddBuffer(&buffer)? };

		let mut data = [MFT_OUTPUT_DATA_BUFFER {
			dwStreamID: 0,
			pSample: core::mem::ManuallyDrop::new(Some(out_sample)),
			dwStatus: 0,
			pEvents: core::mem::ManuallyDrop::new(None),
		}];
		let mut status = 0u32;
		let process_result = unsafe { output.encoder.ProcessOutput(0, &mut data, &mut status) };
		let produced = unsafe { core::mem::ManuallyDrop::take(&mut data[0].pSample) };
		let _events = unsafe { core::mem::ManuallyDrop::take(&mut data[0].pEvents) };

		match process_result {
			Ok(()) => {
				if let Some(produced) = produced {
					send_sample(output, &produced)?;
				}
			}
			Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(()),
			Err(e) => return Err(e.into()),
		}
	}
}

fn send_sample(output: &mut Output, sample: &IMFSample) -> crate::error::Result<()> {
	unsafe {
		let buffer = sample.ConvertToContiguousBuffer()?;
		let mut ptr: *mut u8 = std::ptr::null_mut();
		let mut len: u32 = 0;
		buffer.Lock(&mut ptr, None, Some(&mut len))?;
		let duration_100ns = sample.GetSampleDuration().unwrap_or(0).max(0);
		let duration_ms = (duration_100ns / 10_000) as u32;

		// Written straight from the locked buffer: copying the frame out and
		// then again behind its header was two copies of every frame.
		let mut header = [0u8; 8];
		header[..4].copy_from_slice(&len.to_be_bytes());
		header[4..].copy_from_slice(&duration_ms.to_be_bytes());
		let data = std::slice::from_raw_parts(ptr, len as usize);
		let written = output.tcp.write_all(&header).and_then(|_| output.tcp.write_all(data));
		let _ = buffer.Unlock();
		written?;
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use std::{io::Read, net::TcpListener};

	use super::*;

	/// Grabs the real screen and runs it through the real encoder, so it needs
	/// an unlocked desktop: `cargo test screen_share_sends -- --ignored`.
	#[test]
	#[ignore]
	fn screen_share_sends_h264_starting_with_a_keyframe() {
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let port = listener.local_addr().unwrap().port();
		let reader = std::thread::spawn(move || {
			let (mut conn, _) = listener.accept().unwrap();
			let mut frames = Vec::new();
			let mut header = [0u8; 8];
			while frames.len() < 5 && conn.read_exact(&mut header).is_ok() {
				let len = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
				let duration_ms = u32::from_be_bytes(header[4..].try_into().unwrap());
				let mut payload = vec![0u8; len];
				conn.read_exact(&mut payload).unwrap();
				frames.push((duration_ms, payload));
			}
			frames
		});

		unsafe {
			let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
		}
		media_foundation::ensure_started();
		let primary = ShareTarget::Monitor { left: 0, top: 0, width: 1920, height: 1080 };
		let mut session = open_session(port, VideoSource::Screen(primary), None).unwrap();
		let stop = Arc::new(AtomicBool::new(false));
		let stopper = Arc::clone(&stop);
		std::thread::spawn(move || {
			std::thread::sleep(Duration::from_secs(3));
			stopper.store(true, Ordering::Relaxed);
		});
		encode_loop(
			&mut session,
			&stop,
			&Arc::new(Mutex::new(None)),
			&Arc::new(AtomicBool::new(false)),
			&Arc::new(AtomicBool::new(false)),
			&mut |_| {},
		);
		shutdown_session(&session);
		drop(session);

		let frames = reader.join().unwrap();
		assert_eq!(frames.len(), 5, "the encoder produced too few frames in three seconds");
		let nal_types =
			|unit: &[u8]| -> Vec<u8> { unit.windows(4).filter(|w| w[..3] == [0, 0, 1]).map(|w| w[3] & 0x1f).collect() };
		let first = nal_types(&frames[0].1);
		assert!(first.contains(&7) && first.contains(&8) && first.contains(&5), "first unit NAL types: {first:?}");
		for (duration_ms, _) in &frames {
			assert_eq!(*duration_ms, 1000 / SCREEN_FPS, "frame duration");
		}
	}
}
