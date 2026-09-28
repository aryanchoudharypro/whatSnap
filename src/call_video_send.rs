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
	reader: IMFSourceReader,
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

fn open_session(target_port: u16, device_name: Option<&str>) -> anyhow::Result<Session> {
	let Some(symbolic_link) = video_devices::resolve_camera_symbolic_link(device_name) else {
		anyhow::bail!("no camera available");
	};

	unsafe {
		let source = activate_camera_source(&symbolic_link)?;

		let mut reader_attrs: Option<windows::Win32::Media::MediaFoundation::IMFAttributes> = None;
		MFCreateAttributes(&mut reader_attrs, 1)?;
		let reader_attrs = reader_attrs.ok_or_else(|| anyhow::anyhow!("MFCreateAttributes returned no attributes"))?;
		reader_attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)?;
		let reader = MFCreateSourceReaderFromMediaSource(&source, &reader_attrs)?;

		let capture_type = build_video_type(&MFVideoFormat_NV12, |_| Ok(()))?;
		reader.SetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32, None, &capture_type)?;

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

		Ok(Session { reader, encoder, codec_api, tcp, opened_symbolic_link: symbolic_link })
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
			let device_matches = match &current_pref {
				Some(name) => {
					video_devices::resolve_camera_symbolic_link(Some(name)).as_deref()
						== Some(session.opened_symbolic_link.as_str())
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
			session.reader.ReadSample(
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
