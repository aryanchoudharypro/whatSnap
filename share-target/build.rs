fn main() {
	println!("cargo:rerun-if-changed=build.rs");
	println!("cargo:rerun-if-changed=whatSnapShare.exe.manifest");
	let target = std::env::var("TARGET").unwrap_or_default();
	if target.contains("windows-msvc") {
		let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
		println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
		println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{dir}\\whatSnapShare.exe.manifest");
	}
}
