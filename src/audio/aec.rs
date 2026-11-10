//! Handwritten acoustic echo canceller for calls. It removes from the mic
//! whatever the computer itself is playing (the other person's voice, a
//! screen reader, a video) when that leaks back in, from speakers or from
//! headphones sitting next to the mic.
//!
//! Three parts, the same split as WebRTC's and Speex's cancellers:
//!
//! - `DelayFinder` works out how long played sound takes to come back, by
//!   matching coarse one-bit spectra of the played and recorded sound (as
//!   WebRTC's delay estimator does), so the filter below only has to cover
//!   the room, not the audio device's latency (Bluetooth can add 200 ms).
//! - A partitioned-block frequency-domain adaptive filter (the "MDF" filter)
//!   learns the path from what is played to the mic and subtracts its
//!   prediction. Two copies run, as in Speex: a background one that always
//!   learns and a foreground one that makes the output and only takes over
//!   the background's weights once those have proved better, so talking over
//!   the far end cannot wreck the filter. The learning rate follows Valin's
//!   "On adjusting the learning rate in frequency domain echo cancellation
//!   with double-talk": it is scaled by how much of the remaining signal is
//!   still echo.
//! - What the filter cannot remove is reported as an echo estimate, and the
//!   noise suppressor (audio/denoise.rs) turns those frequencies down.
//!
//! 16 kHz mono, 8 ms blocks.

use super::fft::Fft;

const BLOCK: usize = 128;
const FRAME: usize = BLOCK * 2;
const BINS: usize = FRAME / 2 + 1;
/// Filter length in blocks: 160 ms.
const PARTS: usize = 20;
/// The maths below was tuned on 16-bit sample values.
const SCALE: f32 = 32768.0;
const SAMPLE_RATE: f32 = 16_000.0;

/// How far back the delay finder looks: 800 ms.
const DELAY_HISTORY: usize = 100;
/// The bins compared: about 250 to 2250 Hz.
const DELAY_FIRST_BIN: usize = 4;
/// The played sound must be at least this loud (per-sample power, 16-bit
/// scale) for a block to count: about -55 dBFS.
const FAR_ACTIVE_POWER: f32 = 3000.0;
/// How many of the 32 compared bits the best delay must beat the average by,
/// and for how many blocks in a row, before it is believed; and the same for
/// replacing a delay found earlier.
const DELAY_MIN_DEPTH: f32 = 4.0;
const DELAY_CONFIRM_BLOCKS: u32 = 40;
const DELAY_MOVE_DEPTH: f32 = 5.0;
const DELAY_MOVE_BLOCKS: u32 = 120;
/// The filter is placed so the echo starts this many blocks into it.
const DELAY_HEADROOM_BLOCKS: usize = 2;

struct DelayFinder {
	far_bits: [u32; DELAY_HISTORY],
	head: usize,
	far_mean: [f32; 32],
	mic_mean: [f32; 32],
	cost: [f32; DELAY_HISTORY],
	seen: u32,
	candidate: usize,
	candidate_blocks: u32,
	found: Option<usize>,
}

impl DelayFinder {
	fn new() -> Self {
		Self {
			far_bits: [0; DELAY_HISTORY],
			head: 0,
			far_mean: [0.0; 32],
			mic_mean: [0.0; 32],
			cost: [16.0; DELAY_HISTORY],
			seen: 0,
			candidate: 0,
			candidate_blocks: 0,
			found: None,
		}
	}

	fn bits(re: &[f32], im: &[f32], mean: &mut [f32; 32]) -> u32 {
		let mut bits = 0u32;
		for (i, m) in mean.iter_mut().enumerate() {
			let k = DELAY_FIRST_BIN + i;
			let p = re[k] * re[k] + im[k] * im[k];
			*m += (p - *m) / 64.0;
			if p > *m {
				bits |= 1 << i;
			}
		}
		bits
	}

