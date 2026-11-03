use whatsnap::ChatMessage;
use wxdragon::prelude::*;

use crate::{AppState, CallTarget};

pub(crate) fn update_loaded_messages(
	s: &mut AppState,
	pick: impl FnMut(&ChatMessage) -> bool,
	change: impl FnMut(&mut ChatMessage),
) -> Vec<usize> {
	whatsnap::update_shared_messages(&mut s.messages, &mut s.display_items, &mut s.selection_rows, pick, change)
}

pub(crate) fn update_loaded_message(
	s: &mut AppState,
	id: &str,
	change: impl FnOnce(&mut ChatMessage),
) -> Option<usize> {
	if id.is_empty() {
		return None;
	}
	let mut change = Some(change);
	let mut found = false;
	let rows = update_loaded_messages(
		s,
		|m| {
			let hit = !found && m.id == id;
			found |= hit;
			hit
		},
		|m| {
			if let Some(change) = change.take() {
				change(m)
			}
		},
	);
	rows.first().copied()
}

pub(crate) fn build_media_messages(messages: &[whatsnap::Msg]) -> Vec<whatsnap::Msg> {
	messages.iter().filter(|m| m.is_photo_or_video()).cloned().collect()
}

pub(crate) fn build_document_messages(messages: &[whatsnap::Msg]) -> Vec<whatsnap::Msg> {
	messages.iter().filter(|m| m.is_document()).cloned().collect()
}

pub(crate) fn build_link_messages(messages: &[whatsnap::Msg]) -> Vec<whatsnap::Msg> {
	messages.iter().filter(|m| m.first_link().is_some()).cloned().collect()
}

pub(crate) struct InfoMediaLists<'a> {
	pub(crate) media: &'a ListBox,
	pub(crate) documents: &'a ListBox,
	pub(crate) links: &'a ListBox,
}

pub(crate) fn fill_info_media(
	s: &AppState,
	lists: InfoMediaLists,
	from: Option<&[whatsnap::Msg]>,
) -> (Vec<whatsnap::Msg>, Vec<whatsnap::Msg>, Vec<whatsnap::Msg>) {
	let whole_chat = s.older_unloaded == 0 && s.newer_unloaded == 0;
	let Some(messages) = from.or(whole_chat.then_some(&s.messages[..])) else {
		if let Some(jid) = &s.selected_chat {
			let payload = serde_json::json!({ "action": "get_chat_media", "data": { "jid": jid } });
			let _ = s.ws_tx.send(payload.to_string());
		}
		for list in [lists.media, lists.documents, lists.links] {
			list.clear();
			list.append("Loading...");
		}
		return Default::default();
	};
	let media = build_media_messages(messages);
	let documents = build_document_messages(messages);
	let links = build_link_messages(messages);
	populate_message_list(lists.media, &media, |m| m.display_filename());
	populate_message_list(lists.documents, &documents, |m| m.display_filename());
	populate_message_list(lists.links, &links, |m| m.first_link().unwrap_or_default());
	(media, documents, links)
}

pub(crate) fn populate_message_list(
	list: &ListBox,
	messages: &[whatsnap::Msg],
	label: impl Fn(&ChatMessage) -> String,
) {
	list.clear();
	for msg in messages {
		list.append(&label(msg));
	}
}

/// Whether a chat belongs in the list shown for `tab`. Conversations (0) hides
/// archived chats, Archived (3) shows only those, and Communities (4) shows
/// the communities and the groups inside them. The Status and Starred tabs
/// build their rows from something other than the chat list and never reach
/// this.
///
/// Communities are kept out of Conversations as well: a community parent
/// carries no messages of its own - everything is posted in its announcement
/// and member sub-groups, which appear as ordinary chats - so a row for it
/// there would be permanently empty.
#[allow(clippy::too_many_arguments)]
pub(crate) fn chat_visible_in_tab(
	tab: usize,
	jid: &str,
	archived: &std::collections::HashSet<String>,
	communities: &std::collections::HashSet<String>,
	parents: &std::collections::HashMap<String, String>,
	channels: &std::collections::HashSet<String>,
	locked: &std::collections::HashSet<String>,
	chats_unlocked: bool,
) -> bool {
	if chats_unlocked {
		return locked.contains(jid);
	}
	if locked.contains(jid) {
		return false;
	}
	match tab {
		0 => !archived.contains(jid) && !communities.contains(jid) && !channels.contains(jid),
		3 => archived.contains(jid),
		4 => communities.contains(jid) || parents.get(jid).is_some_and(|p| communities.contains(p)),
		5 => channels.contains(jid),
		_ => true,
	}
}

