//! A small FFT, shared by the call echo canceller's filter and its suppressor.

use std::f32::consts::PI;

/// Iterative radix-2 complex FFT.
pub(crate) struct Fft {
	n: usize,
	rev: Vec<usize>,
	cos: Vec<f32>,
	sin: Vec<f32>,
}

impl Fft {
	pub(crate) fn new(n: usize) -> Self {
		assert!(n.is_power_of_two() && n >= 2);
		let bits = n.trailing_zeros();
		Self {
			n,
			rev: (0..n).map(|i| i.reverse_bits() >> (usize::BITS - bits)).collect(),
			cos: (0..n / 2).map(|k| (2.0 * PI * k as f32 / n as f32).cos()).collect(),
			sin: (0..n / 2).map(|k| (2.0 * PI * k as f32 / n as f32).sin()).collect(),
		}
	}

	pub(crate) fn transform(&self, re: &mut [f32], im: &mut [f32], inverse: bool) {
		let n = self.n;
		for i in 0..n {
			let j = self.rev[i];
			if j > i {
				re.swap(i, j);
				im.swap(i, j);
			}
		}
		let mut len = 2;
		while len <= n {
			let half = len / 2;
			let step = n / len;
			for start in (0..n).step_by(len) {
				for k in 0..half {
					let c = self.cos[k * step];
					let s = if inverse { self.sin[k * step] } else { -self.sin[k * step] };
					let (a, b) = (start + k, start + k + half);
					let tr = re[b] * c - im[b] * s;
					let ti = re[b] * s + im[b] * c;
					re[b] = re[a] - tr;
					im[b] = im[a] - ti;
					re[a] += tr;
					im[a] += ti;
				}
			}
			len <<= 1;
		}
		if inverse {
			let scale = 1.0 / n as f32;
			for (r, i) in re.iter_mut().zip(im.iter_mut()) {
				*r *= scale;
				*i *= scale;
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn fft_round_trips() {
		let fft = Fft::new(512);
		let mut seed = 3u32;
		let orig: Vec<f32> = (0..512)
			.map(|_| {
				seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
				seed as f32 / u32::MAX as f32 - 0.5
			})
			.collect();
		let (mut re, mut im) = (orig.clone(), vec![0.0; 512]);
		fft.transform(&mut re, &mut im, false);
		fft.transform(&mut re, &mut im, true);
		assert!(re.iter().zip(&orig).all(|(a, b)| (a - b).abs() < 1e-4));
	}
}
