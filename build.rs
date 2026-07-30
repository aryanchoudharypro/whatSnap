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

		let status = std::process::Command::new("go")
			.args(&["build", "-o", out_path.to_str().unwrap()])
			.current_dir("backend")
			.status();

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

		if let Ok(out_dir) = env::var("OUT_DIR") {
			let mut current = Some(std::path::Path::new(&out_dir));
			while let Some(dir) = current {
				let webview_x64 = dir.join(
					"wxwidgets_cmake_build/libs/webview/packages/Microsoft.Web.WebView2.1.0.3485.44/build/native/x64",
				);
				if webview_x64.exists() {
					println!("cargo:rustc-link-search=native={}", webview_x64.display());
					break;
				}
				current = dir.parent();
			}
		}
	}
}
