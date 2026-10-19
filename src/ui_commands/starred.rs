//! The Starred Messages tab's context menu. Its rows are messages, not
//! chats, so they get message actions instead of the chat list's.

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{AppState, ui::window::WindowParts};

const GO_TO: i32 = 4401;
const UNSTAR: i32 = 4402;

pub(crate) fn show_starred_menu(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, idx: usize) {
	let entry = {
		let s = state.borrow();
		let Some(id) = s.filtered_starred_message_ids.get(idx) else { return };
		let Some(entry) = s.starred_entries.iter().find(|e| &e.message_id == id).cloned() else { return };
		entry
	};
	let mut menu = Menu::builder()
		.append_item(GO_TO, "&Go to message", "Open the chat at this message")
		.append_item(UNSTAR, "&Unstar", "Take this message off your starred messages")
		.build();
	let state_menu = state.clone();
	let parts_menu = parts.clone();
	menu.on_selected(move |evt| match evt.get_id() {
		GO_TO => super::chat_list::open_conversation(&state_menu, &parts_menu, idx),
		UNSTAR => {
			let payload = serde_json::json!({
				"action": "star_message",
				"data": {
					"jid": entry.jid,
					"message_id": entry.message_id,
					"sender_jid": if entry.from_me { "me" } else { entry.sender_jid.as_str() },
					"from_me": entry.from_me,
					"starred": false
				}
			});
			let _ = state_menu.borrow().ws_tx.send(payload.to_string());
			crate::announce::announce(parts_menu.conversations_parts.messages_announcer, "Message unstarred.");
		}
		_ => {}
	});
	parts.conversations_parts.conversations_list.popup_menu(&mut menu, None);
	menu.destroy_menu();
	crate::tray::enter_release_guard().set(true);
}
