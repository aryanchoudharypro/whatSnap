use std::path::{Path, PathBuf};

use windows::{
	ApplicationModel::{
		Activation::{ActivationKind, ShareTargetActivatedEventArgs},
		AppInstance,
		DataTransfer::{DataPackageView, StandardDataFormats},
	},
	Storage::{IStorageItem, NameCollisionOption, StorageFile, StorageFolder, StorageItemTypes, Streams::DataReader},
	Win32::{
		System::{
			Com::{
				CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, IDispatch,
				IServiceProvider,
			},
			Variant::VARIANT,
		},
		UI::{
			Shell::{
				IShellBrowser, IShellDispatch2, IShellFolderViewDual, IShellView, IShellWindows, SID_STopLevelBrowser,
				SVGIO_BACKGROUND, SWC_DESKTOP, SWFO_NEEDDISPATCH, ShellWindows,
			},
			WindowsAndMessaging::SW_SHOWNORMAL,
		},
	},
	core::{BSTR, HSTRING, Interface},
};

use crate::package::{app_dir, log};

enum Shared {
	Files(Vec<PathBuf>),
	Link(String),
}

pub fn handle_share_activation() -> i32 {
	let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
	let share = match AppInstance::GetActivatedEventArgs().and_then(|args| {
		if args.Kind()? == ActivationKind::ShareTarget {
			Ok(Some(args.cast::<ShareTargetActivatedEventArgs>()?))
		} else {
			Ok(None)
		}
	}) {
		Ok(Some(share)) => share,
		Ok(None) => return 0,
		Err(e) => {
			log(&format!("Couldn't read the share activation: {e}"));
			return 1;
		}
	};
	let Ok(operation) = share.ShareOperation() else { return 1 };
	let _ = operation.ReportStarted();
	let result = operation.Data().and_then(|data| collect(&data));
	match result {
		Ok(Some(shared)) => {
			let _ = operation.ReportDataRetrieved();
			launch_whatsnap(&shared);
			let _ = operation.ReportCompleted();
			0
		}
		Ok(None) => {
			let _ = operation.ReportError(&HSTRING::from("whatSnap can't send this."));
			1
		}
		Err(e) => {
			log(&format!("Reading what was shared failed: {e}"));
			let _ = operation.ReportError(&HSTRING::from(format!("whatSnap couldn't read what was shared: {e}")));
			1
		}
	}
}

fn share_folder() -> std::io::Result<PathBuf> {
	let root = std::env::temp_dir().join("whatSnap shared");
	if let Ok(entries) = std::fs::read_dir(&root) {
		for entry in entries.flatten() {
			let old = entry
				.metadata()
				.and_then(|m| m.modified())
				.ok()
				.and_then(|t| t.elapsed().ok())
				.is_some_and(|age| age.as_secs() > 24 * 60 * 60);
			if old {
				let _ = std::fs::remove_dir_all(entry.path());
			}
		}
	}
	let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
	let folder = root.join(format!("{stamp:x}"));
	std::fs::create_dir_all(&folder)?;
	Ok(folder)
}

fn collect(data: &DataPackageView) -> windows::core::Result<Option<Shared>> {
	if data.Contains(&StandardDataFormats::StorageItems()?)? {
		let items = data.GetStorageItemsAsync()?.join()?;
		let folder = share_folder()?;
		let mut files = Vec::new();
		for item in items {
			if item.IsOfType(StorageItemTypes::Folder)? {
				continue;
			}
			match copy_item(&item, &folder) {
				Ok(path) => files.push(path),
				Err(e) => log(&format!("Couldn't copy a shared file: {e}")),
			}
		}
		if !files.is_empty() {
			return Ok(Some(Shared::Files(files)));
		}
	}
	if data.Contains(&StandardDataFormats::Bitmap()?)? {
		let folder = share_folder()?;
		return Ok(Some(Shared::Files(vec![save_bitmap(data, &folder)?])));
	}
	let mut text = String::new();
	if data.Contains(&StandardDataFormats::Text()?)? {
		text = data.GetTextAsync()?.join()?.to_string();
	}
	if data.Contains(&StandardDataFormats::WebLink()?)? {
		let link = data.GetWebLinkAsync()?.join()?.AbsoluteUri()?.to_string();
		if !text.contains(&link) {
			if !text.trim().is_empty() {
				text.push('\n');
			}
			text.push_str(&link);
		}
	}
	if text.trim().is_empty() {
		return Ok(None);
	}
	Ok(Some(Shared::Link(format!("whatsapp://send?text={}", percent_encode(&text)))))
}