pub(crate) fn unread_chats_in_tab(s: &AppState, tab: usize) -> usize {
	whatsnap::unread_chat_count(&s.chats, &s.unread_counts, |jid| {
		chat_visible_in_tab(
			tab,
			jid,
			&s.archived_jids,
			&s.community_jids,
			&s.chat_parents,
			&s.channel_jids,
			&s.locked_jids,
			s.chats_unlocked,
		)
	})
}

pub(crate) fn chats_in_tab_order(s: &AppState) -> Vec<&String> {
	if s.active_tab == 4 {
		whatsnap::community_tab_order(&s.chats, &s.community_jids, &s.chat_parents, &s.announcement_jids)
	} else {
		s.chats.iter().collect()
	}
}

/// The index of the one row that `new` lacks when it is `old` with exactly one
/// row taken out, keeping the order of the rest.
pub(crate) fn dropped_row_offset(old: &[String], new: &[String]) -> Option<usize> {
	if old.len() != new.len() + 1 {
		return None;
	}
	let at = old.iter().zip(new).position(|(a, b)| a != b).unwrap_or(new.len());
	(old[at + 1..] == new[at..]).then_some(at)
}

pub(crate) fn find_single_removal_offset(old: &[String], new: &[String]) -> Option<usize> {
	if new.is_empty() || old.len() != new.len() {
		return None;
	}
	let new_tail = &new[1..];
	let mut oi = 0usize;
	let mut ni = 0usize;
	let mut skipped: Option<usize> = None;
	while ni < new_tail.len() {
		if old[oi] == new_tail[ni] {
			oi += 1;
			ni += 1;
		} else if skipped.is_none() {
			skipped = Some(oi);
			oi += 1;
		} else {
			return None;
		}
	}
	Some(skipped.unwrap_or(oi))
}

pub(crate) const BUILT_IN_CHAT_FILTERS: [&str; 4] = ["All chats", "Unread", "Favorites", "Groups"];

pub(crate) fn chat_passes_filter(s: &AppState, jid: &str) -> bool {
	if s.active_tab != 0 || s.chats_unlocked {
		return true;
	}
	match s.chat_filter {
		0 => true,
		1 => s.unread_counts.get(jid).is_some_and(|&n| n > 0) || s.selected_chat.as_deref() == Some(jid),
		2 => s.favorite_jids.iter().any(|f| f == jid),
		3 => jid.ends_with("@g.us"),
		n => s.chat_lists.get(n - BUILT_IN_CHAT_FILTERS.len()).is_none_or(|l| l.jids.iter().any(|j| j == jid)),
	}
}

