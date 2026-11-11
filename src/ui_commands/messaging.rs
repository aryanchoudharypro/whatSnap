use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState,
	ui::{
		chat_ui::{clear_pending_reply, clear_unread_divider_for_open_chat},
		window::WindowParts,
	},
};

pub(crate) fn send_message(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, text: String) {
	if crate::ui_commands::blocking::stop_if_cant_send(state, parts) {
		return;
	}
	let _slow = crate::backend::events::SlowUiLog::new("sending a message");
	let mut s = state.borrow_mut();
	if let Some(ref jid) = s.selected_chat.clone() {
		clear_unread_divider_for_open_chat(&mut s, parts, jid);

		if !s.messages_at_latest && !s.settings.keep_list_position {
			let reload = serde_json::json!({
				"action": "get_messages",
				"data": { "jid": jid, "limit": crate::MESSAGE_PAGE_SIZE, "offset": 0 }
			});
			let _ = s.ws_tx.send(reload.to_string());
		}

		let reply_id = s.pending_reply_id.take().unwrap_or_default();
		let reply_sender = s.pending_reply_sender.take().unwrap_or_default();
		s.pending_reply_preview = None;

		let mentions = crate::ui_commands::mentions::mentions_payload(&s, &text);
		let payload = serde_json::json!({
			"action": "send",
			"data": {
				"to": jid,
				"text": text,
				"reply_to_id": reply_id,
				"reply_to_sender": reply_sender,
				"mentions": mentions
			}
		});
		let _ = s.ws_tx.send(payload.to_string());
		s.draft_messages.remove(jid);
		s.draft_mentions.remove(jid);
		s.composer_mentions.clear();
		crate::ui_commands::mentions::close(&mut s, &parts.conversations_parts);
		parts.conversations_parts.message_field.set_value("");
		crate::ui::chat_ui::update_composer_buttons(&s, &parts.conversations_parts);
	}
}

pub(crate) fn cancel_edit(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let mut s = state.borrow_mut();
	let was_editing = s.editing_message_id.take().is_some();
	s.editing_message_original_text = None;
	s.composer_mentions.clear();
	crate::ui::chat_ui::update_composer_buttons(&s, &parts.conversations_parts);
	drop(s);
	end_edit_mode(parts);
	if was_editing {
		crate::platform::announce::announce(
			parts.conversations_parts.messages_announcer,
			crate::i18n::tr("Editing cancelled."),
		);
	}
}

pub(crate) fn begin_edit_message(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	message_id: String,
	text: String,
) {
	if stop_edit_if_cant_send(state, parts) {
		return;
	}
	let mut s = state.borrow_mut();
	s.editing_message_id = Some(message_id);
	s.editing_message_original_text = Some(text.clone());
	s.composer_mentions.clear();
	let field = &parts.conversations_parts.message_field;
	field.set_value(&text);
	let end = text.chars().count() as i64;
	field.set_selection(end, end);
	crate::ui::chat_ui::update_composer_buttons(&s, &parts.conversations_parts);
	drop(s);
	field.set_focus();
	crate::platform::announce::announce(
		parts.conversations_parts.messages_announcer,
		crate::i18n::tr("Editing message. Press Enter to save, Escape to cancel."),
	);
}

fn end_edit_mode(parts: &Rc<WindowParts>) {
	parts.conversations_parts.message_field.set_value("");
	parts.conversations_parts.message_field.set_focus();
}

fn stop_edit_if_cant_send(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) -> bool {
	let reason = {
		let s = state.borrow();
		s.selected_chat.as_deref().and_then(|jid| crate::ui::chat_ui::group_send_block_reason(&s, jid))
	};
	if let Some(reason) = reason {
		crate::platform::announce::announce(parts.conversations_parts.messages_announcer, reason);
		return true;
	}
	false
}

