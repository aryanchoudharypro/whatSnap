//! Sending from the emoji and stickers panel (`ui::emoji_panel`): favorite
//! stickers (synced from the phone), stickers recently seen in chats,
//! WhatsApp's own sticker packs, or a .webp file; and favorites, whose
//! changes the backend reports (`sticker_favorite`).

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState,
	message_display::update_loaded_message,
	ui::{
		emoji_panel::{PanelChoice, StickerPick, show_emoji_panel},
		window::WindowParts,
	},
};

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

/// The emoji and stickers panel, from the button by the message box, the
/// Conversation menu or the attach menu's "Sticker": an emoji goes into the
/// message box, a sticker straight to the chat.
pub(crate) fn open_emoji_panel(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, stickers: bool) {
	if crate::ui_commands::blocking::stop_if_cant_send(state, parts) || state.borrow().selected_chat.is_none() {
		return;
	}
	let ws_tx = state.borrow().ws_tx.clone();
	let state_store = state.clone();
	let choice = show_emoji_panel(&parts.frame, ws_tx, stickers, move |panel| {
		state_store.borrow_mut().emoji_panel = panel;
	});
	let field = parts.conversations_parts.message_field;
	let Some(choice) = choice else {
		field.set_focus();
		return;
	};
	let Some(jid) = state.borrow().selected_chat.clone() else { return };
	let payload = match choice {
		PanelChoice::Emoji(emoji) => {
			field.write_text(&emoji);
			field.set_focus();
			return;
		}
		PanelChoice::StickerFile => {
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
		PanelChoice::Sticker(pick) => {
			let (reply_id, reply_sender) = take_reply(state, parts);
			let mut data = serde_json::json!({ "jid": jid, "reply_to_id": reply_id, "reply_to_sender": reply_sender });
			match pick {
				StickerPick::Favorite(hash) => data["hash"] = hash.into(),
				StickerPick::Recent(message_id) => data["message_id"] = message_id.into(),
				StickerPick::Store { pack_id, hash } => {
					data["pack_id"] = pack_id.into();
					data["hash"] = hash.into();
				}
			}
			serde_json::json!({ "action": "send_sticker", "data": data })
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
