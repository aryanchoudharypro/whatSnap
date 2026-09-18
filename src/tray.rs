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
