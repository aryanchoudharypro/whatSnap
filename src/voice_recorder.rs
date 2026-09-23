//! WASAPI-based recorder for voice messages, honoring the user's chosen input
//! device.
//!
//! Always writes 48kHz stereo 16-bit PCM, matching what the old MCI-based
//! recorder always produced (it opened the waveaudio device with a hardcoded
//! "channels 2", regardless of what the mic actually captured) and what
//! `convert_wav_to_opus_ogg` (audio.rs) requires of its input.
//!
//! When Settings -> "Clean up voice messages" is on, the signal goes through
//! `voice_dsp::VoiceCleanup` (high-pass, noise suppression, automatic gain,
//! limiter) on its way to the file - the same processing the browser gives
//! WhatsApp Web's recordings. Windows' own communications effects are not used
//! here: they vary by driver, often do nothing, and would stack a second noise
//! suppressor on top.

use std::{
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
		mpsc,
	},
	thread::JoinHandle,
	time::Duration,
};

use hound::{SampleFormat, WavSpec, WavWriter};
use windows::Win32::{
	Foundation::WAIT_OBJECT_0,
	Media::Audio::{AUDCLNT_BUFFERFLAGS_SILENT, IAudioCaptureClient, eCapture},
	System::{
		Com::{COINIT_MULTITHREADED, CoInitializeEx},
		Threading::WaitForSingleObject,
	},
};

use crate::{
	call_audio::{ComGuard, EventGuard, log_line, open_communications_stream},
	voice_dsp::VoiceCleanup,
};

const OUTPUT_SAMPLE_RATE: u32 = 48000;

/// One open recording segment. Dropped/finished sessions leave behind a
/// complete, independently-playable WAV file at the path passed to `start`.
pub struct RecordingSession {
	stop: Arc<AtomicBool>,
	thread: Option<JoinHandle<()>>,
}

impl RecordingSession {
	/// Opens the input stream and starts writing to `path`, blocking briefly
	/// until the device is confirmed open (or failed) so the caller can give
	/// the same immediate success/failure feedback the old MCI-based
	/// recorder gave.
	pub fn start(path: &str, preferred_device_name: Option<String>, clean_up: bool) -> anyhow::Result<Self> {
		let stop = Arc::new(AtomicBool::new(false));
		let stop_thread = Arc::clone(&stop);
		let path = path.to_string();
		let (ready_tx, ready_rx) = mpsc::channel::<anyhow::Result<()>>();

		let thread = std::thread::spawn(move || {
			run_recording_session(&stop_thread, &path, preferred_device_name.as_deref(), clean_up, ready_tx);
		});

		match ready_rx.recv_timeout(Duration::from_secs(3)) {
			Ok(Ok(())) => Ok(Self { stop, thread: Some(thread) }),
			Ok(Err(e)) => {
				let _ = thread.join();
				Err(e)
			}
			Err(_) => {
				stop.store(true, Ordering::Relaxed);
				let _ = thread.join();
				Err(anyhow::anyhow!("timed out opening the recording device"))
			}
		}
	}

	/// Like `start`, but opens the device on a background thread instead of
	/// blocking the caller - `start`'s brief block is fine on a worker
	/// thread but freezes the whole UI when called from the wx event
	/// handler. `on_ready` runs on that background thread once the device
	/// has been confirmed open (or failed), so it must hand the result off
	/// rather than touch UI state directly.
	pub fn start_async(
		path: &str,
		preferred_device_name: Option<String>,
		clean_up: bool,
		on_ready: impl FnOnce(anyhow::Result<Self>) + Send + 'static,
	) {
		let path = path.to_string();
		std::thread::spawn(move || {
			let result = Self::start(&path, preferred_device_name, clean_up);
			on_ready(result);
		});
	}

	/// Stops capture and finalizes the WAV file.
	pub fn finish(mut self) {
		self.stop.store(true, Ordering::Relaxed);
		if let Some(t) = self.thread.take() {
			let _ = t.join();
		}
	}
}

impl Drop for RecordingSession {
	fn drop(&mut self) {
		// Safety net for a session that's dropped without an explicit
		// `finish()` call (e.g. an early-return error path elsewhere) - still
		// signal the capture thread to stop and flush rather than leaking it.
		self.stop.store(true, Ordering::Relaxed);
	}
}

fn run_recording_session(
	stop: &AtomicBool,
	path: &str,
	preferred_device_name: Option<&str>,
	clean_up: bool,
	ready_tx: mpsc::Sender<anyhow::Result<()>>,
) {
	if let Err(e) = run_recording_session_inner(stop, path, preferred_device_name, clean_up, ready_tx) {
		log_line(&format!("voice recording session ended with error: {:?}", e));
	}
}

