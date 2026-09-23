//! Voice-message clean-up, applied while recording when Settings -> "Clean up
//! voice messages" is on.
//!
//! WhatsApp Web opens the mic with a plain `getUserMedia({audio: true})`, so the
//! browser's WebRTC voice processing always runs on its voice notes: high-pass
//! filter, noise suppression, automatic gain control. That - not anything
//! WhatsApp does itself - is why its recordings come out loud and clean however
//! the Windows mic enhancements are set. This is the same chain, in the same
//! order:
//!
//! 1. an 80 Hz high-pass (desk thumps, breath pops, mains rumble),
//! 2. RNNoise noise suppression (`nnnoiseless`), whose voice-activity estimate
//!    also drives
//! 3. an automatic gain control that brings speech to a steady level - it only
//!    measures and only raises the gain while someone is talking, so pauses
//!    don't get pumped up into audible hiss,
//! 4. a peak limiter so the raised gain can never clip.
//!
//! Works on 48 kHz stereo in `[-1, 1]`; RNNoise needs exactly 48 kHz. Each
//! channel is filtered and denoised on its own, but both share one gain and
//! one limiter, so the stereo balance is never pulled around.

use nnnoiseless::DenoiseState;

const FRAME: usize = DenoiseState::FRAME_SIZE; // 480 samples = 10 ms at 48 kHz
const SAMPLE_RATE: f32 = 48_000.0;

/// Average speech level the gain control aims for, in dBFS (RMS over the
/// 10 ms frames RNNoise marks as speech). Syllable peaks sit ~12-15 dB above
/// this, so it lands loud without leaning on the limiter.
const TARGET_SPEECH_DBFS: f32 = -20.0;
const MAX_GAIN_DB: f32 = 30.0;
const MIN_GAIN_DB: f32 = -12.0;
/// Frames RNNoise must rate at least this likely to be speech before they
/// count towards the level estimate or may raise the gain.
const SPEECH_VAD: f32 = 0.6;
/// Quieter than this isn't speech however confident the VAD is.
const SPEECH_FLOOR_DBFS: f32 = -60.0;
/// Speech frames of fast adaptation at the start (about half a second of
/// talking), so the first words aren't left quiet while the estimate settles.
const FAST_START_FRAMES: u32 = 50;
/// The gain is never raised so far that the background left after noise
/// suppression would come out louder than this - WebRTC's gain control has
/// the same cap (`max_output_noise_level_dbfs`, also -50). Without it, a
/// recording whose noise the VAD mistakes for speech just gets louder hiss.
const MAX_OUTPUT_NOISE_DBFS: f32 = -50.0;
/// How fast the background estimate may creep up, per 10 ms frame (0.5 dB/s),
/// so a long stretch of talking isn't mistaken for a louder background.
const NOISE_RISE_DB_PER_FRAME: f32 = 0.005;
const LIMIT_CEILING: f32 = 0.9; // about -1 dBFS
const LIMIT_RELEASE_SECS: f32 = 0.08;

pub(crate) struct VoiceCleanup {
	hpf: [HighPass; 2],
	denoise: [Box<DenoiseState<'static>>; 2],
	agc: Agc,
	pending: [Vec<f32>; 2],
	frame_out: [Vec<f32>; 2],
	/// RNNoise's output lags its input by one frame; the first frame out is
	/// that delay (plus its fade-in), so it is dropped.
	skip: usize,
	samples_in: usize,
	samples_out: usize,
}

impl VoiceCleanup {
	pub(crate) fn new() -> Self {
		Self {
			hpf: [HighPass::new(80.0), HighPass::new(80.0)],
			denoise: [DenoiseState::new(), DenoiseState::new()],
			agc: Agc::new(),
			pending: [Vec::with_capacity(FRAME * 2), Vec::with_capacity(FRAME * 2)],
			frame_out: [vec![0.0; FRAME], vec![0.0; FRAME]],
			skip: FRAME,
			samples_in: 0,
			samples_out: 0,
		}
	}

	/// Feeds mic frames in; returns whatever processed audio is ready, which
	/// trails the input by up to two 10 ms frames.
	pub(crate) fn process(&mut self, input: &[[f32; 2]]) -> Vec<[f32; 2]> {
		self.samples_in += input.len();
		for frame in input {
			for c in 0..2 {
				let filtered = self.hpf[c].process(frame[c]);
				self.pending[c].push(filtered);
			}
		}
		let mut out = Vec::with_capacity(self.pending[0].len());
		self.drain_frames(&mut out);
		out
	}

	/// Pushes out the audio still buffered, so the result is exactly as long
	/// as everything fed in.
	pub(crate) fn finish(&mut self) -> Vec<[f32; 2]> {
		let mut out = Vec::new();
		while self.samples_out < self.samples_in {
			let missing = FRAME - self.pending[0].len() % FRAME;
			for p in &mut self.pending {
				p.extend(std::iter::repeat_n(0.0, missing));
			}
			self.drain_frames(&mut out);
		}
		let excess = self.samples_out - self.samples_in;
		out.truncate(out.len() - excess.min(out.len()));
		self.samples_out -= excess;
		out
	}

