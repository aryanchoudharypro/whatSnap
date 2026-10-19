//! Small helpers written out here instead of pulling in a crate for each.

/// 32 hex digits no other process can predict, for pipe and temp-file names.
/// std seeds `RandomState` from the OS random source, and SipHash keyed with
/// those secret keys gives output as unguessable as the keys themselves.
pub(crate) fn random_hex() -> String {
	use std::hash::{BuildHasher, Hasher, RandomState};
	(0..2u64)
		.map(|i| {
			let mut h = RandomState::new().build_hasher();
			h.write_u64(i);
			format!("{:016x}", h.finish())
		})
		.collect()
}

/// Standard (padded) base64.
pub(crate) fn base64(data: &[u8]) -> String {
	const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
	let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
	for chunk in data.chunks(3) {
		let n = (chunk[0] as u32) << 16
			| (chunk.get(1).copied().unwrap_or(0) as u32) << 8
			| chunk.get(2).copied().unwrap_or(0) as u32;
		for i in 0..4 {
			out.push(if i <= chunk.len() { TABLE[(n >> (18 - 6 * i)) as usize & 63] as char } else { '=' });
		}
	}
	out
}

#[cfg(test)]
mod tests {
	#[test]
	fn base64_matches_rfc_4648_vectors() {
		for (input, expected) in [
			("", ""),
			("f", "Zg=="),
			("fo", "Zm8="),
			("foo", "Zm9v"),
			("foob", "Zm9vYg=="),
			("fooba", "Zm9vYmE="),
			("foobar", "Zm9vYmFy"),
		] {
			assert_eq!(super::base64(input.as_bytes()), expected);
		}
		assert_eq!(super::base64(&[0xfb, 0xff]), "+/8=");
	}

	#[test]
	fn random_hex_is_32_hex_digits_and_varies() {
		let (a, b) = (super::random_hex(), super::random_hex());
		assert_eq!(a.len(), 32);
		assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
		assert_ne!(a, b);
	}
}