	/// Stands in for `update` on a block with nothing played.
	fn skip(&mut self) {
		self.head = (self.head + DELAY_HISTORY - 1) % DELAY_HISTORY;
		self.far_bits[self.head] = 0;
	}

	/// Takes the spectra of one block of played and recorded sound and
	/// updates `found`, the delay in blocks.
	fn update(&mut self, far_re: &[f32], far_im: &[f32], mic_re: &[f32], mic_im: &[f32], far_active: bool) {
		self.head = (self.head + DELAY_HISTORY - 1) % DELAY_HISTORY;
		self.far_bits[self.head] = Self::bits(far_re, far_im, &mut self.far_mean);
		let mic_bits = Self::bits(mic_re, mic_im, &mut self.mic_mean);
		if !far_active {
			return;
		}
		self.seen += 1;
		let (mut best, mut best_cost, mut total) = (0, f32::MAX, 0.0);
		for lag in 0..DELAY_HISTORY {
			let far = self.far_bits[(self.head + lag) % DELAY_HISTORY];
			let differ = (far ^ mic_bits).count_ones() as f32;
			self.cost[lag] += (differ - self.cost[lag]) * 0.02;
			total += self.cost[lag];
			if self.cost[lag] < best_cost {
				best_cost = self.cost[lag];
				best = lag;
			}
		}
		let mean = total / DELAY_HISTORY as f32;
		// Unrelated spectra differ in half their 32 bits; a real echo path
		// stands clear of that. Moving away from a delay already found
		// (the output device changed) takes clearer and longer evidence
		// than finding the first, and small wobbles around it are ignored:
		// talking over the far end throws up false matches.
		let depth = mean - best_cost;
		if self.seen < 50 || depth < DELAY_MIN_DEPTH {
			self.candidate_blocks = 0;
			return;
		}
		if best.abs_diff(self.candidate) <= 1 {
			self.candidate_blocks += 1;
		} else {
			self.candidate = best;
			self.candidate_blocks = 0;
		}
		match self.found {
			None if self.candidate_blocks >= DELAY_CONFIRM_BLOCKS => self.found = Some(self.candidate),
			Some(found)
				if found.abs_diff(self.candidate) > 2
					&& depth >= DELAY_MOVE_DEPTH
					&& self.candidate_blocks >= DELAY_MOVE_BLOCKS =>
			{
				self.found = Some(self.candidate);
			}
			_ => {}
		}
	}
}

pub(crate) struct EchoCanceller {
	fft: Fft,
	mic_in: Vec<f32>,
	far_in: Vec<f32>,
	/// Played sound not yet old enough to be used, see `shift`.
	far_line: Vec<f32>,
	/// How many samples the played sound is held back before the filter.
	shift: usize,
	delay: DelayFinder,
	far_prev: [f32; BLOCK],
	mic_prev: [f32; BLOCK],
	raw_far_prev: [f32; BLOCK],
	/// Spectra of the last PARTS played frames, newest at `x_head`.
	x_re: Vec<f32>,
	x_im: Vec<f32>,
	x_head: usize,
	fg_re: Vec<f32>,
	fg_im: Vec<f32>,
	bg_re: Vec<f32>,
	bg_im: Vec<f32>,
	power: [f32; BINS],
	eh_avg: [f32; BINS],
	yh_avg: [f32; BINS],
	pey: f32,
	pyy: f32,
	davg1: f32,
	davg2: f32,
	dvar1: f32,
	dvar2: f32,
	sum_adapt: f32,
	adapted: bool,
	worse_blocks: u32,
	blocks: usize,
	/// Blocks in a row with nothing played.
	silent_blocks: usize,
	leak: f32,
	/// Smoothed power of the mic and of the output while sound is played.
	mic_power: f32,
	out_power: f32,
	a_re: Vec<f32>,
	a_im: Vec<f32>,
	b_re: Vec<f32>,
	b_im: Vec<f32>,
}

