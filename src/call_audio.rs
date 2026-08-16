use std::{
	net::UdpSocket,
	sync::{
		Arc,
		atomic::{AtomicBool, AtomicU64, Ordering},
		mpsc,
	},
	time::{Duration, Instant},
};

use rodio::cpal::{
	self, BufferSize, StreamConfig,
	traits::{DeviceTrait, HostTrait, StreamTrait},
};

// The user reported inbound audio playing for a moment then cutting off for
// good, while outbound (mic) kept working - but meowcaller's own diagnostics
// showed RTP arriving with zero loss and real decoded PCM energy the whole
// call. That means the break is somewhere between the UDP bridge here and
// actually reaching the speaker, which is invisible: this file's own
// eprintln!() calls go to a console that a windowed GUI app doesn't have, so
// none of them were ever seen. Append-only log to a real file instead, plus
// cheap atomic counters sampled periodically (never file I/O from the
// real-time cpal callbacks themselves) so a repro shows exactly which stage
// stalls: UDP reader dying/stalling, the channel backing up, or the output
// callback finding the buffer starved.
fn log_line(msg: &str) {
	use std::io::Write;
	let path = crate::paths::log_dir().join("call_audio.log");
	if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
		let now = chrono::Local::now().format("%H:%M:%S%.3f");
		let _ = writeln!(f, "{} {}", now, msg);
	}
}

// On Windows, a UDP socket that has sent a datagram to a port with no
// listener can later fail recvfrom() with WSAECONNRESET when the OS
// delivers the resulting ICMP Port Unreachable back to it - a purely local
// artifact that has nothing to do with whether the actual call is still up,
// but was previously killing the audio reader thread outright (see below).
// SIO_UDP_CONNRESET with a FALSE input buffer disables that behavior for
// this socket, matching how the OS itself recommends handling this for
// UDP sockets that expect to keep listening regardless of ICMP replies.
#[cfg(windows)]
fn disable_udp_connreset(socket: &UdpSocket) {
	use std::os::windows::io::AsRawSocket;

	use windows::Win32::Networking::WinSock::{SOCKET, WSAIoctl};
	const SIO_UDP_CONNRESET: u32 = 0x9800000C;
	let sock = SOCKET(socket.as_raw_socket() as usize);
	let input: u32 = 0; // FALSE - disable connection-reset reporting
	let mut bytes_returned: u32 = 0;
	unsafe {
		let _ = WSAIoctl(
			sock,
			SIO_UDP_CONNRESET,
			Some(&input as *const u32 as *const core::ffi::c_void),
			std::mem::size_of::<u32>() as u32,
			None,
			0,
			&mut bytes_returned,
			None,
			None,
		);
	}
}

#[cfg(not(windows))]
fn disable_udp_connreset(_socket: &UdpSocket) {}

pub struct CallAudioState {
	pub input_stream: Option<cpal::Stream>,
	pub output_stream: Option<cpal::Stream>,
	pub is_muted: Arc<AtomicBool>,
	reader_stop: Arc<AtomicBool>,
}

impl Drop for CallAudioState {
	fn drop(&mut self) {
		// Signals the background UDP-reader thread (see start_audio_streams)
		// to exit within its read timeout instead of blocking forever.
		self.reader_stop.store(true, Ordering::Relaxed);
	}
}

