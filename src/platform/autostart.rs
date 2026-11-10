use windows::{
	Win32::System::Registry::{
		HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
		RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
	},
	core::PCWSTR,
};

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE_NAME: &str = "whaNTV";
/// Carried by the Run entry so a start with Windows stays in the tray.
const BACKGROUND_FLAG: &str = "--background";

static BACKGROUND_START: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Reads the command line once at startup. Returns whether Windows started us.
pub fn note_launch_args(mut args: impl Iterator<Item = String>) -> bool {
	let background = args.any(|arg| arg == BACKGROUND_FLAG);
	BACKGROUND_START.store(background, std::sync::atomic::Ordering::SeqCst);
	background
}

/// Starts this run in the tray, as if Windows had started it: for a copy
/// started by a notification's button, which shouldn't open the window.
pub fn start_hidden() {
	BACKGROUND_START.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// The user asked for the window (clicked a notification) before it first
/// showed: it shows after all.
pub fn show_on_start() {
	BACKGROUND_START.store(false, std::sync::atomic::Ordering::SeqCst);
}

/// Whether this run was started by Windows at sign-in, not by the user.
pub fn started_in_background() -> bool {
	BACKGROUND_START.load(std::sync::atomic::Ordering::SeqCst)
}

/// Rewrites an existing Run entry, so one written by an older version (or by
/// the installer) gains the background flag and the current path.
pub fn refresh() {
	if is_enabled() {
		set_enabled(true);
	}
}

fn wide(s: &str) -> Vec<u16> {
	s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn is_enabled() -> bool {
	let key_name = wide(RUN_KEY);
	let value_name = wide(VALUE_NAME);
	unsafe {
		let mut hkey = Default::default();
		if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(key_name.as_ptr()), Some(0), KEY_QUERY_VALUE, &mut hkey).is_err() {
			return false;
		}
		let found = RegQueryValueExW(hkey, PCWSTR(value_name.as_ptr()), None, None, None, None).is_ok();
		let _ = RegCloseKey(hkey);
		found
	}
}

pub fn set_enabled(enabled: bool) -> bool {
	let key_name = wide(RUN_KEY);
	let value_name = wide(VALUE_NAME);
	unsafe {
		let mut hkey = Default::default();
		let opened = RegCreateKeyExW(
			HKEY_CURRENT_USER,
			PCWSTR(key_name.as_ptr()),
			None,
			PCWSTR::null(),
			REG_OPTION_NON_VOLATILE,
			KEY_SET_VALUE,
			None,
			&mut hkey,
			None,
		);
		if opened.is_err() {
			return false;
		}

		let ok = if enabled {
			let Ok(exe) = std::env::current_exe() else {
				let _ = RegCloseKey(hkey);
				return false;
			};
			let quoted = format!("\"{}\" {BACKGROUND_FLAG}", exe.display());
			let data = wide(&quoted);
			let bytes = std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), data.len() * 2);
			RegSetValueExW(hkey, PCWSTR(value_name.as_ptr()), None, REG_SZ, Some(bytes)).is_ok()
		} else {
			let result = RegDeleteValueW(hkey, PCWSTR(value_name.as_ptr()));
			result.is_ok() || result == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND
		};

		let _ = RegCloseKey(hkey);
		ok
	}
}
