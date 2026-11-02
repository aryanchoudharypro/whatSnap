//! Sound files picked for tones and ringtones: one the user browsed to, or
//! one of the ringtones Windows keeps in its Media folder.

use std::{sync::OnceLock, time::Duration};

use rodio::Source;

/// A custom message tone is cut off here, so a song picked by mistake
/// doesn't play to the end on every message.
pub(crate) const LONGEST_TONE: Duration = Duration::from_secs(10);
/// How much of a ringtone a ringtone list plays when one is picked.
pub(crate) const LONGEST_PREVIEW: Duration = Duration::from_secs(6);

/// File types the Browse button offers; Media Foundation plays them all.
pub(crate) const BROWSE_WILDCARD: &str = "Sound files (*.wav;*.mp3;*.ogg;*.opus;*.m4a;*.aac;*.wma;*.flac)|*.wav;*.mp3;*.ogg;*.opus;*.m4a;*.aac;*.wma;*.flac|All files (*.*)|*.*";

pub(crate) fn open(path: &str) -> Option<Box<dyn Source + Send>> {
	super::file::open(path, Duration::ZERO)
}

/// The ringtones Windows ships (Media\Ring01.wav to Ring10.wav and
/// Alarm01.wav to Alarm10.wav, the sounds behind its Call and Alarm toast
/// sounds), as (name, path), for the ones this computer has.
pub(crate) fn windows_ringtones() -> &'static [(String, String)] {
	static RINGTONES: OnceLock<Vec<(String, String)>> = OnceLock::new();
	RINGTONES.get_or_init(|| {
		let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".to_string());
		let media = std::path::Path::new(&windir).join("Media");
		let mut found = Vec::new();
		for (file, name) in [("Ring", "Windows Ring"), ("Alarm", "Windows Alarm")] {
			for n in 1..=10 {
				let path = media.join(format!("{file}{n:02}.wav"));
				if path.is_file() {
					found.push((format!("{name} {n}"), path.to_string_lossy().into_owned()));
				}
			}
		}
		found
	})
}

/// A sound file played over and over, opened afresh each time round so a
/// long song isn't held in memory.
pub(crate) struct Looped {
	path: String,
	inner: Box<dyn Source + Send>,
	/// Whether this time round has given a sample yet. A file that ends
	/// before giving one would otherwise be reopened forever.
	started: bool,
}

impl Looped {
	pub(crate) fn open(path: &str) -> Option<Self> {
		Some(Self { path: path.to_string(), inner: open(path)?, started: false })
	}
}

impl Iterator for Looped {
	type Item = f32;

	fn next(&mut self) -> Option<f32> {
		if let Some(sample) = self.inner.next() {
			self.started = true;
			return Some(sample);
		}
		if !self.started {
			return None;
		}
		let next = open(&self.path)?;
		// The mixer was set up for the first round's format.
		if next.channels() != self.inner.channels() || next.sample_rate() != self.inner.sample_rate() {
			return None;
		}
		self.inner = next;
		self.started = false;
		self.next()
	}
}

impl Source for Looped {
	fn current_span_len(&self) -> Option<usize> {
		None
	}

	fn channels(&self) -> rodio::ChannelCount {
		self.inner.channels()
	}

	fn sample_rate(&self) -> rodio::SampleRate {
		self.inner.sample_rate()
	}

	fn total_duration(&self) -> Option<Duration> {
		None
	}
}