impl EchoCanceller {
	pub(crate) fn new() -> Self {
		Self {
			fft: Fft::new(FRAME),
			mic_in: Vec::with_capacity(BLOCK * 4),
			far_in: Vec::with_capacity(BLOCK * 4),
			far_line: Vec::with_capacity(BLOCK * DELAY_HISTORY),
			shift: 0,
			delay: DelayFinder::new(),
			far_prev: [0.0; BLOCK],
			mic_prev: [0.0; BLOCK],
			raw_far_prev: [0.0; BLOCK],
			x_re: vec![0.0; PARTS * FRAME],
			x_im: vec![0.0; PARTS * FRAME],
			x_head: 0,
			fg_re: vec![0.0; PARTS * BINS],
			fg_im: vec![0.0; PARTS * BINS],
			bg_re: vec![0.0; PARTS * BINS],
			bg_im: vec![0.0; PARTS * BINS],
			power: [0.0; BINS],
			eh_avg: [0.0; BINS],
			yh_avg: [0.0; BINS],
			pey: 1.0,
			pyy: 1.0,
			davg1: 0.0,
			davg2: 0.0,
			dvar1: 0.0,
			dvar2: 0.0,
			sum_adapt: 0.0,
			adapted: false,
			worse_blocks: 0,
			blocks: 0,
			silent_blocks: 0,
			leak: 1.0,
			mic_power: 0.0,
			out_power: 0.0,
			a_re: vec![0.0; FRAME],
			a_im: vec![0.0; FRAME],
			b_re: vec![0.0; FRAME],
			b_im: vec![0.0; FRAME],
		}
	}

	/// The delay between played and recorded sound, in ms, once found.
	pub(crate) fn delay_ms(&self) -> Option<u32> {
		self.delay.found.map(|blocks| (blocks * BLOCK * 1000 / SAMPLE_RATE as usize) as u32)
	}

	/// Takes the same number of mic and played ("far") samples. Appends the
	/// mic with the echo taken out to `out` and, sample for sample beside it,
	/// an estimate of the echo still left in it to `residual`. Works in
	/// blocks, so what comes out trails what went in by up to one block.
	pub(crate) fn process(&mut self, mic: &[f32], far: &[f32], out: &mut Vec<f32>, residual: &mut Vec<f32>) {
		debug_assert_eq!(mic.len(), far.len());
		self.mic_in.extend(mic.iter().map(|s| s * SCALE));
		self.far_in.extend(far.iter().map(|s| s * SCALE));
		let mut used = 0;
		let (mut mic_block, mut far_block) = ([0.0f32; BLOCK], [0.0f32; BLOCK]);
		while self.mic_in.len() - used >= BLOCK {
			mic_block.copy_from_slice(&self.mic_in[used..used + BLOCK]);
			far_block.copy_from_slice(&self.far_in[used..used + BLOCK]);
			used += BLOCK;
			self.process_block(&mic_block, &far_block, out, residual);
		}
		self.mic_in.drain(..used);
		self.far_in.drain(..used);
	}

	fn reset_filter(&mut self) {
		for v in [&mut self.fg_re, &mut self.fg_im, &mut self.bg_re, &mut self.bg_im, &mut self.x_re, &mut self.x_im] {
			v.fill(0.0);
		}
		self.power = [0.0; BINS];
		self.eh_avg = [0.0; BINS];
		self.yh_avg = [0.0; BINS];
		(self.pey, self.pyy, self.leak) = (1.0, 1.0, 1.0);
		(self.davg1, self.davg2, self.dvar1, self.dvar2) = (0.0, 0.0, 0.0, 0.0);
		self.sum_adapt = 0.0;
		self.adapted = false;
		self.worse_blocks = 0;
		(self.mic_power, self.out_power) = (0.0, 0.0);
	}

