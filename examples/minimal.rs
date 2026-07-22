use wxdragon::prelude::*;

fn main() {
	let _ = wxdragon::main(|_| {
		let frame = Frame::builder()
			.with_title("Test")
			.with_size(Size::new(400, 300))
			.build();
		frame.show(true);
	});
}
