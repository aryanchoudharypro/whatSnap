//! Notification-area tray icon: install/refresh, and the unread-count
//! tooltip shown on it.

use crate::{AppState, ui::window::WindowParts};

/// Repaints the tray icon's tooltip with the current unread total, so the count
/// can be read straight off the notification area without restoring the window.
///
/// Called from every place that touches `unread_counts` rather than from one
/// funnel, because there isn't one: the backend's chats broadcast covers most of
/// it, but marking a chat read/unread, deleting a chat and leaving a group all
/// adjust the map locally with no resync behind them.
///
/// Gated on exactly the condition `ui::window` installs the icon under - a
/// refresh must never resurrect an icon switched off in Settings or pulled on
/// logout, and `set_icon` on its own would do precisely that.
/// Adds the tray icon to the notification area, then immediately writes the
/// current unread count over it.
///
/// The two calls are deliberate and not redundant: the shell takes the item's
/// accessible name from the tooltip it was *added* with and sends every later
/// change to the description, so adding the icon under [`whantv::TRAY_APP_NAME`]
/// and only then writing counts is what makes a screen reader say
/// "whaNTV, 7 unread messages" instead of pairing the live count with whatever
/// it happened to be at login.
pub(crate) fn install_tray_icon(s: &AppState, parts: &WindowParts) {
	if !s.settings.show_tray_icon || !s.main_ui_shown {
		return;
	}
	parts.tray_icon.set_icon(&crate::ui::theme::build_app_icon(32), whantv::TRAY_APP_NAME);
	refresh_tray_tooltip(s, parts);
	enable_keyboard_select();
}

thread_local! {
	static ENTER_RELEASE_HANDLED: std::rc::Rc<std::cell::Cell<bool>> =
		std::rc::Rc::new(std::cell::Cell::new(false));
}

/// Whether an Enter release still to come has already been dealt with, so the
/// chat list mustn't open a chat on it. One flag for the whole UI thread: the
/// chat list reads it, and menus and every way of restoring the window arm it.
pub(crate) fn enter_release_guard() -> std::rc::Rc<std::cell::Cell<bool>> {
	ENTER_RELEASE_HANDLED.with(|guard| guard.clone())
}

/// Brings the main window back: from the tray (the icon or the menu's Open
/// item) or when launching whaNTV again, say from the Start menu. When Enter is
/// what did it, its release lands on whatever regains focus - the chat list,
/// which opens a chat on Enter's release - so the Enter guard is armed to
/// swallow it.
pub(crate) fn restore_main_window(frame: &wxdragon::prelude::Frame) {
	use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_RETURN};
	use wxdragon::prelude::*;
	if unsafe { GetAsyncKeyState(VK_RETURN.0 as i32) } as u16 & 0x8000 != 0 {
		enter_release_guard().set(true);
	}
	frame.show(true);
	frame.iconize(false);
	frame.raise();
}

pub(crate) fn refresh_tray_tooltip(s: &AppState, parts: &WindowParts) {
	if !s.settings.show_tray_icon || !s.main_ui_shown {
		return;
	}
	let unread = whantv::unread_notification_total(
		&s.unread_counts,
		&s.muted_jids,
		&s.archived_jids,
		&s.settings.archived_alert_levels,
	);
	parts.tray_icon.set_icon(&crate::ui::theme::build_app_icon(32), &whantv::tray_tooltip_text(unread));
}

