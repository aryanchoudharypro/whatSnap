use std::{
	io::Write,
	path::{Path, PathBuf},
};

use windows::{
	Foundation::Uri,
	Management::Deployment::{AddPackageOptions, PackageManager, RemovalOptions},
	Win32::{
		Foundation::{CloseHandle, WAIT_OBJECT_0},
		System::Threading::{GetExitCodeProcess, WaitForSingleObject},
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
		return Err(SignFailure::Failed("can't find whatSnapShare.exe".into()));
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
			return Err(SignFailure::Failed("Windows didn't start the administrator step".into()));
		}
		let mut code = 1u32;
		let done = WaitForSingleObject(info.hProcess, 30_000) == WAIT_OBJECT_0
			&& GetExitCodeProcess(info.hProcess, &mut code).is_ok();
		let _ = CloseHandle(info.hProcess);
		log(&format!("Administrator step finished with exit code {code:#x}"));
		if done && code == 0 {
			return Ok(());
		}
	}
	let reason = std::fs::read_to_string(sign_error_file())
		.unwrap_or_else(|_| "signing as administrator failed without saying why".to_string());
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
		return Err(
			"This copy of whatSnap doesn't include the Share menu package. Reinstall whatSnap and try again.".into()
		);
	}
	if needs_signing() {
		match run_sign_as_admin() {
			Ok(()) => {}
			Err(SignFailure::NotApproved) => {
				return Err("whatSnap wasn't added to the Share menu, because that needs an administrator to approve it once. You can still send files with Send to in Explorer's menu.".into());
			}
			Err(SignFailure::Failed(reason)) => {
				log(&format!("Signing failed: {reason}"));
				return Err(format!("whatSnap couldn't be added to the Share menu.\n\nSigning failed: {reason}"));
			}
		}
	}
	register().map(|()| "whatSnap is now in the Windows Share menu.").map_err(|e| {
		log(&format!("Registering the package failed: {e}"));
		format!("whatSnap couldn't be added to the Share menu.\n\n{e}")
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