pub(crate) fn apply_chat_filter(s: &mut AppState, parts: &crate::ui::window::WindowParts) {
	crate::ui::window::refresh_menu_bar(s, &parts.frame);
	crate::platform::tray::refresh_nav_unread_labels(s, parts);
	let show_locked_btn = s.active_tab == 0 && !s.locked_jids.is_empty();
	let show_filter = s.active_tab == 0 && !s.chats_unlocked;
	let conv = &parts.conversations_parts;
	if conv.locked_chats_btn.is_shown() != show_locked_btn || conv.chat_filter_combo.is_shown() != show_filter {
		conv.locked_chats_btn.show(show_locked_btn);
		conv.chat_filter_label.show(show_filter);
		conv.chat_filter_combo.show(show_filter);
		conv.panel.layout();
	}

	let lower_query = s.search_query.to_lowercase();
	let list_selected_jid = parts
		.conversations_parts
		.conversations_list
		.get_selection()
		.and_then(|i| s.filtered_chats.get(i as usize).cloned());

	let mut filtered = Vec::new();
	let mut labels = Vec::new();
	let mut filtered_starred_ids = Vec::new();
	let mut filtered_status_ids = Vec::new();
	let mut filtered_call_ids: Vec<String> = Vec::new();
	let selected_index = list_selected_index(parts);
	let selected_call_id = selected_index
		.filter(|_| s.active_tab == 6)
		.and_then(|i| s.filtered_call_ids.get(i))
		.filter(|id| !id.is_empty())
		.cloned();

	if s.active_tab == 1 {
		// One row per person, like WA Web, under its Recent / Viewed / Muted
		// headings. A row opens the viewer at that person's first unseen
		// update (its message id is the row's).
		let mut section = None;
		for (sec, group) in crate::ui_commands::status_extras::sorted_groups(&s.status_groups) {
			let Some(start) = crate::ui_commands::status_extras::start_item(group) else { continue };
			let label = status_row_label(group);
			if !lower_query.is_empty() && !label.to_lowercase().contains(&lower_query) {
				continue;
			}
			if section != Some(sec) {
				section = Some(sec);
				if let Some(heading) = sec.heading() {
					labels.push(heading.to_string());
					filtered.push(String::new());
					filtered_status_ids.push(String::new());
				}
			}
			labels.push(label);
			filtered.push(group.poster_jid.clone());
			filtered_status_ids.push(start.id.clone());
		}
	} else if s.active_tab == 2 {
		for entry in &s.starred_entries {
			if lower_query.is_empty() || entry.label.to_lowercase().contains(&lower_query) {
				labels.push(entry.label.clone());
				filtered.push(entry.jid.clone());
				filtered_starred_ids.push(entry.message_id.clone());
			}
		}
	} else if s.active_tab == 6 {
		let matches_target = |t: &CallTarget| {
			lower_query.is_empty()
				|| t.name.to_lowercase().contains(&lower_query)
				|| (!t.phone.is_empty() && t.phone.contains(&lower_query))
		};
		let target_label = |t: &CallTarget| {
			if t.phone.is_empty() || t.name == t.phone || t.name.contains(&t.phone) {
				t.name.clone()
			} else {
				format!("{}, {}", t.name, t.phone)
			}
		};
		let mut push_section = |heading: &str, rows: Vec<(String, String, String)>| {
			if rows.is_empty() {
				return;
			}
			labels.push(heading.to_string());
			filtered.push(String::new());
			filtered_call_ids.push(String::new());
			for (label, jid, call_id) in rows {
				labels.push(label);
				filtered.push(jid);
				filtered_call_ids.push(call_id);
			}
		};

		let ongoing: Vec<(String, String, String)> = s
			.joinable_calls
			.iter()
			.map(|c| (crate::ui_commands::calls::joinable_call_label(s, c), c))
			.filter(|(label, _)| lower_query.is_empty() || label.to_lowercase().contains(&lower_query))
			.map(|(label, c)| {
				(label, format!("{}{}", crate::ui_commands::calls::JOIN_ROW_PREFIX, c.call_id), String::new())
			})
			.collect();
		push_section("Going on now", ongoing);

		let frequent: Vec<(String, String, String)> = s
			.call_frequent
			.iter()
			.filter(|t| matches_target(t))
			.map(|t| (target_label(t), t.jid.clone(), String::new()))
			.collect();
		push_section("Frequent contacts", frequent);

		let mut recent = Vec::new();
		for entry in &s.call_logs {
			let name = if !entry.peer_name.is_empty() {
				&entry.peer_name
			} else if let Some(n) = s.contacts.get(&entry.jid) {
				n
			} else {
				&entry.jid
			};

			let mut label = format_call_log_label(name, &entry.text, entry.from_me, entry.timestamp);
			let mut row_jid = entry.jid.clone();
			if s.joinable_calls.iter().any(|c| c.call_id == entry.id) {
				label.push_str(crate::ui_commands::calls::ONGOING_CALL_SUFFIX);
				row_jid = format!("{}{}", crate::ui_commands::calls::JOIN_ROW_PREFIX, entry.id);
			}

			if lower_query.is_empty() || label.to_lowercase().contains(&lower_query) || entry.jid.contains(&lower_query)
			{
				recent.push((label, row_jid, entry.id.clone()));
			}
		}
		push_section("Recent calls", recent);

		let all: Vec<(String, String, String)> = s
			.call_contacts
			.iter()
			.filter(|t| matches_target(t))
			.map(|t| (target_label(t), t.jid.clone(), String::new()))
			.collect();
		push_section("All contacts", all);
	} else {
		for jid in chats_in_tab_order(s) {
			if !chat_visible_in_tab(
				s.active_tab,
				jid,
				&s.archived_jids,
				&s.community_jids,
				&s.chat_parents,
				&s.channel_jids,
				&s.locked_jids,
				s.chats_unlocked,
			) || !chat_passes_filter(s, jid)
			{
				continue;
			}

			let name = s.contacts.get(jid).unwrap_or(jid);
			if lower_query.is_empty() || name.to_lowercase().contains(&lower_query) {
				labels.push(s.display_names.get(jid).unwrap_or(name).clone());
				filtered.push(jid.clone());
			}
		}
	}

	s.filtered_chats = filtered;
	s.filtered_starred_message_ids = filtered_starred_ids;
	s.filtered_status_message_ids = filtered_status_ids;
	s.filtered_call_ids = filtered_call_ids;

	// Clearing and refilling the list resets the focused item and makes the
	// screen reader re-announce, so leave the control untouched when the
	// backend re-sends chats that produce identical contents.
	if labels == s.displayed_chat_labels {
		return;
	}

	let list = &parts.conversations_parts.conversations_list;
	let had_focus = list.has_focus();

	let moved_from = find_single_removal_offset(&s.displayed_chat_labels, &labels);

	let removed_at = dropped_row_offset(&s.displayed_chat_labels, &labels);

	list.freeze();
	if let Some(idx) = moved_from {
		list.delete(idx as u32);
		list.insert(&labels[0], 0);
	} else if let Some(idx) = removed_at {
		// One row gone, like a deleted call: take just that row out, so the
		// list isn't refilled and read out again from the top.
		list.delete(idx as u32);
	} else {
		list.set_items(&labels.iter().map(String::as_str).collect::<Vec<_>>());
	}
	list.thaw();

	// A deleted call leaves the focus where it was, on the next call, instead
	// of jumping to the first row for the same person.
	let call_target = selected_call_id.as_ref().map(|id| {
		s.filtered_call_ids.iter().position(|c| c == id).or_else(|| {
			let len = s.filtered_chats.len();
			let from = selected_index.unwrap_or(0).min(len.saturating_sub(1));
			(from..len).chain((0..from).rev()).find(|&i| !s.filtered_call_ids.get(i).is_none_or(|c| c.is_empty()))
		})
	});
	let target = call_target
		.flatten()
		.or_else(|| list_selected_jid.as_ref().and_then(|jid| s.filtered_chats.iter().position(|j| j == jid)))
		.or_else(|| s.selected_chat.as_ref().and_then(|sel| s.filtered_chats.iter().position(|j| j == sel)));
	if let Some(idx) = target {
		let count = list.get_count();
		if count > 0 && (idx as u32) < count && list.get_selection() != Some(idx as u32) {
			list.set_selection(idx as u32, true);
		}
	}

	if (had_focus || list.has_focus()) && list.get_selection().is_none() && list.get_count() > 0 {
		list.set_selection(0_u32, true);
	}

	if had_focus {
		list.set_focus();
	}

	s.displayed_chat_labels = labels;
}

