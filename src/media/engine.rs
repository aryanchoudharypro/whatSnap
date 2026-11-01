//! Video (and audio) playback for the in-app player, on Media Foundation's
//! Media Engine: the same engine behind the HTML5 `<video>` element of
//! Windows' own web views, so it plays what WhatsApp sends (H.264/AAC MP4)
//! with nothing extra shipped. Given a window, it draws the picture into it
//! itself; the controls around it are ordinary wx ones (`ui/media_player.rs`).
//!
//! The engine reports what happens (loaded, ended, failed) on a thread of its
//! own. Those reports are only noted here, and the player's UI-thread timer
//! picks them up, so nothing crosses threads but a few atomics.

use std::sync::{
	Arc,
	atomic::{AtomicBool, AtomicU32, Ordering},
};

use windows::{
	Win32::{
		Foundation::{HWND, RECT},
		Media::MediaFoundation::{
			CLSID_MFMediaEngineClassFactory, IMFAttributes, IMFMediaEngine, IMFMediaEngineClassFactory,
			IMFMediaEngineEx, IMFMediaEngineNotify, IMFMediaEngineNotify_Impl, MF_MEDIA_ENGINE_CALLBACK,
			MF_MEDIA_ENGINE_EVENT_ENDED, MF_MEDIA_ENGINE_EVENT_ERROR, MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA,
			MF_MEDIA_ENGINE_PLAYBACK_HWND, MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, MFCreateAttributes,
		},
		System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
		UI::WindowsAndMessaging::GetClientRect,
	},
	core::{BSTR, Interface, Result as WinResult},
};

use crate::media::foundation;

/// DXGI_FORMAT_B8G8R8A8_UNORM, the picture format asked of the engine.
const OUTPUT_FORMAT_BGRA: u32 = 87;

/// What the engine has reported since it was last asked.
#[derive(Default)]
struct Reports {
	loaded: AtomicBool,
	ended: AtomicBool,
	/// MF_MEDIA_ENGINE_ERR, 0 for none.
	error: AtomicU32,
}

#[windows::core::implement(IMFMediaEngineNotify)]
struct Notify {
	reports: Arc<Reports>,
}

impl IMFMediaEngineNotify_Impl for Notify_Impl {
	fn EventNotify(&self, event: u32, param1: usize, _param2: u32) -> WinResult<()> {
		let event = event as i32;
		if event == MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA.0 {
			self.reports.loaded.store(true, Ordering::Release);
		} else if event == MF_MEDIA_ENGINE_EVENT_ENDED.0 {
			self.reports.ended.store(true, Ordering::Release);
		} else if event == MF_MEDIA_ENGINE_EVENT_ERROR.0 {
			// param1 is the MF_MEDIA_ENGINE_ERR code.
			self.reports.error.store((param1 as u32).max(1), Ordering::Release);
		}
		Ok(())
	}
}

/// What changed since the last [`MediaEngine::take_reports`].
pub(crate) struct Changes {
	pub(crate) loaded: bool,
	pub(crate) ended: bool,
	/// A sentence for the user, when playback failed.
	pub(crate) error: Option<&'static str>,
}

/// One file's playback, drawn into `hwnd`. Dropping it stops playback.
pub(crate) struct MediaEngine {
	engine: IMFMediaEngine,
	hwnd: HWND,
	reports: Arc<Reports>,
}

