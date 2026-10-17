//! Captures the local webcam and encodes H.264 via Media Foundation, shipping
//! Annex-B access units to the backend's per-call TCP video bridge
//! (`backend/video_bridge.go`). Mirrors `call_audio.rs`'s WASAPI pattern: a
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

use crate::{media_foundation, paths, ui_wake::UiCommandSender, video_devices};

const TARGET_WIDTH: u32 = 1280;
const TARGET_HEIGHT: u32 = 720;
const TARGET_FPS: u32 = 30;
const TARGET_BITRATE: u32 = 1_500_000;
/// Screen sharing sends fewer frames: what's on a screen changes far less
/// often than a face, and every frame is a full-screen GDI copy.
const SCREEN_FPS: u64 = 15;

/// The "device" that makes the sender capture the main screen instead of a
/// camera (`set_device` with it starts a screen share, any other name goes
/// back to that camera).
pub(crate) const SCREEN_SOURCE: &str = "\u{1}screen";

pub(crate) fn log_line(msg: &str) {
	let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
	let line = format!("[{}.{:03}] {}\n", now.as_secs(), now.subsec_millis(), msg);
	if let Ok(mut f) =
		std::fs::OpenOptions::new().create(true).append(true).open(paths::log_dir().join("call_video_send.log"))
	{
		let _ = f.write_all(line.as_bytes());
	}
}

/// Handle to a running camera capture/encode/send session. Dropping it stops
/// the background thread; the thread notices on its own timing, same as
/// `call_audio::CallAudioState`.
pub struct CallVideoSendState {
	stop: Arc<AtomicBool>,
	preferred_device: Arc<Mutex<Option<String>>>,
	force_reopen: Arc<AtomicBool>,
	force_keyframe: Arc<AtomicBool>,
}

impl CallVideoSendState {
	/// Switches the camera in place, without dropping the running session
	/// wholesale from the caller's point of view - the capture thread notices
	/// the change and reopens on the new device itself.
	pub fn set_device(&self, name: Option<String>) {
		*self.preferred_device.lock().unwrap() = name;
	}

