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
	crate::platform::tray::quit_app(&parts.tray_icon, &[parts.frame, parts.call_parts.frame]);
}

pub(crate) fn restart_app(_state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	if let Err(err) = crate::platform::single_instance::spawn_restarted_copy() {
		use wxdragon::prelude::*;
		let msg = crate::i18n::tr_fmt("Couldn't restart whatSnap automatically:\n{}\n\nPlease close and reopen the app manually.", &[&err]);
		MessageDialog::builder(&parts.frame, &msg, crate::i18n::tr("Restart Required"))
			.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconWarning)
			.build()
			.show_modal();
		return;
	}
	crate::platform::tray::quit_app(&parts.tray_icon, &[parts.frame, parts.call_parts.frame]);
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
	use wxdragon::prelude::*;
	// Until the chats are up the main window is kept hidden on purpose:
	// showing it would show an empty frame. Bring up the sign-in window if
	// that's what's open; otherwise the window opens by itself once ready.
	if !state.borrow().main_ui_shown {
		for dialog in [&parts.login_panel, &parts.qrcode_dialog, &parts.pairing_dialog] {
			if dialog.is_shown() {
				dialog.raise();
				dialog.set_focus();
				break;
			}
		}
		return;
	}
	crate::platform::tray::restore_main_window(&parts.frame);
	// wx's raise only reorders windows; taking the keyboard from another app
	// needs the foreground, which the notification click or the launch that
	// handed off to us allowed.
	#[cfg(target_os = "windows")]
	crate::platform::focus::bring_to_front(windows::Win32::Foundation::HWND(parts.frame.get_handle().cast()));
	handle_ui_command(UiCommand::SetPresence(true), state, parts);
}

pub(crate) fn set_presence(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, available: bool) {
	if available && state.borrow().pending_update_check {
		let (channel, ui_tx) = {
			let mut s = state.borrow_mut();
			s.pending_update_check = false;
			(s.settings.update_channel.clone(), s.ui_tx.clone())
		};
		crate::platform::update_check::run(&parts.frame, &ui_tx, &channel, true);
	}
	let mut s = state.borrow_mut();
	let regained_focus = available && !s.window_focused;
	s.window_focused = available;

	// Like WA Web, the open chat is marked read only once the window has
	// stayed in front for a moment, not the instant it flickers past.
	if regained_focus {
		match s.selected_chat.clone() {
			Some(jid) => s.focus_read_at = Some((std::time::Instant::now() + crate::FOCUS_READ_DELAY, jid)),
			None => s.background_unread_message_ids.clear(),
		}
	} else if !available {
		s.focus_read_at = None;
	}

	if available {
		s.presence.hidden = false;
		s.presence.focus_settle_since = Some(std::time::Instant::now());
	} else {
		if let Some(jid) = s.composing.as_ref().map(|c| c.jid.clone()) {
			crate::ui_commands::messaging::mark_paused(&mut s, &jid);
		}
		s.presence.focus_settle_since = None;
		mark_unavailable_within(&mut s, crate::PRESENCE_BLUR_GRACE);
	}
}

fn mark_unavailable_within(s: &mut AppState, within: std::time::Duration) {
	let at = std::time::Instant::now() + within;
	s.presence.unavailable_at = Some(s.presence.unavailable_at.map_or(at, |t| t.min(at)));
}

pub(crate) fn window_hidden(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	set_presence(state, parts, false);
	// Sent to the tray, the open chat is closed as Escape would: otherwise
	// its new messages are treated as seen and never notify. A voice message
	// being recorded is kept.
	let close_chat = {
		let s = state.borrow();
		s.selected_chat.is_some() && !s.is_voice_recording
	};
	if close_chat {
		crate::ui_commands::chat_list::close_conversation(state, parts);
	}
	let mut s = state.borrow_mut();
	s.presence.hidden = true;
	apply_presence(&mut s);
}

fn last_input_tick() -> u32 {
	use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
	let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
	if unsafe { GetLastInputInfo(&mut info) }.as_bool() { info.dwTime } else { 0 }
}

pub(crate) fn presence_tick(s: &mut AppState) {
	let now = std::time::Instant::now();

	if s.focus_read_at.as_ref().is_some_and(|(at, _)| now >= *at) {
		let (_, jid) = s.focus_read_at.take().unwrap();
		if s.window_focused && s.selected_chat.as_deref() == Some(jid.as_str()) {
			mark_open_chat_read(s, &jid);
		}
	}

	if s.presence.focus_settle_since.is_some_and(|t| now.duration_since(t) >= crate::PRESENCE_FOCUS_SETTLE) {
		s.presence.focus_settle_since = None;
		if s.window_focused {
			s.presence.available = true;
			s.presence.unavailable_at = None;
			if let Some(jid) = s.selected_chat.clone() {
				let payload = serde_json::json!({ "action": "subscribe_presence", "data": { "jid": jid } });
				let _ = s.ws_tx.send(payload.to_string());
			}
		}
	}

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

/// Marks the open chat read after it was left unread in the background: the
/// messages that came in meanwhile, or the whole chat if it still counts
/// unread ones (which also catches any the queue missed).
pub(crate) fn mark_open_chat_read(s: &mut AppState, jid: &str) {
	let ids = std::mem::take(&mut s.background_unread_message_ids);
	if ids.is_empty() && s.unread_counts.get(jid).copied().unwrap_or(0) == 0 {
		return;
	}
	let payload = serde_json::json!({
		"action": "mark_read",
		"data": { "jid": jid, "message_ids": ids }
	});
	let _ = s.ws_tx.send(payload.to_string());
}
