use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{AppState, ui::window::WindowParts};

const GO_TO: i32 = 4401;
const COPY: i32 = 4402;
const FORWARD: i32 = 4403;
const UNSTAR: i32 = 4404;
const UNSTAR_ALL: i32 = 4405;

pub(crate) fn unstar_message_at(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, idx: usize) {
	let entry = {
		let s = state.borrow();
		let Some(id) = s.filtered_starred_message_ids.get(idx).filter(|id| !id.is_empty()) else { return };
		let Some(entry) = s.starred_entries.iter().find(|e| &e.message_id == id).cloned() else { return };
		entry
	};
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
	let _ = state.borrow().ws_tx.send(payload.to_string());
	{
		let mut s = state.borrow_mut();
		s.starred_entries.retain(|e| e.message_id != entry.message_id);
		let target_id = entry.message_id.clone();
		if let Some(di) = crate::ui::message_display::update_loaded_message(&mut s, &target_id, |m| m.starred = false)
			&& let Some(item) = s.display_items.get(di)
		{
			let is_group = entry.jid.ends_with("@g.us");
			let updated_display = item.format_display(is_group);
			parts.conversations_parts.messages_list.set_string(di as u32, &updated_display);
		}
		crate::ui::message_display::apply_chat_filter(&mut s, parts);
	}
	crate::platform::announce::announce(
		parts.conversations_parts.messages_announcer,
		crate::i18n::tr("Message unstarred."),
	);
}

pub(crate) fn copy_message_at(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, idx: usize) {
	let entry = {
		let s = state.borrow();
		let Some(id) = s.filtered_starred_message_ids.get(idx).filter(|id| !id.is_empty()) else { return };
		let Some(entry) = s.starred_entries.iter().find(|e| &e.message_id == id).cloned() else { return };
		entry
	};
	if !entry.text.is_empty() {
		if crate::platform::clipboard::copy_text(&entry.text) {
			crate::platform::announce::announce(
				parts.conversations_parts.messages_announcer,
				crate::i18n::tr("Message text copied to clipboard."),
			);
		} else {
			crate::platform::announce::announce(
				parts.conversations_parts.messages_announcer,
				crate::i18n::tr("Couldn't copy to clipboard."),
			);
		}
	} else {
		crate::platform::announce::announce(
			parts.conversations_parts.messages_announcer,
			crate::i18n::tr("No text to copy."),
		);
	}
}

pub(crate) fn show_starred_menu(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, idx: usize) {
	let (entry, total_starred) = {
		let s = state.borrow();
		let Some(id) = s.filtered_starred_message_ids.get(idx).filter(|id| !id.is_empty()) else { return };
		let Some(entry) = s.starred_entries.iter().find(|e| &e.message_id == id).cloned() else { return };
		(entry, s.starred_entries.len())
	};

	let mut builder = Menu::builder();
	builder =
		builder.append_item(GO_TO, crate::i18n::tr("&Go to message"), crate::i18n::tr("Open the chat at this message"));
	if !entry.text.is_empty() {
		builder = builder.append_item(
			COPY,
			crate::i18n::tr("&Copy text	Ctrl+C"),
			crate::i18n::tr("Copy message text to clipboard"),
		);
	}
	builder = builder
		.append_item(
			FORWARD,
			crate::i18n::tr("&Forward..."),
			crate::i18n::tr("Forward this message to contacts or groups"),
		)
		.append_item(
			UNSTAR,
			crate::i18n::tr("&Unstar	Del"),
			crate::i18n::tr("Take this message off your starred messages"),
		);
	if total_starred > 1 {
		builder = builder.append_separator().append_item(
			UNSTAR_ALL,
			crate::i18n::tr("Unstar &all..."),
			crate::i18n::tr("Remove all messages from starred messages"),
		);
	}
	let mut menu = builder.build();

	let state_menu = state.clone();
	let parts_menu = parts.clone();
	let entry_forward = entry.clone();
	menu.on_selected(move |evt| match evt.get_id() {
		GO_TO => super::chat_list::open_conversation(&state_menu, &parts_menu, idx),
		COPY => copy_message_at(&state_menu, &parts_menu, idx),
		FORWARD => {
			let targets = crate::ui_commands::links::send_targets(&state_menu.borrow());
			let Some(to_jids) = crate::ui::dialogs::send_to::show_forward_dialog(
				&parts_menu.frame,
				crate::i18n::tr("Forward message to"),
				targets,
			) else {
				return;
			};
			if to_jids.is_empty() {
				return;
			}
			let payload = serde_json::json!({
				"action": "forward_message",
				"data": { "message_ids": vec![entry_forward.message_id.clone()], "to_jids": to_jids }
			});
			let _ = state_menu.borrow().ws_tx.send(payload.to_string());
			let names: Vec<String> = {
				let s = state_menu.borrow();
				to_jids.iter().map(|jid| s.contacts.get(jid).cloned().unwrap_or_else(|| jid.clone())).collect()
			};
			let names_joined = names.join(", ");
			let spoken = crate::i18n::tr_fmt("Message forwarded to {}.", &[&names_joined]);
			crate::platform::announce::announce(parts_menu.conversations_parts.messages_announcer, &spoken);
		}
		UNSTAR => unstar_message_at(&state_menu, &parts_menu, idx),
		UNSTAR_ALL => {
			let count = state_menu.borrow().starred_entries.len();
			if count == 0 {
				return;
			}
			let question = crate::i18n::tr_fmt("Are you sure you want to unstar all {} starred messages?", &[&count]);
			let confirm = MessageDialog::builder(&parts_menu.frame, &question, crate::i18n::tr("Unstar All Messages"))
				.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
				.build();
			if confirm.show_modal() == wxdragon::id::ID_YES {
				let all_entries = state_menu.borrow().starred_entries.clone();
				for e in &all_entries {
					let payload = serde_json::json!({
						"action": "star_message",
						"data": {
							"jid": e.jid,
							"message_id": e.message_id,
							"sender_jid": if e.from_me { "me" } else { e.sender_jid.as_str() },
							"from_me": e.from_me,
							"starred": false
						}
					});
					let _ = state_menu.borrow().ws_tx.send(payload.to_string());
				}
				{
					let mut s = state_menu.borrow_mut();
					s.starred_entries.clear();
					crate::ui::message_display::apply_chat_filter(&mut s, &parts_menu);
				}
				crate::platform::announce::announce(
					parts_menu.conversations_parts.messages_announcer,
					crate::i18n::tr("All starred messages unstarred."),
				);
			}
		}
		_ => {}
	});
	parts.conversations_parts.conversations_list.popup_menu(&mut menu, None);
	menu.destroy_menu();
	crate::platform::tray::enter_release_guard().set(true);
}
