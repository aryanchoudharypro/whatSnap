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
	println!("cargo:rerun-if-changed=assets/icon.ico");
	let commit = embed_commit_info("WHATSNAP");

	let target_dir = profile_dir().or_else(target_profile_dir).unwrap_or_else(|| {
		let root = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
		Path::new(&root).join("target").join(env::var("PROFILE").unwrap_or_default())
	});
	build_mo_catalogs();
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
		icon: Some("assets/icon.ico"),
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

fn build_mo_catalogs() {
	let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
	let patois_entries: &[(&str, &str)] = &[
		("%d hour\0%d hours", "%d jam"),
		("%d minute\0%d minutes", "%d menit"),
		("%d second\0%d seconds", "%d detik"),
		("%s Update", "Pembaruan %s"),
		("&Alt", "&Alt"),
		("&Clear", "&Hapus"),
		("&Clear Shortcut", "&Hapus Pintasan"),
		("&Cmd", "&Cmd"),
		("&Ctrl", "&Ctrl"),
		("&Key:", "&Tombol:"),
		("&No", "&Tidak"),
		("&Option", "&Option"),
		("&Reset to Default", "&Kembalikan ke Bawaan"),
		("&Set Shortcut...", "&Atur Pintasan..."),
		("&Shift", "&Shift"),
		("&Shortcuts:", "&Pintasan:"),
		("&Win", "&Win"),
		("&Yes", "&Ya"),
		("'{}' is already assigned to '{}'. Reassign it to '{}'?", "'{}' sudah dipakai untuk '{}'. Alihkan ke '{}'?"),
		("A new version of %s is available. Here's what's new:", "Versi baru %s sudah tersedia. Berikut pembaruannya:"),
		(
			"A system-wide shortcut needs Ctrl, Alt, or Win, or it would take that key from every other program.",
			"Pintasan global memerlukan Ctrl, Alt, atau Win agar tidak mengambil tombol tersebut dari program lain.",
		),
		("Cancel", "Batal"),
		("Checking for updates...", "Memeriksa pembaruan..."),
		("Configure shortcut for {}:", "Atur pintasan untuk {}:"),
		("Could not reach the update server: %s", "Tidak dapat menghubungi server pembaruan: %s"),
		("Current version is not a valid semver.", "Versi saat ini bukan format semver yang valid."),
		("Customize Keyboard Shortcuts", "Sesuaikan Pintasan Keyboard"),
		("Detected: (none)", "Terdeteksi: (tidak ada)"),
		("Detected: {}", "Terdeteksi: {}"),
		("Downloading update...", "Mengunduh pembaruan..."),
		("Error", "Kesalahan"),
		("Failed to create file", "Gagal membuat file"),
		("Failed to create private download directory", "Gagal membuat folder unduhan"),
		("Failed to determine exe path", "Gagal menentukan lokasi aplikasi"),
		("Failed to get current exe path", "Gagal membaca lokasi aplikasi saat ini"),
		("Failed to get exe directory", "Gagal membaca folder aplikasi"),
		("Failed to launch update script", "Gagal menjalankan skrip pembaruan"),
		("Failed to parse release JSON", "Gagal membaca data rilis"),
		("Failed to read file for verification", "Gagal membaca file untuk verifikasi"),
		("Failed to read signature", "Gagal membaca tanda tangan digital"),
		("Failed to rename verified file", "Gagal mengganti nama file terverifikasi"),
		("Failed to write file", "Gagal menulis file"),
		("Info", "Info"),
		("Invalid public key", "Kunci publik tidak valid"),
		("Invalid signature", "Tanda tangan digital tidak valid"),
		(
			"Latest release does not include downloadable assets.",
			"Rilis terbaru tidak menyertakan file yang dapat diunduh.",
		),
		("Latest release tag is not a valid semver.", "Tag rilis terbaru bukan format semver yang valid."),
		("No key detected", "Tidak ada tombol terdeteksi"),
		("No release notes provided.", "Tidak ada catatan rilis."),
		("No updates available.", "Tidak ada pembaruan."),
		("No updates available. Latest version: %s", "Sudah versi terbaru: %s"),
		("None", "Tidak ada"),
		("OK", "Oke"),
		("Preparing update...", "Menyiapkan pembaruan..."),
		("Reset &All to Defaults", "Kembalikan &Semua ke Bawaan"),
		("Reset Shortcuts", "Kembalikan Pintasan"),
		(
			"Reset all shortcuts in this tab to their default values?",
			"Kembalikan semua pintasan di tab ini ke pengaturan bawaan?",
		),
		("Reset all shortcuts to their default values?", "Kembalikan semua pintasan ke pengaturan bawaan?"),
		("Security Error", "Kesalahan Keamanan"),
		(
			"Security verification failed. The update might have been tampered with: %s",
			"Verifikasi keamanan gagal. File pembaruan mungkin telah diubah: %s",
		),
		("Set Shortcut for {}", "Atur Pintasan untuk {}"),
		("Shortcut Conflict", "Pintasan Bentrok"),
		("Shortcut Not Allowed", "Pintasan Tidak Diizinkan"),
		("Signature verification failed", "Verifikasi tanda tangan digital gagal"),
		("The update server returned HTTP error %s.", "Server pembaruan mengembalikan kesalahan HTTP %s."),
		(
			"Tip: click in the key field and press the key combination you want.",
			"Tips: klik kolom tombol lalu tekan kombinasi tombol yang diinginkan.",
		),
		("Update Ready", "Pembaruan Siap"),
		("Update cancelled.", "Pembaruan dibatalkan."),
		(
			"Update is available but no matching download asset or signature was found.",
			"Pembaruan tersedia, tetapi file unduhan atau tanda tangan yang cocok tidak ditemukan.",
		),
		("Update to %s", "Perbarui ke %s"),
		("{} (not on this keyboard layout)", "{} (tidak ada di tata letak keyboard ini)"),
	];
	let patois_mo = build_mo_bytes(patois_entries);
	let patois_deflated = deflate_stored(&patois_mo);
	std::fs::write(out_dir.join("patois_id.mo.deflate"), patois_deflated).expect("write patois_id.mo.deflate");

	let wxstd_entries: &[(&str, &str)] = &[
		("&Apply", "Ter&apkan"),
		("&Cancel", "&Batal"),
		("&Clear", "&Hapus"),
		("&Close", "&Tutup"),
		("&Copy", "&Salin"),
		("&Delete", "&Hapus"),
		("&Details", "&Detail"),
		("&Help", "&Bantuan"),
		("&No", "&Tidak"),
		("&OK", "&Oke"),
		("&Open", "&Buka"),
		("&Paste", "&Tempel"),
		("&Redo", "&Ulangi"),
		("&Save", "&Simpan"),
		("&Undo", "&Urungkan"),
		("&Yes", "&Ya"),
		("All files (*)|*", "Semua file (*)|*"),
		("All files (*.*)|*.*", "Semua file (*.*)|*.*"),
		("All files (%s)|%s", "Semua file (%s)|%s"),
		("Apply", "Terapkan"),
		("Cancel", "Batal"),
		("Choose a directory", "Pilih folder"),
		("Choose a file", "Pilih file"),
		("Clear", "Hapus"),
		("Close", "Tutup"),
		("Cu&t", "Po&tong"),
		("Elapsed time:", "Waktu berlalu:"),
		("Elapsed time : ", "Waktu berlalu : "),
		("Error", "Kesalahan"),
		("Estimated time:", "Perkiraan waktu:"),
		("Estimated time : ", "Perkiraan waktu : "),
		("Help", "Bantuan"),
		("Hide details", "Sembunyikan detail"),
		("Information", "Informasi"),
		("No", "Tidak"),
		("OK", "Oke"),
		("Open", "Buka"),
		("Open file", "Buka file"),
		("Please wait...", "Mohon tunggu..."),
		("Remaining time:", "Waktu tersisa:"),
		("Remaining time : ", "Waktu tersisa : "),
		("Save", "Simpan"),
		("Save file", "Simpan file"),
		("Searching...", "Mencari..."),
		("Select &All", "Pilih &Semua"),
		("Show details", "Tampilkan detail"),
		("Warning", "Peringatan"),
		("Working...", "Memproses..."),
		("Yes", "Ya"),
	];
	let wxstd_mo = build_mo_bytes(wxstd_entries);
	std::fs::write(out_dir.join("wxstd_id.mo"), wxstd_mo).expect("write wxstd_id.mo");
}

