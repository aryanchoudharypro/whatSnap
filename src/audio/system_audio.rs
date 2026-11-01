//! Captures what the computer is playing, for a screen share that carries
//! sound too. WhatsApp calls carry one audio stream per person, so the sound is
//! mixed into the mic frames `calls/audio.rs` sends instead of travelling on
//! its own.
//!
//! The capture is Windows' process loopback (Windows 10 2004 and later) in
//! exclude mode: everything every other program plays, minus whatSnap and the
//! backend it started. A plain loopback of the speakers would also pick up
//! the call itself and send the others their own voices back.

use std::{
	collections::VecDeque,
	sync::{
		Arc, Mutex,
		atomic::{AtomicBool, Ordering},
		mpsc,
	},
	time::Duration,
};

use windows::{
	Win32::{
		Foundation::WAIT_OBJECT_0,
		Media::Audio::{
			AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
			AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
			AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0,
			AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
			ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
			IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
			IAudioCaptureClient, IAudioClient, PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
			VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVE_FORMAT_PCM, WAVEFORMATEX,
		},
		System::{
			Com::{
				BLOB, COINIT_MULTITHREADED, CoInitializeEx,
				StructuredStorage::{PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0},
			},
			Threading::{CreateEventW, GetCurrentProcessId, WaitForSingleObject},
			Variant::VT_BLOB,
		},
	},
	core::{HRESULT, IUnknown, Interface, PCWSTR, Ref},
};

use crate::{
	UiCommand,
	calls::audio::{ComGuard, Downsampler, EventGuard, log_line},
	ui::wake::UiCommandSender,
};

/// What the capture asks Windows for; Windows converts whatever is playing.
const CAPTURE_RATE: u32 = 48_000;
const CAPTURE_CHANNELS: u16 = 2;
/// 16 kHz samples held back before mixing starts, and again after the queue
/// runs dry: the two capture threads wake on their own clocks, so mixing
/// from an empty queue would cut the sound into pieces.
const MIX_PRIME: usize = 960;
/// More than this queued means the system clock runs ahead of the mic's;
/// the oldest sound is dropped back down to `MIX_PRIME`.
const MIX_MAX: usize = 4000;
/// System audio goes in a little under full scale so a voice over loud music
/// stays clear of clipping.
const MIX_GAIN: f32 = 0.7;

/// System audio at 16 kHz, between its capture thread and the mic thread.
#[derive(Default)]
pub(crate) struct SystemMix {
	queue: VecDeque<f32>,
	primed: bool,
}

impl SystemMix {
	pub(crate) fn push(&mut self, samples: &[f32]) {
		self.queue.extend(samples);
		if self.queue.len() > MIX_MAX {
			let excess = self.queue.len() - MIX_PRIME;
			self.queue.drain(..excess);
		}
	}

	/// One mic sample, with the next system sample added when there is one.
	pub(crate) fn mix(&mut self, voice: f32) -> f32 {
		if !self.primed {
			if self.queue.len() < MIX_PRIME {
				return voice;
			}
			self.primed = true;
		}
		match self.queue.pop_front() {
			Some(system) => (voice + system * MIX_GAIN).clamp(-1.0, 1.0),
			None => {
				self.primed = false;
				voice
			}
		}
	}

	pub(crate) fn clear(&mut self) {
		self.queue.clear();
		self.primed = false;
	}
}

/// A running system audio capture; dropping it stops the thread.
pub(crate) struct SystemAudioCapture {
	stop: Arc<AtomicBool>,
}

impl Drop for SystemAudioCapture {
	fn drop(&mut self) {
		self.stop.store(true, Ordering::Relaxed);
	}
}

