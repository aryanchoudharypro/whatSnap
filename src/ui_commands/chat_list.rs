use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState,
	platform::tray::refresh_tray_tooltip,
	ui::{
		chat_ui::{clear_pending_reply, clear_unread_divider_for_open_chat, open_chat_for_jid, save_composer_draft},
		contact_picker::refresh_selection_list,
		message_display::{InfoMediaLists, apply_chat_filter, fill_info_media},
		window::WindowParts,
	},
};

pub(crate) fn request_call_tab_data(s: &AppState) {
	for action in ["get_call_logs", "get_call_targets"] {
		let _ = s.ws_tx.send(serde_json::json!({ "action": action }).to_string());
	}
}

pub(crate) fn nav_selected(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, idx: usize) {
	parts.conversations_parts.calls_btn_panel.show(idx == 6);
	parts.conversations_parts.search_label.set_label(if idx == 6 {
		"Search or enter phone number to place call"
	} else if idx == 1 {
		"Search status updates"
	} else {
		"Search chats"
	});
	parts.conversations_parts.new_conv_btn.show(idx != 6);
	match idx {
		0 => {
			parts.conversations_parts.panel.show(true);
			state.borrow_mut().active_tab = 0;
			parts.conversations_parts.new_conv_btn.set_label("New Chat");
			parts.conversations_parts.post_media_status_btn.show(false);
			parts.conversations_parts.conversations_label.set_label("Chats");
			apply_chat_filter(&mut state.borrow_mut(), parts);
		}
		1 => {
			parts.conversations_parts.panel.show(true);
			state.borrow_mut().active_tab = 1;
			parts.conversations_parts.new_conv_btn.set_label("Post Status Update...");
			parts.conversations_parts.post_media_status_btn.set_label("Post Photo/Video Status...");
			parts.conversations_parts.post_media_status_btn.show(true);
			parts.conversations_parts.conversations_label.set_label("Status Updates");
			let payload = serde_json::json!({ "action": "get_statuses" });
			let _ = state.borrow().ws_tx.send(payload.to_string());
			apply_chat_filter(&mut state.borrow_mut(), parts);
		}
		2 => {
			parts.conversations_parts.panel.show(true);
			state.borrow_mut().active_tab = 2;
			parts.conversations_parts.new_conv_btn.set_label("New Chat");
			parts.conversations_parts.post_media_status_btn.show(false);
			parts.conversations_parts.conversations_label.set_label("Starred Messages");
			let payload = serde_json::json!({ "action": "get_starred_messages" });
			let _ = state.borrow().ws_tx.send(payload.to_string());
			apply_chat_filter(&mut state.borrow_mut(), parts);
		}
		3 => {
			parts.conversations_parts.panel.show(true);
			state.borrow_mut().active_tab = 3;
			parts.conversations_parts.new_conv_btn.set_label("New Chat");
			parts.conversations_parts.post_media_status_btn.show(false);
			parts.conversations_parts.conversations_label.set_label("Archived Chats");
			apply_chat_filter(&mut state.borrow_mut(), parts);
		}
		4 => {
			parts.conversations_parts.panel.show(true);
			state.borrow_mut().active_tab = 4;
			parts.conversations_parts.new_conv_btn.set_label("New Chat");
			parts.conversations_parts.post_media_status_btn.show(false);
			parts.conversations_parts.conversations_label.set_label("Communities");
			apply_chat_filter(&mut state.borrow_mut(), parts);
		}
		5 => {
			parts.conversations_parts.panel.show(true);
			state.borrow_mut().active_tab = 5;
			parts.conversations_parts.new_conv_btn.set_label("Find channels...");
			parts.conversations_parts.post_media_status_btn.set_label("Create channel...");
			parts.conversations_parts.post_media_status_btn.show(true);
			parts.conversations_parts.conversations_label.set_label("Channels");
			apply_chat_filter(&mut state.borrow_mut(), parts);
		}
		6 => {
			parts.conversations_parts.panel.show(true);
			state.borrow_mut().active_tab = 6;
			parts.conversations_parts.post_media_status_btn.show(false);
			parts.conversations_parts.locked_chats_btn.show(false);
			parts.conversations_parts.chat_filter_label.show(false);
			parts.conversations_parts.chat_filter_combo.show(false);
			parts.conversations_parts.conversation_panel.show(false);
			parts.conversations_parts.conversations_label.set_label("Recent Calls");
			request_call_tab_data(&state.borrow());
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
		let message_id = state.borrow().filtered_status_message_ids.get(idx).cloned();
		if let Some(message_id) = message_id {
			let outcome = crate::ui::status_viewer::show(state, parts, &message_id);
			crate::platform::tray::enter_release_guard().set(true);
			apply_chat_filter(&mut state.borrow_mut(), parts);
			if let crate::ui::status_viewer::ViewerOutcome::OpenChat(jid) = outcome {
				open_chat_for_jid(&jid, state, parts);
			}
		}
		return;
	}
	if state.borrow().active_tab == 6 {
		let jid = state.borrow().filtered_chats.get(idx).filter(|j| !j.is_empty()).cloned();
		if let Some(jid) = jid {
			if let Some(call_id) = jid.strip_prefix(crate::ui_commands::calls::JOIN_ROW_PREFIX) {
				crate::ui_commands::calls::join_group_call(state, parts, Some(call_id.to_string()));
				return;
			}
			state.borrow_mut().selected_chat = Some(jid);
			crate::ui_commands::calls::place_call(state, parts);
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
		if state.borrow().community_jids.contains(jid) {
			show_conversation_data(state, parts);
		}
	}
}

const GROUPS_TAB: usize = 4;

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
		parts.conversations_parts.emoji_btn.show(false);
		parts.conversations_parts.send_message_btn.show(false);
		parts.conversations_parts.record_voice_btn.show(false);
		parts.conversations_parts.messages_select_list.set_focus();
	} else {
		let at = parts
			.conversations_parts
			.messages_select_list
			.get_selection()
			.and_then(|i| s.selection_rows.get(i as usize))
			.and_then(|item| s.display_items.iter().position(|d| d.id() == item.id()));
		if let Some(pos) = at {
			parts.conversations_parts.messages_list.set_selection(pos as u32, true);
		}
		s.selected_message_ids.clear();
		s.selection_rows = Vec::new();
		parts.conversations_parts.messages_select_list.clear();
		parts.conversations_parts.messages_select_list.show(false);
		parts.conversations_parts.selection_bar_panel.show(false);
		parts.conversations_parts.messages_list.show(true);
		parts.conversations_parts.message_field.show(true);
		parts.conversations_parts.add_attachment_btn.show(true);
		parts.conversations_parts.emoji_btn.show(true);
		parts
			.conversations_parts
			.send_message_btn
			.show(!parts.conversations_parts.message_field.get_value().trim().is_empty());
		parts.conversations_parts.record_voice_btn.show(true);
		parts.conversations_parts.messages_list.set_focus();
	}
	parts.conversations_parts.conversation_panel.layout();
}

