//! The ringing heard while calling someone ("ringback").
//!
//! WhatsApp doesn't ship this as a sound file: its calling engine
//! (wa_call_ringback_tones.cc in WhatsApp Web's VoIP WebAssembly) synthesises
//! it from a per-country table of telephone ringback tones, picked by the
//! dialling code of your own number and falling back to the US tone. The table
//! below is that table, read out of WhatsApp Web 2.3000.1047483476's engine, so
//! calls ring the way they do in WhatsApp for your country.

use std::num::NonZero;

use rodio::buffer::SamplesBuffer;

const SAMPLE_RATE: u32 = 48_000;
/// Peak level of the tone, well under full scale like a phone's ringback.
const LEVEL: f32 = 0.25;
/// Short fade at each edge of a burst, so it doesn't click.
const FADE_SECS: f32 = 0.005;

/// (dialling code, first frequency Hz, second frequency Hz or 0, whether the
/// second frequency amplitude-modulates the first rather than being mixed in,
/// on/off cadence in ms).
type Ringback = (&'static str, u16, u16, bool, &'static [u16]);

const RINGBACKS: &[Ringback] = &[
	("1", 440, 480, false, &[2000, 4000]),
	("20", 425, 50, true, &[2000, 1000]),
	("213", 425, 0, false, &[1500, 3500]),
	("216", 425, 0, false, &[1200, 4600]),
	("220", 425, 0, false, &[1000, 4000]),
	("221", 400, 0, false, &[1700, 3300]),
	("222", 425, 0, false, &[1500, 3500]),
	("223", 425, 0, false, &[1700, 3300]),
	("224", 450, 0, false, &[400, 200]),
	("225", 450, 0, false, &[1000, 4000]),
	("226", 425, 0, false, &[1000, 4000]),
	("227", 440, 0, false, &[1700, 3300]),
	("229", 440, 0, false, &[1500, 3500]),
	("230", 425, 0, false, &[1000, 4000]),
	("231", 425, 0, false, &[1000, 4000]),
	("232", 425, 0, false, &[1000, 4000]),
	("233", 400, 0, false, &[1000, 4000]),
	("234", 400, 0, false, &[2000, 4000]),
	("236", 425, 0, false, &[1700, 3300]),
	("237", 440, 0, false, &[1700, 3300]),
	("239", 425, 0, false, &[1000, 5000]),
	("241", 440, 0, false, &[1500, 3000]),
	("244", 400, 0, false, &[1000, 5000]),
	("246", 440, 0, false, &[1000, 3000]),
	("247", 440, 0, false, &[1000, 3000]),
	("248", 425, 0, false, &[400, 200, 400, 2000]),
	("250", 440, 0, false, &[1700, 3300]),
	("251", 425, 0, false, &[1000, 4000]),
	("253", 425, 0, false, &[1500, 3500]),
	("254", 425, 0, false, &[1000, 4000, 1000, 2000, 1000, 5000, 400, 200]),
	("255", 400, 450, false, &[400, 200, 400, 2000]),
	("256", 425, 0, false, &[1000, 4000, 0, 0]),
	("257", 425, 0, false, &[1700, 3300]),
	("258", 400, 0, false, &[1000, 5000]),
	("260", 425, 0, false, &[1000, 4000]),
	("261", 425, 0, false, &[1660, 3330]),
	("262", 440, 0, false, &[1500, 3500]),
	("263", 400, 450, true, &[400, 200, 400, 2000]),
	("264", 400, 33, true, &[400, 200, 400, 2000]),
	("265", 425, 0, false, &[1000, 4000]),
	("267", 400, 0, false, &[400, 200, 400, 2000]),
	("268", 400, 0, false, &[400, 200, 400, 4000]),
	("269", 425, 0, false, &[1710, 3310]),
	("27", 400, 33, true, &[400, 200, 400, 2000]),
	("290", 400, 450, false, &[400, 200]),
	("297", 425, 0, false, &[1000, 4000]),
	("298", 425, 0, false, &[1000, 4000]),
	("299", 445, 0, false, &[1000, 4000]),
	("30", 425, 0, false, &[1000, 4000]),
	("31", 425, 0, false, &[1000, 4000]),
	("32", 425, 0, false, &[1000, 3000]),
	("33", 440, 0, false, &[1500, 3500]),
	("34", 425, 0, false, &[1500, 3000]),
	("350", 400, 450, false, &[400, 200, 400, 2000]),
	("351", 400, 0, false, &[1000, 5000]),
	("352", 425, 0, false, &[1000, 4000]),
	("353", 400, 0, false, &[400, 200, 400, 2000]),
	("354", 425, 0, false, &[1200, 4700]),
	("355", 425, 0, false, &[1000, 4000]),
	("356", 400, 450, false, &[400, 200, 400, 2000]),
	("357", 425, 0, false, &[1500, 3000]),
	("358", 425, 0, false, &[1000, 4000]),
	("359", 425, 0, false, &[1000, 9000]),
	("36", 425, 0, false, &[1200, 3700]),
	("370", 425, 0, false, &[800, 3200]),
	("372", 425, 0, false, &[1000, 4000]),
	("373", 425, 0, false, &[800, 3200]),
	("375", 425, 0, false, &[800, 3200]),
	("377", 425, 0, false, &[1660, 3330]),
	("378", 425, 0, false, &[1000, 4000]),
	("385", 425, 0, false, &[1000, 4000]),
	("386", 425, 0, false, &[1000, 4000]),
	("39", 425, 0, false, &[1000, 4000]),
	("40", 400, 16, true, &[2000, 4000]),
	("41", 425, 0, false, &[1000, 4000]),
	("420", 425, 0, false, &[1000, 4000]),
	("421", 425, 0, false, &[1000, 4000]),
	("43", 420, 0, false, &[1000, 5000]),
	("44", 400, 450, false, &[400, 200, 400, 2000]),
	("45", 425, 0, false, &[1000, 4000]),
	("46", 425, 0, false, &[1000, 5000]),
	("47", 425, 0, false, &[1000, 4000]),
	("48", 425, 0, false, &[1000, 4000]),
	("49", 425, 0, false, &[250, 4000, 1000, 4000, 1000, 4000]),
	("500", 400, 450, false, &[400, 200]),
	("503", 425, 0, false, &[1200, 5000]),
	("504", 440, 0, false, &[500, 4000, 2000, 4000]),
	("506", 425, 0, false, &[1200, 4600]),
	("507", 425, 0, false, &[1200, 4650]),
	("508", 440, 0, false, &[1500, 3500]),
	("52", 425, 0, false, &[1000, 4000]),
	("53", 440, 480, false, &[2000, 4000]),
	("54", 425, 0, false, &[1000, 4000]),
	("55", 425, 0, false, &[1000, 4000]),
	("56", 400, 0, false, &[1000, 3000]),
	("590", 440, 0, false, &[1500, 3500]),
	("592", 425, 0, false, &[1750, 3250]),
	("593", 425, 0, false, &[1200, 4650]),
	("594", 440, 0, false, &[1500, 3500]),
	("595", 425, 0, false, &[1000, 4000]),
	("596", 440, 0, false, &[1500, 3500]),
	("597", 425, 0, false, &[15000, 500]),
	("598", 425, 0, false, &[1000, 4000]),
	("60", 425, 0, false, &[400, 200, 400, 2000]),
	("61", 400, 17, true, &[400, 200, 400, 2000]),
	("62", 425, 0, false, &[1000, 4000]),
	("63", 450, 0, false, &[1000, 10000]),
	("64", 400, 450, false, &[400, 200, 400, 2000]),
	("65", 425, 24, true, &[400, 200, 400, 2000]),
	("66", 400, 0, false, &[1000, 4000]),
	("673", 400, 450, false, &[400, 300, 400, 2000]),
	("674", 400, 0, false, &[1000, 4000]),
	("675", 425, 0, false, &[400, 200, 400, 2000]),
	("677", 400, 450, false, &[400, 200]),
	("678", 425, 0, false, &[1000, 4000]),
	("679", 425, 25, true, &[400, 200, 400, 2000]),
	("681", 440, 0, false, &[1500, 3500]),
	("685", 425, 25, true, &[375, 250, 375, 2000]),
	("686", 425, 0, false, &[400, 200, 400, 2000]),
	("687", 440, 0, false, &[1500, 3500]),
	("689", 440, 0, false, &[1500, 3500]),
	("7", 425, 0, false, &[800, 3200]),
	("81", 400, 16, true, &[1000, 2000]),
	("82", 440, 480, false, &[1000, 2000]),
	("850", 440, 480, false, &[1000, 2000]),
	("852", 440, 480, false, &[400, 200, 400, 3000]),
	("853", 425, 0, false, &[1000, 4000]),
	("856", 425, 0, false, &[1000, 4000]),
	("86", 450, 0, false, &[1000, 4000]),
	("90", 450, 0, false, &[2000, 4000]),
	("91", 400, 25, true, &[400, 200, 400, 2600]),
	("92", 450, 0, false, &[1000, 4000]),
	("94", 450, 0, false, &[1000, 4000]),
	("960", 400, 0, false, &[400, 200, 400, 2000]),
	("961", 435, 0, false, &[1200, 4400]),
	("962", 420, 40, true, &[1200, 4800]),
	("963", 425, 0, false, &[1000, 4000]),
	("964", 425, 0, false, &[1200, 4650]),
	("965", 425, 0, false, &[1000, 4000]),
	("966", 425, 0, false, &[1200, 4600]),
	("968", 425, 0, false, &[400, 200, 400, 2000]),
	("971", 400, 450, false, &[400, 200, 400, 2000]),
	("972", 400, 0, false, &[1000, 3000]),
	("973", 440, 0, false, &[400, 200, 400, 2000]),
	("974", 400, 450, false, &[400, 200, 400, 2200]),
	("975", 400, 25, false, &[400, 200, 400, 2600]),
	("977", 425, 0, false, &[1100, 3100]),
	("98", 425, 0, false, &[1000, 4000]),
	("992", 425, 0, false, &[800, 3200]),
	("993", 425, 0, false, &[400, 4000]),
	("996", 425, 0, false, &[800, 3200]),
];

