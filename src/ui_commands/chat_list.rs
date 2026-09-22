//! `handle_ui_command` arms for the chat list and open conversation view (nav,
//! filtering, search, selection, per-chat pin/mute/archive/lock/read toggles).

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState,
	chat_ui::{clear_pending_reply, clear_unread_divider_for_open_chat, open_chat_for_jid, save_composer_draft},
	contact_picker::refresh_selection_list,
	message_display::{
		apply_chat_filter, build_document_messages, build_link_messages, build_media_messages, populate_message_list,
	},
	tray::refresh_tray_tooltip,
	ui::window::WindowParts,
};

pub(crate) fn nav_selected(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, idx: usize) {
	match idx {
		0 => {
			// Conversations
			parts.conversations_parts.panel.show(true);
			state.borrow_mut().active_tab = 0;
			parts.conversations_parts.new_conv_btn.set_label("New Conversation");
			parts.conversations_parts.post_media_status_btn.show(false);
			parts.conversations_parts.conversations_label.set_label("Conversations");
			apply_chat_filter(&mut state.borrow_mut(), parts);
		}
		1 => {
			// Status
			parts.conversations_parts.panel.show(true);
			state.borrow_mut().active_tab = 1;
			parts.conversations_parts.new_conv_btn.set_label("Post Status Update...");
			parts.conversations_parts.post_media_status_btn.show(true);
			parts.conversations_parts.conversations_label.set_label("Status Updates");
			let payload = serde_json::json!({ "action": "get_statuses" });
			let _ = state.borrow().ws_tx.send(payload.to_string());
			apply_chat_filter(&mut state.borrow_mut(), parts);
		}
		2 => {
			// Starred Messages
			parts.conversations_parts.panel.show(true);
			state.borrow_mut().active_tab = 2;
			parts.conversations_parts.new_conv_btn.set_label("New Conversation");
			parts.conversations_parts.post_media_status_btn.show(false);
			parts.conversations_parts.conversations_label.set_label("Starred Messages");
			let payload = serde_json::json!({ "action": "get_starred_messages" });
			let _ = state.borrow().ws_tx.send(payload.to_string());
			apply_chat_filter(&mut state.borrow_mut(), parts);
		}
		3 => {
			// Archived Chats
			parts.conversations_parts.panel.show(true);
			state.borrow_mut().active_tab = 3;
			parts.conversations_parts.new_conv_btn.set_label("New Conversation");
			parts.conversations_parts.post_media_status_btn.show(false);
			parts.conversations_parts.conversations_label.set_label("Archived Chats");
			apply_chat_filter(&mut state.borrow_mut(), parts);
		}
		4 => {
			// Communities
			parts.conversations_parts.panel.show(true);
			state.borrow_mut().active_tab = 4;
			parts.conversations_parts.new_conv_btn.set_label("New Conversation");
			parts.conversations_parts.post_media_status_btn.show(false);
			parts.conversations_parts.conversations_label.set_label("Communities");
			apply_chat_filter(&mut state.borrow_mut(), parts);
		}
		_ => {}
	}
	parts.content_panel.layout();
}

pub(crate) fn filter_conversations(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, query: String) {
	let mut s = state.borrow_mut();
	s.search_query = query;
	apply_chat_filter(&mut s, parts);
}

