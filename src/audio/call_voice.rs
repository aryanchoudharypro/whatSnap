//! The clean-up a call's microphone goes through before it is encoded, in
//! the order WebRTC and WhatsApp's own engine use: high-pass, echo canceller
//! (audio/aec.rs), noise suppressor (the same one voice messages use,
//! audio/voice_dsp.rs), auto gain, then a gate that lowers the background
//! between words. Nothing here touches Windows, so the whole chain runs in
//! tests.
//!
//! 16 kHz mono in and out. The noise suppressor works at 48 kHz, so the
//! sound is brought up to that for it and back down after.

use super::{
	aec::{EchoCanceller, EchoSuppressor},
	voice_dsp::{HighPass, SpeechDenoiser},
};

const SAMPLE_RATE: f32 = 16_000.0;
const MIC_CLIP_LEVEL: f32 = 0.95;
const MIC_RAISE_ABOVE_GAIN_DB: f32 = 20.0;

// Auto gain for the outgoing call audio, modelled on WebRTC's AGC2 adaptive
// digital controller, which is what WhatsApp for Windows' native engine runs
// (WhatsAppNative.Voip.dll ships webrtc/latest agc2 and the server sends it
// agc {"enable_uwp_agc2":"true"}); WA Web gets the same from the browser's
// getUserMedia autoGainControl. Windows' communications effects barely level
// the mic: live calls measured voiced speech around -40 dBFS, and phones
// that receive standard Opus play it as sent.
//
// Like AGC2 it works on 10 ms slices: a simple energy VAD over a tracked
// noise floor, a slow speech-level estimate fed only by speech, a gain that
// moves at most AGC_MAX_GAIN_CHANGE_DB_PER_S towards bringing that speech
// level to AGC_TARGET_DBFS, capped so the noise floor isn't lifted above
// AGC_MAX_NOISE_DBFS. The gain falls faster than it rises, so a loud start
// or a raised mic level isn't left too hot for seconds. Peaks are handled
// after the system audio mix by a lookahead limiter (audio/limiter.rs).
// Runs after the echo canceller and the noise suppressor, as in WebRTC.
pub(crate) const AGC_SLICE: usize = 160; // 10 ms at 16 kHz, one noise suppressor hop
pub(crate) const AGC_TARGET_DBFS: f32 = -20.0;
const AGC_MAX_GAIN_DB: f32 = 40.0;
const AGC_INITIAL_GAIN_DB: f32 = 15.0;
const AGC_MAX_GAIN_CHANGE_DB_PER_S: f32 = 6.0;
const AGC_MAX_GAIN_DROP_DB_PER_S: f32 = 30.0;
// As AGC2: the background left between words is never lifted above this.
// The noise suppressor in front lowers it first, so speech still reaches the
// target (live calls had a -52..-65 dBFS floor before suppression).
const AGC_MAX_NOISE_DBFS: f32 = -50.0;
const AGC_SPEECH_ABOVE_NOISE_DB: f32 = 10.0;
const AGC_MIN_SPEECH_DBFS: f32 = -65.0;
const AGC_SPEECH_VERDICT: f32 = 0.5;

pub(crate) struct VoiceAgc {
	gain_db: f32,
	noise_db: f32,
	speech_db: Option<f32>,
	// Since the mic level controller last looked: speech slices, speech
	// slices that needed more than MIC_RAISE_ABOVE_GAIN_DB of gain, and
	// slices whose raw input came close to clipping.
	pub(crate) speech_slices: u32,
	pub(crate) quiet_speech_slices: u32,
	pub(crate) clipped_slices: u32,
}

impl VoiceAgc {
	fn new() -> Self {
		Self {
			gain_db: AGC_INITIAL_GAIN_DB,
			noise_db: -70.0,
			speech_db: None,
			speech_slices: 0,
			quiet_speech_slices: 0,
			clipped_slices: 0,
		}
	}

