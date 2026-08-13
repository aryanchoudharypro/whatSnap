//! Verifies the embedded WAVs decode into playable wxSound objects.
//! Run with: cargo run --release --example sound_embed_check

use wxdragon::sound::{Sound, SoundFlags};

const SOUNDS: [(&str, &[u8]); 4] = [
	("message_current", include_bytes!("../sounds/message_current.wav")),
	("message_background", include_bytes!("../sounds/message_background.wav")),
	("message_sent", include_bytes!("../sounds/message_sent.wav")),
	("voicemsg_send", include_bytes!("../sounds/voicemsg_send.wav")),
];

fn main() {
	let _ = wxdragon::main(|_| {
		let mut failures = 0;

		for (name, bytes) in SOUNDS {
			let sound = Sound::from_data(bytes);
			let ok = sound.is_ok();
			let played = if ok { sound.play(SoundFlags::Sync) } else { false };

			println!("{name:<20} {:>7} bytes  is_ok={ok}  played={played}", bytes.len());
			if !ok || !played {
				failures += 1;
			}
		}

		if failures == 0 {
			println!("\nAll {} embedded sounds OK", SOUNDS.len());
		} else {
			println!("\n{failures} FAILED");
		}

		std::process::exit(i32::from(failures != 0));
	});
}