pub(crate) fn open_conversation(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, idx: usize) {
	if state.borrow().active_tab == 1 {
		// Status: selecting a row views that status instead of opening a chat.
		let (poster_jid, message_id) = {
			let s = state.borrow();
			(s.filtered_chats.get(idx).cloned(), s.filtered_status_message_ids.get(idx).cloned())
		};
		if let (Some(poster_jid), Some(message_id)) = (poster_jid, message_id) {
			let mut s = state.borrow_mut();
			let mut media_type = String::new();
			let mut text = String::new();
			let mut poster_name = poster_jid.clone();
			for group in s.status_groups.iter_mut() {
				if group.poster_jid == poster_jid {
					poster_name = group.poster_name.clone();
					if let Some(item) = group.items.iter_mut().find(|it| it.id == message_id) {
						item.viewed = true;
						media_type = item.media_type.clone();
						text = item.text.clone();
					}
					group.all_viewed = group.items.iter().all(|it| it.viewed);
					break;
				}
			}
			apply_chat_filter(&mut s, parts);
			let ws_tx = s.ws_tx.clone();
			drop(s);

			let mark_payload = serde_json::json!({
				"action": "mark_status_viewed",
				"data": { "id": message_id, "poster_jid": poster_jid }
			});
			let _ = ws_tx.send(mark_payload.to_string());

			if !media_type.is_empty() {
				let open_payload = serde_json::json!({
					"action": "open_media",
					"data": { "jid": "status@broadcast", "message_id": message_id }
				});
				let _ = ws_tx.send(open_payload.to_string());
				live_region::announce(
					parts.conversations_parts.messages_announcer.clone(),
					&format!("Opening {}'s status media...", poster_name),
				);
			} else {
				let msg = if text.is_empty() {
					format!("{}'s status has no content.", poster_name)
				} else {
					format!("{}: {}", poster_name, text)
				};
				live_region::announce(parts.conversations_parts.messages_announcer.clone(), &msg);
			}
		}
		return;
	}
	let jid = state.borrow().filtered_chats.get(idx).cloned();
	if let Some(ref jid) = jid {
		if state.borrow().active_tab == 2 {
			let msg_id = state.borrow().filtered_starred_message_ids.get(idx).cloned();
			if let Some(msg_id) = msg_id {
				state.borrow_mut().search_target_id = Some(msg_id);
			}
		}
		open_chat_for_jid(jid, state, parts);
	}
}

pub(crate) fn open_chat_by_phone(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, digits: String) {
	let jid = format!("{}@s.whatsapp.net", digits);
	open_chat_for_jid(&jid, state, parts);
}

pub(crate) fn open_chat_by_jid(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String) {
	open_chat_for_jid(&jid, state, parts);
}

pub(crate) fn toggle_selection_mode(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let mut s = state.borrow_mut();
	s.selection_mode = !s.selection_mode;
	if s.selection_mode {
		s.selected_message_ids.clear();
		refresh_selection_list(&mut s, parts);
		parts.conversations_parts.messages_list.show(false);
		parts.conversations_parts.messages_select_list.show(true);
		parts.conversations_parts.selection_bar_panel.show(true);
		parts.conversations_parts.message_field.show(false);
		parts.conversations_parts.add_attachment_btn.show(false);
		parts.conversations_parts.send_message_btn.show(false);
		parts.conversations_parts.record_voice_btn.show(false);
	} else {
		s.selected_message_ids.clear();
		s.selection_rows.clear();
		parts.conversations_parts.messages_select_list.show(false);
		parts.conversations_parts.selection_bar_panel.show(false);
		parts.conversations_parts.messages_list.show(true);
		parts.conversations_parts.message_field.show(true);
		parts.conversations_parts.add_attachment_btn.show(true);
		parts
			.conversations_parts
			.send_message_btn
			.show(!parts.conversations_parts.message_field.get_value().trim().is_empty());
		parts.conversations_parts.record_voice_btn.show(true);
	}
	parts.conversations_parts.conversation_panel.layout();
}

