use super::*;

pub(super) fn drop_seen_ids<'a>(msgs: Vec<ChatMessage>, seen: impl Iterator<Item = &'a str>) -> Vec<ChatMessage> {
	let keep: Vec<bool> = {
		let mut seen: std::collections::HashSet<&str> = seen.collect();
		msgs.iter().map(|m| m.id.is_empty() || seen.insert(m.id.as_str())).collect()
	};
	msgs.into_iter().zip(keep).filter_map(|(m, keep)| keep.then_some(m)).collect()
}

pub(super) fn trim_window(messages: Vec<whatsnap::Msg>, keep_start: bool) -> (Vec<whatsnap::Msg>, usize) {
	if whatsnap::real_message_count(&messages) <= crate::MAX_LOADED_MESSAGES {
		return (messages, 0);
	}
	let rows_before = whatsnap::group_messages_into_display_items(&messages).len();
	let (kept, _) = whatsnap::trim_real_messages(messages, crate::MAX_LOADED_MESSAGES, keep_start);
	let rows_after = whatsnap::group_messages_into_display_items(&kept).len();
	(kept, rows_before.saturating_sub(rows_after))
}

pub(super) fn apply_newer_messages(page: MessagesPage, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let mut s = state.borrow_mut();
	s.newer_loading = false;
	if page.is_latest {
		s.messages_at_latest = true;
	}
	if let Some(newer) = page.newer {
		s.newer_unloaded = newer;
	}
	let parsed_msgs = page.messages;
	let chat_jid = s.selected_chat.clone().unwrap_or_default();
	let is_group = chat_jid.ends_with("@g.us");
	let chat_name = s.contacts.get(&chat_jid).cloned().unwrap_or_else(|| chat_jid.clone());
	let mut added = drop_seen_ids(parsed_msgs, s.messages.iter().map(|m| m.id.as_str()));
	if added.is_empty() {
		return;
	}
	for msg in &mut added {
		if !is_group && !msg.from_me && msg.sender_name.is_empty() && msg.sender_jid.is_empty() {
			msg.sender_name = chat_name.clone();
		}
		if let Some(st) = s.pending_receipts.remove(&msg.id)
			&& get_status_rank(&st) >= get_status_rank(&msg.status)
		{
			msg.status = st;
		}
	}
	let mut merged = std::mem::take(&mut s.messages);
	merged.extend(added.into_iter().map(whatsnap::Msg::new));
	let (merged, dropped) = trim_window(whatsnap::rebuild_date_dividers(merged), false);
	if dropped > 0 {
		s.older_unloaded += dropped;
		s.no_more_messages = false;
	}
	s.messages = merged;

	let old_display_items = std::mem::take(&mut s.display_items);
	let new_display_items = whatsnap::group_messages_into_display_items(&s.messages);
	let old_len = old_display_items.len();
	let append_only = new_display_items.len() > old_len
		&& new_display_items[..old_len].iter().zip(&old_display_items).all(|(a, b)| a.id() == b.id());
	let list = &parts.conversations_parts.messages_list;
	list.set_numbering(s.older_unloaded, s.newer_unloaded);
	list.freeze();
	if append_only {
		let rows: Vec<String> = new_display_items[old_len..].iter().map(|d| d.format_display(is_group)).collect();
		list.append_items(&rows.iter().map(String::as_str).collect::<Vec<_>>());
	} else {
		let selected_id = list
			.get_selection()
			.and_then(|i| old_display_items.get(i as usize))
			.map(|d| d.id().to_string())
			.filter(|id| !id.is_empty());
		let rows: Vec<String> = new_display_items.iter().map(|d| d.format_display(is_group)).collect();
		list.set_items(&rows.iter().map(String::as_str).collect::<Vec<_>>());
		if let Some(pos) = selected_id.and_then(|id| new_display_items.iter().position(|d| d.id() == id)) {
			list.set_selection(pos as u32, true);
		}
	}
	list.thaw();
	s.prev_msg_count = new_display_items.len();
	s.display_items = new_display_items;
	refresh_selection_list(&mut s, parts);
	crate::ui_commands::calls::mark_joinable_rows(&s, parts);
}

