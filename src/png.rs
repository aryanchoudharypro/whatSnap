//! A small PNG encoder for the pictures whatSnap makes itself (the pairing QR
//! code and video previews): 8-bit RGBA, each row with whichever PNG filter
//! suits it best, compressed by `deflate` below.

/// Encodes `rgba` (`width * height` pixels, top row first) as a PNG.
pub(crate) fn encode_rgba(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
	let stride = width as usize * 4;
	assert_eq!(rgba.len(), stride * height as usize, "pixel data doesn't match the size");

	let mut filtered = Vec::with_capacity((stride + 1) * height as usize);
	let zero_row = vec![0u8; stride];
	let mut candidate = vec![0u8; stride];
	let mut best = vec![0u8; stride];
	for y in 0..height as usize {
		let row = &rgba[y * stride..][..stride];
		let above = if y == 0 { &zero_row[..] } else { &rgba[(y - 1) * stride..][..stride] };
		// libpng's heuristic: the filter whose output bytes, read as signed,
		// sum to the least.
		let mut best_filter = 0u8;
		let mut best_cost = u64::MAX;
		for filter in 0..5u8 {
			for i in 0..stride {
				let left = if i >= 4 { row[i - 4] } else { 0 };
				let up_left = if i >= 4 { above[i - 4] } else { 0 };
				let predicted = match filter {
					0 => 0,
					1 => left,
					2 => above[i],
					3 => ((left as u16 + above[i] as u16) / 2) as u8,
					_ => paeth(left, above[i], up_left),
				};
				candidate[i] = row[i].wrapping_sub(predicted);
			}
			let cost = candidate.iter().map(|&b| (b as i8).unsigned_abs() as u64).sum();
			if cost < best_cost {
				(best_cost, best_filter) = (cost, filter);
				std::mem::swap(&mut best, &mut candidate);
			}
		}
		filtered.push(best_filter);
		filtered.extend_from_slice(&best);
	}

	let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
	let mut ihdr = Vec::with_capacity(13);
	ihdr.extend_from_slice(&width.to_be_bytes());
	ihdr.extend_from_slice(&height.to_be_bytes());
	// 8 bits per channel, colour type 6 (RGBA), deflate, adaptive filtering,
	// no interlacing.
	ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
	chunk(&mut png, b"IHDR", &ihdr);
	chunk(&mut png, b"IDAT", &zlib(&filtered));
	chunk(&mut png, b"IEND", &[]);
	png
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
	let p = a as i16 + b as i16 - c as i16;
	let (pa, pb, pc) = ((p - a as i16).abs(), (p - b as i16).abs(), (p - c as i16).abs());
	if pa <= pb && pa <= pc {
		a
	} else if pb <= pc {
		b
	} else {
		c
	}
}

fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
	png.extend_from_slice(&(data.len() as u32).to_be_bytes());
	let start = png.len();
	png.extend_from_slice(kind);
	png.extend_from_slice(data);
	let crc = crc32(&png[start..]);
	png.extend_from_slice(&crc.to_be_bytes());
}

/// The CRC-32 PNG (and zip) use.
fn crc32(data: &[u8]) -> u32 {
	const TABLE: [u32; 256] = {
		let mut table = [0u32; 256];
		let mut i = 0;
		while i < 256 {
			let mut c = i as u32;
			let mut bit = 0;
			while bit < 8 {
				c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
				bit += 1;
			}
			table[i] = c;
			i += 1;
		}
		table
	};
	!data.iter().fold(!0u32, |c, &b| TABLE[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8))
}

/// `data` as a zlib stream.
fn zlib(data: &[u8]) -> Vec<u8> {
	let mut out = vec![0x78, 0x9c];
	deflate(data, &mut out);
	let (mut a, mut b) = (1u32, 0u32);
	for chunk in data.chunks(5552) {
		for &byte in chunk {
			a += byte as u32;
			b += a;
		}
		(a, b) = (a % 65_521, b % 65_521);
	}
	out.extend_from_slice(&(b << 16 | a).to_be_bytes());
	out
}

/// Writes bits least significant first, as deflate packs them.
struct BitWriter<'a> {
	out: &'a mut Vec<u8>,
	acc: u64,
	bits: u32,
}

impl BitWriter<'_> {
	fn bits(&mut self, value: u32, count: u32) {
		self.acc |= (value as u64) << self.bits;
		self.bits += count;
		while self.bits >= 8 {
			self.out.push(self.acc as u8);
			self.acc >>= 8;
			self.bits -= 8;
		}
	}

	/// A Huffman code, which deflate stores most significant bit first.
	fn code(&mut self, code: u32, len: u32) {
		self.bits(code.reverse_bits() >> (32 - len), len);
	}

	fn finish(mut self) {
		if self.bits > 0 {
			self.out.push(self.acc as u8);
		}
		self.acc = 0;
		self.bits = 0;
	}
}

const WINDOW: usize = 1 << 15;
const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 258;
/// How many earlier positions with the same three bytes to try.
const MAX_CHAIN: usize = 128;

