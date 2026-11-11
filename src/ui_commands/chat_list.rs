use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState, UiCommand,
	platform::{tray::refresh_tray_tooltip, windows_hello},
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
	if idx != 0 && state.borrow().chats_unlocked {
		close_locked_chats(state, parts);
	}
	parts.conversations_parts.calls_btn_panel.show(idx == 6);
	parts.conversations_parts.search_label.set_label(crate::i18n::tr(if idx == 6 {
		"Search or enter phone number to place call"
	} else if idx == 1 {
		"Search status updates"
	} else if idx == 0 {
		"Search or start a new chat"
	} else if idx == 2 {
		"Search starred messages"
	} else {
		"Search chats"
	}));
	parts.conversations_parts.new_conv_btn.show(idx != 6 && idx != 2);
	match idx {
		0 => {
			parts.conversations_parts.panel.show(true);
			state.borrow_mut().active_tab = 0;
			parts.conversations_parts.new_conv_btn.set_label(crate::i18n::tr("New Chat"));
			parts.conversations_parts.post_media_status_btn.show(false);
			parts.conversations_parts.status_privacy_btn.show(false);
			parts.conversations_parts.conversations_label.set_label(crate::i18n::tr("Chats"));
			apply_chat_filter(&mut state.borrow_mut(), parts);
		}
		1 => {
			parts.conversations_parts.panel.show(true);
			state.borrow_mut().active_tab = 1;
			parts.conversations_parts.new_conv_btn.set_label(crate::i18n::tr("Post Status Update..."));
			parts.conversations_parts.post_media_status_btn.set_label(crate::i18n::tr("Post Photo/Video Status..."));
			parts.conversations_parts.post_media_status_btn.show(true);
			parts.conversations_parts.status_privacy_btn.show(true);
			parts.conversations_parts.locked_chats_btn.show(false);
			parts.conversations_parts.chat_filter_label.show(false);
			parts.conversations_parts.chat_filter_combo.show(false);
			parts.conversations_parts.conversation_panel.show(false);
			parts.conversations_parts.conversations_label.set_label(crate::i18n::tr("Status Updates"));
			let payload = serde_json::json!({ "action": "get_statuses" });
			let _ = state.borrow().ws_tx.send(payload.to_string());
			apply_chat_filter(&mut state.borrow_mut(), parts);
		}
		2 => {
			parts.conversations_parts.panel.show(true);
			state.borrow_mut().active_tab = 2;
			parts.conversations_parts.post_media_status_btn.show(false);
			parts.conversations_parts.status_privacy_btn.show(false);
			parts.conversations_parts.locked_chats_btn.show(false);
			parts.conversations_parts.chat_filter_label.show(false);
			parts.conversations_parts.chat_filter_combo.show(false);
			parts.conversations_parts.conversations_label.set_label(crate::i18n::tr("Starred Messages"));
			let payload = serde_json::json!({ "action": "get_starred_messages" });
			let _ = state.borrow().ws_tx.send(payload.to_string());
			apply_chat_filter(&mut state.borrow_mut(), parts);
		}
		3 => {
			parts.conversations_parts.panel.show(true);
			state.borrow_mut().active_tab = 3;
			parts.conversations_parts.new_conv_btn.set_label(crate::i18n::tr("New Chat"));
			parts.conversations_parts.post_media_status_btn.show(false);
			parts.conversations_parts.status_privacy_btn.show(false);
			parts.conversations_parts.conversations_label.set_label(crate::i18n::tr("Archived Chats"));
			apply_chat_filter(&mut state.borrow_mut(), parts);
		}
		4 => {
			parts.conversations_parts.panel.show(true);
			state.borrow_mut().active_tab = 4;
			parts.conversations_parts.new_conv_btn.set_label(crate::i18n::tr("New Chat"));
			parts.conversations_parts.post_media_status_btn.show(false);
			parts.conversations_parts.status_privacy_btn.show(false);
			parts.conversations_parts.conversations_label.set_label(crate::i18n::tr("Communities"));
			apply_chat_filter(&mut state.borrow_mut(), parts);
		}
		5 => {
			parts.conversations_parts.panel.show(true);
			state.borrow_mut().active_tab = 5;
			parts.conversations_parts.new_conv_btn.set_label(crate::i18n::tr("Find channels..."));
			parts.conversations_parts.post_media_status_btn.set_label(crate::i18n::tr("Create channel..."));
			parts.conversations_parts.post_media_status_btn.show(true);
			parts.conversations_parts.status_privacy_btn.show(false);
			parts.conversations_parts.conversations_label.set_label(crate::i18n::tr("Channels"));
			apply_chat_filter(&mut state.borrow_mut(), parts);
		}
		6 => {
			parts.conversations_parts.panel.show(true);
			state.borrow_mut().active_tab = 6;
			parts.conversations_parts.post_media_status_btn.show(false);
			parts.conversations_parts.status_privacy_btn.show(false);
			parts.conversations_parts.locked_chats_btn.show(false);
			parts.conversations_parts.chat_filter_label.show(false);
			parts.conversations_parts.chat_filter_combo.show(false);
			parts.conversations_parts.conversation_panel.show(false);
			parts.conversations_parts.conversations_label.set_label(crate::i18n::tr("Recent Calls"));
			parts.conversations_parts.calls_call_btn.set_label(crate::i18n::tr("Call"));
			parts.conversations_parts.calls_video_btn.set_label(crate::i18n::tr("Video call"));
			request_call_tab_data(&state.borrow());
			apply_chat_filter(&mut state.borrow_mut(), parts);
		}
		_ => {}
	}
	parts.content_panel.layout();
	if idx == 0 {
		search_all_messages(state);
	}
}

