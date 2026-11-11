use std::{
	io::Write,
	path::{Path, PathBuf},
};

use windows::{
	Foundation::Uri,
	Management::Deployment::{AddPackageOptions, PackageManager, RemovalOptions},
	Win32::{
		Foundation::{CloseHandle, WAIT_OBJECT_0},
		System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject},
		UI::{
			Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
			WindowsAndMessaging::{
				MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_TOPMOST, MessageBoxW, SW_HIDE,
			},
		},
	},
	core::{HSTRING, PCWSTR, w},
};

/// Must match packaging/share/AppxManifest.xml and whatSnapShare.exe.manifest.
pub const PACKAGE_NAME: &str = "whatSnap.Share";
pub const PUBLISHER: &str = "CN=whatSnap Local Share";

#[link(name = "kernel32")]
unsafe extern "system" {
	fn GetUserDefaultUILanguage() -> u16;
}

fn settings_language_override() -> Option<bool> {
	let base = std::env::var_os("LOCALAPPDATA")
		.map(PathBuf::from)
		.or_else(|| std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("AppData").join("Local")))?;
	let text = std::fs::read_to_string(base.join("whatSnap").join("settings.json")).ok()?;
	let bytes = text.as_bytes();
	let needle = "\"language\"";
	let mut pos = 0;
	while let Some(found) = text[pos..].find(needle) {
		let idx = pos + found;
		pos = idx + needle.len();
		if idx > 0 && bytes[idx - 1] == b'\\' {
			continue;
		}
		let rest = text[pos..].trim_start();
		let Some(after_colon) = rest.strip_prefix(':') else { continue };
		let val = after_colon.trim_start();
		if val.starts_with("\"id\"") {
			return Some(true);
		}
		if val.starts_with("\"en\"") {
			return Some(false);
		}
		if val.starts_with("\"system\"") {
			return None;
		}
	}
	None
}

pub(crate) fn is_indonesian() -> bool {
	static IS_ID: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*IS_ID.get_or_init(|| {
		if let Some(choice) = settings_language_override() {
			return choice;
		}
		let lang_id = unsafe { GetUserDefaultUILanguage() };
		(lang_id & 0x03ff) == 0x21
	})
}

pub fn app_dir() -> PathBuf {
	std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default()
}

pub fn unsigned_package() -> PathBuf {
	app_dir().join("share").join("whatSnapShare.msix")
}

pub fn local_dir() -> PathBuf {
	app_dir().join("share").join("local")
}

pub fn signed_package() -> PathBuf {
	local_dir().join("whatSnapShare.msix")
}

pub fn signed_source() -> PathBuf {
	local_dir().join("source.msix")
}

pub fn log(line: &str) {
	let dir = app_dir().join("share");
	let _ = std::fs::create_dir_all(&dir);
	if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("share.log")) {
		let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
		let _ = writeln!(file, "[{secs} pid {}] {line}", std::process::id());
	}
}

fn needs_signing() -> bool {
	let Ok(unsigned) = std::fs::read(unsigned_package()) else { return false };
	if !signed_package().is_file() {
		return true;
	}
	std::fs::read(signed_source()).map_or(true, |source| source != unsigned)
}

fn message(text: &str, warning: bool) {
	let text = HSTRING::from(text);
	let icon = if warning { MB_ICONWARNING } else { MB_ICONINFORMATION };
	unsafe {
		// Started from Settings, possibly after an approval prompt took the
		// focus: bring the box to the front so a screen reader reads it.
		MessageBoxW(None, &text, w!("whatSnap"), MB_OK | icon | MB_SETFOREGROUND | MB_TOPMOST);
	}
}

pub fn sign_error_file() -> PathBuf {
	app_dir().join("share").join("sign-error.txt")
}

enum SignFailure {
	NotApproved,
	Failed(String),
}

fn run_sign_as_admin() -> Result<(), SignFailure> {
	let _ = std::fs::remove_file(sign_error_file());
	let Ok(exe) = std::env::current_exe() else {
		let msg =
			if is_indonesian() { "tidak dapat menemukan whatSnapShare.exe" } else { "can't find whatSnapShare.exe" };
		return Err(SignFailure::Failed(msg.into()));
	};
	let exe = HSTRING::from(exe.as_os_str());
	let mut info = SHELLEXECUTEINFOW {
		cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
		fMask: SEE_MASK_NOCLOSEPROCESS,
		lpVerb: w!("runas"),
		lpFile: PCWSTR(exe.as_ptr()),
		lpParameters: w!("--sign"),
		nShow: SW_HIDE.0,
		..Default::default()
	};
	unsafe {
		if let Err(e) = ShellExecuteExW(&mut info) {
			log(&format!("Starting the administrator step failed: {e}"));
			return Err(SignFailure::NotApproved);
		}
		if info.hProcess.is_invalid() {
			log("Starting the administrator step gave no process to wait for");
			let msg = if is_indonesian() {
				"Windows tidak menjalankan langkah administrator"
			} else {
				"Windows didn't start the administrator step"
			};
			return Err(SignFailure::Failed(msg.into()));
		}
		let mut code = 1u32;
		let done = WaitForSingleObject(info.hProcess, INFINITE) == WAIT_OBJECT_0
			&& GetExitCodeProcess(info.hProcess, &mut code).is_ok();
		let _ = CloseHandle(info.hProcess);
		log(&format!("Administrator step finished with exit code {code:#x}"));
		if done && code == 0 {
			return Ok(());
		}
	}
	let reason = std::fs::read_to_string(sign_error_file()).unwrap_or_else(|_| {
		if is_indonesian() {
			"penandatanganan sebagai administrator gagal tanpa keterangan"
		} else {
			"signing as administrator failed without saying why"
		}
		.to_string()
	});
	Err(SignFailure::Failed(reason))
}

