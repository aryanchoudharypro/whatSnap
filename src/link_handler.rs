//! Registers whatSnap as a handler for whatsapp:// links, the kind "Share on
//! WhatsApp" buttons and wa.me pages open. Everything goes under
//! HKEY_CURRENT_USER, so no admin rights are needed, and it's rewritten on
//! every start so the path follows the exe if it moves.
//!
//! Two parts: a plain `whatsapp` URL class, which Windows uses when nothing
//! else claims the scheme, and a registered-application entry, which lists
//! whatSnap in Settings > Apps > Default apps so it can be chosen when another
//! app (such as the official WhatsApp) also handles these links.

#![cfg(target_os = "windows")]

use windows::{
	Win32::System::Registry::{
		HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ, RegCloseKey,
		RegCreateKeyExW, RegGetValueW, RegSetValueExW,
	},
	core::PCWSTR,
};

// Registry keys keep the old name: changing them would leave a stale entry behind.
const PROG_ID: &str = "whaNTV.WhatsAppLink";
const CAPABILITIES_KEY: &str = "Software\\whaNTV\\Capabilities";

fn wide(s: &str) -> Vec<u16> {
	s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Sets `name` (None for the key's default value) under HKCU\`path`,
/// creating the key if needed.
fn set_string(path: &str, name: Option<&str>, value: &str) -> bool {
	let key_name = wide(path);
	let value_name = name.map(wide);
	let data = wide(value);
	unsafe {
		let mut hkey = HKEY::default();
		if RegCreateKeyExW(
			HKEY_CURRENT_USER,
			PCWSTR(key_name.as_ptr()),
			None,
			PCWSTR::null(),
			REG_OPTION_NON_VOLATILE,
			KEY_SET_VALUE,
			None,
			&mut hkey,
			None,
		)
		.is_err()
		{
			return false;
		}
		let bytes = std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), data.len() * 2);
		let name_ptr = value_name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr()));
		let ok = RegSetValueExW(hkey, name_ptr, None, REG_SZ, Some(bytes)).is_ok();
		let _ = RegCloseKey(hkey);
		ok
	}
}

/// Reads the default value of HKCU\`path`, if the key exists.
fn get_default_string(path: &str) -> Option<String> {
	let key_name = wide(path);
	let mut buffer = vec![0u16; 2048];
	let mut size = (buffer.len() * 2) as u32;
	unsafe {
		RegGetValueW(
			HKEY_CURRENT_USER,
			PCWSTR(key_name.as_ptr()),
			PCWSTR::null(),
			RRF_RT_REG_SZ,
			None,
			Some(buffer.as_mut_ptr().cast()),
			Some(&mut size),
		)
		.ok()
		.ok()?;
	}
	let len = (size as usize / 2).saturating_sub(1);
	Some(String::from_utf16_lossy(&buffer[..len.min(buffer.len())]))
}

fn write_url_class(class_key: &str, description: &str, command: &str, icon: &str) {
	set_string(class_key, None, description);
	set_string(class_key, Some("URL Protocol"), "");
	set_string(&format!("{}\\DefaultIcon", class_key), None, icon);
	set_string(&format!("{}\\shell\\open\\command", class_key), None, command);
}

/// Registers whatSnap for whatsapp:// links. Doesn't take over a plain
/// `whatsapp` class another program already set up in the user's registry;
/// the Default apps entry still lets the user pick whatSnap.
pub fn register() {
	let Ok(exe) = std::env::current_exe() else { return };
	let exe = exe.display().to_string();
	let command = format!("\"{}\" \"%1\"", exe);
	let icon = format!("\"{}\",0", exe);

	write_url_class(&format!("Software\\Classes\\{}", PROG_ID), "URL:WhatsApp link", &command, &icon);

	// whatsnap:// links come from our own toast notifications (see app_link.rs).
	write_url_class("Software\\Classes\\whatsnap", "URL:whatSnap link", &command, &icon);

	let class_command_key = "Software\\Classes\\whatsapp\\shell\\open\\command";
	let ours_or_unset = match get_default_string(class_command_key) {
		None => true,
		Some(existing) => {
			let existing = existing.to_lowercase();
			existing.contains("whantv") || existing.contains("whatsnap")
		}
	};
	if ours_or_unset {
		write_url_class("Software\\Classes\\whatsapp", "URL:WhatsApp link", &command, &icon);
	}

	set_string(CAPABILITIES_KEY, Some("ApplicationName"), "whatSnap");
	set_string(
		CAPABILITIES_KEY,
		Some("ApplicationDescription"),
		"An accessible WhatsApp client. Opens WhatsApp share and chat links.",
	);
	set_string(&format!("{}\\URLAssociations", CAPABILITIES_KEY), Some("whatsapp"), PROG_ID);
	set_string("Software\\RegisteredApplications", Some("whaNTV"), CAPABILITIES_KEY);
}
