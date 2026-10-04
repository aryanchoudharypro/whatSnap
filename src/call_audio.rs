use std::{
	collections::VecDeque,
	net::UdpSocket,
	sync::{
		Arc, Mutex,
		atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
		mpsc,
	},
	time::Duration,
};

use windows::{
	Win32::{
		Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0},
		Media::{
			Audio::{
				AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
				AUDCLNT_STREAMOPTIONS_NONE, AudioCategory_Communications, AudioClientProperties, IAudioCaptureClient,
				IAudioClient2, IAudioRenderClient, IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEXTENSIBLE,
				eCapture, eCommunications, eRender,
			},
			KernelStreaming::WAVE_FORMAT_EXTENSIBLE,
			Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT},
		},
		System::{
			Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize},
			Threading::{CreateEventW, WaitForSingleObject},
		},
	},
	core::PCWSTR,
};

use crate::paths;

// Every frame on the wire to/from the Go backend is 60 ms of 16 kHz mono PCM
// (meowcaller.FrameSamples) - see backend/audio_udp.go.
const FRAME_SAMPLES: usize = 960;
const NETWORK_SAMPLE_RATE: u32 = 16000;

// Each mic frame also goes out as one standard Opus packet, after its PCM in
// the same datagram. The backend encodes MLow itself from the PCM; the Opus
// copy is for a call with someone whose WhatsApp isn't on MLow (WhatsApp for
// Windows, for one) and decodes Opus instead, which the backend can't encode.
// 24 kbit/s at complexity 5 is what the official clients use at 16 kHz.
const OPUS_BITRATE: i32 = 24_000;
const OPUS_COMPLEXITY: i32 = 5;
// The largest packet libopus can produce for one frame.
const OPUS_MAX_PACKET: usize = 1275;

// Debugging aid for call-audio issues reported in the field (e.g. one-sided
// silence). Appends timestamped lines to call_audio.log alongside the main
// backend.log; safe to call from any thread, but never from a WASAPI
// event-loop iteration that's about to block on I/O under time pressure -
// only from plain background threads and stream-lifecycle points.
pub(crate) fn log_line(msg: &str) {
	use std::io::Write;
	let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
	let line = format!("[{}.{:03}] {}\n", now.as_secs(), now.subsec_millis(), msg);
	if let Ok(mut f) =
		std::fs::OpenOptions::new().create(true).append(true).open(paths::log_dir().join("call_audio.log"))
	{
		let _ = f.write_all(line.as_bytes());
	}
}

// On Windows, a UDP socket that has sent a datagram to a port with no
// listener can later fail recvfrom() with WSAECONNRESET when the OS
// delivers the resulting ICMP Port Unreachable back to it - a purely local
// artifact that has nothing to do with whether the actual call is still up,
// but was previously killing the audio reader thread outright (see below).
// SIO_UDP_CONNRESET with a FALSE input buffer disables that behavior for
// this socket, matching how the OS itself recommends handling this for
// UDP sockets that expect to keep listening regardless of ICMP replies.
fn disable_udp_connreset(socket: &UdpSocket) {
	use std::os::windows::io::AsRawSocket;

	use windows::Win32::Networking::WinSock::{SOCKET, WSAIoctl};
	const SIO_UDP_CONNRESET: u32 = 0x9800000C;
	let sock = SOCKET(socket.as_raw_socket() as usize);
	let input: u32 = 0; // FALSE - disable connection-reset reporting
	let mut bytes_returned: u32 = 0;
	unsafe {
		let _ = WSAIoctl(
			sock,
			SIO_UDP_CONNRESET,
			Some(&input as *const u32 as *const core::ffi::c_void),
			std::mem::size_of::<u32>() as u32,
			None,
			0,
			&mut bytes_returned,
			None,
			None,
		);
	}
}

// Closes the WASAPI event handle when a capture/render thread exits, however it exits.
pub(crate) struct EventGuard(pub(crate) HANDLE);
impl Drop for EventGuard {
	fn drop(&mut self) {
		unsafe {
			let _ = CloseHandle(self.0);
		}
	}
}

// Balances a per-thread CoInitializeEx: each capture/render thread owns its WASAPI COM
// objects and must uninitialize COM on the same thread before exiting, on every path
// (including early '?' returns from a failed setup step).
pub(crate) struct ComGuard;
impl Drop for ComGuard {
	fn drop(&mut self) {
		unsafe { CoUninitialize() };
	}
}

