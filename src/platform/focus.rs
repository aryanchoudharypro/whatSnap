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

/// The window the user was last in that belongs to another program, kept so a
/// notification button can hand the keyboard back to it. Pressing a button in
/// a notification moves the foreground to the notification, and when it
/// closes Windows leaves nothing focused (a screen reader says "unknown").
static PREVIOUS: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

fn is_notification_host(hwnd: HWND) -> bool {
	let mut name = [0u16; 64];
	let len = unsafe { windows::Win32::UI::WindowsAndMessaging::GetClassNameW(hwnd, &mut name) } as usize;
	String::from_utf16_lossy(&name[..len.min(name.len())]) == "Windows.UI.Core.CoreWindow"
}

unsafe extern "system" fn on_foreground(
	_hook: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
	_event: u32,
	hwnd: HWND,
	_object: i32,
	_child: i32,
	_thread: u32,
	_time: u32,
) {
	if hwnd.is_invalid() || is_notification_host(hwnd) {
		return;
	}
	let mut pid = 0u32;
	unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
	if pid != std::process::id() {
		PREVIOUS.store(hwnd.0 as isize, std::sync::atomic::Ordering::SeqCst);
	}
}

/// Starts remembering which other program's window was in front.
pub fn track_foreground() {
	use windows::Win32::UI::{
		Accessibility::SetWinEventHook,
		WindowsAndMessaging::{
			DispatchMessageW, EVENT_SYSTEM_FOREGROUND, GetMessageW, MSG, TranslateMessage, WINEVENT_OUTOFCONTEXT,
		},
	};
	std::thread::spawn(|| unsafe {
		let hook = SetWinEventHook(
			EVENT_SYSTEM_FOREGROUND,
			EVENT_SYSTEM_FOREGROUND,
			None,
			Some(on_foreground),
			0,
			0,
			WINEVENT_OUTOFCONTEXT,
		);
		if hook.is_invalid() {
			return;
		}
		let mut msg = MSG::default();
		while GetMessageW(&mut msg, None, 0, 0).as_bool() {
			let _ = TranslateMessage(&msg);
			DispatchMessageW(&msg);
		}
	});
}

/// After a notification button that works without whatSnap's window: waits for
/// the notification to close and, if that left no window focused, puts the
/// user back in the window they were in. Does nothing if they already moved on.
pub fn restore_previous_after_toast() {
	let raw = PREVIOUS.load(std::sync::atomic::Ordering::SeqCst);
	if raw == 0 {
		return;
	}
	std::thread::spawn(move || {
		let target = HWND(raw as *mut _);
		for _ in 0..12 {
			std::thread::sleep(std::time::Duration::from_millis(100));
			unsafe {
				use windows::Win32::UI::WindowsAndMessaging::IsIconic;
				if !IsWindow(Some(target)).as_bool() || !IsWindowVisible(target).as_bool() || IsIconic(target).as_bool()
				{
					return;
				}
				let current = GetForegroundWindow();
				if current == target {
					return;
				}
				if !current.is_invalid() && !is_notification_host(current) {
					return;
				}
				if SetForegroundWindow(target).as_bool() {
					return;
				}
				let foreground_thread = GetWindowThreadProcessId(current, None);
				let me = GetCurrentThreadId();
				let attached = foreground_thread != 0
					&& foreground_thread != me
					&& AttachThreadInput(me, foreground_thread, true).as_bool();
				let _ = SetForegroundWindow(target);
				if attached {
					let _ = AttachThreadInput(me, foreground_thread, false);
				}
			}
		}
	});
}