	/// Forces a reopen even though the preferred device name didn't change -
	/// used when "System Default" is selected and the OS's notion of the
	/// default camera has drifted (see the periodic check in `encode_loop`).
	pub fn force_device_refresh(&self) {
		self.force_reopen.store(true, Ordering::Relaxed);
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
/// very first attempt to open a camera fails outright (no camera present,
/// permission denied, etc), the thread reports it once via
/// `UiCommand::CameraFailed` and exits without retrying - there's nothing to
/// recover into. A camera that opens fine and is later lost (unplugged) does
/// retry, the same way `call_audio.rs` recovers from a lost audio device.
pub fn start_video_send(
	target_port: u16,
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

	std::thread::spawn(move || {
		run(target_port, thread_stop, thread_preferred, thread_force_reopen, thread_force_keyframe, ui_tx);
	});

	CallVideoSendState { stop, preferred_device, force_reopen, force_keyframe }
}

fn run(
	target_port: u16,
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

		match open_session(target_port, device_name.as_deref()) {
			Ok(mut session) => {
				ever_opened = true;
				encode_loop(&mut session, &stop, &preferred_device, &force_reopen, &force_keyframe);
				shutdown_session(&session);
			}
			Err(e) => {
				log_line(&format!("Failed to open camera session: {e}"));
				if !ever_opened {
					let _ = ui_tx.send(crate::UiCommand::CameraFailed(e.to_string()));
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
	/// The camera's reader, or None when sharing the screen.
	reader: Option<IMFSourceReader>,
	screen: Option<screen::ScreenGrabber>,
	encoder: IMFTransform,
	codec_api: ICodecAPI,
	tcp: TcpStream,
	opened_symbolic_link: String,
}

fn pack(high: u32, low: u32) -> u64 {
	media_foundation::pack_u64(high, low)
}

/// Finds the `IMFActivate` for the camera identified by `symbolic_link`,
/// re-enumerating every time - Media Foundation hands out fresh activate
/// objects per call and they can't usefully be cached across sessions.
unsafe fn activate_camera_source(
	symbolic_link: &str,
) -> anyhow::Result<windows::Win32::Media::MediaFoundation::IMFMediaSource> {
	unsafe {
		let mut attributes: Option<windows::Win32::Media::MediaFoundation::IMFAttributes> = None;
		MFCreateAttributes(&mut attributes, 1)?;
		let attributes = attributes.ok_or_else(|| anyhow::anyhow!("MFCreateAttributes returned no attributes"))?;
		attributes.SetGUID(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID)?;

		let mut activates: *mut Option<windows::Win32::Media::MediaFoundation::IMFActivate> = std::ptr::null_mut();
		let mut count: u32 = 0;
		MFEnumDeviceSources(&attributes, &mut activates, &mut count)?;
		if activates.is_null() {
			anyhow::bail!("no cameras available");
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
			anyhow::bail!("camera is no longer available");
		};
		Ok(activate.ActivateObject::<windows::Win32::Media::MediaFoundation::IMFMediaSource>()?)
	}
}

fn build_video_type(
	subtype: &GUID,
	extra: impl FnOnce(&IMFMediaType) -> anyhow::Result<()>,
) -> anyhow::Result<IMFMediaType> {
	unsafe {
		let mt = MFCreateMediaType()?;
		mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
		mt.SetGUID(&MF_MT_SUBTYPE, subtype)?;
		mt.SetUINT64(&MF_MT_FRAME_SIZE, pack(TARGET_WIDTH, TARGET_HEIGHT))?;
		mt.SetUINT64(&MF_MT_FRAME_RATE, pack(TARGET_FPS, 1))?;
		mt.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
		extra(&mt)?;
		Ok(mt)
	}
}

fn open_reader(symbolic_link: &str) -> anyhow::Result<IMFSourceReader> {
	unsafe {
		let source = activate_camera_source(symbolic_link)?;

		let mut reader_attrs: Option<windows::Win32::Media::MediaFoundation::IMFAttributes> = None;
		MFCreateAttributes(&mut reader_attrs, 1)?;
		let reader_attrs = reader_attrs.ok_or_else(|| anyhow::anyhow!("MFCreateAttributes returned no attributes"))?;
		reader_attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)?;
		let reader = MFCreateSourceReaderFromMediaSource(&source, &reader_attrs)?;

		let capture_type = build_video_type(&MFVideoFormat_NV12, |_| Ok(()))?;
		reader.SetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32, None, &capture_type)?;
		Ok(reader)
	}
}

fn open_session(target_port: u16, device_name: Option<&str>) -> anyhow::Result<Session> {
	let (reader, screen, symbolic_link) = if device_name == Some(SCREEN_SOURCE) {
		(None, Some(screen::ScreenGrabber::new(TARGET_WIDTH, TARGET_HEIGHT)?), SCREEN_SOURCE.to_string())
	} else {
		let Some(symbolic_link) = video_devices::resolve_camera_symbolic_link(device_name) else {
			anyhow::bail!("no camera available");
		};
		(Some(open_reader(&symbolic_link)?), None, symbolic_link)
	};

	unsafe {
		let encoder: IMFTransform =
			CoCreateInstance(&windows::Win32::Media::MediaFoundation::CLSID_MSH264EncoderMFT, None, CLSCTX_ALL)?;

		let output_type = build_video_type(&MFVideoFormat_H264, |mt| {
			mt.SetUINT32(&MF_MT_AVG_BITRATE, TARGET_BITRATE)?;
			mt.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_Base.0 as u32)?;
			Ok(())
		})?;
		encoder.SetOutputType(0, &output_type, 0)?;

		let input_type = build_video_type(&MFVideoFormat_NV12, |_| Ok(()))?;
		encoder.SetInputType(0, &input_type, 0)?;

		let codec_api: ICodecAPI = encoder.cast()?;
		let _ = codec_api
			.SetValue(&CODECAPI_AVEncCommonRateControlMode, &VARIANT::from(eAVEncCommonRateControlMode_CBR.0 as u32));
		let _ = codec_api.SetValue(&CODECAPI_AVEncCommonMeanBitRate, &VARIANT::from(TARGET_BITRATE));

		encoder.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
		encoder.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;

		let tcp = connect_with_retry(target_port)?;

		Ok(Session { reader, screen, encoder, codec_api, tcp, opened_symbolic_link: symbolic_link })
	}
}

fn connect_with_retry(target_port: u16) -> anyhow::Result<TcpStream> {
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
		let _ = session.encoder.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
		let _ = session.encoder.ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
	}
}

fn encode_loop(
	session: &mut Session,
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
			if is_default {
				if let Some(current_default) = video_devices::resolve_camera_symbolic_link(None) {
					if current_default != session.opened_symbolic_link {
						return;
					}
				}
			}
		}

		{
			let current_pref = preferred_device.lock().unwrap().clone();
			// Reopen if the explicit selection changed underneath us.
			let on_screen = session.opened_symbolic_link == SCREEN_SOURCE;
			let device_matches = match &current_pref {
				Some(name) if name == SCREEN_SOURCE => on_screen,
				Some(name) => {
					video_devices::resolve_camera_symbolic_link(Some(name)).as_deref()
						== Some(session.opened_symbolic_link.as_str())
				}
				None => !on_screen,
			};
			if !device_matches {
				return;
			}
		}

