//! `handle_ui_command` arms that don't fit another feature area (QR pairing,
//! presence, app exit, force sync).

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{AppState, UiCommand, handle_ui_command, tray::refresh_tray_tooltip, ui::window::WindowParts};

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
	parts.frame.show(true);
	parts.frame.iconize(false);
	parts.frame.raise();
	handle_ui_command(UiCommand::SetPresence(true), state, parts);
}

pub(crate) fn set_presence(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, available: bool) {
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
			let _ = s.ws_tx.send(payload.to_string());
			// handle_mark_read on the backend only broadcasts a
			// "chat_updated" event, which nothing here listens for -
			// left alone, unread_counts (and thus the chat list's
			// "N unread messages" label, and the divider recreated
			// from it on the next OpenConversation) would stay stale
			// until some unrelated event happened to trigger a full
			// chats resync. Clear it locally now and ask for a fresh
			// snapshot to repaint the list right away.
			s.unread_counts.insert(jid, 0);
			refresh_tray_tooltip(&s, parts);
			let refresh_payload = serde_json::json!({ "action": "get_chats" });
			let _ = s.ws_tx.send(refresh_payload.to_string());
		} else {
			s.background_unread_message_ids.clear();
		}
	}

	// Going unavailable (blur, minimize, hide-to-tray) must pause any
	// in-flight composing state too, for the same reason a chat switch
	// does - otherwise backgrounding the app mid-compose leaves the
	// other side seeing "typing…" indefinitely.
	if !available {
		if let Some(jid) = s.selected_chat.clone() {
			let typing_payload = serde_json::json!({
				"action": "send_typing",
				"data": { "jid": jid, "state": "paused", "media": "" }
			});
			let _ = s.ws_tx.send(typing_payload.to_string());
		}
	}

	let payload = serde_json::json!({
		"action": "set_presence",
		"data": {
			"available": available
		}
	});
	let _ = s.ws_tx.send(payload.to_string());
}