pub(super) fn apply_conversation_messages(page: MessagesPage, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let started = std::time::Instant::now();
	let load_more = page.load_more;
	let mut deduplicated = drop_seen_ids(page.messages, std::iter::empty());

	let mut s = state.borrow_mut();
	let chat_jid = s.selected_chat.clone().unwrap_or_default();
	let chat_name = s.contacts.get(&chat_jid).cloned().unwrap_or_else(|| chat_jid.clone());
	let is_group_chat = chat_jid.ends_with("@g.us");

	if load_more {
		// An offset-based local page should never overlap what's already
		// loaded, but a page fetched after an on-demand phone request can:
		// the phone's answer lands as ordinary messages the backend already
		// wrote to the same table, so a plain offset re-query can hand back
		// rows this chat already has. Drop those here rather than let them
		// double up in the merged list.
		let already_loaded: std::collections::HashSet<&str> = s.messages.iter().map(|m| m.id.as_str()).collect();
		deduplicated.retain(|m| m.id.is_empty() || !already_loaded.contains(m.id.as_str()));
	}

	if load_more && deduplicated.is_empty() {
		if s.phone_history_pending_jid.as_deref() == Some(chat_jid.as_str()) {
			eprintln!("[LOG UI] Phone sent nothing new for {} this time.", chat_jid);
			s.phone_history_pending_jid = None;
			s.phone_history_deadline = None;
			s.history_loading = false;
		} else {
			eprintln!("[LOG UI] Local database has no older history for {} yet; asking the phone.", chat_jid);
			s.no_more_messages = true;
			crate::ui_commands::chat_list::request_phone_history_sync(&mut s, &chat_jid);
		}
		return;
	}

	let mut with_dates = Vec::new();
	let mut last_date: Option<whatsnap::Date> = None;
	for mut msg in deduplicated {
		if !is_group_chat && !msg.from_me && msg.sender_name.is_empty() && msg.sender_jid.is_empty() {
			msg.sender_name = chat_name.clone();
		}

		if let Some(local_datetime) = whatsnap::LocalTime::from_unix(msg.timestamp) {
			let day = whatsnap::date_key(local_datetime);
			if last_date != Some(day) {
				last_date = Some(day);
				with_dates.push(whatsnap::Msg::new(ChatMessage {
					text: whatsnap::format_date_divider_label(local_datetime),
					timestamp: msg.timestamp,
					is_date_divider: true,
					..Default::default()
				}));
			}
		}
		with_dates.push(whatsnap::Msg::new(msg));
	}

	if !load_more && s.messages_at_latest && s.unread_divider_jid.as_deref() == Some(chat_jid.as_str()) {
		whatsnap::insert_unread_divider(&mut with_dates, s.unread_divider_count);
	}

	s.loading_messages = false;
	s.no_more_messages = page.older.is_none_or(|older| older == 0);
	if let Some(older) = page.older {
		s.older_unloaded = older;
	}
	if !load_more {
		s.newer_unloaded = page.newer.unwrap_or(0);
	}
	let quiet = !load_more && s.quiet_reload_jid.take().as_deref() == Some(chat_jid.as_str());
	let was_history_loading = s.history_loading;
	s.history_loading = false;
	let old_count = s.prev_msg_count;
	let is_group = s.selected_chat.as_ref().map(|j| j.ends_with("@g.us")).unwrap_or(false);
	if load_more {
		s.phone_history_pending_jid = None;
		s.phone_history_deadline = None;
		with_dates.append(&mut s.messages);
		with_dates = whatsnap::rebuild_date_dividers(with_dates);
		let dropped;
		(with_dates, dropped) = trim_window(with_dates, true);
		if dropped > 0 {
			s.newer_unloaded += dropped;
			s.messages_at_latest = false;
		}
	}
	s.messages = with_dates;
	let from_phone = chat_jid.ends_with("@g.us") || chat_jid.ends_with("@s.whatsapp.net") || chat_jid.ends_with("@lid");
	if !load_more
		&& from_phone
		&& whatsnap::real_message_count(&s.messages) == 0
		&& s.empty_chat_history_asked.insert(chat_jid.clone())
	{
		s.history_loading = true;
		crate::ui_commands::chat_list::request_phone_history_sync(&mut s, &chat_jid);
	}
	let pending_ids: Vec<(String, String)> = s
		.messages
		.iter()
		.filter_map(|msg| s.pending_receipts.get(&msg.id).map(|st| (msg.id.clone(), st.clone())))
		.collect();
	for (id, st) in pending_ids {
		s.pending_receipts.remove(&id);
		if let Some(msg) = s.messages.iter_mut().find(|m| m.id == id)
			&& get_status_rank(&st) >= get_status_rank(&msg.status)
		{
			std::rc::Rc::make_mut(msg).status = st;
		}
	}
	let old_display_items = std::mem::take(&mut s.display_items);
	let new_display_items = whatsnap::group_messages_into_display_items(&s.messages);
	let new_len = new_display_items.len();
	s.prev_msg_count = new_len;

	let prepend = if load_more { whatsnap::prepend_plan(&old_display_items, &new_display_items) } else { None };

	let shown = match prepend {
		Some(plan) => &new_display_items[..plan.insert],
		None => &new_display_items[..],
	};
	let rows: Vec<String> = shown.iter().map(|item| item.format_display(is_group)).collect();
	let row_refs: Vec<&str> = rows.iter().map(String::as_str).collect();
	// Rows are keyed by message, so a rebuild keeps the cursor (and the
	// screen reader's object for it) on the same message.
	let keys: Vec<String> = shown.iter().zip(&rows).map(|(item, text)| message_row_key(item, text)).collect();
	let key_refs: Vec<&str> = keys.iter().map(String::as_str).collect();
	let cursor_id = parts
		.conversations_parts
		.messages_list
		.get_selection()
		.and_then(|i| old_display_items.get(i as usize..))
		.and_then(|rest| rest.iter().map(|d| d.id()).find(|id| !id.is_empty()))
		.map(|id| id.to_string());
	let prepared = started.elapsed();
	parts.conversations_parts.messages_list.set_numbering(s.older_unloaded, s.newer_unloaded);
	parts.conversations_parts.messages_list.freeze();
	if let Some(plan) = prepend {
		for _ in 0..plan.remove_leading {
			parts.conversations_parts.messages_list.delete(0);
		}
		parts.conversations_parts.messages_list.insert_keyed_items(&key_refs, &row_refs, 0);
	} else {
		parts.conversations_parts.messages_list.set_keyed_items(&key_refs, &row_refs);
	}
	parts.conversations_parts.messages_list.thaw();
	ui_timing_log(&format!(
		"Showed {} rows for {} ({}): preparing {} ms, filling the list {} ms",
		new_len,
		chat_jid,
		if prepend.is_some() {
			"prepended"
		} else if load_more {
			"rebuilt, older page"
		} else {
			"full load"
		},
		prepared.as_millis(),
		(started.elapsed() - prepared).as_millis()
	));
	s.display_items = new_display_items;
	refresh_selection_list(&mut s, parts);
	crate::ui_commands::calls::mark_joinable_rows(&s, parts);

	let count = parts.conversations_parts.messages_list.get_count();
	let target_pos = if let Some(target_id) = s.search_target_id.take() {
		s.display_items.iter().position(|item| match item {
			whatsnap::DisplayItem::Single(m) => m.id == target_id,
			whatsnap::DisplayItem::Album { messages, .. } => messages.iter().any(|m| m.id == target_id),
		})
	} else if let Some(target_text) = s.search_target_text.take() {
		let voice_tr = crate::i18n::tr("Voice message");
		let audio_tr = crate::i18n::tr("Audio");
		let is_generic = target_text == "[Voice Message]"
			|| target_text == "[Audio]"
			|| target_text == "Voice message"
			|| target_text == voice_tr
			|| target_text == "Audio"
			|| target_text == audio_tr
			|| target_text == "[Media]";
		if is_generic {
			None
		} else {
			s.display_items.iter().rposition(|item| match item {
				whatsnap::DisplayItem::Single(m) => {
					m.text == target_text || (!m.text.is_empty() && m.text.contains(&target_text))
				}
				whatsnap::DisplayItem::Album { messages, .. } => messages
					.iter()
					.any(|m| m.text == target_text || (!m.text.is_empty() && m.text.contains(&target_text))),
			})
		}
	} else if old_count == 0 && s.unread_divider_jid.as_deref() == Some(chat_jid.as_str()) {
		s.display_items.iter().position(|item| matches!(item, whatsnap::DisplayItem::Single(m) if m.is_unread_divider))
	} else {
		None
	};
	if let Some(pos) = target_pos {
		parts.conversations_parts.messages_list.set_selection(pos as u32, true);
		if !quiet {
			parts.conversations_parts.messages_list.set_focus();
		}
	} else if count > 0 {
		if parts.conversations_parts.messages_list.get_selection().is_some() && old_count > 0 {
		} else if was_history_loading && load_more && old_count > 0 {
			let keep_idx =
				cursor_id.and_then(|id| s.display_items.iter().position(|d| d.id() == id)).unwrap_or(0) as u32;
			if keep_idx < count {
				parts.conversations_parts.messages_list.set_selection(keep_idx, true);
			} else {
				parts.conversations_parts.messages_list.set_selection(0, true);
			}
		} else if old_count == 0 {
			let row = if std::mem::take(&mut s.land_on_first) { 0 } else { count - 1 };
			parts.conversations_parts.messages_list.set_selection(row, true);
			if !quiet {
				parts.conversations_parts.messages_list.set_focus();
			}
		}
	}
	parts.conversations_parts.conversation_panel.freeze();
	if update_action_buttons(&s, &parts.conversations_parts) {
		parts.conversations_parts.conversation_panel.layout();
	}
	parts.conversations_parts.conversation_panel.thaw();
	refresh_voice_action_button(&s, &parts.conversations_parts);
}

