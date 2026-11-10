//! The registered handler Windows calls when a whatSnap notification is
//! clicked, one of its buttons is pressed, or a reply is typed into it.
//!
//! A `whatsnap://` link (what toasts used before) can't carry what was typed,
//! and the copy of whatSnap it starts can't bring the running one to the front
//! reliably. Instead, a COM class (the toast activator) is registered for the
//! current user and named on the Start menu shortcut. When whatSnap is
//! running, Windows calls it in this process; when it isn't, Windows starts
//! whatSnap with [`LAUNCH_FLAG`] and calls it once it's registered. Nothing
//! here needs administrator rights, so the installed app and the portable zip
//! work the same.

use std::sync::atomic::{AtomicBool, Ordering};

use windows::{
	Win32::{
		Foundation::{CLASS_E_NOAGGREGATION, E_POINTER, PROPERTYKEY},
		System::{
			Com::{
				CLSCTX_INPROC_SERVER, CLSCTX_LOCAL_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
				CoRegisterClassObject, CoTaskMemAlloc, IClassFactory, IClassFactory_Impl, IPersistFile,
				REGCLS_MULTIPLEUSE, STGM_READWRITE,
				StructuredStorage::{PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0, PropVariantToStringAlloc},
			},
			Variant::{VT_CLSID, VT_LPWSTR},
		},
		UI::{
			Notifications::{INotificationActivationCallback, INotificationActivationCallback_Impl, NOTIFICATION_USER_INPUT_DATA},
			Shell::{IShellLinkW, PropertiesSystem::IPropertyStore, ShellLink},
		},
	},
	core::{BOOL, GUID, HSTRING, IUnknown, Interface, PCWSTR, PWSTR, Ref, implement},
};

use crate::platform::{app_link, single_instance::LaunchRequest, toast};

/// The activator's class id. Fixed for good: shortcuts and toasts already out
/// there point at it.
const CLSID: GUID = GUID::from_u128(0x6f1c_8f2e_3a5d_4b71_9c0e_5d2a_7b4e_91c3);

/// What Windows puts on the command line when it starts whatSnap for a
/// notification. COM adds `-Embedding` after it.
pub const LAUNCH_FLAG: &str = "-ToastActivated";

const APP_USER_MODEL_ID: PROPERTYKEY =
	PROPERTYKEY { fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3), pid: 5 };
const TOAST_ACTIVATOR_CLSID: PROPERTYKEY =
	PROPERTYKEY { fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3), pid: 26 };

static READY: AtomicBool = AtomicBool::new(false);

/// Whether Windows can reach the activator. Until it can, toasts keep using
/// `whatsnap://` links and have no reply box.
pub fn is_ready() -> bool {
	READY.load(Ordering::SeqCst)
}

/// Whether Windows started this copy for a notification.
pub fn launched_for_toast(args: impl Iterator<Item = String>) -> bool {
	args.into_iter().any(|arg| arg.eq_ignore_ascii_case(LAUNCH_FLAG) || arg.eq_ignore_ascii_case("-Embedding"))
}

/// Registers the activator for this run: the class in the registry, the
/// shortcut that names it, and the class object Windows calls. Runs on a
/// thread of its own that stays for the life of the app (the class object
/// lives in its apartment); call it as early as possible, since a copy
/// started for a notification has to answer before Windows gives up on it.
pub fn start() {
	std::thread::spawn(|| unsafe {
		if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
			eprintln!("[NOTIFY] Toast activator: COM wouldn't start");
			return;
		}
		let factory: IClassFactory = Factory.into();
		if let Err(err) = CoRegisterClassObject(&CLSID, &factory, CLSCTX_LOCAL_SERVER, REGCLS_MULTIPLEUSE) {
			eprintln!("[NOTIFY] Toast activator: couldn't register the class object: {err:?}");
			return;
		}
		let registered = register_class();
		match ensure_shortcut() {
			Ok(()) if registered => READY.store(true, Ordering::SeqCst),
			Ok(()) => eprintln!("[NOTIFY] Toast activator: couldn't write the registry; using links"),
			Err(err) => eprintln!("[NOTIFY] Toast activator: Start menu shortcut failed, using links: {err:?}"),
		}
		loop {
			std::thread::park();
		}
	});
}

