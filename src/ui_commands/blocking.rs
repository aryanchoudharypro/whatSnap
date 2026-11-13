use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState, ContactPickerMode,
	ui::{dialogs::blocked_contacts::BlockedEntry, window::WindowParts},
};

pub(crate) fn is_blocked(s: &AppState, jid: &str) -> bool {
	s.blocked_jids.contains(jid)
}

fn can_block(jid: &str) -> bool {
	jid.ends_with("@s.whatsapp.net") || jid.ends_with("@lid")
}

fn chat_name(state: &AppState, jid: &str) -> String {
	state.contacts.get(jid).cloned().unwrap_or_else(|| jid.to_string())
}

fn announce(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, message: &str) {
	let dialog = state.borrow().blocked_contacts.clone();
	match dialog {
		Some(dialog) => dialog.announce(message),
		None => crate::platform::announce::announce(parts.conversations_parts.messages_announcer, message),
	}
}

fn send(state: &Rc<RefCell<AppState>>, action: &str, jid: &str) {
	let payload = serde_json::json!({ "action": action, "data": { "jid": jid } });
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

fn block_with_confirm(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: &str) {
	let name = chat_name(&state.borrow(), jid);
	let question = crate::i18n::tr_fmt(
		"Block {}? They won't be able to call you or send you messages. They won't be told.",
		&[&name],
	);
	let title = crate::i18n::tr("Block contact");
	let dialog = state.borrow().blocked_contacts.clone();
	let answer = match dialog {
		Some(dialog) => MessageDialog::builder(&dialog.dialog, &question, title)
			.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
			.build()
			.show_modal(),
		None => MessageDialog::builder(&parts.frame, &question, title)
			.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
			.build()
			.show_modal(),
	};
	if answer != ID_YES {
		return;
	}
	send(state, "block_contact", jid);
	let msg = crate::i18n::tr_fmt("Blocking {}...", &[&name]);
	announce(state, parts, &msg);
}

pub(crate) fn toggle_block(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String) {
	if !can_block(&jid) {
		announce(state, parts, crate::i18n::tr("Only contacts can be blocked."));
		return;
	}
	if is_blocked(&state.borrow(), &jid) {
		let name = chat_name(&state.borrow(), &jid);
		send(state, "unblock_contact", &jid);
		let msg = crate::i18n::tr_fmt("Unblocking {}...", &[&name]);
		announce(state, parts, &msg);
	} else {
		block_with_confirm(state, parts, &jid);
	}
}

pub(crate) fn block_picked_contact(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String) {
	if is_blocked(&state.borrow(), &jid) {
		let name = chat_name(&state.borrow(), &jid);
		let msg = crate::i18n::tr_fmt("{} is already blocked.", &[&name]);
		announce(state, parts, &msg);
		return;
	}
	block_with_confirm(state, parts, &jid);
}

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

pub(crate) fn block_result(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
	let name = data.get("name").and_then(|n| n.as_str()).unwrap_or_else(|| crate::i18n::tr("Contact"));
	let blocked = data.get("blocked").and_then(|b| b.as_bool()).unwrap_or(false);
	let Some(error) = data.get("error").and_then(|e| e.as_str()).filter(|e| !e.is_empty()) else {
		if !jid.is_empty() {
			let mut s = state.borrow_mut();
			if blocked {
				s.blocked_jids.insert(jid.to_string());
				if !s.blocked_entries.iter().any(|e| e.jid == jid) {
					s.blocked_entries.push(BlockedEntry {
						jid: jid.to_string(),
						name: name.to_string(),
						phone: String::new(),
					});
				}
			} else {
				s.blocked_jids.remove(jid);
				s.blocked_entries.retain(|e| e.jid != jid);
			}
		}
		{
			let s = state.borrow();
			if s.selected_chat.as_deref() == Some(jid) {
				crate::ui_commands::calls::apply_call_buttons(&s, parts, jid);
			}
		}
		let msg = crate::i18n::tr_fmt(if blocked { "{} blocked." } else { "{} unblocked." }, &[&name]);
		announce(state, parts, &msg);
		return;
	};
	let err_text = crate::i18n::translate_backend_message(error);
	let message = crate::i18n::tr_fmt(
		if blocked { "Couldn't block {}: {}" } else { "Couldn't unblock {}: {}" },
		&[&name, &err_text],
	);
	let title = crate::i18n::tr("Error");
	let dialog = state.borrow().blocked_contacts.clone();
	let style = MessageDialogStyle::OK | MessageDialogStyle::IconError;
	match dialog {
		Some(dialog) => {
			let owner = windows::Win32::Foundation::HWND(dialog.dialog.get_handle());
			crate::platform::focus::show_modal_and_refocus(owner, false, || {
				MessageDialog::builder(&dialog.dialog, &message, title).with_style(style).build().show_modal()
			});
		}
		None => {
			let owner = windows::Win32::Foundation::HWND(parts.frame.get_handle());
			crate::platform::focus::show_modal_and_refocus(owner, false, || {
				MessageDialog::builder(&parts.frame, &message, title).with_style(style).build().show_modal()
			});
		}
	};
}

pub(crate) fn show_blocked_contacts(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let (ws_tx, entries) = {
		let s = state.borrow();
		(s.ws_tx.clone(), s.blocked_entries.clone())
	};
	let _ = ws_tx.send(serde_json::json!({ "action": "get_blocklist" }).to_string());
	let (state_add, parts_add) = (state.clone(), parts.clone());
	let state_store = state.clone();
	let add = move || {
		crate::ui_commands::contacts_groups::prepare_contact_picker(
			&state_add,
			&parts_add,
			ContactPickerMode::BlockContact,
		);
	};
	let store = move |dialog| state_store.borrow_mut().blocked_contacts = dialog;
	// Opened from Settings, it goes over Settings, so focus comes back there.
	let settings = parts.settings_dialog.dialog;
	if settings.is_shown() {
		crate::ui::dialogs::blocked_contacts::show_blocked_contacts(&settings, &entries, ws_tx, add, store);
	} else {
		crate::ui::dialogs::blocked_contacts::show_blocked_contacts(&parts.frame, &entries, ws_tx, add, store);
	}
}

pub(crate) fn stop_if_blocked(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) -> bool {
	let Some(jid) = state.borrow().selected_chat.clone() else { return false };
	stop_if_blocked_jid(state, parts, &jid)
}

pub(crate) fn stop_if_blocked_jid(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: &str) -> bool {
	let name = {
		let s = state.borrow();
		if !is_blocked(&s, jid) {
			return false;
		}
		chat_name(&s, jid)
	};
	let question = crate::i18n::tr_fmt("You blocked {}. Unblock them to message or call them?", &[&name]);
	let answer = MessageDialog::builder(&parts.frame, &question, crate::i18n::tr("Blocked contact"))
		.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
		.build()
		.show_modal();
	if answer == ID_YES {
		send(state, "unblock_contact", jid);
		let msg = crate::i18n::tr_fmt("Unblocking {}...", &[&name]);
		announce(state, parts, &msg);
	}
	true
}

pub(crate) fn stop_if_cant_send(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) -> bool {
	if stop_if_blocked(state, parts) {
		return true;
	}
	let reason = {
		let s = state.borrow();
		s.selected_chat.as_deref().and_then(|jid| crate::ui::chat_ui::group_send_block_reason(&s, jid))
	};
	if let Some(reason) = reason {
		announce(state, parts, reason);
		return true;
	}
	false
}

pub(crate) fn info_line(s: &AppState, jid: &str) -> &'static str {
	if is_blocked(s, jid) { crate::i18n::tr("Blocked: Yes\n") } else { "" }
}