#[derive(Deserialize)]
pub(super) struct TypeTag<'a> {
	#[serde(rename = "type", borrow, default)]
	ty: std::borrow::Cow<'a, str>,
}

#[derive(Deserialize)]
pub(super) struct ConversationMessagesEnvelope<'a> {
	#[serde(borrow)]
	data: ConversationMessagesData<'a>,
}

#[derive(Deserialize)]
pub(super) struct ConversationMessagesData<'a> {
	#[serde(default)]
	jid: String,
	#[serde(borrow, default)]
	messages: Vec<&'a serde_json::value::RawValue>,
	#[serde(rename = "loadMore", default)]
	load_more: bool,
	#[serde(rename = "loadNewer", default)]
	load_newer: bool,
	#[serde(rename = "isLatest", default = "default_true")]
	is_latest: bool,
	#[serde(default)]
	older: Option<usize>,
	#[serde(default)]
	newer: Option<usize>,
	#[serde(rename = "targetFound", default = "default_true")]
	target_found: bool,
	#[serde(rename = "targetId", default)]
	target_id: Option<String>,
}

pub(super) struct MessagesPage {
	pub(super) jid: String,
	pub(super) load_more: bool,
	pub(super) load_newer: bool,
	pub(super) is_latest: bool,
	pub(super) older: Option<usize>,
	pub(super) newer: Option<usize>,
	pub(super) target_found: bool,
	/// The message a Go to Date jump landed on.
	pub(super) target_id: Option<String>,
	pub(super) messages: Vec<ChatMessage>,
}