fn list_selected_index(parts: &crate::ui::window::WindowParts) -> Option<usize> {
	parts.conversations_parts.conversations_list.get_selection().map(|i| i as usize)
}

pub(crate) fn update_action_buttons(s: &AppState, parts: &crate::ui::conversations::ConversationsPanelParts) -> bool {
	parts.action_panel.freeze();
	let result = update_action_buttons_inner(s, parts);
	parts.action_panel.thaw();
	result
}

fn update_action_buttons_inner(s: &AppState, parts: &crate::ui::conversations::ConversationsPanelParts) -> bool {
	let sel = parts.messages_list.get_selection();
	let mut need_layout = false;

	if let Some(idx) = sel
		&& let Some(item) = s.display_items.get(idx as usize)
	{
		match item {
			whatsnap::DisplayItem::Single(msg) => {
				let is_deleted = msg.is_deleted;
				let is_voice =
					msg.is_audio || msg.text.starts_with("[Voice Message]") || msg.text.starts_with("[Audio]");
				let is_media = !is_deleted
					&& !is_voice
					&& !msg.is_system()
					&& !msg.is_call()
					&& (!msg.media_type.is_empty()
						|| msg.is_document()
						|| msg.text.starts_with("[Photo]")
						|| msg.text.starts_with("[Image]")
						|| msg.text.starts_with("[Video]")
						|| msg.text.starts_with("[Document]")
						|| msg.text.starts_with("[Media]"));

				let is_failed = msg.from_me && !msg.is_date_divider && msg.status == "failed";
				let is_uploading = msg.from_me && msg.status == "uploading";
				if is_failed || is_uploading {
					let cancel_label = (is_uploading && !is_voice).then(|| upload_cancel_label(s, &msg.id));
					return need_layout | show_send_state_buttons(parts, is_failed, cancel_label);
				}
				need_layout |= set_shown(&parts.action_retry_btn, false);
				need_layout |= set_shown(&parts.action_cancel_upload_btn, false);

				if is_media {
					if msg.is_media_downloaded() {
						if parts.action_download_btn.is_shown() {
							parts.action_download_btn.show(false);
							need_layout = true;
						}
						if !parts.action_open_btn.is_shown() {
							parts.action_open_btn.show(true);
							need_layout = true;
						}
						parts.action_open_btn.set_label("Open");
						if !parts.action_save_as_btn.is_shown() {
							parts.action_save_as_btn.show(true);
							need_layout = true;
						}
						parts.action_save_as_btn.set_label("Save as");
					} else {
						if parts.action_open_btn.is_shown() {
							parts.action_open_btn.show(false);
							need_layout = true;
						}
						if parts.action_save_as_btn.is_shown() {
							parts.action_save_as_btn.show(false);
							need_layout = true;
						}
						if !parts.action_download_btn.is_shown() {
							parts.action_download_btn.show(true);
							need_layout = true;
						}
						let label = if let Some(pct) = s.downloads_in_progress.get(&msg.id) {
							format!("Cancel download ({}%)", pct)
						} else if let Some(size) = whatsnap::format_file_size(msg.media_size) {
							format!("Download ({})", size)
						} else {
							"Download".to_string()
						};
						parts.action_download_btn.set_label(&label);
					}
					return need_layout;
				}
			}
			whatsnap::DisplayItem::Album { messages, .. } => {
				let valid_messages: Vec<_> = messages.iter().filter(|m| !m.is_deleted).collect();
				if !valid_messages.is_empty() {
					let is_failed = valid_messages.iter().any(|m| m.from_me && m.status == "failed");
					let is_uploading = valid_messages.iter().any(|m| m.from_me && m.status == "uploading");
					if is_failed || is_uploading {
						let cancel_label = valid_messages
							.iter()
							.find(|m| m.from_me && m.status == "uploading")
							.map(|m| upload_cancel_label(s, &m.id));
						return need_layout | show_send_state_buttons(parts, is_failed, cancel_label);
					}
					need_layout |= set_shown(&parts.action_retry_btn, false);
					need_layout |= set_shown(&parts.action_cancel_upload_btn, false);

					let all_dl = valid_messages.iter().all(|m| m.is_media_downloaded());
					if all_dl {
						if parts.action_download_btn.is_shown() {
							parts.action_download_btn.show(false);
							need_layout = true;
						}
						if !parts.action_open_btn.is_shown() {
							parts.action_open_btn.show(true);
							need_layout = true;
						}
						parts.action_open_btn.set_label("Open album");

						let photo_count = valid_messages
							.iter()
							.filter(|m| {
								m.media_type == "image"
									|| m.text.starts_with("[Photo]")
									|| m.text.starts_with("[Image]")
							})
							.count();
						let video_count = valid_messages
							.iter()
							.filter(|m| m.media_type == "video" || m.text.starts_with("[Video]"))
							.count();
						let save_label = if photo_count > 0 && video_count == 0 {
							"Save all photos"
						} else if video_count > 0 && photo_count == 0 {
							"Save all videos"
						} else {
							"Save all media"
						};
						if !parts.action_save_as_btn.is_shown() {
							parts.action_save_as_btn.show(true);
							need_layout = true;
						}
						parts.action_save_as_btn.set_label(save_label);
					} else {
						if parts.action_open_btn.is_shown() {
							parts.action_open_btn.show(false);
							need_layout = true;
						}
						if parts.action_save_as_btn.is_shown() {
							parts.action_save_as_btn.show(false);
							need_layout = true;
						}
						if !parts.action_download_btn.is_shown() {
							parts.action_download_btn.show(true);
							need_layout = true;
						}

						let uncollected: Vec<_> = valid_messages.iter().filter(|m| !m.is_media_downloaded()).collect();
						let is_dl_in_progress =
							valid_messages.iter().any(|m| s.downloads_in_progress.contains_key(&m.id));
						let dl_label = if is_dl_in_progress {
							format!("Downloading album ({} items remaining)", uncollected.len())
						} else {
							let total_size: i64 = uncollected.iter().map(|m| m.media_size).sum();
							match whatsnap::format_file_size(total_size) {
								Some(size) => {
									format!("Download album ({} items remaining, {})", uncollected.len(), size)
								}
								None => format!("Download album ({} items remaining)", uncollected.len()),
							}
						};
						parts.action_download_btn.set_label(&dl_label);
					}
				}
				return need_layout;
			}
		}
	}

	need_layout |= set_shown(&parts.action_retry_btn, false);
	need_layout |= set_shown(&parts.action_cancel_upload_btn, false);
	if parts.action_download_btn.is_shown() {
		parts.action_download_btn.show(false);
		need_layout = true;
	}
	if parts.action_open_btn.is_shown() {
		parts.action_open_btn.show(false);
		need_layout = true;
	}
	if parts.action_save_as_btn.is_shown() {
		parts.action_save_as_btn.show(false);
		need_layout = true;
	}
	need_layout
}