impl MediaEngine {
	/// Starts loading `path` (playback waits for [`Self::play`]). Must be
	/// called on the UI thread, which wx has already set up for COM.
	pub(crate) fn open(hwnd: *mut std::ffi::c_void, path: &str) -> crate::error::Result<Self> {
		foundation::ensure_started();
		let reports = Arc::new(Reports::default());
		unsafe {
			let factory: IMFMediaEngineClassFactory =
				CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)?;
			let mut attrs: Option<IMFAttributes> = None;
			MFCreateAttributes(&mut attrs, 3)?;
			let attrs = attrs.ok_or_else(|| crate::error::err!("MFCreateAttributes returned no attributes"))?;
			let notify: IMFMediaEngineNotify = Notify { reports: reports.clone() }.into();
			attrs.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify)?;
			attrs.SetUINT64(&MF_MEDIA_ENGINE_PLAYBACK_HWND, hwnd as u64)?;
			attrs.SetUINT32(&MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, OUTPUT_FORMAT_BGRA)?;
			let engine = factory.CreateInstance(0, &attrs)?;
			engine.SetSource(&BSTR::from(path))?;
			Ok(Self { engine, hwnd: HWND(hwnd), reports })
		}
	}

	pub(crate) fn take_reports(&self) -> Changes {
		let error = match self.reports.error.swap(0, Ordering::AcqRel) {
			0 => None,
			// MF_MEDIA_ENGINE_ERR_SRC_NOT_SUPPORTED
			4 => Some("This video's format can't be played here."),
			// MF_MEDIA_ENGINE_ERR_DECODE
			3 => Some("The video couldn't be decoded. The file may be damaged."),
			_ => Some("The video couldn't be played."),
		};
		Changes {
			loaded: self.reports.loaded.swap(false, Ordering::AcqRel),
			ended: self.reports.ended.swap(false, Ordering::AcqRel),
			error,
		}
	}

	pub(crate) fn play(&self) {
		unsafe {
			let _ = self.engine.Play();
		}
	}

	pub(crate) fn pause(&self) {
		unsafe {
			let _ = self.engine.Pause();
		}
	}

	pub(crate) fn is_paused(&self) -> bool {
		unsafe { self.engine.IsPaused().as_bool() }
	}

	pub(crate) fn is_ended(&self) -> bool {
		unsafe { self.engine.IsEnded().as_bool() }
	}

	/// Seconds into the file.
	pub(crate) fn position(&self) -> f64 {
		let at = unsafe { self.engine.GetCurrentTime() };
		if at.is_finite() { at.max(0.0) } else { 0.0 }
	}

	/// Length in seconds, 0 until known.
	pub(crate) fn duration(&self) -> f64 {
		let length = unsafe { self.engine.GetDuration() };
		if length.is_finite() { length.max(0.0) } else { 0.0 }
	}

	pub(crate) fn seek(&self, seconds: f64) {
		let length = self.duration();
		let target = if length > 0.0 { seconds.clamp(0.0, length) } else { seconds.max(0.0) };
		unsafe {
			let _ = self.engine.SetCurrentTime(target);
		}
	}

	/// 0.0 to 1.0.
	pub(crate) fn set_volume(&self, volume: f64) {
		unsafe {
			let _ = self.engine.SetVolume(volume.clamp(0.0, 1.0));
		}
	}

	pub(crate) fn is_muted(&self) -> bool {
		unsafe { self.engine.GetMuted().as_bool() }
	}

	pub(crate) fn set_muted(&self, muted: bool) {
		unsafe {
			let _ = self.engine.SetMuted(muted);
		}
	}

	pub(crate) fn set_rate(&self, rate: f64) {
		unsafe {
			let _ = self.engine.SetPlaybackRate(rate);
		}
	}

	/// Fits the picture to the window again, after it was resized.
	pub(crate) fn fit_to_window(&self) {
		unsafe {
			let mut rect = RECT::default();
			if GetClientRect(self.hwnd, &mut rect).is_err() {
				return;
			}
			if let Ok(ex) = self.engine.cast::<IMFMediaEngineEx>() {
				let _ = ex.UpdateVideoStream(None, Some(&rect), None);
			}
		}
	}
}

impl Drop for MediaEngine {
	fn drop(&mut self) {
		unsafe {
			// Shutdown releases the engine's hold on the callback and window;
			// without it the engine (and its decoder threads) outlive the player.
			let _ = self.engine.Shutdown();
		}
	}
}