	#[cfg(test)]
	fn process(&mut self, frame: &mut [f32]) {
		for slice in frame.chunks_mut(AGC_SLICE) {
			self.process_slice(slice, true, 1.0);
		}
	}

	/// Applies the gain to one slice. With `learn` off (the slice is mostly
	/// echo of the far end, not our voice) the levels and the gain stay put.
	/// `speech` is the noise suppressor's verdict on the slice, 0..1: only
	/// what it takes for a voice counts towards the speech level, so clicks
	/// and quiet in-between sounds don't pull the gain about.
	fn process_slice(&mut self, slice: &mut [f32], learn: bool, speech: f32) {
		if slice.is_empty() {
			return;
		}
		let rms = (slice.iter().map(|s| s * s).sum::<f32>() / slice.len() as f32).sqrt();
		let level_db = 20.0 * rms.max(1e-6).log10();
		let old_gain_db = self.gain_db;
		if slice.iter().any(|s| s.abs() > MIC_CLIP_LEVEL) {
			self.clipped_slices += 1;
		}
		if rms > 0.0 && learn {
			// Noise floor: follows quiet slices at once, creeps up 1 dB/s.
			self.noise_db = if level_db < self.noise_db { level_db } else { self.noise_db + 0.01 };
			if speech >= AGC_SPEECH_VERDICT
				&& level_db > AGC_MIN_SPEECH_DBFS
				&& level_db > self.noise_db + AGC_SPEECH_ABOVE_NOISE_DB
			{
				// About half a second of speech to settle.
				self.speech_db = Some(match self.speech_db {
					Some(prev) => prev + 0.02 * (level_db - prev),
					None => level_db,
				});
				self.speech_slices += 1;
				if AGC_TARGET_DBFS - level_db > MIC_RAISE_ABOVE_GAIN_DB {
					self.quiet_speech_slices += 1;
				}
			}
			if let Some(speech_db) = self.speech_db {
				let want =
					(AGC_TARGET_DBFS - speech_db).min(AGC_MAX_NOISE_DBFS - self.noise_db).clamp(0.0, AGC_MAX_GAIN_DB);
				let secs = slice.len() as f32 / SAMPLE_RATE;
				self.gain_db += (want - self.gain_db)
					.clamp(-AGC_MAX_GAIN_DROP_DB_PER_S * secs, AGC_MAX_GAIN_CHANGE_DB_PER_S * secs);
			}
		}
		let from = 10f32.powf(old_gain_db / 20.0);
		let to = 10f32.powf(self.gain_db / 20.0);
		let n = slice.len() as f32;
		for (i, sample) in slice.iter_mut().enumerate() {
			let g = from + (to - from) * (i + 1) as f32 / n;
			*sample *= g;
		}
	}
}

// Turns the background down a further GATE_DEPTH_DB between words, so the
// noise left by the suppressor isn't heard on its own once you stop talking.
// Opens within a few ms when the noise suppressor hears speech (its verdict
// is one hop ahead of its output, so word starts aren't cut), stays open
// GATE_HOLD_SLICES after, then fades down slowly.
pub(crate) const GATE_DEPTH_DB: f32 = 12.0;
const GATE_OPEN_SPEECH: f32 = 0.6;
const GATE_HOLD_SLICES: u32 = 25; // 250 ms
const GATE_ATTACK_SECS: f32 = 0.005;
const GATE_RELEASE_SECS: f32 = 0.15;

struct PauseGate {
	gain: f32,
	hold: u32,
	floor: f32,
	attack: f32,
	release: f32,
}

impl PauseGate {
	fn new() -> Self {
		let coeff = |secs: f32| 1.0 - (-1.0 / (secs * SAMPLE_RATE)).exp();
		Self {
			gain: 1.0,
			hold: GATE_HOLD_SLICES,
			floor: 10f32.powf(-GATE_DEPTH_DB / 20.0),
			attack: coeff(GATE_ATTACK_SECS),
			release: coeff(GATE_RELEASE_SECS),
		}
	}