pub(crate) fn delete_selected_messages(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, for_everyone: bool) {
	let (jid_opt, ws_tx, to_delete) = {
		let s = state.borrow();
		let ids = &s.selected_message_ids;
		let msgs: Vec<whantv::ChatMessage> =
			s.selection_rows.iter().filter(|item| ids.contains(item.id())).flat_map(|item| item.messages()).collect();
		(s.selected_chat.clone(), s.ws_tx.clone(), msgs)
	};
	if let Some(jid) = jid_opt {
		{
			let mut s = state.borrow_mut();
			for msg in &to_delete {
				s.bulk_delete_pending.insert((jid.clone(), msg.id.clone()));
			}
		}
		for msg in &to_delete {
			let sender_jid = if msg.from_me { "me".to_string() } else { msg.sender_jid.clone() };
			let payload = serde_json::json!({
				"action": "delete_message",
				"data": {
					"jid": jid,
					"message_id": msg.id,
					"sender_jid": sender_jid,
					"for_everyone": for_everyone
				}
			});
			let _ = ws_tx.send(payload.to_string());
		}
	}

	let mut s = state.borrow_mut();
	s.selection_mode = false;
	s.selected_message_ids.clear();
	s.selection_rows.clear();
	drop(s);
	parts.conversations_parts.messages_select_list.show(false);
	parts.conversations_parts.selection_bar_panel.show(false);
	parts.conversations_parts.messages_list.show(true);
	parts.conversations_parts.message_field.show(true);
	parts.conversations_parts.add_attachment_btn.show(true);
	parts.conversations_parts.record_voice_btn.show(true);
	parts
		.conversations_parts
		.send_message_btn
		.show(!parts.conversations_parts.message_field.get_value().trim().is_empty());
	parts.conversations_parts.conversation_panel.layout();
	if !to_delete.is_empty() {
		live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Selected messages deleted.");
	}
}

pub(crate) fn load_more_messages(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>) {
	let mut s = state.borrow_mut();
	if s.history_loading {
		eprintln!("[LOG UI] Request already in flight for chat, skipping duplicate trigger");
		return;
	}
	let jid = match s.selected_chat.clone() {
		Some(jid) => jid,
		None => return,
	};
	if s.completed_history_chats.contains(&jid) {
		eprintln!("[LOG UI] All available history already loaded for chat {}, skipping request", jid);
		return;
	}
	s.history_loading = true;
	if s.no_more_messages {
		// Already know the local database has nothing older for this chat (a
		// previous page came back short) - go straight to the phone instead
		// of paying for another empty local query.
		request_phone_history_sync(&mut s, &jid);
	} else {
		let offset = whantv::real_message_count(&s.messages) as i32;
		eprintln!("[LOG UI] Reached top of chat! Requesting next local page for {} at offset {}", jid, offset);
		let payload = serde_json::json!({
			"action": "get_messages",
			"data": { "jid": jid, "limit": crate::MESSAGE_PAGE_SIZE, "offset": offset }
		});
		let _ = s.ws_tx.send(payload.to_string());
	}
}

/// Asks the phone for WhatsApp history older than what's in the local
/// database, once local pagination (`get_messages` with a growing `offset`)
/// has come back short for the open chat. Arms `phone_history_deadline` so
/// the app stops waiting - and stops asking again for this jid - if the
/// phone never answers (see `PHONE_HISTORY_TIMEOUT_SECS`). The actual
/// messages, if any, arrive later as a "history_sync" event handled in
/// `websocket_events`, which re-queries the same local offset rather than
/// reloading the whole chat.
pub(crate) fn request_phone_history_sync(s: &mut AppState, jid: &str) {
	eprintln!("[LOG UI] Requesting older history from phone for: {}", jid);
	let oldest_msg_id = s.messages.first().map(|m| m.id.clone()).unwrap_or_default();
	s.phone_history_pending_jid = Some(jid.to_string());
	s.phone_history_deadline =
		Some(std::time::Instant::now() + std::time::Duration::from_secs(crate::PHONE_HISTORY_TIMEOUT_SECS));
	let payload = serde_json::json!({
		"action": "request_history_sync",
		"data": { "jid": jid, "message_id": oldest_msg_id, "count": 50 }
	});
	let _ = s.ws_tx.send(payload.to_string());
}

pub(crate) fn search_in_conversation(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>, query: String) {
	if let Some(ref jid) = state.borrow().selected_chat {
		let payload = serde_json::json!({
			"action": "search_messages",
			"data": { "jid": jid, "query": query }
		});
		let _ = state.borrow().ws_tx.send(payload.to_string());
	}
}

