//! Writes Ogg pages (RFC 3533) for the Opus voice notes whaNTV sends.
//! Reading them is done inline by `audio::decode_opus`.

use std::io::{self, Write};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PacketEnd {
	/// More packets may follow on the same page.
	Normal,
	/// The page is written out after this packet.
	EndPage,
	/// The page is written out after this packet, marked as the stream's last.
	EndStream,
}

pub(crate) struct OggWriter<W: Write> {
	out: W,
	serial: u32,
	sequence: u32,
	first_page: bool,
	/// The current page opens with the rest of a packet from the previous one.
	continued: bool,
	lacing: Vec<u8>,
	body: Vec<u8>,
	/// Granule position of the last packet that ended on the current page.
	granule: Option<u64>,
}

impl<W: Write> OggWriter<W> {
	pub(crate) fn new(out: W, serial: u32) -> Self {
		OggWriter {
			out,
			serial,
			sequence: 0,
			first_page: true,
			continued: false,
			lacing: Vec::new(),
			body: Vec::new(),
			granule: None,
		}
	}

	pub(crate) fn write_packet(&mut self, packet: &[u8], end: PacketEnd, granule: u64) -> io::Result<()> {
		let mut rest = packet;
		loop {
			// A page holds at most 255 lacing values; a packet that doesn't fit
			// carries on onto the next page.
			let room = 255 - self.lacing.len();
			let whole = rest.len() / 255;
			if whole < room {
				self.lacing.extend(std::iter::repeat_n(255u8, whole));
				self.lacing.push((rest.len() % 255) as u8);
				self.body.extend_from_slice(rest);
				self.granule = Some(granule);
				break;
			}
			self.lacing.extend(std::iter::repeat_n(255u8, room));
			self.body.extend_from_slice(&rest[..room * 255]);
			rest = &rest[room * 255..];
			self.flush_page(false)?;
			self.continued = true;
		}
		if end != PacketEnd::Normal {
			self.flush_page(end == PacketEnd::EndStream)?;
		}
		Ok(())
	}

	fn flush_page(&mut self, last: bool) -> io::Result<()> {
		let mut page = Vec::with_capacity(27 + self.lacing.len() + self.body.len());
		page.extend_from_slice(b"OggS\0");
		page.push(std::mem::take(&mut self.continued) as u8 | (self.first_page as u8) << 1 | (last as u8) << 2);
		// -1 marks a page on which no packet ends.
		page.extend_from_slice(&self.granule.take().map_or(-1i64, |g| g as i64).to_le_bytes());
		page.extend_from_slice(&self.serial.to_le_bytes());
		page.extend_from_slice(&self.sequence.to_le_bytes());
		page.extend_from_slice(&[0; 4]);
		page.push(self.lacing.len() as u8);
		page.extend_from_slice(&self.lacing);
		page.extend_from_slice(&self.body);
		let crc = crc32(&page);
		page[22..26].copy_from_slice(&crc.to_le_bytes());
		self.out.write_all(&page)?;
		self.lacing.clear();
		self.body.clear();
		self.sequence += 1;
		self.first_page = false;
		Ok(())
	}

	pub(crate) fn into_inner(self) -> W {
		self.out
	}
}

/// Ogg's CRC-32: polynomial 0x04c11db7, unreflected, zero initial value.
fn crc32(data: &[u8]) -> u32 {
	const TABLE: [u32; 256] = {
		let mut table = [0u32; 256];
		let mut i = 0;
		while i < 256 {
			let mut r = (i as u32) << 24;
			let mut bit = 0;
			while bit < 8 {
				r = if r & 0x8000_0000 != 0 { (r << 1) ^ 0x04c1_1db7 } else { r << 1 };
				bit += 1;
			}
			table[i] = r;
			i += 1;
		}
		table
	};
	data.iter().fold(0u32, |crc, &b| (crc << 8) ^ TABLE[((crc >> 24) as u8 ^ b) as usize])
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn crc_matches_a_known_page() {
		// The checksum of "123456789" under Ogg's CRC parameters (CRC-32/MPEG-2
		// without its 0xFFFFFFFF init and final xor).
		assert_eq!(crc32(b"123456789"), 0x89a1_897f);
	}

	#[test]
	fn long_packets_continue_onto_the_next_page() {
		let mut w = OggWriter::new(Vec::new(), 7);
		w.write_packet(&[1u8; 300 * 255], PacketEnd::EndStream, 42).unwrap();
		let data = w.into_inner();
		// The first page carries 255 full segments and no packet end.
		assert_eq!(data[5], 0x02);
		assert_eq!(i64::from_le_bytes(data[6..14].try_into().unwrap()), -1);
		assert_eq!(data[26], 255);
		let second = 27 + 255 + 255 * 255;
		assert_eq!(&data[second..second + 4], b"OggS");
		assert_eq!(data[second + 5], 0x01 | 0x04);
		assert_eq!(i64::from_le_bytes(data[second + 6..second + 14].try_into().unwrap()), 42);
		assert_eq!(data[second + 26], 46);
	}
}