	fn process_slice(&mut self, slice: &mut [f32], speech: f32) {
		if speech >= GATE_OPEN_SPEECH {
			self.hold = GATE_HOLD_SLICES;
		} else {
			self.hold = self.hold.saturating_sub(1);
		}
		let (target, rate) = if self.hold > 0 { (1.0, self.attack) } else { (self.floor, self.release) };
		for sample in slice.iter_mut() {
			self.gain += (target - self.gain) * rate;
			*sample *= self.gain;
		}
	}
}

// The low-pass used on both sides of the noise suppressor: 97 taps at 48 kHz,
// flat to about 6.8 kHz and gone by 9.3 kHz, 1 ms of delay each way.
const RESAMPLE_TAPS: usize = 97;
const RESAMPLE_CUTOFF_HZ: f32 = 8000.0;

fn resample_taps() -> [f32; RESAMPLE_TAPS] {
	use std::f32::consts::PI;
	let fc = RESAMPLE_CUTOFF_HZ / 48_000.0;
	let mid = (RESAMPLE_TAPS / 2) as f32;
	let mut taps = [0.0; RESAMPLE_TAPS];
	for (i, tap) in taps.iter_mut().enumerate() {
		let t = i as f32 - mid;
		let sinc = if t == 0.0 { 2.0 * fc } else { (2.0 * PI * fc * t).sin() / (PI * t) };
		let x = 2.0 * PI * i as f32 / (RESAMPLE_TAPS - 1) as f32;
		*tap = sinc * (0.42 - 0.5 * x.cos() + 0.08 * (2.0 * x).cos());
	}
	let sum: f32 = taps.iter().sum();
	taps.map(|t| t / sum)
}

/// 16 kHz to 48 kHz: three samples out for each one in.
struct Upsampler3 {
	taps: [f32; RESAMPLE_TAPS],
	history: [f32; RESAMPLE_TAPS / 3 + 1],
}

impl Upsampler3 {
	fn new() -> Self {
		Self { taps: resample_taps(), history: [0.0; RESAMPLE_TAPS / 3 + 1] }
	}

	fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
		for &sample in input {
			self.history.copy_within(..RESAMPLE_TAPS / 3, 1);
			self.history[0] = sample;
			for phase in 0..3 {
				let mut acc = 0.0;
				for (j, &h) in self.history.iter().enumerate() {
					if let Some(tap) = self.taps.get(phase + 3 * j) {
						acc += tap * h;
					}
				}
				out.push(3.0 * acc);
			}
		}
	}
}

/// 48 kHz to 16 kHz: one sample out for each three in.
struct Decimator3 {
	taps: [f32; RESAMPLE_TAPS],
	history: Vec<f32>,
}

impl Decimator3 {
	fn new() -> Self {
		Self { taps: resample_taps(), history: vec![0.0; RESAMPLE_TAPS - 3] }
	}

	fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
		self.history.extend_from_slice(input);
		let mut start = 0;
		while start + RESAMPLE_TAPS <= self.history.len() {
			out.push(self.taps.iter().zip(&self.history[start..]).map(|(t, s)| t * s).sum());
			start += 3;
		}
		self.history.drain(..start);
	}
}

// What the computer is playing, handed from the thread that records it (see
// run_echo_reference in calls/audio.rs) to the mic thread, which needs one
// played sample beside every mic sample. The two must stay lined up for the
// whole call, since the echo canceller learns the delay between them, so the
// mic side never skips ahead: if the played sound for some mic samples has
// not arrived yet, those mic samples wait for the next round. Only when
// nothing has arrived for REFERENCE_MAX_WAIT (nothing is playing, or the
// device went away) does the mic go on without, and the two are lined up
// afresh when sound comes back.
const REFERENCE_RING: usize = 16_384;
const REFERENCE_MAX_AHEAD: u64 = 8000;
const REFERENCE_MAX_WAIT: usize = 640; // 40 ms
/// On lining up, reading starts this far behind the newest played sound.
const REFERENCE_START_BEHIND: u64 = 80;

