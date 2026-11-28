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

/// libopus 1.6.1 from third_party/opus, with its neural decoder features that
/// calls use (src/audio/libopus.rs): Deep PLC (packet-loss concealment), and
/// OSCE - LACE/NoLACE speech enhancement and the blind bandwidth extension that
/// turns a 16 kHz call into 48 kHz sound. DRED stays off (its data isn't kept).
fn build_libopus() {
	println!("cargo:rerun-if-changed=third_party/opus");
	let out = cmake::Config::new("third_party/opus")
		.define("OPUS_DEEP_PLC", "ON")
		.define("OPUS_DRED", "OFF")
		.define("OPUS_OSCE", "ON")
		// The bandwidth extension: only libopus's autotools build defines it.
		.cflag("-DENABLE_OSCE_BWE")
		.define("OPUS_BUILD_TESTING", "OFF")
		.define("OPUS_BUILD_PROGRAMS", "OFF")
		.define("OPUS_BUILD_SHARED_LIBRARY", "OFF")
		.define("OPUS_INSTALL_PKG_CONFIG_MODULE", "OFF")
		.define("OPUS_INSTALL_CMAKE_CONFIG_MODULE", "OFF")
		// The release C runtime in every profile, linked the same way as the Rust code: static
		// (/MT) with crt-static (.cargo/config.toml), so the exe needs no Visual C++ redistributable.
		// libopus's CMakeLists overwrites CMAKE_MSVC_RUNTIME_LIBRARY from its own
		// OPUS_STATIC_RUNTIME option, so that is the switch that actually counts.
		.define("OPUS_STATIC_RUNTIME", if crt_static() { "ON" } else { "OFF" })
		.define("CMAKE_MSVC_RUNTIME_LIBRARY", if crt_static() { "MultiThreaded" } else { "MultiThreadedDLL" })
		.profile("Release")
		.build();
	println!("cargo:rustc-link-search=native={}", out.join("lib").display());
	println!("cargo:rustc-link-lib=static=opus");
}

/// Whether the Rust code links the C runtime statically (`+crt-static`, set in .cargo/config.toml).
fn crt_static() -> bool {
	env::var("CARGO_CFG_TARGET_FEATURE").is_ok_and(|features| features.split(',').any(|f| f == "crt-static"))
}

