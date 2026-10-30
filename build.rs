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
	if target.contains("windows-msvc") {
		// Media Foundation (calls' camera and screen share, the video player)
		// and Winsock (call audio) load only when first used, not at launch:
		// each DLL loaded costs private memory for as long as the app runs,
		// and most sessions never touch them.
		for dll in ["mf.dll", "mfplat.dll", "mfreadwrite.dll", "ws2_32.dll"] {
			println!("cargo:rustc-link-arg-bins=/DELAYLOAD:{dll}");
		}
		println!("cargo:rustc-link-arg-bins=delayimp.lib");
	}
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