pub(crate) fn remove_quote(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	clear_pending_reply(&mut state.borrow_mut(), parts);
	crate::platform::announce::announce(
		parts.conversations_parts.messages_announcer,
		crate::i18n::tr("Quoted message removed."),
	);
	parts.conversations_parts.message_field.set_focus();
}

pub(crate) fn send_typing(
	state: &Rc<RefCell<AppState>>,
	_parts: &Rc<WindowParts>,
	jid: String,
	typing_state: String,
	media: String,
) {
	let mut s = state.borrow_mut();
	let Some(target) = (if jid.is_empty() { s.selected_chat.clone() } else { Some(jid) }) else {
		return;
	};
	if typing_state == "composing" {
		mark_composing(&mut s, &target, &media);
	} else {
		mark_paused(&mut s, &target);
	}
}

/// WA Web never sends chatstates to newsletters, bots or broadcast lists.
fn chatstate_allowed(jid: &str) -> bool {
	!(jid.ends_with("@newsletter")
		|| jid.ends_with("@broadcast")
		|| jid.ends_with("@bot")
		|| jid == crate::types::META_AI_JID)
}

pub(crate) fn mark_composing(s: &mut AppState, jid: &str, media: &str) {
	if !chatstate_allowed(jid) {
		return;
	}
	let now = std::time::Instant::now();
	match s.composing.as_mut() {
		Some(c) if c.jid == jid && c.media == media => c.last_key = now,
		_ => {
			if let Some(old) = s.composing.as_ref().map(|c| c.jid.clone())
				&& old != jid
			{
				send_chatstate(s, &old, "paused", "");
			}
			send_chatstate(s, jid, "composing", media);
			s.composing = Some(crate::ComposingState {
				jid: jid.to_string(),
				media: media.to_string(),
				last_sent: now,
				last_key: now,
			});
		}
	}
}

pub(crate) fn mark_paused(s: &mut AppState, jid: &str) {
	if s.composing.as_ref().is_some_and(|c| c.jid == jid) {
		send_chatstate(s, jid, "paused", "");
	}
}

pub(crate) fn send_chatstate(s: &mut AppState, jid: &str, typing_state: &str, media: &str) {
	if typing_state == "paused" {
		s.composing = None;
	}
	let payload = serde_json::json!({
		"action": "send_typing",
		"data": { "jid": jid, "state": typing_state, "media": media }
	});
	let _ = s.ws_tx.send(payload.to_string());
}

pub(crate) fn chatstate_tick(s: &mut AppState) {
	let Some((jid, media, last_sent, last_key)) =
		s.composing.as_ref().map(|c| (c.jid.clone(), c.media.clone(), c.last_sent, c.last_key))
	else {
		return;
	};
	if media.is_empty() && last_key.elapsed() >= crate::CHATSTATE_PAUSE_AFTER {
		send_chatstate(s, &jid, "paused", "");
		return;
	}
	if last_sent.elapsed() >= crate::CHATSTATE_RESEND_EVERY && s.contact_online.get(&jid) != Some(&false) {
		send_chatstate(s, &jid, "composing", &media);
		if let Some(c) = s.composing.as_mut() {
			c.last_sent = std::time::Instant::now();
		}
	}
}

pub(crate) fn contact_came_online(s: &mut AppState, jid: &str) {
	let resend = s.composing.as_ref().filter(|c| c.jid == jid).map(|c| c.media.clone());
	if let Some(media) = resend {
		send_chatstate(s, jid, "composing", &media);
		if let Some(c) = s.composing.as_mut() {
			c.last_sent = std::time::Instant::now();
		}
	}
}