pub(crate) fn filter_conversations(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, query: String) {
	let mut s = state.borrow_mut();
	// A new search finds contacts too, so fetch the address book afresh.
	if s.search_query.trim().is_empty() && !query.trim().is_empty() && s.active_tab == 0 {
		let _ = s.ws_tx.send(serde_json::json!({ "action": "get_contacts" }).to_string());
	}
	s.search_query = query;
	apply_chat_filter(&mut s, parts);
}

/// Whether the chat list's search also looks through message text: on the
/// Chats tab with no filter, as in WA Web, once the query is long enough.
fn searches_all_messages(s: &AppState) -> bool {
	s.active_tab == 0
		&& s.chat_filter == 0
		&& !s.chats_unlocked
		&& crate::ui::message_display::chat_search_query(s).searches_messages()
}

/// Searches every chat's messages for what's in the chat list's search box,
/// once typing has paused, for the search results' Messages section.
pub(crate) fn search_all_messages(state: &Rc<RefCell<AppState>>) {
	let mut s = state.borrow_mut();
	if !searches_all_messages(&s) {
		s.message_search = crate::MessageSearch::default();
		return;
	}
	if s.message_search.query == s.search_query {
		return;
	}
	s.message_search = crate::MessageSearch { query: s.search_query.clone(), pending: true, ..Default::default() };
	let payload = serde_json::json!({
		"action": "search_messages",
		"data": { "query": s.search_query.trim(), "offset": 0 }
	});
	let _ = s.ws_tx.send(payload.to_string());
}

/// The next page of message results, once the last one shown is reached.
pub(crate) fn more_message_results(state: &Rc<RefCell<AppState>>) {
	let mut s = state.borrow_mut();
	let search = &s.message_search;
	if search.pending || !search.has_more || search.query != s.search_query || !searches_all_messages(&s) {
		return;
	}
	let payload = serde_json::json!({
		"action": "search_messages",
		"data": { "query": search.query.trim(), "offset": search.hits.len() }
	});
	s.message_search.pending = true;
	let _ = s.ws_tx.send(payload.to_string());
}

/// A page of message results for the chat list's search came back.
pub(crate) fn apply_message_search_results(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	data: &serde_json::Value,
) {
	let query = data.get("query").and_then(|q| q.as_str()).unwrap_or("");
	let offset = data.get("offset").and_then(|o| o.as_u64()).unwrap_or(0) as usize;
	let mut s = state.borrow_mut();
	// An answer for an earlier query, or a page already had, is stale.
	if !s.message_search.pending || s.message_search.query.trim() != query || offset != s.message_search.hits.len() {
		return;
	}
	let results = data.get("results").and_then(|r| r.as_array()).cloned().unwrap_or_default();
	let hits: Vec<crate::MessageHit> =
		results.iter().filter_map(|item| crate::backend::events::message_hit(&s, item)).collect();
	s.message_search.hits.extend(hits);
	s.message_search.has_more = data.get("hasMore").and_then(|m| m.as_bool()).unwrap_or(false);
	s.message_search.pending = false;
	apply_chat_filter(&mut s, parts);
}

