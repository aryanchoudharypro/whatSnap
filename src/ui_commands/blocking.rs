//! Blocking contacts, as in WhatsApp: block or unblock a chat's contact from
//! its menus, and see everyone blocked under Account > Blocked contacts.
//! WhatsApp keeps the list by LID while chats mostly go by phone JID, so the
//! backend sends every address each person is blocked under and any of them
//! matches.

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState, ContactPickerMode,
	ui::{blocked_contacts::BlockedEntry, window::WindowParts},
};

pub(crate) fn is_blocked(s: &AppState, jid: &str) -> bool {
	s.blocked_jids.contains(jid)
}

/// Only people can be blocked: not groups, channels or Meta AI.
fn can_block(jid: &str) -> bool {
	jid.ends_with("@s.whatsapp.net") || jid.ends_with("@lid")
}

fn chat_name(state: &AppState, jid: &str) -> String {
	state.contacts.get(jid).cloned().unwrap_or_else(|| jid.to_string())
}

/// Speaks in the Blocked contacts dialog while it's open (the main window's
/// live region is behind it then), otherwise in the main window.
fn announce(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, message: &str) {
	let dialog = state.borrow().blocked_contacts.clone();
	match dialog {
		Some(dialog) => dialog.announce(message),
		None => crate::announce::announce(parts.conversations_parts.messages_announcer.clone(), message),
	}
}

fn send(state: &Rc<RefCell<AppState>>, action: &str, jid: &str) {
	let payload = serde_json::json!({ "action": action, "data": { "jid": jid } });
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

/// Asks before blocking, since it stops their messages and calls.
fn block_with_confirm(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: &str) {
	let name = chat_name(&state.borrow(), jid);
	let question = format!("Block {name}? They won't be able to call you or send you messages. They won't be told.");
	let dialog = state.borrow().blocked_contacts.clone();
	let answer = match dialog {
		Some(dialog) => MessageDialog::builder(&dialog.dialog, &question, "Block contact")
			.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
			.build()
			.show_modal(),
		None => MessageDialog::builder(&parts.frame, &question, "Block contact")
			.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
			.build()
			.show_modal(),
	};
	if answer != ID_YES {
		return;
	}
	send(state, "block_contact", jid);
	announce(state, parts, &format!("Blocking {name}..."));
}

/// "Block" / "Unblock" on a chat.
pub(crate) fn toggle_block(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String) {
	if !can_block(&jid) {
		announce(state, parts, "Only contacts can be blocked.");
		return;
	}
	if is_blocked(&state.borrow(), &jid) {
		let name = chat_name(&state.borrow(), &jid);
		send(state, "unblock_contact", &jid);
		announce(state, parts, &format!("Unblocking {name}..."));
	} else {
		block_with_confirm(state, parts, &jid);
	}
}

/// Someone picked in the contact picker opened from "Block a contact...".
pub(crate) fn block_picked_contact(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String) {
	if is_blocked(&state.borrow(), &jid) {
		let name = chat_name(&state.borrow(), &jid);
		announce(state, parts, &format!("{name} is already blocked."));
		return;
	}
	block_with_confirm(state, parts, &jid);
}

/// The backend's list, sent when it changes.
pub(crate) fn blocklist_received(state: &Rc<RefCell<AppState>>, data: &serde_json::Value) {
	let text = |v: &serde_json::Value, key: &str| v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string();
	let mut jids = std::collections::HashSet::new();
	let mut entries = Vec::new();
	for e in data.get("entries").and_then(|e| e.as_array()).into_iter().flatten() {
		let jid = text(e, "jid");
		if jid.is_empty() {
			continue;
		}
		for alt in e.get("alts").and_then(|a| a.as_array()).into_iter().flatten() {
			if let Some(alt) = alt.as_str() {
				jids.insert(alt.to_string());
			}
		}
		jids.insert(jid.clone());
		entries.push(BlockedEntry { jid, name: text(e, "name"), phone: text(e, "phone") });
	}
	let dialog = {
		let mut s = state.borrow_mut();
		s.blocked_jids = jids;
		s.blocked_entries = entries.clone();
		s.blocked_contacts.clone()
	};
	if let Some(dialog) = dialog {
		dialog.apply(&entries);
	}
}

/// The backend's answer to a block or unblock.
pub(crate) fn block_result(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let name = data.get("name").and_then(|n| n.as_str()).unwrap_or("Contact");
	let blocked = data.get("blocked").and_then(|b| b.as_bool()).unwrap_or(false);
	let Some(error) = data.get("error").and_then(|e| e.as_str()).filter(|e| !e.is_empty()) else {
		announce(state, parts, &format!("{name} {}.", if blocked { "blocked" } else { "unblocked" }));
		return;
	};
	let message = format!("Couldn't {} {name}: {error}", if blocked { "block" } else { "unblock" });
	let dialog = state.borrow().blocked_contacts.clone();
	let style = MessageDialogStyle::OK | MessageDialogStyle::IconError;
	match dialog {
		Some(dialog) => {
			MessageDialog::builder(&dialog.dialog, &message, "Error").with_style(style).build().show_modal()
		}
		None => MessageDialog::builder(&parts.frame, &message, "Error").with_style(style).build().show_modal(),
	};
}

/// Account > "Blocked contacts...".
pub(crate) fn show_blocked_contacts(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let (ws_tx, entries) = {
		let s = state.borrow();
		(s.ws_tx.clone(), s.blocked_entries.clone())
	};
	// Blocks made on another device since the last connect show up once the
	// server answers.
	let _ = ws_tx.send(serde_json::json!({ "action": "get_blocklist" }).to_string());
	let (state_add, parts_add) = (state.clone(), parts.clone());
	let state_store = state.clone();
	crate::ui::blocked_contacts::show_blocked_contacts(
		&parts.frame,
		&entries,
		ws_tx,
		move || {
			crate::ui_commands::contacts_groups::prepare_contact_picker(
				&state_add,
				&parts_add,
				ContactPickerMode::BlockContact,
			);
		},
		move |dialog| state_store.borrow_mut().blocked_contacts = dialog,
	);
}

/// Called before sending to or calling the open chat. WhatsApp won't deliver
/// to someone you've blocked, so, like the official apps, offer to unblock
/// them instead. Returns true when the send should stop here.
pub(crate) fn stop_if_blocked(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) -> bool {
	let (jid, name) = {
		let s = state.borrow();
		let Some(jid) = s.selected_chat.clone().filter(|jid| is_blocked(&s, jid)) else {
			return false;
		};
		let name = chat_name(&s, &jid);
		(jid, name)
	};
	let question = format!("You blocked {name}. Unblock them to message or call them?");
	let answer = MessageDialog::builder(&parts.frame, &question, "Blocked contact")
		.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
		.build()
		.show_modal();
	if answer == ID_YES {
		send(state, "unblock_contact", &jid);
		announce(state, parts, &format!("Unblocking {name}..."));
	}
	true
}

/// The line Chat Info adds for a blocked contact.
pub(crate) fn info_line(s: &AppState, jid: &str) -> &'static str {
	if is_blocked(s, jid) { "Blocked: Yes\n" } else { "" }
}
