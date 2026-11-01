fn main() {
	let code = "1234-5678";
	let b64 = "abcd";

	let pairing_output = format!("{{'data': {{'qrcode': {{'pairingCode': '{}'}}}}}}", code);
	let expected_pairing = "{'data': {'qrcode': {'pairingCode': '1234-5678'}}}";
	assert_eq!(pairing_output, expected_pairing);

	let qr_output = format!("{{'data': {{'qrcode': {{'base64': 'data:image/png;base64,{}'}}}}}}", b64);
	let expected_qr = "{'data': {'qrcode': {'base64': 'data:image/png;base64,abcd'}}}";
	assert_eq!(qr_output, expected_qr);

	println!("Pairing output: {}", pairing_output);
	println!("QR output: {}", qr_output);

	let qr_payload = "1@ABCD1234EFGH,IJKL5678MNOP,QRST9012UVWX";
	let qr = qrcode::QrCode::new(qr_payload).expect("Failed to create QrCode");
	let side = (qr.width() + 8) * 5;
	let mut data = vec![255u8; side * side * 4];
	for (i, color) in qr.to_colors().iter().enumerate() {
		if *color == qrcode::Color::Dark {
			let (mx, my) = (i % qr.width() + 4, i / qr.width() + 4);
			for y in my * 5..my * 5 + 5 {
				for x in mx * 5..mx * 5 + 5 {
					data[(y * side + x) * 4..][..3].fill(0);
				}
			}
		}
	}
	let (width, height) = (side as u32, side as u32);
	assert!(width > 0 && height > 0);
	assert_eq!(data.len(), (width * height * 4) as usize);

	let _ = wxdragon::main(move |_| {
		let bitmap =
			wxdragon::bitmap::Bitmap::from_rgba(&data, width, height).expect("Failed to construct Bitmap from RGBA");
		assert!(bitmap.is_ok());
		assert_eq!(bitmap.get_width(), width as i32);
		assert_eq!(bitmap.get_height(), height as i32);
		println!(
			"QR Bitmap verification successful: width={}, height={}, is_ok={}",
			bitmap.get_width(),
			bitmap.get_height(),
			bitmap.is_ok()
		);
		std::process::exit(0);
	});

	println!("Format verification successful!");
}