fn copy_item(item: &IStorageItem, folder: &Path) -> windows::core::Result<PathBuf> {
	let name = item.Name()?.to_string();
	let source = item.Path()?.to_string();
	let target = folder.join(&name);
	if !source.is_empty() && std::fs::copy(&source, &target).is_ok() {
		return Ok(target);
	}
	let file: StorageFile = item.cast()?;
	let destination = StorageFolder::GetFolderFromPathAsync(&HSTRING::from(folder.as_os_str()))?.join()?;
	let copy =
		file.CopyOverload(&destination, &HSTRING::from(name), NameCollisionOption::GenerateUniqueName)?.join()?;
	Ok(PathBuf::from(copy.Path()?.to_string()))
}

fn save_bitmap(data: &DataPackageView, folder: &Path) -> windows::core::Result<PathBuf> {
	let stream = data.GetBitmapAsync()?.join()?.OpenReadAsync()?.join()?;
	let extension = match stream.ContentType()?.to_string().to_ascii_lowercase().as_str() {
		"image/jpeg" | "image/jpg" => "jpg",
		"image/gif" => "gif",
		"image/bmp" => "bmp",
		_ => "png",
	};
	let size = stream.Size()? as u32;
	let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
	reader.LoadAsync(size)?.join()?;
	let mut bytes = vec![0u8; size as usize];
	reader.ReadBytes(&mut bytes)?;
	let path = folder.join(format!("Shared picture.{extension}"));
	std::fs::write(&path, &bytes)?;
	Ok(path)
}

fn percent_encode(text: &str) -> String {
	let mut out = String::with_capacity(text.len());
	for byte in text.bytes() {
		if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
			out.push(byte as char);
		} else {
			out.push_str(&format!("%{byte:02X}"));
		}
	}
	out
}

fn arguments(shared: &Shared) -> String {
	match shared {
		Shared::Files(files) => files.iter().map(|f| format!("\"{}\"", f.display())).collect::<Vec<_>>().join(" "),
		Shared::Link(link) => format!("\"{link}\""),
	}
}

fn launch_whatsnap(shared: &Shared) {
	let exe = app_dir().join("whatSnap.exe");
	let args = arguments(shared);
	if let Err(e) = launch_through_explorer(&exe, &args) {
		log(&format!("Starting whatSnap through Explorer failed ({e}); starting it directly"));
		let mut command = std::process::Command::new(&exe);
		use std::os::windows::process::CommandExt;
		command.raw_arg(&args);
		if let Err(e) = command.spawn() {
			log(&format!("Starting whatSnap failed: {e}"));
		}
	}
}

fn launch_through_explorer(exe: &Path, args: &str) -> windows::core::Result<()> {
	unsafe {
		let windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER)?;
		let mut hwnd = 0i32;
		let desktop = windows.FindWindowSW(
			&VARIANT::from(0i32),
			&VARIANT::default(),
			SWC_DESKTOP,
			&mut hwnd,
			SWFO_NEEDDISPATCH,
		)?;
		let provider: IServiceProvider = desktop.cast()?;
		let browser: IShellBrowser = provider.QueryService(&SID_STopLevelBrowser)?;
		let view: IShellView = browser.QueryActiveShellView()?;
		let background: IDispatch = view.GetItemObject(SVGIO_BACKGROUND)?;
		let folder_view: IShellFolderViewDual = background.cast()?;
		let shell: IShellDispatch2 = folder_view.Application()?.cast()?;
		let directory = exe.parent().map(|d| d.display().to_string()).unwrap_or_default();
		shell.ShellExecute(
			&BSTR::from(exe.display().to_string()),
			&VARIANT::from(BSTR::from(args)),
			&VARIANT::from(BSTR::from(directory)),
			&VARIANT::from(BSTR::from("open")),
			&VARIANT::from(SW_SHOWNORMAL.0),
		)
	}
}