pub struct CallAudioState {
	pub is_muted: Arc<AtomicBool>,
	stop: Arc<AtomicBool>,
	preferred_input: Arc<Mutex<Option<String>>>,
	preferred_output: Arc<Mutex<Option<String>>>,
	/// Set by `force_device_refresh` to make the capture/render poll loops
	/// reopen even though `preferred_input`/`preferred_output` didn't change -
	/// needed for "System Default" (or a pinned device that just vanished):
	/// the live Windows default-device-changed notification (see
	/// `audio_device_watch.rs`) fires with the *name* unchanged from this
	/// call's point of view, so the ordinary change-detection below would
	/// otherwise never notice. Separate flags per direction - each session
	/// loop clears its own via `swap`, so one loop polling first can't eat
	/// the signal the other loop still needs to see.
	force_reopen_input: Arc<AtomicBool>,
	force_reopen_output: Arc<AtomicBool>,
}

impl CallAudioState {
	/// Switches the capture device for this call in place, without dropping
	/// or restarting the call. The capture session loop (see
	/// `run_capture_session`) polls this value on the same cadence it
	/// already polls for stream errors, and treats a change as a request to
	/// close the current WASAPI stream and reopen a fresh one on the new
	/// device - the same recovery path used when a device is lost, just
	/// triggered deliberately instead of by an error.
	pub fn set_input_device(&self, name: Option<String>) {
		*self.preferred_input.lock().unwrap() = name;
	}

	/// Same as `set_input_device`, for the render (speaker) side.
	pub fn set_output_device(&self, name: Option<String>) {
		*self.preferred_output.lock().unwrap() = name;
	}

	/// Reopens both capture and render on their currently-resolved device
	/// (system default, or a pinned device re-resolved in case the old one
	/// came back or is still gone) without waiting for a name change.
	pub fn force_device_refresh(&self) {
		self.force_reopen_input.store(true, Ordering::Relaxed);
		self.force_reopen_output.store(true, Ordering::Relaxed);
	}
}

impl Drop for CallAudioState {
	fn drop(&mut self) {
		// Signals the background capture/render/reader threads (see
		// start_audio_streams) to exit within their poll timeout instead of
		// blocking forever. Not joined: WASAPI shared-mode streams don't need
		// exclusive access, so a brief overlap between an old call's teardown
		// and a new call's startup is harmless.
		self.stop.store(true, Ordering::Relaxed);
	}
}

// Opens a WASAPI shared-mode stream on the eCommunications-role default (or, if
// `preferred_device_name` names a still-present device, that specific) device for
// `dataflow` (eCapture or eRender). When `apply_effects` is set, the stream is also
// tagged with the Communications audio-session category, which is what makes Windows
// apply its own echo cancellation/noise-suppression/AGC audio-effects pipeline to the
// stream - the previous cpal-based implementation used the plain default multimedia-role
// devices with no session category set, which never engaged any of that, so
// speakerphone/laptop mic+speaker setups got raw, unprocessed audio (echo, room noise)
// end to end. Calls always pass `apply_effects: true`; voice-message recording passes
// false and does its own clean-up instead (voice_dsp.rs).
pub(crate) struct WasapiOpen {
	pub(crate) client: IAudioClient2,
	pub(crate) event: HANDLE,
	pub(crate) buffer_frames: u32,
	pub(crate) sample_rate: u32,
	pub(crate) channels: u16,
	pub(crate) bits_per_sample: u16,
	pub(crate) is_float: bool,
}

