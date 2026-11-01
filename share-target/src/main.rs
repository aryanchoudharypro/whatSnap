#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod package;
#[cfg(windows)]
mod share;
#[cfg(windows)]
mod sign;

#[cfg(windows)]
fn main() {
	let args: Vec<String> = std::env::args().skip(1).collect();
	let has = |flag: &str| args.iter().any(|a| a == flag);
	note_crashes();
	package::log(&format!("Started with {args:?}"));
	let code = if has("--sign") {
		match sign::sign_package() {
			Ok(()) => 0,
			Err(e) => {
				package::log(&format!("Signing failed: {e}"));
				let _ = std::fs::write(package::sign_error_file(), &e);
				1
			}
		}
	} else if has("--enable") {
		package::enable(!has("--quiet"))
	} else if has("--refresh") {
		package::refresh()
	} else if has("--disable") {
		package::disable()
	} else {
		share::handle_share_activation()
	};
	std::process::exit(code);
}

/// Logs a panic or a crash before the process goes, so a failure in the
/// administrator step (which nobody sees) still leaves a trace.
#[cfg(windows)]
fn note_crashes() {
	use windows::Win32::System::Diagnostics::Debug::{EXCEPTION_POINTERS, SetUnhandledExceptionFilter};

	std::panic::set_hook(Box::new(|info| package::log(&format!("Panicked: {info}"))));
	unsafe extern "system" fn on_crash(pointers: *const EXCEPTION_POINTERS) -> i32 {
		let code = unsafe { pointers.as_ref().and_then(|p| p.ExceptionRecord.as_ref()).map(|r| r.ExceptionCode.0) };
		package::log(&format!("Crashed with exception {:#x}", code.unwrap_or(0)));
		0
	}
	unsafe {
		SetUnhandledExceptionFilter(Some(on_crash));
	}
}

#[cfg(not(windows))]
fn main() {}