fn run_recording_session_inner(
	stop: &AtomicBool,
	path: &str,
	preferred_device_name: Option<&str>,
	clean_up: bool,
	ready_tx: mpsc::Sender<anyhow::Result<()>>,
) -> anyhow::Result<()> {
	unsafe {
		CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
		let _com_guard = ComGuard;

		let wasapi = match open_communications_stream(eCapture, "voice-recording", preferred_device_name, false) {
			Ok(w) => w,
			Err(e) => {
				let _ = ready_tx.send(Err(e));
				return Ok(());
			}
		};
		let _event_guard = EventGuard(wasapi.event);
		let capture_client: IAudioCaptureClient = wasapi.client.GetService()?;

		let spec = WavSpec {
			channels: 2,
			sample_rate: OUTPUT_SAMPLE_RATE,
			bits_per_sample: 16,
			sample_format: SampleFormat::Int,
		};
		let mut writer = match WavWriter::create(path, spec) {
			Ok(w) => w,
			Err(e) => {
				let _ = ready_tx.send(Err(anyhow::anyhow!("failed to create wav file: {}", e)));
				return Ok(());
			}
		};

		let mut resampler = StereoResampler::new(wasapi.sample_rate, wasapi.channels);
		let mut cleanup = clean_up.then(VoiceCleanup::new);
		let mut scratch: Vec<f32> = Vec::new();
		let write = |writer: &mut WavWriter<_>, frames: &[[f32; 2]]| {
			for [left, right] in frames {
				let _ = writer.write_sample((left.clamp(-1.0, 1.0) * 32767.0) as i16);
				let _ = writer.write_sample((right.clamp(-1.0, 1.0) * 32767.0) as i16);
			}
		};

		wasapi.client.Start()?;
		let _ = ready_tx.send(Ok(()));
		log_line("voice recording stream started");

		while !stop.load(Ordering::Relaxed) {
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

				capture_client.ReleaseBuffer(num_frames)?;

				let frames = resampler.process(&scratch);
				match cleanup.as_mut() {
					Some(c) => write(&mut writer, &c.process(&frames)),
					None => write(&mut writer, &frames),
				}
			}
		}

		wasapi.client.Stop()?;
		if let Some(c) = cleanup.as_mut() {
			write(&mut writer, &c.finish());
		}
		let _ = writer.finalize();
		log_line("voice recording session exiting (stop set)");
	}
	Ok(())
}

// Maps an arbitrary-channel, device-native-rate signal to a stereo pair at
// `OUTPUT_SAMPLE_RATE` via linear interpolation, same technique as
// call_audio.rs's CaptureDsp/RenderDsp. A mono source is duplicated to L=R
// (matching what the old MCI-based recorder produced from a mono mic - it
// always opened the device as 2-channel, and Windows' own format conversion
// did this same duplication); a source with more than 2 channels keeps just
// the first two. The anti-aliasing low-pass those use before decimating to
// 16kHz is only engaged here when the device's native rate is above
// `OUTPUT_SAMPLE_RATE` (uncommon, but some pro-audio interfaces do this) -
// skipping it when upsampling or already at the target rate avoids dulling
// the signal for the common case.
struct StereoResampler {
	channels: u16,
	apply_lowpass: bool,
	lp1: [f32; 2],
	lp2: [f32; 2],
	lp_alpha: f32,
	accum: [Vec<f32>; 2],
	resample_phase: f64,
	step: f64,
}

impl StereoResampler {
	fn new(sample_rate: u32, channels: u16) -> Self {
		let apply_lowpass = sample_rate > OUTPUT_SAMPLE_RATE;
		// See CaptureDsp in call_audio.rs for why the per-stage cutoff is
		// scaled up from the intended cascaded -3dB point.
		let lp_target_hz: f32 = (OUTPUT_SAMPLE_RATE as f32 / 2.0) * 0.9;
		let lp_cutoff_hz = lp_target_hz / (2.0_f32.sqrt() - 1.0).sqrt();
		let lp_rc = 1.0 / (2.0 * std::f32::consts::PI * lp_cutoff_hz);
		let lp_dt = 1.0 / sample_rate as f32;
		let lp_alpha = lp_dt / (lp_rc + lp_dt);
		Self {
			channels: channels.max(1),
			apply_lowpass,
			lp1: [0.0; 2],
			lp2: [0.0; 2],
			lp_alpha,
			accum: [Vec::new(), Vec::new()],
			resample_phase: 0.0,
			step: sample_rate as f64 / OUTPUT_SAMPLE_RATE as f64,
		}
	}

	fn process(&mut self, data: &[f32]) -> Vec<[f32; 2]> {
		for frame in data.chunks(self.channels as usize) {
			let raw = [frame[0], if frame.len() > 1 { frame[1] } else { frame[0] }];
			for c in 0..2 {
				let filtered = if self.apply_lowpass {
					self.lp1[c] += self.lp_alpha * (raw[c] - self.lp1[c]);
					self.lp2[c] += self.lp_alpha * (self.lp1[c] - self.lp2[c]);
					self.lp2[c]
				} else {
					raw[c]
				};
				self.accum[c].push(filtered);
			}
		}

		let mut out = Vec::new();
		while self.resample_phase.floor() as usize + 1 < self.accum[0].len() {
			let idx = self.resample_phase.floor() as usize;
			let frac = (self.resample_phase - idx as f64) as f32;
			let mut sample = [0.0f32; 2];
			for c in 0..2 {
				sample[c] = self.accum[c][idx] + (self.accum[c][idx + 1] - self.accum[c][idx]) * frac;
			}
			out.push(sample);
			self.resample_phase += self.step;
		}
		let consumed = (self.resample_phase.floor() as usize).min(self.accum[0].len());
		if consumed > 0 {
			self.accum[0].drain(0..consumed);
			self.accum[1].drain(0..consumed);
			self.resample_phase -= consumed as f64;
		}
		out
	}
}
