use std::{
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
				AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
				AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMOPTIONS_NONE,
				AudioCategory_Communications, AudioClientProperties, Endpoints::IAudioEndpointVolume,
				IAudioCaptureClient, IAudioClient2, IAudioRenderClient, IMMDevice, IMMDeviceEnumerator,
				MMDeviceEnumerator, WAVEFORMATEXTENSIBLE, eCapture, eConsole, eRender,
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

use crate::{
	audio::{
		call_voice::{AGC_TARGET_DBFS, CallVoice, EchoReference, GATE_DEPTH_DB, VoiceAgc},
		limiter::Limiter,
		system_audio::{SystemAudioCapture, SystemMix},
	},
	platform::paths,
	ui::wake::UiCommandSender,
};

const FRAME_SAMPLES: usize = 960;
const NETWORK_SAMPLE_RATE: u32 = 16000;

const OPUS_BITRATE: i32 = 24_000;
const OPUS_COMPLEXITY: i32 = 5;
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

fn disable_udp_connreset(socket: &UdpSocket) {
	use std::os::windows::io::AsRawSocket;

	use windows::Win32::Networking::WinSock::{SOCKET, WSAIoctl};
	const SIO_UDP_CONNRESET: u32 = 0x9800000C;
	let sock = SOCKET(socket.as_raw_socket() as usize);
	let input: u32 = 0;
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
	/// `audio/device_watch.rs`) fires with the *name* unchanged from this
	/// call's point of view, so the ordinary change-detection below would
	/// otherwise never notice. Separate flags per direction - each session
	/// loop clears its own via `swap`, so one loop polling first can't eat
	/// the signal the other loop still needs to see.
	force_reopen_input: Arc<AtomicBool>,
	force_reopen_output: Arc<AtomicBool>,
	force_reopen_echo: Arc<AtomicBool>,
	system_mix: Arc<Mutex<SystemMix>>,
	system_audio: Option<SystemAudioCapture>,
}

impl CallAudioState {
	pub fn set_input_device(&self, name: Option<String>) {
		*self.preferred_input.lock().unwrap() = name;
	}

	pub fn set_output_device(&self, name: Option<String>) {
		*self.preferred_output.lock().unwrap() = name;
	}

	pub fn force_device_refresh(&self) {
		self.force_reopen_input.store(true, Ordering::Relaxed);
		self.force_reopen_output.store(true, Ordering::Relaxed);
		self.force_reopen_echo.store(true, Ordering::Relaxed);
	}

	pub fn share_system_audio(&mut self, on: bool, ui_tx: &UiCommandSender) {
		if on == self.system_audio.is_some() {
			return;
		}
		if on {
			self.system_audio = Some(crate::audio::system_audio::start(Arc::clone(&self.system_mix), ui_tx.clone()));
		} else {
			self.system_audio = None;
			self.system_mix.lock().unwrap().clear();
		}
	}

	pub fn sharing_system_audio(&self) -> bool {
		self.system_audio.is_some()
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

// Opens a shared-mode WASAPI stream on the system default (eConsole) or named device.
// `apply_effects` tags the session as Communications for Windows AEC/NS/AGC; `loopback` captures eRender playback.
pub(crate) struct WasapiOpen {
	pub(crate) device: IMMDevice,
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
	loopback: bool,
) -> crate::error::Result<WasapiOpen> {
	unsafe {
		let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
		let device = match preferred_device_name
			.and_then(|name| crate::audio::devices::find_wasapi_device_by_name(&enumerator, dataflow, name))
		{
			Some(device) => device,
			None => enumerator.GetDefaultAudioEndpoint(dataflow, eConsole)?,
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

		let mix_format_ptr = client.GetMixFormat()?;
		let format = *mix_format_ptr;
		let channels = format.nChannels;
		let sample_rate = format.nSamplesPerSec;
		let bits_per_sample = format.wBitsPerSample;
		let format_tag = format.wFormatTag as u32;
		let is_float = match format_tag {
			t if t == WAVE_FORMAT_IEEE_FLOAT => true,
			t if t == WAVE_FORMAT_EXTENSIBLE => {
				let ext = std::ptr::read_unaligned(mix_format_ptr as *const WAVEFORMATEXTENSIBLE);
				let subformat = { ext.SubFormat };
				subformat == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
			}
			_ => false,
		};

		let init_result = client.Initialize(
			AUDCLNT_SHAREMODE_SHARED,
			if loopback {
				AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_LOOPBACK
			} else {
				AUDCLNT_STREAMFLAGS_EVENTCALLBACK
			},
			200_000,
			0,
			mix_format_ptr,
			None,
		);
		CoTaskMemFree(Some(mix_format_ptr as *const core::ffi::c_void));
		init_result?;

		log_line(&format!(
			"{tag}: opened WASAPI device: rate={} channels={} bits={} format_tag={}",
			sample_rate, channels, bits_per_sample, format_tag
		));

		let event = CreateEventW(None, false, false, PCWSTR::null())?;
		client.SetEventHandle(event)?;
		let buffer_frames = client.GetBufferSize()?;

		Ok(WasapiOpen { device, client, event, buffer_frames, sample_rate, channels, bits_per_sample, is_float })
	}
}

// Downmix to mono and resample to NETWORK_SAMPLE_RATE through a Blackman-
// windowed sinc low-pass, so voice keeps its top end up to about 7.5 kHz
// and what lies above 8 kHz is removed instead of folding back as harshness
// (two one-pole filters used to start rolling off from about 3 kHz and let
// much of 8-12 kHz alias). The kernel is tabulated at DS_PHASES points per
// input sample and read with linear interpolation, which handles any device
// rate; each output is normalised by its weights so the gain stays at 1.
const DS_CUTOFF_HZ: f64 = 7500.0;
const DS_TRANSITION_HZ: f64 = 1000.0;
const DS_PHASES: usize = 64;

pub(crate) struct Downsampler {
	channels: u16,
	half: usize,
	table: Vec<f32>,
	history: Vec<f32>,
	pos: f64,
	step: f64,
}

impl Downsampler {
	pub(crate) fn new(sample_rate: u32, channels: u16) -> Self {
		Self::with_output(sample_rate, channels, NETWORK_SAMPLE_RATE, DS_CUTOFF_HZ)
	}

	/// The same filter for another output rate and cutoff (the 48 kHz
	/// high band of a super-wideband call, calls/swb_uplink.rs).
	pub(crate) fn with_output(sample_rate: u32, channels: u16, out_rate: u32, cutoff_hz: f64) -> Self {
		let rate = sample_rate as f64;
		let cutoff = (cutoff_hz / rate).min(0.45);
		// A Blackman window's transition band is about 5.5 / length.
		let half = ((5.5 * rate / DS_TRANSITION_HZ) / 2.0).ceil().max(4.0) as usize;
		let table = (0..=half * DS_PHASES)
			.map(|i| {
				let t = i as f64 / DS_PHASES as f64;
				let x = 2.0 * cutoff * t;
				let sinc = if x == 0.0 { 1.0 } else { (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x) };
				let w = std::f64::consts::PI * (t / half as f64 + 1.0);
				let window = 0.42 - 0.5 * w.cos() + 0.08 * (2.0 * w).cos();
				(sinc * window) as f32
			})
			.collect();
		Self {
			channels,
			half,
			table,
			// Start with a window of silence so the first output can be made.
			history: vec![0.0; half],
			pos: half as f64,
			step: rate / out_rate as f64,
		}
	}

	fn tap(&self, distance: f64) -> f32 {
		let x = distance * DS_PHASES as f64;
		let i = x as usize;
		if i + 1 >= self.table.len() {
			return 0.0;
		}
		let frac = (x - i as f64) as f32;
		self.table[i] + (self.table[i + 1] - self.table[i]) * frac
	}

	pub(crate) fn process(&mut self, data: &[f32], out: &mut Vec<f32>) {
		let channels = self.channels.max(1) as usize;
		self.history.extend(data.chunks(channels).map(|frame| frame.iter().sum::<f32>() / channels as f32));

		while self.pos.floor() as usize + self.half < self.history.len() {
			let centre = self.pos.floor() as usize;
			let (mut acc, mut weights) = (0.0f32, 0.0f32);
			for n in centre + 1 - self.half..=centre + self.half {
				let w = self.tap((self.pos - n as f64).abs());
				acc += w * self.history[n];
				weights += w;
			}
			out.push(if weights > 0.0 { acc / weights } else { 0.0 });
			self.pos += self.step;
		}
		let consumed = (self.pos.floor() as usize + 1).saturating_sub(self.half).min(self.history.len());
		if consumed > 0 {
			self.history.drain(..consumed);
			self.pos -= consumed as f64;
		}
	}
}

struct CaptureDsp {
	downsampler: Downsampler,
	resampled: Vec<f32>,
	echo_reference: Arc<Mutex<EchoReference>>,
	played: Vec<f32>,
	voice: CallVoice,
	cleaned: Vec<f32>,
	/// Left and right together, so a stereo call keeps its image.
	limiter: Limiter<2>,
	sample_accum: Vec<f32>,
	/// The side ((left - right) / 2) of each sample in `sample_accum`: zero
	/// unless the call goes out in stereo.
	side_accum: Vec<f32>,
	system: Arc<Mutex<SystemMix>>,
	frame: Vec<f32>,
	send_buf: Vec<u8>,
	opus: Option<crate::audio::libopus::Encoder>,
	/// The packet-loss percentage last given to `opus`.
	applied_loss_percent: i32,
	slices_since_mic_check: u32,
	/// Mic checks since the last echo diagnostics line.
	checks_since_echo_report: u32,
	logged_echo_delay: Option<u32>,
	device_rate: u32,
	device_channels: u16,
	/// The call's audio rate `opus` was made for.
	send_rate: u32,
	/// Our voice above 8 kHz, while the call runs super-wideband.
	high_band: Option<crate::calls::swb_uplink::HighBand>,
	wide_frame: Vec<f32>,
	muted_scratch: Vec<f32>,
	dtx: bool,
	/// Device frames in a row that were digital silence while unmuted.
	silent_frames: u32,
	/// A stereo mic's side, at 16 kHz in step with `resampled`, while a raw
	/// call may go out in stereo.
	side_downsampler: Option<Downsampler>,
	side_resampled: Vec<f32>,
	voice_side: Vec<f32>,
	/// `opus` is a stereo encoder, and whether it sends stereo now.
	stereo_encoder: bool,
	stereo_on: bool,
	/// 16 kHz samples stereo stays on after its reason went away, so a gap in
	/// the shared sound doesn't flip the encoder back and forth.
	stereo_hold: usize,
	stereo_frame: Vec<f32>,
}

/// Settings > Input & Output "Send stereo in calls (experimental)": on a
/// 16 kHz call the encoder sends stereo while there is something stereo to
/// send (shared screen sound, or a stereo mic with all processing off).
/// WhatsApp's own apps decode it to mono; whatSnap plays it in stereo.
static CALL_STEREO_SEND: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn set_call_stereo_send(on: bool) {
	if CALL_STEREO_SEND.swap(on, Ordering::Relaxed) != on {
		log_line(&format!("call audio: stereo sending {}", if on { "allowed" } else { "off" }));
	}
}

/// Stereo bitrate: wideband stereo needs about a third more than mono voice.
const OPUS_STEREO_BITRATE: i32 = 32_000;
const STEREO_HOLD_SAMPLES: usize = NETWORK_SAMPLE_RATE as usize * 2;

/// Settings > Input & Output "Turn off all sound processing in calls": the mic
/// goes out as it comes in (no echo canceller, noise removal, auto gain, pause
/// gate, limiter or mic level changes) and the peer's Opus is decoded plainly
/// (no Deep PLC, NoLACE or bandwidth extension). Read every round, so it takes
/// effect mid-call.
static CALL_RAW_AUDIO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn set_call_raw_audio(on: bool) {
	if CALL_RAW_AUDIO.swap(on, Ordering::Relaxed) != on {
		log_line(&format!("call audio: all processing {}", if on { "off (raw)" } else { "on" }));
	}
}

fn call_raw_audio() -> bool {
	CALL_RAW_AUDIO.load(Ordering::Relaxed)
}

/// Mic health, like WhatsApp's mic health warning: the capture thread notices
/// a microphone that delivers nothing but digital silence while we're not
/// muted (muted in Windows, a privacy switch, a broken device), and when sound
/// comes back. The call window's timer takes the change (take_mic_health_change).
static MIC_SILENT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static MIC_HEALTH_CHANGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Peak below which a device sample counts as silence (about -100 dBFS): no
/// real microphone is this quiet, even in a silent room.
const MIC_SILENCE_PEAK: f32 = 1e-5;
const MIC_SILENT_AFTER_SECS: u32 = 5;

/// Some(true) once the mic has gone silent, Some(false) once it's back, None
/// when nothing changed since the last call.
pub(crate) fn take_mic_health_change() -> Option<bool> {
	MIC_HEALTH_CHANGED.swap(false, Ordering::Relaxed).then(|| MIC_SILENT.load(Ordering::Relaxed))
}

fn set_mic_silent(silent: bool) {
	if MIC_SILENT.swap(silent, Ordering::Relaxed) != silent {
		MIC_HEALTH_CHANGED.store(true, Ordering::Relaxed);
		log_line(if silent {
			"mic health: the microphone has delivered only silence for 5 s"
		} else {
			"mic health: the microphone picks up sound again"
		});
	}
}

/// How much of our audio the peer says never arrived, in percent, as the
/// backend last reported it.
static CALL_PACKET_LOSS_PERCENT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// The loss the encoder plans for is capped, like WhatsApp's `max_plr_to_opus`:
/// past this, more redundancy costs more quality than it saves.
const MAX_PLANNED_LOSS_PERCENT: i32 = 30;

pub(crate) fn set_call_packet_loss_percent(percent: i32) {
	CALL_PACKET_LOSS_PERCENT.store(percent.clamp(0, MAX_PLANNED_LOSS_PERCENT), Ordering::Relaxed);
}

fn new_call_opus_encoder() -> Option<crate::audio::libopus::Encoder> {
	// WhatsApp's own encoder starts at 0% expected loss and follows what the
	// peer's reports measure, see `set_call_packet_loss_percent`.
	CALL_PACKET_LOSS_PERCENT.store(0, Ordering::Relaxed);
	CALL_SEND_RATE.store(NETWORK_SAMPLE_RATE, Ordering::Relaxed);
	CALL_SEND_DTX.store(false, Ordering::Relaxed);
	MIC_SILENT.store(false, Ordering::Relaxed);
	MIC_HEALTH_CHANGED.store(false, Ordering::Relaxed);
	call_opus_encoder(NETWORK_SAMPLE_RATE, false)
}

/// Whether the call's encoder runs with DTX, as the backend last said (the
/// server's rc.dtx through meowcaller's Call.DTX). The backend leaves the
/// silent frames out of the call.
static CALL_SEND_DTX: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The call's audio rate as the backend last said (meowcaller's
/// Call.AudioRate): 16000, or 24000 or 48000 once both sides advertised
/// super-wideband Opus.
static CALL_SEND_RATE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(NETWORK_SAMPLE_RATE);

/// Bitrates for super-wideband and fullband voice. WhatsApp's own targets are
/// for 16 kHz (25 kbit/s at its best); the wider bands need more to be worth it.
const OPUS_SWB_BITRATE: i32 = 32_000;
const OPUS_FB_BITRATE: i32 = 40_000;

/// An encoder for a call at `rate`. Above 16 kHz it takes 48 kHz input
/// (calls/swb_uplink.rs) and is capped to the band the call agreed on.
/// `stereo` (16 kHz only) makes a stereo encoder that sends stereo packets.
fn call_opus_encoder(rate: u32, stereo: bool) -> Option<crate::audio::libopus::Encoder> {
	let wide = rate > NETWORK_SAMPLE_RATE;
	let stereo = stereo && !wide;
	let mut encoder = crate::audio::libopus::Encoder::new(
		if wide { 48_000 } else { NETWORK_SAMPLE_RATE },
		if stereo { crate::audio::libopus::Channels::Stereo } else { crate::audio::libopus::Channels::Mono },
		crate::audio::libopus::Application::Voip,
	)
	.ok()?;
	if stereo {
		encoder.set_force_channels(2).ok()?;
	}
	let bitrate = call_bitrate(rate, CALL_PACKET_LOSS_PERCENT.load(Ordering::Relaxed), stereo);
	encoder.set_bitrate(crate::audio::libopus::Bitrate::Bits(bitrate)).ok()?;
	encoder.set_complexity(OPUS_COMPLEXITY).ok()?;
	if wide {
		let _ = encoder.set_max_bandwidth_for_rate(rate);
	}
	let _ = encoder.set_inband_fec(true);
	let _ = encoder.set_packet_loss_perc(CALL_PACKET_LOSS_PERCENT.load(Ordering::Relaxed));
	let _ = encoder.set_dtx(CALL_SEND_DTX.load(Ordering::Relaxed));
	Some(encoder)
}

/// The call's audio bitrate, stepped down while the peer reports heavy loss
/// of our audio, the way WhatsApp's adaptive audio quality (UAQC) drops from
/// its high-quality state (about 25 kbit/s) toward its low data rate (rc
/// low_data_usage_bitrate 14000) when the network struggles: a congested link
/// loses less when sent less, and the encoder's FEC gets a bigger share.
fn call_bitrate(rate: u32, loss_percent: i32, stereo: bool) -> i32 {
	let full = match rate {
		48_000.. => OPUS_FB_BITRATE,
		24_000.. => OPUS_SWB_BITRATE,
		_ if stereo => OPUS_STEREO_BITRATE,
		_ => OPUS_BITRATE,
	};
	match loss_percent {
		20.. => full * 3 / 5,
		10.. => full * 4 / 5,
		_ => full,
	}
}

// The limiter after the mix: 2 ms lookahead, a little under full scale.
const CALL_LIMIT_LOOKAHEAD: usize = 32;
const CALL_LIMIT_CEILING: f32 = 0.9;
const CALL_LIMIT_RELEASE_SECS: f32 = 0.08;

// The other half of AGC2: its input volume controller turns the Windows mic
// level itself up when speech stays quiet and down when the input clips, as
// WhatsApp for Windows' engine and Chrome (for WA Web) both do. Checked about
// once a second from the capture thread. The level the user had is put back
// when the capture session ends, unless they moved the slider themselves
// during the call, in which case we stop touching it.
const MIC_STEP: f32 = 0.04;
const MIC_CHECK_SLICES: u32 = 100; // 1 s

struct MicLevelController {
	volume: IAudioEndpointVolume,
	original: f32,
	ours: f32,
	user_took_over: bool,
}

impl MicLevelController {
	unsafe fn new(device: &IMMDevice) -> Option<Self> {
		unsafe {
			let volume: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None).ok()?;
			let original = volume.GetMasterVolumeLevelScalar().ok()?;
			log_line(&format!("mic level controller: starting at {:.2}", original));
			Some(Self { volume, original, ours: original, user_took_over: false })
		}
	}

	fn adjust(&mut self, agc: &mut VoiceAgc) {
		let speech = std::mem::take(&mut agc.speech_slices);
		let quiet = std::mem::take(&mut agc.quiet_speech_slices);
		let clipped = std::mem::take(&mut agc.clipped_slices);
		if self.user_took_over {
			return;
		}
		let Ok(now) = (unsafe { self.volume.GetMasterVolumeLevelScalar() }) else { return };
		if (now - self.ours).abs() > 0.02 {
			self.user_took_over = true;
			log_line(&format!("mic level controller: slider moved to {:.2} by the user, leaving it alone", now));
			return;
		}
		let next = if clipped >= 3 {
			(now - MIC_STEP).max(0.05)
		} else if speech >= 20 && quiet * 2 > speech {
			(now + MIC_STEP).min(1.0)
		} else {
			return;
		};
		if (next - now).abs() < 0.001 {
			return;
		}
		if unsafe { self.volume.SetMasterVolumeLevelScalar(next, std::ptr::null()) }.is_ok() {
			self.ours = next;
		}
	}
}

impl Drop for MicLevelController {
	fn drop(&mut self) {
		if self.user_took_over || (self.ours - self.original).abs() < 0.001 {
			return;
		}
		unsafe {
			let _ = self.volume.SetMasterVolumeLevelScalar(self.original, std::ptr::null());
		}
		log_line(&format!("mic level controller: put the mic back to {:.2} (was at {:.2})", self.original, self.ours));
	}
}

/// The 16 kHz frame, then the same 60 ms as one Opus packet: encoded from
/// `wide` (48 kHz) on a super-wideband call, from `wide` as interleaved left
/// and right for a stereo encoder, else from the frame itself.
fn pack_call_frame(
	frame: &[f32],
	wide: Option<&[f32]>,
	opus: Option<&mut crate::audio::libopus::Encoder>,
	out: &mut Vec<u8>,
) {
	out.clear();
	for sample in frame {
		out.extend_from_slice(&sample.to_le_bytes());
	}
	let Some(encoder) = opus else { return };
	let pcm_len = out.len();
	out.resize(pcm_len + OPUS_MAX_PACKET, 0);
	let encoded = encoder.encode_float(wide.unwrap_or(frame), &mut out[pcm_len..]).unwrap_or(0);
	out.truncate(pcm_len + encoded);
}

impl CaptureDsp {
	fn new(
		sample_rate: u32,
		channels: u16,
		system: Arc<Mutex<SystemMix>>,
		echo_reference: Arc<Mutex<EchoReference>>,
	) -> Self {
		Self {
			downsampler: Downsampler::new(sample_rate, channels),
			resampled: Vec::new(),
			echo_reference,
			played: Vec::new(),
			voice: CallVoice::new(),
			cleaned: Vec::new(),
			limiter: Limiter::new(
				NETWORK_SAMPLE_RATE as f32,
				CALL_LIMIT_LOOKAHEAD,
				CALL_LIMIT_CEILING,
				CALL_LIMIT_RELEASE_SECS,
			),
			sample_accum: Vec::with_capacity(FRAME_SAMPLES * 2),
			side_accum: Vec::with_capacity(FRAME_SAMPLES * 2),
			system,
			frame: Vec::with_capacity(FRAME_SAMPLES),
			send_buf: Vec::with_capacity(FRAME_SAMPLES * 4 + OPUS_MAX_PACKET),
			opus: new_call_opus_encoder(),
			applied_loss_percent: 0,
			slices_since_mic_check: 0,
			checks_since_echo_report: 0,
			logged_echo_delay: None,
			device_rate: sample_rate,
			device_channels: channels,
			send_rate: NETWORK_SAMPLE_RATE,
			high_band: None,
			wide_frame: Vec::with_capacity(FRAME_SAMPLES * 3),
			muted_scratch: Vec::new(),
			dtx: false,
			silent_frames: 0,
			side_downsampler: None,
			side_resampled: Vec::new(),
			voice_side: Vec::new(),
			stereo_encoder: false,
			stereo_on: false,
			stereo_hold: 0,
			stereo_frame: Vec::with_capacity(FRAME_SAMPLES * 2),
		}
	}

	/// Counts how long the device has given only digital silence. Muted time
	/// says nothing either way.
	fn watch_mic_health(&mut self, data: &[f32], muted: bool) {
		if muted {
			self.silent_frames = 0;
			return;
		}
		let frames = (data.len() / self.device_channels.max(1) as usize) as u32;
		if data.iter().any(|s| s.abs() > MIC_SILENCE_PEAK) {
			self.silent_frames = 0;
			set_mic_silent(false);
			return;
		}
		self.silent_frames = self.silent_frames.saturating_add(frames);
		if self.silent_frames >= self.device_rate * MIC_SILENT_AFTER_SECS {
			set_mic_silent(true);
		}
	}

	/// Follows the call's audio rate: a new encoder for it, and the high band
	/// on above 16 kHz.
	fn follow_send_rate(&mut self) {
		let dtx = CALL_SEND_DTX.load(Ordering::Relaxed);
		if dtx != self.dtx {
			self.dtx = dtx;
			if let Some(encoder) = self.opus.as_mut() {
				let _ = encoder.set_dtx(dtx);
			}
		}
		let rate = CALL_SEND_RATE.load(Ordering::Relaxed);
		if rate == self.send_rate {
			return;
		}
		self.send_rate = rate;
		if let Some(encoder) = call_opus_encoder(rate, false) {
			self.opus = Some(encoder);
			self.applied_loss_percent = CALL_PACKET_LOSS_PERCENT.load(Ordering::Relaxed);
			self.stereo_encoder = false;
			self.stereo_on = false;
		}
		self.high_band = (rate > NETWORK_SAMPLE_RATE)
			.then(|| crate::calls::swb_uplink::HighBand::new(self.device_rate, self.device_channels));
		log_line(&format!("call audio: sending at {rate} Hz"));
	}

	/// Whether this round goes out in stereo, and the encoder to match: a
	/// stereo one once there's something stereo to send, sending mono again
	/// while there isn't; the mono one back when the setting goes off.
	fn follow_stereo(&mut self, allowed: bool, has_stereo: bool, samples: usize) -> bool {
		if has_stereo && allowed {
			self.stereo_hold = STEREO_HOLD_SAMPLES;
		} else {
			self.stereo_hold = self.stereo_hold.saturating_sub(samples);
		}
		let want = allowed && self.stereo_hold > 0;
		if !allowed {
			if self.stereo_encoder
				&& let Some(encoder) = call_opus_encoder(self.send_rate, false)
			{
				self.opus = Some(encoder);
				self.stereo_encoder = false;
				self.stereo_on = false;
				log_line("call audio: back to the mono encoder");
			}
			return false;
		}
		if want && !self.stereo_encoder {
			let Some(encoder) = call_opus_encoder(self.send_rate, true) else { return false };
			self.opus = Some(encoder);
			self.stereo_encoder = true;
			self.stereo_on = true;
			self.applied_loss_percent = CALL_PACKET_LOSS_PERCENT.load(Ordering::Relaxed);
			log_line("call audio: sending stereo");
			return true;
		}
		if self.stereo_encoder && want != self.stereo_on {
			self.stereo_on = want;
			if let Some(encoder) = self.opus.as_mut() {
				let _ = encoder.set_force_channels(if want { 2 } else { 1 });
				let bitrate = call_bitrate(self.send_rate, self.applied_loss_percent, want);
				let _ = encoder.set_bitrate(crate::audio::libopus::Bitrate::Bits(bitrate));
			}
			log_line(if want { "call audio: sending stereo" } else { "call audio: nothing stereo to send, mono" });
		}
		self.stereo_encoder && self.stereo_on
	}

	fn process(
		&mut self,
		data: &[f32],
		muted: bool,
		socket_out: &UdpSocket,
		target_addr: &str,
		mic: Option<&mut MicLevelController>,
	) {
		// mic -> echo canceller -> noise suppressor -> auto gain -> pause gate
		// (all in CallVoice) -> + system audio (ducked while talking) -> limiter.
		self.follow_send_rate();
		self.watch_mic_health(data, muted);
		let raw = call_raw_audio();
		let stereo_allowed = CALL_STEREO_SEND.load(Ordering::Relaxed) && self.send_rate == NETWORK_SAMPLE_RATE;
		let stereo_mic = stereo_allowed && raw && self.device_channels >= 2;
		if stereo_mic {
			let channels = self.device_channels as usize;
			let side_downsampler = self.side_downsampler.get_or_insert_with(|| Downsampler::new(self.device_rate, 1));
			// In step with `resampled`: a downsampler started mid-call lines
			// up with what is already waiting there.
			self.side_resampled.resize(self.resampled.len(), 0.0);
			self.voice_side.clear();
			self.voice_side.extend(data.chunks_exact(channels).map(|frame| (frame[0] - frame[1]) * 0.5));
			side_downsampler.process(&self.voice_side, &mut self.side_resampled);
		} else if self.side_downsampler.take().is_some() {
			self.side_resampled.clear();
		}
		self.downsampler.process(data, &mut self.resampled);
		if self.side_downsampler.is_some() {
			self.side_resampled.resize(self.resampled.len(), 0.0);
		}
		if let Some(high_band) = self.high_band.as_mut() {
			high_band.push_device(data);
		}
		let ready = self.echo_reference.lock().unwrap().take(self.resampled.len(), &mut self.played);
		self.cleaned.clear();
		if raw {
			self.cleaned.extend_from_slice(&self.resampled[..ready]);
		} else {
			self.slices_since_mic_check +=
				self.voice.process(&self.resampled[..ready], &self.played[..ready], muted, &mut self.cleaned);
		}
		if let Some(high_band) = self.high_band.as_mut() {
			high_band.chain_took(&self.resampled[..ready]);
			if muted {
				self.muted_scratch.clear();
				self.muted_scratch.resize(self.cleaned.len(), 0.0);
				high_band.push_cleaned(&self.muted_scratch);
			} else {
				high_band.push_cleaned(&self.cleaned);
			}
		}
		self.resampled.drain(..ready);
		// In raw mode `cleaned` is `resampled[..ready]`, so the mic's side
		// lines up with it sample for sample.
		self.voice_side.clear();
		if self.side_downsampler.is_some() {
			let taken = ready.min(self.side_resampled.len());
			self.voice_side.extend(self.side_resampled.drain(..taken));
		}
		let sharing = self.system.lock().unwrap().sharing();
		let stereo = self.follow_stereo(stereo_allowed, stereo_mic || sharing, self.cleaned.len());
		let mut system = self.system.lock().unwrap();
		for (i, &voice) in self.cleaned.iter().enumerate() {
			// WhatsApp's mixer turns the shared sound down while you talk;
			// "no processing" leaves it as it is.
			let [system_left, system_right] = system.next(voice, !raw);
			let mut mid = voice + (system_left + system_right) * 0.5;
			let mut side = if stereo {
				self.voice_side.get(i).copied().unwrap_or(0.0) + (system_left - system_right) * 0.5
			} else {
				0.0
			};
			if muted {
				(mid, side) = (0.0, 0.0);
			}
			let pair = [mid + side, mid - side];
			let [left, right] = if raw { pair.map(|s| s.clamp(-1.0, 1.0)) } else { self.limiter.process(pair) };
			self.sample_accum.push((left + right) * 0.5);
			self.side_accum.push((left - right) * 0.5);
		}
		drop(system);

		while self.sample_accum.len() >= FRAME_SAMPLES {
			self.frame.clear();
			self.frame.extend(self.sample_accum.drain(..FRAME_SAMPLES));
			self.stereo_frame.clear();
			let sides = self.side_accum.drain(..FRAME_SAMPLES);
			if self.stereo_encoder {
				for (&mid, side) in self.frame.iter().zip(sides) {
					self.stereo_frame.extend([mid + side, mid - side]);
				}
			} else {
				drop(sides);
			}
			let planned_loss = CALL_PACKET_LOSS_PERCENT.load(Ordering::Relaxed);
			if planned_loss != self.applied_loss_percent {
				if let Some(encoder) = self.opus.as_mut() {
					let _ = encoder.set_packet_loss_perc(planned_loss);
					let before = call_bitrate(self.send_rate, self.applied_loss_percent, self.stereo_on);
					let now = call_bitrate(self.send_rate, planned_loss, self.stereo_on);
					if now != before {
						let _ = encoder.set_bitrate(crate::audio::libopus::Bitrate::Bits(now));
						log_line(&format!("call audio: {now} bit/s for {planned_loss}% loss"));
					}
				}
				self.applied_loss_percent = planned_loss;
			}
			let wide = match self.high_band.as_mut() {
				Some(high_band) => {
					high_band.wide_frame(&self.frame, &mut self.wide_frame);
					Some(self.wide_frame.as_slice())
				}
				None if self.stereo_encoder => Some(self.stereo_frame.as_slice()),
				None => None,
			};
			pack_call_frame(&self.frame, wide, self.opus.as_mut(), &mut self.send_buf);
			let _ = socket_out.send_to(&self.send_buf, target_addr);
		}
		if self.slices_since_mic_check >= MIC_CHECK_SLICES {
			self.slices_since_mic_check = 0;
			if let Some(mic) = mic {
				mic.adjust(&mut self.voice.agc);
			}
			let delay = self.voice.echo_delay_ms();
			if delay != self.logged_echo_delay {
				self.logged_echo_delay = delay;
				log_line(&format!("echo canceller: played sound comes back after about {:?} ms", delay));
			}
			// Every 10 s, enough to tell a real echo (removed dB well above 0)
			// from none, and the mic and played streams' alignment.
			self.checks_since_echo_report += 1;
			if self.checks_since_echo_report >= 10 {
				self.checks_since_echo_report = 0;
				let queued = self.echo_reference.lock().unwrap().queued();
				log_line(&format!(
					"echo: delay={:?} ms removed={:.1} dB share={:.2} played_queued={} ms mic_waiting={} ms",
					delay,
					self.voice.echo_removed_db(),
					self.voice.echo_share(),
					queued / 16,
					self.resampled.len() / 16
				));
			}
		}
	}
}

/// The playout runs at 48 kHz: Opus is decoded straight to it with libopus's
/// bandwidth extension (calls/opus_jitter.rs), and audio the backend decoded
/// (MLow, 16 kHz) is upsampled to it.
const PLAYOUT_RATE: u32 = 48000;
const PLAYOUT_SCALE: usize = (PLAYOUT_RATE / NETWORK_SAMPLE_RATE) as usize;
/// One network frame (60 ms) at the playout rate.
const PLAYOUT_FRAME: usize = FRAME_SAMPLES * PLAYOUT_SCALE;
const PLAYOUT_REFILL_SAMPLES: usize = PLAYOUT_FRAME * 2;
/// Where playback likes to sit: the refill level plus one packet of jitter.
const PLAYOUT_TARGET_SAMPLES: usize = PLAYOUT_REFILL_SAMPLES + PLAYOUT_FRAME;
/// Above this (smoothed) the waiting audio is shortened by accelerate.
const PLAYOUT_CATCH_UP_ABOVE: f64 = (PLAYOUT_TARGET_SAMPLES + PLAYOUT_FRAME) as f64;
/// Delay the network never needed is cut too: the least audio that was
/// waiting, just before new audio arrived, over each 3 s of playback is
/// cushion that wasn't used, and everything above PLAYOUT_SLACK of it is
/// shortened by accelerate. A jittery network keeps that low point near zero
/// (so its cushion stays); a steady one no longer carries whatever delay a
/// late burst or the start of the call left in the queue (up to 240 ms used
/// to stay for the rest of the call). NetEQ does the same with its target
/// level from measured arrival jitter.
const PLAYOUT_SLACK: usize = PLAYOUT_RATE as usize * 30 / 1000;
const PLAYOUT_LOW_POINT_WINDOW: u64 = PLAYOUT_RATE as u64 * 3;

/// The call's sound, from the network to the output device: the playout
/// (calls/playout.rs: WhatsApp's NetEQ-style expand, merge and accelerate) at
/// 48 kHz, then linear resampling to the device's rate.
/// What the backend sends the playout: audio it decoded (MLow, group mixes),
/// or one of the peer's standard Opus packets, which the playout decodes
/// itself (calls/opus_jitter.rs).
pub(crate) enum Incoming {
	Pcm(Vec<f32>),
	Opus {
		sequence: u16,
		timestamp: u32,
		packet: Vec<u8>,
	},
	/// The call's audio rate changed (16000, 24000 or 48000).
	AudioRate(u32),
	/// Whether to encode with DTX ([4][on u8]).
	AudioDTX(bool),
}

/// One datagram from the backend's audio bridge (backend/audio_udp.go):
/// `[1][f32 LE samples...]`, `[2][ssrc u32][seq u16][timestamp u32][Opus packet]`,
/// `[3][audio rate u32]` or `[4][DTX on u8]` (little endian).
fn parse_incoming(datagram: &[u8]) -> Option<Incoming> {
	match datagram.split_first()? {
		(1, pcm) => Some(Incoming::Pcm(pcm.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect())),
		(2, rest) if rest.len() > 10 => Some(Incoming::Opus {
			sequence: u16::from_le_bytes(rest[4..6].try_into().ok()?),
			timestamp: u32::from_le_bytes(rest[6..10].try_into().ok()?),
			packet: rest[10..].to_vec(),
		}),
		(3, rest) if rest.len() >= 4 => Some(Incoming::AudioRate(u32::from_le_bytes(rest[..4].try_into().ok()?))),
		(4, [on, ..]) => Some(Incoming::AudioDTX(*on != 0)),
		_ => None,
	}
}

struct RenderDsp {
	channels: u16,
	/// Output rounds since the playout's counters were last logged.
	rounds_since_report: u32,
	playout: crate::calls::playout::Playout,
	/// Decoded 16 kHz audio from the backend, to the playout's 48 kHz.
	upsampler: crate::audio::call_voice::Upsampler3,
	upsampled: Vec<f32>,
	/// Playout samples per output sample, and where between `previous` and
	/// `upcoming` the next output sample falls.
	step: f64,
	phase: f64,
	previous: f32,
	upcoming: f32,
	/// The same for the side of a stereo peer.
	previous_side: f32,
	upcoming_side: f32,
	smoothed_level: f64,
	max_buffered_src: usize,
	/// The least audio waiting before an arrival in this window, the
	/// playout samples pulled in it, and audio still to cut (see PLAYOUT_SLACK).
	low_point: usize,
	window_pulled: u64,
	trim_left: u64,
	accelerated_mark: u64,
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
			rounds_since_report: 0,
			playout: crate::calls::playout::Playout::new(PLAYOUT_REFILL_SAMPLES, PLAYOUT_RATE),
			upsampler: crate::audio::call_voice::Upsampler3::new(),
			upsampled: Vec::new(),
			step: PLAYOUT_RATE as f64 / sample_rate as f64,
			phase: 0.0,
			previous: 0.0,
			upcoming: 0.0,
			previous_side: 0.0,
			upcoming_side: 0.0,
			smoothed_level: 0.0,
			max_buffered_src: PLAYOUT_FRAME * 8,
			low_point: usize::MAX,
			window_pulled: 0,
			trim_left: 0,
			accelerated_mark: 0,
			played,
			starved,
			accum_len,
		}
	}

	fn process(&mut self, data: &mut [f32], frame_rx: &mpsc::Receiver<Incoming>) {
		let channels = self.channels as usize;
		// Everything waiting in the channel is taken every round, so this one
		// buffer holds the whole backlog: when it overflows the OLDEST audio
		// goes (the channel itself would drop the newest), and a backlog left
		// by a burst of late packets is worked off by accelerate.
		if !self.playout.refilling {
			self.low_point = self.low_point.min(self.playout.queue.len());
		}
		let (queue, side, opus) = (&mut self.playout.queue, &mut self.playout.side, &mut self.playout.opus);
		while let Ok(item) = frame_rx.try_recv() {
			match item {
				Incoming::Pcm(frame) => {
					// Decoded audio again (MLow, or a group's mix): the Opus
					// stream, if there was one, is over.
					*opus = None;
					self.upsampled.clear();
					self.upsampler.process(&frame, &mut self.upsampled);
					queue.extend(&self.upsampled);
					side.resize(queue.len(), 0.0);
				}
				Incoming::Opus { sequence, timestamp, packet } => {
					if opus.is_none() {
						match crate::calls::opus_jitter::OpusJitter::new(PLAYOUT_RATE) {
							Ok(jitter) => {
								log_line(
									"playout: the peer sends standard Opus; decoding it here at 48 kHz (Deep PLC, NoLACE, bandwidth extension)",
								);
								*opus = Some(jitter);
							}
							Err(e) => log_line(&format!("playout: no Opus decoder: {e}")),
						}
					}
					if let Some(jitter) = opus.as_mut() {
						jitter.set_enhanced(!call_raw_audio());
						jitter.push(sequence, timestamp, packet);
						side.resize(queue.len(), 0.0);
						jitter.drain_ready(queue);
						side.extend(jitter.side.drain(..));
					}
				}
				// Taken by the reader thread; never queued.
				Incoming::AudioRate(_) | Incoming::AudioDTX(_) => {}
			}
		}
		while queue.len() > self.max_buffered_src {
			queue.pop_front();
			side.pop_front();
		}
		self.smoothed_level += (queue.len() as f64 - self.smoothed_level) * 0.02;
		let trimming = self.playout.accelerated - self.accelerated_mark < self.trim_left;
		let backlog = self.smoothed_level > PLAYOUT_CATCH_UP_ABOVE || trimming;

		let concealed_before = self.playout.concealed;
		let mut pulled = 0u64;
		for out_frame in data.chunks_mut(channels) {
			let phase = self.phase as f32;
			let sample = self.previous + (self.upcoming - self.previous) * phase;
			let side = self.previous_side + (self.upcoming_side - self.previous_side) * phase;
			for slot in out_frame.iter_mut() {
				*slot = sample;
			}
			// A stereo peer's left and right on the first two speakers.
			if let [left, right, ..] = out_frame {
				*left = sample + side;
				*right = sample - side;
			}
			self.phase += self.step;
			while self.phase >= 1.0 {
				self.phase -= 1.0;
				self.previous = self.upcoming;
				self.previous_side = self.upcoming_side;
				self.upcoming = self.playout.next(backlog);
				self.upcoming_side = self.playout.last_side;
				pulled += 1;
			}
		}
		let concealed = self.playout.concealed - concealed_before;
		self.window_pulled += pulled;
		if self.window_pulled >= PLAYOUT_LOW_POINT_WINDOW {
			self.window_pulled = 0;
			self.trim_left = 0;
			if self.low_point != usize::MAX && self.low_point > PLAYOUT_SLACK && concealed == 0 {
				self.trim_left = (self.low_point - PLAYOUT_SLACK) as u64;
				self.accelerated_mark = self.playout.accelerated;
			}
			self.low_point = usize::MAX;
		}
		self.played.fetch_add(pulled - concealed, Ordering::Relaxed);
		self.starved.fetch_add(concealed, Ordering::Relaxed);
		self.accum_len.store(self.playout.queue.len(), Ordering::Relaxed);

		self.rounds_since_report += 1;
		if self.rounds_since_report >= 1000 {
			self.rounds_since_report = 0;
			let p = &self.playout;
			let (plc, fec, late) = p.opus.as_ref().map_or((0, 0, 0), |j| (j.concealed, j.recovered, j.dropped_late));
			log_line(&format!(
				"playout: queued={} ms concealed={} ms (expand {} ms, opus plc {} ms) fec_rebuilt={} ms late_dropped={} accelerated={} ms",
				p.queue.len() / 48,
				p.concealed / 48 + plc / 16,
				p.concealed / 48,
				plc / 16,
				fec / 16,
				late,
				p.accelerated / 48
			));
		}
	}
}

fn run_capture(
	stop: &AtomicBool,
	is_muted: &AtomicBool,
	socket_out: &UdpSocket,
	target_addr: &str,
	preferred_device: &Mutex<Option<String>>,
	force_reopen: &AtomicBool,
	system: &Arc<Mutex<SystemMix>>,
	echo_reference: &Arc<Mutex<EchoReference>>,
) {
	while !stop.load(Ordering::Relaxed) {
		if let Err(e) = run_capture_session(
			stop,
			is_muted,
			socket_out,
			target_addr,
			preferred_device,
			force_reopen,
			system,
			echo_reference,
		) {
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
	system: &Arc<Mutex<SystemMix>>,
	echo_reference: &Arc<Mutex<EchoReference>>,
) -> crate::error::Result<()> {
	unsafe {
		CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
		let _com_guard = ComGuard;

		let session_device = preferred_device.lock().unwrap().clone();
		force_reopen.store(false, Ordering::Relaxed);
		let wasapi = open_communications_stream(eCapture, "capture", session_device.as_deref(), true, false)?;
		let _event_guard = EventGuard(wasapi.event);
		let capture_client: IAudioCaptureClient = wasapi.client.GetService()?;

		let mut dsp =
			CaptureDsp::new(wasapi.sample_rate, wasapi.channels, Arc::clone(system), Arc::clone(echo_reference));
		log_line(&format!(
			"capture dsp: echo canceller, noise suppressor, auto gain to {AGC_TARGET_DBFS} dBFS, pause gate -{GATE_DEPTH_DB} dB, limiter {CALL_LIMIT_CEILING}"
		));
		let mut mic_level = MicLevelController::new(&wasapi.device);
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
				if (flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32) != 0 {
					// The mic dropped samples: the played sound is now paired
					// with the wrong mic samples, so line the two up afresh.
					echo_reference.lock().unwrap().line_up_again();
					note_discontinuity("capture");
				}

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
				dsp.process(&scratch, muted, socket_out, target_addr, mic_level.as_mut());

				capture_client.ReleaseBuffer(num_frames)?;
			}
		}

		wasapi.client.Stop()?;
		log_line("capture session exiting (stop set)");
	}
	Ok(())
}

fn run_render(
	stop: &AtomicBool,
	frame_rx: mpsc::Receiver<Incoming>,
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
	frame_rx: &mpsc::Receiver<Incoming>,
	played: &Arc<AtomicU64>,
	starved: &Arc<AtomicU64>,
	accum_len: &Arc<AtomicUsize>,
	preferred_device: &Mutex<Option<String>>,
	force_reopen: &AtomicBool,
) -> crate::error::Result<()> {
	unsafe {
		CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
		let _com_guard = ComGuard;

		let session_device = preferred_device.lock().unwrap().clone();
		force_reopen.store(false, Ordering::Relaxed);
		let wasapi = open_communications_stream(eRender, "render", session_device.as_deref(), true, false)?;
		let _event_guard = EventGuard(wasapi.event);
		let render_client: IAudioRenderClient = wasapi.client.GetService()?;

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

// Records what the call's output device is playing (WASAPI loopback: every
// app's sound, so the other person's voice but also a screen reader or a
// video) and hands it to the mic thread as the echo canceller's reference.
// Windows only delivers loopback sound while something is playing; during a
// call our own render stream always is.
fn run_echo_reference(
	stop: &AtomicBool,
	preferred_device: &Mutex<Option<String>>,
	force_reopen: &AtomicBool,
	echo_reference: &Mutex<EchoReference>,
) {
	while !stop.load(Ordering::Relaxed) {
		if let Err(e) = run_echo_reference_session(stop, preferred_device, force_reopen, echo_reference) {
			log_line(&format!("echo reference session ended with error, reopening: {:?}", e));
			if stop.load(Ordering::Relaxed) {
				break;
			}
			std::thread::sleep(Duration::from_millis(500));
		}
	}
	log_line("echo reference thread exiting (stop set)");
}

fn run_echo_reference_session(
	stop: &AtomicBool,
	preferred_device: &Mutex<Option<String>>,
	force_reopen: &AtomicBool,
	echo_reference: &Mutex<EchoReference>,
) -> crate::error::Result<()> {
	unsafe {
		CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
		let _com_guard = ComGuard;

		let session_device = preferred_device.lock().unwrap().clone();
		force_reopen.store(false, Ordering::Relaxed);
		let wasapi = open_communications_stream(eRender, "echo reference", session_device.as_deref(), false, true)?;
		let _event_guard = EventGuard(wasapi.event);
		let capture_client: IAudioCaptureClient = wasapi.client.GetService()?;
		let mut downsampler = Downsampler::new(wasapi.sample_rate, wasapi.channels);
		let mut scratch: Vec<f32> = Vec::new();
		let mut resampled: Vec<f32> = Vec::new();

		wasapi.client.Start()?;
		log_line("echo reference stream started (loopback of the output device)");

		while !stop.load(Ordering::Relaxed) {
			if preferred_device.lock().unwrap().as_ref() != session_device.as_ref() {
				log_line("echo reference: device switch requested, reopening");
				break;
			}
			if force_reopen.swap(false, Ordering::Relaxed) {
				log_line("echo reference: default device changed, reopening");
				break;
			}
			// Looked at every 20 ms even without the event: not every driver
			// signals it for loopback streams.
			let _ = WaitForSingleObject(wasapi.event, 20);
			loop {
				if capture_client.GetNextPacketSize()? == 0 {
					break;
				}
				let mut data_ptr: *mut u8 = std::ptr::null_mut();
				let mut num_frames: u32 = 0;
				let mut flags: u32 = 0;
				capture_client.GetBuffer(&mut data_ptr, &mut num_frames, &mut flags, None, None)?;
				if (flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32) != 0 {
					echo_reference.lock().unwrap().line_up_again();
					note_discontinuity("echo reference");
				}
				let total_samples = num_frames as usize * wasapi.channels as usize;
				scratch.clear();
				if data_ptr.is_null() || (flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 {
					scratch.resize(total_samples, 0.0);
				} else if wasapi.is_float {
					scratch.extend_from_slice(std::slice::from_raw_parts(data_ptr as *const f32, total_samples));
				} else if wasapi.bits_per_sample == 16 {
					let src = std::slice::from_raw_parts(data_ptr as *const i16, total_samples);
					scratch.extend(src.iter().map(|&s| s as f32 / 32768.0));
				} else {
					scratch.resize(total_samples, 0.0);
				}
				capture_client.ReleaseBuffer(num_frames)?;

				resampled.clear();
				downsampler.process(&scratch, &mut resampled);
				echo_reference.lock().unwrap().push(&resampled);
			}
		}

		wasapi.client.Stop()?;
		log_line("echo reference session exiting");
	}
	Ok(())
}

/// Logs the first few times a capture stream reports it dropped samples, then
/// every hundredth, so a call's log shows whether glitches are happening.
fn note_discontinuity(stream: &str) {
	static COUNT: AtomicU64 = AtomicU64::new(0);
	let n = COUNT.fetch_add(1, Ordering::Relaxed) + 1;
	if n <= 5 || n % 100 == 0 {
		log_line(&format!("{stream} stream dropped samples (glitch {n}); echo reference lined up again"));
	}
}

pub fn start_audio_streams(
	target_port: u16,
	preferred_input_device: Option<String>,
	preferred_output_device: Option<String>,
) -> crate::error::Result<CallAudioState> {
	crate::platform::paths::rotate_log("call_audio.log");
	let socket = UdpSocket::bind("127.0.0.1:0")?;
	let target_addr = format!("127.0.0.1:{}", target_port);

	socket.send_to(&[0u8; 4], &target_addr)?;

	let socket_read = socket.try_clone()?;
	let socket_out = socket.try_clone()?;

	// The output side must never block on a network read on a real-time audio
	// thread; a plain background thread blocks on the socket and hands decoded
	// frames to the render thread through a small bounded channel, which the
	// render loop only ever drains with a non-blocking try_recv.
	let (frame_tx, frame_rx) = mpsc::sync_channel::<Incoming>(16);
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

	crate::util::spawn_named("call audio reader", move || {
		let _ = socket_read.set_read_timeout(Some(Duration::from_millis(200)));
		disable_udp_connreset(&socket_read);
		let mut buf = vec![0u8; 65536];
		while !reader_stop_thread.load(Ordering::Relaxed) {
			match socket_read.recv_from(&mut buf) {
				Ok((size, _)) => {
					reader_recv_count_thread.fetch_add(1, Ordering::Relaxed);
					let Some(frame) = parse_incoming(&buf[..size]) else {
						reader_error_count_thread.fetch_add(1, Ordering::Relaxed);
						continue;
					};
					if let Incoming::AudioRate(rate) = frame {
						if matches!(rate, 16_000 | 24_000 | 48_000) {
							log_line(&format!("call audio: the call runs at {rate} Hz"));
							CALL_SEND_RATE.store(rate, Ordering::Relaxed);
						}
						continue;
					}
					if let Incoming::AudioDTX(on) = frame {
						log_line(&format!("call audio: DTX {}", if on { "on" } else { "off" }));
						CALL_SEND_DTX.store(on, Ordering::Relaxed);
						continue;
					}
					if frame_tx.try_send(frame).is_err() {
						tx_drop_count_thread.fetch_add(1, Ordering::Relaxed);
					}
				}
				Err(ref e)
					if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {}
				Err(e) => {
					reader_error_count_thread.fetch_add(1, Ordering::Relaxed);
					log_line(&format!("reader recv_from error (ignored, continuing): {:?}", e));
				}
			}
		}
		log_line("reader thread exiting (reader_stop set)");
	});

	{
		let health_stop = Arc::clone(&stop);
		let reader_recv_count = Arc::clone(&reader_recv_count);
		let reader_error_count = Arc::clone(&reader_error_count);
		let tx_drop_count = Arc::clone(&tx_drop_count);
		let output_played_samples = Arc::clone(&output_played_samples);
		let output_starved_samples = Arc::clone(&output_starved_samples);
		let output_src_accum_len = Arc::clone(&output_src_accum_len);
		crate::util::spawn_named("call audio health", move || {
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
	let force_reopen_input = Arc::new(AtomicBool::new(false));
	let force_reopen_output = Arc::new(AtomicBool::new(false));
	let force_reopen_echo = Arc::new(AtomicBool::new(false));
	let system_mix = Arc::new(Mutex::new(SystemMix::default()));
	let echo_reference = Arc::new(Mutex::new(EchoReference::default()));

	{
		let stop = Arc::clone(&stop);
		let preferred_output = Arc::clone(&preferred_output);
		let force_reopen_echo = Arc::clone(&force_reopen_echo);
		let echo_reference = Arc::clone(&echo_reference);
		crate::util::spawn_named("call echo reference", move || {
			run_echo_reference(&stop, &preferred_output, &force_reopen_echo, &echo_reference);
		});
	}

	{
		let stop = Arc::clone(&stop);
		let is_muted = Arc::clone(&is_muted);
		let target_addr = target_addr.clone();
		let preferred_input = Arc::clone(&preferred_input);
		let force_reopen_input = Arc::clone(&force_reopen_input);
		let system_mix = Arc::clone(&system_mix);
		crate::util::spawn_named("call capture", move || {
			run_capture(
				&stop,
				&is_muted,
				&socket_out,
				&target_addr,
				&preferred_input,
				&force_reopen_input,
				&system_mix,
				&echo_reference,
			);
		});
	}

	{
		let stop = Arc::clone(&stop);
		let preferred_output = Arc::clone(&preferred_output);
		let force_reopen_output = Arc::clone(&force_reopen_output);
		crate::util::spawn_named("call render", move || {
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

	Ok(CallAudioState {
		is_muted,
		stop,
		preferred_input,
		preferred_output,
		force_reopen_input,
		force_reopen_output,
		force_reopen_echo,
		system_mix,
		system_audio: None,
	})
}

#[cfg(test)]
mod tests {

	fn tone_at(rate: u32, hz: f32, secs: f32) -> Vec<f32> {
		(0..(rate as f32 * secs) as usize)
			.map(|i| 0.5 * (2.0 * std::f32::consts::PI * hz * i as f32 / rate as f32).sin())
			.collect()
	}

	fn level_db(x: &[f32]) -> f32 {
		10.0 * (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32 + 1e-20).log10()
	}

	#[test]
	fn downsampler_keeps_voice_and_removes_what_would_alias() {
		for rate in [44_100u32, 48_000, 96_000] {
			let through = |hz: f32, chunk: usize| {
				let mut ds = Downsampler::new(rate, 2);
				let stereo: Vec<f32> = tone_at(rate, hz, 1.0).iter().flat_map(|&s| [s, s]).collect();
				let mut out = Vec::new();
				for c in stereo.chunks(chunk) {
					ds.process(c, &mut out);
				}
				let expect = NETWORK_SAMPLE_RATE as usize;
				assert!(out.len().abs_diff(expect) < 600, "{rate}: {} samples out", out.len());
				level_db(&out[2000..out.len() - 2000]) - level_db(&tone_at(rate, hz, 1.0))
			};
			for hz in [100.0, 1000.0, 4000.0, 7000.0] {
				let change = through(hz, 2 * 441);
				assert!(change.abs() < 0.5, "{rate}: {hz} Hz changed {change} dB");
			}
			for hz in [9000.0, 12_000.0, 20_000.0] {
				let change = through(hz, 2 * 480);
				assert!(change < -40.0, "{rate}: {hz} Hz only fell {change} dB");
			}
		}
	}
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
		let mut decoder =
			crate::audio::libopus::Decoder::new(NETWORK_SAMPLE_RATE, crate::audio::libopus::Channels::Mono)
				.expect("libopus decoder");
		let mut datagram = Vec::new();
		let mut energy = 0.0f32;
		for n in 0..10 {
			let frame = tone_frame(n * FRAME_SAMPLES);
			pack_call_frame(&frame, None, Some(&mut encoder), &mut datagram);

			let (pcm, packet) = datagram.split_at(FRAME_SAMPLES * 4);
			let sent: Vec<f32> =
				pcm.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
			assert_eq!(sent, frame, "the PCM half is the frame, untouched");

			assert!(!packet.is_empty(), "frame {n} has no Opus packet");
			assert_eq!(
				crate::audio::libopus::packet::get_nb_samples(packet, NETWORK_SAMPLE_RATE).unwrap(),
				FRAME_SAMPLES
			);
			assert_eq!(packet[0], 0x58, "frame {n} is not one 60 ms SILK wideband frame");

			let mut decoded = vec![0.0f32; FRAME_SAMPLES];
			assert_eq!(decoder.decode_float(packet, &mut decoded, false).unwrap(), FRAME_SAMPLES);
			if n >= 2 {
				energy += decoded.iter().map(|s| s * s).sum::<f32>();
			}
		}
		assert!(energy > 1.0, "the Opus packets decode back to the tone, not silence");
	}

	fn render_dsp() -> RenderDsp {
		let counter = || Arc::new(AtomicU64::new(0));
		RenderDsp::new(NETWORK_SAMPLE_RATE, 1, counter(), counter(), Arc::new(AtomicUsize::new(0)))
	}

	fn mean(x: &[f32]) -> f32 {
		x.iter().sum::<f32>() / x.len() as f32
	}

	#[test]
	fn playback_waits_for_the_refill_level_again_after_running_dry() {
		let mut dsp = render_dsp();
		let (tx, rx) = mpsc::sync_channel::<Incoming>(8);
		let mut out = vec![0.0f32; 160];

		tx.send(Incoming::Pcm(vec![0.5; FRAME_SAMPLES])).unwrap();
		dsp.process(&mut out, &rx);
		assert!(out.iter().all(|&s| s == 0.0), "started before the refill level");

		tx.send(Incoming::Pcm(vec![0.5; FRAME_SAMPLES])).unwrap();
		dsp.process(&mut out, &rx);
		dsp.process(&mut out, &rx);
		assert!(out.iter().all(|&s| (s - 0.5).abs() < 0.01), "did not start once the refill level arrived");

		for _ in 0..(2 * FRAME_SAMPLES / out.len()) {
			dsp.process(&mut out, &rx);
		}
		assert!(dsp.playout.refilling, "running dry did not start a refill");

		// While refilling, the sound carries on (expand), not silence.
		tx.send(Incoming::Pcm(vec![0.25; FRAME_SAMPLES])).unwrap();
		dsp.process(&mut out, &rx);
		assert!(mean(&out) > 0.2, "went silent while waiting: {}", mean(&out));
		tx.send(Incoming::Pcm(vec![0.25; FRAME_SAMPLES])).unwrap();
		for _ in 0..3 {
			dsp.process(&mut out, &rx);
		}
		assert!(out.iter().all(|&s| (s - 0.25).abs() < 0.01), "did not resume after refilling");
	}

	#[test]
	fn a_backlog_from_a_burst_is_played_off_and_overflow_drops_the_oldest() {
		let mut dsp = render_dsp();
		let (tx, rx) = mpsc::sync_channel::<Incoming>(16);
		let mut out = vec![0.0f32; 160];
		// A burst: ten frames at once, numbered so overflow shows which went.
		for i in 0..10 {
			let _ = tx.try_send(Incoming::Pcm(vec![i as f32; FRAME_SAMPLES]));
		}
		dsp.process(&mut out, &rx);
		dsp.process(&mut out, &rx);
		assert!((out[100] - 2.0).abs() < 0.02, "overflow dropped the newest audio instead of the oldest");
		// Then packets keep coming at exactly real time.
		let mut fed = 0;
		for round in 0..6000 {
			if round % 6 == 0 {
				tx.send(Incoming::Pcm(vec![0.5; FRAME_SAMPLES])).unwrap();
				fed += 1;
			}
			dsp.process(&mut out, &rx);
		}
		assert!(fed > 900);
		assert!(
			dsp.playout.queue.len() <= PLAYOUT_TARGET_SAMPLES + PLAYOUT_FRAME * 2,
			"backlog still {} samples after a minute",
			dsp.playout.queue.len()
		);
		assert!(dsp.playout.accelerated > 0, "the backlog wasn't shortened by accelerate");
		assert!(!dsp.playout.refilling, "caught up so far that it ran dry");
	}

	#[test]
	fn bitrate_steps_down_with_heavy_loss() {
		assert_eq!(call_bitrate(16_000, 0, false), OPUS_BITRATE);
		assert_eq!(call_bitrate(16_000, 12, false), OPUS_BITRATE * 4 / 5);
		assert_eq!(call_bitrate(16_000, 30, false), OPUS_BITRATE * 3 / 5);
		assert_eq!(call_bitrate(48_000, 0, false), OPUS_FB_BITRATE);
		assert_eq!(call_bitrate(16_000, 0, true), OPUS_STEREO_BITRATE);
		assert_eq!(call_bitrate(48_000, 0, true), OPUS_FB_BITRATE);
	}

	#[test]
	fn delay_the_network_never_needed_is_cut() {
		let mut dsp = render_dsp();
		let (tx, rx) = mpsc::sync_channel::<Incoming>(16);
		let mut out = vec![0.0f32; 160];
		// 150 ms more than needed waiting, then packets at exactly real time:
		// the old catch-up threshold (240 ms) never touched this.
		for _ in 0..3 {
			tx.send(Incoming::Pcm(vec![0.0; FRAME_SAMPLES])).unwrap();
		}
		for round in 0..3000 {
			if round % 6 == 0 {
				tx.send(Incoming::Pcm(vec![0.0; FRAME_SAMPLES])).unwrap();
			}
			dsp.process(&mut out, &rx);
		}
		// Just before each arrival the queue now sits near the slack.
		let mut lowest = usize::MAX;
		for round in 0..60 {
			if round % 6 == 0 {
				lowest = lowest.min(dsp.playout.queue.len());
				tx.send(Incoming::Pcm(vec![0.0; FRAME_SAMPLES])).unwrap();
			}
			dsp.process(&mut out, &rx);
		}
		assert!(lowest <= PLAYOUT_SLACK + PLAYOUT_FRAME / 6, "still {} ms waiting", lowest / 48);
		assert!(!dsp.playout.refilling, "cut so far that it ran dry");
	}

	#[test]
	fn datagrams_from_the_backend_are_told_apart() {
		let mut pcm = vec![1u8];
		pcm.extend(0.5f32.to_le_bytes());
		pcm.extend((-0.25f32).to_le_bytes());
		assert!(matches!(parse_incoming(&pcm), Some(Incoming::Pcm(s)) if s == [0.5, -0.25]));
		let mut opus = vec![2u8, 0xD4, 0xC3, 0xB2, 0xA1, 7, 0];
		opus.extend(96000u32.to_le_bytes());
		opus.extend([0x58, 1, 2]);
		assert!(matches!(
			parse_incoming(&opus),
			Some(Incoming::Opus { sequence: 7, timestamp: 96000, packet }) if packet == [0x58, 1, 2]
		));
		assert!(parse_incoming(&[]).is_none());
		assert!(parse_incoming(&[9, 1, 2]).is_none());
	}

	/// The peer's Opus through the playout as the render thread runs it: one
	/// packet lost, one 40 ms late. The sound never stops for either.
	#[test]
	fn opus_from_the_peer_plays_through_loss_and_lateness_without_silence() {
		let mut dsp = render_dsp();
		let (tx, rx) = mpsc::sync_channel::<Incoming>(64);
		let mut encoder = new_call_opus_encoder().expect("libopus encoder");
		let _ = encoder.set_packet_loss_perc(20);
		let mut out = vec![0.0f32; 160];
		let mut played = Vec::new();
		let mut late = None;
		for n in 0..40usize {
			let mut packet = vec![0u8; OPUS_MAX_PACKET];
			let len = encoder.encode_float(&tone_frame(n * FRAME_SAMPLES), &mut packet).unwrap();
			packet.truncate(len);
			let item = Incoming::Opus { sequence: n as u16, timestamp: (n * FRAME_SAMPLES) as u32, packet };
			match n {
				20 => {}                 // lost
				30 => late = Some(item), // held back 40 ms
				_ => tx.send(item).unwrap(),
			}
			// 60 ms of playback per packet, 10 ms at a time.
			for round in 0..6 {
				if n == 30
					&& round == 4
					&& let Some(item) = late.take()
				{
					tx.send(item).unwrap();
				}
				dsp.process(&mut out, &rx);
				played.extend_from_slice(&out);
			}
		}
		let jitter = dsp.playout.opus.as_ref().expect("no Opus decoder");
		assert!(jitter.recovered + jitter.concealed > 0, "nothing was concealed or rebuilt");
		// From the first packet on, no 10 ms stretch is silent.
		let start = played.iter().position(|&s| s != 0.0).expect("nothing played");
		for (i, chunk) in played[start..].chunks(160).enumerate() {
			let rms = (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt();
			assert!(rms > 0.05, "silence {} ms in (rms {rms})", i * 10);
		}
	}

	#[test]
	fn a_call_frame_without_an_encoder_is_pcm_alone() {
		let mut datagram = vec![1, 2, 3];
		pack_call_frame(&tone_frame(0), None, None, &mut datagram);
		assert_eq!(datagram.len(), FRAME_SAMPLES * 4);
	}

	/// A stereo call frame (left a tone, right silent) goes out as stereo
	/// Opus, and the playout's decoder gets its side back: left loud, right
	/// near silent.
	#[test]
	fn stereo_frames_come_back_as_left_and_right() {
		let mut encoder = call_opus_encoder(NETWORK_SAMPLE_RATE, true).expect("stereo encoder");
		let mut jitter = crate::calls::opus_jitter::OpusJitter::new(PLAYOUT_RATE).expect("decoder");
		let mut mid = std::collections::VecDeque::new();
		let mut datagram = Vec::new();
		for n in 0..20 {
			let frame: Vec<f32> = tone_frame(n * FRAME_SAMPLES).iter().map(|s| s * 0.5).collect();
			let stereo: Vec<f32> = tone_frame(n * FRAME_SAMPLES).iter().flat_map(|&l| [l, 0.0]).collect();
			pack_call_frame(&frame, Some(&stereo), Some(&mut encoder), &mut datagram);
			let packet = datagram[FRAME_SAMPLES * 4..].to_vec();
			assert_ne!(packet[0] & 0x04, 0, "frame {n} went out mono");
			jitter.push(n as u16, (n * FRAME_SAMPLES) as u32, packet);
			jitter.drain_ready(&mut mid);
		}
		let side: Vec<f32> = jitter.side.iter().copied().collect();
		assert_eq!(side.len(), mid.len());
		let skip = PLAYOUT_FRAME * 4;
		let (mut left, mut right) = (0.0f32, 0.0f32);
		for (m, s) in mid.iter().zip(&side).skip(skip) {
			left += (m + s) * (m + s);
			right += (m - s) * (m - s);
		}
		assert!(right * 20.0 < left, "right isn't quieter than left: {right} against {left}");
	}

	#[test]
	fn a_mono_peer_has_no_side() {
		let mut encoder = new_call_opus_encoder().expect("encoder");
		let mut jitter = crate::calls::opus_jitter::OpusJitter::new(PLAYOUT_RATE).expect("decoder");
		let mut mid = std::collections::VecDeque::new();
		let mut datagram = Vec::new();
		for n in 0..5 {
			pack_call_frame(&tone_frame(n * FRAME_SAMPLES), None, Some(&mut encoder), &mut datagram);
			jitter.push(n as u16, (n * FRAME_SAMPLES) as u32, datagram[FRAME_SAMPLES * 4..].to_vec());
			jitter.drain_ready(&mut mid);
		}
		assert_eq!(jitter.side.len(), mid.len());
		assert!(jitter.side.iter().all(|&s| s == 0.0));
	}
}