pub(crate) fn forward_message(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	message_id: String,
	to_jids: Vec<String>,
) {
	let payload = serde_json::json!({
		"action": "forward_message",
		"data": {
			"message_id": message_id,
			"to_jids": to_jids
		}
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	crate::platform::announce::announce(
		parts.conversations_parts.messages_announcer,
		crate::i18n::tr("Message forwarded."),
	);
}

pub(crate) fn edit_message(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	message_id: String,
	new_text: String,
) {
	let new_text = new_text.trim().to_string();
	let mut s = state.borrow_mut();
	let original = s.editing_message_original_text.take();
	s.editing_message_id = None;
	let jid = s.selected_chat.clone();
	let mentions = crate::ui_commands::mentions::mentions_payload(&s, &new_text);
	s.composer_mentions.clear();
	drop(s);

	let announcement = if new_text.is_empty() {
		crate::i18n::tr("Editing cancelled - message can't be empty.")
	} else if original.as_deref() == Some(new_text.as_str()) {
		crate::i18n::tr("Editing cancelled - no changes made.")
	} else {
		if let Some(jid) = jid {
			let payload = serde_json::json!({
				"action": "edit_message",
				"data": {
					"jid": jid,
					"message_id": message_id,
					"new_text": new_text,
					"mentions": mentions
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		crate::i18n::tr("Message edited.")
	};
	end_edit_mode(parts);
	crate::platform::announce::announce(parts.conversations_parts.messages_announcer, announcement);
}

pub(crate) fn delete_message(
	state: &Rc<RefCell<AppState>>,
	_parts: &Rc<WindowParts>,
	message_id: String,
	sender_jid: String,
	for_everyone: bool,
) {
	let jid = state.borrow().selected_chat.clone();
	if let Some(jid) = jid {
		let payload = serde_json::json!({
			"action": "delete_message",
			"data": {
				"jid": jid,
				"message_id": message_id,
				"sender_jid": sender_jid,
				"for_everyone": for_everyone
			}
		});
		let _ = state.borrow().ws_tx.send(payload.to_string());
	}
}

pub(crate) fn star_message(
	state: &Rc<RefCell<AppState>>,
	_parts: &Rc<WindowParts>,
	message_id: String,
	sender_jid: String,
	from_me: bool,
	starred: bool,
) {
	let jid = state.borrow().selected_chat.clone();
	if let Some(jid) = jid {
		let payload = serde_json::json!({
			"action": "star_message",
			"data": {
				"jid": jid,
				"message_id": message_id,
				"sender_jid": sender_jid,
				"from_me": from_me,
				"starred": starred
			}
		});
		let _ = state.borrow().ws_tx.send(payload.to_string());
	}
}

pub(crate) fn export_chat(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String, dest_path: String) {
	let payload = serde_json::json!({
		"action": "export_chat",
		"data": { "jid": jid, "dest_path": dest_path }
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	crate::platform::announce::announce(
		parts.conversations_parts.messages_announcer,
		crate::i18n::tr("Exporting chat, this may take a while for chats with a lot of media..."),
	);
}

pub(crate) fn export_chat_text(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String, dest_path: String) {
	let payload = serde_json::json!({
		"action": "export_chat_text",
		"data": { "jid": jid, "dest_path": dest_path }
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	crate::platform::announce::announce(
		parts.conversations_parts.messages_announcer,
		crate::i18n::tr("Exporting chat text..."),
	);
}

pub(crate) fn post_status(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, text: String) {
	let payload = serde_json::json!({
		"action": "post_status",
		"data": { "text": text }
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	crate::platform::announce::announce(
		parts.conversations_parts.messages_announcer,
		crate::i18n::tr("Posting status update..."),
	);
}

pub(crate) fn post_custom_status(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	text: String,
	background: u32,
	text_color: u32,
	font: i32,
) {
	let payload = serde_json::json!({
		"action": "post_status",
		"data": {
			"text": text,
			"background_argb": background,
			"text_argb": text_color,
			"font": font,
		}
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	crate::platform::announce::announce(
		parts.conversations_parts.messages_announcer,
		crate::i18n::tr("Posting status update..."),
	);
}

pub(crate) fn open_status_composer(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	crate::ui::dialogs::status_composer::show_text_status_composer(&parts.frame, state, parts);
}

pub(crate) fn post_media_status(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	crate::ui::dialogs::status_composer::show_media_status_composer(&parts.frame, state, parts);
}

pub(crate) fn get_message_info(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>, message_id: String) {
	let payload = serde_json::json!({
		"action": "get_message_info",
		"data": { "id": message_id }
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

pub(crate) fn retry_message(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>, message_id: String) {
	let payload = serde_json::json!({
		"action": "retry_message",
		"data": { "message_id": message_id }
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

pub(crate) fn retry_selected_message(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let s = state.borrow();
	let failed: Vec<String> = parts
		.conversations_parts
		.messages_list
		.get_selection()
		.and_then(|idx| s.display_items.get(idx as usize))
		.map(|item| match item {
			whatsnap::DisplayItem::Single(m) => std::slice::from_ref(m),
			whatsnap::DisplayItem::Album { messages, .. } => messages.as_slice(),
		})
		.unwrap_or_default()
		.iter()
		.filter(|m| m.from_me && !m.is_date_divider && m.status == "failed")
		.map(|m| m.id.clone())
		.collect();
	for message_id in &failed {
		let payload = serde_json::json!({
			"action": "retry_message",
			"data": { "message_id": message_id }
		});
		let _ = s.ws_tx.send(payload.to_string());
	}
	if !failed.is_empty() {
		crate::platform::announce::announce(
			parts.conversations_parts.messages_announcer,
			crate::i18n::tr("Retrying send..."),
		);
	}
}

pub(crate) fn cancel_selected_upload(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	// The borrow must end before the modal dialog: its event loop can run
	// handlers that borrow the state themselves.
	let (jid, uploading): (String, Vec<String>) = {
		let s = state.borrow();
		let Some(jid) = s.selected_chat.clone() else { return };
		let uploading = parts
			.conversations_parts
			.messages_list
			.get_selection()
			.and_then(|idx| s.display_items.get(idx as usize))
			.map(|item| match item {
				whatsnap::DisplayItem::Single(m) => std::slice::from_ref(m),
				whatsnap::DisplayItem::Album { messages, .. } => messages.as_slice(),
			})
			.unwrap_or_default()
			.iter()
			.filter(|m| m.from_me && !m.is_date_divider && m.status == "uploading")
			.map(|m| m.id.clone())
			.collect();
		(jid, uploading)
	};
	if uploading.is_empty() {
		return;
	}
	let confirm = MessageDialog::builder(
		&parts.frame,
		crate::i18n::tr("Cancel this upload? The message will be removed from the chat."),
		crate::i18n::tr("Cancel upload"),
	)
	.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
	.build();
	if confirm.show_modal() != ID_YES {
		return;
	}
	let s = state.borrow();
	for message_id in &uploading {
		let payload = serde_json::json!({
			"action": "cancel_upload",
			"data": { "jid": jid, "message_id": message_id }
		});
		let _ = s.ws_tx.send(payload.to_string());
	}
}

pub(crate) fn request_message_resend(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, message_id: String) {
	let s = state.borrow();
	if let Some(jid) = s.selected_chat.clone() {
		let payload = serde_json::json!({
			"action": "request_message_resend",
			"data": { "jid": jid, "message_id": message_id }
		});
		let _ = s.ws_tx.send(payload.to_string());
		crate::platform::announce::announce(
			parts.conversations_parts.messages_announcer,
			crate::i18n::tr("Requested this message from your phone"),
		);
	}
}

pub(crate) fn react_to_message(
	state: &Rc<RefCell<AppState>>,
	_parts: &Rc<WindowParts>,
	message_id: String,
	sender_jid: String,
	emoji: String,
) {
	let s = state.borrow();
	if let Some(ref jid) = s.selected_chat {
		let payload = serde_json::json!({
			"action": "react_message",
			"data": {
				"jid": jid,
				"message_id": message_id,
				"sender_jid": sender_jid,
				"emoji": emoji
			}
		});
		let _ = s.ws_tx.send(payload.to_string());
	}
}
