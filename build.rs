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
	let target = env::var("TARGET").unwrap_or_default();
	if target.contains("windows") {
		let manifest = new_manifest("WinZappRust")
			.supported_os(Windows7..=Windows10)
			.active_code_page(ActiveCodePage::Utf8)
			.heap_type(HeapType::SegmentHeap)
			.dpi_awareness(DpiAwareness::PerMonitorV2)
			.long_path_aware(Setting::Enabled);
		if let Err(e) = embed_manifest(manifest) {
			println!("cargo:warning=Failed to embed manifest: {e}");
		}
		
		let mut res = WindowsResource::new();
		if let Err(e) = res.compile() {
			println!("cargo:warning=Failed to embed version info: {e}");
		}
	}
}