/// The ringback for someone whose own number is `self_phone` (digits,
/// international format without the +): the longest dialling code it starts
/// with, else the US tone WhatsApp defaults to.
fn ringback_for(self_phone: &str) -> Ringback {
	RINGBACKS
		.iter()
		.filter(|r| self_phone.starts_with(r.0))
		.max_by_key(|r| r.0.len())
		.or_else(|| RINGBACKS.iter().find(|r| r.0 == "1"))
		.copied()
		.expect("the US ringback is in the table")
}

/// One full on/off cycle of the ringback for `self_phone`, mono, ready to be
/// looped.
pub(crate) fn ringback_cycle(self_phone: &str) -> SamplesBuffer {
	let (_, f1, f2, modulated, cadence) = ringback_for(self_phone);
	let rate = SAMPLE_RATE as f32;
	let fade = (FADE_SECS * rate) as usize;
	let mut samples = Vec::new();
	for (i, &ms) in cadence.iter().enumerate() {
		let n = ms as usize * SAMPLE_RATE as usize / 1000;
		if i % 2 == 1 {
			samples.extend(std::iter::repeat_n(0.0, n));
			continue;
		}
		for k in 0..n {
			let t = k as f32 / rate;
			let carrier = (std::f32::consts::TAU * f1 as f32 * t).sin();
			let tone = match (f2, modulated) {
				(0, _) => carrier,
				(f2, true) => carrier * (0.5 + 0.5 * (std::f32::consts::TAU * f2 as f32 * t).sin()),
				(f2, false) => 0.5 * (carrier + (std::f32::consts::TAU * f2 as f32 * t).sin()),
			};
			let edge = k.min(n - 1 - k);
			let gain = if edge < fade { edge as f32 / fade as f32 } else { 1.0 };
			samples.push(tone * gain * LEVEL);
		}
	}
	SamplesBuffer::new(NonZero::new(1).unwrap(), NonZero::new(SAMPLE_RATE).unwrap(), samples)
}

