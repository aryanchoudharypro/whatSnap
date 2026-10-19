//! The attach menu's sticker picker: favorite stickers (synced from the
//! phone), stickers recently seen in chats, or a .webp file. The backend
//! lists them (`stickers`) and says when a favorite changes
//! (`sticker_favorite`).

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{AppState, message_display::update_loaded_message, ui::window::WindowParts};

fn announce(parts: &WindowParts, message: &str) {
	crate::announce::announce(parts.conversations_parts.messages_announcer, message);
}

/// The pending reply, taken for a sticker going out now.
fn take_reply(state: &Rc<RefCell<AppState>>, parts: &WindowParts) -> (String, String) {
	let mut s = state.borrow_mut();
	let reply_id = s.pending_reply_id.take().unwrap_or_default();
	let reply_sender = s.pending_reply_sender.take().unwrap_or_default();
	s.pending_reply_preview = None;
	parts.conversations_parts.remove_quote_btn.show(false);
	(reply_id, reply_sender)
}

/// Attach menu > "Sticker": ask the backend which stickers there are.
pub(crate) fn open_sticker_picker(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	if crate::ui_commands::blocking::stop_if_cant_send(state, parts) || state.borrow().selected_chat.is_none() {
		return;
	}
	let payload = serde_json::json!({ "action": "get_stickers" });
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

enum Pick {
	Favorite(String),
	Recent(String),
	File,
}

/// The backend listed the stickers: pick one and send it.
pub(crate) fn stickers(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let Some(jid) = state.borrow().selected_chat.clone() else { return };
	let list = |key: &str| data.get(key).and_then(|v| v.as_array()).cloned().unwrap_or_default();
	let field = |v: &serde_json::Value, key: &str| v.get(key).and_then(|f| f.as_str()).unwrap_or("").to_string();
	let mut picks = Vec::new();
	let mut labels = Vec::new();
	for f in list("favorites") {
		picks.push(Pick::Favorite(field(&f, "hash")));
		labels.push(format!("Favorite: {}", field(&f, "label")));
	}
	for r in list("recent") {
		picks.push(Pick::Recent(field(&r, "message_id")));
		labels.push(format!("Recent: {}", field(&r, "label")));
	}
	picks.push(Pick::File);
	labels.push("Choose a sticker file (.webp)...".to_string());
	let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
	let dialog = SingleChoiceDialog::builder(&parts.frame, "Send a sticker:", "Stickers", &label_refs).build();
	if dialog.show_modal() != wxdragon::id::ID_OK {
		return;
	}
	let Some(pick) = usize::try_from(dialog.get_selection()).ok().and_then(|i| picks.get(i)) else { return };
	let payload = match pick {
		Pick::File => {
			let file_dialog = FileDialog::builder(&parts.frame)
				.with_message("Choose a sticker to send")
				.with_wildcard("Stickers (*.webp)|*.webp")
				.with_style(FileDialogStyle::Open | FileDialogStyle::FileMustExist)
				.build();
			if file_dialog.show_modal() != wxdragon::id::ID_OK {
				return;
			}
			let Some(path) = file_dialog.get_path() else { return };
			let filename = std::path::Path::new(&path).file_name().unwrap_or_default().to_string_lossy().to_string();
			let (reply_id, reply_sender) = take_reply(state, parts);
			serde_json::json!({ "action": "send_attachment", "data": {
				"jid": jid, "type": "sticker", "path": path, "filename": filename, "caption": "",
				"reply_to_id": reply_id, "reply_to_sender": reply_sender,
			}})
		}
		Pick::Favorite(hash) => {
			let (reply_id, reply_sender) = take_reply(state, parts);
			serde_json::json!({ "action": "send_sticker", "data": {
				"jid": jid, "hash": hash, "reply_to_id": reply_id, "reply_to_sender": reply_sender,
			}})
		}
		Pick::Recent(message_id) => {
			let (reply_id, reply_sender) = take_reply(state, parts);
			serde_json::json!({ "action": "send_sticker", "data": {
				"jid": jid, "message_id": message_id, "reply_to_id": reply_id, "reply_to_sender": reply_sender,
			}})
		}
	};
	let _ = state.borrow().ws_tx.send(payload.to_string());
	announce(parts, "Sending sticker...");
}

/// The message menu's favorite item: add a sticker in a chat to the
/// favorites, or take it off.
pub(crate) fn set_favorite(state: &Rc<RefCell<AppState>>, message_id: &str, favorite: bool) {
	let payload = serde_json::json!({ "action": "set_sticker_favorite", "data": { "message_id": message_id, "favorite": favorite } });
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

/// The backend changed a favorite.
pub(crate) fn sticker_favorite(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let favorite = data.get("favorite").and_then(|v| v.as_bool()).unwrap_or(false);
	let message_id = data.get("message_id").and_then(|v| v.as_str()).unwrap_or("");
	update_loaded_message(&mut state.borrow_mut(), message_id, |m| m.sticker_favorite = favorite);
	announce(parts, if favorite { "Added to favorite stickers." } else { "Removed from favorite stickers." });
}
