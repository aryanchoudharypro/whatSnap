//! Alert 1 to Alert 10, the notification tones WhatsApp for Windows offers
//! next to its default sound. Its own sound files live inside that app, so
//! these are small bell chimes made here instead: the same ten choices,
//! each easy to tell apart, and no sound files to ship.

use std::num::NonZero;

use rodio::buffer::SamplesBuffer;

const SAMPLE_RATE: u32 = 48_000;
const LEVEL: f32 = 0.3;

/// Each alert's notes as (frequency in Hz, start in ms, length in ms).
const ALERTS: [&[(f32, u32, u32)]; 10] = [
	&[(880.0, 0, 260), (1318.5, 130, 420)],
	&[(1318.5, 0, 260), (880.0, 130, 420)],
	&[(1046.5, 0, 180), (1046.5, 200, 360)],
	&[(784.0, 0, 160), (987.8, 110, 160), (1174.7, 220, 420)],
	&[(1568.0, 0, 480)],
	&[(659.3, 0, 600)],
	&[(1174.7, 0, 130), (1174.7, 150, 130), (1174.7, 300, 300)],
	&[(1046.5, 0, 200), (1568.0, 90, 200), (2093.0, 180, 450)],
	&[(587.3, 0, 220), (880.0, 160, 500)],
	&[(1396.9, 0, 150), (1046.5, 120, 150), (1396.9, 240, 400)],
];

/// The sound of a [`whatsnap::TONE_DEFAULT`] style tone number from Alert 1
/// (2) to Alert 10 (11), or `None` for any other number.
pub(crate) fn alert_tone(tone: u8) -> Option<SamplesBuffer> {
	let notes = ALERTS.get(usize::from(tone).checked_sub(2)?)?;
	let ms = |ms: u32| (ms * SAMPLE_RATE / 1000) as usize;
	let len = notes.iter().map(|&(_, start, length)| ms(start + length)).max().unwrap_or(0);
	let mut samples = vec![0.0f32; len];
	for &(freq, start, length) in *notes {
		let attack = ms(4) as f32;
		for i in 0..ms(length) {
			let t = i as f32 / SAMPLE_RATE as f32;
			let phase = std::f32::consts::TAU * freq * t;
			// A bell: the note with a quieter octave, struck and dying away.
			let tone = phase.sin() + 0.3 * (2.0 * phase).sin();
			let envelope = (i as f32 / attack).min(1.0) * (-t * 9.0).exp() * (1.0 - i as f32 / ms(length) as f32);
			samples[ms(start) + i] += tone * envelope;
		}
	}
	let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
	if peak > 0.0 {
		samples.iter_mut().for_each(|s| *s *= LEVEL / peak);
	}
	Some(SamplesBuffer::new(NonZero::new(1).unwrap(), NonZero::new(SAMPLE_RATE).unwrap(), samples))
}

#[cfg(test)]
mod tests {
	use super::alert_tone;

	#[test]
	fn only_alerts_one_to_ten_have_a_sound() {
		assert!(alert_tone(whatsnap::TONE_NONE).is_none());
		assert!(alert_tone(whatsnap::TONE_DEFAULT).is_none());
		assert!(alert_tone(whatsnap::TONE_LAST + 1).is_none());
		for tone in 2..=whatsnap::TONE_LAST {
			let samples: Vec<f32> = alert_tone(tone).unwrap().collect();
			assert!(samples.len() > 48_000 / 4 && samples.len() < 48_000, "Alert {}", tone - 1);
			assert!(samples.iter().all(|s| s.abs() <= 0.31));
		}
	}
}
