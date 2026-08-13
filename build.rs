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
	if target.contains("windows") {
		let manifest = new_manifest("whantv")
			.supported_os(Windows7..=Windows10)
			.active_code_page(ActiveCodePage::Utf8)
			.heap_type(HeapType::SegmentHeap)
			.dpi_awareness(DpiAwareness::PerMonitorV2)
			.long_path_aware(Setting::Enabled);
		if let Err(e) = embed_manifest(manifest) {
			println!("cargo:warning=Failed to embed manifest: {e}");
		}

		let res = WindowsResource::new();
		if let Err(e) = res.compile() {
			println!("cargo:warning=Failed to embed version info: {e}");
		}
	}
}
