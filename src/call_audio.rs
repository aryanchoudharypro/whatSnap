use rodio::cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rodio::cpal::{self, StreamConfig, BufferSize};
use std::net::UdpSocket;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub struct CallAudioState {
	pub input_stream: Option<cpal::Stream>,
	pub output_stream: Option<cpal::Stream>,
	pub is_muted: Arc<AtomicBool>,
}

pub fn start_audio_streams(target_port: u16) -> anyhow::Result<CallAudioState> {
	let host = cpal::default_host();

	let input_device = host.default_input_device().ok_or_else(|| anyhow::anyhow!("No input device found"))?;
	let output_device = host.default_output_device().ok_or_else(|| anyhow::anyhow!("No output device found"))?;

	let socket = UdpSocket::bind("127.0.0.1:0")?;
	let target_addr = format!("127.0.0.1:{}", target_port);
	
	// Send a dummy packet to punch the hole for Go to learn our port!
	socket.send_to(&[0u8; 4], &target_addr)?;

	let socket_in = socket.try_clone()?;
	let socket_out = socket.try_clone()?;

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
			
			let stream_config = StreamConfig {
				channels,
				sample_rate,
				buffer_size: BufferSize::Default,
			};
			
			let stream = input_device.build_input_stream(
				&stream_config,
				move |data: &[f32], _: &cpal::InputCallbackInfo| {
					let ratio = sample_rate_val / 16000;
					let actual_ratio = if ratio == 0 { 1 } else { ratio };
					
					let mut buf = Vec::with_capacity((data.len() / (channels as usize * actual_ratio as usize)) * 4);
					
					for i in (0..data.len()).step_by(channels as usize * actual_ratio as usize) {
						let mut sum = 0.0;
						for c in 0..channels as usize {
							if i + c < data.len() {
								sum += data[i + c];
							}
						}
						let sample = sum / (channels as f32);
						buf.extend_from_slice(&sample.to_le_bytes());
					}
					
					if mute_flag.load(std::sync::atomic::Ordering::Relaxed) {
						buf.fill(0);
					}
					
					if !buf.is_empty() {
						let _ = socket_out.send_to(&buf, &target_addr);
					}
				},
				|err| eprintln!("Input stream error: {}", err),
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
			
			let stream_config = StreamConfig {
				channels,
				sample_rate,
				buffer_size: BufferSize::Default,
			};
			
			let stream = output_device.build_output_stream(
				&stream_config,
				move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
					let mut buf = vec![0u8; 960 * 4]; // 60ms at 16kHz
					if let Ok((size, _)) = socket_in.recv_from(&mut buf) {
						let floats = size / 4;
						
						let ratio = sample_rate_val / 16000;
						let actual_ratio = if ratio == 0 { 1 } else { ratio };
						
						let mut out_idx = 0;
						for i in 0..floats {
							let bytes = [buf[i*4], buf[i*4+1], buf[i*4+2], buf[i*4+3]];
							let sample = f32::from_le_bytes(bytes);
							
							for _ in 0..actual_ratio {
								for _ in 0..channels {
									if out_idx < data.len() {
										data[out_idx] = sample;
										out_idx += 1;
									}
								}
							}
						}
						for i in out_idx..data.len() {
							data[i] = 0.0;
						}
					} else {
						for i in 0..data.len() {
							data[i] = 0.0;
						}
					}
				},
				|err| eprintln!("Output stream error: {}", err),
				None,
			);
			
			if let Ok(s) = stream {
				if s.play().is_ok() {
					output_stream = Some(s);
				}
			}
		}
	}

	Ok(CallAudioState {
		input_stream,
		output_stream,
		is_muted,
	})
}