pub(crate) fn start(mix: Arc<Mutex<SystemMix>>, ui_tx: UiCommandSender) -> SystemAudioCapture {
	let stop = Arc::new(AtomicBool::new(false));
	let thread_stop = Arc::clone(&stop);
	let spawned = std::thread::Builder::new().name("system audio".into()).spawn(move || {
		// Like the mic, a session that fails mid-stream (the default speakers
		// changing, say) is reopened. Only a first open that fails is
		// reported, since then there's nothing being shared at all.
		let mut first = true;
		while !thread_stop.load(Ordering::Relaxed) {
			if let Err(e) = run_session(&thread_stop, &mix, &mut first) {
				log_line(&format!("system audio: session ended with error: {:?}", e));
				if first {
					let _ = ui_tx.send(UiCommand::SystemAudioFailed(e.to_string()));
					break;
				}
				std::thread::sleep(Duration::from_millis(500));
			}
		}
		mix.lock().unwrap().clear();
		log_line("system audio: thread exiting");
	});
	if let Err(e) = spawned {
		log_line(&format!("system audio: couldn't start its thread: {e}"));
	}
	SystemAudioCapture { stop }
}

#[windows::core::implement(IActivateAudioInterfaceCompletionHandler)]
struct Activated {
	done: mpsc::SyncSender<()>,
}

impl IActivateAudioInterfaceCompletionHandler_Impl for Activated_Impl {
	fn ActivateCompleted(&self, _operation: Ref<IActivateAudioInterfaceAsyncOperation>) -> windows::core::Result<()> {
		let _ = self.done.try_send(());
		Ok(())
	}
}

/// An audio client on everything playing except this process tree.
unsafe fn activate_loopback() -> crate::error::Result<IAudioClient> {
	unsafe {
		let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
			ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
			Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
				ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
					TargetProcessId: GetCurrentProcessId(),
					ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
				},
			},
		};
		// A VT_BLOB pointing at `params`, which outlives the call. It must
		// never be dropped: the windows crate's PROPVARIANT runs
		// PropVariantClear on drop, which hands the blob pointer to
		// CoTaskMemFree, and freeing `params` (on the stack) crashed the app.
		let variant = std::mem::ManuallyDrop::new(PROPVARIANT {
			Anonymous: PROPVARIANT_0 {
				Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
					vt: VT_BLOB,
					wReserved1: 0,
					wReserved2: 0,
					wReserved3: 0,
					Anonymous: PROPVARIANT_0_0_0 {
						blob: BLOB {
							cbSize: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
							pBlobData: &mut params as *mut _ as *mut u8,
						},
					},
				}),
			},
		});
		let (done_tx, done_rx) = mpsc::sync_channel(1);
		let handler: IActivateAudioInterfaceCompletionHandler = Activated { done: done_tx }.into();
		let operation = ActivateAudioInterfaceAsync(
			VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
			&IAudioClient::IID,
			Some(&*variant),
			&handler,
		)?;
		log_line("system audio: activation requested");
		if done_rx.recv_timeout(Duration::from_secs(5)).is_err() {
			crate::error::bail!("Windows didn't open the system audio in time");
		}
		let mut result = HRESULT(0);
		let mut activated: Option<IUnknown> = None;
		log_line("system audio: activation completed");
		operation.GetActivateResult(&mut result, &mut activated)?;
		log_line(&format!("system audio: activation result {:#010x}", result.0 as u32));
		result.ok()?;
		let Some(activated) = activated else {
			crate::error::bail!("Windows returned no system audio stream");
		};
		Ok(activated.cast()?)
	}
}