/// Base length and extra bits of length codes 257 to 285.
const LENGTHS: [(u16, u8); 29] = [
	(3, 0),
	(4, 0),
	(5, 0),
	(6, 0),
	(7, 0),
	(8, 0),
	(9, 0),
	(10, 0),
	(11, 1),
	(13, 1),
	(15, 1),
	(17, 1),
	(19, 2),
	(23, 2),
	(27, 2),
	(31, 2),
	(35, 3),
	(43, 3),
	(51, 3),
	(59, 3),
	(67, 4),
	(83, 4),
	(99, 4),
	(115, 4),
	(131, 5),
	(163, 5),
	(195, 5),
	(227, 5),
	(258, 0),
];

/// Base distance and extra bits of distance codes 0 to 29.
const DISTANCES: [(u16, u8); 30] = [
	(1, 0),
	(2, 0),
	(3, 0),
	(4, 0),
	(5, 1),
	(7, 1),
	(9, 2),
	(13, 2),
	(17, 3),
	(25, 3),
	(33, 4),
	(49, 4),
	(65, 5),
	(97, 5),
	(129, 6),
	(193, 6),
	(257, 7),
	(385, 7),
	(513, 8),
	(769, 8),
	(1025, 9),
	(1537, 9),
	(2049, 10),
	(3073, 10),
	(4097, 11),
	(6145, 11),
	(8193, 12),
	(12289, 12),
	(16385, 13),
	(24577, 13),
];

/// Compresses `data` as one deflate block with the fixed Huffman codes:
/// LZ77 matches found through hash chains, taken greedily.
fn deflate(data: &[u8], out: &mut Vec<u8>) {
	let mut w = BitWriter { out, acc: 0, bits: 0 };
	// Final block, fixed codes.
	w.bits(0b011, 3);
	let literal = |w: &mut BitWriter, sym: u32| match sym {
		0..=143 => w.code(0x30 + sym, 8),
		144..=255 => w.code(0x190 + sym - 144, 9),
		256..=279 => w.code(sym - 256, 7),
		_ => w.code(0xc0 + sym - 280, 8),
	};

	let hash = |i: usize| ((data[i] as usize) << 10 ^ (data[i + 1] as usize) << 5 ^ data[i + 2] as usize) & 0x7fff;
	let mut head = vec![u32::MAX; 1 << 15];
	let mut prev = vec![u32::MAX; WINDOW];
	let insert = |i: usize, head: &mut [u32], prev: &mut [u32]| {
		if i + MIN_MATCH <= data.len() {
			let h = hash(i);
			prev[i % WINDOW] = head[h];
			head[h] = i as u32;
		}
	};

	let mut i = 0;
	while i < data.len() {
		let (mut best_len, mut best_dist) = (0, 0);
		if i + MIN_MATCH <= data.len() {
			let max = MAX_MATCH.min(data.len() - i);
			let mut candidate = head[hash(i)];
			let mut chain = 0;
			while candidate != u32::MAX && chain < MAX_CHAIN {
				let c = candidate as usize;
				if i - c > WINDOW - 1 {
					break;
				}
				let len = data[c..].iter().zip(&data[i..i + max]).take_while(|(a, b)| a == b).count();
				if len > best_len {
					(best_len, best_dist) = (len, i - c);
					if len == max {
						break;
					}
				}
				candidate = prev[c % WINDOW];
				chain += 1;
			}
		}
		if best_len >= MIN_MATCH {
			let code = LENGTHS.iter().rposition(|&(base, _)| base as usize <= best_len).unwrap();
			literal(&mut w, 257 + code as u32);
			let (base, extra) = LENGTHS[code];
			w.bits(best_len as u32 - base as u32, extra as u32);
			let code = DISTANCES.iter().rposition(|&(base, _)| base as usize <= best_dist).unwrap();
			w.code(code as u32, 5);
			let (base, extra) = DISTANCES[code];
			w.bits(best_dist as u32 - base as u32, extra as u32);
			for j in i..i + best_len {
				insert(j, &mut head, &mut prev);
			}
			i += best_len;
		} else {
			literal(&mut w, data[i] as u32);
			insert(i, &mut head, &mut prev);
			i += 1;
		}
	}
	literal(&mut w, 256);
	w.finish();
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn crc_and_adler_match_known_values() {
		assert_eq!(crc32(b"IEND"), 0xae42_6082);
		let z = zlib(b"Wikipedia");
		assert_eq!(&z[z.len() - 4..], &0x11e6_0398u32.to_be_bytes());
	}

	#[test]
	fn png_has_the_expected_header() {
		let png = encode_rgba(2, 1, &[255, 0, 0, 255, 0, 255, 0, 128]);
		assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
		assert_eq!(&png[12..16], b"IHDR");
		assert_eq!(&png[16..24], &[0, 0, 0, 2, 0, 0, 0, 1]);
		assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
	}
}