/// The class's per-user registration, pointed at this copy of whatSnap.
/// Rewritten every start, so a moved portable copy or an update keeps working.
fn register_class() -> bool {
	let Ok(exe) = std::env::current_exe() else { return false };
	let key = format!("Software\\Classes\\CLSID\\{{{CLSID:?}}}");
	crate::platform::link_handler::set_string(&key, None, "whatSnap notifications")
		&& crate::platform::link_handler::set_string(
			&format!("{key}\\LocalServer32"),
			None,
			&format!("\"{}\" {LAUNCH_FLAG}", exe.display()),
		)
}

/// Makes sure the Start menu shortcut carries whatSnap's app id and names the
/// activator: Windows drops toasts from an app id no shortcut carries, and
/// finds the activator through it. The installer makes the shortcut; the
/// portable copy gets one here. An existing shortcut keeps its target unless
/// that's gone.
fn ensure_shortcut() -> windows::core::Result<()> {
	let path = toast::start_menu_shortcut_path();
	let path_w = HSTRING::from(path.as_str());
	let exe = std::env::current_exe().map_err(|_| windows::core::Error::from(E_POINTER))?;
	unsafe {
		let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
		let file: IPersistFile = link.cast()?;
		let mut changed = false;
		if std::path::Path::new(&path).exists() {
			file.Load(&path_w, STGM_READWRITE)?;
			let mut target = [0u16; 1024];
			let has_target = link.GetPath(&mut target, std::ptr::null_mut(), 0).is_ok() && {
				let len = target.iter().position(|&c| c == 0).unwrap_or(target.len());
				std::path::Path::new(&String::from_utf16_lossy(&target[..len])).exists()
			};
			if !has_target {
				link.SetPath(&HSTRING::from(exe.as_os_str()))?;
				changed = true;
			}
		} else {
			link.SetPath(&HSTRING::from(exe.as_os_str()))?;
			if let Some(dir) = exe.parent() {
				link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))?;
			}
			changed = true;
		}
		let store: IPropertyStore = link.cast()?;
		let clsid_text = format!("{{{CLSID:?}}}");
		if !property_is(&store, &APP_USER_MODEL_ID, toast::APP_ID) {
			store.SetValue(&APP_USER_MODEL_ID, &string_variant(toast::APP_ID)?)?;
			changed = true;
		}
		if !property_is(&store, &TOAST_ACTIVATOR_CLSID, &clsid_text) {
			store.SetValue(&TOAST_ACTIVATOR_CLSID, &clsid_variant()?)?;
			changed = true;
		}
		if changed {
			store.Commit()?;
			file.Save(&path_w, true)?;
		}
	}
	Ok(())
}

unsafe fn property_is(store: &IPropertyStore, key: &PROPERTYKEY, expected: &str) -> bool {
	unsafe {
		let Ok(value) = store.GetValue(key) else { return false };
		let Ok(text) = PropVariantToStringAlloc(&value) else { return false };
		let same = text.to_string().is_ok_and(|t| t.eq_ignore_ascii_case(expected));
		windows::Win32::System::Com::CoTaskMemFree(Some(text.0 as *const _));
		same
	}
}

/// A PROPVARIANT owning a CoTaskMemAlloc'd copy of its data, which its drop
/// (PropVariantClear) frees.
unsafe fn variant(vt: windows::Win32::System::Variant::VARENUM, data: PROPVARIANT_0_0_0) -> PROPVARIANT {
	PROPVARIANT {
		Anonymous: PROPVARIANT_0 {
			Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
				vt,
				wReserved1: 0,
				wReserved2: 0,
				wReserved3: 0,
				Anonymous: data,
			}),
		},
	}
}

