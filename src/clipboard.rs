//! Windows clipboard file-drop helpers, used when copying message
//! attachments (single files or whole albums) to the system clipboard so
//! they can be pasted into Explorer or another app, plus the plain text copy
//! every Copy menu item and button goes through.

/// Puts `text` on the clipboard and flushes it, so it is still there after
/// whatSnap fully exits. wxWidgets hands Windows an OLE data object that
/// lives in our process, and Windows empties the clipboard when that process
/// goes away unless it was flushed first (the same fix Fedra uses).
pub fn copy_text(text: &str) -> bool {
	let clipboard = wxdragon::prelude::Clipboard::get();
	let copied = clipboard.locker().is_some_and(|_locker| clipboard.set_text(text));
	if copied {
		let _ = clipboard.flush();
	}
	copied
}

#[cfg(target_os = "windows")]
mod win_cb {
	#[link(name = "user32")]
	unsafe extern "system" {
		pub fn OpenClipboard(hwnd: *mut std::ffi::c_void) -> i32;
		pub fn CloseClipboard() -> i32;
		pub fn EmptyClipboard() -> i32;
		pub fn SetClipboardData(uFormat: u32, hMem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
	}

	#[link(name = "kernel32")]
	unsafe extern "system" {
		pub fn GlobalAlloc(uFlags: u32, dwBytes: usize) -> *mut std::ffi::c_void;
		pub fn GlobalLock(hMem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
		pub fn GlobalUnlock(hMem: *mut std::ffi::c_void) -> i32;
	}
}

/// Disambiguates a generated clipboard filename against others already used
/// in the same copy batch (e.g. photos from one album share the same
/// second-precision timestamp, so `display_filename()` returns identical
/// names for all of them). Collisions get " (2)", " (3)", ... appended before
/// the extension, matching how Windows Explorer disambiguates duplicate
/// names, so every file lands at its own temp path instead of overwriting
/// the previous one.
pub(crate) fn unique_clipboard_filename(name: &str, used: &mut std::collections::HashMap<String, u32>) -> String {
	let count = used.entry(name.to_string()).or_insert(0);
	*count += 1;
	if *count == 1 {
		return name.to_string();
	}
	let path = std::path::Path::new(name);
	let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
	match path.extension().and_then(|e| e.to_str()) {
		Some(ext) => format!("{} ({}).{}", stem, count, ext),
		None => format!("{} ({})", stem, count),
	}
}

#[cfg(target_os = "windows")]
pub fn copy_files_to_clipboard(paths: &[String]) -> bool {
	let valid_paths: Vec<String> =
		paths.iter().filter(|p| !p.is_empty() && std::path::Path::new(p).exists()).cloned().collect();

	if valid_paths.is_empty() {
		return false;
	}

	let mut buffer: Vec<u16> = Vec::new();
	for p in &valid_paths {
		if let Ok(abs_path) = std::fs::canonicalize(p) {
			let clean = abs_path.to_string_lossy().trim_start_matches(r"\\?\").to_string();
			buffer.extend(clean.encode_utf16());
			buffer.push(0);
		} else {
			buffer.extend(p.encode_utf16());
			buffer.push(0);
		}
	}
	buffer.push(0);

	let dropfiles_size = 20;
	let total_bytes = dropfiles_size + buffer.len() * 2;

	unsafe {
		let h_global = win_cb::GlobalAlloc(0x0042, total_bytes);
		if h_global.is_null() {
			return false;
		}
		let ptr = win_cb::GlobalLock(h_global) as *mut u8;
		if ptr.is_null() {
			return false;
		}

		let p_files_ptr = ptr as *mut u32;
		*p_files_ptr = 20;
		let f_wide_ptr = ptr.add(16) as *mut u32;
		*f_wide_ptr = 1;

		let dst_str_ptr = ptr.add(dropfiles_size) as *mut u16;
		std::ptr::copy_nonoverlapping(buffer.as_ptr(), dst_str_ptr, buffer.len());

		let _ = win_cb::GlobalUnlock(h_global);

		if win_cb::OpenClipboard(std::ptr::null_mut()) != 0 {
			let _ = win_cb::EmptyClipboard();
			let res = win_cb::SetClipboardData(15, h_global);
			let _ = win_cb::CloseClipboard();
			return !res.is_null();
		}
	}
	false
}

#[cfg(not(target_os = "windows"))]
pub fn copy_files_to_clipboard(_paths: &[String]) -> bool {
	false
}