	fn find_delay(&mut self, mic: &[f32; BLOCK], far: &[f32; BLOCK]) {
		let far_power = far.iter().map(|s| s * s).sum::<f32>() / BLOCK as f32;
		self.a_re[..BLOCK].copy_from_slice(&self.raw_far_prev);
		self.a_re[BLOCK..].copy_from_slice(far);
		self.a_im.fill(0.0);
		self.fft.transform(&mut self.a_re, &mut self.a_im, false);
		self.b_re[..BLOCK].copy_from_slice(&self.mic_prev);
		self.b_re[BLOCK..].copy_from_slice(mic);
		self.b_im.fill(0.0);
		self.fft.transform(&mut self.b_re, &mut self.b_im, false);
		self.raw_far_prev = *far;
		self.delay.update(&self.a_re, &self.a_im, &self.b_re, &self.b_im, far_power > FAR_ACTIVE_POWER);

		let Some(found) = self.delay.found else { return };
		// Leave the filter alone while the echo's start sits inside it with
		// room to spare, or while it is plainly removing echo.
		let now = self.shift / BLOCK;
		if (found >= now + 1 && found + 8 <= now + PARTS) || self.out_power < 0.25 * self.mic_power {
			return;
		}
		let wanted = found.saturating_sub(DELAY_HEADROOM_BLOCKS);
		if wanted != now {
			self.shift = wanted * BLOCK;
			self.reset_filter();
		}
	}

	/// Filters the played frames through `w`, leaving the predicted echo for
	/// this block in `self.a_re[BLOCK..]`.
	fn predict(&mut self, foreground: bool) {
		let (w_re, w_im) = if foreground { (&self.fg_re, &self.fg_im) } else { (&self.bg_re, &self.bg_im) };
		self.a_re.fill(0.0);
		self.a_im.fill(0.0);
		for m in 0..PARTS {
			let x = ((self.x_head + m) % PARTS) * FRAME;
			let w = m * BINS;
			for k in 0..BINS {
				let (xr, xi) = (self.x_re[x + k], self.x_im[x + k]);
				let (wr, wi) = (w_re[w + k], w_im[w + k]);
				self.a_re[k] += xr * wr - xi * wi;
				self.a_im[k] += xr * wi + xi * wr;
			}
		}
		for k in 1..FRAME / 2 {
			self.a_re[FRAME - k] = self.a_re[k];
			self.a_im[FRAME - k] = -self.a_im[k];
		}
		self.fft.transform(&mut self.a_re, &mut self.a_im, true);
	}

