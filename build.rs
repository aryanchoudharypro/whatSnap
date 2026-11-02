use std::{
	env,
	path::{Path, PathBuf},
	process::Command,
};

use shipfitter::{
	build::{configure_file, embed_commit_info, target_profile_dir},
	docs::{Page, convert},
	windows::{VersionInfo, embed_manifest},
};

fn main() {
	println!("cargo:rerun-if-changed=build.rs");
	println!("cargo:rerun-if-changed=backend");
	println!("cargo:rerun-if-changed=doc");
	let commit = embed_commit_info("WHATSNAP");

	let target_dir = profile_dir().or_else(target_profile_dir).unwrap_or_else(|| {
		let root = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
		Path::new(&root).join("target").join(env::var("PROFILE").unwrap_or_default())
	});
	build_backend(&target_dir);
	build_docs(&target_dir);
	let root = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
	if let Err(e) =
		configure_file(Path::new("packaging/whatSnap.iss.in"), &target_dir.join("whatSnap.iss"), &[("ROOT", &root)])
	{
		println!("cargo:warning=Failed to configure the installer script: {e}");
	}

	if let Err(e) = embed_manifest("whatSnap") {
		println!("cargo:warning=Failed to embed manifest: {e}");
	}
	let version = env::var("CARGO_PKG_VERSION").unwrap_or_default();
	let dev_version = format!("{version} ({})", commit.short_hash);
	let version_info = VersionInfo {
		product_name: "whatSnap",
		company: "Aryan Choudhary",
		copyright: "Copyright © 2026 Aryan Choudhary",
		original_filename: "whatSnap.exe",
		product_version: commit.is_dev.then_some(dev_version.as_str()),
		..VersionInfo::default()
	};
	if let Err(e) = version_info.embed() {
		println!("cargo:warning=Failed to embed version info: {e}");
	}
}

/// The profile directory (target/release) found by walking up from OUT_DIR to the folder named
/// after the profile. shipfitter counts a fixed number of levels, which lands in target/release/build
/// with cargo's newer build/<package>/<hash>/out layout, so the Go backend ended up in the wrong place.
fn profile_dir() -> Option<PathBuf> {
	let profile = env::var("PROFILE").ok()?;
	let out_dir = PathBuf::from(env::var("OUT_DIR").ok()?);
	out_dir.ancestors().find(|dir| dir.file_name().is_some_and(|name| name == profile.as_str())).map(Path::to_path_buf)
}

fn build_backend(target_dir: &Path) {
	for var in ["GOFLAGS", "GOTOOLCHAIN", "CGO_ENABLED"] {
		println!("cargo:rerun-if-env-changed={var}");
	}
	let out_path: PathBuf = target_dir.join("backend").join("backend.exe");
	let _ = std::fs::create_dir_all(target_dir.join("backend"));
	let mut args = vec!["build".to_string(), "-o".to_string(), out_path.display().to_string()];
	if env::var("PROFILE").is_ok_and(|p| p == "release") {
		args.push("-ldflags=-s -w".to_string());
		args.push("-trimpath".to_string());
	}
	// Build for the Rust target, not the host, so a cross check (say from Linux) builds the
	// Windows backend instead of failing on its Windows-only imports.
	let mut go = Command::new("go");
	go.args(&args).current_dir("backend");
	if env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows") {
		go.env("GOOS", "windows");
	}
	match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
		Ok("x86_64") => go.env("GOARCH", "amd64"),
		Ok("aarch64") => go.env("GOARCH", "arm64"),
		Ok("x86") => go.env("GOARCH", "386"),
		_ => &mut go,
	};
	// A failed Go build fails the cargo build, so an old backend.exe never ships beside a new app.
	match go.status() {
		Ok(status) if !status.success() => panic!("go build failed with {status}"),
		Ok(_) => {}
		Err(e) => panic!("couldn't run go build (is Go installed and on PATH?): {e}"),
	}
}

fn build_docs(target_dir: &Path) {
	let page = Page { title: "whatSnap User Manual", ..Page::default() };
	if let Err(e) = convert(Path::new("doc/readme.md"), &target_dir.join("readme.html"), &page) {
		println!("cargo:warning=Failed to generate the readme: {e}");
	}
}