fn main() {
	println!("cargo:rerun-if-changed=build.rs");
	println!("cargo:rerun-if-changed=backend");
	println!("cargo:rerun-if-changed=doc");
	println!("cargo:rerun-if-changed=po");
	println!("cargo:rerun-if-changed=assets/icon.ico");
	let commit = embed_commit_info("WHATSNAP");

	let target_dir = profile_dir().or_else(target_profile_dir).unwrap_or_else(|| {
		let root = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
		Path::new(&root).join("target").join(env::var("PROFILE").unwrap_or_default())
	});
	build_libopus();
	build_mo_catalogs();
	build_language_table();
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
	let patois_mo = build_mo_bytes(patois_entries, ONE_FORM);
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
	let wxstd_mo = build_mo_bytes(wxstd_entries, ONE_FORM);
	std::fs::write(out_dir.join("wxstd_id.mo"), wxstd_mo).expect("write wxstd_id.mo");

	let patois_it_entries: &[(&str, &str)] = &[
		("%d hour\0%d hours", "%d ora\0%d ore"),
		("%d minute\0%d minutes", "%d minuto\0%d minuti"),
		("%d second\0%d seconds", "%d secondo\0%d secondi"),
		("%s Update", "Aggiornamento di %s"),
		("&Alt", "&Alt"),
		("&Clear", "Ca&ncella"),
		("&Clear Shortcut", "Ca&ncella scorciatoia"),
		("&Cmd", "&Cmd"),
		("&Ctrl", "&Ctrl"),
		("&Key:", "&Tasto:"),
		("&No", "&No"),
		("&Option", "&Option"),
		("&Reset to Default", "&Ripristina predefinita"),
		("&Set Shortcut...", "&Imposta scorciatoia..."),
		("&Shift", "&Maiusc"),
		("&Shortcuts:", "&Scorciatoie:"),
		("&Win", "&Win"),
		("&Yes", "&Sì"),
		("'{}' is already assigned to '{}'. Reassign it to '{}'?", "'{}' è già assegnato a '{}'. Assegnarlo a '{}'?"),
		(
			"A new version of %s is available. Here's what's new:",
			"È disponibile una nuova versione di %s. Ecco le novità:",
		),
		(
			"A system-wide shortcut needs Ctrl, Alt, or Win, or it would take that key from every other program.",
			"Una scorciatoia valida in tutto il sistema deve usare Ctrl, Alt o Win, altrimenti toglierebbe quel tasto a tutti gli altri programmi.",
		),
		("Cancel", "Annulla"),
		("Checking for updates...", "Ricerca aggiornamenti in corso..."),
		("Configure shortcut for {}:", "Configura la scorciatoia per {}:"),
		("Could not reach the update server: %s", "Impossibile raggiungere il server degli aggiornamenti: %s"),
		("Current version is not a valid semver.", "Il numero della versione attuale non è valido."),
		("Customize Keyboard Shortcuts", "Personalizza scorciatoie da tastiera"),
		("Detected: (none)", "Rilevato: (nessuno)"),
		("Detected: {}", "Rilevato: {}"),
		("Downloading update...", "Download dell'aggiornamento in corso..."),
		("Error", "Errore"),
		("Failed to create file", "Impossibile creare il file"),
		("Failed to create private download directory", "Impossibile creare la cartella per il download"),
		("Failed to determine exe path", "Impossibile trovare il percorso del programma"),
		("Failed to get current exe path", "Impossibile leggere il percorso del programma"),
		("Failed to get exe directory", "Impossibile leggere la cartella del programma"),
		("Failed to launch update script", "Impossibile avviare lo script di aggiornamento"),
		("Failed to parse release JSON", "Impossibile leggere i dati della versione"),
		("Failed to read file for verification", "Impossibile leggere il file da verificare"),
		("Failed to read signature", "Impossibile leggere la firma digitale"),
		("Failed to rename verified file", "Impossibile rinominare il file verificato"),
		("Failed to write file", "Impossibile scrivere il file"),
		("Info", "Informazioni"),
		("Invalid public key", "Chiave pubblica non valida"),
		("Invalid signature", "Firma digitale non valida"),
		("Latest release does not include downloadable assets.", "L'ultima versione non contiene file da scaricare."),
		("Latest release tag is not a valid semver.", "Il numero dell'ultima versione non è valido."),
		("No key detected", "Nessun tasto rilevato"),
		("No release notes provided.", "Nessuna nota di versione."),
		("No updates available.", "Nessun aggiornamento disponibile."),
		("No updates available. Latest version: %s", "Nessun aggiornamento disponibile. Ultima versione: %s"),
		("None", "Nessuna"),
		("OK", "OK"),
		("Preparing update...", "Preparazione dell'aggiornamento..."),
		("Reset &All to Defaults", "Ripristina &tutte le predefinite"),
		("Reset Shortcuts", "Ripristina scorciatoie"),
		(
			"Reset all shortcuts in this tab to their default values?",
			"Ripristinare i valori predefiniti di tutte le scorciatoie di questa scheda?",
		),
		("Reset all shortcuts to their default values?", "Ripristinare i valori predefiniti di tutte le scorciatoie?"),
		("Security Error", "Errore di sicurezza"),
		(
			"Security verification failed. The update might have been tampered with: %s",
			"Verifica di sicurezza non riuscita. L'aggiornamento potrebbe essere stato manomesso: %s",
		),
		("Set Shortcut for {}", "Imposta la scorciatoia per {}"),
		("Shortcut Conflict", "Scorciatoia già in uso"),
		("Shortcut Not Allowed", "Scorciatoia non consentita"),
		("Signature verification failed", "Verifica della firma digitale non riuscita"),
		("The update server returned HTTP error %s.", "Il server degli aggiornamenti ha restituito l'errore HTTP %s."),
		(
			"Tip: click in the key field and press the key combination you want.",
			"Suggerimento: fai clic nel campo del tasto e premi la combinazione che vuoi.",
		),
		("Update Ready", "Aggiornamento pronto"),
		("Update cancelled.", "Aggiornamento annullato."),
		(
			"Update is available but no matching download asset or signature was found.",
			"C'è un aggiornamento, ma non è stato trovato il file da scaricare o la sua firma.",
		),
		("Update to %s", "Aggiorna alla versione %s"),
		("{} (not on this keyboard layout)", "{} (non presente in questo layout di tastiera)"),
	];
	let patois_it_mo = build_mo_bytes(patois_it_entries, SINGULAR_AND_PLURAL);
	std::fs::write(out_dir.join("patois_it.mo.deflate"), deflate_stored(&patois_it_mo))
		.expect("write patois_it.mo.deflate");

	let wxstd_it_entries: &[(&str, &str)] = &[
		("&Apply", "&Applica"),
		("&Cancel", "&Annulla"),
		("&Clear", "&Cancella"),
		("&Close", "&Chiudi"),
		("&Copy", "&Copia"),
		("&Delete", "&Elimina"),
		("&Details", "&Dettagli"),
		("&Help", "&Guida"),
		("&No", "&No"),
		("&OK", "&OK"),
		("&Open", "A&pri"),
		("&Paste", "&Incolla"),
		("&Redo", "&Ripeti"),
		("&Save", "&Salva"),
		("&Undo", "&Annulla"),
		("&Yes", "&Sì"),
		("All files (*)|*", "Tutti i file (*)|*"),
		("All files (*.*)|*.*", "Tutti i file (*.*)|*.*"),
		("All files (%s)|%s", "Tutti i file (%s)|%s"),
		("Apply", "Applica"),
		("Cancel", "Annulla"),
		("Choose a directory", "Scegli una cartella"),
		("Choose a file", "Scegli un file"),
		("Clear", "Cancella"),
		("Close", "Chiudi"),
		("Cu&t", "&Taglia"),
		("Elapsed time:", "Tempo trascorso:"),
		("Elapsed time : ", "Tempo trascorso : "),
		("Error", "Errore"),
		("Estimated time:", "Tempo stimato:"),
		("Estimated time : ", "Tempo stimato : "),
		("Help", "Guida"),
		("Hide details", "Nascondi dettagli"),
		("Information", "Informazioni"),
		("No", "No"),
		("OK", "OK"),
		("Open", "Apri"),
		("Open file", "Apri file"),
		("Please wait...", "Attendere..."),
		("Remaining time:", "Tempo rimanente:"),
		("Remaining time : ", "Tempo rimanente : "),
		("Save", "Salva"),
		("Save file", "Salva file"),
		("Searching...", "Ricerca in corso..."),
		("Select &All", "Seleziona &tutto"),
		("Show details", "Mostra dettagli"),
		("Warning", "Attenzione"),
		("Working...", "Elaborazione in corso..."),
		("Yes", "Sì"),
	];
	let wxstd_it_mo = build_mo_bytes(wxstd_it_entries, SINGULAR_AND_PLURAL);
	std::fs::write(out_dir.join("wxstd_it.mo"), wxstd_it_mo).expect("write wxstd_it.mo");
}