fn set_shown(btn: &Button, show: bool) -> bool {
	if btn.is_shown() == show {
		return false;
	}
	btn.show(show);
	true
}

fn upload_cancel_label(s: &AppState, msg_id: &str) -> String {
	format!("Cancel upload, {}% complete", s.uploads_in_progress.get(msg_id).copied().unwrap_or(0))
}

fn show_send_state_buttons(
	parts: &crate::ui::conversations::ConversationsPanelParts,
	failed: bool,
	cancel_label: Option<String>,
) -> bool {
	let mut need_layout = set_shown(&parts.action_retry_btn, failed);
	if let Some(label) = &cancel_label
		&& parts.action_cancel_upload_btn.get_label() != *label
	{
		parts.action_cancel_upload_btn.set_label(label);
	}
	need_layout |= set_shown(&parts.action_cancel_upload_btn, cancel_label.is_some());
	need_layout |= set_shown(&parts.action_open_btn, false);
	need_layout |= set_shown(&parts.action_save_as_btn, false);
	need_layout |= set_shown(&parts.action_download_btn, false);
	need_layout
}

pub(crate) fn refresh_voice_action_button(s: &AppState, parts: &crate::ui::conversations::ConversationsPanelParts) {
	parts.action_panel.freeze();
	let mut need_layout = update_action_buttons_inner(s, parts);

	let msg_opt = parts.messages_list.get_selection().and_then(|idx| s.display_items.get(idx as usize)).and_then(
		|item| match item {
			whatsnap::DisplayItem::Single(m) => Some(m.clone()),
			whatsnap::DisplayItem::Album { messages, .. } => messages.first().cloned(),
		},
	);

	let is_voice = msg_opt.as_ref().is_some_and(|msg| {
		let is_deleted =
			msg.is_deleted || msg.text == "[This message was deleted]" || msg.text == "[You deleted this message]";
		!is_deleted && (msg.is_audio || msg.text.starts_with("[Voice Message]") || msg.text.starts_with("[Audio]"))
	});

	if parts.play_voice_btn.is_shown() != is_voice {
		parts.play_voice_btn.show(is_voice);
		need_layout = true;
	}

	if is_voice {
		let msg = msg_opt.expect("is_voice implies msg_opt is Some");
		let is_playing = s.playing_msg_id.as_deref() == Some(msg.id.as_str());
		if is_playing {
			// No sink while a message is marked playing means playback never
			// actually started, so present it as paused rather than "Pause".
			let is_paused = s.audio_sink.as_ref().is_none_or(|sink| sink.is_paused() || sink.empty());
			parts.play_voice_btn.set_label(if is_paused { "Play voice message" } else { "Pause voice message" });
			if !parts.audio_slider.is_shown() {
				parts.audio_slider.show(true);
				parts.audio_speed_btn.show(true);
				parts.audio_speed_btn.set_label(&format!("{:.1}x", s.audio_speed));
				parts.audio_progress_label.show(true);
				need_layout = true;
			}
		} else {
			parts.play_voice_btn.set_label("Play voice message");
			if parts.audio_slider.is_shown() {
				parts.audio_slider.show(false);
				parts.audio_speed_btn.show(false);
				parts.audio_progress_label.show(false);
				need_layout = true;
			}
		}
	} else if parts.audio_slider.is_shown() {
		parts.audio_slider.show(false);
		parts.audio_speed_btn.show(false);
		parts.audio_progress_label.show(false);
		need_layout = true;
	}

	let any_action_visible = parts.play_voice_btn.is_shown()
		|| parts.action_retry_btn.is_shown()
		|| parts.action_cancel_upload_btn.is_shown()
		|| parts.action_open_btn.is_shown()
		|| parts.action_save_as_btn.is_shown()
		|| parts.action_download_btn.is_shown()
		|| parts.audio_slider.is_shown()
		|| parts.audio_speed_btn.is_shown();
	if parts.action_panel.is_shown() != any_action_visible {
		parts.action_panel.show(any_action_visible);
		need_layout = true;
	}

	parts.action_panel.thaw();
	if need_layout {
		parts.action_panel.layout();
	}
}

