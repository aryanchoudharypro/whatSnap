use std::{
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
	},
	thread,
	time::Duration,
};

use tungstenite::{Message, connect};

fn main() {
	let ui_alive = Arc::new(AtomicBool::new(true));
	let ui_alive_clone = ui_alive.clone();

	let mut pairing_received = false;
	let mut qr_received = false;

	println!("Mock client starting and connecting to ws://localhost:9000/ws");

	loop {
		if !ui_alive_clone.load(Ordering::SeqCst) {
			break;
		}

		if let Ok((mut socket, _)) = connect("ws://localhost:9000/ws") {
			println!("Connected to mock server!");
			loop {
				if !ui_alive_clone.load(Ordering::SeqCst) {
					break;
				}

				match socket.read() {
					Ok(Message::Text(text)) => {
						if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
							if let Some(t) = val.get("type").and_then(|v| v.as_str()) {
								match t {
									"pairing_code" => {
										if let Some(code) =
											val.get("data").and_then(|d| d.get("code")).and_then(|c| c.as_str())
										{
											println!("{{'data': {{'qrcode': {{'pairingCode': '{}'}}}}}}", code);
											pairing_received = true;
										}
									}
									"qr" => {
										if let Some(code) =
											val.get("data").and_then(|d| d.get("code")).and_then(|c| c.as_str())
										{
											if let Ok(qr) = qrcode::QrCode::new(code) {
												let image = qr
													.render::<image::Rgba<u8>>()
													.quiet_zone(false)
													.module_dimensions(8, 8)
													.build();
												let width = image.width();
												let height = image.height();
												let data = image.into_raw();
												let mut png_bytes = Vec::new();
												let encoder = image::codecs::png::PngEncoder::new(&mut png_bytes);
												use image::ImageEncoder;
												if encoder
													.write_image(&data, width, height, image::ExtendedColorType::Rgba8)
													.is_ok()
												{
													use base64::{Engine as _, engine::general_purpose};
													let b64 = general_purpose::STANDARD.encode(&png_bytes);
													println!(
														"{{'data': {{'qrcode': {{'base64': 'data:image/png;base64,{}'}}}}}}",
														b64
													);
													qr_received = true;
												}
											}
										}
									}
									_ => {}
								}
							}
						}
						if pairing_received && qr_received {
							println!("Both pairing code and QR code processed. Exiting mock client.");
							ui_alive.store(false, Ordering::SeqCst);
							break;
						}
					}
					Ok(_) => {}
					Err(e) => {
						eprintln!("Socket error: {}", e);
						break;
					}
				}
			}
		} else {
			thread::sleep(Duration::from_millis(500));
		}
	}
}
