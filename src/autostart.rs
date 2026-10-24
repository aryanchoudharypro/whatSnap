//! Launch-at-login support via the per-user `Run` registry key. The registry
//! is the source of truth (not the JSON settings file) so the checkbox always
//! reflects what Windows will actually do, even if the setting file and the
//! registry ever drift apart.

use windows::{
	Win32::System::Registry::{
		HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
		RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
	},
	core::PCWSTR,
};

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
// Kept as the old name so existing "start with Windows" entries still match.
const VALUE_NAME: &str = "whaNTV";

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
			let quoted = format!("\"{}\"", exe.display());
			let data = wide(&quoted);
			let bytes = std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), data.len() * 2);
			RegSetValueExW(hkey, PCWSTR(value_name.as_ptr()), None, REG_SZ, Some(bytes)).is_ok()
		} else {
			// Absence of the value is the disabled state, so a missing value
			// on delete is not a failure.
			let result = RegDeleteValueW(hkey, PCWSTR(value_name.as_ptr()));
			result.is_ok() || result == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND
		};

		let _ = RegCloseKey(hkey);
		ok
	}
}