pub(crate) struct EchoReference {
	ring: Vec<f32>,
	written: u64,
	read: u64,
	lined_up: bool,
	stalled_at: u64,
}

impl Default for EchoReference {
	fn default() -> Self {
		Self { ring: vec![0.0; REFERENCE_RING], written: 0, read: 0, lined_up: false, stalled_at: 0 }
	}
}

impl EchoReference {
	pub(crate) fn push(&mut self, samples: &[f32]) {
		for &sample in samples {
			self.ring[(self.written % REFERENCE_RING as u64) as usize] = sample;
			self.written += 1;
		}
	}

	/// Fills `out` with the played sound for the next mic samples and says
	/// how many of the `want` mic samples can be processed now.
	pub(crate) fn take(&mut self, want: usize, out: &mut Vec<f32>) -> usize {
		out.clear();
		if !self.lined_up {
			if self.written == self.stalled_at {
				out.resize(want, 0.0);
				return want;
			}
			self.read = self.written.saturating_sub(REFERENCE_START_BEHIND).max(self.stalled_at);
			self.lined_up = true;
		}
		if self.written - self.read > REFERENCE_MAX_AHEAD {
			// The mic fell far behind (its thread stalled): start over.
			self.read = self.written - REFERENCE_START_BEHIND;
		}
		let ready = ((self.written - self.read) as usize).min(want);
		for i in 0..ready as u64 {
			out.push(self.ring[((self.read + i) % REFERENCE_RING as u64) as usize]);
		}
		self.read += ready as u64;
		if want - ready > REFERENCE_MAX_WAIT {
			self.lined_up = false;
			self.stalled_at = self.written;
			out.resize(want, 0.0);
			return want;
		}
		ready
	}
}

/// Rumble under the voice is removed first; it carries no speech and only
/// gets in the echo canceller's and the auto gain's way.
const HIGH_PASS_HZ: f32 = 80.0;
/// Where the echo left by the canceller is estimated at more than this share
/// of a hop, the auto gain does not take the hop for our voice (or it would
/// turn up to make the far end's leftovers loud); over the second share the
/// hop is taken as all echo and the pause gate stays shut too.
const SOME_ECHO: f32 = 0.1;
const MOSTLY_ECHO: f32 = 1.0;

pub(crate) struct CallVoice {
	high_pass: HighPass,
	filtered: Vec<f32>,
	canceller: EchoCanceller,
	cancelled: Vec<f32>,
	residual: Vec<f32>,
	suppressor: EchoSuppressor,
	hop: [f32; AGC_SLICE],
	/// The echo suppressor's echo share of the last hops, newest last.
	echo: [f32; 3],
	upsampler: Upsampler3,
	wide: Vec<f32>,
	denoiser: SpeechDenoiser,
	denoised: [f32; SpeechDenoiser::FRAME],
	decimator: Decimator3,
	slice: Vec<f32>,
	pub(crate) agc: VoiceAgc,
	gate: PauseGate,
}

impl CallVoice {
	pub(crate) fn new() -> Self {
		Self {
			high_pass: HighPass::new(HIGH_PASS_HZ, SAMPLE_RATE),
			filtered: Vec::new(),
			canceller: EchoCanceller::new(),
			cancelled: Vec::new(),
			residual: Vec::new(),
			suppressor: EchoSuppressor::new(),
			hop: [0.0; AGC_SLICE],
			echo: [0.0; 3],
			upsampler: Upsampler3::new(),
			wide: Vec::new(),
			denoiser: SpeechDenoiser::new(),
			denoised: [0.0; SpeechDenoiser::FRAME],
			decimator: Decimator3::new(),
			slice: Vec::new(),
			agc: VoiceAgc::new(),
			gate: PauseGate::new(),
		}
	}

	/// The delay the echo canceller found between played and recorded sound.
	pub(crate) fn echo_delay_ms(&self) -> Option<u32> {
		self.canceller.delay_ms()
	}