pub(crate) fn toggle_search_panel(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	if state.borrow().selected_chat.is_none() {
		live_region::announce(parts.conversations_parts.messages_announcer.clone(), "No chat open.");
		return;
	}
	let visible = parts.conversations_parts.search_panel.is_shown();
	parts.conversations_parts.search_panel.show(!visible);
	if !visible {
		parts.conversations_parts.search_conv_field.set_value("");
		parts.conversations_parts.search_conv_field.set_focus();
	}
	parts.conversations_parts.conversation_panel.layout();
}

pub(crate) fn prompt_locked_chats(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let dlg = TextEntryDialog::builder(&parts.frame, "Enter Secret Code:", "Locked Chats").password().build();
	if dlg.show_modal() == wxdragon::id::ID_OK {
		if let Some(val) = dlg.get_value() {
			let text = val.trim().to_string();
			if !text.is_empty() {
				let payload = serde_json::json!({
					"action": "verify_chat_lock_code",
					"data": { "code": text }
				});
				let _ = state.borrow().ws_tx.send(payload.to_string());
			}
		}
	}
}

pub(crate) fn close_locked_chats(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let mut s = state.borrow_mut();
	if s.chats_unlocked {
		s.chats_unlocked = false;
		apply_chat_filter(&mut s, parts);
		live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Locked chats hidden.");
	}
}

pub(crate) fn close_search(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	// Hide search panel and reload the real chat messages
	parts.conversations_parts.search_panel.show(false);
	parts.conversations_parts.search_conv_field.set_value("");
	if let Some(jid) = state.borrow().selected_chat.clone() {
		let payload = serde_json::json!({
			"action": "get_messages",
			"data": { "jid": jid, "limit": crate::MESSAGE_PAGE_SIZE, "offset": 0 }
		});
		let _ = state.borrow().ws_tx.send(payload.to_string());
	}
	parts.conversations_parts.conversation_panel.layout();
}

pub(crate) fn jump_to_search_result(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	message_id: String,
	message_text: String,
) {
	// Hide search, set the target so get_messages scrolls to it, reload chat
	parts.conversations_parts.search_panel.show(false);
	parts.conversations_parts.search_conv_field.set_value("");
	{
		let mut s = state.borrow_mut();
		if !message_id.is_empty() {
			s.search_target_id = Some(message_id.clone());
		} else if !message_text.is_empty() {
			s.search_target_text = Some(message_text.clone());
		}
		if let Some(jid) = s.selected_chat.clone() {
			// Fetches a bounded window of local history centered on the target
			// message instead of the whole chat - the existing
			// `search_target_id`/`search_target_text` handling above finds it
			// within whatever page comes back, same as it always has.
			let payload = serde_json::json!({
				"action": "get_messages_around",
				"data": { "jid": jid, "messageId": message_id, "text": message_text }
			});
			let _ = s.ws_tx.send(payload.to_string());
		}
	}
	parts.conversations_parts.conversation_panel.layout();
}

pub(crate) fn show_conversation_data(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let s = state.borrow();
	if let Some(ref jid) = s.selected_chat {
		if jid.ends_with("@g.us") {
			let local_name = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.clone());
			parts.group_data_dialog.group_name_field.set_value(&local_name);
			parts.group_data_dialog.members_list.clear();
			parts.group_data_dialog.media_list.clear();
			parts.group_data_dialog.documents_list.clear();
			parts.group_data_dialog.links_list.clear();
			parts.group_data_dialog.groups_list.clear();

			let payload = serde_json::json!({
				"action": "get_group_info",
				"data": { "jid": jid }
			});
			let _ = s.ws_tx.send(payload.to_string());
			drop(s);
			state.borrow_mut().group_data_subgroup_jids.clear();

			parts.group_data_dialog.dialog.show_modal();
		} else {
			let local_name = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.clone());
			let raw_phone = jid.split('@').next().unwrap_or(jid);
			let mut phone = raw_phone.to_string();
			if let Ok(parsed) = phonenumber::parse(None, format!("+{}", raw_phone)) {
				phone = parsed.format().mode(phonenumber::Mode::International).to_string();
			}

			let initial_text =
				format!("Name: {}\nPhone: {}\nWhatsApp Name: Loading...\nAbout/Status: Loading...", local_name, phone);
			parts.conversation_data_dialog.info_field.set_value(&initial_text);

			let payload = serde_json::json!({
				"action": "get_user_info",
				"data": { "jid": jid }
			});
			let _ = s.ws_tx.send(payload.to_string());

			let media = build_media_messages(&s.messages);
			let documents = build_document_messages(&s.messages);
			let links = build_link_messages(&s.messages);
			drop(s);

			populate_message_list(&parts.conversation_data_dialog.media_list, &media, |m| m.display_filename());
			populate_message_list(&parts.conversation_data_dialog.documents_list, &documents, |m| m.display_filename());
			populate_message_list(&parts.conversation_data_dialog.links_list, &links, |m| {
				m.first_link().unwrap_or_default()
			});

			let mut s_mut = state.borrow_mut();
			s_mut.conversation_data_media = media;
			s_mut.conversation_data_documents = documents;
			s_mut.conversation_data_links = links;
			drop(s_mut);

			parts.conversation_data_dialog.info_field.set_focus();
			parts.conversation_data_dialog.dialog.show_modal();
		}
	}
}