	fn process_block(
		&mut self,
		mic: &[f32; BLOCK],
		raw_far: &[f32; BLOCK],
		out: &mut Vec<f32>,
		residual: &mut Vec<f32>,
	) {
		// With nothing played for longer than the filter reaches back there
		// is no echo to predict: pass the mic through and skip the work.
		if raw_far.iter().all(|s| s.abs() < 1.0) {
			self.silent_blocks = self.silent_blocks.saturating_add(1);
		} else {
			self.silent_blocks = 0;
		}
		if self.silent_blocks > self.shift / BLOCK + PARTS + 1 {
			self.delay.skip();
			self.raw_far_prev = *raw_far;
			self.mic_prev = *mic;
			self.far_line.extend_from_slice(raw_far);
			let keep = DELAY_HISTORY * BLOCK;
			if self.far_line.len() > keep * 2 {
				self.far_line.drain(..self.far_line.len() - keep);
			}
			out.extend(mic.iter().map(|s| s / SCALE));
			residual.extend(std::iter::repeat_n(0.0, BLOCK));
			return;
		}
		self.find_delay(mic, raw_far);
		self.mic_prev = *mic;

		// Hold the played sound back by `shift`.
		self.far_line.extend_from_slice(raw_far);
		let mut far = [0.0f32; BLOCK];
		if self.far_line.len() >= self.shift + BLOCK {
			let start = self.far_line.len() - self.shift - BLOCK;
			far.copy_from_slice(&self.far_line[start..start + BLOCK]);
		}
		let keep = DELAY_HISTORY * BLOCK;
		if self.far_line.len() > keep * 2 {
			self.far_line.drain(..self.far_line.len() - keep);
		}

		// Newest played frame into the history.
		self.x_head = (self.x_head + PARTS - 1) % PARTS;
		let x = self.x_head * FRAME;
		self.x_re[x..x + BLOCK].copy_from_slice(&self.far_prev);
		self.x_re[x + BLOCK..x + FRAME].copy_from_slice(&far);
		self.x_im[x..x + FRAME].fill(0.0);
		self.fft.transform(&mut self.x_re[x..x + FRAME], &mut self.x_im[x..x + FRAME], false);
		self.far_prev = far;

		let sxx: f32 = far.iter().map(|s| s * s).sum();
		let sdd: f32 = mic.iter().map(|s| s * s).sum();

		let (mut y_fg, mut e_fg, mut y_bg, mut e_bg) =
			([0.0f32; BLOCK], [0.0f32; BLOCK], [0.0f32; BLOCK], [0.0f32; BLOCK]);
		self.predict(true);
		y_fg.copy_from_slice(&self.a_re[BLOCK..]);
		self.predict(false);
		y_bg.copy_from_slice(&self.a_re[BLOCK..]);
		for i in 0..BLOCK {
			e_fg[i] = mic[i] - y_fg[i];
			e_bg[i] = mic[i] - y_bg[i];
		}
		let mut sff: f32 = e_fg.iter().map(|s| s * s).sum();
		let mut see: f32 = e_bg.iter().map(|s| s * s).sum();
		let dbf: f32 = 10.0 + y_fg.iter().zip(&y_bg).map(|(f, b)| (f - b) * (f - b)).sum::<f32>();

		// Has the background filter proved itself better (take it over), or
		// worse (throw it away and restart it from the foreground)?
		let gain = sff - see;
		self.davg1 = 0.6 * self.davg1 + 0.4 * gain;
		self.davg2 = 0.85 * self.davg2 + 0.15 * gain;
		self.dvar1 = 0.36 * self.dvar1 + 0.16 * sff * dbf;
		self.dvar2 = 0.7225 * self.dvar2 + 0.0225 * sff * dbf;
		let better = gain * gain.abs() > sff * dbf
			|| self.davg1 * self.davg1.abs() > 0.5 * self.dvar1
			|| self.davg2 * self.davg2.abs() > 0.25 * self.dvar2;
		let worse = -gain * gain.abs() > 4.0 * sff * dbf
			|| -self.davg1 * self.davg1.abs() > 4.0 * self.dvar1
			|| -self.davg2 * self.davg2.abs() > 4.0 * self.dvar2;
		if better {
			self.fg_re.copy_from_slice(&self.bg_re);
			self.fg_im.copy_from_slice(&self.bg_im);
			(y_fg, e_fg, sff) = (y_bg, e_bg, see);
			(self.davg1, self.davg2, self.dvar1, self.dvar2) = (0.0, 0.0, 0.0, 0.0);
		} else if worse {
			self.bg_re.copy_from_slice(&self.fg_re);
			self.bg_im.copy_from_slice(&self.fg_im);
			(y_bg, e_bg, see) = (y_fg, e_fg, sff);
			(self.davg1, self.davg2, self.dvar1, self.dvar2) = (0.0, 0.0, 0.0, 0.0);
		}

		// Spectra of the background filter's error and of its echo estimate.
		self.a_re[..BLOCK].fill(0.0);
		self.a_re[BLOCK..].copy_from_slice(&e_bg);
		self.a_im.fill(0.0);
		self.fft.transform(&mut self.a_re, &mut self.a_im, false);
		self.b_re[..BLOCK].fill(0.0);
		self.b_re[BLOCK..].copy_from_slice(&y_bg);
		self.b_im.fill(0.0);
		self.fft.transform(&mut self.b_re, &mut self.b_im, false);

		// How much of what is left is still echo: the part of the error's
		// spectrum that moves together with the echo estimate's.
		let syy: f32 = y_bg.iter().map(|s| s * s).sum();
		let sey: f32 = e_bg.iter().zip(&y_bg).map(|(e, y)| e * y).sum();
		let spec_average = BLOCK as f32 / SAMPLE_RATE;
		let (mut pey_now, mut pyy_now) = (1.0f32, 1.0f32);
		let mut err_power = [0.0f32; BINS];
		let mut est_power = [0.0f32; BINS];
		let x = self.x_head * FRAME;
		let power_smooth = 0.35 / PARTS as f32;
		for k in 0..BINS {
			err_power[k] = self.a_re[k] * self.a_re[k] + self.a_im[k] * self.a_im[k];
			est_power[k] = self.b_re[k] * self.b_re[k] + self.b_im[k] * self.b_im[k];
			let eh = err_power[k] - self.eh_avg[k];
			let yh = est_power[k] - self.yh_avg[k];
			pey_now += eh * yh;
			pyy_now += yh * yh;
			self.eh_avg[k] += spec_average * (err_power[k] - self.eh_avg[k]);
			self.yh_avg[k] += spec_average * (est_power[k] - self.yh_avg[k]);
			let xp = self.x_re[x + k] * self.x_re[x + k] + self.x_im[x + k] * self.x_im[x + k];
			self.power[k] = (1.0 - power_smooth) * self.power[k] + 1.0 + power_smooth * xp;
		}
		let pyy_now = pyy_now.sqrt();
		let pey_now = pey_now / pyy_now;
		let see_safe = see.max(1.0);
		let alpha = (2.0 * spec_average * syy / see_safe).min(0.5 * spec_average);
		self.pey = (1.0 - alpha) * self.pey + alpha * pey_now;
		self.pyy = ((1.0 - alpha) * self.pyy + alpha * pyy_now).max(1.0);
		self.pey = self.pey.clamp(0.005 * self.pyy, self.pyy);
		self.leak = self.pey / self.pyy;
		let rer = ((0.0001 * sxx + 3.0 * self.leak * syy) / see_safe).max(sey * sey / (1.0 + see * syy)).min(0.5);

		if !self.adapted && self.sum_adapt > PARTS as f32 && self.leak * syy > 0.03 * syy {
			self.adapted = true;
		}
		let mut step = [0.0f32; BINS];
		if self.adapted {
			for k in 0..BINS {
				let e = err_power[k] + 1.0;
				let r = (self.leak * est_power[k]).min(0.5 * e);
				step[k] = (0.7 * r + 0.3 * rer * e) / (e * (self.power[k] + 10.0));
			}
		} else {
			let rate = if sxx > BLOCK as f32 * 1000.0 { (0.25 * sxx).min(0.25 * see) / see_safe } else { 0.0 };
			for k in 0..BINS {
				step[k] = rate / (self.power[k] + 10.0);
			}
			self.sum_adapt += rate;
		}

		// Move the background filter along the error's gradient, keeping
		// each partition's impulse response to its own block.
		// Partitions that already carry more of the echo path learn faster
		// (proportionate adaptation); the shares add up to one.
		let mut share = [0.0f32; PARTS];
		for (m, s) in share.iter_mut().enumerate() {
			let w = m * BINS;
			let energy: f32 =
				(w..w + BINS).map(|i| self.bg_re[i] * self.bg_re[i] + self.bg_im[i] * self.bg_im[i]).sum();
			*s = (1.0 + energy).sqrt();
		}
		let largest = share.iter().copied().fold(0.0, f32::max);
		let total: f32 = share.iter().map(|s| s + 0.1 * largest).sum();
		let (mut err_re, mut err_im) = ([0.0f32; BINS], [0.0f32; BINS]);
		err_re.copy_from_slice(&self.a_re[..BINS]);
		err_im.copy_from_slice(&self.a_im[..BINS]);
		self.blocks = self.blocks.wrapping_add(1);
		for m in 0..PARTS {
			let x = ((self.x_head + m) % PARTS) * FRAME;
			let w = m * BINS;
			let part = (share[m] + 0.1 * largest) / total;
			for k in 0..BINS {
				let (xr, xi) = (self.x_re[x + k], self.x_im[x + k]);
				let mu = step[k] * part;
				self.b_re[k] = self.bg_re[w + k] + mu * (xr * err_re[k] + xi * err_im[k]);
				self.b_im[k] = self.bg_im[w + k] + mu * (xr * err_im[k] - xi * err_re[k]);
			}
			// Trimming a partition back to its own block costs two FFTs, so
			// only the first (where most of the echo sits) and one other in
			// turn are trimmed each block, as Speex does.
			if m == 0 || m == 1 + self.blocks % (PARTS - 1) {
				for k in 1..FRAME / 2 {
					self.b_re[FRAME - k] = self.b_re[k];
					self.b_im[FRAME - k] = -self.b_im[k];
				}
				self.fft.transform(&mut self.b_re, &mut self.b_im, true);
				self.b_re[BLOCK..].fill(0.0);
				self.b_im.fill(0.0);
				self.fft.transform(&mut self.b_re, &mut self.b_im, false);
			}
			self.bg_re[w..w + BINS].copy_from_slice(&self.b_re[..BINS]);
			self.bg_im[w..w + BINS].copy_from_slice(&self.b_im[..BINS]);
		}

		// A filter that makes things louder than the mic alone has gone
		// wrong: pass the mic through, and start over if it stays that way.
		let broken = !(sff.is_finite() && see.is_finite()) || sff > sdd + BLOCK as f32 * 10_000.0;
		if broken {
			self.worse_blocks += 1;
			if self.worse_blocks >= 50 || !sff.is_finite() || !see.is_finite() {
				self.reset_filter();
			}
			out.extend(mic.iter().map(|s| s / SCALE));
			residual.extend(std::iter::repeat_n(0.0, BLOCK));
			return;
		}
		self.worse_blocks = 0;
		if sxx > BLOCK as f32 * FAR_ACTIVE_POWER {
			self.mic_power += 0.02 * (sdd - self.mic_power);
			self.out_power += 0.02 * (sff - self.out_power);
		}
		let left = if self.leak > 0.5 { 1.0 } else { (2.0 * self.leak).sqrt() };
		out.extend(e_fg.iter().map(|s| s / SCALE));
		residual.extend(y_fg.iter().map(|s| s * left / SCALE));
	}
}