pub(crate) fn delete_selected_messages(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, for_everyone: bool) {
	let (jid_opt, ws_tx, to_delete) = {
		let s = state.borrow();
		let ids = &s.selected_message_ids;
		let msgs: Vec<whatsnap::Msg> = s
			.selection_rows
			.iter()
			.filter(|item| ids.contains(item.id()))
			.flat_map(|item| item.messages())
			.cloned()
			.collect();
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
	parts.conversations_parts.emoji_btn.show(true);
	parts.conversations_parts.record_voice_btn.show(true);
	parts
		.conversations_parts
		.send_message_btn
		.show(!parts.conversations_parts.message_field.get_value().trim().is_empty());
	parts.conversations_parts.conversation_panel.layout();
	if !to_delete.is_empty() {
		crate::platform::announce::announce(parts.conversations_parts.messages_announcer, "Selected messages deleted.");
	}
}

pub(crate) fn load_more_messages(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let mut s = state.borrow_mut();
	if s.history_loading {
		eprintln!("[LOG UI] Request already in flight for chat, skipping duplicate trigger");
		return;
	}
	let jid = match s.selected_chat.clone() {
		Some(jid) => jid,
		None => return,
	};
	if s.no_more_messages && s.history_start_chats.contains(&jid) {
		drop(s);
		crate::platform::announce::announce(
			parts.conversations_parts.messages_announcer,
			"This is the start of the chat.",
		);
		return;
	}
	s.history_loading = true;
	if s.no_more_messages {
		request_phone_history_sync(&mut s, &jid);
	} else {
		eprintln!("[LOG UI] Reached top of chat! Requesting next local page for {}", jid);
		let payload = older_page_request(&s.messages, &jid);
		let _ = s.ws_tx.send(payload.to_string());
	}
}

pub(crate) fn load_newer_messages(state: &Rc<RefCell<AppState>>) {
	let mut s = state.borrow_mut();
	if s.messages_at_latest || s.newer_loading {
		return;
	}
	let Some(jid) = s.selected_chat.clone() else { return };
	let Some((after_ts, after_id)) = whatsnap::newest_message_cursor(&s.messages) else { return };
	s.newer_loading = true;
	let payload = serde_json::json!({
		"action": "get_messages",
		"data": {
			"jid": jid,
			"limit": crate::MESSAGE_PAGE_SIZE,
			"after_id": after_id,
			"after_timestamp": after_ts
		}
	});
	let _ = s.ws_tx.send(payload.to_string());
}

pub(crate) fn jump_to_latest_message(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>) {
	let mut s = state.borrow_mut();
	if s.messages_at_latest {
		return;
	}
	let Some(jid) = s.selected_chat.clone() else { return };
	s.prev_msg_count = 0;
	s.newer_loading = true;
	let payload = serde_json::json!({
		"action": "get_messages",
		"data": { "jid": jid, "limit": crate::MESSAGE_PAGE_SIZE, "offset": 0 }
	});
	let _ = s.ws_tx.send(payload.to_string());
}

pub(crate) fn jump_to_oldest_message(state: &Rc<RefCell<AppState>>) {
	let mut s = state.borrow_mut();
	if s.older_unloaded == 0 || s.history_loading {
		return;
	}
	let Some(jid) = s.selected_chat.clone() else { return };
	s.prev_msg_count = 0;
	s.land_on_first = true;
	let payload = serde_json::json!({
		"action": "get_messages",
		"data": { "jid": jid, "limit": crate::MESSAGE_PAGE_SIZE, "from_start": true }
	});
	let _ = s.ws_tx.send(payload.to_string());
}

pub(crate) fn around_request(jid: &str, id: &str, text: &str) -> serde_json::Value {
	serde_json::json!({
		"action": "get_messages_around",
		"data": { "jid": jid, "messageId": id, "text": text, "context": crate::MESSAGE_PAGE_SIZE / 2 }
	})
}

pub(crate) fn older_page_request(messages: &[whatsnap::Msg], jid: &str) -> serde_json::Value {
	let (before_ts, before_id) = whatsnap::oldest_message_cursor(messages).unwrap_or_default();
	serde_json::json!({
		"action": "get_messages",
		"data": {
			"jid": jid,
			"limit": crate::MESSAGE_PAGE_SIZE,
			"before_id": before_id,
			"before_timestamp": before_ts,
			"offset": if before_id.is_empty() { whatsnap::real_message_count(messages) as i32 } else { 0 }
		}
	})
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
	// Not `s.messages.first()`: that's always a date divider, with no ID.
	// The backend picks the actual anchor itself (skipping rows the phone
	// never sent, like group events); this is only a hint.
	let oldest_msg_id = whatsnap::oldest_message_cursor(&s.messages).map(|(_, id)| id).unwrap_or_default();
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
		crate::platform::announce::announce(parts.conversations_parts.messages_announcer, "No chat open.");
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
	if dlg.show_modal() == wxdragon::id::ID_OK
		&& let Some(val) = dlg.get_value()
	{
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

pub(crate) fn close_locked_chats(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let mut s = state.borrow_mut();
	if s.chats_unlocked {
		s.chats_unlocked = false;
		apply_chat_filter(&mut s, parts);
		crate::platform::announce::announce(parts.conversations_parts.messages_announcer, "Locked chats hidden.");
	}
}

pub(crate) fn close_search(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
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
			let payload = around_request(&jid, &message_id, &message_text);
			let _ = s.ws_tx.send(payload.to_string());
		}
	}
	parts.conversations_parts.conversation_panel.layout();
}

pub(crate) fn show_conversation_data(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let s = state.borrow();
	if let Some(ref jid) = s.selected_chat {
		if jid.ends_with("@newsletter") {
			let payload = serde_json::json!({ "action": "get_channel_info", "data": { "jid": jid } });
			let _ = s.ws_tx.send(payload.to_string());
		} else if jid.ends_with("@g.us") {
			let local_name = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.clone());
			parts.group_data_dialog.group_name_field.set_value(&local_name);
			parts.group_data_dialog.members_list.clear();
			parts.group_data_dialog.media_list.clear();
			parts.group_data_dialog.documents_list.clear();
			parts.group_data_dialog.links_list.clear();
			parts.group_data_dialog.groups_list.clear();
			parts.group_data_dialog.groups_label.set_label("Groups in this community");
			parts.group_data_dialog.join_group_btn.set_label("Join group");
			if s.community_jids.contains(jid) {
				parts.group_data_dialog.notebook.set_selection(GROUPS_TAB);
			}

			parts.group_data_dialog.photo.show(false);
			let payload = serde_json::json!({ "action": "get_avatar", "data": { "jid": jid } });
			let _ = s.ws_tx.send(payload.to_string());
			let payload = serde_json::json!({
				"action": "get_group_info",
				"data": { "jid": jid }
			});
			let _ = s.ws_tx.send(payload.to_string());
			drop(s);
			let mut s = state.borrow_mut();
			s.group_data_subgroup_jids.clear();
			s.group_data_subgroup_joined.clear();
			s.group_data_community = None;
			drop(s);

			parts.group_data_dialog.dialog.show_modal();
			let mut s = state.borrow_mut();
			s.group_data_media = Vec::new();
			s.group_data_documents = Vec::new();
			s.group_data_links = Vec::new();
		} else {
			let jid = jid.clone();
			drop(s);
			show_contact_info(state, parts, jid);
		}
	}
}