pub fn start_audio_streams(target_port: u16) -> anyhow::Result<CallAudioState> {
	let host = cpal::default_host();

	let input_device = host.default_input_device().ok_or_else(|| anyhow::anyhow!("No input device found"))?;
	let output_device = host.default_output_device().ok_or_else(|| anyhow::anyhow!("No output device found"))?;

	let socket = UdpSocket::bind("127.0.0.1:0")?;
	let target_addr = format!("127.0.0.1:{}", target_port);

	// Send a dummy packet to punch the hole for Go to learn our port!
	socket.send_to(&[0u8; 4], &target_addr)?;

	let socket_read = socket.try_clone()?;
	let socket_out = socket.try_clone()?;

	log_line(&format!(
		"start_audio_streams: target_port={} input_device={:?} output_device={:?}",
		target_port,
		input_device.name().unwrap_or_default(),
		output_device.name().unwrap_or_default()
	));

	// Cheap atomics only - sampled by the periodic health-check below, never
	// logged from inside a real-time cpal callback. This is what actually
	// tells us which stage stalls on a repro: the reader thread going quiet,
	// the channel to the output callback backing up, or the output callback
	// itself finding nothing to play.
	let reader_recv_count = Arc::new(AtomicU64::new(0));
	let reader_error_count = Arc::new(AtomicU64::new(0));
	let tx_drop_count = Arc::new(AtomicU64::new(0));
	let output_starved_samples = Arc::new(AtomicU64::new(0));
	let output_played_samples = Arc::new(AtomicU64::new(0));
	let output_src_accum_len = Arc::new(AtomicU64::new(0));

	// The output cpal callback runs on a real-time audio thread that must
	// never block; a blocking UDP recv there previously stalled (or on
	// Windows/WASAPI, could glitch/silence) playback whenever a frame from
	// Go hadn't arrived yet. Instead, a plain background thread blocks on
	// the socket and hands decoded frames to the callback through a small
	// bounded channel; the callback only ever does a non-blocking try_recv.
	let (frame_tx, frame_rx) = mpsc::sync_channel::<Vec<f32>>(8);
	let reader_stop = Arc::new(AtomicBool::new(false));
	let reader_stop_thread = Arc::clone(&reader_stop);
	{
		let reader_recv_count = Arc::clone(&reader_recv_count);
		let reader_error_count = Arc::clone(&reader_error_count);
		let tx_drop_count = Arc::clone(&tx_drop_count);
		std::thread::spawn(move || {
			let _ = socket_read.set_read_timeout(Some(Duration::from_millis(200)));
			disable_udp_connreset(&socket_read);
			let mut buf = vec![0u8; 960 * 4];
			while !reader_stop_thread.load(Ordering::Relaxed) {
				match socket_read.recv_from(&mut buf) {
					Ok((size, _)) => {
						let floats = size / 4;
						let mut frame = Vec::with_capacity(floats);
						for i in 0..floats {
							let bytes = [buf[i * 4], buf[i * 4 + 1], buf[i * 4 + 2], buf[i * 4 + 3]];
							frame.push(f32::from_le_bytes(bytes));
						}
						reader_recv_count.fetch_add(1, Ordering::Relaxed);
						// A full channel means playback is falling behind; drop
						// the frame rather than block this thread's UDP reads.
						if frame_tx.try_send(frame).is_err() {
							tx_drop_count.fetch_add(1, Ordering::Relaxed);
						}
					}
					Err(ref e)
						if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {}
					// Any other recv error (notably WSAECONNRESET on Windows,
					// which a loopback UDP socket can get handed after any
					// datagram it sent found no listener at that instant - pure
					// local plumbing noise, unrelated to whether the call/relay
					// is still up) used to permanently kill this thread, which
					// dropped frame_tx and silenced playback for the rest of the
					// call even though nothing about the connection had changed.
					// Log and keep listening instead; only the explicit stop
					// flag above should end this loop.
					Err(e) => {
						reader_error_count.fetch_add(1, Ordering::Relaxed);
						log_line(&format!("reader recv error (continuing): {}", e));
					}
				}
			}
			log_line("reader thread exiting (reader_stop set)");
		});
	}

	// Periodic health snapshot - runs on its own plain thread so it can do
	// file I/O freely without ever touching a real-time cpal callback.
	// Stops itself the same way the reader thread does.
	{
		let reader_stop = Arc::clone(&reader_stop);
		let reader_recv_count = Arc::clone(&reader_recv_count);
		let reader_error_count = Arc::clone(&reader_error_count);
		let tx_drop_count = Arc::clone(&tx_drop_count);
		let output_starved_samples = Arc::clone(&output_starved_samples);
		let output_played_samples = Arc::clone(&output_played_samples);
		let output_src_accum_len = Arc::clone(&output_src_accum_len);
		std::thread::spawn(move || {
			let mut last = Instant::now();
			while !reader_stop.load(Ordering::Relaxed) {
				std::thread::sleep(Duration::from_millis(200));
				if last.elapsed() < Duration::from_secs(2) {
					continue;
				}
				last = Instant::now();
				log_line(&format!(
					"health: udp_recv={} udp_err={} tx_dropped={} src_accum_len={} played={} starved={}",
					reader_recv_count.load(Ordering::Relaxed),
					reader_error_count.load(Ordering::Relaxed),
					tx_drop_count.load(Ordering::Relaxed),
					output_src_accum_len.load(Ordering::Relaxed),
					output_played_samples.load(Ordering::Relaxed),
					output_starved_samples.load(Ordering::Relaxed),
				));
			}
		});
	}

	let is_muted = Arc::new(AtomicBool::new(false));
	let mute_flag = Arc::clone(&is_muted);

	let mut input_stream = None;

	// Try to get a supported input config
	if let Ok(mut supported_configs) = input_device.supported_input_configs() {
		if let Some(config) = supported_configs.next() {
			let config = config.with_max_sample_rate();
			let sample_rate = config.sample_rate();
			let sample_rate_val = sample_rate as u32;
			let channels = config.channels();

			let stream_config = StreamConfig { channels, sample_rate, buffer_size: BufferSize::Default };

			// cpal delivers input in device-native buffer sizes (BufferSize::Default,
			// typically far smaller than 60ms) at whatever cadence the device driver
			// chooses - not aligned to the 960-sample (60ms) frame meowcaller's engine
			// pulls on. Sending one UDP packet per callback produced mismatched-size
			// packets faster than Go's single per-tick read could drain them, so most
			// audio piled up unread while Go zero-padded each undersized frame it did
			// read - the peer heard mostly silence. Accumulate resampled samples here
			// and only emit full FRAME_SAMPLES packets, so every packet Go receives is
			// a complete, correctly-sized frame at roughly the real cadence.
			const FRAME_SAMPLES: usize = 960;
			let mut sample_accum: Vec<f32> = Vec::with_capacity(FRAME_SAMPLES * 2);

			// device_rate / 16000 is only exact when the device's native rate is a
			// clean multiple of 16000 - it's not for 44100Hz, or for the 8000/16000Hz
			// narrowband rates common on Bluetooth call headsets. Truncating that
			// integer division silently mistimes every sample: too few input frames
			// consumed per output sample plays content faster/higher-pitched, too many
			// plays it slower/deeper - exactly the garbled/"monster" pitch distortion
			// reported. A running fractional phase with linear interpolation resamples
			// correctly for any device rate, and stays continuous across callbacks so
			// there's no periodic glitch at buffer boundaries either.
			let mut mono_accum: Vec<f32> = Vec::new();
			let mut resample_phase: f64 = 0.0;
			let downsample_step: f64 = sample_rate_val as f64 / 16000.0;

			let stream = input_device.build_input_stream(
				&stream_config,
				move |data: &[f32], _: &cpal::InputCallbackInfo| {
					let muted = mute_flag.load(std::sync::atomic::Ordering::Relaxed);

					for frame in data.chunks(channels as usize) {
						let sum: f32 = frame.iter().sum();
						mono_accum.push(sum / (channels as f32));
					}

					while resample_phase.floor() as usize + 1 < mono_accum.len() {
						let idx = resample_phase.floor() as usize;
						let frac = (resample_phase - idx as f64) as f32;
						let sample = mono_accum[idx] + (mono_accum[idx + 1] - mono_accum[idx]) * frac;
						sample_accum.push(if muted { 0.0 } else { sample });
						resample_phase += downsample_step;
					}
					let consumed = (resample_phase.floor() as usize).min(mono_accum.len());
					if consumed > 0 {
						mono_accum.drain(0..consumed);
						resample_phase -= consumed as f64;
					}

					while sample_accum.len() >= FRAME_SAMPLES {
						let remainder = sample_accum.split_off(FRAME_SAMPLES);
						let mut buf = Vec::with_capacity(FRAME_SAMPLES * 4);
						for sample in &sample_accum {
							buf.extend_from_slice(&sample.to_le_bytes());
						}
						let _ = socket_out.send_to(&buf, &target_addr);
						sample_accum = remainder;
					}
				},
				|err| log_line(&format!("Input stream error: {}", err)),
				None,
			);

			if let Ok(s) = stream {
				if s.play().is_ok() {
					input_stream = Some(s);
				}
			}
		}
	}

	let mut output_stream = None;

	if let Ok(mut supported_configs) = output_device.supported_output_configs() {
		if let Some(config) = supported_configs.next() {
			let config = config.with_max_sample_rate();
			let sample_rate = config.sample_rate();
			let sample_rate_val = sample_rate as u32;
			let channels = config.channels();

			let stream_config = StreamConfig { channels, sample_rate, buffer_size: BufferSize::Default };

			// Mirrors the input-side fix, for the same two reasons: cpal's output
			// buffer (BufferSize::Default) is typically much smaller than one
			// decoded 60ms/960-sample frame once expanded to the device's rate, so
			// src_accum carries surplus decoded samples across callbacks instead of
			// discarding them; and device_rate/16000 integer division mistimes
			// playback for any non-clean-multiple device rate (44100Hz, or the
			// 8000/16000Hz narrowband common on Bluetooth call headsets), which is
			// what produced the pitched-down "monster voice". A running fractional
			// phase with linear interpolation resamples correctly for any rate.
			let mut src_accum: std::collections::VecDeque<f32> = std::collections::VecDeque::new();
			let mut resample_phase: f64 = 0.0;
			let upsample_step: f64 = 16000.0 / sample_rate_val as f64;
			let max_buffered_src = 960 * 8;

			let stream = output_device.build_output_stream(
				&stream_config,
				move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
					// Never blocks: pulls whatever the background reader
					// thread has already decoded, or plays silence if
					// nothing is queued yet (call still connecting, or a
					// momentary gap) rather than stalling this real-time
					// callback on a network read.
					//
					// `needed` must be scaled by upsample_step: this callback
					// will only actually consume
					// (data.len()/channels)*upsample_step source samples this
					// round (advancing resample_phase by exactly that much
					// below), not one source sample per output frame. The
					// old `+ data.len()/channels` demanded up to ~3x more
					// than would ever be consumed on a 48kHz device
					// (upsample_step ~= 0.33), forcing every callback to
					// drain far more of the decode queue than it needed.
					let out_frames = data.len() / channels as usize;
					let needed =
						resample_phase.floor() as usize + 1 + (out_frames as f64 * upsample_step).ceil() as usize;
					while src_accum.len() < needed {
						match frame_rx.try_recv() {
							Ok(frame) => src_accum.extend(frame),
							Err(_) => break,
						}
					}
					while src_accum.len() > max_buffered_src {
						src_accum.pop_front();
					}

					let mut starved = 0u64;
					for out_frame in data.chunks_mut(channels as usize) {
						let idx = resample_phase.floor() as usize;
						let sample = if idx + 1 < src_accum.len() {
							let frac = (resample_phase - idx as f64) as f32;
							src_accum[idx] + (src_accum[idx + 1] - src_accum[idx]) * frac
						} else if idx < src_accum.len() {
							src_accum[idx]
						} else {
							// Nothing buffered at this position - play silence
							// and, critically, do NOT advance resample_phase
							// past what's actually in src_accum. Advancing it
							// anyway (the old behavior) let phase run away
							// unboundedly for every callback spent starved,
							// since the drain step below only ever caps how
							// much gets consumed, never rolls phase itself
							// back down. Once that happened, phase pointed
							// far past any real data forever after, so newly
							// arriving audio could never catch up to it -
							// this is why playback would work briefly, then
							// go silent for the rest of the call. Freezing
							// phase at the buffer boundary means it's already
							// aligned with the front of whatever arrives next.
							starved += 1;
							0.0
						};
						for slot in out_frame.iter_mut() {
							*slot = sample;
						}
						if (idx as f64) < src_accum.len() as f64 {
							resample_phase += upsample_step;
						}
					}
					let consumed = (resample_phase.floor() as usize).min(src_accum.len());
					if consumed > 0 {
						src_accum.drain(0..consumed);
						resample_phase -= consumed as f64;
					}
					if src_accum.is_empty() && resample_phase > 0.0 {
						resample_phase = 0.0;
					}

					output_played_samples.fetch_add(out_frames as u64, Ordering::Relaxed);
					output_starved_samples.fetch_add(starved, Ordering::Relaxed);
					output_src_accum_len.store(src_accum.len() as u64, Ordering::Relaxed);
				},
				|err| log_line(&format!("Output stream error: {}", err)),
				None,
			);

			match stream {
				Ok(s) => {
					if s.play().is_ok() {
						output_stream = Some(s);
						log_line("output stream started");
					} else {
						log_line("output stream play() failed");
					}
				}
				Err(e) => log_line(&format!("output build_output_stream failed: {}", e)),
			}
		}
	}

	Ok(CallAudioState { input_stream, output_stream, is_muted, reader_stop })
}
