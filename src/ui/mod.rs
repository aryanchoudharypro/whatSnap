pub mod avatars;
pub mod call_window;
pub mod chat_overview;
pub mod chat_rows;
pub mod chat_ui;
pub mod contact_picker;
pub mod conversation_data;
pub mod conversations;
pub mod dialog_look;
pub mod dialogs;
pub mod emoji_panel;
pub mod emoji_replace;
pub mod gdi;
pub mod group_data;
pub mod keys;
pub mod media_player;
pub mod message_art;
pub mod message_display;
pub mod message_list;
pub mod owner_draw;
pub mod sound_picker;
pub mod spell_check;
pub mod status_viewer;
pub mod theme;
pub mod thumbnails;
pub mod video_player;
pub mod wake;

pub mod window;

/// Shows, modally, one of the dialogs that are built once and reused.
///
/// wxWidgets 3.3 won't show a dialog whose return code isn't 0: it takes
/// that to mean EndModal was called while the dialog was being set up. The
/// code is left over from the last time it closed, so a dialog closed with
/// Escape, Cancel or OK (anything but 0) would never open again. Clearing it
/// first is what wx 3.2 effectively did.
pub(crate) fn show_modal_again(dialog: &wxdragon::prelude::Dialog) -> i32 {
	use wxdragon::prelude::WxWidget;
	dialog.set_return_code(0);
	let hwnd = windows::Win32::Foundation::HWND(dialog.get_handle());
	crate::platform::focus::show_modal_and_refocus(crate::platform::focus::owner_of(hwnd), false, || {
		dialog.show_modal()
	})
}
