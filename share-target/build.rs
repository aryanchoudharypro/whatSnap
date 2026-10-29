fn main() {
	println!("cargo:rerun-if-changed=build.rs");
	println!("cargo:rerun-if-changed=whatSnapShare.exe.manifest");
	// The manifest names the package this exe belongs to; without it Windows
	// doesn't give the exe the package's identity when the Share menu starts it.
	let target = std::env::var("TARGET").unwrap_or_default();
	if target.contains("windows-msvc") {
		let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
		println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
		println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{dir}\\whatSnapShare.exe.manifest");
	}
}
