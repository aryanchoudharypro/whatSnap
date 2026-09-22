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
	let mut s = state.borrow_mut();
	if let Some(ref jid) = s.selected_chat.clone() {
		// Sending a reply means the unread messages have obviously been
		// seen, so drop the "N unread messages" divider from the list
		// right away rather than waiting for the next time the chat is
		// opened.
		clear_unread_divider_for_open_chat(&mut s, parts, jid);

		let reply_id = s.pending_reply_id.take().unwrap_or_default();
		let reply_sender = s.pending_reply_sender.take().unwrap_or_default();
		s.pending_reply_preview = None;

		let payload = serde_json::json!({
			"action": "send",
			"data": {
				"to": jid,
				"text": text,
				"reply_to_id": reply_id,
				"reply_to_sender": reply_sender
			}
		});
		let _ = s.ws_tx.send(payload.to_string());
		s.draft_messages.remove(jid);
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
	drop(s);
	end_edit_mode(parts);
	if was_editing {
		live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Editing cancelled.");
	}
}

/// Puts the composer into edit mode for `message_id`: fills `message_field`
/// with its current text (selected, so typing immediately replaces it),
/// swaps the composer's label to "Edit message", and shows the "Cancel
/// edit" button in place of "Add attachment" - editing an existing message
/// can't also attach new media, and hiding that button puts "Cancel edit"
/// right after the field in tab order (forward *and* shift+tab both reach
/// it in one step, mirroring the single-line edit dialog this replaced).
pub(crate) fn begin_edit_message(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	message_id: String,
	text: String,
) {
	let mut s = state.borrow_mut();
	s.editing_message_id = Some(message_id);
	s.editing_message_original_text = Some(text.clone());
	drop(s);
	let field = &parts.conversations_parts.message_field;
	field.set_value(&text);
	field.set_selection(0, text.chars().count() as i64);
	parts.conversations_parts.message_label.set_label("Edit message");
	parts.conversations_parts.add_attachment_btn.show(false);
	parts.conversations_parts.cancel_edit_btn.show(true);
	parts.conversations_parts.conversation_panel.layout();
	field.set_focus();
	live_region::announce(
		parts.conversations_parts.messages_announcer.clone(),
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

pub(crate) fn remove_quote(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	clear_pending_reply(&mut state.borrow_mut(), parts);
	parts.conversations_parts.conversation_panel.layout();
	live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Quoted message removed.");
	parts.conversations_parts.message_field.set_focus();
}

pub(crate) fn send_typing(
	state: &Rc<RefCell<AppState>>,
	_parts: &Rc<WindowParts>,
	jid: String,
	typing_state: String,
	media: String,
) {
	let target_jid = if jid.is_empty() { state.borrow().selected_chat.clone() } else { Some(jid) };
	if let Some(target) = target_jid {
		let payload = serde_json::json!({
			"action": "send_typing",
			"data": {
				"jid": target,
				"state": typing_state,
				"media": media
			}
		});
		let _ = state.borrow().ws_tx.send(payload.to_string());
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
	live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Message forwarded.");
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
					"new_text": new_text
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		"Message edited."
	};
	end_edit_mode(parts);
	live_region::announce(parts.conversations_parts.messages_announcer.clone(), announcement);
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
	live_region::announce(
		parts.conversations_parts.messages_announcer.clone(),
		"Exporting chat, this may take a while for chats with a lot of media...",
	);
}

pub(crate) fn export_chat_text(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String, dest_path: String) {
	let payload = serde_json::json!({
		"action": "export_chat_text",
		"data": { "jid": jid, "dest_path": dest_path }
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Exporting chat text...");
}

pub(crate) fn post_status(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, text: String) {
	let payload = serde_json::json!({
		"action": "post_status",
		"data": { "text": text }
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Posting status update...");
}

pub(crate) fn post_media_status(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let file_dialog = FileDialog::builder(&parts.frame)
		.with_message("Choose a photo or video to post as a status")
		.with_wildcard("Photos and videos|*.jpg;*.jpeg;*.png;*.gif;*.webp;*.bmp;*.mp4;*.avi;*.mov;*.mkv;*.webm;*.3gp")
		.with_style(FileDialogStyle::Open | FileDialogStyle::FileMustExist)
		.build();
	if file_dialog.show_modal() == wxdragon::id::ID_OK {
		if let Some(path) = file_dialog.get_path() {
			let ext = std::path::Path::new(&path).extension().unwrap_or_default().to_string_lossy().to_lowercase();
			let media_type = match ext.as_str() {
				"mp4" | "avi" | "mov" | "mkv" | "webm" | "3gp" => "video",
				_ => "image",
			};
			let filename = std::path::Path::new(&path).file_name().unwrap_or_default().to_string_lossy().to_string();

			let dlg = wxdragon::TextEntryDialog::builder(
				&parts.frame,
				"Add a caption (optional):",
				"Post Photo/Video Status",
			)
			.build();
			let caption = if dlg.show_modal() == wxdragon::id::ID_OK {
				dlg.get_value().unwrap_or_default()
			} else {
				String::new()
			};

			let payload = serde_json::json!({
				"action": "post_media_status",
				"data": { "type": media_type, "path": path, "filename": filename, "caption": caption }
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
			live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Posting status update...");
		}
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
			whantv::DisplayItem::Single(m) => std::slice::from_ref(m),
			whantv::DisplayItem::Album { messages, .. } => messages.as_slice(),
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
		live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Retrying send...");
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
		live_region::announce(
			parts.conversations_parts.messages_announcer.clone(),
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
