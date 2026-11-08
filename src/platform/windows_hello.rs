//! Windows Hello as a second way into locked chats: the fingerprint, face or
//! PIN prompt Windows already shows, in place of typing the secret code.
//!
//! This uses Windows' consent prompt (UserConsentVerifier), which runs in the
//! user's own session and covers every sign-in method the PC has set up. The
//! answer comes back to whatSnap, so it's a convenience lock like the secret
//! code (also checked here), not a barrier against code running as the user.

use std::sync::atomic::{AtomicBool, Ordering};

use windows::{
	Security::Credentials::UI::{UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability},
	Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
	core::HSTRING,
};
use windows_future::IAsyncOperation;

/// What Windows Hello can do on this PC for the current user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
	/// A fingerprint, face or PIN is set up, so the prompt can be shown.
	Ready,
	/// Windows Hello is not set up for this user, or the PC has no way to use it.
	NotSetUp,
	/// Windows can't offer it right now (busy, or turned off by policy).
	Unavailable,
}

/// The outcome of a Windows Hello verification request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyOutcome {
	/// Verified successfully via fingerprint, face, or PIN.
	Verified,
	/// The user explicitly dismissed or canceled the prompt.
	Canceled,
	/// Verification failed (wrong biometric, retries exhausted, device busy, or error).
	Failed,
}

/// A prompt is on screen. A second request while one is open is refused, so
/// two Windows Hello prompts never stack up.
static IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// Initializes WinRT on the current thread for as long as it lives. The
/// threads that call into WinRT here are fresh, so the mode is always ours.
struct WinRtApartment;

impl WinRtApartment {
	fn enter() -> windows::core::Result<Self> {
		unsafe { RoInitialize(RO_INIT_MULTITHREADED)? };
		Ok(Self)
	}
}

impl Drop for WinRtApartment {
	fn drop(&mut self) {
		unsafe { RoUninitialize() };
	}
}

/// Runs `work` on a fresh thread and waits for it. WinRT's blocking `get()`
/// can wait on a completion that needs a message-pumping thread, so it never
/// runs on the UI thread itself.
fn on_worker<T: Send>(work: impl FnOnce() -> T + Send) -> Option<T> {
	std::thread::scope(|scope| scope.spawn(work).join().ok())
}

/// Whether Windows Hello can be asked for right now. Quick, but it does wait
/// for Windows, so call it when a dialog opens rather than in a loop.
pub fn availability() -> Availability {
	let answer = on_worker(|| -> windows::core::Result<UserConsentVerifierAvailability> {
		let _apartment = WinRtApartment::enter()?;
		UserConsentVerifier::CheckAvailabilityAsync()?.join()
	});
	match answer {
		Some(Ok(a)) if a == UserConsentVerifierAvailability::Available => Availability::Ready,
		Some(Ok(a))
			if a == UserConsentVerifierAvailability::NotConfiguredForUser
				|| a == UserConsentVerifierAvailability::DeviceNotPresent =>
		{
			Availability::NotSetUp
		}
		_ => Availability::Unavailable,
	}
}

/// Shows the Windows Hello prompt with `message` as its text. `done` then runs
/// on a worker thread with the verification outcome.
/// Returns `false` straight away, without calling `done`, when a prompt is already open.
///
/// The prompt has no owner window on purpose. Parenting it to any window made
/// Windows disable that window while the prompt was up, and it stayed
/// "unavailable" afterwards (the main window, then the hidden Settings dialog).
pub fn verify(message: String, done: impl FnOnce(VerifyOutcome) + Send + 'static) -> bool {
	if IN_FLIGHT.swap(true, Ordering::SeqCst) {
		return false;
	}
	let spawned = std::thread::Builder::new().name("windows-hello".into()).spawn(move || {
		let outcome = ask(&message).unwrap_or(VerifyOutcome::Failed);
		IN_FLIGHT.store(false, Ordering::SeqCst);
		done(outcome);
	});
	if spawned.is_err() {
		IN_FLIGHT.store(false, Ordering::SeqCst);
		return false;
	}
	true
}

/// The blocking part of [`verify`]; runs on its own thread.
fn ask(message: &str) -> windows::core::Result<VerifyOutcome> {
	let _apartment = WinRtApartment::enter()?;
	let operation: IAsyncOperation<UserConsentVerificationResult> =
		UserConsentVerifier::RequestVerificationAsync(&HSTRING::from(message))?;
	let result = operation.join()?;
	Ok(match result {
		UserConsentVerificationResult::Verified => VerifyOutcome::Verified,
		UserConsentVerificationResult::Canceled => VerifyOutcome::Canceled,
		_ => VerifyOutcome::Failed,
	})
}
