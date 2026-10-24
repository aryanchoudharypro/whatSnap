//! `handle_ui_command` arms that don't fit another feature area (QR pairing,
//! presence, app exit, force sync).

use std::{cell::RefCell, rc::Rc};

use crate::{AppState, UiCommand, handle_ui_command, ui::window::WindowParts};

pub(crate) fn request_qr(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>) {
	state.borrow_mut().is_pairing_active = false;
	let payload = serde_json::json!({
		"action": "get_qr"
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

pub(crate) fn cancel_qr(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>) {
	state.borrow_mut().is_pairing_active = false;
	let payload = serde_json::json!({
		"action": "cancel_qr"
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

pub(crate) fn cancel_pairing(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>) {
	state.borrow_mut().is_pairing_active = false;
	let payload = serde_json::json!({
		"action": "cancel_pairing"
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

pub(crate) fn request_pairing(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>, phone: String) {
	state.borrow_mut().is_pairing_active = true;
	let payload = serde_json::json!({
		"action": "pair",
		"data": {
			"phone": phone
		}
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

pub(crate) fn exit_app(_state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	// See the matching comment on the tray menu's Exit handler in
	// ui/window.rs: closing the main frame alone doesn't end the app
	// because other top-level windows (the call window, dialogs) are
	// built once at startup and never destroyed.
	parts.frame.close(true);
	if let Some(app) = wxdragon::get_app_instance() {
		app.exit_main_loop();
	}
}

pub(crate) fn force_sync(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>) {
	let _ = state.borrow().ws_tx.send(serde_json::json!({ "action": "force_sync" }).to_string());
}

pub(crate) fn subscribe_presence(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>, jid: String) {
	let payload = serde_json::json!({
		"action": "subscribe_presence",
		"data": {
			"jid": jid
		}
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

pub(crate) fn activate_window(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	// A newly launched second instance is asking us to come to the
	// foreground instead of starting its own copy. We have to be the
	// one to call raise()/SetForegroundWindow on our own window —
	// the launcher process doing it directly would just flash the
	// taskbar icon, since Windows only lets a process steal focus
	// for itself (see single_instance::request_activate).
	crate::tray::restore_main_window(&parts.frame);
	handle_ui_command(UiCommand::SetPresence(true), state, parts);
}

pub(crate) fn set_presence(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>, available: bool) {
	let mut s = state.borrow_mut();
	let regained_focus = available && !s.window_focused;
	s.window_focused = available;

	// The "N unread messages" divider is already built and kept
	// up to date live as messages arrive in the background (see the
	// "message" event handler); on refocus there's nothing left to
	// do but send the batched read receipt for what queued up.
	if regained_focus && !s.background_unread_message_ids.is_empty() {
		if let Some(jid) = s.selected_chat.clone() {
			let ids = std::mem::take(&mut s.background_unread_message_ids);
			let payload = serde_json::json!({
				"action": "mark_read",
				"data": { "jid": jid, "message_ids": ids }
			});
			// The chat list and tray catch up on the backend's "chat_updated",
			// sent once the server has acknowledged the receipts.
			let _ = s.ws_tx.send(payload.to_string());
		} else {
			s.background_unread_message_ids.clear();
		}
	}

	if available {
		// WAWebConversation: availability comes back only once focus has
		// held for PRESENCE_FOCUS_SETTLE - a focus flicker isn't presence.
		s.presence.hidden = false;
		s.presence.focus_settle_since = Some(std::time::Instant::now());
	} else {
		// Losing focus stops typing right away (the other side shouldn't
		// keep seeing "typing..." for text nobody is writing), but presence
		// only drops after PRESENCE_BLUR_GRACE.
		if let Some(jid) = s.composing.as_ref().map(|c| c.jid.clone()) {
			crate::ui_commands::messaging::mark_paused(&mut s, &jid);
		}
		s.presence.focus_settle_since = None;
		mark_unavailable_within(&mut s, crate::PRESENCE_BLUR_GRACE);
	}
}

/// Stream.markUnavailable(ms): a shift timer - fires at the earlier of any
/// already-pending deadline and this one.
fn mark_unavailable_within(s: &mut AppState, within: std::time::Duration) {
	let at = std::time::Instant::now() + within;
	s.presence.unavailable_at = Some(s.presence.unavailable_at.map_or(at, |t| t.min(at)));
}

/// The window went to the tray: unavailable at once (WAWebStreamModel's
/// isMinimizedToTray check), unless a call is connected.
pub(crate) fn window_hidden(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	set_presence(state, parts, false);
	let mut s = state.borrow_mut();
	s.presence.hidden = true;
	apply_presence(&mut s);
}

/// Milliseconds-since-boot tick of the last keyboard/mouse input anywhere
/// (GetLastInputInfo) - WA Web's mousemove/scroll/keydown activity signal.
fn last_input_tick() -> u32 {
	use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
	let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
	if unsafe { GetLastInputInfo(&mut info) }.as_bool() { info.dwTime } else { 0 }
}

/// Periodic timer driving the availability model: focus settle, WA Web's
/// idle loop, and the pending markUnavailable deadline.
pub(crate) fn presence_tick(s: &mut AppState) {
	let now = std::time::Instant::now();

	if s.presence.focus_settle_since.is_some_and(|t| now.duration_since(t) >= crate::PRESENCE_FOCUS_SETTLE) {
		s.presence.focus_settle_since = None;
		if s.window_focused {
			s.presence.available = true;
			s.presence.unavailable_at = None;
			// WA Web resubscribes the open chat's presence on coming back.
			if let Some(jid) = s.selected_chat.clone() {
				let payload = serde_json::json!({ "action": "subscribe_presence", "data": { "jid": jid } });
				let _ = s.ws_tx.send(payload.to_string());
			}
		}
	}

	// Idle loop (WAWebMain): every PRESENCE_IDLE_CHECK_EVERY, arm
	// markUnavailable(PRESENCE_IDLE_GRACE); any input cancels it (and, if
	// focused, marks available again) and restarts the wait.
	match s.presence.idle_armed_at_input {
		None if now >= s.presence.next_idle_check => {
			s.presence.idle_armed_at_input = Some(last_input_tick());
			mark_unavailable_within(s, crate::PRESENCE_IDLE_GRACE);
		}
		Some(armed) if last_input_tick() != armed => {
			s.presence.idle_armed_at_input = None;
			s.presence.next_idle_check = now + crate::PRESENCE_IDLE_CHECK_EVERY;
			if s.window_focused && !s.presence.hidden {
				s.presence.available = true;
				s.presence.unavailable_at = None;
			}
		}
		_ => {}
	}

	if s.presence.unavailable_at.is_some_and(|t| now >= t) {
		s.presence.unavailable_at = None;
		s.presence.available = false;
	}

	apply_presence(s);
}

/// What WAWebStreamModel.sendAvailability would send right now.
pub(crate) fn desired_presence(s: &AppState) -> bool {
	let in_connected_call = s.active_call_audio.is_some();
	in_connected_call || (!s.presence.hidden && s.presence.available)
}

fn apply_presence(s: &mut AppState) {
	let want = desired_presence(s);
	send_presence(s, want);
}

pub(crate) fn send_presence(s: &mut AppState, available: bool) {
	if s.presence_sent == Some(available) {
		return;
	}
	s.presence_sent = Some(available);
	let payload = serde_json::json!({
		"action": "set_presence",
		"data": { "available": available }
	});
	let _ = s.ws_tx.send(payload.to_string());
}
