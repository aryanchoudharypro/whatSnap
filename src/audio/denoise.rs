//! Handwritten spectral noise suppressor: short-time FFT, a per-bin noise
//! estimate that follows the quietest moments, a decision-directed Wiener
//! gain with a floor set by the intensity, then overlap-add back to audio.
//! Voice messages run it lightly after the RNNoise network
//! (audio/voice_dsp.rs) to take out the steady hiss the network leaves,
//! mostly above 8 kHz.
//!
//! Works on 10 ms hops with 50% overlapped sqrt-Hann windows, so it adds one
//! hop of delay. Mono; run one per channel.

use std::f32::consts::PI;

use super::fft::Fft;

/// The first hops are taken as background to seed the noise estimate.
const INIT_HOPS: u32 = 20;
/// How fast the noise estimate may creep up when the input is louder than it
/// (about +4 dB per second), so a new background (a fan, a video playing) is
/// learned in a few seconds, while speech, which keeps pausing, isn't.
const NOISE_RISE: f32 = 1.01;
/// The tracked minimum sits a little under the real noise mean.
const NOISE_BIAS: f32 = 1.6;
/// Decision-directed smoothing of the a-priori SNR (Ephraim-Malah).
const DD_ALPHA: f32 = 0.98;
/// Attenuation at full intensity, in dB.
const MAX_SUPPRESSION_DB: f32 = 25.0;
/// Speech-likeness is judged on this band.
const VOICE_LO_HZ: f32 = 300.0;
const VOICE_HI_HZ: f32 = 3500.0;

pub(crate) struct Denoiser {
	hop: usize,
	win_len: usize,
	window: Vec<f32>,
	input: Vec<f32>,
	overlap: Vec<f32>,
	fft: Fft,
	re: Vec<f32>,
	im: Vec<f32>,
	smoothed: Vec<f32>,
	noise: Vec<f32>,
	prev_clean: Vec<f32>,
	gains: Vec<f32>,
	hops: u32,
	floor: f32,
	voice_lo: usize,
	voice_hi: usize,
}

impl Denoiser {
	/// `intensity` 0..1: how far noise is pushed down (1 = 25 dB).
	pub(crate) fn new(sample_rate: u32, intensity: f32) -> Self {
		let hop = (sample_rate / 100) as usize;
		let win_len = hop * 2;
		let n = win_len.next_power_of_two();
		let bins = n / 2 + 1;
		let bin_hz = sample_rate as f32 / n as f32;
		Self {
			hop,
			win_len,
			window: (0..win_len).map(|i| (PI * i as f32 / win_len as f32).sin()).collect(),
			input: vec![0.0; win_len],
			overlap: vec![0.0; hop],
			fft: Fft::new(n),
			re: vec![0.0; n],
			im: vec![0.0; n],
			smoothed: vec![0.0; bins],
			noise: vec![0.0; bins],
			prev_clean: vec![0.0; bins],
			gains: vec![1.0; bins],
			hops: 0,
			floor: 10f32.powf(-intensity.clamp(0.0, 1.0) * MAX_SUPPRESSION_DB / 20.0),
			voice_lo: (VOICE_LO_HZ / bin_hz).round() as usize,
			voice_hi: ((VOICE_HI_HZ / bin_hz).round() as usize).min(bins - 1),
		}
	}

	#[cfg(test)]
	pub(crate) fn hop(&self) -> usize {
		self.hop
	}