pub(crate) fn show_contact_info(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String) {
	let s = state.borrow();
	let local_name = s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
	let raw_phone = jid.split('@').next().unwrap_or(&jid);
	let mut phone = raw_phone.to_string();
	if let Some(formatted) = whatsnap::phone::format_international(raw_phone) {
		phone = formatted;
	}

	let initial_text = format!(
		"Name: {}\nPhone: {}\n{}WhatsApp Name: Loading...\nAbout/Status: Loading...",
		local_name,
		phone,
		crate::ui_commands::blocking::info_line(&s, &jid)
	);
	let dlg = &parts.conversation_data_dialog;
	dlg.info_field.set_value(&initial_text);
	let callable = crate::ui_commands::calls::can_call(&s, &jid);
	dlg.voice_call_btn.show(callable);
	dlg.video_call_btn.show(callable);

	let payload = serde_json::json!({
		"action": "get_user_info",
		"data": { "jid": jid }
	});
	let _ = s.ws_tx.send(payload.to_string());
	dlg.photo.show(false);
	let payload = serde_json::json!({ "action": "get_avatar", "data": { "jid": jid } });
	let _ = s.ws_tx.send(payload.to_string());
	let payload = serde_json::json!({ "action": "get_common_groups", "data": { "jid": jid } });
	let _ = s.ws_tx.send(payload.to_string());
	dlg.common_groups_list.clear();
	dlg.common_groups_list.append("Loading...");

	let lists = InfoMediaLists { media: &dlg.media_list, documents: &dlg.documents_list, links: &dlg.links_list };
	let (media, documents, links) = if s.selected_chat.as_deref() == Some(jid.as_str()) {
		fill_info_media(&s, lists, None)
	} else {
		let payload = serde_json::json!({ "action": "get_chat_media", "data": { "jid": jid } });
		let _ = s.ws_tx.send(payload.to_string());
		for list in [lists.media, lists.documents, lists.links] {
			list.clear();
			list.append("Loading...");
		}
		Default::default()
	};
	drop(s);
	let mut s_mut = state.borrow_mut();
	s_mut.conversation_data_contact = None;
	s_mut.info_chat = Some(jid);
	drop(s_mut);
	crate::ui_commands::contacts_groups::show_contact_card(parts, None);

	let mut s_mut = state.borrow_mut();
	s_mut.conversation_data_media = media;
	s_mut.conversation_data_documents = documents;
	s_mut.conversation_data_links = links;
	drop(s_mut);

	parts.conversation_data_dialog.info_field.set_focus();
	parts.conversation_data_dialog.dialog.show_modal();
	let mut s = state.borrow_mut();
	s.info_chat = None;
	s.conversation_data_media = Vec::new();
	s.conversation_data_documents = Vec::new();
	s.conversation_data_links = Vec::new();
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
	if s.editing_message_id.take().is_some() {
		s.editing_message_original_text = None;
		parts.conversations_parts.cancel_edit_btn.show(false);
		parts.conversations_parts.add_attachment_btn.show(true);
		parts.conversations_parts.emoji_btn.show(true);
		parts.conversations_parts.message_label.set_label("Type a message");
		parts.conversations_parts.message_field.set_value("");
	}
	save_composer_draft(&mut s, parts);
	s.selected_chat = None;
	crate::ui::window::refresh_menu_bar(&s, &parts.frame);
	clear_pending_reply(&mut s, parts);
	parts.conversations_parts.conversation_panel.show(false);
	release_conversation_buffers(&mut s, parts);
	parts.conversations_parts.panel.layout();
	parts.conversations_parts.conversations_list.set_focus();
}

