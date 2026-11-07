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
				AUDCLNT_STREAMOPTIONS_NONE, AudioCategory_Communications, AudioClientProperties,
				Endpoints::IAudioEndpointVolume, IAudioCaptureClient, IAudioClient2, IAudioRenderClient, IMMDevice,
				IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEXTENSIBLE, eCapture, eCommunications, eRender,
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
		denoise::Denoiser,
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

// Opens a WASAPI shared-mode stream on the eCommunications-role default (or, if
// `preferred_device_name` names a still-present device, that specific) device for
// `dataflow` (eCapture or eRender). When `apply_effects` is set, the stream is also
// tagged with the Communications audio-session category, which is what makes Windows
// apply its own echo cancellation/noise-suppression/AGC audio-effects pipeline to the
// stream - the previous cpal-based implementation used the plain default multimedia-role
// devices with no session category set, which never engaged any of that, so
// speakerphone/laptop mic+speaker setups got raw, unprocessed audio (echo, room noise)
// end to end. Calls always pass `apply_effects: true`; voice-message recording passes
// false and does its own clean-up instead (audio/voice_dsp.rs).
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
) -> crate::error::Result<WasapiOpen> {
	unsafe {
		let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
		let device = match preferred_device_name
			.and_then(|name| crate::audio::devices::find_wasapi_device_by_name(&enumerator, dataflow, name))
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
		let rate = sample_rate as f64;
		let cutoff = (DS_CUTOFF_HZ / rate).min(0.45);
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
			step: rate / NETWORK_SAMPLE_RATE as f64,
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
	denoiser: Denoiser,
	denoised: Vec<f32>,
	limiter: Limiter<1>,
	sample_accum: Vec<f32>,
	system: Arc<Mutex<SystemMix>>,
	frame: Vec<f32>,
	send_buf: Vec<u8>,
	opus: Option<opus::Encoder>,
	agc: VoiceAgc,
	gate: PauseGate,
	slices_since_mic_check: u32,
}

fn new_call_opus_encoder() -> Option<opus::Encoder> {
	let mut encoder = opus::Encoder::new(NETWORK_SAMPLE_RATE, opus::Channels::Mono, opus::Application::Voip).ok()?;
	encoder.set_bitrate(opus::Bitrate::Bits(OPUS_BITRATE)).ok()?;
	encoder.set_complexity(OPUS_COMPLEXITY).ok()?;
	let _ = encoder.set_inband_fec(true);
	let _ = encoder.set_packet_loss_perc(10);
	Some(encoder)
}

// Auto gain for the outgoing call audio, modelled on WebRTC's AGC2 adaptive
// digital controller, which is what WhatsApp for Windows' native engine runs
// (WhatsAppNative.Voip.dll ships webrtc/latest agc2 and the server sends it
// agc {"enable_uwp_agc2":"true"}); WA Web gets the same from the browser's
// getUserMedia autoGainControl. Windows' communications effects barely level
// the mic: live calls measured voiced speech around -40 dBFS, and phones
// that receive standard Opus play it as sent.
//
// Like AGC2 it works on 10 ms slices: a simple energy VAD over a tracked
// noise floor, a slow speech-level estimate fed only by speech, a gain that
// moves at most AGC_MAX_GAIN_CHANGE_DB_PER_S towards bringing that speech
// level to AGC_TARGET_DBFS, capped so the noise floor isn't lifted above
// AGC_MAX_NOISE_DBFS. The gain falls faster than it rises, so a loud start
// or a raised mic level isn't left too hot for seconds. Peaks are handled
// after the system audio mix by a lookahead limiter (audio/limiter.rs).
const AGC_SLICE: usize = 160; // 10 ms at 16 kHz, one noise suppressor hop
const AGC_TARGET_DBFS: f32 = -20.0;
const AGC_MAX_GAIN_DB: f32 = 40.0;
const AGC_INITIAL_GAIN_DB: f32 = 15.0;
const AGC_MAX_GAIN_CHANGE_DB_PER_S: f32 = 6.0;
const AGC_MAX_GAIN_DROP_DB_PER_S: f32 = 30.0;
// As AGC2: the background left between words is never lifted above this.
// The noise suppressor in front lowers it first, so speech still reaches the
// target (live calls had a -52..-65 dBFS floor before suppression).
const AGC_MAX_NOISE_DBFS: f32 = -50.0;
const AGC_SPEECH_ABOVE_NOISE_DB: f32 = 10.0;
const AGC_MIN_SPEECH_DBFS: f32 = -65.0;

struct VoiceAgc {
	gain_db: f32,
	noise_db: f32,
	speech_db: Option<f32>,
	// Since the mic level controller last looked: speech slices, speech
	// slices that needed more than MIC_RAISE_ABOVE_GAIN_DB of gain, and
	// slices whose raw input came close to clipping.
	speech_slices: u32,
	quiet_speech_slices: u32,
	clipped_slices: u32,
}

impl VoiceAgc {
	fn new() -> Self {
		Self {
			gain_db: AGC_INITIAL_GAIN_DB,
			noise_db: -70.0,
			speech_db: None,
			speech_slices: 0,
			quiet_speech_slices: 0,
			clipped_slices: 0,
		}
	}

	#[cfg(test)]
	fn process(&mut self, frame: &mut [f32]) {
		for slice in frame.chunks_mut(AGC_SLICE) {
			self.process_slice(slice);
		}
	}

	fn process_slice(&mut self, slice: &mut [f32]) {
		if slice.is_empty() {
			return;
		}
		let rms = (slice.iter().map(|s| s * s).sum::<f32>() / slice.len() as f32).sqrt();
		let level_db = 20.0 * rms.max(1e-6).log10();
		let old_gain_db = self.gain_db;
		if slice.iter().any(|s| s.abs() > MIC_CLIP_LEVEL) {
			self.clipped_slices += 1;
		}
		if rms > 0.0 {
			// Noise floor: follows quiet slices at once, creeps up 1 dB/s.
			self.noise_db = if level_db < self.noise_db { level_db } else { self.noise_db + 0.01 };
			if level_db > AGC_MIN_SPEECH_DBFS && level_db > self.noise_db + AGC_SPEECH_ABOVE_NOISE_DB {
				// About half a second of speech to settle.
				self.speech_db = Some(match self.speech_db {
					Some(prev) => prev + 0.02 * (level_db - prev),
					None => level_db,
				});
				self.speech_slices += 1;
				if AGC_TARGET_DBFS - level_db > MIC_RAISE_ABOVE_GAIN_DB {
					self.quiet_speech_slices += 1;
				}
			}
			if let Some(speech_db) = self.speech_db {
				let want =
					(AGC_TARGET_DBFS - speech_db).min(AGC_MAX_NOISE_DBFS - self.noise_db).clamp(0.0, AGC_MAX_GAIN_DB);
				let secs = slice.len() as f32 / NETWORK_SAMPLE_RATE as f32;
				self.gain_db += (want - self.gain_db)
					.clamp(-AGC_MAX_GAIN_DROP_DB_PER_S * secs, AGC_MAX_GAIN_CHANGE_DB_PER_S * secs);
			}
		}
		let from = 10f32.powf(old_gain_db / 20.0);
		let to = 10f32.powf(self.gain_db / 20.0);
		let n = slice.len() as f32;
		for (i, sample) in slice.iter_mut().enumerate() {
			let g = from + (to - from) * (i + 1) as f32 / n;
			*sample *= g;
		}
	}
}

// Turns the background down a further GATE_DEPTH_DB between words, so the
// noise left by the suppressor isn't heard on its own once you stop talking.
// Opens within a few ms when the noise suppressor hears speech (its verdict
// is one hop ahead of its output, so word starts aren't cut), stays open
// GATE_HOLD_SLICES after, then fades down slowly.
const GATE_DEPTH_DB: f32 = 12.0;
const GATE_OPEN_SPEECH: f32 = 0.4;
const GATE_HOLD_SLICES: u32 = 25; // 250 ms
const GATE_ATTACK_SECS: f32 = 0.005;
const GATE_RELEASE_SECS: f32 = 0.15;

struct PauseGate {
	gain: f32,
	hold: u32,
	floor: f32,
	attack: f32,
	release: f32,
}

impl PauseGate {
	fn new() -> Self {
		let coeff = |secs: f32| 1.0 - (-1.0 / (secs * NETWORK_SAMPLE_RATE as f32)).exp();
		Self {
			gain: 1.0,
			hold: GATE_HOLD_SLICES,
			floor: 10f32.powf(-GATE_DEPTH_DB / 20.0),
			attack: coeff(GATE_ATTACK_SECS),
			release: coeff(GATE_RELEASE_SECS),
		}
	}

	fn process_slice(&mut self, slice: &mut [f32], speech: f32) {
		if speech >= GATE_OPEN_SPEECH {
			self.hold = GATE_HOLD_SLICES;
		} else {
			self.hold = self.hold.saturating_sub(1);
		}
		let (target, rate) = if self.hold > 0 { (1.0, self.attack) } else { (self.floor, self.release) };
		for sample in slice.iter_mut() {
			self.gain += (target - self.gain) * rate;
			*sample *= self.gain;
		}
	}
}

// Noise suppression before the auto gain, like WhatsApp for Windows' engine
// and WebRTC. WhatsApp's server asks its engine for denoiser_intensity 0.55,
// but on our scale that left too much background in a live call, so calls
// use the same strength as voice messages (about 20 dB).
const CALL_DENOISE_INTENSITY: f32 = 0.8;
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
const MIC_CLIP_LEVEL: f32 = 0.95;
const MIC_RAISE_ABOVE_GAIN_DB: f32 = 20.0;
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
	fn new(sample_rate: u32, channels: u16, system: Arc<Mutex<SystemMix>>) -> Self {
		Self {
			downsampler: Downsampler::new(sample_rate, channels),
			resampled: Vec::new(),
			denoiser: Denoiser::new(NETWORK_SAMPLE_RATE, CALL_DENOISE_INTENSITY),
			denoised: vec![0.0; AGC_SLICE],
			limiter: Limiter::new(
				NETWORK_SAMPLE_RATE as f32,
				CALL_LIMIT_LOOKAHEAD,
				CALL_LIMIT_CEILING,
				CALL_LIMIT_RELEASE_SECS,
			),
			sample_accum: Vec::with_capacity(FRAME_SAMPLES * 2),
			system,
			frame: Vec::with_capacity(FRAME_SAMPLES),
			send_buf: Vec::with_capacity(FRAME_SAMPLES * 4 + OPUS_MAX_PACKET),
			opus: new_call_opus_encoder(),
			agc: VoiceAgc::new(),
			gate: PauseGate::new(),
			slices_since_mic_check: 0,
		}
	}

	fn process(
		&mut self,
		data: &[f32],
		muted: bool,
		socket_out: &UdpSocket,
		target_addr: &str,
		mic: Option<&mut MicLevelController>,
	) {
		// mic -> noise suppressor -> auto gain -> + system audio -> limiter.
		self.downsampler.process(data, &mut self.resampled);
		let mut system = self.system.lock().unwrap();
		let mut used = 0;
		while self.resampled.len() - used >= AGC_SLICE {
			let speech = self.denoiser.process_hop(&self.resampled[used..used + AGC_SLICE], &mut self.denoised);
			used += AGC_SLICE;
			if !muted {
				self.agc.process_slice(&mut self.denoised);
				self.gate.process_slice(&mut self.denoised, speech);
				self.slices_since_mic_check += 1;
			}
			for &sample in &self.denoised {
				let mixed = system.mix(sample);
				let [limited] = self.limiter.process([if muted { 0.0 } else { mixed }]);
				self.sample_accum.push(limited);
			}
		}
		drop(system);
		self.resampled.drain(..used);

		while self.sample_accum.len() >= FRAME_SAMPLES {
			self.frame.clear();
			self.frame.extend(self.sample_accum.drain(..FRAME_SAMPLES));
			pack_call_frame(&self.frame, self.opus.as_mut(), &mut self.send_buf);
			let _ = socket_out.send_to(&self.send_buf, target_addr);
		}
		if self.slices_since_mic_check >= MIC_CHECK_SLICES {
			self.slices_since_mic_check = 0;
			if let Some(mic) = mic {
				mic.adjust(&mut self.agc);
			}
		}
	}
}

