//! `handle_ui_command` arms for composing and managing messages (send/edit/delete/
//! star/react/forward/retry, status posts, typing indicator, chat export).

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState,
	chat_ui::{clear_pending_reply, clear_unread_divider_for_open_chat},
	ui::window::WindowParts,
};

pub(crate) fn send_message(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, text: String) {
	if crate::ui_commands::blocking::stop_if_cant_send(state, parts) {
		return;
	}
	let _slow = crate::websocket_events::SlowUiLog::new("sending a message");
	let mut s = state.borrow_mut();
	if let Some(ref jid) = s.selected_chat.clone() {
		// Sending a reply means the unread messages have obviously been
		// seen, so drop the "N unread messages" divider from the list
		// right away rather than waiting for the next time the chat is
		// opened.
		clear_unread_divider_for_open_chat(&mut s, parts, jid);

		// Sending from a window opened around an old search hit: jump back to
		// the newest messages first, so the one being sent shows up where it
		// belongs (live messages aren't appended under such a window).
		if !s.messages_at_latest {
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
		parts.conversations_parts.send_message_btn.show(false);
		parts.conversations_parts.remove_quote_btn.show(false);
		parts.conversations_parts.conversation_panel.layout();
	}
}

pub(crate) fn cancel_edit(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let mut s = state.borrow_mut();
	let was_editing = s.editing_message_id.take().is_some();
	s.editing_message_original_text = None;
	s.composer_mentions.clear();
	drop(s);
	end_edit_mode(parts);
	if was_editing {
		crate::announce::announce(parts.conversations_parts.messages_announcer, "Editing cancelled.");
	}
}

/// Puts the composer into edit mode for `message_id`: fills `message_field`
/// with its current text (caret at the end, nothing selected), swaps the
/// composer's label to "Edit message", and shows the "Cancel edit" button in
/// place of "Add attachment" - editing an existing message can't also attach
/// new media, and hiding that button puts "Cancel edit" one Tab away from the
/// field, mirroring the single-line edit dialog this replaced.
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
	// Mentions already in the message are kept by the backend; this holds
	// only ones picked during the edit.
	s.composer_mentions.clear();
	drop(s);
	let field = &parts.conversations_parts.message_field;
	field.set_value(&text);
	// Caret at the end, nothing selected - typing continues the message
	// rather than replacing it outright, since most edits are small tweaks
	// (a typo, an added word) rather than a full rewrite.
	let end = text.chars().count() as i64;
	field.set_selection(end, end);
	parts.conversations_parts.message_label.set_label("Edit message");
	parts.conversations_parts.add_attachment_btn.show(false);
	parts.conversations_parts.cancel_edit_btn.show(true);
	parts.conversations_parts.conversation_panel.layout();
	field.set_focus();
	crate::announce::announce(
		parts.conversations_parts.messages_announcer,
		"Editing message. Press Enter to save, Escape to cancel.",
	);
}

/// Restores the composer to its normal (not editing) state - shared by
/// `cancel_edit` and `edit_message` (once the edit is sent, the composer
/// goes back to normal too), so the two only ever agree on what
/// "not editing" looks like in one place.
fn end_edit_mode(parts: &Rc<WindowParts>) {
	parts.conversations_parts.message_field.set_value("");
	parts.conversations_parts.cancel_edit_btn.show(false);
	parts.conversations_parts.add_attachment_btn.show(true);
	parts.conversations_parts.message_label.set_label("Type a message");
	parts.conversations_parts.conversation_panel.layout();
	parts.conversations_parts.message_field.set_focus();
}

/// Stops an edit from starting where nothing can be sent; says why.
fn stop_edit_if_cant_send(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) -> bool {
	let reason = {
		let s = state.borrow();
		s.selected_chat.as_deref().and_then(|jid| crate::chat_ui::group_send_block_reason(&s, jid))
	};
	if let Some(reason) = reason {
		crate::announce::announce(parts.conversations_parts.messages_announcer, reason);
		return true;
	}
	false
}

pub(crate) fn remove_quote(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	clear_pending_reply(&mut state.borrow_mut(), parts);
	parts.conversations_parts.conversation_panel.layout();
	crate::announce::announce(parts.conversations_parts.messages_announcer, "Quoted message removed.");
	parts.conversations_parts.message_field.set_focus();
}

/// Mirrors WhatsApp Web's WAWebPresenceChatAction: a keystroke in a non-empty
/// composer marks composing (sent once, then resent every
/// CHATSTATE_RESEND_EVERY while typing goes on), and CHATSTATE_PAUSE_AFTER
/// without a keystroke - or the composer emptying - sends "paused".
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

