//! Password check for the app lock: PBKDF2-HMAC-SHA256 with a random salt,
//! stored as one text value in settings. `ring` is already part of the app
//! (the update check uses it through rustls), so this adds no new code to the
//! installer.

use std::num::NonZeroU32;

use ring::{
	pbkdf2,
	rand::{SecureRandom, SystemRandom},
};

const PREFIX: &str = "pbkdf2-sha256";
const ITERATIONS: u32 = 150_000;
const SALT_LEN: usize = 16;
const HASH_LEN: usize = 32;

fn to_hex(bytes: &[u8]) -> String {
	bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
	if !text.len().is_multiple_of(2) || !text.is_ascii() {
		return None;
	}
	(0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok()).collect()
}

/// Turns `password` into the text to save. `None` only if the system has no
/// random number source.
pub fn hash_password(password: &str) -> Option<String> {
	let mut salt = [0u8; SALT_LEN];
	SystemRandom::new().fill(&mut salt).ok()?;
	let mut hash = [0u8; HASH_LEN];
	pbkdf2::derive(pbkdf2::PBKDF2_HMAC_SHA256, NonZeroU32::new(ITERATIONS)?, &salt, password.as_bytes(), &mut hash);
	Some(format!("{PREFIX}${ITERATIONS}${}${}", to_hex(&salt), to_hex(&hash)))
}

/// Whether `password` is the one `stored` was made from. A missing or damaged
/// `stored` value never matches.
pub fn verify_password(stored: &str, password: &str) -> bool {
	let mut parts = stored.split('$');
	if parts.next() != Some(PREFIX) {
		return false;
	}
	let (Some(iterations), Some(salt), Some(hash), None) = (
		parts.next().and_then(|n| n.parse::<u32>().ok()).and_then(NonZeroU32::new),
		parts.next().and_then(from_hex),
		parts.next().and_then(from_hex),
		parts.next(),
	) else {
		return false;
	};
	pbkdf2::verify(pbkdf2::PBKDF2_HMAC_SHA256, iterations, &salt, password.as_bytes(), &hash).is_ok()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn right_password_passes_and_wrong_fails() {
		let stored = hash_password("correct horse").unwrap();
		assert!(verify_password(&stored, "correct horse"));
		assert!(!verify_password(&stored, "correct hors"));
		assert!(!verify_password(&stored, ""));
	}

	#[test]
	fn same_password_gets_different_salts() {
		assert_ne!(hash_password("x").unwrap(), hash_password("x").unwrap());
	}

	#[test]
	fn damaged_or_empty_values_never_match() {
		assert!(!verify_password("", ""));
		assert!(!verify_password("pbkdf2-sha256$0$00$00", "a"));
		assert!(!verify_password("pbkdf2-sha256$1000$zz$00", "a"));
		assert!(!verify_password("other$1000$00$00", "a"));
		let stored = hash_password("pw").unwrap();
		assert!(!verify_password(&format!("{stored}$extra"), "pw"));
	}
}
