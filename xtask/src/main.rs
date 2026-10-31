//! Release builds, run with `cargo release` (an alias in .cargo/config.toml).
//!
//! `cargo release` builds whatSnap (whose build script also builds the Go
//! backend, readme.html and the installer script), the Share menu helper and
//! its package, then makes `target\release\whatSnap.zip` and
//! `target\release\whatSnap_setup.exe`. Inno Setup is used from PATH or its
//! usual install folder, and is otherwise downloaded once into a per-user
//! cache (shipfitter's `tools::iscc`).
//!
//! With `MINISIGN_KEY` (and `MINISIGN_PASSWORD`) set, each file gets a
//! `.minisig` beside it. The update check (src/update_check.rs) installs
//! nothing whose signature doesn't verify. `cargo run -p xtask -- sign
//! <file>...` signs other files the same way.

use std::{
	env,
	error::Error,
	path::{Path, PathBuf},
	process::Command,
};

use shipfitter::{
	package::{Zip, cargo_build_release, inno_setup},
	sign::Minisign,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn main() -> Result<()> {
	let mut args = env::args().skip(1);
	match args.next().as_deref() {
		Some("release") => release(),
		Some("sign") => sign(&args.map(PathBuf::from).collect::<Vec<_>>()),
		_ => {
			println!("Tasks:");
			println!("	release	Build whatSnap and package the zip and the installer");
			println!("	sign <file>...	Sign files with the key in MINISIGN_KEY");
			Ok(())
		}
	}
}

fn project_root() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(1).unwrap().to_path_buf()
}

fn release() -> Result<()> {
	let root = project_root();
	// Load the signing key before building anything, so a broken secret fails fast.
	let minisign = Minisign::from_env()?;
	cargo_build_release(&root, &[])?;
	cargo_build_release(&root, &["whatsnap-share"])?;
	let target_dir = root.join("target").join("release");
	pack_share_package(&root, &target_dir)?;

	let files: [(&str, PathBuf); 8] = [
		("whatSnap.exe", target_dir.join("whatSnap.exe")),
		("whatSnapShare.exe", target_dir.join("whatSnapShare.exe")),
		("share/whatSnapShare.msix", target_dir.join("share").join("whatSnapShare.msix")),
		("backend/backend.exe", target_dir.join("backend").join("backend.exe")),
		("backend/LICENSE", root.join("backend").join("LICENSE")),
		("LICENSE", root.join("LICENSE")),
		("THIRD-PARTY-NOTICES.md", root.join("THIRD-PARTY-NOTICES.md")),
		("readme.html", target_dir.join("readme.html")),
	];
	if let Some((_, missing)) = files.iter().find(|(_, path)| !path.is_file()) {
		return Err(format!("expected build output missing: {}", missing.display()).into());
	}

	// The portable layout is the installed one: whatSnap.exe finds the
	// backend in the backend folder next to it, so an unpacked folder works
	// with no install step.
	println!("Packaging the portable zip...");
	let zip_path = target_dir.join("whatSnap.zip");
	let mut zip = Zip::create(&zip_path)?;
	for (name, path) in &files {
		zip.file(path, name)?;
	}
	zip.finish()?;
	println!("Created {}", zip_path.display());
	let mut artifacts = vec![zip_path];

	if inno_setup(&target_dir.join("whatSnap.iss"))? {
		let setup = target_dir.join("whatSnap_setup.exe");
		println!("Created {}", setup.display());
		artifacts.push(setup);
	}

	match minisign {
		Some(minisign) => {
			for artifact in &artifacts {
				println!("Signed {}", minisign.sign(artifact)?.display());
			}
		}
		None => println!("MINISIGN_KEY isn't set, so the files aren't signed and the update check won't install them."),
	}
	Ok(())
}

fn sign(files: &[PathBuf]) -> Result<()> {
	if files.is_empty() {
		return Err("sign needs at least one file".into());
	}
	let Some(minisign) = Minisign::from_env()? else {
		return Err("MINISIGN_KEY isn't set, so nothing was signed".into());
	};
	for file in files {
		println!("Signed {}", minisign.sign(file)?.display());
	}
	Ok(())
}

/// Packs packaging\share into the Share menu's sparse package, left
/// unsigned: each PC that turns the menu on signs it with a certificate of
/// its own (see share-target/src/sign.rs).
fn pack_share_package(root: &Path, target_dir: &Path) -> Result<()> {
	let makeappx = makeappx()
		.ok_or("makeappx.exe not found. It comes with the Windows SDK (Visual Studio's C++ tools install it).")?;
	let output = target_dir.join("share").join("whatSnapShare.msix");
	std::fs::create_dir_all(output.parent().unwrap())?;
	let status = Command::new(makeappx)
		.args(["pack", "/o", "/nv", "/d"])
		.arg(root.join("packaging").join("share"))
		.arg("/p")
		.arg(&output)
		.status()?;
	if !status.success() {
		return Err("makeappx failed".into());
	}
	Ok(())
}

/// makeappx.exe from PATH, or else the newest x64 copy in the Windows 10/11 SDK.
fn makeappx() -> Option<PathBuf> {
	let on_path = env::var_os("PATH")
		.into_iter()
		.flat_map(|path| env::split_paths(&path).collect::<Vec<_>>())
		.map(|dir| dir.join("makeappx.exe"))
		.find(|path| path.is_file());
	if on_path.is_some() {
		return on_path;
	}
	let bin = PathBuf::from(env::var_os("ProgramFiles(x86)")?).join("Windows Kits").join("10").join("bin");
	let mut versions: Vec<PathBuf> = std::fs::read_dir(bin).ok()?.filter_map(|e| e.ok().map(|e| e.path())).collect();
	versions.sort();
	versions.into_iter().rev().map(|dir| dir.join("x64").join("makeappx.exe")).find(|path| path.is_file())
}