fn build_mo_bytes(entries: &[(&str, &str)]) -> Vec<u8> {
	let header = "Content-Type: text/plain; charset=UTF-8\nContent-Transfer-Encoding: 8bit\nPlural-Forms: nplurals=1; plural=0;\n";
	let mut pairs: Vec<(&str, &str)> = Vec::with_capacity(entries.len() + 1);
	pairs.push(("", header));
	pairs.extend_from_slice(entries);
	pairs.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
	pairs.dedup_by(|a, b| a.0 == b.0);

	let count = pairs.len() as u32;
	let orig_table_offset = 28u32;
	let trans_table_offset = orig_table_offset + count * 8;
	let mut data_offset = trans_table_offset + count * 8;

	let mut orig_descriptors = Vec::with_capacity((count * 8) as usize);
	let mut trans_descriptors = Vec::with_capacity((count * 8) as usize);
	let mut string_data = Vec::new();

	for (orig, _) in &pairs {
		let len = orig.len() as u32;
		orig_descriptors.extend_from_slice(&len.to_le_bytes());
		orig_descriptors.extend_from_slice(&data_offset.to_le_bytes());
		string_data.extend_from_slice(orig.as_bytes());
		string_data.push(0);
		data_offset += len + 1;
	}
	for (_, trans) in &pairs {
		let len = trans.len() as u32;
		trans_descriptors.extend_from_slice(&len.to_le_bytes());
		trans_descriptors.extend_from_slice(&data_offset.to_le_bytes());
		string_data.extend_from_slice(trans.as_bytes());
		string_data.push(0);
		data_offset += len + 1;
	}

	let mut mo = Vec::with_capacity(data_offset as usize);
	mo.extend_from_slice(&0x9504_12DEu32.to_le_bytes());
	mo.extend_from_slice(&0u32.to_le_bytes());
	mo.extend_from_slice(&count.to_le_bytes());
	mo.extend_from_slice(&orig_table_offset.to_le_bytes());
	mo.extend_from_slice(&trans_table_offset.to_le_bytes());
	mo.extend_from_slice(&0u32.to_le_bytes());
	mo.extend_from_slice(&0u32.to_le_bytes());
	mo.extend_from_slice(&orig_descriptors);
	mo.extend_from_slice(&trans_descriptors);
	mo.extend_from_slice(&string_data);
	mo
}

fn deflate_stored(data: &[u8]) -> Vec<u8> {
	if data.is_empty() {
		return vec![0x01, 0x00, 0x00, 0xFF, 0xFF];
	}
	let chunks: Vec<&[u8]> = data.chunks(65535).collect();
	let mut out = Vec::with_capacity(data.len() + chunks.len() * 5);
	for (idx, chunk) in chunks.iter().enumerate() {
		let is_last = idx + 1 == chunks.len();
		let len = chunk.len() as u16;
		let nlen = !len;
		out.push(u8::from(is_last));
		out.extend_from_slice(&len.to_le_bytes());
		out.extend_from_slice(&nlen.to_le_bytes());
		out.extend_from_slice(chunk);
	}
	out
}
