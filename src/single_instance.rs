//! Detects whether another whaNTV instance is already running and, if so,
//! asks it to activate its own window instead of starting a second copy.
//!
//! A brand-new process calling `SetForegroundWindow` on someone else's
//! window is subject to Windows' foreground-lock restrictions and would
//! just flash the taskbar icon instead of truly focusing it (so e.g.
//! Alt+F4 wouldn't reach it) — only the window's own process can reliably
//! steal focus for itself. So instead of activating the other instance's
//! window directly, this hands the request off over a local pipe and lets
//! that instance raise itself.

#![cfg(target_os = "windows")]

use std::io::{Read, Write};

use windows::{
	Win32::{
		Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE},
		System::Threading::CreateMutexW,
		UI::WindowsAndMessaging::{AllowSetForegroundWindow, FindWindowW, GetWindowThreadProcessId},
	},
	core::w,
};

use crate::pipe::{PipeListener, PipeStream};

/// Fixed, app-specific name so this can't collide with another app's mutex.
const MUTEX_NAME: windows::core::PCWSTR = w!("whaNTV-SingleInstance-9f3b2e7a-4c1d-4e2a-b6a0-mutex");
const ACTIVATE_PIPE_NAME: &str = "whantv-activate-9f3b2e7a";

/// Holds the OS mutex for the lifetime of the process; dropping it releases
/// the "an instance is running" claim so the next launch can take it over.
pub struct InstanceLock {
	handle: HANDLE,
}

impl Drop for InstanceLock {
	fn drop(&mut self) {
		unsafe {
			let _ = CloseHandle(self.handle);
		}
	}
}

/// Returns `Some(lock)` if this is the first running instance — hold the
/// lock for the process's lifetime and call `listen_for_activation` once
/// the UI command channel exists. Returns `None` if another instance is
/// already running, after asking it to activate itself, so the caller
/// should exit immediately without spawning the backend or opening a window.
///
/// `link` is a WhatsApp link this launch was started with (whaNTV is the
/// whatsapp:// handler); it's passed on to the running instance.
pub fn acquire_or_activate_existing(link: Option<&str>) -> Option<InstanceLock> {
	let Ok(handle) = (unsafe { CreateMutexW(None, true, MUTEX_NAME) }) else {
		// Can't even tell whether another instance is running; err on the
		// side of letting this one start rather than locking the user out.
		return Some(InstanceLock { handle: HANDLE::default() });
	};
	let already_running = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;

	if !already_running {
		return Some(InstanceLock { handle });
	}

	request_activate(link);
	unsafe {
		let _ = CloseHandle(handle);
	}
	None
}

/// Tells the already-running instance to raise its window, and hands it the
/// link this launch was given, if any.
fn request_activate(link: Option<&str>) {
	// Grant the existing instance's process temporary permission to steal
	// the foreground: on Windows, a process can only call
	// SetForegroundWindow successfully if it currently holds that right or
	// was just granted it. This process, having just been launched
	// (typically by direct user action), usually already holds it and can
	// pass it along.
	unsafe {
		if let Ok(hwnd) = FindWindowW(None, w!("whaNTV")) {
			let mut pid = 0u32;
			GetWindowThreadProcessId(hwnd, Some(&mut pid));
			if pid != 0 {
				let _ = AllowSetForegroundWindow(pid);
			}
		}
	}

	if let Ok(mut stream) = PipeStream::connect(ACTIVATE_PIPE_NAME) {
		let message = match link {
			Some(link) => format!("activate\n{}", link),
			None => "activate".to_string(),
		};
		let _ = stream.write_all(message.as_bytes());
	}
}

/// Starts listening (on a background thread) for activation requests from
/// future launches of the app, invoking `on_activate` each time one arrives,
/// with the link that launch was given, if any. Call once, after the UI
/// command channel is ready to receive the result.
pub fn listen_for_activation(on_activate: impl Fn(Option<String>) + Send + 'static) {
	let Ok(mut listener) = PipeListener::bind(ACTIVATE_PIPE_NAME) else { return };

	std::thread::spawn(move || {
		loop {
			let Ok(mut conn) = listener.accept() else {
				// Couldn't make the next instance; don't spin on it.
				std::thread::sleep(std::time::Duration::from_secs(1));
				continue;
			};
			// The sender writes one short message and closes; a link is
			// capped well above any real one so a stray client can't make
			// this grow without bound.
			let mut message = String::new();
			let _ = (&mut conn).take(64 * 1024).read_to_string(&mut message);
			let link = message.split_once('\n').map(|(_, link)| link.to_string()).filter(|l| !l.is_empty());
			on_activate(link);
		}
	});
}
