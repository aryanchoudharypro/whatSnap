pub(crate) struct Frame {
	pub(crate) rgba: Vec<u8>,
	pub(crate) delay_ms: u32,
}

pub(crate) struct GifDecoder<'a> {
	data: &'a [u8],
	pos: usize,
	pub(crate) width: u32,
	pub(crate) height: u32,
	global_palette: Option<&'a [u8]>,
	canvas: Vec<u8>,
	pending_disposal: Option<Disposal>,
}

enum Disposal {
	Background { left: usize, top: usize, width: usize, height: usize },
	Previous(Vec<u8>),
}

impl<'a> GifDecoder<'a> {
	pub(crate) fn new(data: &'a [u8]) -> Option<Self> {
		if data.len() < 13 || !(data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a")) {
			return None;
		}
		let width = u16::from_le_bytes([data[6], data[7]]) as u32;
		let height = u16::from_le_bytes([data[8], data[9]]) as u32;
		if width == 0 || height == 0 {
			return None;
		}
		let flags = data[10];
		let mut pos = 13;
		let global_palette = if flags & 0x80 != 0 {
			let len = 3 << ((flags & 7) + 1);
			let palette = data.get(pos..pos + len)?;
			pos += len;
			Some(palette)
		} else {
			None
		};
		Some(GifDecoder {
			data,
			pos,
			width,
			height,
			global_palette,
			canvas: vec![0; width as usize * height as usize * 4],
			pending_disposal: None,
		})
	}

	fn byte(&mut self) -> Option<u8> {
		let b = *self.data.get(self.pos)?;
		self.pos += 1;
		Some(b)
	}

	fn take(&mut self, n: usize) -> Option<&'a [u8]> {
		let s = self.data.get(self.pos..self.pos + n)?;
		self.pos += n;
		Some(s)
	}

	fn sub_blocks(&mut self) -> Option<Vec<u8>> {
		let mut out = Vec::new();
		loop {
			let len = self.byte()? as usize;
			if len == 0 {
				return Some(out);
			}
			out.extend_from_slice(self.take(len)?);
		}
	}

	fn skip_sub_blocks(&mut self) -> Option<()> {
		loop {
			let len = self.byte()? as usize;
			if len == 0 {
				return Some(());
			}
			self.take(len)?;
		}
	}

	fn next_frame(&mut self) -> Option<Frame> {
		let (mut delay_ms, mut transparent, mut disposal) = (0u32, None, 0u8);
		loop {
			match self.byte()? {
				0x21 => {
					let label = self.byte()?;
					if label == 0xf9 {
						let block = self.sub_blocks()?;
						if block.len() >= 4 {
							disposal = (block[0] >> 2) & 7;
							delay_ms = u16::from_le_bytes([block[1], block[2]]) as u32 * 10;
							transparent = (block[0] & 1 != 0).then_some(block[3]);
						}
					} else {
						self.skip_sub_blocks()?;
					}
				}
				0x2c => {
					let d = self.take(9)?;
					let left = u16::from_le_bytes([d[0], d[1]]) as usize;
					let top = u16::from_le_bytes([d[2], d[3]]) as usize;
					let fw = u16::from_le_bytes([d[4], d[5]]) as usize;
					let fh = u16::from_le_bytes([d[6], d[7]]) as usize;
					let flags = d[8];
					let palette = if flags & 0x80 != 0 {
						let len = 3 << ((flags & 7) + 1);
						self.take(len)?
					} else {
						self.global_palette.unwrap_or(&[])
					};
					let interlaced = flags & 0x40 != 0;
					let min_code_size = self.byte()?;
					let compressed = self.sub_blocks()?;
					let indices = lzw_decode(&compressed, min_code_size, fw * fh);

					match self.pending_disposal.take() {
						Some(Disposal::Background { left, top, width, height }) => self.clear(left, top, width, height),
						Some(Disposal::Previous(saved)) => self.canvas = saved,
						None => {}
					}
					self.pending_disposal = match disposal {
						2 => Some(Disposal::Background { left, top, width: fw, height: fh }),
						3 => Some(Disposal::Previous(self.canvas.clone())),
						_ => None,
					};

					let (cw, ch) = (self.width as usize, self.height as usize);
					for (i, &index) in indices.iter().enumerate() {
						let row = if interlaced { interlaced_row(i / fw.max(1), fh) } else { i / fw.max(1) };
						let (x, y) = (left + i % fw.max(1), top + row);
						if x >= cw || y >= ch || Some(index) == transparent {
							continue;
						}
						let Some(rgb) = palette.get(index as usize * 3..index as usize * 3 + 3) else { continue };
						let o = (y * cw + x) * 4;
						self.canvas[o..o + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
					}
					return Some(Frame { rgba: self.canvas.clone(), delay_ms });
				}
				_ => return None,
			}
		}
	}

	fn clear(&mut self, left: usize, top: usize, width: usize, height: usize) {
		let cw = self.width as usize;
		for y in top..(top + height).min(self.height as usize) {
			for x in left..(left + width).min(cw) {
				self.canvas[(y * cw + x) * 4..][..4].fill(0);
			}
		}
	}
}