/// Makes the tray icon answer the keyboard and clicks.
///
/// The icon is switched to version 3, where selecting it from the keyboard
/// (Space or Enter) sends NIN_KEYSELECT and the context-menu key sends
/// WM_CONTEXTMENU; wxWidgets understands neither. wx's hidden tray window is
/// subclassed so that a click or a keyboard select calls [`set_on_select`]'s
/// handler, and the context-menu key becomes the right-click wx opens its menu
/// on.
///
/// Clicks are handled here rather than through wxdragon's `on_left_up` and
/// `on_left_double_click`: since wxDragon 0587af2 (AllenDang/wxDragon#224) the
/// tray event types are only mapped on macOS (the mapper in taskbar.cpp sits
/// inside `#ifdef __WXOSX__`), so on Windows those handlers are never bound.
///
/// wx adds its icon with id 99 to a hidden frame of its own, which it doesn't
/// expose, so that frame is found as the window on this thread that owns icon
/// 99. Re-run after every (re)install: a removed and re-added icon starts over
/// at version 0.
pub(crate) fn enable_keyboard_select() {
	use windows::{
		Win32::{
			Foundation::{HWND, LPARAM},
			System::Threading::GetCurrentThreadId,
			UI::{
				Shell::{NIM_SETVERSION, NOTIFYICONDATAW, NOTIFYICONDATAW_0, SetWindowSubclass, Shell_NotifyIconW},
				WindowsAndMessaging::EnumThreadWindows,
			},
		},
		core::BOOL,
	};

	unsafe extern "system" fn collect(hwnd: HWND, windows: LPARAM) -> BOOL {
		unsafe { (*(windows.0 as *mut Vec<HWND>)).push(hwnd) };
		BOOL(1)
	}

	let mut windows: Vec<HWND> = Vec::new();
	unsafe {
		let _ = EnumThreadWindows(GetCurrentThreadId(), Some(collect), LPARAM(&mut windows as *mut _ as isize));
	}
	for hwnd in windows {
		let data = NOTIFYICONDATAW {
			cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
			hWnd: hwnd,
			uID: WX_TRAY_ICON_ID,
			Anonymous: NOTIFYICONDATAW_0 { uVersion: 3 },
			..Default::default()
		};
		// Only the window that owns the icon can set its version.
		if unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data) }.as_bool() {
			unsafe {
				let _ = SetWindowSubclass(hwnd, Some(tray_subclass_proc), TRAY_SUBCLASS_ID, 0);
			}
			return;
		}
	}
}

thread_local! {
	static ON_SELECT: std::cell::RefCell<Option<Box<dyn Fn()>>> = std::cell::RefCell::new(None);
}

/// What clicking the tray icon, or selecting it from the keyboard, does.
pub(crate) fn set_on_select(f: impl Fn() + 'static) {
	ON_SELECT.with(|cell| *cell.borrow_mut() = Some(Box::new(f)));
}

/// The icon id wxWidgets' MSW wxTaskBarIcon always uses.
const WX_TRAY_ICON_ID: u32 = 99;
const TRAY_SUBCLASS_ID: usize = 0x7472_6179;
const NIN_KEYSELECT: isize = 0x0401;

unsafe extern "system" fn tray_subclass_proc(
	hwnd: windows::Win32::Foundation::HWND,
	msg: u32,
	wparam: windows::Win32::Foundation::WPARAM,
	lparam: windows::Win32::Foundation::LPARAM,
	_id: usize,
	_data: usize,
) -> windows::Win32::Foundation::LRESULT {
	use windows::Win32::{
		Foundation::LPARAM,
		UI::{
			Shell::DefSubclassProc,
			WindowsAndMessaging::{RegisterWindowMessageW, WM_CONTEXTMENU, WM_LBUTTONUP, WM_RBUTTONUP},
		},
	};
	thread_local! {
		static WX_TRAY_MSG: u32 = unsafe { RegisterWindowMessageW(windows::core::w!("wxTaskBarIconMessage")) };
	}
	if msg == WX_TRAY_MSG.with(|m| *m) {
		let event = lparam.0 & 0xFFFF;
		if event == NIN_KEYSELECT || event == WM_LBUTTONUP as isize {
			// Taken out while it runs: restoring the window can pump messages
			// that land back here.
			if let Some(f) = ON_SELECT.with(|cell| cell.borrow_mut().take()) {
				f();
				ON_SELECT.with(|cell| {
					cell.borrow_mut().get_or_insert(f);
				});
			}
			return windows::Win32::Foundation::LRESULT(0);
		}
		if event == WM_CONTEXTMENU as isize {
			return unsafe { DefSubclassProc(hwnd, msg, wparam, LPARAM(WM_RBUTTONUP as isize)) };
		}
	}
	unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}
