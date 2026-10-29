//! Turning the Share menu entry on and off: the signed package's files, and
//! registering it for this user.
//!
//! Layout, next to whatSnap.exe:
//! - `share\whatSnapShare.msix`: the unsigned package, from the installer.
//! - `share\local\whatSnapShare.msix`: the same package signed on this PC.
//! - `share\local\source.msix`: a copy of the unsigned package it was signed
//!   from, so an update that changes the package is noticed and re-signed.
//!
//! The installer never writes `share\local`, so an update leaves the signed
//! copy alone and doesn't need administrator rights again unless the package
//! itself changed.

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
			WindowsAndMessaging::{MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MessageBoxW, SW_HIDE},
		},
	},
	core::{HSTRING, PCWSTR, w},
};

/// Must match packaging/share/AppxManifest.xml and whatSnapShare.exe.manifest.
pub const PACKAGE_NAME: &str = "whatSnap.Share";
pub const PUBLISHER: &str = "CN=whatSnap Local Share";

/// The folder whatSnap.exe and this exe are installed in; the package's
/// external location.
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

/// Appends a line to share.log in whatSnap's logs folder, for when the
/// menu entry doesn't appear and nobody saw why.
pub fn log(line: &str) {
	let Some(base) = std::env::var_os("LOCALAPPDATA") else { return };
	let dir = PathBuf::from(base).join("whatSnap").join("logs");
	let _ = std::fs::create_dir_all(&dir);
	if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("share.log")) {
		let _ = writeln!(file, "{line}");
	}
}

/// Whether the signed copy is missing or was signed from a different package
/// than the one installed now.
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
		MessageBoxW(None, &text, w!("whatSnap"), MB_OK | icon);
	}
}

/// Runs this exe again with `--sign` as administrator (Windows asks first)
/// and waits for it. Returns whether it signed the package.
fn run_sign_as_admin() -> bool {
	let Ok(exe) = std::env::current_exe() else { return false };
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
		// Fails if the prompt is cancelled or no administrator signs in.
		if ShellExecuteExW(&mut info).is_err() || info.hProcess.is_invalid() {
			return false;
		}
		let mut code = 1u32;
		let done = WaitForSingleObject(info.hProcess, INFINITE) == WAIT_OBJECT_0
			&& GetExitCodeProcess(info.hProcess, &mut code).is_ok();
		let _ = CloseHandle(info.hProcess);
		done && code == 0
	}
}

/// A `file:` URI for a local path, which the package manager wants.
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

/// Registers the signed package for this user, with this folder as its
/// external location. A package of the same version that was signed by an
/// earlier certificate is removed first, since Windows won't replace it.
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

/// Removes every registered version of the package for this user.
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

/// `--enable`: signs the package if needed (asking for administrator rights),
/// then registers it. Says how it went unless `report` is false.
pub fn enable(report: bool) -> i32 {
	if !unsigned_package().is_file() {
		if report {
			message(
				"This copy of whatSnap doesn't include the Share menu package. Reinstall whatSnap and try again.",
				true,
			);
		}
		return 1;
	}
	if needs_signing() && !run_sign_as_admin() {
		if report {
			message(
				"whatSnap wasn't added to the Share menu, because that needs an administrator to approve it once. You can still send files with Send to in Explorer's menu.",
				true,
			);
		}
		return 1;
	}
	match register() {
		Ok(()) => {
			if report {
				message("whatSnap is now in the Windows Share menu.", false);
			}
			0
		}
		Err(e) => {
			log(&format!("Registering the package failed: {e}"));
			if report {
				message(&format!("whatSnap couldn't be added to the Share menu.\n\n{e}"), true);
			}
			1
		}
	}
}

/// `--refresh`: after an update, keeps the entry if it was turned on: signs
/// again only if the package changed (which asks for administrator rights),
/// and registers it again so it points at this install.
pub fn refresh() -> i32 {
	if !signed_package().is_file() {
		return 0;
	}
	enable(false)
}

/// `--disable`: removes the entry and the signed copy. The certificate stays
/// trusted, but its private key was deleted when it was made, so it can't
/// sign anything else.
pub fn disable() -> i32 {
	if let Ok(manager) = PackageManager::new() {
		remove_registered(&manager);
	}
	let _ = std::fs::remove_dir_all(local_dir());
	0
}
