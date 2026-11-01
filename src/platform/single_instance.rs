#![cfg(target_os = "windows")]

use std::{
	io::{Read, Write},
	sync::Mutex,
};

use windows::{
	Win32::{
		Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE},
		System::Threading::CreateMutexW,
		UI::WindowsAndMessaging::{AllowSetForegroundWindow, MB_ICONWARNING, MB_OK, MessageBoxW},
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
	let message = match request {
		Some(request) => format!("activate\n{}", request.encode()),
		None => "activate".to_string(),
	};
	match PipeStream::connect(ACTIVATE_PIPE_NAME) {
		Ok(mut stream) => {
			// Only the foreground process (this one, just launched) may hand
			// the foreground on. The pipe names the copy that answers, which a
			// window title can't do reliably: the main window may not exist
			// yet, and other windows share the title.
			if let Some(pid) = stream.server_pid() {
				unsafe {
					let _ = AllowSetForegroundWindow(pid);
				}
			}
			let _ = stream.write_all(message.as_bytes());
			Activation::Done
		}
		Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Activation::Denied,
		Err(_) => Activation::NotListening,
	}
}

type ActivationHandler = Box<dyn Fn(Option<LaunchRequest>) + Send>;

/// Who handles a hand-off, and the hand-offs that came before there was
/// anyone to.
struct Activations {
	handler: Option<ActivationHandler>,
	waiting: Vec<Option<LaunchRequest>>,
}

static ACTIVATIONS: Mutex<Activations> = Mutex::new(Activations { handler: None, waiting: Vec::new() });

fn deliver(request: Option<LaunchRequest>) {
	let mut activations = ACTIVATIONS.lock().unwrap_or_else(|e| e.into_inner());
	match &activations.handler {
		Some(handler) => handler(request),
		None => activations.waiting.push(request),
	}
}

/// Says what to do with a hand-off from a later launch, including any that
/// arrived while the window was still being built.
pub fn on_activation(handler: impl Fn(Option<LaunchRequest>) + Send + 'static) {
	let mut activations = ACTIVATIONS.lock().unwrap_or_else(|e| e.into_inner());
	for request in std::mem::take(&mut activations.waiting) {
		handler(request);
	}
	activations.handler = Some(Box::new(handler));
}

/// Opens the pipe later launches hand off to. Called as soon as this copy
/// holds the mutex, so a launch never finds the mutex taken and nobody
/// answering while this copy is still starting up; what arrives early waits
/// for [`on_activation`].
pub fn listen_for_activation() {
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
			// Read each hand-off on its own thread: one that connects and
			// never finishes must not keep every later launch waiting.
			std::thread::spawn(move || {
				let mut message = String::new();
				let _ = (&mut conn).take(1024 * 1024).read_to_string(&mut message);
				if !message.starts_with("activate") {
					return;
				}
				deliver(message.split_once('\n').and_then(|(_, rest)| LaunchRequest::decode(rest)));
			});
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