const PLAYOUT_REFILL_SAMPLES: usize = FRAME_SAMPLES * 2;

struct RenderDsp {
	channels: u16,
	src_accum: VecDeque<f32>,
	resample_phase: f64,
	upsample_step: f64,
	max_buffered_src: usize,
	refilling: bool,
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
			refilling: true,
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
		while self.src_accum.len() < needed.max(PLAYOUT_REFILL_SAMPLES) {
			match frame_rx.try_recv() {
				Ok(frame) => self.src_accum.extend(frame),
				Err(_) => break,
			}
		}
		while self.src_accum.len() > self.max_buffered_src {
			self.src_accum.pop_front();
		}

		if self.refilling {
			if self.src_accum.len() < PLAYOUT_REFILL_SAMPLES {
				data.fill(0.0);
				self.starved.fetch_add(out_frames as u64, Ordering::Relaxed);
				self.accum_len.store(self.src_accum.len(), Ordering::Relaxed);
				return;
			}
			self.refilling = false;
		}

		let (mut played, mut starved) = (0u64, 0u64);
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
				// Refill before playing again.
				starved += 1;
				self.refilling = true;
				0.0
			};
			if idx < self.src_accum.len() {
				played += 1;
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
		self.played.fetch_add(played, Ordering::Relaxed);
		self.starved.fetch_add(starved, Ordering::Relaxed);
		self.accum_len.store(self.src_accum.len(), Ordering::Relaxed);
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
) {
	while !stop.load(Ordering::Relaxed) {
		if let Err(e) =
			run_capture_session(stop, is_muted, socket_out, target_addr, preferred_device, force_reopen, system)
		{
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
) -> crate::error::Result<()> {
	unsafe {
		CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
		let _com_guard = ComGuard;

		let session_device = preferred_device.lock().unwrap().clone();
		force_reopen.store(false, Ordering::Relaxed);
		let wasapi = open_communications_stream(eCapture, "capture", session_device.as_deref(), true)?;
		let _event_guard = EventGuard(wasapi.event);
		let capture_client: IAudioCaptureClient = wasapi.client.GetService()?;

		let mut dsp = CaptureDsp::new(wasapi.sample_rate, wasapi.channels, Arc::clone(system));
		log_line(&format!(
			"capture dsp: noise suppressor {CALL_DENOISE_INTENSITY}, auto gain to {AGC_TARGET_DBFS} dBFS, pause gate -{GATE_DEPTH_DB} dB, limiter {CALL_LIMIT_CEILING}"
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
) -> crate::error::Result<()> {
	unsafe {
		CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
		let _com_guard = ComGuard;

		let session_device = preferred_device.lock().unwrap().clone();
		force_reopen.store(false, Ordering::Relaxed);
		let wasapi = open_communications_stream(eRender, "render", session_device.as_deref(), true)?;
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
		let mut buf = vec![0u8; 65536];
		while !reader_stop_thread.load(Ordering::Relaxed) {
			match socket_read.recv_from(&mut buf) {
				Ok((size, _)) => {
					reader_recv_count_thread.fetch_add(1, Ordering::Relaxed);
					let frame: Vec<f32> = buf[..size]
						.as_chunks::<4>()
						.0
						.iter()
						.map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
						.collect();
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
	let force_reopen_input = Arc::new(AtomicBool::new(false));
	let force_reopen_output = Arc::new(AtomicBool::new(false));
	let system_mix = Arc::new(Mutex::new(SystemMix::default()));

	{
		let stop = Arc::clone(&stop);
		let is_muted = Arc::clone(&is_muted);
		let target_addr = target_addr.clone();
		let preferred_input = Arc::clone(&preferred_input);
		let force_reopen_input = Arc::clone(&force_reopen_input);
		let system_mix = Arc::clone(&system_mix);
		std::thread::spawn(move || {
			run_capture(
				&stop,
				&is_muted,
				&socket_out,
				&target_addr,
				&preferred_input,
				&force_reopen_input,
				&system_mix,
			);
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

	Ok(CallAudioState {
		is_muted,
		stop,
		preferred_input,
		preferred_output,
		force_reopen_input,
		force_reopen_output,
		system_mix,
		system_audio: None,
	})
}

#[cfg(test)]
mod tests {

	fn speechy(amp: f32, frames: usize) -> Vec<Vec<f32>> {
		// 300 ms talk, 200 ms pause, repeated, over a faint noise floor.
		(0..frames)
			.map(|f| {
				(0..FRAME_SAMPLES)
					.map(|i| {
						let t = f * FRAME_SAMPLES + i;
						let talking = (t / 1600) % 5 < 3;
						let noise = 0.0003 * (((t * 7919) % 1000) as f32 / 500.0 - 1.0);
						noise + if talking { amp * 1.414 * ((t as f32) * 0.3).sin() } else { 0.0 }
					})
					.collect()
			})
			.collect()
	}

	fn speech_rms_db(frames: &[Vec<f32>]) -> f32 {
		let loud: Vec<f32> = frames
			.iter()
			.flatten()
			.copied()
			.collect::<Vec<_>>()
			.chunks(160)
			.map(|c| (c.iter().map(|s| s * s).sum::<f32>() / c.len() as f32).sqrt())
			.filter(|r| *r > 0.003)
			.collect();
		let mean = loud.iter().sum::<f32>() / loud.len() as f32;
		20.0 * mean.log10()
	}

	#[test]
	fn agc_brings_quiet_speech_up_to_target() {
		let mut agc = VoiceAgc::new();
		let mut frames = speechy(0.01, 200); // -40 dBFS speech, 12 s
		for f in frames.iter_mut() {
			agc.process(f);
		}
		let level = speech_rms_db(&frames[150..]);
		assert!((level - AGC_TARGET_DBFS).abs() < 3.0, "settled speech level {level} dBFS");
	}

	#[test]
	fn agc_brings_loud_speech_down_to_unity_within_a_second() {
		let mut agc = VoiceAgc::new();
		let mut frames = speechy(0.2, 20); // -14 dBFS speech
		for f in frames.iter_mut() {
			agc.process(f);
		}
		assert!(agc.gain_db < 0.5, "gain {} dB", agc.gain_db);
	}

	#[test]
	fn agc_holds_gain_through_silence() {
		let mut agc = VoiceAgc::new();
		agc.gain_db = 12.0;
		let mut silence = vec![0.0f32; FRAME_SAMPLES];
		agc.process(&mut silence);
		assert_eq!(agc.gain_db, 12.0);
		assert!(silence.iter().all(|s| *s == 0.0));
	}

	fn tone_at(rate: u32, hz: f32, secs: f32) -> Vec<f32> {
		(0..(rate as f32 * secs) as usize)
			.map(|i| 0.5 * (2.0 * std::f32::consts::PI * hz * i as f32 / rate as f32).sin())
			.collect()
	}

	fn level_db(x: &[f32]) -> f32 {
		10.0 * (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32 + 1e-20).log10()
	}

	#[test]
	fn pause_gate_opens_fast_holds_then_fades_the_background() {
		let mut gate = PauseGate::new();
		let mut slice = vec![1.0f32; AGC_SLICE];
		gate.process_slice(&mut slice, 1.0);
		assert!(slice[80] > 0.99, "open within 5 ms");
		for _ in 0..GATE_HOLD_SLICES - 1 {
			slice.fill(1.0);
			gate.process_slice(&mut slice, 0.0);
			assert!(slice.iter().all(|&s| s > 0.99), "held open after speech");
		}
		for _ in 0..100 {
			slice.fill(1.0);
			gate.process_slice(&mut slice, 0.0);
		}
		let db = 20.0 * slice[AGC_SLICE - 1].log10();
		assert!((db + GATE_DEPTH_DB).abs() < 0.5, "settled at {db} dB");
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
		let mut decoder = opus::Decoder::new(NETWORK_SAMPLE_RATE, opus::Channels::Mono).expect("libopus decoder");
		let mut datagram = Vec::new();
		let mut energy = 0.0f32;
		for n in 0..10 {
			let frame = tone_frame(n * FRAME_SAMPLES);
			pack_call_frame(&frame, Some(&mut encoder), &mut datagram);

			let (pcm, packet) = datagram.split_at(FRAME_SAMPLES * 4);
			let sent: Vec<f32> =
				pcm.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
			assert_eq!(sent, frame, "the PCM half is the frame, untouched");

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

	fn render_dsp() -> RenderDsp {
		let counter = || Arc::new(AtomicU64::new(0));
		RenderDsp::new(NETWORK_SAMPLE_RATE, 1, counter(), counter(), Arc::new(AtomicUsize::new(0)))
	}

	#[test]
	fn playback_waits_for_the_refill_level_again_after_running_dry() {
		let mut dsp = render_dsp();
		let (tx, rx) = mpsc::sync_channel::<Vec<f32>>(8);
		let mut out = vec![0.0f32; 160];

		tx.send(vec![0.5; FRAME_SAMPLES]).unwrap();
		dsp.process(&mut out, &rx);
		assert!(out.iter().all(|&s| s == 0.0), "started before the refill level");

		tx.send(vec![0.5; FRAME_SAMPLES]).unwrap();
		dsp.process(&mut out, &rx);
		assert!(out.iter().all(|&s| s == 0.5), "did not start once the refill level arrived");

		for _ in 0..(2 * FRAME_SAMPLES / out.len()) {
			dsp.process(&mut out, &rx);
		}
		assert!(dsp.refilling, "running dry did not start a refill");

		tx.send(vec![0.25; FRAME_SAMPLES]).unwrap();
		dsp.process(&mut out, &rx);
		assert!(out.iter().all(|&s| s == 0.0), "played a lone frame before refilling");
		tx.send(vec![0.25; FRAME_SAMPLES]).unwrap();
		dsp.process(&mut out, &rx);
		assert!(out.iter().all(|&s| s == 0.25), "did not resume after refilling");
	}

	#[test]
	fn a_call_frame_without_an_encoder_is_pcm_alone() {
		let mut datagram = vec![1, 2, 3];
		pack_call_frame(&tone_frame(0), None, &mut datagram);
		assert_eq!(datagram.len(), FRAME_SAMPLES * 4);
	}
}