	fn drain_frames(&mut self, out: &mut Vec<[f32; 2]>) {
		let mut frame_in = [0.0f32; FRAME];
		while self.pending[0].len() >= FRAME {
			let mut vad = 0.0f32;
			for c in 0..2 {
				for (d, s) in frame_in.iter_mut().zip(self.pending[c].drain(..FRAME)) {
					*d = s * 32768.0;
				}
				vad = vad.max(self.denoise[c].process_frame(&mut self.frame_out[c], &frame_in));
				for s in self.frame_out[c].iter_mut() {
					*s /= 32768.0;
				}
			}
			let [left, right] = &mut self.frame_out;
			self.agc.process(left, right, vad);
			let start = self.skip.min(FRAME);
			self.skip -= start;
			out.extend((start..FRAME).map(|i| [left[i], right[i]]));
			self.samples_out += FRAME - start;
		}
	}
}

/// Second-order Butterworth high-pass (RBJ cookbook biquad).
struct HighPass {
	b0: f32,
	b1: f32,
	b2: f32,
	a1: f32,
	a2: f32,
	x1: f32,
	x2: f32,
	y1: f32,
	y2: f32,
}

impl HighPass {
	fn new(cutoff_hz: f32) -> Self {
		let w0 = 2.0 * std::f32::consts::PI * cutoff_hz / SAMPLE_RATE;
		let alpha = w0.sin() / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
		let cos = w0.cos();
		let a0 = 1.0 + alpha;
		Self {
			b0: (1.0 + cos) / 2.0 / a0,
			b1: -(1.0 + cos) / a0,
			b2: (1.0 + cos) / 2.0 / a0,
			a1: -2.0 * cos / a0,
			a2: (1.0 - alpha) / a0,
			x1: 0.0,
			x2: 0.0,
			y1: 0.0,
			y2: 0.0,
		}
	}

	fn process(&mut self, x: f32) -> f32 {
		let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
		self.x2 = self.x1;
		self.x1 = x;
		self.y2 = self.y1;
		self.y1 = y;
		y
	}
}

struct Agc {
	/// Running estimate of the speech level, dBFS; `None` until someone speaks.
	level_db: Option<f32>,
	/// Background level estimate, dBFS: follows the quietest frames down at
	/// once and creeps back up slowly (minimum tracking).
	noise_db: Option<f32>,
	speech_frames: u32,
	gain_db: f32,
	limiter_gain: f32,
	limiter_release: f32,
}

impl Agc {
	fn new() -> Self {
		Self {
			level_db: None,
			noise_db: None,
			speech_frames: 0,
			gain_db: 0.0,
			limiter_gain: 1.0,
			limiter_release: 1.0 - (-1.0 / (LIMIT_RELEASE_SECS * SAMPLE_RATE)).exp(),
		}
	}