pub(crate) fn format_call_log_label(name: &str, text: &str, from_me: bool, timestamp: i64) -> String {
	let call_desc = if text.starts_with("Incoming ") {
		let rest = &text["Incoming ".len()..];
		if let Some((kind, dur)) = rest.split_once(", ") {
			let kind_cap = if let Some(first) = kind.chars().next() {
				format!("{}{}", first.to_uppercase(), &kind[first.len_utf8()..])
			} else {
				kind.to_string()
			};
			format!("{} from {}, duration {}", kind_cap, name, dur)
		} else {
			format!("Incoming {} from {}", rest, name)
		}
	} else if text.starts_with("Outgoing ") {
		let rest = &text["Outgoing ".len()..];
		if let Some((kind, dur)) = rest.split_once(", ") {
			format!("Outgoing {} to {}, duration {}", kind, name, dur)
		} else {
			format!("Outgoing {} to {}", rest, name)
		}
	} else if text.starts_with("Missed ") {
		format!("Missed {} from {}", &text["Missed ".len()..], name)
	} else {
		let prep = if from_me { "to" } else { "from" };
		format!("{} {} {}", text, prep, name)
	};

	let time_str = if timestamp > 0 {
		if let Some(local_dt) = whatsnap::LocalTime::from_unix(timestamp) {
			let now_dt = whatsnap::LocalTime::now();
			let msg_date = local_dt.date();
			let today = now_dt.date();
			let yesterday = today.add_days(-1);
			if msg_date == today {
				format!("Today {}", local_dt.format("%I:%M %p"))
			} else if msg_date == yesterday {
				format!("Yesterday {}", local_dt.format("%I:%M %p"))
			} else {
				local_dt.format("%d/%m/%Y %I:%M %p")
			}
		} else {
			String::new()
		}
	} else {
		String::new()
	};

	if time_str.is_empty() { call_desc } else { format!("{}, {}", call_desc, time_str) }
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_dropped_row_offset() {
		let v = |x: &[&str]| x.iter().map(|s| s.to_string()).collect::<Vec<_>>();
		assert_eq!(dropped_row_offset(&v(&["a", "b", "c"]), &v(&["a", "c"])), Some(1));
		assert_eq!(dropped_row_offset(&v(&["a", "b", "c"]), &v(&["a", "b"])), Some(2));
		assert_eq!(dropped_row_offset(&v(&["a", "b", "c"]), &v(&["b", "c"])), Some(0));
		assert_eq!(dropped_row_offset(&v(&["a", "b", "c"]), &v(&["a", "x"])), None);
		assert_eq!(dropped_row_offset(&v(&["a", "b"]), &v(&["a", "b"])), None);
	}

	#[test]
	fn test_format_call_log_label() {
		assert_eq!(
			format_call_log_label("Aryan", "Incoming voice call, 44 minutes 32 seconds", false, 0),
			"Voice call from Aryan, duration 44 minutes 32 seconds"
		);
		assert_eq!(
			format_call_log_label("Aryan", "Outgoing voice call, 5 minutes", true, 0),
			"Outgoing voice call to Aryan, duration 5 minutes"
		);
		assert_eq!(format_call_log_label("Aryan", "Missed voice call", false, 0), "Missed voice call from Aryan");
		assert_eq!(format_call_log_label("Aryan", "Voice call, declined", true, 0), "Voice call, declined to Aryan");
	}
}