pub(crate) unsafe fn open_communications_stream(
	dataflow: windows::Win32::Media::Audio::EDataFlow,
	tag: &str,
	preferred_device_name: Option<&str>,
	apply_effects: bool,
) -> anyhow::Result<WasapiOpen> {
	unsafe {
		let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
		// Falls back to the default communications-role device if no preference is
		// set, or if the preferred device (unplugged, renamed, etc) can't be found -
		// same behavior as before this became user-selectable.
		let device = match preferred_device_name
			.and_then(|name| crate::audio_devices::find_wasapi_device_by_name(&enumerator, dataflow, name))
		{
			Some(device) => device,
			None => enumerator.GetDefaultAudioEndpoint(dataflow, eCommunications)?,
		};
		let client: IAudioClient2 = device.Activate(CLSCTX_ALL, None)?;

		if apply_effects {
			let props = AudioClientProperties {
				cbSize: std::mem::size_of::<AudioClientProperties>() as u32,
				bIsOffload: false.into(),
				eCategory: AudioCategory_Communications,
				Options: AUDCLNT_STREAMOPTIONS_NONE,
			};
			if let Err(e) = client.SetClientProperties(&props) {
				log_line(&format!("{tag}: SetClientProperties(Communications) failed, continuing without it: {:?}", e));
			}
		}

		// GetMixFormat's returned pointer may point at a WAVEFORMATEXTENSIBLE (larger
		// than WAVEFORMATEX) - only the pointer itself, still owning the full
		// allocation, may be handed to Initialize; a truncated local copy would make
		// Initialize read past a stack value if the format's cbSize claims extension
		// bytes that aren't actually there.
		let mix_format_ptr = client.GetMixFormat()?;
		let format = *mix_format_ptr;
		let channels = format.nChannels;
		let sample_rate = format.nSamplesPerSec;
		let bits_per_sample = format.wBitsPerSample;
		let format_tag = format.wFormatTag as u32;
		// wFormatTag alone doesn't say PCM vs float for WAVE_FORMAT_EXTENSIBLE (which is what
		// GetMixFormat virtually always returns) - that's carried in the extended struct's
		// SubFormat GUID instead. The previous `bits_per_sample == 32 && format_tag !=
		// WAVE_FORMAT_PCM` heuristic would misread a 32-bit-PCM extensible mix format (some
		// pro-audio interfaces/drivers report this) as float, feeding raw PCM integer bytes
		// through the f32 path as garbage/noise audio. read_unaligned is required here: the
		// windows crate's WAVEFORMATEXTENSIBLE is `repr(packed)`, so a plain deref/cast could
		// read a misaligned field.
		let is_float = match format_tag {
			t if t == WAVE_FORMAT_IEEE_FLOAT => true,
			t if t == WAVE_FORMAT_EXTENSIBLE => {
				let ext = std::ptr::read_unaligned(mix_format_ptr as *const WAVEFORMATEXTENSIBLE);
				// `ext.SubFormat == ...` would implicitly take `&ext.SubFormat` to call
				// PartialEq::eq - a reference to a field of a repr(packed) struct, which is a
				// hard compile error (E0793) since GUID's natural alignment (4) can exceed the
				// struct's packed alignment (1). Block-copying the field out first (`{ ext.SubFormat }`)
				// reads it by value with no reference ever formed.
				let subformat = { ext.SubFormat };
				subformat == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
			}
			_ => false,
		};

		let init_result = client.Initialize(
			AUDCLNT_SHAREMODE_SHARED,
			AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
			200_000,
			0,
			mix_format_ptr,
			None,
		);
		CoTaskMemFree(Some(mix_format_ptr as *const core::ffi::c_void));
		init_result?;

		log_line(&format!(
			"{tag}: opened communications-role device: rate={} channels={} bits={} format_tag={}",
			sample_rate, channels, bits_per_sample, format_tag
		));

		let event = CreateEventW(None, false, false, PCWSTR::null())?;
		client.SetEventHandle(event)?;
		let buffer_frames = client.GetBufferSize()?;

		Ok(WasapiOpen { client, event, buffer_frames, sample_rate, channels, bits_per_sample, is_float })
	}
}

// Mic-side DSP: mono downmix -> anti-aliasing low-pass -> linear-interpolation
// resample to 16 kHz -> batched into FRAME_SAMPLES packets sent to Go. Identical to
// the filter design previously run inside cpal's input callback; only the buffer
// source (a WASAPI capture-thread loop instead of a cpal callback) changed.
struct CaptureDsp {
	channels: u16,
	lp1: f32,
	lp2: f32,
	lp_alpha: f32,
	mono_accum: Vec<f32>,
	resample_phase: f64,
	downsample_step: f64,
	sample_accum: Vec<f32>,
	/// One frame's samples, then the datagram built from it; both reused so
	/// the capture thread doesn't allocate every 20ms.
	frame: Vec<f32>,
	send_buf: Vec<u8>,
	/// None if libopus refused to start: frames then go out as PCM alone.
	opus: Option<opus::Encoder>,
}

fn new_call_opus_encoder() -> Option<opus::Encoder> {
	let mut encoder = opus::Encoder::new(NETWORK_SAMPLE_RATE, opus::Channels::Mono, opus::Application::Voip).ok()?;
	encoder.set_bitrate(opus::Bitrate::Bits(OPUS_BITRATE)).ok()?;
	encoder.set_complexity(OPUS_COMPLEXITY).ok()?;
	Some(encoder)
}

/// Builds one datagram for the backend: the frame as little-endian f32 PCM,
/// then the same frame as a standard Opus packet when there's an encoder.
fn pack_call_frame(frame: &[f32], opus: Option<&mut opus::Encoder>, out: &mut Vec<u8>) {
	out.clear();
	for sample in frame {
		out.extend_from_slice(&sample.to_le_bytes());
	}
	let Some(encoder) = opus else { return };
	let pcm_len = out.len();
	out.resize(pcm_len + OPUS_MAX_PACKET, 0);
	let encoded = encoder.encode_float(frame, &mut out[pcm_len..]).unwrap_or(0);
	out.truncate(pcm_len + encoded);
}

