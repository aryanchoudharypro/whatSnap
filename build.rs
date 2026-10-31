use std::env;

use embed_manifest::{
	embed_manifest,
	manifest::{
		ActiveCodePage, DpiAwareness, HeapType, Setting,
		SupportedOS::{Windows7, Windows10},
	},
	new_manifest,
};
use winres::WindowsResource;

fn main() {
	println!("cargo:rerun-if-changed=build.rs");
	println!("cargo:rerun-if-changed=backend");
	embed_commit_hash();

	if let Ok(profile) = env::var("PROFILE") {
		let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
		let out_path =
			std::path::Path::new(&manifest_dir).join("target").join(&profile).join("backend").join("backend.exe");

		let _ = std::fs::create_dir_all(out_path.parent().unwrap());

		let mut args = vec!["build".to_string(), "-o".to_string(), out_path.to_str().unwrap().to_string()];
		if profile == "release" {
			args.push("-ldflags=-s -w".to_string());
			args.push("-trimpath".to_string());
		}

		let status = std::process::Command::new("go").args(&args).current_dir("backend").status();

		if let Ok(s) = status {
			if !s.success() {
				println!("cargo:warning=Go build failed with status {}", s);
			}
		} else {
			println!("cargo:warning=Failed to execute go build");
		}
	}
	let target = env::var("TARGET").unwrap_or_default();
	if target.contains("windows") {
		let manifest = new_manifest("whatSnap")
			.supported_os(Windows7..=Windows10)
			.active_code_page(ActiveCodePage::Utf8)
			.heap_type(HeapType::SegmentHeap)
			.dpi_awareness(DpiAwareness::PerMonitorV2)
			.long_path_aware(Setting::Enabled);
		if let Err(e) = embed_manifest(manifest) {
			println!("cargo:warning=Failed to embed manifest: {e}");
		}

		// winres would otherwise name the product after the crate, "whatsnap",
		// which is what screen readers say when asked for the version.
		let mut res = WindowsResource::new();
		res.set("ProductName", "whatSnap").set("FileDescription", "whatSnap");
		if let Err(e) = res.compile() {
			println!("cargo:warning=Failed to embed version info: {e}");
		}
	}
}

/// The commit this build is from, as `WHATSNAP_COMMIT_HASH`: the update check
/// compares it with the Development Build's commits. "unknown" outside a git
/// checkout.
fn embed_commit_hash() {
	println!("cargo:rerun-if-changed=.git/HEAD");
	println!("cargo:rerun-if-changed=.git/refs/heads");
	println!("cargo:rerun-if-changed=.git/packed-refs");
	let hash = std::process::Command::new("git")
		.args(["rev-parse", "HEAD"])
		.output()
		.ok()
		.filter(|out| out.status.success())
		.and_then(|out| String::from_utf8(out.stdout).ok())
		.map(|hash| hash.trim().to_string())
		.filter(|hash| !hash.is_empty())
		.unwrap_or_else(|| "unknown".to_string());
	println!("cargo:rustc-env=WHATSNAP_COMMIT_HASH={hash}");
}