		let Some(reader) = &session.reader else {
			// Sharing the screen: one frame per tick, timed by us.
			let Some(screen) = session.screen.as_mut() else { return };
			std::thread::sleep(screen.until_next_frame(SCREEN_FPS));
			let captured = match screen.grab(SCREEN_FPS) {
				Ok(sample) => sample,
				Err(e) => {
					log_line(&format!("Screen capture error (reopening): {e}"));
					return;
				}
			};
			if force_keyframe.swap(false, Ordering::Relaxed) {
				let _ = unsafe { session.codec_api.SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &VARIANT::from(true)) };
			}
			if let Err(e) = unsafe { session.encoder.ProcessInput(0, &captured, 0) } {
				log_line(&format!("ProcessInput error (reopening): {e}"));
				return;
			}
			if let Err(e) = drain_encoder_output(session) {
				log_line(&format!("Encoder output error (reopening): {e}"));
				return;
			}
			continue;
		};

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

		if force_keyframe.swap(false, Ordering::Relaxed) {
			let _ = unsafe { session.codec_api.SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &VARIANT::from(true)) };
		}

		if let Err(e) = unsafe { session.encoder.ProcessInput(0, &captured, 0) } {
			log_line(&format!("ProcessInput error (reopening): {e}"));
			return;
		}

		if let Err(e) = drain_encoder_output(session) {
			log_line(&format!("Encoder output error (reopening): {e}"));
			return;
		}
	}
}

fn drain_encoder_output(session: &mut Session) -> anyhow::Result<()> {
	loop {
		let stream_info = unsafe { session.encoder.GetOutputStreamInfo(0)? };
		let out_sample = unsafe { MFCreateSample()? };
		let buffer = unsafe { MFCreateMemoryBuffer(stream_info.cbSize.max(1 << 20))? };
		unsafe { out_sample.AddBuffer(&buffer)? };

		let mut output = [MFT_OUTPUT_DATA_BUFFER {
			dwStreamID: 0,
			pSample: core::mem::ManuallyDrop::new(Some(out_sample)),
			dwStatus: 0,
			pEvents: core::mem::ManuallyDrop::new(None),
		}];
		let mut status = 0u32;
		let process_result = unsafe { session.encoder.ProcessOutput(0, &mut output, &mut status) };
		let produced = unsafe { core::mem::ManuallyDrop::take(&mut output[0].pSample) };
		let _events = unsafe { core::mem::ManuallyDrop::take(&mut output[0].pEvents) };

		match process_result {
			Ok(()) => {
				if let Some(produced) = produced {
					send_sample(session, &produced)?;
				}
			}
			Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(()),
			Err(e) => return Err(e.into()),
		}
	}
}

fn send_sample(session: &mut Session, sample: &IMFSample) -> anyhow::Result<()> {
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
		let written = session.tcp.write_all(&header).and_then(|_| session.tcp.write_all(data));
		let _ = buffer.Unlock();
		written?;
	}
	Ok(())
}

/// The main screen as NV12 frames, scaled (keeping its shape) into the
/// call's frame size with GDI: a DIB section the screen is StretchBlt'd
/// into, converted by hand. The primary monitor only.
mod screen {
	use std::time::{Duration, Instant};