/// The busy tone heard when the person you're calling is on another call.
/// WhatsApp Web (WAWebCallBusyTone) plays one tone for every country, not one
/// per country like the ringback: 480 Hz and 620 Hz together, each at gain
/// 0.15, half a second on and half off, four times.
pub(crate) fn busy_tone() -> SamplesBuffer {
	const FREQS: [f32; 2] = [480.0, 620.0];
	const GAIN: f32 = 0.15;
	const HALF_MS: usize = 500;
	const CYCLES: usize = 4;
	let rate = SAMPLE_RATE as f32;
	let fade = (FADE_SECS * rate) as usize;
	let n = HALF_MS * SAMPLE_RATE as usize / 1000;
	let mut samples = Vec::with_capacity(2 * n * CYCLES);
	for cycle in 0..CYCLES {
		let start = cycle * 2 * n;
		for k in 0..n {
			// Oscillators run from the start, as WhatsApp's do, gated on and off.
			let t = (start + k) as f32 / rate;
			let tone: f32 = FREQS.iter().map(|f| (std::f32::consts::TAU * f * t).sin()).sum();
			let edge = k.min(n - 1 - k);
			let gain = if edge < fade { edge as f32 / fade as f32 } else { 1.0 };
			samples.push(tone * GAIN * gain);
		}
		samples.extend(std::iter::repeat_n(0.0, n));
	}
	SamplesBuffer::new(NonZero::new(1).unwrap(), NonZero::new(SAMPLE_RATE).unwrap(), samples)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn picks_the_longest_matching_dialling_code() {
		assert_eq!(ringback_for("919876543210").0, "91");
		assert_eq!(ringback_for("971501234567").0, "971");
		assert_eq!(ringback_for("447700900123").0, "44");
		assert_eq!(ringback_for("12025550123").0, "1");
	}

	#[test]
	fn unknown_or_missing_number_rings_like_the_us() {
		assert_eq!(ringback_for("").0, "1");
		assert_eq!(ringback_for("0000").0, "1");
	}

	#[test]
	fn cycle_matches_the_cadence() {
		// India: 400 Hz modulated at 25 Hz, 0.4 on, 0.2 off, 0.4 on, 2.6 off.
		let samples: Vec<f32> = ringback_cycle("919876543210").collect();
		assert_eq!(samples.len(), 48_000 * 36 / 10);
		assert!(samples.iter().all(|s| s.abs() <= LEVEL));
		let off = &samples[(0.45 * 48_000.0) as usize..(0.55 * 48_000.0) as usize];
		assert!(off.iter().all(|&s| s == 0.0));
		let on = &samples[(0.1 * 48_000.0) as usize..(0.3 * 48_000.0) as usize];
		assert!(on.iter().any(|s| s.abs() > LEVEL * 0.5));
	}

	#[test]
	fn busy_tone_is_four_half_second_beeps() {
		let samples: Vec<f32> = busy_tone().collect();
		assert_eq!(samples.len(), 48_000 * 4);
		assert!(samples.iter().all(|s| s.abs() <= 0.3));
		for beep in 0..4 {
			let on = &samples[(beep * 48_000 + 12_000)..(beep * 48_000 + 13_000)];
			assert!(on.iter().any(|s| s.abs() > 0.1), "beep {beep} is silent");
			let off = &samples[(beep * 48_000 + 25_000)..(beep * 48_000 + 47_000)];
			assert!(off.iter().all(|&s| s == 0.0), "gap after beep {beep} isn't silent");
		}
	}
}