impl Iterator for GifDecoder<'_> {
	type Item = Frame;

	fn next(&mut self) -> Option<Frame> {
		self.next_frame()
	}
}

fn interlaced_row(n: usize, height: usize) -> usize {
	let passes = [(0, 8), (4, 8), (2, 4), (1, 2)];
	let mut n = n;
	for (start, step) in passes {
		let rows = height.saturating_sub(start).div_ceil(step);
		if n < rows {
			return start + n * step;
		}
		n -= rows;
	}
	height
}

fn lzw_decode(data: &[u8], min_code_size: u8, max: usize) -> Vec<u8> {
	let min_code_size = min_code_size.clamp(2, 11) as u32;
	let clear = 1u16 << min_code_size;
	let end = clear + 1;
	let mut prefix = [0u16; 4096];
	let mut suffix = [0u8; 4096];
	let mut first = [0u8; 4096];
	for c in 0..clear {
		suffix[c as usize] = c as u8;
		first[c as usize] = c as u8;
	}
	let mut out = Vec::with_capacity(max);
	let mut stack = Vec::new();
	let (mut size, mut next) = (min_code_size + 1, end + 1);
	let mut last: Option<u16> = None;
	let (mut acc, mut bits) = (0u32, 0u32);
	let mut bytes = data.iter();
	while out.len() < max {
		while bits < size {
			let Some(&b) = bytes.next() else { return out };
			acc |= (b as u32) << bits;
			bits += 8;
		}
		let code = (acc & ((1 << size) - 1)) as u16;
		acc >>= size;
		bits -= size;
		if code == clear {
			(size, next, last) = (min_code_size + 1, end + 1, None);
			continue;
		}
		if code == end {
			break;
		}
		let Some(prev) = last else {
			if code < clear {
				out.push(code as u8);
				last = Some(code);
			}
			continue;
		};
		let (start, known) = if code < next { (code, true) } else { (prev, false) };
		if !known && code != next {
			break;
		}
		let mut c = start;
		loop {
			stack.push(suffix[c as usize]);
			if c < clear {
				break;
			}
			c = prefix[c as usize];
		}
		let lead = first[start as usize];
		out.extend(stack.drain(..).rev());
		if !known {
			out.push(lead);
		}
		if next < 4096 {
			prefix[next as usize] = prev;
			suffix[next as usize] = lead;
			first[next as usize] = first[prev as usize];
			next += 1;
			if next == 1 << size && size < 12 {
				size += 1;
			}
		}
		last = Some(code);
	}
	out.truncate(max);
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn interlaced_rows_cover_every_row_once() {
		for height in 1..40 {
			let mut rows: Vec<usize> = (0..height).map(|n| interlaced_row(n, height)).collect();
			rows.sort();
			assert_eq!(rows, (0..height).collect::<Vec<_>>());
		}
	}
}