	use windows::Win32::{
		Graphics::Gdi::{
			BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLACKNESS, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
			DeleteDC, DeleteObject, GdiFlush, GetDC, HALFTONE, HBITMAP, HDC, HGDIOBJ, PatBlt, ReleaseDC, SRCCOPY,
			SelectObject, SetBrushOrgEx, SetStretchBltMode, StretchBlt,
		},
		Media::MediaFoundation::{IMFSample, MFCreateMemoryBuffer, MFCreateSample},
		UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN},
	};

	pub(super) struct ScreenGrabber {
		width: u32,
		height: u32,
		screen_dc: HDC,
		mem_dc: HDC,
		bitmap: HBITMAP,
		old: HGDIOBJ,
		bits: *const u8,
		frames: u64,
		started: Instant,
	}

	impl ScreenGrabber {
		pub(super) fn new(width: u32, height: u32) -> anyhow::Result<Self> {
			unsafe {
				let screen_dc = GetDC(None);
				if screen_dc.is_invalid() {
					anyhow::bail!("couldn't get the screen");
				}
				let mem_dc = CreateCompatibleDC(Some(screen_dc));
				let info = BITMAPINFO {
					bmiHeader: BITMAPINFOHEADER {
						biSize: size_of::<BITMAPINFOHEADER>() as u32,
						biWidth: width as i32,
						// Negative: rows top to bottom.
						biHeight: -(height as i32),
						biPlanes: 1,
						biBitCount: 32,
						biCompression: BI_RGB.0,
						..Default::default()
					},
					..Default::default()
				};
				let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
				let bitmap = match CreateDIBSection(Some(mem_dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
					Ok(bitmap) if !bits.is_null() => bitmap,
					_ => {
						let _ = DeleteDC(mem_dc);
						ReleaseDC(None, screen_dc);
						anyhow::bail!("couldn't make a bitmap for the screen");
					}
				};
				let old = SelectObject(mem_dc, bitmap.into());
				SetStretchBltMode(mem_dc, HALFTONE);
				let _ = SetBrushOrgEx(mem_dc, 0, 0, None);
				Ok(Self {
					width,
					height,
					screen_dc,
					mem_dc,
					bitmap,
					old,
					bits: bits as *const u8,
					frames: 0,
					started: Instant::now(),
				})
			}
		}

		pub(super) fn until_next_frame(&self, fps: u64) -> Duration {
			let due = Duration::from_micros(self.frames * 1_000_000 / fps);
			due.saturating_sub(self.started.elapsed())
		}

		pub(super) fn grab(&mut self, fps: u64) -> anyhow::Result<IMFSample> {
			let (w, h) = (self.width as i32, self.height as i32);
			unsafe {
				let sw = GetSystemMetrics(SM_CXSCREEN).max(1);
				let sh = GetSystemMetrics(SM_CYSCREEN).max(1);
				// Fit the screen in the frame, bars where its shape differs.
				let (mut dw, mut dh) = (w, (sh as i64 * w as i64 / sw as i64) as i32);
				if dh > h {
					dh = h;
					dw = (sw as i64 * h as i64 / sh as i64) as i32;
				}
				let (dx, dy) = ((w - dw) / 2, (h - dh) / 2);
				if dw != w || dh != h {
					let _ = PatBlt(self.mem_dc, 0, 0, w, h, BLACKNESS);
				}
				if !StretchBlt(self.mem_dc, dx, dy, dw, dh, Some(self.screen_dc), 0, 0, sw, sh, SRCCOPY).as_bool() {
					anyhow::bail!("couldn't copy the screen");
				}
				let _ = GdiFlush();

				let len = (self.width * self.height * 3 / 2) as usize;
				let buffer = MFCreateMemoryBuffer(len as u32)?;
				let mut ptr: *mut u8 = std::ptr::null_mut();
				buffer.Lock(&mut ptr, None, None)?;
				let bgra = std::slice::from_raw_parts(self.bits, (self.width * self.height * 4) as usize);
				bgra_to_nv12(bgra, std::slice::from_raw_parts_mut(ptr, len), self.width as usize, self.height as usize);
				buffer.Unlock()?;
				buffer.SetCurrentLength(len as u32)?;

				let sample = MFCreateSample()?;
				sample.AddBuffer(&buffer)?;
				let duration = 10_000_000 / fps as i64;
				sample.SetSampleTime(self.frames as i64 * duration)?;
				sample.SetSampleDuration(duration)?;
				self.frames += 1;
				Ok(sample)
			}
		}
	}

	impl Drop for ScreenGrabber {
		fn drop(&mut self) {
			unsafe {
				SelectObject(self.mem_dc, self.old);
				let _ = DeleteObject(self.bitmap.into());
				let _ = DeleteDC(self.mem_dc);
				ReleaseDC(None, self.screen_dc);
			}
		}
	}

	/// BT.601 limited range, as the camera path's NV12 is: a Y per pixel,
	/// then a U/V pair per 2x2 block from the block's average colour.
	fn bgra_to_nv12(bgra: &[u8], out: &mut [u8], width: usize, height: usize) {
		let (luma, chroma) = out.split_at_mut(width * height);
		for (i, px) in bgra.chunks_exact(4).enumerate() {
			let (b, g, r) = (px[0] as i32, px[1] as i32, px[2] as i32);
			luma[i] = (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16) as u8;
		}
		for y in (0..height).step_by(2) {
			for x in (0..width).step_by(2) {
				let (mut r, mut g, mut b) = (0i32, 0i32, 0i32);
				for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
					let o = ((y + dy).min(height - 1) * width + (x + dx).min(width - 1)) * 4;
					b += bgra[o] as i32;
					g += bgra[o + 1] as i32;
					r += bgra[o + 2] as i32;
				}
				let (r, g, b) = (r / 4, g / 4, b / 4);
				let o = (y / 2) * width + x;
				chroma[o] = (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128) as u8;
				chroma[o + 1] = (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128) as u8;
			}
		}
	}

	#[cfg(test)]
	mod tests {
		use super::bgra_to_nv12;

		#[test]
		fn white_and_black() {
			let white = [255u8; 2 * 2 * 4];
			let mut out = [0u8; 6];
			bgra_to_nv12(&white, &mut out, 2, 2);
			assert_eq!(out, [235, 235, 235, 235, 128, 128]);
			let black = [0u8; 2 * 2 * 4];
			bgra_to_nv12(&black, &mut out, 2, 2);
			assert_eq!(out, [16, 16, 16, 16, 128, 128]);
		}
	}
}