impl CaptureDsp {
	fn new(sample_rate: u32, channels: u16) -> Self {
		// The linear-interpolation resampler has no anti-aliasing of its own:
		// decimating a 44.1/48kHz mic signal straight to 16kHz folds everything
		// above the 8kHz output Nyquist back into the audible band. A cascaded
		// two-pole one-pole low-pass at ~7kHz (comfortably under 8kHz) run on the
		// native-rate signal before decimation removes that content first.
		//
		// Cascading two identical one-pole stages does NOT put the pair's -3dB point
		// at the per-stage cutoff: each stage alone is already -3dB there, so the pair
		// is -6dB, and the actual combined -3dB point lands at cutoff * sqrt(sqrt(2)-1)
		// (~0.644x) - using 7kHz directly for both stages put the real rolloff around
		// 4.5kHz, cutting into sibilance/formants and making voice sound muffled. Scale
		// the per-stage cutoff up so the cascade's own -3dB point is the intended 7kHz.
		let lp_target_hz: f32 = 7000.0;
		let lp_cutoff_hz: f32 = lp_target_hz / (2.0_f32.sqrt() - 1.0).sqrt();
		let lp_rc = 1.0 / (2.0 * std::f32::consts::PI * lp_cutoff_hz);
		let lp_dt = 1.0 / sample_rate as f32;
		let lp_alpha = lp_dt / (lp_rc + lp_dt);
		Self {
			channels,
			lp1: 0.0,
			lp2: 0.0,
			lp_alpha,
			mono_accum: Vec::new(),
			resample_phase: 0.0,
			downsample_step: sample_rate as f64 / NETWORK_SAMPLE_RATE as f64,
			sample_accum: Vec::with_capacity(FRAME_SAMPLES * 2),
			frame: Vec::with_capacity(FRAME_SAMPLES),
			send_buf: Vec::with_capacity(FRAME_SAMPLES * 4 + OPUS_MAX_PACKET),
			opus: new_call_opus_encoder(),
		}
	}

	fn process(&mut self, data: &[f32], muted: bool, socket_out: &UdpSocket, target_addr: &str) {
		for frame in data.chunks(self.channels as usize) {
			let sum: f32 = frame.iter().sum();
			let mono = sum / (self.channels as f32);
			self.lp1 += self.lp_alpha * (mono - self.lp1);
			self.lp2 += self.lp_alpha * (self.lp1 - self.lp2);
			self.mono_accum.push(self.lp2);
		}

		while self.resample_phase.floor() as usize + 1 < self.mono_accum.len() {
			let idx = self.resample_phase.floor() as usize;
			let frac = (self.resample_phase - idx as f64) as f32;
			let sample = self.mono_accum[idx] + (self.mono_accum[idx + 1] - self.mono_accum[idx]) * frac;
			self.sample_accum.push(if muted { 0.0 } else { sample });
			self.resample_phase += self.downsample_step;
		}
		let consumed = (self.resample_phase.floor() as usize).min(self.mono_accum.len());
		if consumed > 0 {
			self.mono_accum.drain(0..consumed);
			self.resample_phase -= consumed as f64;
		}

		while self.sample_accum.len() >= FRAME_SAMPLES {
			self.frame.clear();
			self.frame.extend(self.sample_accum.drain(..FRAME_SAMPLES));
			pack_call_frame(&self.frame, self.opus.as_mut(), &mut self.send_buf);
			let _ = socket_out.send_to(&self.send_buf, target_addr);
		}
	}
}

// Speaker-side DSP: pulls 16 kHz mono frames off frame_rx into a jitter-absorbing
// queue, then linear-interpolation upsamples to the device's native rate on demand.
// Identical to the filter design previously run inside cpal's output callback.
struct RenderDsp {
	channels: u16,
	src_accum: VecDeque<f32>,
	resample_phase: f64,
	upsample_step: f64,
	max_buffered_src: usize,
	played: Arc<AtomicU64>,
	starved: Arc<AtomicU64>,
	accum_len: Arc<AtomicUsize>,
}

impl RenderDsp {
	fn new(
		sample_rate: u32,
		channels: u16,
		played: Arc<AtomicU64>,
		starved: Arc<AtomicU64>,
		accum_len: Arc<AtomicUsize>,
	) -> Self {
		Self {
			channels,
			src_accum: VecDeque::new(),
			resample_phase: 0.0,
			upsample_step: NETWORK_SAMPLE_RATE as f64 / sample_rate as f64,
			max_buffered_src: FRAME_SAMPLES * 8,
			played,
			starved,
			accum_len,
		}
	}

