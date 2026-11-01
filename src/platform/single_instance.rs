//! Detects whether another whatSnap instance is already running and, if so,
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
		UI::WindowsAndMessaging::{
			AllowSetForegroundWindow, FindWindowW, GetWindowThreadProcessId, MB_ICONWARNING, MB_OK, MessageBoxW,
		},
	},
	core::w,
};

use crate::backend::pipe::{PipeListener, PipeStream};

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

/// What a launch was started to do, passed on to the running instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchRequest {
	/// A WhatsApp or whatSnap link (whatSnap is the whatsapp:// handler).
	Link(String),
	/// Files to send, from Explorer's Send to menu.
	Files(Vec<String>),
}

/// Marks a hand-off that lists files rather than a link. A link is one line
/// and starts with its scheme, so it can't be mistaken for this.
const FILES_HEADER: &str = "files\n";

impl LaunchRequest {
	/// The text after "activate\n" in a hand-off message. Windows paths
	/// can't hold a newline, so one path per line is unambiguous.
	fn encode(&self) -> String {
		match self {
			LaunchRequest::Link(link) => link.clone(),
			LaunchRequest::Files(paths) => format!("{FILES_HEADER}{}", paths.join("\n")),
		}
	}

	fn decode(text: &str) -> Option<LaunchRequest> {
		if let Some(paths) = text.strip_prefix(FILES_HEADER) {
			let paths: Vec<String> = paths.lines().filter(|p| !p.is_empty()).map(str::to_string).collect();
			return (!paths.is_empty()).then_some(LaunchRequest::Files(paths));
		}
		(!text.is_empty()).then(|| LaunchRequest::Link(text.to_string()))
	}
}

/// Returns `Some(lock)` if this is the first running instance — hold the
/// lock for the process's lifetime and call `listen_for_activation` once
/// the UI command channel exists. Returns `None` if another instance is
/// already running, after asking it to activate itself, so the caller
/// should exit immediately without spawning the backend or opening a window.
///
/// `request` is what this launch was started to do (open a link, send
/// files); it's passed on to the running instance.
pub fn acquire_or_activate_existing(request: Option<&LaunchRequest>) -> Option<InstanceLock> {
	// A running copy can be slow to answer: it may still be starting (its
	// pipe opens only once its window exists), or it may be closing (it
	// holds the mutex until its process ends, long after its pipe has gone).
	// So keep trying for a while, and take over as soon as it has gone.
	let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
	loop {
		let Ok(handle) = (unsafe { CreateMutexW(None, true, MUTEX_NAME) }) else {
			// Can't even tell whether another instance is running; err on the
			// side of letting this one start rather than locking the user out.
			return Some(InstanceLock { handle: HANDLE::default() });
		};
		if unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
			return Some(InstanceLock { handle });
		}
		unsafe {
			let _ = CloseHandle(handle);
		}
		match request_activate(request) {
			Activation::Done => return None,
			// A copy started as administrator never lets this one in, so
			// waiting won't help.
			Activation::Denied => break,
			Activation::NotListening if std::time::Instant::now() >= deadline => break,
			Activation::NotListening => std::thread::sleep(std::time::Duration::from_millis(200)),
		}
	}
	show_unreachable_message();
	None
}

/// Tells the user why this launch did nothing. Without it, a copy that can't
/// reach the running one (for example one started as administrator, which
/// normal launches can't talk to) would just exit and look broken.
fn show_unreachable_message() {
	unsafe {
		MessageBoxW(
			None,
			w!(
				"whatSnap is already running, but this copy can't reach it. It may have been started as administrator. Close whatSnap in Task Manager, then start it again."
			),
			w!("whatSnap"),
			MB_OK | MB_ICONWARNING,
		);
	}
}

enum Activation {
	/// The running instance got the request.
	Done,
	/// Its pipe refused this process, as one started as administrator does.
	Denied,
	/// Nothing is listening yet (still starting) or any more (closing).
	NotListening,
}

/// Tells the already-running instance to raise its window, and hands it what
/// this launch was asked to do, if anything.
fn request_activate(request: Option<&LaunchRequest>) -> Activation {
	// Grant the existing instance's process temporary permission to steal
	// the foreground: on Windows, a process can only call
	// SetForegroundWindow successfully if it currently holds that right or
	// was just granted it. This process, having just been launched
	// (typically by direct user action), usually already holds it and can
	// pass it along.
	unsafe {
		if let Ok(hwnd) = FindWindowW(None, w!("whatSnap")) {
			let mut pid = 0u32;
			GetWindowThreadProcessId(hwnd, Some(&mut pid));
			if pid != 0 {
				let _ = AllowSetForegroundWindow(pid);
			}
		}
	}

	let message = match request {
		Some(request) => format!("activate\n{}", request.encode()),
		None => "activate".to_string(),
	};
	match PipeStream::connect(ACTIVATE_PIPE_NAME) {
		Ok(mut stream) => {
			let _ = stream.write_all(message.as_bytes());
			Activation::Done
		}
		Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Activation::Denied,
		Err(_) => Activation::NotListening,
	}
}

/// Starts listening (on a background thread) for activation requests from
/// future launches of the app, invoking `on_activate` each time one arrives,
/// with what that launch was asked to do, if anything. Call once, after the UI
/// command channel is ready to receive the result.
pub fn listen_for_activation(on_activate: impl Fn(Option<LaunchRequest>) + Send + 'static) {
	std::thread::spawn(move || {
		// The pipe can still be held for a moment by a copy that is closing
		// (say, the one an update just replaced), so keep trying to open it.
		let mut listener = loop {
			match PipeListener::bind(ACTIVATE_PIPE_NAME) {
				Ok(listener) => break listener,
				Err(_) => std::thread::sleep(std::time::Duration::from_millis(500)),
			}
		};
		loop {
			let Ok(mut conn) = listener.accept() else {
				// Couldn't make the next instance; don't spin on it.
				std::thread::sleep(std::time::Duration::from_secs(1));
				continue;
			};
			// The sender writes one short message and closes; it's capped
			// well above any real link or list of files so a stray client
			// can't make this grow without bound.
			let mut message = String::new();
			let _ = (&mut conn).take(1024 * 1024).read_to_string(&mut message);
			let request = message.split_once('\n').and_then(|(_, rest)| LaunchRequest::decode(rest));
			on_activate(request);
		}
	});
}

#[cfg(test)]
mod tests {
	use super::LaunchRequest;

	#[test]
	fn hand_off_round_trips() {
		for request in [
			LaunchRequest::Link("whatsapp://send?text=hi".to_string()),
			LaunchRequest::Files(vec![r"C:\Users\a\photo.jpg".to_string(), r"D:\notes final.pdf".to_string()]),
		] {
			assert_eq!(LaunchRequest::decode(&request.encode()), Some(request));
		}
		assert_eq!(LaunchRequest::decode(""), None);
		assert_eq!(LaunchRequest::decode("files\n"), None);
	}
}
