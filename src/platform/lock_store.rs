//! Where the app lock lives: the on/off flag, the unlock method and the
//! password hash are kept in `app_lock.dat` as a Windows DPAPI blob (current
//! user), not in `settings.json`. Hand-editing `settings.json` can't turn the
//! lock off, and the file is useless on another PC or under another account.
//!
//! This is still a convenience lock: code running as the user can read the
//! blob. The recovery path is deleting `app_lock.dat`, see [`reset`].

use std::path::PathBuf;

/// What the blob holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LockData {
	pub enabled: bool,
	pub hello: bool,
	pub hash: String,
}

/// DPAPI's extra secret, so a blob made for something else isn't accepted.
const ENTROPY: &[u8] = b"whatSnap app lock v1";

fn path() -> PathBuf {
	super::paths::app_data_dir().join("app_lock.dat")
}

/// The stored lock. No file means the lock is off. A file that can't be read
/// (damaged, or made on another PC or account) fails closed: the app stays
/// locked and only Exit gets out.
pub fn load() -> LockData {
	let Ok(bytes) = std::fs::read(path()) else { return LockData::default() };
	unprotect(&bytes).and_then(|plain| serde_json::from_slice(&plain).ok()).unwrap_or(LockData {
		enabled: true,
		hello: false,
		hash: String::new(),
	})
}

/// Whether the lock store file exists at all.
pub fn exists() -> bool {
	path().exists()
}

/// Saves the lock. A lock that is off and has no password removes the file.
pub fn save(data: &LockData) {
	if !data.enabled && data.hash.is_empty() {
		reset();
		return;
	}
	let Ok(json) = serde_json::to_vec(data) else { return };
	if let Some(blob) = protect(&json) {
		let _ = std::fs::write(path(), blob);
	}
}

/// Forgets the lock completely: the recovery path when Windows Hello breaks.
pub fn reset() {
	let _ = std::fs::remove_file(path());
}

#[cfg(windows)]
fn protect(plain: &[u8]) -> Option<Vec<u8>> {
	use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptProtectData};
	let input = CRYPT_INTEGER_BLOB { cbData: plain.len() as u32, pbData: plain.as_ptr() as *mut u8 };
	let entropy = CRYPT_INTEGER_BLOB { cbData: ENTROPY.len() as u32, pbData: ENTROPY.as_ptr() as *mut u8 };
	let mut out = CRYPT_INTEGER_BLOB::default();
	// SAFETY: the blobs point at live slices for the call; DPAPI allocates `out`.
	unsafe { CryptProtectData(&input, None, Some(&entropy), None, None, 0, &mut out).ok()? };
	Some(take_blob(out))
}

#[cfg(windows)]
fn unprotect(sealed: &[u8]) -> Option<Vec<u8>> {
	use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptUnprotectData};
	let input = CRYPT_INTEGER_BLOB { cbData: sealed.len() as u32, pbData: sealed.as_ptr() as *mut u8 };
	let entropy = CRYPT_INTEGER_BLOB { cbData: ENTROPY.len() as u32, pbData: ENTROPY.as_ptr() as *mut u8 };
	let mut out = CRYPT_INTEGER_BLOB::default();
	// SAFETY: as in `protect`.
	unsafe { CryptUnprotectData(&input, None, Some(&entropy), None, None, 0, &mut out).ok()? };
	Some(take_blob(out))
}

/// Copies a DPAPI-allocated blob out and frees it.
#[cfg(windows)]
fn take_blob(blob: windows::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB) -> Vec<u8> {
	use windows::Win32::Foundation::{HLOCAL, LocalFree};
	// SAFETY: DPAPI returned `cbData` bytes at `pbData`, allocated with LocalAlloc.
	unsafe {
		let bytes = std::slice::from_raw_parts(blob.pbData, blob.cbData as usize).to_vec();
		let _ = LocalFree(Some(HLOCAL(blob.pbData as *mut _)));
		bytes
	}
}

#[cfg(not(windows))]
fn protect(_plain: &[u8]) -> Option<Vec<u8>> {
	None
}

#[cfg(not(windows))]
fn unprotect(_sealed: &[u8]) -> Option<Vec<u8>> {
	None
}

#[cfg(all(test, windows))]
mod tests {
	use super::*;

	#[test]
	fn sealed_data_round_trips_and_rejects_tampering() {
		let sealed = protect(b"hello").unwrap();
		assert_ne!(sealed, b"hello");
		assert_eq!(unprotect(&sealed).unwrap(), b"hello");
		let mut bad = sealed.clone();
		let last = bad.len() - 1;
		bad[last] ^= 1;
		assert!(unprotect(&bad).is_none());
	}
}
