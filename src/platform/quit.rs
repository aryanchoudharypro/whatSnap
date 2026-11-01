//! Ending the app, so that Exit really ends it.
//!
//! Modelled on Fedra's exit: the tray icon and the windows are put away first,
//! before any teardown starts, so nothing is left in the notification area and a
//! screen reader isn't walked through windows being destroyed. Then, unlike
//! Fedra, the process is ended outright once the backend has stopped, because
//! whatSnap has more that can keep a process alive after its window has gone: a
//! dialog's modal loop still running under the main loop, wxWidgets' teardown of
//! windows that were built once and only ever hidden, audio drivers' DLL
//! detach code. Whatever of that hangs, the app still goes away.

use std::{
	sync::{
		Condvar, Mutex, OnceLock,
		atomic::{AtomicBool, Ordering},
	},
	time::Duration,
};

type Hook = Box<dyn Fn() + Send + Sync>;

static BACKEND_SHUTDOWN: OnceLock<Hook> = OnceLock::new();
static STARTED: AtomicBool = AtomicBool::new(false);
static BACKEND_DONE: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

/// How long the window code gets, once the backend is down, to finish closing
/// before the process is ended from the quit thread.
const UI_GRACE: Duration = Duration::from_secs(8);
/// Longest wait for the backend: its own shutdown gives it 5 s before killing
/// it, plus up to 2 s to get through to its pipe.
const BACKEND_WAIT: Duration = Duration::from_secs(15);

/// How to stop the backend gracefully (ask it to quit, then wait or kill).
/// Set once at startup.
pub fn on_backend_shutdown(f: impl Fn() + Send + Sync + 'static) {
	let _ = BACKEND_SHUTDOWN.set(Box::new(f));
}

/// Whether Exit has started. Nothing may bring the tray icon or a window back
/// once it has.
pub fn is_quitting() -> bool {
	STARTED.load(Ordering::SeqCst)
}

/// Starts quitting: stops the backend on a thread of its own, so it runs while
/// the UI tears down rather than after it, and arms the watchdog that ends the
/// process if the UI never gets to [`finish`]. Safe to call more than once.
pub fn begin() {
	if STARTED.swap(true, Ordering::SeqCst) {
		return;
	}
	let spawned = std::thread::Builder::new().name("quit".into()).spawn(|| {
		shut_down_backend();
		std::thread::sleep(UI_GRACE);
		crate::backend::events::ui_timing_log(
			"Exit: the window code didn't finish closing in time after the backend stopped; ending the process",
		);
		end_process();
	});
	if spawned.is_err() {
		shut_down_backend();
	}
}

fn shut_down_backend() {
	if let Some(f) = BACKEND_SHUTDOWN.get() {
		f();
	}
	let (done, cv) = &BACKEND_DONE;
	*done.lock().unwrap_or_else(|e| e.into_inner()) = true;
	cv.notify_all();
}

/// Waits for the backend to be down, then ends the process. Called once the
/// main loop has returned, and on Windows logoff.
pub fn finish() -> ! {
	begin();
	let (done, cv) = &BACKEND_DONE;
	let guard = done.lock().unwrap_or_else(|e| e.into_inner());
	let _ = cv.wait_timeout_while(guard, BACKEND_WAIT, |done| !*done);
	end_process()
}

/// Ends the process now, without running DLL detach code or destructors: by
/// this point there is nothing left to save, and those are exactly what can
/// hang. The backend's kill-on-close job and the single-instance mutex are
/// released by Windows as the process goes.
pub fn end_process() -> ! {
	use windows::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
	unsafe {
		let _ = TerminateProcess(GetCurrentProcess(), 0);
	}
	std::process::exit(0)
}