	fn process(&mut self, data: &mut [f32], frame_rx: &mpsc::Receiver<Vec<f32>>) {
		let channels = self.channels as usize;
		let out_frames = data.len() / channels;
		// `needed` must be scaled by upsample_step: this call will only actually
		// consume out_frames*upsample_step source samples this round (advancing
		// resample_phase by exactly that much below), not one source sample per
		// output frame.
		let needed =
			self.resample_phase.floor() as usize + 1 + (out_frames as f64 * self.upsample_step).ceil() as usize;
		while self.src_accum.len() < needed {
			match frame_rx.try_recv() {
				Ok(frame) => self.src_accum.extend(frame),
				Err(_) => break,
			}
		}
		while self.src_accum.len() > self.max_buffered_src {
			self.src_accum.pop_front();
		}

		for out_frame in data.chunks_mut(channels) {
			let idx = self.resample_phase.floor() as usize;
			let sample = if idx + 1 < self.src_accum.len() {
				let frac = (self.resample_phase - idx as f64) as f32;
				self.src_accum[idx] + (self.src_accum[idx + 1] - self.src_accum[idx]) * frac
			} else if idx < self.src_accum.len() {
				self.src_accum[idx]
			} else {
				// Nothing buffered at this position - play silence and, critically, do
				// NOT advance resample_phase past what's actually in src_accum (see the
				// cpal-era version of this comment for why: it's what previously caused
				// playback to go silent for the rest of a call after any starvation).
				self.starved.fetch_add(1, Ordering::Relaxed);
				0.0
			};
			if idx < self.src_accum.len() {
				self.played.fetch_add(1, Ordering::Relaxed);
			}
			for slot in out_frame.iter_mut() {
				*slot = sample;
			}
			if (idx as f64) < self.src_accum.len() as f64 {
				self.resample_phase += self.upsample_step;
			}
		}
		let consumed = (self.resample_phase.floor() as usize).min(self.src_accum.len());
		if consumed > 0 {
			self.src_accum.drain(0..consumed);
			self.resample_phase -= consumed as f64;
		}
		if self.src_accum.is_empty() && self.resample_phase > 0.0 {
			self.resample_phase = 0.0;
		}
		self.accum_len.store(self.src_accum.len(), Ordering::Relaxed);
	}
}

// Runs capture sessions back-to-back until `stop` is set. A single session ending in
// error (any WASAPI call failing mid-stream, e.g. the communications-role device
// changing or being temporarily reclaimed) used to permanently and silently kill mic
// capture for the rest of the call, since every WASAPI call in the old single-pass
// loop was a bare `?` with no caller to retry it. Reopening the device and starting a
// fresh session is the same recovery WASAPI apps are expected to perform on
// AUDCLNT_E_DEVICE_INVALIDATED and similar - there's no other way to recover a live
// IAudioClient once the OS has torn down its underlying endpoint.
fn run_capture(
	stop: &AtomicBool,
	is_muted: &AtomicBool,
	socket_out: &UdpSocket,
	target_addr: &str,
	preferred_device: &Mutex<Option<String>>,
	force_reopen: &AtomicBool,
) {
	while !stop.load(Ordering::Relaxed) {
		if let Err(e) = run_capture_session(stop, is_muted, socket_out, target_addr, preferred_device, force_reopen) {
			log_line(&format!("capture session ended with error, reopening: {:?}", e));
			if stop.load(Ordering::Relaxed) {
				break;
			}
			std::thread::sleep(Duration::from_millis(500));
		}
	}
	log_line("capture thread exiting (stop set)");
}