	/// Takes one hop of input, writes one hop of output (from one hop ago) and
	/// returns how speech-like the input was, 0..1.
	pub(crate) fn process_hop(&mut self, input: &[f32], out: &mut [f32]) -> f32 {
		let hop = self.hop;
		debug_assert_eq!(input.len(), hop);
		debug_assert_eq!(out.len(), hop);
		self.input.copy_within(hop.., 0);
		self.input[self.win_len - hop..].copy_from_slice(input);

		let n = self.re.len();
		for i in 0..n {
			self.re[i] = if i < self.win_len { self.input[i] * self.window[i] } else { 0.0 };
			self.im[i] = 0.0;
		}
		self.fft.transform(&mut self.re, &mut self.im, false);

		let bins = n / 2 + 1;
		let first = self.hops == 0;
		let init = self.hops < INIT_HOPS;
		let (mut voice_gamma, mut voice_bins) = (0.0f32, 0usize);
		for k in 0..bins {
			let p = self.re[k] * self.re[k] + self.im[k] * self.im[k];
			let s = if first { p } else { 0.7 * self.smoothed[k] + 0.3 * p };
			self.smoothed[k] = s;
			self.noise[k] = if init {
				(self.noise[k] * self.hops as f32 + s) / (self.hops + 1) as f32
			} else if s < self.noise[k] {
				0.85 * self.noise[k] + 0.15 * s
			} else {
				(self.noise[k] * NOISE_RISE).min(s)
			};
			let noise = (self.noise[k] * NOISE_BIAS).max(1e-12);
			let gamma = p / noise;
			let xi = (DD_ALPHA * self.prev_clean[k] / noise + (1.0 - DD_ALPHA) * (gamma - 1.0).max(0.0)).max(1e-3);
			let g = (xi / (1.0 + xi)).max(self.floor);
			self.gains[k] = g;
			self.prev_clean[k] = g * g * p;
			if (self.voice_lo..=self.voice_hi).contains(&k) {
				voice_gamma += gamma;
				voice_bins += 1;
			}
		}
		// A little smoothing across neighbouring bins keeps lone bins from
		// flickering ("musical noise").
		let mut prev = self.gains[0];
		for k in 1..bins - 1 {
			let cur = self.gains[k];
			self.gains[k] = (0.25 * prev + 0.5 * cur + 0.25 * self.gains[k + 1]).max(self.floor);
			prev = cur;
		}
		for k in 0..bins {
			let g = self.gains[k];
			self.re[k] *= g;
			self.im[k] *= g;
			if k != 0 && k != n / 2 {
				self.re[n - k] *= g;
				self.im[n - k] *= g;
			}
		}
		self.fft.transform(&mut self.re, &mut self.im, true);

		for i in 0..hop {
			out[i] = self.overlap[i] + self.re[i] * self.window[i];
			self.overlap[i] = self.re[hop + i] * self.window[hop + i];
		}
		self.hops = self.hops.saturating_add(1);

		if init || voice_bins == 0 {
			return 0.0;
		}
		let snr_db = 10.0 * (voice_gamma / voice_bins as f32).max(1e-6).log10();
		((snr_db - 4.0) / 8.0).clamp(0.0, 1.0)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn lcg(seed: &mut u32) -> f32 {
		*seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
		*seed as f32 / u32::MAX as f32 - 0.5
	}

	fn rms_db(x: &[f32]) -> f32 {
		10.0 * (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32 + 1e-20).log10()
	}

	fn run(d: &mut Denoiser, input: &[f32]) -> (Vec<f32>, Vec<f32>) {
		let hop = d.hop();
		let mut out = vec![0.0; input.len()];
		let mut probs = Vec::new();
		for (i, o) in input.chunks(hop).zip(out.chunks_mut(hop)) {
			if i.len() == hop {
				probs.push(d.process_hop(i, o));
			}
		}
		(out, probs)
	}

	#[test]
	fn passes_clean_signal_through_with_one_hop_delay() {
		// With a floor of 1 (intensity 0) it must be transparent.
		let mut d = Denoiser::new(16_000, 0.0);
		let input: Vec<f32> = (0..16_000).map(|i| 0.3 * (i as f32 * 0.2).sin()).collect();
		let (out, _) = run(&mut d, &input);
		let hop = d.hop();
		for i in 2 * hop..input.len() - hop {
			assert!((out[i] - input[i - hop]).abs() < 1e-3, "sample {i}");
		}
	}

	#[test]
	fn pushes_steady_noise_down_and_keeps_speech() {
		for rate in [16_000u32, 48_000] {
			let mut d = Denoiser::new(rate, 0.55);
			let mut seed = 11;
			let secs = 4;
			let input: Vec<f32> = (0..rate as usize * secs)
				.map(|i| {
					let t = i as f32 / rate as f32;
					let talking = t > 2.0 && (t * 2.0) as u32 % 2 == 0;
					let noise = 0.01 * lcg(&mut seed);
					noise + if talking { 0.2 * (2.0 * PI * 400.0 * t).sin() } else { 0.0 }
				})
				.collect();
			let (out, probs) = run(&mut d, &input);
			let r = rate as usize;
			// 1.0-2.0 s: noise only.
			let noise_drop = rms_db(&input[r..2 * r]) - rms_db(&out[r..2 * r]);
			assert!(noise_drop > 9.0, "{rate}: noise only fell {noise_drop} dB");
			// 3.0-3.5 s: talking.
			let speech_change =
				rms_db(&out[3 * r + r / 20..3 * r + r / 2]) - rms_db(&input[3 * r..3 * r + r / 2 - r / 20]);
			assert!(speech_change.abs() < 1.5, "{rate}: speech changed {speech_change} dB");
			let hops_per_s = 100;
			assert!(probs[hops_per_s + 50] < 0.2, "{rate}: noise judged speech-like");
			assert!(probs[3 * hops_per_s + 20] > 0.8, "{rate}: speech not judged speech-like");
		}
	}
}