/// A channel's details, asked for from the chat list without opening the
/// channel, so it isn't marked read.
pub(crate) fn show_channel_info(state: &Rc<RefCell<AppState>>, jid: String) {
	let payload = serde_json::json!({ "action": "get_channel_info", "data": { "jid": jid } });
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

pub(crate) fn open_conversation(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, idx: usize) {
	if state.borrow().active_tab == 1 {
		let message_id = state.borrow().filtered_status_message_ids.get(idx).filter(|id| !id.is_empty()).cloned();
		if let Some(message_id) = message_id {
			let outcome = crate::ui::status_viewer::show(state, parts, &message_id);
			crate::platform::tray::enter_release_guard().set(true);
			apply_chat_filter(&mut state.borrow_mut(), parts);
			if let crate::ui::status_viewer::ViewerOutcome::OpenChat(jid) = outcome {
				nav_selected(state, parts, 0);
				open_chat_for_jid(&jid, state, parts);
			}
		} else {
			let is_me = state.borrow().filtered_chats.get(idx).is_some_and(|j| j == "me");
			if is_me {
				crate::ui::dialogs::status_composer::show_status_composer_choice(&parts.frame, state, parts);
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
			let is_video = {
				let s = state.borrow();
				s.filtered_call_ids
					.get(idx)
					.filter(|id| !id.is_empty())
					.and_then(|id| s.call_logs.iter().find(|entry| entry.id.as_str() == id.as_str()))
					.map(|entry| entry.is_video)
					.unwrap_or(false)
			};
			state.borrow_mut().selected_chat = Some(jid);
			if is_video {
				crate::ui_commands::calls::place_video_call(state, parts);
			} else {
				crate::ui_commands::calls::place_call(state, parts);
			}
		}
		return;
	}
	// Headings and notices have no chat behind them.
	let jid = state.borrow().filtered_chats.get(idx).filter(|j| !j.is_empty()).cloned();
	if let Some(ref jid) = jid {
		let msg_id = {
			let s = state.borrow();
			match s.active_tab {
				2 => s.filtered_starred_message_ids.get(idx).cloned(),
				_ => s.filtered_message_ids.get(idx).filter(|id| !id.is_empty()).cloned(),
			}
		};
		if let Some(msg_id) = msg_id {
			state.borrow_mut().search_target_id = Some(msg_id);
		}
		open_chat_for_jid(jid, state, parts);
		if state.borrow().community_jids.contains(jid) {
			show_conversation_data(state, parts);
		}
	}
}

pub(crate) fn conversation_selection_changed(idx: usize, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let active_tab = state.borrow().active_tab;
	// Reaching the last message a search found loads the next page of them.
	let last_message = {
		let s = state.borrow();
		idx + 1 == s.filtered_chats.len() && s.filtered_message_ids.get(idx).is_some_and(|id| !id.is_empty())
	};
	if last_message {
		more_message_results(state);
	}
	if active_tab == 6 {
		let jid = state.borrow().filtered_chats.get(idx).filter(|j| !j.is_empty()).cloned();
		if let Some(jid) = jid {
			if !jid.starts_with(crate::ui_commands::calls::JOIN_ROW_PREFIX) {
				state.borrow_mut().selected_chat = Some(jid.clone());
				let s = state.borrow();
				let peer_name = s.contacts.get(&jid).cloned().unwrap_or_else(|| {
					let raw_phone = jid.split('@').next().unwrap_or(&jid);
					whatsnap::phone::format_international(raw_phone).unwrap_or_else(|| format!("+{raw_phone}"))
				});
				drop(s);
				let (call_lbl, video_lbl) = (crate::i18n::tr_fmt("Call {}", &[&peer_name]), crate::i18n::tr_fmt("Video call {}", &[&peer_name]));
				parts.conversations_parts.calls_call_btn.set_label(&call_lbl);
				parts.conversations_parts.calls_video_btn.set_label(&video_lbl);
				parts.conversations_parts.calls_btn_panel.layout();
			}
		}
	}
}

const GROUPS_TAB: usize = 5;

pub(crate) fn open_chat_by_phone(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, digits: String) {
	let jid = format!("{}@s.whatsapp.net", digits);
	open_chat_for_jid(&jid, state, parts);
}

pub(crate) fn open_chat_by_jid(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String) {
	open_chat_for_jid(&jid, state, parts);
}

pub(crate) fn toggle_selection_mode(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let on = !state.borrow().selection_mode;
	set_selection_mode(state, parts, on, true);
}

/// Turns selection mode on or off. `speak` says so; the select bar's actions
/// turn it off quietly and announce what they did instead.
pub(crate) fn set_selection_mode(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, on: bool, speak: bool) {
	let mut s = state.borrow_mut();
	if s.selection_mode == on {
		return;
	}
	s.selection_mode = on;
	let cp = &parts.conversations_parts;
	if on {
		s.selected_message_ids.clear();
		refresh_selection_list(&mut s, parts);
		cp.messages_list.show(false);
		cp.messages_select_list.show(true);
		cp.selection_bar_panel.show(true);
		cp.message_field.show(false);
		cp.add_attachment_btn.show(false);
		cp.emoji_btn.show(false);
		cp.send_message_btn.show(false);
		cp.record_voice_btn.show(false);
		crate::ui_commands::selection::update_bar(&s, cp);
		drop(s);
		cp.messages_select_list.set_focus();
		if speak {
			crate::platform::announce::announce(
				cp.messages_announcer,
				crate::i18n::tr("Selection mode. Press Space to select a message. 0 messages selected."),
			);
		}
	} else {
		let at = cp
			.messages_select_list
			.get_selection()
			.and_then(|i| s.selection_rows.get(i as usize))
			.and_then(|item| s.display_items.iter().position(|d| d.id() == item.id()));
		if let Some(pos) = at {
			cp.messages_list.set_selection(pos as u32, true);
		}
		s.selected_message_ids.clear();
		s.selection_rows = Vec::new();
		crate::ui::chat_ui::update_composer_buttons(&s, cp);
		drop(s);
		cp.messages_select_list.clear();
		cp.messages_select_list.show(false);
		cp.selection_bar_panel.show(false);
		cp.messages_list.show(true);
		cp.messages_list.set_focus();
		if speak {
			crate::platform::announce::announce(cp.messages_announcer, crate::i18n::tr("Selection mode off."));
		}
	}
	cp.conversation_panel.layout();
}

/// Asks for a date (YYYY-MM-DD) and shows the open chat around its first
/// message on or after that day.
pub(crate) fn go_to_date(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let Some(jid) = state.borrow().selected_chat.clone() else { return };
	let dlg = TextEntryDialog::builder(
		&parts.frame,
		crate::i18n::tr("Date to go to, as year-month-day, like 2026-03-27:"),
		crate::i18n::tr("Go to Date"),
	)
	.build();
	let answer = (dlg.show_modal() == wxdragon::id::ID_OK).then(|| dlg.get_value()).flatten();
	dlg.destroy();
	let Some(answer) = answer else { return };
	let say = |text: &str| crate::platform::announce::announce(parts.conversations_parts.messages_announcer, text);
	let nums: Vec<u32> = answer.trim().split(['-', '/', '.']).filter_map(|p| p.trim().parse().ok()).collect();
	let valid = answer.trim().split(['-', '/', '.']).count() == 3
		&& nums.len() == 3
		&& (1..=12).contains(&nums[1])
		&& (1..=31).contains(&nums[2])
		&& nums[0] >= 2009;
	if !valid {
		say(crate::i18n::tr("That is not a date. Use year-month-day, like 2026-03-27."));
		return;
	}
	let date = format!("{:04}-{:02}-{:02}", nums[0], nums[1], nums[2]);
	let payload = serde_json::json!({
		"action": "get_messages_around",
		"data": { "jid": jid, "date": date, "context": crate::MESSAGE_PAGE_SIZE / 2 }
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	let say_msg =
		crate::i18n::tr_fmt("Going to {}.", &[&date]);
	say(&say_msg);
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
			crate::i18n::tr("This is the start of the chat."),
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
		crate::platform::announce::announce(
			parts.conversations_parts.messages_announcer,
			crate::i18n::tr("No chat open."),
		);
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

/// The Locked Chats button: Windows Hello when the user turned it on and it
/// can be used, otherwise the secret code.
pub(crate) fn prompt_locked_chats(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let (use_hello, ui_tx) = {
		let s = state.borrow();
		(s.settings.chat_lock_hello, s.ui_tx.clone())
	};
	if use_hello && windows_hello::availability() == windows_hello::Availability::Ready {
		let started = windows_hello::verify(
			crate::i18n::tr("Unlock your locked chats in whatSnap").to_string(),
			move |outcome| {
				let _ = ui_tx.send(UiCommand::ChatLockHello { outcome });
			},
		);
		if started {
			return;
		}
	}
	ask_secret_code(state, parts);
}

/// The Windows Hello prompt finished. A pass shows the chats; an explicit cancel
/// leaves chats locked; failure falls back to the secret code.
pub(crate) fn finish_chat_lock_hello(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	outcome: windows_hello::VerifyOutcome,
) {
	// The Hello prompt has closed. Give whatSnap the keyboard back and put
	// focus on the button that opened it, so a screen reader user isn't left
	// with nothing focused.
	let frame = windows::Win32::Foundation::HWND(parts.frame.get_handle());
	crate::platform::focus::bring_to_front(frame);
	parts.conversations_parts.locked_chats_btn.set_focus();
	match outcome {
		windows_hello::VerifyOutcome::Verified => {
			show_locked_chats(state, parts);
		}
		windows_hello::VerifyOutcome::Canceled => {
			state.borrow_mut().pending_locked_chat = None;
		}
		windows_hello::VerifyOutcome::Failed => {
			ask_secret_code(state, parts);
		}
	}
}

/// Shows the locked chats. Called once the user has proved they may see them.
pub(crate) fn show_locked_chats(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let pending_chat = {
		let mut s = state.borrow_mut();
		s.chats_unlocked = true;
		s.search_query.clear();
		parts.conversations_parts.search_field.set_value("");
		apply_chat_filter(&mut s, parts);
		parts.conversations_parts.locked_chats_btn.set_label(crate::i18n::tr("Hide Locked Chats"));
		crate::platform::announce::announce(
			parts.conversations_parts.messages_announcer,
			crate::i18n::tr("Locked chats shown."),
		);
		s.pending_locked_chat.take()
	};
	if let Some(jid) = pending_chat {
		open_chat_for_jid(&jid, state, parts);
	}
}

/// After the secret code works for the first time, offer Windows Hello for
/// next time. Asked once, whatever the answer; if Windows Hello isn't ready
/// yet, the offer waits for a later unlock.
pub(crate) fn offer_chat_lock_hello(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	if state.borrow().settings.chat_lock_hello_offered {
		return;
	}
	if windows_hello::availability() != windows_hello::Availability::Ready {
		return;
	}
	let question = MessageDialog::builder(
		&parts.frame,
		crate::i18n::tr(
			"Unlock your chats faster. Next time, use Windows Hello, with your fingerprint, face or PIN, instead of typing the secret code. You can change this in Settings, Chats.\n\nTurn on Windows Hello for locked chats?",
		),
		crate::i18n::tr("Unlock Your Chats Faster"),
	)
	.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
	.build();
	// The dialog is up with no state borrow held, so it can't clash with the UI.
	let answer = question.show_modal();
	let mut s = state.borrow_mut();
	s.settings.chat_lock_hello_offered = true;
	if answer == wxdragon::id::ID_YES {
		s.settings.chat_lock_hello = true;
	}
	s.settings.save();
}

fn ask_secret_code(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let dlg = TextEntryDialog::builder(
		&parts.frame,
		crate::i18n::tr("Enter your WhatsApp Secret Code configured on your phone:"),
		crate::i18n::tr("Unlock Locked Chats"),
	)
	.password()
	.build();
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
			return;
		}
	}
	state.borrow_mut().pending_locked_chat = None;
}

pub(crate) fn close_locked_chats(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let should_close_conv = {
		let mut s = state.borrow_mut();
		if s.chats_unlocked {
			s.chats_unlocked = false;
			s.pending_locked_chat = None;
			parts.conversations_parts.locked_chats_btn.set_label(crate::i18n::tr("Locked Chats"));
			apply_chat_filter(&mut s, parts);
			crate::platform::announce::announce(
				parts.conversations_parts.messages_announcer,
				crate::i18n::tr("Locked chats hidden."),
			);
			s.selected_chat.as_ref().is_some_and(|j| s.locked_jids.contains(j))
		} else {
			false
		}
	};
	if should_close_conv {
		close_conversation(state, parts);
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
	// Back to the chat's messages, rather than wherever the hidden box leaves it.
	parts.conversations_parts.messages_list.set_focus();
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
			parts.group_data_dialog.groups_label.set_label(crate::i18n::tr("Groups in this community"));
			parts.group_data_dialog.join_group_btn.set_label(crate::i18n::tr("Join group"));
			parts.group_data_dialog.notebook.set_selection(if s.community_jids.contains(jid) { GROUPS_TAB } else { 0 });

			parts.group_data_dialog.photo.show(false);
			let payload = serde_json::json!({ "action": "get_avatar", "data": { "jid": jid } });
			let _ = s.ws_tx.send(payload.to_string());
			let jid_for_overview = jid.clone();
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

			crate::ui::chat_overview::refresh(state, &parts.group_data_dialog.overview, &jid_for_overview);
			crate::ui::show_modal_again(&parts.group_data_dialog.dialog);
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

	let initial_text = crate::i18n::tr_fmt("Name: {}\nPhone: {}\n{}WhatsApp Name: Loading...\nAbout/Status: Loading...", &[&local_name, &phone, &crate::ui_commands::blocking::info_line(&s, &jid)]);
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
	dlg.common_groups_list.append(crate::i18n::tr("Loading..."));

	let lists = InfoMediaLists { media: &dlg.media_list, documents: &dlg.documents_list, links: &dlg.links_list };
	let (media, documents, links) = if s.selected_chat.as_deref() == Some(jid.as_str()) {
		fill_info_media(&s, lists, None)
	} else {
		let payload = serde_json::json!({ "action": "get_chat_media", "data": { "jid": jid } });
		let _ = s.ws_tx.send(payload.to_string());
		for list in [lists.media, lists.documents, lists.links] {
			list.clear();
			list.append(crate::i18n::tr("Loading..."));
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

	parts.conversation_data_dialog.notebook.set_selection(0);
	parts.conversation_data_dialog.overview.summary.set_focus();
	let overview_jid = state.borrow().info_chat.clone();
	if let Some(jid) = overview_jid {
		crate::ui::chat_overview::refresh(state, &parts.conversation_data_dialog.overview, &jid);
	}
	crate::ui::show_modal_again(&parts.conversation_data_dialog.dialog);
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
		parts.conversations_parts.message_label.set_label(crate::i18n::tr("Type a message"));
		parts.conversations_parts.message_field.set_value("");
	}
	save_composer_draft(&mut s, parts);
	// A voice message left paused (or playing) must not carry on, or wake
	// up on a later arrow key, once its chat is closed.
	crate::ui_commands::voice::stop_voice_message(&mut s, parts);
	s.selected_chat = None;
	crate::ui::window::refresh_menu_bar(&s, &parts.frame);
	// Its unsent text now shows in the chat list as a draft.
	crate::ui::message_display::refresh_chat_rows(&mut s, parts);
	clear_pending_reply(&mut s, parts);
	parts.conversations_parts.conversation_panel.show(false);
	release_conversation_buffers(&mut s, parts);
	parts.conversations_parts.panel.layout();
	parts.conversations_parts.conversations_list.set_focus();
}

pub(crate) fn release_conversation_buffers(s: &mut AppState, parts: &Rc<WindowParts>) {
	if s.is_voice_recording {
		crate::ui_commands::voice::stop_voice_preview(s, parts);
		s.is_voice_recording = false;
		crate::ui::conversations::set_recording_clock(None);
		s.is_voice_paused = false;
		s.voice_recording_generation += 1;
		s.voice_recording_opening = false;
		if let Some(session) = s.voice_recording_session.take() {
			session.finish();
		}
		if let Some(path) = s.voice_recording_path.take() {
			let _ = std::fs::remove_file(path);
		}
		for seg in s.voice_recording_segments.drain(..) {
			let _ = std::fs::remove_file(seg);
		}
		parts.conversations_parts.discard_voice_btn.show(false);
		parts.conversations_parts.pause_resume_btn.show(false);
		parts.conversations_parts.preview_voice_btn.show(false);
		parts.conversations_parts.view_once_voice_check.show(false);
		parts.conversations_parts.send_voice_btn.show(false);
		parts.conversations_parts.record_voice_btn.enable(true);
		parts.conversations_parts.pause_resume_btn.enable(true);
	}
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
	let labels: Vec<&str> = CHOICES.iter().map(|(label, _)| crate::i18n::tr(label)).collect();
	let prompt = crate::i18n::tr_fmt("Make new messages in {} disappear after:", &[&chat_name]);
	let dialog =
		SingleChoiceDialog::builder(&parts.frame, &prompt, crate::i18n::tr("Disappearing Messages"), &labels).build();
	if dialog.show_modal() != wxdragon::id::ID_OK {
		return;
	}
	let sel = dialog.get_selection();
	if sel < 0 {
		return;
	}
	let idx = sel as usize;
	let Some(&(label, seconds)) = CHOICES.get(idx) else { return };
	let localized_label = crate::i18n::tr(label);
	let payload = serde_json::json!({
		"action": "set_disappearing_timer",
		"data": { "jid": jid, "seconds": seconds }
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	let spoken = if seconds == 0 {
		crate::i18n::tr_fmt("Disappearing messages turned off for {}.", &[&chat_name])
	} else {
		crate::i18n::tr_fmt("Disappearing messages set to {} for {}.", &[&localized_label, &chat_name])
	};
	crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &spoken);
}

pub(crate) fn toggle_lock_chat(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String, lock: bool) {
	let (chat_name, should_close_conv) = {
		let mut s = state.borrow_mut();
		if lock {
			s.locked_jids.insert(jid.clone());
		} else {
			s.locked_jids.remove(&jid);
		}
		apply_chat_filter(&mut s, parts);
		let name = s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
		let close = lock && s.selected_chat.as_deref() == Some(&jid) && !s.chats_unlocked;
		(name, close)
	};
	if should_close_conv {
		close_conversation(state, parts);
	}
	let payload = serde_json::json!({
		"action": "set_chat_locked",
		"data": {
			"jid": jid,
			"value": lock
		}
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	let msg = crate::i18n::tr_fmt(if lock { "Locked chat {}." } else { "Unlocked chat {}." }, &[&chat_name]);
	crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &msg);
}

pub(crate) fn toggle_pin_chat(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String, pin: bool) {
	let chat_name = {
		let mut s = state.borrow_mut();
		if pin {
			s.pinned_jids.insert(jid.clone());
			// Pinning unarchives too, as the backend sends it (a chat is
			// never both), so the list shows it at once.
			s.archived_jids.remove(&jid);
		} else {
			s.pinned_jids.remove(&jid);
		}
		let name = s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
		apply_chat_filter(&mut s, parts);
		name
	};
	let payload = serde_json::json!({
		"action": "set_chat_pinned",
		"data": {
			"jid": jid,
			"value": pin
		}
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	let msg = crate::i18n::tr_fmt(if pin { "Chat {} pinned." } else { "Chat {} unpinned." }, &[&chat_name]);
	crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &msg);
}

pub(crate) fn toggle_archive_chat(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String, archive: bool) {
	let chat_name = {
		let mut s = state.borrow_mut();
		if archive {
			s.archived_jids.insert(jid.clone());
			// Archiving unpins, as the backend does.
			s.pinned_jids.remove(&jid);
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
	let msg = crate::i18n::tr_fmt(if archive { "Chat {} archived." } else { "Chat {} unarchived." }, &[&chat_name]);
	crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &msg);
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
			whatsnap::ArchivedAlert::Silent => crate::i18n::tr("silent"),
			whatsnap::ArchivedAlert::SoundOnly => crate::i18n::tr("sound only"),
			whatsnap::ArchivedAlert::Full => crate::i18n::tr("notify normally"),
		};
		(name, spoken)
	};
	let msg = crate::i18n::tr_fmt("Archived alerts for {} set to {}.", &[&chat_name, &spoken]);
	crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &msg);
}

pub(crate) fn set_chat_notify(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	jid: String,
	notify: whatsnap::ChatNotify,
) {
	let spoken = {
		let mut s = state.borrow_mut();
		s.settings.set_chat_notify(&jid, notify.clone());
		s.settings.save();
		refresh_tray_tooltip(&s, parts);
		let name = s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
		let alert = match notify.alert {
			whatsnap::ChatAlert::Default => crate::i18n::tr("as set in Settings"),
			whatsnap::ChatAlert::Full => crate::i18n::tr("notification and sound"),
			whatsnap::ChatAlert::NoSound => crate::i18n::tr("notification only, no sound"),
			whatsnap::ChatAlert::SoundOnly => crate::i18n::tr("sound only, no notification"),
			whatsnap::ChatAlert::Off => crate::i18n::tr("off on this computer"),
		};
		{
			let content = if notify.hide_content { crate::i18n::tr(", message content hidden") } else { "" };
			crate::i18n::tr_fmt("Notifications for {}: {}{}.", &[&name, &alert, &content])
		}
	};
	crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &spoken);
}

pub(crate) fn show_chat_notify(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String) {
	let (name, current, default_tone, default_ringtone, output_device) = {
		let s = state.borrow();
		let group = jid.ends_with("@g.us");
		let default_tone = if group {
			(s.settings.group_tone, s.settings.group_tone_file.clone())
		} else {
			(s.settings.message_tone, s.settings.message_tone_file.clone())
		};
		let default_ringtone =
			if group { s.settings.group_call_ringtone_sound.clone() } else { s.settings.call_ringtone_sound.clone() };
		let name = s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
		(name, s.settings.chat_notify(&jid), default_tone, default_ringtone, s.settings.output_device_name.clone())
	};
	let Some(notify) = crate::ui::dialogs::chat_notify_dialog::ask(
		&parts.frame,
		&name,
		current.clone(),
		default_tone,
		default_ringtone,
		output_device,
	) else {
		return;
	};
	// A custom tone is this computer's own; WhatsApp has nothing to hold it.
	if notify.tone != current.tone && notify.tone != whatsnap::TONE_CUSTOM {
		// Shared like WhatsApp for Windows does, as this chat's
		// notificationToneId (WAWebChatNotificationToneSetting).
		let payload = serde_json::json!({"action": "sync_chat_tone", "data": {"jid": jid, "tone": notify.tone}});
		let _ = state.borrow().ws_tx.send(payload.to_string());
	}
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
		builder = builder.append_item(
			first_id + i as i32,
			crate::i18n::tr(label),
			crate::i18n::tr("Mute notifications from this chat"),
		);
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
	let spoken = {
		let how_long = MUTE_DURATIONS.iter().find(|(_, _, s)| *s == seconds).map_or("", |(_, how_long, _)| how_long);
		if mute {
			crate::i18n::tr_fmt("Notifications from {} muted{}.", &[&chat_name, &crate::i18n::tr_str(how_long)])
		} else {
			crate::i18n::tr_fmt("Notifications from {} unmuted.", &[&chat_name])
		}
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
		apply_chat_filter(&mut s, parts);
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
	let msg = if read {
		crate::i18n::tr_fmt("Chat {} marked as read.", &[&chat_name])
	} else {
		crate::i18n::tr_fmt("Chat {} marked as unread.", &[&chat_name])
	};
	crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &msg);
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
	let msg = crate::i18n::tr_fmt("Chat {} deleted.", &[&chat_name]);
	crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &msg);
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
	let msg = crate::i18n::tr_fmt("Left and deleted chat {}.", &[&chat_name]);
	crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &msg);
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
	let msg = crate::i18n::tr_fmt("Chat {} cleared.", &[&chat_name]);
	crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &msg);
}