fn run_session(stop: &AtomicBool, mix: &Mutex<SystemMix>, first: &mut bool) -> crate::error::Result<()> {
	unsafe {
		CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
		let _com_guard = ComGuard;

		// Each step is logged: a crash in here leaves nothing else behind.
		log_line("system audio: opening process loopback");
		let client = activate_loopback()?;
		log_line("system audio: loopback opened");
		// Process loopback has no mix format to ask for, so the format is
		// ours to pick and Windows converts to it.
		let block_align = CAPTURE_CHANNELS * 2;
		let format = WAVEFORMATEX {
			wFormatTag: WAVE_FORMAT_PCM as u16,
			nChannels: CAPTURE_CHANNELS,
			nSamplesPerSec: CAPTURE_RATE,
			nAvgBytesPerSec: CAPTURE_RATE * block_align as u32,
			nBlockAlign: block_align,
			wBitsPerSample: 16,
			cbSize: 0,
		};
		client.Initialize(
			AUDCLNT_SHAREMODE_SHARED,
			AUDCLNT_STREAMFLAGS_LOOPBACK
				| AUDCLNT_STREAMFLAGS_EVENTCALLBACK
				| AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
				| AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
			200_000,
			0,
			&format,
			None,
		)?;
		log_line("system audio: stream initialised");
		let event = CreateEventW(None, false, false, PCWSTR::null())?;
		let _event_guard = EventGuard(event);
		client.SetEventHandle(event)?;
		let capture_client: IAudioCaptureClient = client.GetService()?;
		client.Start()?;
		*first = false;
		log_line("system audio: capture started");

		let mut downsampler = Downsampler::new(CAPTURE_RATE, CAPTURE_CHANNELS);
		let mut scratch: Vec<f32> = Vec::new();
		let mut resampled: Vec<f32> = Vec::new();
		while !stop.load(Ordering::Relaxed) {
			// Loopback only signals while something is playing; the timeout
			// keeps the stop flag checked through silence.
			if WaitForSingleObject(event, 200) != WAIT_OBJECT_0 {
				continue;
			}
			loop {
				if capture_client.GetNextPacketSize()? == 0 {
					break;
				}
				let mut data_ptr: *mut u8 = std::ptr::null_mut();
				let mut num_frames: u32 = 0;
				let mut flags: u32 = 0;
				capture_client.GetBuffer(&mut data_ptr, &mut num_frames, &mut flags, None, None)?;
				let total_samples = num_frames as usize * CAPTURE_CHANNELS as usize;
				scratch.clear();
				if data_ptr.is_null() || (flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 {
					scratch.resize(total_samples, 0.0);
				} else {
					let src = std::slice::from_raw_parts(data_ptr as *const i16, total_samples);
					scratch.extend(src.iter().map(|&s| s as f32 / 32768.0));
				}
				capture_client.ReleaseBuffer(num_frames)?;

				resampled.clear();
				downsampler.process(&scratch, &mut resampled);
				mix.lock().unwrap().push(&resampled);
			}
		}
		client.Stop()?;
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn mixing_waits_for_a_primed_queue_then_adds_the_sound() {
		let mut mix = SystemMix::default();
		mix.push(&[0.5; MIX_PRIME - 1]);
		assert_eq!(mix.mix(0.1), 0.1, "mixed before the queue was primed");
		mix.push(&[0.5]);
		assert!((mix.mix(0.1) - (0.1 + 0.5 * MIX_GAIN)).abs() < 1e-6);
	}

	#[test]
	fn running_dry_waits_to_prime_again() {
		let mut mix = SystemMix::default();
		mix.push(&[0.5; MIX_PRIME]);
		for _ in 0..MIX_PRIME {
			mix.mix(0.0);
		}
		assert_eq!(mix.mix(0.2), 0.2);
		mix.push(&[0.5; 10]);
		assert_eq!(mix.mix(0.2), 0.2, "mixed a few late samples before priming again");
	}

	#[test]
	fn a_queue_running_ahead_drops_its_oldest_sound() {
		let mut mix = SystemMix::default();
		mix.push(&[0.1; MIX_MAX]);
		mix.push(&[0.9]);
		assert_eq!(mix.queue.len(), MIX_PRIME);
		assert_eq!(mix.queue.back(), Some(&0.9));
	}

	#[test]
	fn the_mix_never_clips_past_full_scale() {
		let mut mix = SystemMix::default();
		mix.push(&[1.0; MIX_PRIME]);
		assert_eq!(mix.mix(0.9), 1.0);
	}
}