pub(crate) fn mark_read(
	state: &Rc<RefCell<AppState>>,
	_parts: &Rc<WindowParts>,
	jid: String,
	message_ids: Vec<String>,
) {
	let payload = serde_json::json!({
		"action": "mark_read",
		"data": {
			"jid": jid,
			"message_ids": message_ids
		}
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

pub(crate) fn close_conversation(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let mut s = state.borrow_mut();
	// See the matching guard in `open_chat_for_jid` - closing the
	// conversation is just as much a "leave the message being edited" as
	// switching to a different one is.
	if s.editing_message_id.take().is_some() {
		s.editing_message_original_text = None;
		parts.conversations_parts.cancel_edit_btn.show(false);
		parts.conversations_parts.add_attachment_btn.show(true);
		parts.conversations_parts.message_label.set_label("Type a message");
		// An edit-in-progress is not a new message someone was composing -
		// clear it rather than letting the `save_composer_draft` call below
		// pick up the message's own (edited) text as if it were an ordinary
		// unsent draft for this chat.
		parts.conversations_parts.message_field.set_value("");
	}
	save_composer_draft(&mut s, parts);
	s.selected_chat = None;
	clear_pending_reply(&mut s, parts);
	parts.conversations_parts.conversation_panel.show(false);
	// Hiding the panel does not release anything: without this the closed
	// chat's messages (and their second copy in display_items, and the native
	// list's rows) stay resident for the rest of the session. On a chat with
	// tens of thousands of messages that is tens of MB held for a view the
	// user has explicitly left. `clear_chat` already does the same.
	release_conversation_buffers(&mut s, parts);
	parts.conversations_parts.panel.layout();
	parts.conversations_parts.conversations_list.set_focus();
}

/// Drops everything tied to the currently-displayed conversation: the message
/// vectors, their display-item copies, any selection-mode rows, the media /
/// documents / links snapshots taken for the info dialogs, and the native list
/// controls' contents. Called when a chat is closed or switched away from.
pub(crate) fn release_conversation_buffers(s: &mut AppState, parts: &Rc<WindowParts>) {
	s.messages = Vec::new();
	s.display_items = Vec::new();
	s.selection_rows = Vec::new();
	s.prev_msg_count = 0;
	// These hold full ChatMessage clones taken when an info dialog is opened
	// and were never released - not on dialog close, not on chat switch.
	s.group_data_media = Vec::new();
	s.group_data_documents = Vec::new();
	s.group_data_links = Vec::new();
	s.conversation_data_media = Vec::new();
	s.conversation_data_documents = Vec::new();
	s.conversation_data_links = Vec::new();
	// NOT cleared here, deliberately. Dropping in-flight downloads would make
	// the action button read "Download" for a file that is already downloading
	// until the next progress event re-inserted it - a wrong announcement for
	// anyone navigating by screen reader. The only thing left behind is an
	// entry for a download that never reports completion or failure, which is
	// rare and a few dozen bytes; the false announcement costs more than the
	// bytes are worth. (`s.downloads_in_progress`)
	parts.conversations_parts.messages_list.clear();
	parts.conversations_parts.messages_select_list.clear();
	parts.group_data_dialog.media_list.clear();
	parts.group_data_dialog.documents_list.clear();
	parts.group_data_dialog.links_list.clear();
	parts.conversation_data_dialog.media_list.clear();
	parts.conversation_data_dialog.documents_list.clear();
	parts.conversation_data_dialog.links_list.clear();
}

pub(crate) fn toggle_lock_chat(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String, lock: bool) {
	let chat_name = {
		let mut s = state.borrow_mut();
		if lock {
			s.locked_jids.insert(jid.clone());
		} else {
			s.locked_jids.remove(&jid);
		}
		apply_chat_filter(&mut s, parts);
		s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone())
	};
	let payload = serde_json::json!({
		"action": "set_chat_locked",
		"data": {
			"jid": jid,
			"value": lock
		}
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	let action_str = if lock { "Locked" } else { "Unlocked" };
	live_region::announce(
		parts.conversations_parts.messages_announcer.clone(),
		&format!("{} chat {}.", action_str, chat_name),
	);
}

pub(crate) fn toggle_pin_chat(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String, pin: bool) {
	let chat_name = {
		let mut s = state.borrow_mut();
		if pin {
			s.pinned_jids.insert(jid.clone());
		} else {
			s.pinned_jids.remove(&jid);
		}
		s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone())
	};
	let payload = serde_json::json!({
		"action": "set_chat_pinned",
		"data": {
			"jid": jid,
			"value": pin
		}
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	live_region::announce(
		parts.conversations_parts.messages_announcer.clone(),
		&format!("Chat {} {}.", chat_name, if pin { "pinned" } else { "unpinned" }),
	);
}

pub(crate) fn toggle_archive_chat(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String, archive: bool) {
	let chat_name = {
		let mut s = state.borrow_mut();
		if archive {
			s.archived_jids.insert(jid.clone());
		} else {
			s.archived_jids.remove(&jid);
		}
		let name = s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
		apply_chat_filter(&mut s, parts);
		name
	};
	let payload = serde_json::json!({
		"action": "set_chat_archived",
		"data": {
			"jid": jid,
			"value": archive
		}
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	live_region::announce(
		parts.conversations_parts.messages_announcer.clone(),
		&format!("Chat {} {}.", chat_name, if archive { "archived" } else { "unarchived" }),
	);
}

pub(crate) fn set_archived_alert(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	jid: String,
	level: whantv::ArchivedAlert,
) {
	let (chat_name, spoken) = {
		let mut s = state.borrow_mut();
		s.settings.set_archived_alert(&jid, level);
		s.settings.save();
		// The tooltip total only counts chats that would actually alert,
		// so changing a level changes it.
		refresh_tray_tooltip(&s, parts);
		let name = s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
		let spoken = match level {
			whantv::ArchivedAlert::Silent => "silent",
			whantv::ArchivedAlert::SoundOnly => "sound only",
			whantv::ArchivedAlert::Full => "notify normally",
		};
		(name, spoken)
	};
	live_region::announce(
		parts.conversations_parts.messages_announcer.clone(),
		&format!("Archived alerts for {} set to {}.", chat_name, spoken),
	);
}

pub(crate) fn toggle_mute_chat(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String, mute: bool) {
	let chat_name = {
		let mut s = state.borrow_mut();
		if mute {
			s.muted_jids.insert(jid.clone());
		} else {
			s.muted_jids.remove(&jid);
		}
		s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone())
	};
	let payload = serde_json::json!({
		"action": "set_chat_muted",
		"data": {
			"jid": jid,
			"muted": mute,
			"duration_seconds": 0
		}
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	live_region::announce(
		parts.conversations_parts.messages_announcer.clone(),
		&format!("Notifications {} for {}.", if mute { "muted" } else { "unmuted" }, chat_name),
	);
}

pub(crate) fn toggle_read_chat(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String, read: bool) {
	let chat_name = {
		let mut s = state.borrow_mut();
		if read {
			s.unread_counts.insert(jid.clone(), 0);
			clear_unread_divider_for_open_chat(&mut s, parts, &jid);
		} else {
			s.unread_counts.insert(jid.clone(), 1);
		}
		refresh_tray_tooltip(&s, parts);
		s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone())
	};
	let payload = serde_json::json!({
		"action": "set_chat_read",
		"data": {
			"jid": jid,
			"value": read
		}
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	live_region::announce(
		parts.conversations_parts.messages_announcer.clone(),
		&format!("Chat {} marked as {}.", chat_name, if read { "read" } else { "unread" }),
	);
}

/// Removes `jid` from every local chat-list bookkeeping structure and, if it
/// was the open conversation, closes that view - the optimistic local half
/// of both `delete_chat` and `delete_chat_and_leave_group`. Returns the
/// chat's display name for the announcement each of those makes afterward.
fn remove_chat_locally(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: &str) -> String {
	let mut s = state.borrow_mut();
	s.chats.retain(|j| j != jid);
	s.filtered_chats.retain(|j| j != jid);
	s.archived_jids.remove(jid);
	s.pinned_jids.remove(jid);
	s.muted_jids.remove(jid);
	s.unread_counts.remove(jid);
	s.draft_messages.remove(jid);
	let name = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.to_string());
	if s.selected_chat.as_deref() == Some(jid) {
		s.selected_chat = None;
		clear_pending_reply(&mut s, parts);
		parts.conversations_parts.conversation_panel.show(false);
	}
	apply_chat_filter(&mut s, parts);
	refresh_tray_tooltip(&s, parts);
	name
}

pub(crate) fn delete_chat(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String) {
	let chat_name = remove_chat_locally(state, parts, &jid);
	let payload = serde_json::json!({
		"action": "delete_chat",
		"data": {
			"jid": jid
		}
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	live_region::announce(
		parts.conversations_parts.messages_announcer.clone(),
		&format!("Chat {} deleted.", chat_name),
	);
}

/// "Delete chat and exit group": one backend action (`leave_and_delete_group`)
/// that leaves the group before deleting the chat, in that order, guarded
/// against the group's own "You left" system message resurrecting the chat
/// in between - see `handleLeaveAndDeleteGroup` on the backend. The local
/// side is the same optimistic removal as a plain delete.
pub(crate) fn delete_chat_and_leave_group(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String) {
	let chat_name = remove_chat_locally(state, parts, &jid);
	let payload = serde_json::json!({
		"action": "leave_and_delete_group",
		"data": {
			"jid": jid
		}
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	live_region::announce(
		parts.conversations_parts.messages_announcer.clone(),
		&format!("Left and deleted chat {}.", chat_name),
	);
}

/// Deletes a chat's messages while leaving the chat itself in the list -
/// distinct from `delete_chat`, which removes the chat entirely. If the
/// chat is currently open, its message list is cleared immediately rather
/// than waiting for a later `chats` refresh (which wouldn't touch the open
/// conversation view anyway).
pub(crate) fn clear_chat(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String) {
	let (chat_name, is_open) = {
		let mut s = state.borrow_mut();
		let name = s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
		let is_open = s.selected_chat.as_ref() == Some(&jid);
		if is_open {
			s.messages.clear();
			s.display_items.clear();
			s.prev_msg_count = 0;
		}
		(name, is_open)
	};
	if is_open {
		parts.conversations_parts.messages_list.clear();
		crate::message_display::update_action_buttons(&state.borrow(), &parts.conversations_parts);
	}
	let payload = serde_json::json!({
		"action": "clear_chat",
		"data": {
			"jid": jid
		}
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	live_region::announce(
		parts.conversations_parts.messages_announcer.clone(),
		&format!("Chat {} cleared.", chat_name),
	);
}
