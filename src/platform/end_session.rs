//! Lets Windows close whatSnap for an update or a sign-out, and start it again
//! after an update.
//!
//! The installer replaces the running exe by asking the Restart Manager to
//! close it, which sends the same WM_QUERYENDSESSION/WM_ENDSESSION pair a
//! sign-out does. wx answers the query by closing the main frame without
//! forcing it, and while logged in that close hides to the tray and vetoes,
//! so the installer was told whatSnap refused to close. The frame is subclassed
//! here to always agree, and to shut the backend down cleanly when the session
//! really ends instead of leaving that to wx, which exits the process from
//! inside the message without ever reaching `main`'s own cleanup.
//!
//! Registering for restart is what lets the installer start whatSnap again once
//! the new files are in place.

#![cfg(target_os = "windows")]

use windows::{
	Win32::{
		Foundation::{HWND, LPARAM, LRESULT, WPARAM},
		System::Recovery::{RESTART_NO_CRASH, RESTART_NO_HANG, RegisterApplicationRestart},
		UI::{
			Shell::{DefSubclassProc, SetWindowSubclass},
			WindowsAndMessaging::{WM_ENDSESSION, WM_QUERYENDSESSION},
		},
	},
	core::PCWSTR,
};

const SUBCLASS_ID: usize = 0x656e_6473;

/// Hooks session end on the main frame. `on_end` runs once Windows has
/// committed to closing whatSnap, just before the process exits; it should stop
/// the backend.
pub fn install(frame: &wxdragon::prelude::Frame, on_end: impl Fn() + 'static) {
	use wxdragon::prelude::*;
	// No command line: a restart is an ordinary launch. Only restarts the
	// Restart Manager asks for (an update, or Windows bringing apps back after
	// it restarts) - a crash or hang shouldn't relaunch it behind the user.
	unsafe {
		let _ = RegisterApplicationRestart(PCWSTR::null(), RESTART_NO_CRASH | RESTART_NO_HANG);
	}
	let on_end: Box<Box<dyn Fn()>> = Box::new(Box::new(on_end));
	let hwnd = HWND(frame.get_handle());
	unsafe {
		// The frame lives as long as the process, so the hook is never freed.
		let _ = SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, Box::into_raw(on_end) as usize);
	}
}

unsafe extern "system" fn subclass_proc(
	hwnd: HWND,
	msg: u32,
	wparam: WPARAM,
	lparam: LPARAM,
	_id: usize,
	data: usize,
) -> LRESULT {
	match msg {
		// Hiding to the tray is for the close button, not for Windows asking.
		WM_QUERYENDSESSION => LRESULT(1),
		WM_ENDSESSION => {
			if wparam.0 != 0 {
				let on_end = unsafe { &*(data as *const Box<dyn Fn()>) };
				on_end();
				// Exiting here rather than ending the main loop: a modal dialog
				// left open runs a loop of its own, which would keep the process
				// alive until Windows gave up and killed it.
				std::process::exit(0);
			}
			LRESULT(0)
		}
		_ => unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
	}
}