unsafe fn string_variant(text: &str) -> windows::core::Result<PROPVARIANT> {
	let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
	unsafe {
		let buffer = CoTaskMemAlloc(wide.len() * 2) as *mut u16;
		if buffer.is_null() {
			return Err(E_POINTER.into());
		}
		std::ptr::copy_nonoverlapping(wide.as_ptr(), buffer, wide.len());
		Ok(variant(VT_LPWSTR, PROPVARIANT_0_0_0 { pwszVal: PWSTR(buffer) }))
	}
}

unsafe fn clsid_variant() -> windows::core::Result<PROPVARIANT> {
	unsafe {
		let buffer = CoTaskMemAlloc(size_of::<GUID>()) as *mut GUID;
		if buffer.is_null() {
			return Err(E_POINTER.into());
		}
		buffer.write(CLSID);
		Ok(variant(VT_CLSID, PROPVARIANT_0_0_0 { puuid: buffer }))
	}
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
	fn CreateInstance(
		&self,
		outer: Ref<IUnknown>,
		riid: *const GUID,
		object: *mut *mut core::ffi::c_void,
	) -> windows::core::Result<()> {
		if object.is_null() || riid.is_null() {
			return Err(E_POINTER.into());
		}
		unsafe { *object = std::ptr::null_mut() };
		if !outer.is_null() {
			return Err(CLASS_E_NOAGGREGATION.into());
		}
		let activator: INotificationActivationCallback = Activator.into();
		unsafe { activator.query(riid, object).ok() }
	}

	fn LockServer(&self, _lock: BOOL) -> windows::core::Result<()> {
		Ok(())
	}
}

#[implement(INotificationActivationCallback)]
struct Activator;

impl INotificationActivationCallback_Impl for Activator_Impl {
	fn Activate(
		&self,
		_app_id: &PCWSTR,
		arguments: &PCWSTR,
		data: *const NOTIFICATION_USER_INPUT_DATA,
		count: u32,
	) -> windows::core::Result<()> {
		let arguments = if arguments.is_null() { String::new() } else { unsafe { arguments.to_string() }.unwrap_or_default() };
		let inputs = if data.is_null() { &[][..] } else { unsafe { std::slice::from_raw_parts(data, count as usize) } };
		let reply = inputs
			.iter()
			.find(|input| unsafe { input.Key.to_string() }.is_ok_and(|key| key == toast::REPLY_INPUT))
			.and_then(|input| unsafe { input.Value.to_string() }.ok());
		if let Some(request) = request_for(&arguments, reply) {
			crate::platform::single_instance::deliver(request);
		}
		Ok(())
	}
}

/// What a press on a toast asks for: `Some(None)` just brings whatSnap to the
/// front. A reply with nothing typed asks for nothing at all.
fn request_for(arguments: &str, reply: Option<String>) -> Option<Option<LaunchRequest>> {
	match app_link::parse(arguments) {
		Some(app_link::AppLink::Reply { jid, account }) => {
			let text = reply.map(|t| t.trim().to_string()).filter(|t| !t.is_empty())?;
			Some(Some(LaunchRequest::Reply { jid, account, text }))
		}
		Some(_) => Some(Some(LaunchRequest::Link(arguments.to_string()))),
		None => Some(None),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_reply_carries_the_typed_text() {
		let link = app_link::reply_link("1@g.us", None);
		assert_eq!(
			request_for(&link, Some("  hi there ".into())),
			Some(Some(LaunchRequest::Reply { jid: "1@g.us".into(), account: None, text: "hi there".into() }))
		);
		assert_eq!(request_for(&link, Some("   ".into())), None);
		assert_eq!(request_for(&link, None), None);
	}

	#[test]
	fn other_presses_are_links_or_just_activate() {
		let open = app_link::open_link("1@g.us", None);
		assert_eq!(request_for(&open, None), Some(Some(LaunchRequest::Link(open.clone()))));
		assert_eq!(request_for(&app_link::activate_link(), None), Some(None));
		assert_eq!(request_for("", None), Some(None));
	}

	#[test]
	fn clsid_formats_with_braces() {
		assert_eq!(format!("{{{CLSID:?}}}"), "{6F1C8F2E-3A5D-4B71-9C0E-5D2A7B4E91C3}");
	}
}
