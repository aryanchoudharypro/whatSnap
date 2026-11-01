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

const MUTEX_NAME: windows::core::PCWSTR = w!("whaNTV-SingleInstance-9f3b2e7a-4c1d-4e2a-b6a0-mutex");
const ACTIVATE_PIPE_NAME: &str = "whantv-activate-9f3b2e7a";

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchRequest {
	Link(String),
	Files(Vec<String>),
}

const FILES_HEADER: &str = "files\n";

impl LaunchRequest {
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

pub fn acquire_or_activate_existing(request: Option<&LaunchRequest>) -> Option<InstanceLock> {
	let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
	loop {
		let Ok(handle) = (unsafe { CreateMutexW(None, true, MUTEX_NAME) }) else {
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
	Done,
	Denied,
	NotListening,
}

fn request_activate(request: Option<&LaunchRequest>) -> Activation {
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

pub fn listen_for_activation(on_activate: impl Fn(Option<LaunchRequest>) + Send + 'static) {
	std::thread::spawn(move || {
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
