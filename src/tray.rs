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
/// change to the description, so adding the icon under [`whatsnap::TRAY_APP_NAME`]
/// and only then writing counts is what makes a screen reader say
/// "whatSnap, 7 unread messages" instead of pairing the live count with whatever
/// it happened to be at login.
pub(crate) fn install_tray_icon(s: &AppState, parts: &WindowParts) {
	if !s.settings.show_tray_icon || !s.main_ui_shown {
		return;
	}
	parts.tray_icon.set_icon(&crate::ui::theme::build_app_icon(32), whatsnap::TRAY_APP_NAME);
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
/// item) or when launching whatSnap again, say from the Start menu. When Enter is
/// what did it, its release lands on whatever regains focus - the chat list,
/// which opens a chat on Enter's release - so the Enter guard is armed to
/// swallow it.
pub(crate) fn restore_main_window(frame: &wxdragon::prelude::Frame) {
	use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_RETURN};
	use wxdragon::prelude::*;
	if unsafe { GetAsyncKeyState(VK_RETURN.0 as i32) } as u16 & 0x8000 != 0 {
		enter_release_guard().set(true);
	}
	HIDDEN_TO_TRAY.set(false);
	frame.show(true);
	frame.iconize(false);
	frame.raise();
}

thread_local! {
	static HIDDEN_TO_TRAY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether the main window is put away in the tray, as opposed to open or
/// merely minimized.
pub(crate) fn is_hidden_to_tray() -> bool {
	HIDDEN_TO_TRAY.get()
}

/// Puts the main window away in the tray and makes sure it stays there until
/// [`restore_main_window`].
///
/// Hiding alone isn't enough. With nothing else open, Windows has no next app
/// to hand activation to, so the hidden window stays the foreground one - and
/// the next program to close (a clipboard manager dismissed with Alt+F4, say)
/// hands focus "back" to it, restoring it with a ShowWindow of its own. wx
/// never hears about that and goes on believing the window is hidden, so the
/// Alt+F4 that should put it away again does nothing at all. So the
/// foreground is given up to the desktop here, and while hidden the window
/// refuses every attempt to show it that didn't come from us.
pub(crate) fn hide_main_window(frame: &wxdragon::prelude::Frame) {
	use windows::Win32::{
		Foundation::HWND,
		UI::{
			Shell::SetWindowSubclass,
			WindowsAndMessaging::{SW_HIDE, ShowWindow},
		},
	};
	use wxdragon::prelude::*;
	// Minimizing first takes the frame's owned windows down with it. A bare
	// hide can leave keyboard focus inside the now-hidden window, so its
	// controls stay reachable.
	frame.iconize(true);
	frame.show(false);
	HIDDEN_TO_TRAY.set(true);
	let hwnd = HWND(frame.get_handle());
	unsafe {
		let _ = SetWindowSubclass(hwnd, Some(hidden_frame_subclass_proc), FRAME_SUBCLASS_ID, 0);
		// Whatever wx believes: if something else showed the window behind its
		// back, wx's own hide above was a no-op.
		let _ = ShowWindow(hwnd, SW_HIDE);
	}
	release_foreground(hwnd);
}

/// Hands the foreground to the desktop if `hwnd` holds it.
fn release_foreground(hwnd: windows::Win32::Foundation::HWND) {
	use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetShellWindow, SetForegroundWindow};
	unsafe {
		if GetForegroundWindow() == hwnd {
			let _ = SetForegroundWindow(GetShellWindow());
		}
	}
}

const FRAME_SUBCLASS_ID: usize = 0x6672_616d;
/// Posted to the frame by itself to give the foreground up once the
/// activation that prompted it has finished.
const WM_RELEASE_FOREGROUND: u32 = 0x8000 + 0x3a71;