/// Plural rules for the catalogs above: Indonesian has one form, Italian a
/// singular and a plural.
const ONE_FORM: &str = "nplurals=1; plural=0;";
const SINGULAR_AND_PLURAL: &str = "nplurals=2; plural=(n != 1);";

fn build_mo_bytes(entries: &[(&str, &str)], plural_forms: &str) -> Vec<u8> {
	let header = format!(
		"Content-Type: text/plain; charset=UTF-8\nContent-Transfer-Encoding: 8bit\nPlural-Forms: {plural_forms}\n"
	);
	let mut pairs: Vec<(&str, &str)> = Vec::with_capacity(entries.len() + 1);
	pairs.push(("", header.as_str()));
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

/// Lists every `po/<code>.po` so a new language needs no Rust change: the file
/// stem is the language code and its `X-Language-Name` header the menu label.
fn build_language_table() {
	let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string()));
	let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
	let mut entries: Vec<(String, String, PathBuf)> = Vec::new();
	if let Ok(dir) = std::fs::read_dir(root.join("po")) {
		for entry in dir.flatten() {
			let path = entry.path();
			if path.extension().and_then(|e| e.to_str()) != Some("po") {
				continue;
			}
			let Some(code) = path.file_stem().and_then(|s| s.to_str()).map(str::to_string) else { continue };
			let text = std::fs::read_to_string(&path).unwrap_or_default();
			let name = text
				.lines()
				.find_map(|l| {
					l.trim()
						.strip_prefix("\"X-Language-Name:")
						.map(|r| r.trim().trim_end_matches("\\n\"").trim().to_string())
				})
				.unwrap_or_else(|| code.clone());
			entries.push((code, name, path));
		}
	}
	entries.sort();
	let mut out = String::from("pub(crate) static LANGUAGE_SOURCES: &[(&str, &str, &str)] = &[\n");
	for (code, name, path) in &entries {
		out.push_str(&format!("\t({code:?}, {name:?}, include_str!({:?})),\n", path.to_string_lossy()));
	}
	out.push_str("];\n");
	std::fs::write(out_dir.join("languages.rs"), out).expect("write languages.rs");
}