fn file_uri(path: &Path) -> windows::core::Result<Uri> {
	let mut uri = String::from("file:///");
	for byte in path.to_string_lossy().replace('\\', "/").bytes() {
		if byte.is_ascii_alphanumeric() || b"-._~/:".contains(&byte) {
			uri.push(byte as char);
		} else {
			uri.push_str(&format!("%{byte:02X}"));
		}
	}
	Uri::CreateUri(&HSTRING::from(uri))
}

fn register() -> Result<(), String> {
	let manager = PackageManager::new().map_err(|e| e.to_string())?;
	let add = || -> windows::core::Result<()> {
		let options = AddPackageOptions::new()?;
		options.SetExternalLocationUri(&file_uri(&app_dir())?)?;
		let result = manager.AddPackageByUriAsync(&file_uri(&signed_package())?, &options)?.join()?;
		let code = result.ExtendedErrorCode()?;
		if code.is_err() {
			return Err(windows::core::Error::new(code, result.ErrorText()?.to_string()));
		}
		Ok(())
	};
	if let Err(first) = add() {
		log(&format!("Registering the package failed ({first}); removing the old one and trying again"));
		remove_registered(&manager);
		add().map_err(|e| e.to_string())?;
	}
	Ok(())
}

fn remove_registered(manager: &PackageManager) {
	let Ok(packages) = manager.FindPackagesByUserSecurityIdNamePublisher(
		&HSTRING::new(),
		&HSTRING::from(PACKAGE_NAME),
		&HSTRING::from(PUBLISHER),
	) else {
		return;
	};
	for package in packages {
		let Ok(full_name) = package.Id().and_then(|id| id.FullName()) else { continue };
		let removed = manager
			.RemovePackageWithOptionsAsync(&full_name, RemovalOptions::None)
			.and_then(|op| op.join())
			.and_then(|result| result.ExtendedErrorCode());
		if let Err(e) = removed {
			log(&format!("Removing {full_name} failed: {e}"));
		}
	}
}

pub fn enable_result_file() -> PathBuf {
	app_dir().join("share").join("enable-result.txt")
}

fn try_enable() -> Result<&'static str, String> {
	if !unsigned_package().is_file() {
		let msg = if is_indonesian() {
			"Salinan whatSnap ini tidak menyertakan paket menu Bagikan. Instal ulang whatSnap lalu coba lagi."
		} else {
			"This copy of whatSnap doesn't include the Share menu package. Reinstall whatSnap and try again."
		};
		return Err(msg.into());
	}
	if needs_signing() {
		match run_sign_as_admin() {
			Ok(()) => {}
			Err(SignFailure::NotApproved) => {
				let msg = if is_indonesian() {
					"whatSnap tidak ditambahkan ke menu Bagikan karena perlu persetujuan administrator satu kali. Anda masih dapat mengirim file lewat Kirim ke di menu Explorer."
				} else {
					"whatSnap wasn't added to the Share menu, because that needs an administrator to approve it once. You can still send files with Send to in Explorer's menu."
				};
				return Err(msg.into());
			}
			Err(SignFailure::Failed(reason)) => {
				log(&format!("Signing failed: {reason}"));
				let msg = if is_indonesian() {
					format!("whatSnap tidak dapat ditambahkan ke menu Bagikan.\n\nPenandatanganan gagal: {reason}")
				} else {
					format!("whatSnap couldn't be added to the Share menu.\n\nSigning failed: {reason}")
				};
				return Err(msg);
			}
		}
	}
	let ok_msg = if is_indonesian() {
		"whatSnap kini ada di menu Bagikan Windows."
	} else {
		"whatSnap is now in the Windows Share menu."
	};
	register().map(|()| ok_msg).map_err(|e| {
		log(&format!("Registering the package failed: {e}"));
		if is_indonesian() {
			format!("whatSnap tidak dapat ditambahkan ke menu Bagikan.\n\n{e}")
		} else {
			format!("whatSnap couldn't be added to the Share menu.\n\n{e}")
		}
	})
}

pub fn enable(report: bool) -> i32 {
	let result = try_enable();
	let (text, failed) = match &result {
		Ok(text) => (text.to_string(), false),
		Err(text) => (text.clone(), true),
	};
	let _ = std::fs::write(enable_result_file(), &text);
	if report {
		message(&text, failed);
	}
	i32::from(failed)
}

pub fn refresh() -> i32 {
	if !signed_package().is_file() {
		return 0;
	}
	enable(false)
}

pub fn disable() -> i32 {
	if let Ok(manager) = PackageManager::new() {
		remove_registered(&manager);
	}
	let _ = std::fs::remove_dir_all(local_dir());
	0
}