fn status_time(item: &crate::StatusItem) -> String {
	if item.timestamp > 0 {
		whatsnap::LocalTime::from_unix(item.timestamp).map(whatsnap::time_or_day).unwrap_or_default()
	} else {
		String::new()
	}
}

/// A person's row in the Status tab. With one update it reads as before
/// (name, new or viewed, the update, its time); with several it says how
/// many and how many are new, and the newest one's time.
fn status_row_label(group: &crate::StatusGroup) -> String {
	let Some(newest) = group.items.iter().max_by_key(|it| it.timestamp) else { return String::new() };
	let time = status_time(newest);
	let count = group.items.len();
	let mut parts: Vec<String> = Vec::new();
	if count == 1 {
		let preview = if !newest.text.is_empty() {
			newest.text.clone()
		} else if !newest.media_type.is_empty() {
			"Media".to_string()
		} else {
			String::new()
		};
		parts.push(preview);
	} else {
		parts.push(format!("{count} updates"));
		let unseen = group.items.iter().filter(|it| !it.viewed).count();
		if unseen > 0 && !group.from_me {
			parts.push(format!("{unseen} new"));
		}
	}
	if !time.is_empty() {
		parts.push(time);
	}
	if group.from_me {
		let views: i64 = if count == 1 { newest.viewers } else { 0 };
		if views > 0 {
			parts.push(format!("{} view{}", views, if views == 1 { "" } else { "s" }));
		}
		return format!("My status: {}", parts.join(", "));
	}
	if count == 1 && !newest.my_reaction.is_empty() {
		parts.push(format!("you reacted {}", newest.my_reaction));
	}
	let viewed_tag = match (group.items.iter().all(|it| it.viewed), group.muted) {
		(true, false) => "Viewed",
		(false, false) => "New",
		(true, true) => "Viewed, muted",
		(false, true) => "New, muted",
	};
	format!("{} [{}]: {}", group.poster_name, viewed_tag, parts.join(", "))
}