pub(super) fn default_true() -> bool {
	true
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum FastPath {
	ConversationMessages,
	Chats,
	Other,
}

pub(super) fn fast_path(raw: &str) -> FastPath {
	match serde_json::from_str::<TypeTag>(raw).as_ref().map(|t| t.ty.as_ref()) {
		Ok("conversation_messages") => FastPath::ConversationMessages,
		Ok("chats") => FastPath::Chats,
		_ => FastPath::Other,
	}
}

#[cfg(test)]
pub(super) fn is_conversation_messages(raw: &str) -> bool {
	fast_path(raw) == FastPath::ConversationMessages
}

pub(super) fn parse_conversation_messages(raw: &str) -> Option<MessagesPage> {
	let data = serde_json::from_str::<ConversationMessagesEnvelope>(raw).ok()?.data;
	let parsed = data
		.messages
		.into_iter()
		.map(|raw_msg| match serde_json::from_str::<ChatMessage>(raw_msg.get()) {
			Ok(mut msg) => {
				if msg.is_call() {
					msg.text = translate_call_text(&msg.text);
				}
				msg
			}
			Err(e) => ChatMessage {
				text: if crate::i18n::is_indonesian() {
					format!("Kesalahan penguraian: {} untuk json: {}", e, raw_msg.get())
				} else {
					format!("Parse error: {} for json: {}", e, raw_msg.get())
				},
				..Default::default()
			},
		})
		.collect();
	Some(MessagesPage {
		jid: data.jid,
		load_more: data.load_more,
		load_newer: data.load_newer,
		is_latest: data.is_latest,
		older: data.older,
		newer: data.newer,
		target_found: data.target_found,
		target_id: data.target_id.filter(|id| !id.is_empty()),
		messages: parsed,
	})
}