/// markComposing / markRecording: `media` empty for typing, "audio" while
/// recording a voice note.
pub(crate) fn mark_composing(s: &mut AppState, jid: &str, media: &str) {
	if !chatstate_allowed(jid) {
		return;
	}
	let now = std::time::Instant::now();
	match s.composing.as_mut() {
		Some(c) if c.jid == jid && c.media == media => c.last_key = now,
		_ => {
			// A different chat (or typing <-> recording): take the old one back first.
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

/// markPaused: only a chatstate we actually sent needs taking back.
pub(crate) fn mark_paused(s: &mut AppState, jid: &str) {
	if s.composing.as_ref().is_some_and(|c| c.jid == jid) {
		send_chatstate(s, jid, "paused", "");
	}
}

/// Sends one chatstate. A "paused" also clears the tracked state, whoever sends it.
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

/// Periodic timer: WA Web's paused timer (typing only - recording has none)
/// and its resend timer, which only resends while the contact is online.
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

/// WA Web's presenceOnlineChanged: a contact coming online while we're still
/// typing to them gets the chatstate straight away.
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
	crate::announce::announce(parts.conversations_parts.messages_announcer, "Message forwarded.");
}

/// Saves an edit typed into the composer's edit mode. A trimmed-empty or
/// unchanged-from-original result is treated as an implicit Cancel (matching
/// the single-line edit dialog this replaced, which just closed with no
/// action for either) rather than sent as a no-op edit.
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
		"Editing cancelled - message can't be empty."
	} else if original.as_deref() == Some(new_text.as_str()) {
		"Editing cancelled - no changes made."
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
		"Message edited."
	};
	end_edit_mode(parts);
	crate::announce::announce(parts.conversations_parts.messages_announcer, announcement);
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
	crate::announce::announce(
		parts.conversations_parts.messages_announcer,
		"Exporting chat, this may take a while for chats with a lot of media...",
	);
}

pub(crate) fn export_chat_text(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String, dest_path: String) {
	let payload = serde_json::json!({
		"action": "export_chat_text",
		"data": { "jid": jid, "dest_path": dest_path }
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	crate::announce::announce(parts.conversations_parts.messages_announcer, "Exporting chat text...");
}

pub(crate) fn post_status(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, text: String) {
	let payload = serde_json::json!({
		"action": "post_status",
		"data": { "text": text }
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	crate::announce::announce(parts.conversations_parts.messages_announcer, "Posting status update...");
}

pub(crate) fn post_media_status(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let file_dialog = FileDialog::builder(&parts.frame)
		.with_message("Choose a photo or video to post as a status")
		.with_wildcard(&whatsnap::AttachKind::Media.file_filter())
		.with_style(FileDialogStyle::Open | FileDialogStyle::FileMustExist)
		.build();
	if file_dialog.show_modal() == wxdragon::id::ID_OK
		&& let Some(path) = file_dialog.get_path()
	{
		// The same photos and videos the attach menu sends; AVI, MKV and
		// WebM don't play on phones, and a BMP isn't a photo to WhatsApp.
		let media_type = match whatsnap::AttachKind::Media.send_type(&path) {
			"video" => "video",
			_ => "image",
		};
		let filename = std::path::Path::new(&path).file_name().unwrap_or_default().to_string_lossy().to_string();

		let dlg =
			wxdragon::TextEntryDialog::builder(&parts.frame, "Add a caption (optional):", "Post Photo/Video Status")
				.build();
		let caption =
			if dlg.show_modal() == wxdragon::id::ID_OK { dlg.get_value().unwrap_or_default() } else { String::new() };

		let mut payload = serde_json::json!({
			"action": "post_media_status",
			"data": { "type": media_type, "path": path, "filename": filename, "caption": caption }
		});
		let ws_tx = state.borrow().ws_tx.clone();
		if media_type == "video" {
			// A preview of the first frame, as for a video sent to a chat.
			std::thread::spawn(move || {
				if let Some(png) = crate::video_thumbnail::first_frame_png(&path) {
					payload["data"]["thumbnail"] = serde_json::json!(crate::util::base64(&png));
				}
				let _ = ws_tx.send(payload.to_string());
			});
		} else {
			let _ = ws_tx.send(payload.to_string());
		}
		crate::announce::announce(parts.conversations_parts.messages_announcer, "Posting status update...");
	}
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
	// The toolbar's Retry send button: unlike the context menu's single
	// RetryMessage, this retries every failed item of an album.
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
		crate::announce::announce(parts.conversations_parts.messages_announcer, "Retrying send...");
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
		crate::announce::announce(
			parts.conversations_parts.messages_announcer,
			"Requested this message from your phone",
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
