//! whatSnapShare.exe: puts whatSnap in the Windows Share menu.
//!
//! Started by the Share menu (no arguments), it collects what was shared
//! (files, a picture, text or a link) and starts whatSnap.exe with it, the same
//! way Explorer's Send to does; whatSnap then asks which chats to send it to.
//!
//! The menu only lists packaged apps, so this exe belongs to a small sparse
//! package (packaging/share/), which has to be signed by a certificate Windows
//! trusts. Nothing is signed when whatSnap is built: turning the menu entry on
//! makes a certificate on this PC, signs the package with it, trusts it, and
//! deletes its private key, so there's no shared key that could sign anything
//! else. Trusting it needs administrator rights once; registering the package
//! is per user and doesn't.
//!
//! Command lines (from the installer, the uninstaller and whatSnap's Settings):
//! - `--enable [--quiet]`: turn the menu entry on, asking for administrator
//!   rights if the package needs signing, and say how it went unless quiet.
//! - `--refresh`: after an update, register again if it was turned on.
//! - `--disable`: remove the menu entry.
//! - `--sign`: the step that needs administrator rights; run by `--enable`.

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
		0 // EXCEPTION_CONTINUE_SEARCH
	}
	unsafe {
		SetUnhandledExceptionFilter(Some(on_crash));
	}
}

#[cfg(not(windows))]
fn main() {}
