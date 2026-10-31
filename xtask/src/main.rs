//! Release chores that are easier in Rust than in PowerShell.
//!
//! `cargo run -p xtask -- sign <file>...` writes a minisign signature,
//! `<file>.minisig`, beside each file, with the key in the `MINISIGN_KEY`
//! environment variable and its password in `MINISIGN_PASSWORD`. The update
//! check (src/update_check.rs) installs nothing whose signature doesn't
//! verify against the matching public key.

use std::{env, error::Error, path::Path};

use shipfitter::sign::Minisign;

fn main() -> Result<(), Box<dyn Error>> {
	let mut args = env::args().skip(1);
	match args.next().as_deref() {
		Some("sign") => {
			let files: Vec<String> = args.collect();
			if files.is_empty() {
				return Err("sign needs at least one file".into());
			}
			let Some(minisign) = Minisign::from_env()? else {
				return Err("MINISIGN_KEY isn't set, so nothing was signed".into());
			};
			for file in &files {
				println!("Signed {}", minisign.sign(Path::new(file))?.display());
			}
			Ok(())
		}
		_ => {
			println!("Tasks:");
			println!("	sign <file>...	Sign release files with the key in MINISIGN_KEY");
			Ok(())
		}
	}
}
