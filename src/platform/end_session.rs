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
		WM_QUERYENDSESSION => LRESULT(1),
		WM_ENDSESSION => {
			if wparam.0 != 0 {
				let on_end = unsafe { &*(data as *const Box<dyn Fn()>) };
				on_end();
				crate::platform::quit::end_process();
			}
			LRESULT(0)
		}
		_ => unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
	}
}
