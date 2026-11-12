use std::{
	env,
	error::Error,
	path::{Path, PathBuf},
	process::Command,
};

use shipfitter::{
	host_arch_suffix,
	package::{Zip, inno_setup},
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
			println!("	release	Build whatSnap for this machine's architecture and package the zip and the installer");
			println!("	sign <file>...	Sign files with the key in MINISIGN_KEY");
			Ok(())
		}
	}
}

fn project_root() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(1).unwrap().to_path_buf()
}

/// What goes before the extension of the packages: nothing for x64, which the update check
/// looks for unsuffixed (see platform/update_check.rs), and `-arm64` for arm64.
fn asset_suffix() -> &'static str {
	if host_arch_suffix() == "arm64" { "-arm64" } else { "" }
}

/// Builds each of `packages` in release mode, or the default members when there are none, one
/// Cargo run apiece so a package doesn't get the native library paths another one asked for.
fn build_release(root: &Path, packages: &[&str]) -> Result<()> {
	let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
	let runs: Vec<Option<&str>> =
		if packages.is_empty() { vec![None] } else { packages.iter().copied().map(Some).collect() };
	for package in runs {
		let mut command = Command::new(&cargo);
		command.current_dir(root).args(["build", "--release"]);
		if let Some(package) = package {
			command.args(["-p", package]);
		}
		if !command.status()?.success() {
			return Err(format!("cargo build failed for {}", package.unwrap_or("the workspace")).into());
		}
	}
	Ok(())
}

fn release() -> Result<()> {
	let root = project_root();
	let minisign = Minisign::from_env()?;
	build_release(&root, &[])?;
	build_release(&root, &["whatsnap-share"])?;
	let target_dir = root.join("target").join("release");
	pack_share_package(&root, &target_dir)?;
	let suffix = asset_suffix();

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

	println!("Packaging the portable zip...");
	let zip_path = target_dir.join(format!("whatSnap{suffix}.zip"));
	let mut zip = Zip::create(&zip_path)?;
	for (name, path) in &files {
		zip.file(path, name)?;
	}
	zip.finish()?;
	println!("Created {}", zip_path.display());
	let mut artifacts = vec![zip_path];

	if inno_setup(&target_dir.join("whatSnap.iss"))? {
		let setup = target_dir.join(format!("whatSnap_setup{suffix}.exe"));
		if !suffix.is_empty() {
			std::fs::rename(target_dir.join("whatSnap_setup.exe"), &setup)?;
		}
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

fn pack_share_package(root: &Path, target_dir: &Path) -> Result<()> {
	let makeappx = makeappx()
		.ok_or("makeappx.exe not found. It comes with the Windows SDK (Visual Studio's C++ tools install it).")?;
	let output = target_dir.join("share").join("whatSnapShare.msix");
	std::fs::create_dir_all(output.parent().unwrap())?;
	let source = root.join("packaging").join("share");
	let source = if host_arch_suffix() == "arm64" { stage_arm64_share_package(&source, target_dir)? } else { source };
	let status =
		Command::new(makeappx).args(["pack", "/o", "/nv", "/d"]).arg(source).arg("/p").arg(&output).status()?;
	if !status.success() {
		return Err("makeappx failed".into());
	}
	Ok(())
}

/// A copy of the share package's files whose manifest says arm64, since the checked in one is x64.
fn stage_arm64_share_package(source: &Path, target_dir: &Path) -> Result<PathBuf> {
	fn copy_dir(from: &Path, to: &Path) -> Result<()> {
		std::fs::create_dir_all(to)?;
		for entry in std::fs::read_dir(from)? {
			let entry = entry?;
			let dest = to.join(entry.file_name());
			if entry.file_type()?.is_dir() {
				copy_dir(&entry.path(), &dest)?;
			} else {
				std::fs::copy(entry.path(), dest)?;
			}
		}
		Ok(())
	}
	let staged = target_dir.join("share-package");
	if staged.exists() {
		std::fs::remove_dir_all(&staged)?;
	}
	copy_dir(source, &staged)?;
	let manifest = staged.join("AppxManifest.xml");
	let text = std::fs::read_to_string(&manifest)?;
	let patched = text.replace("ProcessorArchitecture=\"x64\"", "ProcessorArchitecture=\"arm64\"");
	if patched == text {
		return Err("AppxManifest.xml has no x64 ProcessorArchitecture to change".into());
	}
	std::fs::write(&manifest, patched)?;
	Ok(staged)
}

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
	versions
		.into_iter()
		.rev()
		.flat_map(|dir| [dir.join("x64").join("makeappx.exe"), dir.join("arm64").join("makeappx.exe")])
		.find(|path| path.is_file())
}