pub(crate) fn release_conversation_buffers(s: &mut AppState, parts: &Rc<WindowParts>) {
	s.messages = Vec::new();
	s.display_items = Vec::new();
	s.selection_rows = Vec::new();
	s.prev_msg_count = 0;
	// Receipts parked for messages of this chat that never got loaded -
	// nothing will ever claim them once it's closed.
	s.pending_receipts = std::collections::HashMap::new();
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

pub(crate) fn choose_disappearing_timer(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String) {
	const CHOICES: [(&str, u32); 4] = [("Off", 0), ("24 hours", 86_400), ("7 days", 604_800), ("90 days", 7_776_000)];
	let chat_name = state.borrow().contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
	let labels: Vec<&str> = CHOICES.iter().map(|(label, _)| *label).collect();
	let dialog = SingleChoiceDialog::builder(
		&parts.frame,
		&format!("Make new messages in {} disappear after:", chat_name),
		"Disappearing Messages",
		&labels,
	)
	.build();
	if dialog.show_modal() != wxdragon::id::ID_OK {
		return;
	}
	let Some(choice) = dialog.get_string_selection() else { return };
	let Some(&(label, seconds)) = CHOICES.iter().find(|(l, _)| *l == choice) else { return };
	let payload = serde_json::json!({
		"action": "set_disappearing_timer",
		"data": { "jid": jid, "seconds": seconds }
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	let spoken = if seconds == 0 {
		format!("Disappearing messages turned off for {}.", chat_name)
	} else {
		format!("Disappearing messages set to {} for {}.", label, chat_name)
	};
	crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &spoken);
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
	crate::platform::announce::announce(
		parts.conversations_parts.messages_announcer,
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
	crate::platform::announce::announce(
		parts.conversations_parts.messages_announcer,
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
	crate::platform::announce::announce(
		parts.conversations_parts.messages_announcer,
		&format!("Chat {} {}.", chat_name, if archive { "archived" } else { "unarchived" }),
	);
}

pub(crate) fn set_archived_alert(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	jid: String,
	level: whatsnap::ArchivedAlert,
) {
	let (chat_name, spoken) = {
		let mut s = state.borrow_mut();
		s.settings.set_archived_alert(&jid, level);
		s.settings.save();
		refresh_tray_tooltip(&s, parts);
		let name = s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
		let spoken = match level {
			whatsnap::ArchivedAlert::Silent => "silent",
			whatsnap::ArchivedAlert::SoundOnly => "sound only",
			whatsnap::ArchivedAlert::Full => "notify normally",
		};
		(name, spoken)
	};
	crate::platform::announce::announce(
		parts.conversations_parts.messages_announcer,
		&format!("Archived alerts for {} set to {}.", chat_name, spoken),
	);
}

pub(crate) fn set_chat_notify(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	jid: String,
	notify: whatsnap::ChatNotify,
) {
	let spoken = {
		let mut s = state.borrow_mut();
		s.settings.set_chat_notify(&jid, notify);
		s.settings.save();
		refresh_tray_tooltip(&s, parts);
		let name = s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
		let alert = match notify.alert {
			whatsnap::ChatAlert::Default => "as set in Settings",
			whatsnap::ChatAlert::Full => "notification and sound",
			whatsnap::ChatAlert::NoSound => "notification only, no sound",
			whatsnap::ChatAlert::SoundOnly => "sound only, no notification",
			whatsnap::ChatAlert::Off => "off on this computer",
		};
		let content = if notify.hide_content { ", message content hidden" } else { "" };
		format!("Notifications for {name}: {alert}{content}.")
	};
	crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &spoken);
}

pub(crate) fn show_chat_notify(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String) {
	let (name, current) = {
		let s = state.borrow();
		(s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone()), s.settings.chat_notify(&jid))
	};
	let Some(notify) = crate::ui::dialogs::chat_notify_dialog::ask(&parts.frame, &name, current) else { return };
	if notify != current {
		set_chat_notify(state, parts, jid, notify);
	}
}

pub(crate) const MUTE_DURATIONS: [(&str, &str, u64); 3] =
	[("&8 hours", " for 8 hours", 8 * 3600), ("1 &week", " for 1 week", 168 * 3600), ("&Always", "", 0)];

pub(crate) fn has_mute_durations(jid: &str) -> bool {
	!jid.ends_with("@newsletter")
}

/// The "Mute notifications" submenu, its items numbered from `first_id` in
/// [`MUTE_DURATIONS`] order. A submenu's choice never reaches its parent's
/// handler, so `on_choice` gets the picked length in seconds.
pub(crate) fn mute_duration_menu(first_id: i32, on_choice: impl Fn(u64) + 'static) -> Menu {
	let mut builder = Menu::builder();
	for (i, (label, _, _)) in MUTE_DURATIONS.iter().enumerate() {
		builder = builder.append_item(first_id + i as i32, label, "Mute notifications from this chat");
	}
	let menu = builder.build();
	menu.on_selected(move |evt| {
		let picked = usize::try_from(evt.get_id() - first_id).ok().and_then(|i| MUTE_DURATIONS.get(i));
		if let Some(&(_, _, seconds)) = picked {
			on_choice(seconds);
		}
	});
	menu
}

pub(crate) fn choose_mute_duration(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String) {
	if !has_mute_durations(&jid) {
		toggle_mute_chat(state, parts, jid, true, 0);
		return;
	}
	let state_menu = state.clone();
	let parts_menu = parts.clone();
	let mut menu = mute_duration_menu(MUTE_DURATION_POPUP_ID, move |seconds| {
		toggle_mute_chat(&state_menu, &parts_menu, jid.clone(), true, seconds)
	});
	parts.frame.popup_menu(&mut menu, None);
	menu.destroy_menu();
	crate::platform::tray::enter_release_guard().set(true);
}

const MUTE_DURATION_POPUP_ID: i32 = 4601;

pub(crate) fn toggle_mute_chat(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	jid: String,
	mute: bool,
	seconds: u64,
) {
	let chat_name = {
		let mut s = state.borrow_mut();
		if mute {
			s.muted_jids.insert(jid.clone());
		} else {
			s.muted_jids.remove(&jid);
		}
		s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone())
	};
	let seconds = if mute { seconds } else { 0 };
	let payload = serde_json::json!({
		"action": "set_chat_muted",
		"data": {
			"jid": jid,
			"muted": mute,
			"duration_seconds": seconds
		}
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	let how_long = MUTE_DURATIONS.iter().find(|(_, _, s)| *s == seconds).map_or("", |(_, how_long, _)| how_long);
	let spoken = if mute {
		format!("Notifications from {} muted{}.", chat_name, how_long)
	} else {
		format!("Notifications from {} unmuted.", chat_name)
	};
	crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &spoken);
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
	crate::platform::announce::announce(
		parts.conversations_parts.messages_announcer,
		&format!("Chat {} marked as {}.", chat_name, if read { "read" } else { "unread" }),
	);
}

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
		release_conversation_buffers(&mut s, parts);
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
	crate::platform::announce::announce(
		parts.conversations_parts.messages_announcer,
		&format!("Chat {} deleted.", chat_name),
	);
}

pub(crate) fn delete_chat_and_leave_group(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String) {
	let chat_name = remove_chat_locally(state, parts, &jid);
	let payload = serde_json::json!({
		"action": "leave_and_delete_group",
		"data": {
			"jid": jid
		}
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	crate::platform::announce::announce(
		parts.conversations_parts.messages_announcer,
		&format!("Left and deleted chat {}.", chat_name),
	);
}

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
		crate::ui::message_display::update_action_buttons(&state.borrow(), &parts.conversations_parts);
	}
	let payload = serde_json::json!({
		"action": "clear_chat",
		"data": {
			"jid": jid
		}
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	crate::platform::announce::announce(
		parts.conversations_parts.messages_announcer,
		&format!("Chat {} cleared.", chat_name),
	);
}
