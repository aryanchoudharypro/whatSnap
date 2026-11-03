//! Lookahead peak limiter, shared by call audio and voice messages. The gain
//! starts falling `lookahead` samples before a peak that would go over the
//! ceiling, so peaks are turned down smoothly instead of being clipped or
//! saturated, then recovers over the release time.

pub(crate) struct Limiter<const C: usize> {
	delay: Vec<[f32; C]>,
	need: Vec<f32>,
	held: Vec<f32>,
	pos: usize,
	gain: f32,
	release: f32,
	ceiling: f32,
}

impl<const C: usize> Limiter<C> {
	pub(crate) fn new(sample_rate: f32, lookahead: usize, ceiling: f32, release_secs: f32) -> Self {
		let lookahead = lookahead.max(1);
		Self {
			delay: vec![[0.0; C]; lookahead],
			need: vec![1.0; lookahead],
			held: vec![1.0; lookahead],
			pos: 0,
			gain: 1.0,
			release: 1.0 - (-1.0 / (release_secs * sample_rate)).exp(),
			ceiling,
		}
	}

	/// Delay in samples: what goes in comes out this many calls later.
	#[cfg(test)]
	pub(crate) fn delay(&self) -> usize {
		self.delay.len() - 1
	}

	pub(crate) fn process(&mut self, sample: [f32; C]) -> [f32; C] {
		let len = self.delay.len();
		let peak = sample.iter().fold(0.0f32, |m, s| m.max(s.abs()));
		self.need[self.pos] = if peak > self.ceiling { self.ceiling / peak } else { 1.0 };
		self.held[self.pos] = self.need.iter().copied().fold(1.0, f32::min);
		let smooth = self.held.iter().sum::<f32>() / len as f32;
		self.delay[self.pos] = sample;
		self.pos = (self.pos + 1) % len;
		self.gain = (self.gain + (1.0 - self.gain) * self.release).min(smooth);
		self.delay[self.pos].map(|s| s * self.gain)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn ramps_before_a_peak_and_never_goes_over() {
		const LOOKAHEAD: usize = 48;
		const CEILING: f32 = 0.9;
		let mut lim = Limiter::<2>::new(48_000.0, LOOKAHEAD, CEILING, 0.08);
		let mut input = vec![0.5f32; 20_000];
		input[200] = 1.8;
		let out: Vec<f32> = input
			.iter()
			.chain([0.0; LOOKAHEAD - 1].iter())
			.map(|&s| lim.process([s, s])[0])
			.skip(LOOKAHEAD - 1)
			.collect();
		assert!(out[..200 - LOOKAHEAD].iter().all(|&s| s == 0.5));
		assert!((out[200] - CEILING).abs() < 1e-4, "peak came out at {}", out[200]);
		assert!(out[200 - LOOKAHEAD / 2] < 0.5 && out[200 - LOOKAHEAD / 2] > 0.25);
		assert!(out.iter().all(|s| s.abs() <= CEILING + 1e-6));
		assert!(out[19_999] > 0.49);
	}

	#[test]
	fn leaves_a_quiet_signal_alone() {
		let mut lim = Limiter::<1>::new(16_000.0, 32, 0.9, 0.08);
		let input: Vec<f32> = (0..4000).map(|i| 0.6 * (i as f32 * 0.1).sin()).collect();
		let out: Vec<f32> = input.iter().map(|&s| lim.process([s])[0]).collect();
		assert_eq!(lim.delay(), 31);
		assert!(out[31..].iter().zip(&input).all(|(o, i)| o == i));
	}
}