fn run_capture_session(
	stop: &AtomicBool,
	is_muted: &AtomicBool,
	socket_out: &UdpSocket,
	target_addr: &str,
	preferred_device: &Mutex<Option<String>>,
	force_reopen: &AtomicBool,
) -> anyhow::Result<()> {
	unsafe {
		CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
		let _com_guard = ComGuard;

		// Snapshot the requested device for this session; a change made
		// through CallAudioState::set_input_device while this session is
		// running is picked up by the poll loop below, not here.
		let session_device = preferred_device.lock().unwrap().clone();
		// A refresh requested while we were still opening the previous
		// session belongs to this new session's open, not a reopen after it.
		force_reopen.store(false, Ordering::Relaxed);
		let wasapi = open_communications_stream(eCapture, "capture", session_device.as_deref(), true)?;
		let _event_guard = EventGuard(wasapi.event);
		let capture_client: IAudioCaptureClient = wasapi.client.GetService()?;

		let mut dsp = CaptureDsp::new(wasapi.sample_rate, wasapi.channels);
		let mut scratch: Vec<f32> = Vec::new();

		wasapi.client.Start()?;
		log_line("capture stream started (communications role)");

		while !stop.load(Ordering::Relaxed) {
			if preferred_device.lock().unwrap().as_ref() != session_device.as_ref() {
				log_line("capture: device switch requested, reopening");
				break;
			}
			if force_reopen.swap(false, Ordering::Relaxed) {
				log_line("capture: default device changed, reopening");
				break;
			}
			if WaitForSingleObject(wasapi.event, 200) != WAIT_OBJECT_0 {
				continue;
			}
			loop {
				let packet_len = capture_client.GetNextPacketSize()?;
				if packet_len == 0 {
					break;
				}
				let mut data_ptr: *mut u8 = std::ptr::null_mut();
				let mut num_frames: u32 = 0;
				let mut flags: u32 = 0;
				capture_client.GetBuffer(&mut data_ptr, &mut num_frames, &mut flags, None, None)?;

				let silent = data_ptr.is_null() || (flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0;
				let total_samples = num_frames as usize * wasapi.channels as usize;
				scratch.clear();
				if silent {
					scratch.resize(total_samples, 0.0);
				} else if wasapi.is_float {
					scratch.extend_from_slice(std::slice::from_raw_parts(data_ptr as *const f32, total_samples));
				} else if wasapi.bits_per_sample == 16 {
					let src = std::slice::from_raw_parts(data_ptr as *const i16, total_samples);
					scratch.extend(src.iter().map(|&s| s as f32 / 32768.0));
				} else {
					scratch.resize(total_samples, 0.0);
				}

				let muted = is_muted.load(Ordering::Relaxed);
				dsp.process(&scratch, muted, socket_out, target_addr);

				capture_client.ReleaseBuffer(num_frames)?;
			}
		}

		wasapi.client.Stop()?;
		log_line("capture session exiting (stop set)");
	}
	Ok(())
}

// Same back-to-back-sessions recovery as run_capture, and for the same reason: any
// WASAPI call failing mid-stream used to permanently and silently kill speaker
// playback for the rest of the call.
fn run_render(
	stop: &AtomicBool,
	frame_rx: mpsc::Receiver<Vec<f32>>,
	played: Arc<AtomicU64>,
	starved: Arc<AtomicU64>,
	accum_len: Arc<AtomicUsize>,
	preferred_device: &Mutex<Option<String>>,
	force_reopen: &AtomicBool,
) {
	while !stop.load(Ordering::Relaxed) {
		if let Err(e) =
			run_render_session(stop, &frame_rx, &played, &starved, &accum_len, preferred_device, force_reopen)
		{
			log_line(&format!("render session ended with error, reopening: {:?}", e));
			if stop.load(Ordering::Relaxed) {
				break;
			}
			std::thread::sleep(Duration::from_millis(500));
		}
	}
	log_line("render thread exiting (stop set)");
}

fn run_render_session(
	stop: &AtomicBool,
	frame_rx: &mpsc::Receiver<Vec<f32>>,
	played: &Arc<AtomicU64>,
	starved: &Arc<AtomicU64>,
	accum_len: &Arc<AtomicUsize>,
	preferred_device: &Mutex<Option<String>>,
	force_reopen: &AtomicBool,
) -> anyhow::Result<()> {
	unsafe {
		CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
		let _com_guard = ComGuard;

		// See run_capture_session: a later device switch is picked up by the
		// poll loop below, not by re-reading this snapshot.
		let session_device = preferred_device.lock().unwrap().clone();
		force_reopen.store(false, Ordering::Relaxed);
		let wasapi = open_communications_stream(eRender, "render", session_device.as_deref(), true)?;
		let _event_guard = EventGuard(wasapi.event);
		let render_client: IAudioRenderClient = wasapi.client.GetService()?;

		// Pre-fill the whole buffer with silence before Start(), the standard WASAPI
		// render pattern - starting with an empty buffer glitches the first callback.
		{
			let data_ptr = render_client.GetBuffer(wasapi.buffer_frames)?;
			if !data_ptr.is_null() {
				let bytes =
					wasapi.buffer_frames as usize * wasapi.channels as usize * (wasapi.bits_per_sample as usize / 8);
				std::ptr::write_bytes(data_ptr, 0, bytes);
			}
			render_client.ReleaseBuffer(wasapi.buffer_frames, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)?;
		}

		let mut dsp =
			RenderDsp::new(wasapi.sample_rate, wasapi.channels, played.clone(), starved.clone(), accum_len.clone());
		let mut scratch: Vec<f32> = Vec::new();

		wasapi.client.Start()?;
		log_line("render stream started (communications role)");

		while !stop.load(Ordering::Relaxed) {
			if preferred_device.lock().unwrap().as_ref() != session_device.as_ref() {
				log_line("render: device switch requested, reopening");
				break;
			}
			if force_reopen.swap(false, Ordering::Relaxed) {
				log_line("render: default device changed, reopening");
				break;
			}
			if WaitForSingleObject(wasapi.event, 200) != WAIT_OBJECT_0 {
				continue;
			}
			let padding = wasapi.client.GetCurrentPadding()?;
			let available = wasapi.buffer_frames.saturating_sub(padding);
			if available == 0 {
				continue;
			}

			let data_ptr = render_client.GetBuffer(available)?;
			let total_samples = available as usize * wasapi.channels as usize;
			scratch.clear();
			scratch.resize(total_samples, 0.0);
			dsp.process(&mut scratch, frame_rx);

			if !data_ptr.is_null() {
				if wasapi.is_float {
					std::slice::from_raw_parts_mut(data_ptr as *mut f32, total_samples).copy_from_slice(&scratch);
				} else if wasapi.bits_per_sample == 16 {
					let dst = std::slice::from_raw_parts_mut(data_ptr as *mut i16, total_samples);
					for (d, s) in dst.iter_mut().zip(scratch.iter()) {
						*d = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
					}
				}
			}
			render_client.ReleaseBuffer(available, 0)?;
		}

		wasapi.client.Stop()?;
		log_line("render session exiting (stop set)");
	}
	Ok(())
}

pub fn start_audio_streams(
	target_port: u16,
	preferred_input_device: Option<String>,
	preferred_output_device: Option<String>,
) -> anyhow::Result<CallAudioState> {
	crate::paths::rotate_log("call_audio.log");
	let socket = UdpSocket::bind("127.0.0.1:0")?;
	let target_addr = format!("127.0.0.1:{}", target_port);

	// Send a dummy packet to punch the hole for Go to learn our port!
	socket.send_to(&[0u8; 4], &target_addr)?;

	let socket_read = socket.try_clone()?;
	let socket_out = socket.try_clone()?;

	// The output side must never block on a network read on a real-time audio
	// thread; a plain background thread blocks on the socket and hands decoded
	// frames to the render thread through a small bounded channel, which the
	// render loop only ever drains with a non-blocking try_recv.
	let (frame_tx, frame_rx) = mpsc::sync_channel::<Vec<f32>>(8);
	let stop = Arc::new(AtomicBool::new(false));
	let reader_stop_thread = Arc::clone(&stop);

	let reader_recv_count = Arc::new(AtomicU64::new(0));
	let reader_error_count = Arc::new(AtomicU64::new(0));
	let tx_drop_count = Arc::new(AtomicU64::new(0));
	let output_played_samples = Arc::new(AtomicU64::new(0));
	let output_starved_samples = Arc::new(AtomicU64::new(0));
	let output_src_accum_len = Arc::new(AtomicUsize::new(0));

	let reader_recv_count_thread = Arc::clone(&reader_recv_count);
	let reader_error_count_thread = Arc::clone(&reader_error_count);
	let tx_drop_count_thread = Arc::clone(&tx_drop_count);

	log_line(&format!("start_audio_streams: target_port={}", target_port));

	std::thread::spawn(move || {
		let _ = socket_read.set_read_timeout(Some(Duration::from_millis(200)));
		disable_udp_connreset(&socket_read);
		// Sized to the max possible UDP datagram, not FRAME_SAMPLES * 4: on Windows,
		// recv_from on an undersized buffer returns WSAEMSGSIZE and silently drops the
		// *whole* datagram (no partial read like POSIX gives you), so a relay/peer that
		// occasionally bursts more samples per packet than we send ourselves was having
		// those packets discarded outright, causing choppy/starved audio for that call.
		let mut buf = vec![0u8; 65536];
		while !reader_stop_thread.load(Ordering::Relaxed) {
			match socket_read.recv_from(&mut buf) {
				Ok((size, _)) => {
					reader_recv_count_thread.fetch_add(1, Ordering::Relaxed);
					let frame: Vec<f32> =
						buf[..size].chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
					// A full channel means playback is falling behind; drop
					// the frame rather than block this thread's UDP reads.
					if frame_tx.try_send(frame).is_err() {
						tx_drop_count_thread.fetch_add(1, Ordering::Relaxed);
					}
				}
				Err(ref e)
					if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {}
				// Any other recv error (notably WSAECONNRESET on Windows, which a
				// loopback UDP socket can get handed after any datagram it sent found
				// no listener at that instant - pure local plumbing noise, unrelated
				// to whether the call/relay is still up) used to permanently kill
				// this thread, which dropped frame_tx and silenced playback for the
				// rest of the call even though nothing about the connection had
				// changed. Keep listening instead; only the explicit stop flag above
				// should end this loop.
				Err(e) => {
					reader_error_count_thread.fetch_add(1, Ordering::Relaxed);
					log_line(&format!("reader recv_from error (ignored, continuing): {:?}", e));
				}
			}
		}
		log_line("reader thread exiting (reader_stop set)");
	});

	// Periodic health snapshot so silence/one-sided-audio reports can be
	// correlated against real counters instead of guessing: udp_recv tells
	// us whether packets are even arriving, tx_drop tells us if the reader
	// is receiving but playback can't keep up, and starved/played tell us
	// whether the render loop actually had data to play.
	{
		let health_stop = Arc::clone(&stop);
		let reader_recv_count = Arc::clone(&reader_recv_count);
		let reader_error_count = Arc::clone(&reader_error_count);
		let tx_drop_count = Arc::clone(&tx_drop_count);
		let output_played_samples = Arc::clone(&output_played_samples);
		let output_starved_samples = Arc::clone(&output_starved_samples);
		let output_src_accum_len = Arc::clone(&output_src_accum_len);
		std::thread::spawn(move || {
			while !health_stop.load(Ordering::Relaxed) {
				std::thread::sleep(Duration::from_secs(2));
				if health_stop.load(Ordering::Relaxed) {
					break;
				}
				log_line(&format!(
					"health: udp_recv={} reader_err={} tx_drop={} played={} starved={} src_accum_len={}",
					reader_recv_count.load(Ordering::Relaxed),
					reader_error_count.load(Ordering::Relaxed),
					tx_drop_count.load(Ordering::Relaxed),
					output_played_samples.load(Ordering::Relaxed),
					output_starved_samples.load(Ordering::Relaxed),
					output_src_accum_len.load(Ordering::Relaxed),
				));
			}
		});
	}

	let is_muted = Arc::new(AtomicBool::new(false));
	let preferred_input = Arc::new(Mutex::new(preferred_input_device));
	let preferred_output = Arc::new(Mutex::new(preferred_output_device));
	// One shared flag for both directions: a default-device-changed
	// notification doesn't say which direction changed, so refresh both -
	// reopening a session that turns out to already be on the right device
	// is cheap and harmless.
	let force_reopen_input = Arc::new(AtomicBool::new(false));
	let force_reopen_output = Arc::new(AtomicBool::new(false));

	{
		let stop = Arc::clone(&stop);
		let is_muted = Arc::clone(&is_muted);
		let target_addr = target_addr.clone();
		let preferred_input = Arc::clone(&preferred_input);
		let force_reopen_input = Arc::clone(&force_reopen_input);
		std::thread::spawn(move || {
			run_capture(&stop, &is_muted, &socket_out, &target_addr, &preferred_input, &force_reopen_input);
		});
	}

	{
		let stop = Arc::clone(&stop);
		let preferred_output = Arc::clone(&preferred_output);
		let force_reopen_output = Arc::clone(&force_reopen_output);
		std::thread::spawn(move || {
			run_render(
				&stop,
				frame_rx,
				output_played_samples,
				output_starved_samples,
				output_src_accum_len,
				&preferred_output,
				&force_reopen_output,
			);
		});
	}

	Ok(CallAudioState { is_muted, stop, preferred_input, preferred_output, force_reopen_input, force_reopen_output })
}

#[cfg(test)]
mod tests {
	use super::*;

	fn tone_frame(offset: usize) -> Vec<f32> {
		(0..FRAME_SAMPLES)
			.map(|i| {
				let t = (offset + i) as f32 / NETWORK_SAMPLE_RATE as f32;
				0.4 * (2.0 * std::f32::consts::PI * 220.0 * t).sin()
			})
			.collect()
	}

	#[test]
	fn a_call_frame_carries_its_pcm_then_one_60ms_opus_packet() {
		let mut encoder = new_call_opus_encoder().expect("libopus encoder");
		let mut decoder = opus::Decoder::new(NETWORK_SAMPLE_RATE, opus::Channels::Mono).expect("libopus decoder");
		let mut datagram = Vec::new();
		let mut energy = 0.0f32;
		for n in 0..10 {
			let frame = tone_frame(n * FRAME_SAMPLES);
			pack_call_frame(&frame, Some(&mut encoder), &mut datagram);

			let (pcm, packet) = datagram.split_at(FRAME_SAMPLES * 4);
			let sent: Vec<f32> = pcm.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
			assert_eq!(sent, frame, "the PCM half is the frame, untouched");

			// What the other side's WhatsApp expects on this payload type: one
			// packet for the whole 60 ms, wideband (the official clients send
			// SILK wideband, TOC 0x58).
			assert!(!packet.is_empty(), "frame {n} has no Opus packet");
			assert_eq!(opus::packet::get_nb_samples(packet, NETWORK_SAMPLE_RATE).unwrap(), FRAME_SAMPLES);
			assert_eq!(packet[0], 0x58, "frame {n} is not one 60 ms SILK wideband frame");

			let mut decoded = vec![0.0f32; FRAME_SAMPLES];
			assert_eq!(decoder.decode_float(packet, &mut decoded, false).unwrap(), FRAME_SAMPLES);
			if n >= 2 {
				energy += decoded.iter().map(|s| s * s).sum::<f32>();
			}
		}
		assert!(energy > 1.0, "the Opus packets decode back to the tone, not silence");
	}

	#[test]
	fn a_call_frame_without_an_encoder_is_pcm_alone() {
		let mut datagram = vec![1, 2, 3];
		pack_call_frame(&tone_frame(0), None, &mut datagram);
		assert_eq!(datagram.len(), FRAME_SAMPLES * 4);
	}
}