const SUPPRESS_HOP: usize = 160;
const SUPPRESS_WINDOW: usize = SUPPRESS_HOP * 2;
const SUPPRESS_FFT: usize = 512;
const SUPPRESS_BINS: usize = SUPPRESS_FFT / 2 + 1;
/// The most a frequency that is all echo is turned down: 36 dB.
const ECHO_FLOOR: f32 = 0.016;
/// An echo estimate is kept, fading, for the room's tail after it.
const ECHO_HOLD: f32 = 0.75;
/// Echo estimates under this power (about -100 dBFS per bin) are nothing.
const NO_ECHO: f32 = 1e-9;
/// The estimate is of the echo's level, not its exact spectrum, so it is
/// taken as this much larger to cover the frequencies it falls short on.
const ECHO_OVERESTIMATE: f32 = 4.0;

/// Turns down the frequencies where echo is left after `EchoCanceller`,
/// given its echo estimate. A Wiener gain per frequency on 10 ms hops with
/// half-overlapped windows, so it adds one hop of delay; with no echo
/// estimate the sound passes through untouched.
pub(crate) struct EchoSuppressor {
	window: Vec<f32>,
	input: Vec<f32>,
	echo_input: Vec<f32>,
	overlap: Vec<f32>,
	fft: Fft,
	re: Vec<f32>,
	im: Vec<f32>,
	echo_re: Vec<f32>,
	echo_im: Vec<f32>,
	echo: Vec<f32>,
	prev_clean: Vec<f32>,
	gains: Vec<f32>,
}