	/// Levels one 10 ms stereo frame in place. `vad` is RNNoise's confidence
	/// that it holds speech.
	fn process(&mut self, left: &mut [f32], right: &mut [f32], vad: f32) {
		let energy = left.iter().chain(right.iter()).map(|s| s * s).sum::<f32>() / (left.len() * 2).max(1) as f32;
		let rms_db = 10.0 * (energy + 1e-12).log10();
		let is_speech = vad >= SPEECH_VAD && rms_db > SPEECH_FLOOR_DBFS;
		let fast = self.speech_frames < FAST_START_FRAMES;
		let noise_db = match self.noise_db {
			Some(n) if rms_db >= n => n + NOISE_RISE_DB_PER_FRAME,
			_ => rms_db,
		};
		self.noise_db = Some(noise_db);

		if is_speech {
			self.level_db = Some(match self.level_db {
				None => rms_db,
				Some(l) => l + if fast { 0.15 } else { 0.01 } * (rms_db - l),
			});
			self.speech_frames += 1;
		}

		let prev_gain_db = self.gain_db;
		if let Some(level) = self.level_db {
			let noise_cap = (MAX_OUTPUT_NOISE_DBFS - noise_db).max(0.0);
			let wanted = (TARGET_SPEECH_DBFS - level).clamp(MIN_GAIN_DB, MAX_GAIN_DB).min(noise_cap);
			let delta = wanted - self.gain_db;
			// Raise only while speech is present (so silence is never pumped
			// up), at up to 10 dB/s once settled; lower at up to 50 dB/s.
			self.gain_db += if delta > 0.0 {
				if is_speech { delta.min(if fast { 0.5 } else { 0.1 }) } else { 0.0 }
			} else {
				delta.max(-0.5)
			};
		}

		// Ramp across the frame rather than stepping at its edge.
		let g0 = db_to_linear(prev_gain_db);
		let g1 = db_to_linear(self.gain_db);
		let n = left.len() as f32;
		for (i, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
			let g = g0 + (g1 - g0) * (i as f32 + 1.0) / n;
			let (yl, yr) = (*l * g, *r * g);
			let peak = yl.abs().max(yr.abs());
			// Recover towards unity first, then clamp: the other way round, the
			// recovery step could push this very sample over the ceiling.
			self.limiter_gain += (1.0 - self.limiter_gain) * self.limiter_release;
			if peak * self.limiter_gain > LIMIT_CEILING {
				self.limiter_gain = LIMIT_CEILING / peak;
			}
			*l = yl * self.limiter_gain;
			*r = yr * self.limiter_gain;
		}
	}
}

fn db_to_linear(db: f32) -> f32 {
	10f32.powf(db / 20.0)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn tone(amplitude: f32, hz: f32, samples: usize) -> Vec<f32> {
		(0..samples).map(|i| amplitude * (2.0 * std::f32::consts::PI * hz * i as f32 / SAMPLE_RATE).sin()).collect()
	}

	/// Runs the AGC on a mono frame fed to both channels; returns the left.
	fn agc_mono(agc: &mut Agc, f: &mut Vec<f32>, vad: f32) {
		let mut r = f.clone();
		agc.process(f, &mut r, vad);
		assert_eq!(*f, r, "a mono signal must stay identical on both channels");
	}

	fn rms_db(x: &[f32]) -> f32 {
		10.0 * (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32 + 1e-12).log10()
	}

	#[test]
	fn output_length_matches_input() {
		let mut c = VoiceCleanup::new();
		let input: Vec<[f32; 2]> = tone(0.1, 300.0, 48_000 + 123).into_iter().map(|s| [s, s]).collect();
		let mut out = Vec::new();
		for chunk in input.chunks(441) {
			out.extend(c.process(chunk));
		}
		out.extend(c.finish());
		assert_eq!(out.len(), input.len());
		assert!(out.iter().flatten().all(|s| s.is_finite() && s.abs() <= 1.0));
		// A mono mic arrives as L=R and must leave that way.
		assert!(out.iter().all(|[l, r]| l == r));
	}

	#[test]
	fn high_pass_removes_rumble_keeps_voice() {
		let mut hpf = HighPass::new(80.0);
		let rumble: Vec<f32> = tone(0.5, 20.0, 48_000).into_iter().map(|s| hpf.process(s)).collect();
		let mut hpf = HighPass::new(80.0);
		let voice: Vec<f32> = tone(0.5, 1000.0, 48_000).into_iter().map(|s| hpf.process(s)).collect();
		assert!(rms_db(&rumble[24_000..]) < rms_db(&tone(0.5, 20.0, 24_000)) - 20.0);
		assert!((rms_db(&voice[24_000..]) - rms_db(&tone(0.5, 1000.0, 24_000))).abs() < 0.5);
	}

	#[test]
	fn agc_raises_quiet_speech_to_target() {
		// Talking at about -43 dBFS: 200 ms bursts with near-silent gaps, since
		// an unbroken tone is indistinguishable from steady background noise.
		let mut agc = Agc::new();
		let quiet = tone(0.01, 300.0, FRAME * 600);
		let mut last = Vec::new();
		for (i, chunk) in quiet.chunks(FRAME).enumerate() {
			let talking = (i / 20) % 2 == 0;
			let mut f: Vec<f32> = chunk.iter().map(|s| if talking { *s } else { s * 0.001 }).collect();
			agc_mono(&mut agc, &mut f, if talking { 1.0 } else { 0.0 });
			if talking {
				last = f;
			}
		}
		let got = rms_db(&last);
		assert!((got - TARGET_SPEECH_DBFS).abs() < 2.0, "settled at {got} dBFS");
	}

	#[test]
	fn agc_does_not_boost_silence() {
		let mut agc = Agc::new();
		for chunk in tone(0.01, 300.0, FRAME * 300).chunks(FRAME) {
			let mut f = chunk.to_vec();
			agc_mono(&mut agc, &mut f, 1.0);
		}
		let settled = agc.gain_db;
		for chunk in tone(0.001, 300.0, FRAME * 300).chunks(FRAME) {
			let mut f = chunk.to_vec();
			agc_mono(&mut agc, &mut f, 0.0);
		}
		assert!(agc.gain_db <= settled);
	}

	#[test]
	fn agc_does_not_amplify_loud_background() {
		// Hiss the VAD takes for speech: raising it would only make it louder.
		let mut agc = Agc::new();
		let mut seed = 7u32;
		for _ in 0..500 {
			let mut f: Vec<f32> = (0..FRAME)
				.map(|_| {
					seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
					(seed as f32 / u32::MAX as f32 - 0.5) * 0.03
				})
				.collect();
			agc_mono(&mut agc, &mut f, 1.0);
		}
		assert!(agc.gain_db <= 0.01, "gain rose to {}", agc.gain_db);
	}

	#[test]
	fn limiter_never_clips() {
		let mut agc = Agc::new();
		agc.level_db = Some(-40.0);
		agc.gain_db = MAX_GAIN_DB;
		let mut f = tone(0.8, 300.0, FRAME);
		agc_mono(&mut agc, &mut f, 1.0);
		assert!(f.iter().all(|s| s.abs() <= LIMIT_CEILING + 1e-6));
	}
}
