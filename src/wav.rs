//! Reading and writing the 16-bit PCM WAV files voice messages are recorded
//! to. That's the only kind whatSnap makes, so it's the only kind handled.

use std::{
	fs::File,
	io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WavSpec {
	pub(crate) channels: u16,
	pub(crate) sample_rate: u32,
}

pub(crate) struct WavWriter {
	out: BufWriter<File>,
	data_bytes: u32,
	finalized: bool,
}

impl WavWriter {
	pub(crate) fn create(path: &str, spec: WavSpec) -> io::Result<Self> {
		let mut out = BufWriter::new(File::create(path)?);
		let block_align = spec.channels * 2;
		out.write_all(b"RIFF\0\0\0\0WAVEfmt ")?;
		out.write_all(&16u32.to_le_bytes())?;
		out.write_all(&1u16.to_le_bytes())?; // PCM
		out.write_all(&spec.channels.to_le_bytes())?;
		out.write_all(&spec.sample_rate.to_le_bytes())?;
		out.write_all(&(spec.sample_rate * block_align as u32).to_le_bytes())?;
		out.write_all(&block_align.to_le_bytes())?;
		out.write_all(&16u16.to_le_bytes())?;
		out.write_all(b"data\0\0\0\0")?;
		Ok(WavWriter { out, data_bytes: 0, finalized: false })
	}

	pub(crate) fn write_sample(&mut self, sample: i16) -> io::Result<()> {
		self.out.write_all(&sample.to_le_bytes())?;
		self.data_bytes += 2;
		Ok(())
	}

	/// Fills in the sizes the header was written without.
	pub(crate) fn finalize(mut self) -> io::Result<()> {
		self.write_sizes()
	}

	fn write_sizes(&mut self) -> io::Result<()> {
		self.finalized = true;
		self.out.flush()?;
		let file = self.out.get_mut();
		file.seek(SeekFrom::Start(4))?;
		file.write_all(&(36 + self.data_bytes).to_le_bytes())?;
		file.seek(SeekFrom::Start(40))?;
		file.write_all(&self.data_bytes.to_le_bytes())?;
		file.flush()
	}
}

impl Drop for WavWriter {
	fn drop(&mut self) {
		if !self.finalized {
			let _ = self.write_sizes();
		}
	}
}

/// Iterates over a WAV file's samples, interleaved.
pub(crate) struct WavReader {
	input: BufReader<File>,
	spec: WavSpec,
	remaining: u64,
}

impl WavReader {
	pub(crate) fn open(path: &str) -> io::Result<Self> {
		let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, format!("not a 16-bit PCM WAV: {what}"));
		let mut input = BufReader::new(File::open(path)?);
		let mut header = [0u8; 12];
		input.read_exact(&mut header)?;
		if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
			return Err(bad("no RIFF/WAVE header"));
		}
		let mut spec = None;
		loop {
			let mut chunk = [0u8; 8];
			input.read_exact(&mut chunk)?;
			let size = u32::from_le_bytes(chunk[4..8].try_into().unwrap());
			match &chunk[0..4] {
				b"fmt " => {
					let mut fmt = vec![0u8; size as usize];
					input.read_exact(&mut fmt)?;
					if fmt.len() < 16 {
						return Err(bad("short fmt chunk"));
					}
					let tag = u16::from_le_bytes([fmt[0], fmt[1]]);
					let bits = u16::from_le_bytes([fmt[14], fmt[15]]);
					// 0xFFFE is WAVE_FORMAT_EXTENSIBLE, which still holds plain PCM here.
					if !(tag == 1 || tag == 0xFFFE) || bits != 16 {
						return Err(bad("unsupported sample format"));
					}
					spec = Some(WavSpec {
						channels: u16::from_le_bytes([fmt[2], fmt[3]]).max(1),
						sample_rate: u32::from_le_bytes(fmt[4..8].try_into().unwrap()),
					});
					if size % 2 == 1 {
						input.seek_relative(1)?;
					}
				}
				b"data" => {
					let spec = spec.ok_or_else(|| bad("data before fmt"))?;
					// A recording that never got its sizes filled in says 0:
					// read it to the end of the file instead.
					let remaining = if size == 0 { u64::MAX } else { size as u64 };
					return Ok(WavReader { input, spec, remaining });
				}
				_ => input.seek_relative(size as i64 + (size % 2) as i64)?,
			}
		}
	}

	pub(crate) fn spec(&self) -> WavSpec {
		self.spec
	}
}

impl Iterator for WavReader {
	type Item = io::Result<i16>;

	fn next(&mut self) -> Option<io::Result<i16>> {
		if self.remaining < 2 {
			return None;
		}
		let mut sample = [0u8; 2];
		match self.input.read_exact(&mut sample) {
			Ok(()) => {
				self.remaining -= 2;
				Some(Ok(i16::from_le_bytes(sample)))
			}
			Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => None,
			Err(e) => Some(Err(e)),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn round_trips_samples_and_spec() {
		let path = std::env::temp_dir().join(format!("whantv_wav_test_{}.wav", std::process::id()));
		let path = path.to_str().unwrap();
		let spec = WavSpec { channels: 2, sample_rate: 48_000 };
		let mut w = WavWriter::create(path, spec).unwrap();
		for s in [0i16, 1, -1, i16::MAX, i16::MIN, 1234] {
			w.write_sample(s).unwrap();
		}
		w.finalize().unwrap();
		let data = std::fs::read(path).unwrap();
		assert_eq!(data.len(), 44 + 12);
		assert_eq!(u32::from_le_bytes(data[4..8].try_into().unwrap()), 36 + 12);
		let r = WavReader::open(path).unwrap();
		assert_eq!(r.spec(), spec);
		let samples: Vec<i16> = r.map(Result::unwrap).collect();
		let _ = std::fs::remove_file(path);
		assert_eq!(samples, [0, 1, -1, i16::MAX, i16::MIN, 1234]);
	}
}