impl EchoSuppressor {
	pub(crate) fn new() -> Self {
		Self {
			window: (0..SUPPRESS_WINDOW)
				.map(|i| (std::f32::consts::PI * i as f32 / SUPPRESS_WINDOW as f32).sin())
				.collect(),
			input: vec![0.0; SUPPRESS_WINDOW],
			echo_input: vec![0.0; SUPPRESS_WINDOW],
			overlap: vec![0.0; SUPPRESS_HOP],
			fft: Fft::new(SUPPRESS_FFT),
			re: vec![0.0; SUPPRESS_FFT],
			im: vec![0.0; SUPPRESS_FFT],
			echo_re: vec![0.0; SUPPRESS_FFT],
			echo_im: vec![0.0; SUPPRESS_FFT],
			echo: vec![0.0; SUPPRESS_BINS],
			prev_clean: vec![0.0; SUPPRESS_BINS],
			gains: vec![1.0; SUPPRESS_BINS],
		}
	}

	/// Takes one hop of the canceller's output and of its echo estimate,
	/// writes one hop (from one hop ago) and returns how large the echo
	/// estimate was beside the input, 0 for none to 1 and over for a hop that
	/// was all echo.
	pub(crate) fn process_hop(&mut self, input: &[f32], residual: &[f32], out: &mut [f32]) -> f32 {
		let hop = SUPPRESS_HOP;
		debug_assert!(input.len() == hop && residual.len() == hop && out.len() == hop);
		self.input.copy_within(hop.., 0);
		self.input[hop..].copy_from_slice(input);
		self.echo_input.copy_within(hop.., 0);
		self.echo_input[hop..].copy_from_slice(residual);
		for i in 0..SUPPRESS_FFT {
			let w = if i < SUPPRESS_WINDOW { self.window[i] } else { 0.0 };
			self.re[i] = if i < SUPPRESS_WINDOW { self.input[i] * w } else { 0.0 };
			self.echo_re[i] = if i < SUPPRESS_WINDOW { self.echo_input[i] * w } else { 0.0 };
		}
		self.im.fill(0.0);
		self.echo_im.fill(0.0);
		self.fft.transform(&mut self.re, &mut self.im, false);
		self.fft.transform(&mut self.echo_re, &mut self.echo_im, false);

		let (mut power_in, mut power_echo) = (1e-12f32, 0.0f32);
		for k in 0..SUPPRESS_BINS {
			let p = self.re[k] * self.re[k] + self.im[k] * self.im[k];
			let now = ECHO_OVERESTIMATE * (self.echo_re[k] * self.echo_re[k] + self.echo_im[k] * self.echo_im[k]);
			let echo = now.max(ECHO_HOLD * self.echo[k]);
			self.echo[k] = echo;
			let g = if echo < NO_ECHO {
				1.0
			} else {
				let xi = 0.9 * self.prev_clean[k] / echo + 0.1 * (p / echo - 1.0).max(0.0);
				(xi / (1.0 + xi)).max(ECHO_FLOOR)
			};
			self.gains[k] = g;
			self.prev_clean[k] = g * g * p;
			power_in += p;
			power_echo += echo;
		}
		let mut prev = self.gains[0];
		for k in 1..SUPPRESS_BINS - 1 {
			let cur = self.gains[k];
			self.gains[k] = 0.25 * prev + 0.5 * cur + 0.25 * self.gains[k + 1];
			prev = cur;
		}
		for k in 0..SUPPRESS_BINS {
			let g = self.gains[k];
			self.re[k] *= g;
			self.im[k] *= g;
			if k != 0 && k != SUPPRESS_FFT / 2 {
				self.re[SUPPRESS_FFT - k] *= g;
				self.im[SUPPRESS_FFT - k] *= g;
			}
		}
		self.fft.transform(&mut self.re, &mut self.im, true);
		for i in 0..hop {
			out[i] = self.overlap[i] + self.re[i] * self.window[i];
			self.overlap[i] = self.re[hop + i] * self.window[hop + i];
		}
		power_echo / power_in
	}
}