	/// Takes mic samples and, beside them, what the computer was playing
	/// (`far`, the same length). Appends the cleaned voice to `out` in whole
	/// 10 ms slices, about 30 ms behind the mic, and returns how many of
	/// those slices were our own speech. While `muted` the gain and the gate
	/// rest, but the echo canceller keeps learning.
	pub(crate) fn process(&mut self, mic: &[f32], far: &[f32], muted: bool, out: &mut Vec<f32>) -> u32 {
		self.filtered.clear();
		self.filtered.extend(mic.iter().map(|&s| self.high_pass.process(s)));
		self.canceller.process(&self.filtered, far, &mut self.cancelled, &mut self.residual);

		let mut used = 0;
		while self.cancelled.len() - used >= AGC_SLICE {
			let echo = self.suppressor.process_hop(
				&self.cancelled[used..used + AGC_SLICE],
				&self.residual[used..used + AGC_SLICE],
				&mut self.hop,
			);
			used += AGC_SLICE;
			self.echo.rotate_left(1);
			self.echo[2] = echo;
			self.upsampler.process(&self.hop, &mut self.wide);
		}
		self.cancelled.drain(..used);
		self.residual.drain(..used);

		let mut slices = 0;
		let mut used = 0;
		while self.wide.len() - used >= SpeechDenoiser::FRAME {
			let speech =
				self.denoiser.process_frame(&self.wide[used..used + SpeechDenoiser::FRAME], &mut self.denoised);
			used += SpeechDenoiser::FRAME;
			self.slice.clear();
			self.decimator.process(&self.denoised, &mut self.slice);
			if !muted {
				// The noise suppressor's output is a hop behind the echo
				// suppressor's verdicts, so look at the last few.
				let echo = self.echo.iter().copied().fold(0.0, f32::max);
				self.agc.process_slice(&mut self.slice, echo < SOME_ECHO, speech);
				self.gate.process_slice(&mut self.slice, if echo < MOSTLY_ECHO { speech } else { 0.0 });
				slices += 1;
			}
			out.extend_from_slice(&self.slice);
		}
		self.wide.drain(..used);
		slices
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn speechy(amp: f32, frames: usize) -> Vec<Vec<f32>> {
		// 300 ms talk, 200 ms pause, repeated, over a faint noise floor.
		(0..frames)
			.map(|f| {
				(0..960)
					.map(|i| {
						let t = f * 960 + i;
						let talking = (t / 1600) % 5 < 3;
						let noise = 0.0003 * (((t * 7919) % 1000) as f32 / 500.0 - 1.0);
						noise + if talking { amp * 1.414 * ((t as f32) * 0.3).sin() } else { 0.0 }
					})
					.collect()
			})
			.collect()
	}

	fn speech_rms_db(frames: &[Vec<f32>]) -> f32 {
		let loud: Vec<f32> = frames
			.iter()
			.flatten()
			.copied()
			.collect::<Vec<_>>()
			.chunks(160)
			.map(|c| (c.iter().map(|s| s * s).sum::<f32>() / c.len() as f32).sqrt())
			.filter(|r| *r > 0.003)
			.collect();
		let mean = loud.iter().sum::<f32>() / loud.len() as f32;
		20.0 * mean.log10()
	}

	#[test]
	fn agc_brings_quiet_speech_up_to_target() {
		let mut agc = VoiceAgc::new();
		let mut frames = speechy(0.01, 200); // -40 dBFS speech, 12 s
		for f in frames.iter_mut() {
			agc.process(f);
		}
		let level = speech_rms_db(&frames[150..]);
		assert!((level - AGC_TARGET_DBFS).abs() < 3.0, "settled speech level {level} dBFS");
	}

	#[test]
	fn agc_brings_loud_speech_down_to_unity_within_a_second() {
		let mut agc = VoiceAgc::new();
		let mut frames = speechy(0.2, 20); // -14 dBFS speech
		for f in frames.iter_mut() {
			agc.process(f);
		}
		assert!(agc.gain_db < 0.5, "gain {} dB", agc.gain_db);
	}

	#[test]
	fn agc_holds_gain_through_silence() {
		let mut agc = VoiceAgc::new();
		agc.gain_db = 12.0;
		let mut silence = vec![0.0f32; 960];
		agc.process(&mut silence);
		assert_eq!(agc.gain_db, 12.0);
		assert!(silence.iter().all(|s| *s == 0.0));
	}

	#[test]
	fn pause_gate_opens_fast_holds_then_fades_the_background() {
		let mut gate = PauseGate::new();
		let mut slice = vec![1.0f32; AGC_SLICE];
		gate.process_slice(&mut slice, 1.0);
		assert!(slice[80] > 0.99, "open within 5 ms");
		for _ in 0..GATE_HOLD_SLICES - 1 {
			slice.fill(1.0);
			gate.process_slice(&mut slice, 0.0);
			assert!(slice.iter().all(|&s| s > 0.99), "held open after speech");
		}
		for _ in 0..100 {
			slice.fill(1.0);
			gate.process_slice(&mut slice, 0.0);
		}
		let db = 20.0 * slice[AGC_SLICE - 1].log10();
		assert!((db + GATE_DEPTH_DB).abs() < 0.5, "settled at {db} dB");
	}

	fn lcg(seed: &mut u32) -> f32 {
		*seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
		*seed as f32 / u32::MAX as f32 - 0.5
	}

	fn level_db(x: &[f32]) -> f32 {
		10.0 * (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32 + 1e-12).log10()
	}

	/// Something voice-like enough for the echo canceller: noise shaped into
	/// syllables with pauses.
	fn chatter(seed: u32, len: usize, amp: f32) -> Vec<f32> {
		let mut seed = seed;
		let mut low = 0.0f32;
		(0..len)
			.map(|i| {
				let syllable = (i / 2400) % 4 != 3;
				low += 0.35 * (lcg(&mut seed) - low);
				if syllable { amp * 4.0 * low } else { 0.0 }
			})
			.collect()
	}

	#[test]
	fn resampling_up_and_back_keeps_the_voice_band() {
		for hz in [200.0f32, 1000.0, 3000.0, 6000.0] {
			let tone: Vec<f32> =
				(0..16_000).map(|i| 0.5 * (2.0 * std::f32::consts::PI * hz * i as f32 / 16_000.0).sin()).collect();
			let (mut up, mut down) = (Upsampler3::new(), Decimator3::new());
			let (mut wide, mut back) = (Vec::new(), Vec::new());
			for chunk in tone.chunks(160) {
				wide.clear();
				up.process(chunk, &mut wide);
				assert_eq!(wide.len(), 480);
				down.process(&wide, &mut back);
			}
			assert_eq!(back.len(), tone.len());
			let change = level_db(&back[1000..]) - level_db(&tone[1000..]);
			assert!(change.abs() < 0.5, "{hz} Hz changed {change} dB");
		}
	}

	#[test]
	fn echo_of_the_far_end_is_removed_and_our_voice_is_kept() {
		let len = 16_000 * 12;
		let far = chatter(1, len, 0.3);
		// The mic hears the far end 45 ms late through a short room, at -14 dB.
		let mut seed = 9;
		let path: Vec<f32> = (0..400).map(|i| 0.4 * lcg(&mut seed) * (-(i as f32) / 80.0).exp()).collect();
		let echo: Vec<f32> = (0..len)
			.map(|i| (0..path.len()).filter(|j| i >= 720 + j).map(|j| path[j] * far[i - 720 - j]).sum::<f32>())
			.collect();
		let mut canceller = EchoCanceller::new();
		let mut suppressor = EchoSuppressor::new();
		let (mut cancelled, mut residual) = (Vec::new(), Vec::new());
		for (m, f) in echo.chunks(160).zip(far.chunks(160)) {
			canceller.process(m, f, &mut cancelled, &mut residual);
		}
		let delay = canceller.delay_ms().expect("the delay was found");
		assert!((32..=64).contains(&delay), "delay found as {delay} ms");
		let from = 16_000 * 8;
		let to = cancelled.len() / 160 * 160;
		let removed = level_db(&echo[from..to]) - level_db(&cancelled[from..to]);
		assert!(removed > 15.0, "the filter alone removed {removed} dB");
		let mut clean = vec![0.0; to];
		for ((c, r), o) in cancelled[..to].chunks(160).zip(residual.chunks(160)).zip(clean.chunks_mut(160)) {
			suppressor.process_hop(c, r, o);
		}
		let removed = level_db(&echo[from..to]) - level_db(&clean[from..to]);
		assert!(removed > 25.0, "filter and suppressor removed {removed} dB");
	}

	#[test]
	fn with_nothing_playing_the_echo_stages_leave_the_voice_alone() {
		let voice = chatter(4, 16_000 * 3, 0.2);
		let silence = vec![0.0; voice.len()];
		let mut canceller = EchoCanceller::new();
		let mut suppressor = EchoSuppressor::new();
		let (mut cancelled, mut residual) = (Vec::new(), Vec::new());
		canceller.process(&voice, &silence, &mut cancelled, &mut residual);
		assert_eq!(cancelled[..], voice[..cancelled.len()]);
		let mut out = vec![0.0; 160];
		for (i, (c, r)) in cancelled.chunks_exact(160).zip(residual.chunks_exact(160)).enumerate() {
			suppressor.process_hop(c, r, &mut out);
			if i >= 2 {
				let before = &voice[(i - 1) * 160..i * 160];
				assert!(out.iter().zip(before).all(|(a, b)| (a - b).abs() < 1e-3), "hop {i} changed");
			}
		}
	}

	#[test]
	fn the_played_sound_stays_lined_up_with_the_mic() {
		let mut reference = EchoReference::default();
		let mut out = Vec::new();
		assert_eq!(reference.take(160, &mut out), 160, "nothing playing: the mic goes on");
		assert!(out.iter().all(|&s| s == 0.0));

		let ramp: Vec<f32> = (0..2000).map(|i| i as f32).collect();
		reference.push(&ramp[..480]);
		assert_eq!(reference.take(160, &mut out), 80, "starts a little behind the newest sound");
		assert_eq!(out[0], 400.0);
		// The mic is ahead of the played sound: its samples wait.
		assert_eq!(reference.take(80, &mut out), 0);
		reference.push(&ramp[480..640]);
		assert_eq!(reference.take(240, &mut out), 160);
		assert_eq!((out[0], out[159]), (480.0, 639.0), "nothing skipped, nothing repeated");

		// Playback stops: after 40 ms the mic stops waiting.
		assert_eq!(reference.take(REFERENCE_MAX_WAIT, &mut out), 0);
		assert_eq!(reference.take(REFERENCE_MAX_WAIT + 1, &mut out), REFERENCE_MAX_WAIT + 1);
		assert!(out.iter().all(|&s| s == 0.0));
		assert_eq!(reference.take(160, &mut out), 160);
		// And lines up again when it comes back, never replaying old sound.
		reference.push(&ramp[640..660]);
		assert_eq!(reference.take(160, &mut out), 20);
		assert_eq!(out[0], 640.0);
	}

	#[test]
	fn the_chain_gives_back_as_many_samples_as_it_takes() {
		let mut voice = CallVoice::new();
		let mic = chatter(2, 16_000 * 2, 0.1);
		let far = vec![0.0; mic.len()];
		let mut out = Vec::new();
		for (m, f) in mic.chunks(441).zip(far.chunks(441)) {
			voice.process(m, f, false, &mut out);
		}
		assert!(out.len() % AGC_SLICE == 0);
		assert!(mic.len() - out.len() < 16 * 60, "{} samples held back", mic.len() - out.len());
		assert!(out.iter().all(|s| s.is_finite()));
	}
}