unsafe extern "system" fn hidden_frame_subclass_proc(
	hwnd: windows::Win32::Foundation::HWND,
	msg: u32,
	wparam: windows::Win32::Foundation::WPARAM,
	lparam: windows::Win32::Foundation::LPARAM,
	_id: usize,
	_data: usize,
) -> windows::Win32::Foundation::LRESULT {
	use windows::Win32::UI::{
		Shell::DefSubclassProc,
		WindowsAndMessaging::{PostMessageW, SWP_SHOWWINDOW, WINDOWPOS, WM_ACTIVATE, WM_WINDOWPOSCHANGING},
	};
	if msg == WM_RELEASE_FOREGROUND {
		if is_hidden_to_tray() {
			release_foreground(hwnd);
		}
		return windows::Win32::Foundation::LRESULT(0);
	}
	if is_hidden_to_tray() {
		if msg == WM_WINDOWPOSCHANGING {
			// Every way of showing a window comes through here first.
			let pos = lparam.0 as *mut WINDOWPOS;
			if !pos.is_null() {
				unsafe { (*pos).flags &= !SWP_SHOWWINDOW };
			}
		} else if msg == WM_ACTIVATE && wparam.0 & 0xFFFF != 0 {
			// Activated while invisible: keyboard focus would sit in a window
			// nobody can see. Not from inside the activation itself.
			unsafe {
				let _ = PostMessageW(
					Some(hwnd),
					WM_RELEASE_FOREGROUND,
					windows::Win32::Foundation::WPARAM(0),
					windows::Win32::Foundation::LPARAM(0),
				);
			}
		}
	}
	unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// Also refreshes the unread-chat count on the Chats entry of the navigation
/// list, which has to track the same changes and is not gated on the tray icon.
pub(crate) fn refresh_tray_tooltip(s: &AppState, parts: &WindowParts) {
	refresh_nav_unread_labels(s, parts);
	if !s.settings.show_tray_icon || !s.main_ui_shown {
		return;
	}
	let unread = whatsnap::unread_notification_total(
		&s.unread_counts,
		&s.muted_jids,
		&s.archived_jids,
		&s.settings.archived_alert_levels,
	);
	parts.tray_icon.set_icon(&crate::ui::theme::build_app_icon(32), &whatsnap::tray_tooltip_text(unread));
}

/// Writes the unread-chat counts into the Chats and Archived Chats entries of
/// the navigation list.
///
/// A row changes only when its text does: rewriting a listbox row drops its
/// selection on Windows, so the highlight is put back afterwards without an
/// event, the same way the tab reorder keeps the list quiet.
pub(crate) fn refresh_nav_unread_labels(s: &AppState, parts: &WindowParts) {
	let order = s.settings.tab_order();
	let list = &parts.nav_list;
	for tab in crate::settings::NAV_UNREAD_TABS {
		let Some(pos) = order.iter().position(|&t| t == tab) else { continue };
		let label = crate::settings::nav_tab_label(tab, crate::message_display::unread_chats_in_tab(s, tab));
		if list.get_string(pos as u32).as_deref() == Some(label.as_str()) {
			continue;
		}
		let selected = list.get_selection() == Some(pos as u32);
		list.set_string(pos as u32, &label);
		if selected {
			list.set_selection(pos as u32, true);
		}
	}
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

thread_local! {
	static ON_TASKBAR_CREATED: std::cell::RefCell<Option<Box<dyn Fn() -> bool>>> = std::cell::RefCell::new(None);
}

/// What to do when the taskbar is recreated (Explorer restarting, or a display
/// or DPI change). Left to wx, the icon comes back carrying its last tooltip,
/// so the shell names it "20 unread messages" instead of "whatSnap", and at
/// version 0 again. Returns false to let wx handle it after all.
pub(crate) fn set_on_taskbar_created(f: impl Fn() -> bool + 'static) {
	ON_TASKBAR_CREATED.with(|cell| *cell.borrow_mut() = Some(Box::new(f)));
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
		static TASKBAR_CREATED_MSG: u32 = unsafe { RegisterWindowMessageW(windows::core::w!("TaskbarCreated")) };
	}
	if msg == TASKBAR_CREATED_MSG.with(|m| *m)
		&& let Some(f) = ON_TASKBAR_CREATED.with(|cell| cell.borrow_mut().take())
	{
		let handled = f();
		ON_TASKBAR_CREATED.with(|cell| {
			cell.borrow_mut().get_or_insert(f);
		});
		if handled {
			return windows::Win32::Foundation::LRESULT(0);
		}
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
