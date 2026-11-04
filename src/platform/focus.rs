//! Getting keyboard focus back to whatSnap after another window took it, such
//! as the Windows Hello prompt. When that prompt closes, Windows can leave
//! whatSnap disabled and not in front, so keys go nowhere until Alt+Tab.

use windows::Win32::{
	Foundation::HWND,
	System::Threading::{AttachThreadInput, GetCurrentThreadId},
	UI::{
		Input::KeyboardAndMouse::{EnableWindow, GetFocus, IsWindowEnabled, SetFocus},
		WindowsAndMessaging::{
			GW_OWNER, GetForegroundWindow, GetWindow, GetWindowThreadProcessId, IsWindow, IsWindowVisible,
			SetForegroundWindow,
		},
	},
};

/// Shows a modal dialog, then gives the keyboard back.
///
/// `owner` is the window to return to: the main window, or the dialog the
/// error came from. With `take_focus`, whatSnap is brought forward before the
/// dialog opens, for dialogs that arrive on their own (an error from the
/// service while the user is in another app). Focus then goes back to the
/// control that had it before the dialog, when that's still there.
pub fn show_modal_and_refocus(owner: HWND, take_focus: bool, show: impl FnOnce() -> i32) -> i32 {
	let previous = unsafe { GetFocus() };
	if take_focus {
		bring_to_front(owner);
	}
	let result = show();
	bring_to_front(owner);
	unsafe {
		if !previous.is_invalid()
			&& IsWindow(Some(previous)).as_bool()
			&& IsWindowVisible(previous).as_bool()
			&& IsWindowEnabled(previous).as_bool()
		{
			let _ = SetFocus(Some(previous));
		}
	}
	result
}

/// The window that owns `hwnd` (a dialog's main window), or null.
pub fn owner_of(hwnd: HWND) -> HWND {
	unsafe { GetWindow(hwnd, GW_OWNER) }.unwrap_or_default()
}

/// Makes `hwnd` enabled and in front, so it gets the keyboard again. Call it
/// on the UI thread, once the other window has closed.
pub fn bring_to_front(hwnd: HWND) {
	unsafe {
		if !IsWindow(Some(hwnd)).as_bool() {
			return;
		}
		if !IsWindowEnabled(hwnd).as_bool() {
			let _ = EnableWindow(hwnd, true);
		}
		if GetForegroundWindow() == hwnd {
			return;
		}
		if SetForegroundWindow(hwnd).as_bool() {
			return;
		}
		// Windows only lets the process that has the foreground window (or
		// was last given input) move focus. Sharing input with that window's
		// thread for a moment makes the request count; it's undone at once.
		let foreground_thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
		let me = GetCurrentThreadId();
		let attached = foreground_thread != 0
			&& foreground_thread != me
			&& AttachThreadInput(me, foreground_thread, true).as_bool();
		let _ = SetForegroundWindow(hwnd);
		if attached {
			let _ = AttachThreadInput(me, foreground_thread, false);
		}
	}
}
