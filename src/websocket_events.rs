//! Handles every event pushed from the Go backend over the WebSocket-like
//! local pipe (`UiCommand::ProcessWebSocketMessage`), dispatching on the
//! JSON payload's `type` field: message,call,presence,contacts,chats sync,
//! privacy settings, status updates, and friends.

use std::{borrow::Cow, cell::RefCell, fs::File, io::BufReader, rc::Rc};

use rodio::{Decoder, Player};
use rodio_wsola::WsolaSourceExt;
use serde::Deserialize;
use whantv::ChatMessage;
use wxdragon::prelude::*;

use crate::{
	AppState, CallVideo, Contact, NotificationSound, SENT_SOUND_WINDOW_SECS, StarredEntry, StatusGroup, StatusItem,
	UiCommand,
	audio::{
		decode_opus_file, ensure_audio_stream, play_call_fail_sound, play_notification_sound, start_call_ringback,
		start_call_ringing_sound, stop_call_ringing_sound,
	},
	call_audio,
	call_helpers::{
		announce_call, call_participant_changes, call_window_title, get_status_rank, group_call_status_text,
		incoming_call_description, parse_call_participants, peer_video_announcement, show_call_device_combos,
	},
	chat_ui::show_toast_notification,
	contact_picker::{
		apply_contact_picker_filter, display_name_for_jid, refresh_selection_list, sync_add_members_contact_labels,
		sync_attach_contact_labels, sync_new_group_contact_labels,
	},
	message_display::{
		apply_chat_filter, build_document_messages, build_link_messages, build_media_messages, chat_visible_in_tab,
		chats_in_tab_order, populate_message_list, refresh_voice_action_button, update_action_buttons,
		update_loaded_message, update_loaded_messages,
	},
	tray::{install_tray_icon, refresh_tray_tooltip},
	ui::window::WindowParts,
};

/// Formats the conversation data button label for a chat.
///
/// In 1-on-1 chats:
/// - When typing: `<Chat Name> (typing...)`
/// - When recording: `<Chat Name> (recording audio...)`
/// - When idle: Presence label (e.g. `<Name> (online)` or `<Name> (last seen ...)`) or fallback `<Chat Name>`.
///
/// In group chats:
/// - When typing: `<Group Name>, <Sender Name> is typing...`
/// - When recording: `<Group Name>, <Sender Name> is recording audio...`
/// - When idle: Presence label or fallback `<Group Name>`.
pub(crate) fn format_conv_data_label(
	chat_jid: &str,
	chat_name: &str,
	sender_name: &str,
	is_typing: bool,
	is_recording: bool,
	presence_label: Option<&str>,
) -> String {
	if is_recording || is_typing {
		let is_group = chat_jid.ends_with("@g.us");
		if is_group {
			let status = if is_recording {
				format!("{} is recording audio...", sender_name)
			} else {
				format!("{} is typing...", sender_name)
			};
			format!("{}, {}", chat_name, status)
		} else {
			let status = if is_recording { "recording audio..." } else { "typing..." };
			format!("{} ({})", chat_name, status)
		}
	} else {
		presence_label.filter(|l| !l.is_empty()).map(|l| l.to_string()).unwrap_or_else(|| chat_name.to_string())
	}
}

/// Notes a call event in `logs/ui.log`, with whether the call window was up
/// and what the UI thought the active call was. The backend logs its side, so
/// the two together show whether an incoming call reached the window.
pub(crate) fn call_log(parts: &WindowParts, state: &AppState, event: &str, call_id: &str) {
	ui_timing_log(&format!(
		"Call: {} {} (active call {:?}, placing {}, window shown {})",
		event,
		call_id,
		state.active_call_id,
		state.call_placing,
		parts.call_parts.frame.is_shown()
	));
}

/// Appends one line to `logs/ui.log`: how long opening a chat's messages took,
/// split by stage, so a slow load can be pinned down from a user's logs.
fn ui_timing_log(msg: &str) {
	use std::io::Write;
	let now = chrono::Local::now().format("%Y/%m/%d %H:%M:%S");
	if let Ok(mut f) =
		std::fs::OpenOptions::new().create(true).append(true).open(crate::paths::log_dir().join("ui.log"))
	{
		let _ = writeln!(f, "{now} {msg}");
	}
}

/// Drops the messages in `msgs` whose id is already in `seen` or earlier in
/// `msgs` (messages without an id always stay). The ids are borrowed, not
/// copied: a whole chat is tens of thousands of them.
fn drop_seen_ids<'a>(msgs: Vec<ChatMessage>, seen: impl Iterator<Item = &'a str>) -> Vec<ChatMessage> {
	let keep: Vec<bool> = {
		let mut seen: std::collections::HashSet<&str> = seen.collect();
		msgs.iter().map(|m| m.id.is_empty() || seen.insert(m.id.as_str())).collect()
	};
	msgs.into_iter().zip(keep).filter_map(|(m, keep)| keep.then_some(m)).collect()
}

/// Applies a newer page (see `ui_commands::chat_list::load_newer_messages`)
/// below what's loaded: appends only the new rows, leaving the selection where
/// it is so the next Down arrow continues into them.
fn apply_newer_messages(
	parsed_msgs: Vec<ChatMessage>,
	is_latest: bool,
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
) {
	let mut s = state.borrow_mut();
	s.newer_loading = false;
	if is_latest {
		s.messages_at_latest = true;
	}
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
		if let Some(st) = s.pending_receipts.remove(&msg.id) {
			if get_status_rank(&st) >= get_status_rank(&msg.status) {
				msg.status = st;
			}
		}
	}
	let mut merged = std::mem::take(&mut s.messages);
	merged.extend(added.into_iter().map(whantv::Msg::new));
	s.messages = whantv::rebuild_date_dividers(merged);

	let old_display_items = std::mem::take(&mut s.display_items);
	let new_display_items = whantv::group_messages_into_display_items(&s.messages);
	let old_len = old_display_items.len();
	// Appending leaves every row already shown untouched unless an album
	// regrouped across the seam; rebuild (keeping the selected row) if so.
	let append_only = new_display_items.len() > old_len
		&& new_display_items[..old_len].iter().zip(&old_display_items).all(|(a, b)| a.id() == b.id());
	let list = &parts.conversations_parts.messages_list;
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
}

/// Applies a `conversation_messages` payload to the open chat: dedupes, adds
/// date and unread dividers, replaces (or, for a `load_more` page, prepends
/// onto) `messages`/`display_items`, repaints the message list and restores
/// the right selection (a search jump target, the unread divider, or the
/// bottom).
///
/// Takes already-deserialized messages so the caller can parse them straight
/// off the wire - see `process_websocket_message`'s fast path for why this one
/// payload bypasses `serde_json::Value`.
fn apply_conversation_messages(
	parsed_msgs: Vec<ChatMessage>,
	load_more: bool,
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
) {
	let started = std::time::Instant::now();
	let mut deduplicated = drop_seen_ids(parsed_msgs, std::iter::empty());

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
		// Nothing new came back. Two possibilities: this was a plain local
		// page and the database has nothing older left for this chat (first
		// time seeing that for this jid - fall back to asking the phone), or
		// it was the local re-query that follows a phone response, and the
		// phone had nothing new either (give up on this chat for the
		// session - see `phone_history_pending_jid`'s doc comment on why
		// re-asking isn't safe).
		if s.phone_history_pending_jid.as_deref() == Some(chat_jid.as_str()) {
			eprintln!("[LOG UI] Phone had no older history for {}. Marking history complete.", chat_jid);
			s.phone_history_pending_jid = None;
			s.phone_history_deadline = None;
			s.completed_history_chats.insert(chat_jid);
			s.history_loading = false;
		} else {
			eprintln!("[LOG UI] Local database has no older history for {} yet; asking the phone.", chat_jid);
			s.no_more_messages = true;
			crate::ui_commands::chat_list::request_phone_history_sync(&mut s, &chat_jid);
		}
		return;
	}

	let mut with_dates = Vec::new();
	let mut last_date: Option<chrono::NaiveDate> = None;
	for mut msg in deduplicated {
		if !is_group_chat && !msg.from_me && msg.sender_name.is_empty() && msg.sender_jid.is_empty() {
			msg.sender_name = chat_name.clone();
		}

		if let Some(datetime) = chrono::DateTime::from_timestamp(msg.timestamp, 0) {
			let local_datetime: chrono::DateTime<chrono::Local> = datetime.into();
			let day = whantv::date_key(local_datetime);
			if last_date != Some(day) {
				last_date = Some(day);
				with_dates.push(whantv::Msg::new(ChatMessage {
					text: whantv::format_date_divider_label(local_datetime),
					timestamp: msg.timestamp,
					is_date_divider: true,
					..Default::default()
				}));
			}
		}
		with_dates.push(whantv::Msg::new(msg));
	}

	if !load_more && s.unread_divider_jid.as_deref() == Some(chat_jid.as_str()) {
		whantv::insert_unread_divider(&mut with_dates, s.unread_divider_count);
	}

	s.loading_messages = false;
	// Every request loads everything left in that direction (see
	// `MESSAGE_PAGE_SIZE`), so after any page the local database has nothing
	// older: Home at the top goes straight to the phone. A later history-sync
	// chunk for this chat clears it again (`history_chats_synced`).
	s.no_more_messages = true;
	let quiet = !load_more && s.quiet_reload_jid.take().as_deref() == Some(chat_jid.as_str());
	let was_history_loading = s.history_loading;
	s.history_loading = false;
	let old_count = s.prev_msg_count;
	let is_group = s.selected_chat.as_ref().map(|j| j.ends_with("@g.us")).unwrap_or(false);
	if load_more {
		s.phone_history_pending_jid = None;
		s.phone_history_deadline = None;
		with_dates.append(&mut s.messages);
		// The older page and the already-loaded batch each got their date
		// dividers built independently (see `rebuild_date_dividers`'s doc
		// comment) - rebuild once across the merged list instead of ending
		// up with two adjacent dividers for the same day.
		with_dates = whantv::rebuild_date_dividers(with_dates);
	}
	s.messages = with_dates;
	// Apply any receipts that arrived before messages finished loading.
	// Collect first to avoid simultaneous mutable borrows of s.
	let pending_ids: Vec<(String, String)> = s
		.messages
		.iter()
		.filter_map(|msg| s.pending_receipts.get(&msg.id).map(|st| (msg.id.clone(), st.clone())))
		.collect();
	for (id, st) in pending_ids {
		s.pending_receipts.remove(&id);
		if let Some(msg) = s.messages.iter_mut().find(|m| m.id == id) {
			if get_status_rank(&st) >= get_status_rank(&msg.status) {
				// Not shared yet: the display items are built below.
				std::rc::Rc::make_mut(msg).status = st;
			}
		}
	}
	let old_display_items = std::mem::take(&mut s.display_items);
	let new_display_items = whantv::group_messages_into_display_items(&s.messages);
	let new_len = new_display_items.len();
	s.prev_msg_count = new_len;

	// A load_more page only ever prepends older messages, so in the common
	// case the tail of the rebuilt display list is exactly what was already
	// on screen - rebuilding the whole wxListBox (clear + re-append
	// everything loaded so far, which only gets more expensive the more
	// pages have piled up) to add a handful of rows at the top is what made
	// scrolling to the top of an already-paged-through chat feel freezy.
	// Insert just the new leading rows instead (see `prepend_plan`, which
	// also handles the date divider moving up across the seam); an album
	// spanning the exact seam can change the grouping there (rare), so this
	// still falls back to a full rebuild whenever the rest doesn't line up.
	let prepend = if load_more { whantv::prepend_plan(&old_display_items, &new_display_items) } else { None };

	// Rows go into the list in one call (`set_items`/`insert_items`), not one
	// append per row: a whole chat is tens of thousands of rows, and adding
	// them one at a time took over 2 seconds on a 27k-message chat.
	let rows: Vec<String> = match prepend {
		Some(plan) => &new_display_items[..plan.insert],
		None => &new_display_items[..],
	}
	.iter()
	.map(|item| item.format_display(is_group))
	.collect();
	let row_refs: Vec<&str> = rows.iter().map(String::as_str).collect();
	let prepared = started.elapsed();
	parts.conversations_parts.messages_list.freeze();
	if let Some(plan) = prepend {
		for _ in 0..plan.remove_leading {
			parts.conversations_parts.messages_list.delete(0);
		}
		parts.conversations_parts.messages_list.insert_items(&row_refs, 0);
	} else {
		parts.conversations_parts.messages_list.set_items(&row_refs);
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

	let count = parts.conversations_parts.messages_list.get_count();
	let target_pos = if let Some(target_id) = s.search_target_id.take() {
		s.display_items.iter().position(|item| match item {
			whantv::DisplayItem::Single(m) => m.id == target_id,
			whantv::DisplayItem::Album { messages, .. } => messages.iter().any(|m| m.id == target_id),
		})
	} else if let Some(target_text) = s.search_target_text.take() {
		let is_generic = target_text == "[Voice Message]"
			|| target_text == "[Audio]"
			|| target_text == "Voice message"
			|| target_text == "Audio"
			|| target_text == "[Media]";
		if is_generic {
			None
		} else {
			// The newest match, as the old around-the-hit lookup picked.
			s.display_items.iter().rposition(|item| match item {
				whantv::DisplayItem::Single(m) => {
					m.text == target_text || (!m.text.is_empty() && m.text.contains(&target_text))
				}
				whantv::DisplayItem::Album { messages, .. } => messages
					.iter()
					.any(|m| m.text == target_text || (!m.text.is_empty() && m.text.contains(&target_text))),
			})
		}
	} else if old_count == 0 && s.unread_divider_jid.as_deref() == Some(chat_jid.as_str()) {
		// Opening a chat with unread messages: land on the
		// "N unread messages" divider, same as pressing Enter
		// on it from the chat list, instead of the very bottom.
		s.display_items.iter().position(|item| matches!(item, whantv::DisplayItem::Single(m) if m.is_unread_divider))
	} else {
		None
	};
	if let Some(pos) = target_pos {
		parts.conversations_parts.messages_list.set_selection(pos as u32, true);
		if !quiet {
			parts.conversations_parts.messages_list.set_focus();
		}
	} else if count > 0 {
		if was_history_loading && new_len > old_count && old_count > 0 {
			// Where the row that was at the top before this page now sits.
			let keep_idx = prepend.map_or(new_len - old_count, |p| p.insert) as u32;
			if keep_idx < count {
				parts.conversations_parts.messages_list.set_selection(keep_idx, true);
			} else {
				parts.conversations_parts.messages_list.set_selection(0, true);
			}
		} else if old_count == 0 {
			parts.conversations_parts.messages_list.set_selection((count - 1) as u32, true);
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

/// One chat of a `chats` payload (the backend's `buildChatsList`). The chat
/// list arrives whole, every ~200ms during a sync, so it's read straight off
/// the wire with its text borrowed, rather than built into a `Value` tree
/// that the handler then copied again before reading.
#[derive(Deserialize)]
struct ChatRow<'a> {
	#[serde(rename = "JID", borrow, default)]
	jid: Cow<'a, str>,
	#[serde(rename = "FullName", borrow, default)]
	full_name: Option<Cow<'a, str>>,
	#[serde(rename = "UnreadCount", default)]
	unread_count: u64,
	#[serde(rename = "Pinned", default)]
	pinned: bool,
	#[serde(rename = "Muted", default)]
	muted: bool,
	#[serde(rename = "Archived", default)]
	archived: bool,
	#[serde(rename = "IsLocked", default)]
	is_locked: bool,
	#[serde(rename = "IsCommunity", default)]
	is_community: bool,
	#[serde(rename = "PreviewSnippet", borrow, default)]
	preview_snippet: Cow<'a, str>,
	#[serde(rename = "Timestamp", default)]
	timestamp: i64,
	#[serde(rename = "PreviewSender", borrow, default)]
	preview_sender: Cow<'a, str>,
	#[serde(rename = "PreviewFromMe", default)]
	preview_from_me: bool,
	#[serde(rename = "PreviewIsReaction", default)]
	preview_is_reaction: bool,
	#[serde(rename = "PreviewReactionEmoji", borrow, default)]
	preview_reaction_emoji: Cow<'a, str>,
	#[serde(rename = "IsChannel", default)]
	is_channel: bool,
	#[serde(rename = "ChannelRole", borrow, default)]
	channel_role: Cow<'a, str>,
	#[serde(rename = "ParentJID", borrow, default)]
	parent_jid: Cow<'a, str>,
	#[serde(rename = "CommunityName", borrow, default)]
	community_name: Cow<'a, str>,
	#[serde(rename = "IsAnnouncements", default)]
	is_announcements: bool,
	#[serde(rename = "IsAdmin", default)]
	is_admin: bool,
}

#[derive(Deserialize)]
struct ChatsEnvelope<'a> {
	#[serde(borrow, default)]
	data: Option<Vec<ChatRow<'a>>>,
}

/// Applies a `chats` payload: rebuilds the per-chat lookups and updates the
/// chat list, touching only the rows that changed.
fn apply_chats(chats: Vec<ChatRow>, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let mut chat_list = Vec::new();
	let mut archived_jids = std::collections::HashSet::new();
	let mut community_jids = std::collections::HashSet::new();
	let mut chat_parents = std::collections::HashMap::new();
	let mut announcement_jids = std::collections::HashSet::new();
	let mut channel_jids = std::collections::HashSet::new();
	let mut channel_roles = std::collections::HashMap::new();
	let mut pinned_jids = std::collections::HashSet::new();
	let mut muted_jids = std::collections::HashSet::new();
	let mut admin_group_jids = std::collections::HashSet::new();
	let mut locked_jids = std::collections::HashSet::new();
	let mut unread_counts = std::collections::HashMap::new();
	let mut display_names = std::collections::HashMap::new();
	let mut contacts_map = std::collections::HashMap::new();
	for chat in &chats {
		let jid: &str = &chat.jid;
		if !jid.is_empty() {
			let name = chat.full_name.as_deref().unwrap_or(jid).to_string();
			chat_list.push(jid.to_string());

			let unread = chat.unread_count;
			let pinned = chat.pinned;
			let muted = chat.muted;
			let archived = chat.archived;
			let is_locked = chat.is_locked;
			let is_community = chat.is_community;
			let preview = &*chat.preview_snippet;

			let ts = chat.timestamp;
			let preview_sender = &*chat.preview_sender;
			let preview_from_me = chat.preview_from_me;
			let preview_is_reaction = chat.preview_is_reaction;
			let preview_reaction_emoji = &*chat.preview_reaction_emoji;

			let time_str = if ts > 0 {
				if let Some(datetime) = chrono::DateTime::from_timestamp(ts, 0) {
					let local_dt: chrono::DateTime<chrono::Local> = datetime.into();
					let now_dt = chrono::Local::now();
					let msg_date = local_dt.date_naive();
					let today = now_dt.date_naive();
					let yesterday = today.pred_opt().unwrap_or(today);
					let num_days_ago = (today - msg_date).num_days();
					if msg_date == today {
						local_dt.format("%I:%M %p").to_string()
					} else if msg_date == yesterday {
						"Yesterday".to_string()
					} else if num_days_ago > 1 && num_days_ago < 7 {
						local_dt.format("%A").to_string()
					} else {
						local_dt.format("%d/%m/%Y").to_string()
					}
				} else {
					String::new()
				}
			} else {
				String::new()
			};

			let mut tags: Vec<Cow<str>> = Vec::new();
			if is_locked {
				tags.push("Locked".into());
				locked_jids.insert(jid.to_string());
			}
			if archived {
				tags.push("Archived".into());
				archived_jids.insert(jid.to_string());
			}
			if pinned {
				tags.push("Pinned".into());
				pinned_jids.insert(jid.to_string());
			}
			if muted {
				tags.push("Muted".into());
				muted_jids.insert(jid.to_string());
			}
			if is_community {
				tags.push("Community".into());
				community_jids.insert(jid.to_string());
			}
			if chat.is_channel {
				tags.push("Channel".into());
				channel_jids.insert(jid.to_string());
				let role = &*chat.channel_role;
				channel_roles.insert(jid.to_string(), role.to_string());
			}
			// A group inside a community says which one, and
			// its announcements group says so - that group
			// usually shares the community's own name.
			let parent = &*chat.parent_jid;
			if !parent.is_empty() {
				chat_parents.insert(jid.to_string(), parent.to_string());
				let community_name = &*chat.community_name;
				if chat.is_announcements {
					announcement_jids.insert(jid.to_string());
					tags.push("Announcements".into());
				}
				if !community_name.is_empty() && community_name != name {
					tags.push(format!("In {}", community_name).into());
				}
			}
			if chat.is_admin {
				admin_group_jids.insert(jid.to_string());
			}
			unread_counts.insert(jid.to_string(), unread);

			let name_line = if tags.is_empty() { name.clone() } else { format!("{} [{}]", name, tags.join("] [")) };

			let unread_line = if unread == 1 {
				"1 unread message".to_string()
			} else if unread > 1 {
				format!("{} unread messages", unread)
			} else {
				String::new()
			};

			let preview_line = if preview_is_reaction && !preview_reaction_emoji.is_empty() && !preview.is_empty() {
				let who = if preview_from_me {
					"You".to_string()
				} else if !preview_sender.is_empty() {
					preview_sender.to_string()
				} else {
					"Someone".to_string()
				};
				format!(
					"{} reacted with {} to: {}",
					who,
					preview_reaction_emoji,
					whantv::friendly_preview_text(preview)
				)
			} else if !preview.is_empty() {
				let body = whantv::friendly_preview_text(preview);
				if preview_from_me {
					format!("You: {}", body)
				} else if !preview_sender.is_empty() && jid.ends_with("@g.us") {
					format!("{}: {}", preview_sender, body)
				} else {
					body
				}
			} else {
				String::new()
			};

			let mut parts_vec: Vec<&str> = Vec::new();
			parts_vec.push(&name_line);
			if !unread_line.is_empty() {
				parts_vec.push(&unread_line);
			}
			if !time_str.is_empty() {
				parts_vec.push(&time_str);
			}
			if !preview_line.is_empty() {
				parts_vec.push(&preview_line);
			}
			let display_name = parts_vec.join(" | ");
			display_names.insert(jid.to_string(), display_name);
			contacts_map.insert(jid.to_string(), name);
		}
	}
	let mut s = state.borrow_mut();
	s.chats = chat_list;
	s.contacts = contacts_map;
	s.archived_jids = archived_jids;
	s.locked_jids = locked_jids;
	s.community_jids = community_jids;
	s.chat_parents = chat_parents;
	s.announcement_jids = announcement_jids;
	s.channel_jids = channel_jids;
	s.channel_roles = channel_roles;
	s.pinned_jids = pinned_jids;
	s.muted_jids = muted_jids;
	s.admin_group_jids = admin_group_jids;
	s.unread_counts = unread_counts;
	s.display_names = display_names;
	refresh_tray_tooltip(&s, parts);

	// Compute new filtered list
	let lower_query = s.search_query.to_lowercase();
	let mut new_filtered = Vec::new();
	let mut new_display = Vec::new();
	if s.active_tab != 1 && s.active_tab != 2 {
		for jid in chats_in_tab_order(&s) {
			if !chat_visible_in_tab(
				s.active_tab,
				jid,
				&s.archived_jids,
				&s.community_jids,
				&s.chat_parents,
				&s.channel_jids,
				&s.locked_jids,
				s.chats_unlocked,
			) {
				continue;
			}
			let name = s.contacts.get(jid).unwrap_or(jid);
			if lower_query.is_empty() || name.to_lowercase().contains(&lower_query) {
				new_filtered.push(jid.clone());
				new_display.push(s.display_names.get(jid).unwrap_or(name).clone());
			}
		}
	}

	let structure_changed = s.active_tab != 1
		&& s.active_tab != 2
		&& (new_filtered.len() != s.filtered_chats.len()
			|| new_filtered.iter().zip(s.filtered_chats.iter()).any(|(a, b)| a != b));

	// The Status and Starred tabs keep their own rows (and
	// their own filtered_chats) - a chats refresh must leave
	// them alone, or Enter on a row stops doing anything.
	let own_rows_tab = s.active_tab == 1 || s.active_tab == 2;
	if !own_rows_tab && structure_changed {
		s.filtered_chats = new_filtered;
		drop(s);
		apply_chat_filter(&mut state.borrow_mut(), parts);
	} else if !own_rows_tab {
		// Same rows in the same order: rewrite only the ones
		// whose text actually changed. A chats broadcast lands
		// every ~200ms during a sync, and rewriting every row
		// each time was pure churn (and a screen reader can
		// re-read a row whose text is set, even to itself).
		let changed: Vec<usize> = new_display
			.iter()
			.enumerate()
			.filter(|(i, dn)| s.displayed_chat_labels.get(*i) != Some(*dn))
			.map(|(i, _)| i)
			.collect();
		if !changed.is_empty() {
			parts.conversations_parts.conversations_list.freeze();
			for &i in &changed {
				parts.conversations_parts.conversations_list.set_string(i as u32, &new_display[i]);
			}
			parts.conversations_parts.conversations_list.thaw();
		}
		s.filtered_chats = new_filtered;
		s.displayed_chat_labels = new_display;
		drop(s);
	} else {
		drop(s);
	}
	// A WhatsApp link opened before the chat list was ready. Needs the state
	// free: it borrows it itself.
	crate::ui_commands::links::open_pending_whatsapp_link(state);
}

/// Just the `type` tag, with every other field skipped rather than
/// materialized. Lets the dispatcher below route the one huge payload without
/// first building a `serde_json::Value` for it.
#[derive(Deserialize)]
struct TypeTag<'a> {
	#[serde(rename = "type", borrow, default)]
	ty: std::borrow::Cow<'a, str>,
}

/// A `conversation_messages` payload. Each message is kept as a `RawValue` -
/// a borrowed slice of the original JSON text, not a parsed tree - so that a
/// message that fails to parse can be replaced by a placeholder row without
/// failing the whole chat, which is what deserializing straight into
/// `Vec<ChatMessage>` would do (`id`/`text`/`timestamp` have no serde default).
#[derive(Deserialize)]
struct ConversationMessagesEnvelope<'a> {
	#[serde(borrow)]
	data: ConversationMessagesData<'a>,
}

#[derive(Deserialize)]
struct ConversationMessagesData<'a> {
	#[serde(default)]
	jid: String,
	#[serde(borrow, default)]
	messages: Vec<&'a serde_json::value::RawValue>,
	// Set only when this is a `load_more_messages` local page (offset > 0) -
	// see `MESSAGE_PAGE_SIZE` and `ui_commands::chat_list::load_more_messages`.
	// A fresh load (opening a chat, closing search, `get_messages_around`)
	// replaces `s.messages`; this one prepends onto it instead.
	#[serde(rename = "loadMore", default)]
	load_more: bool,
	/// A page of messages newer than the ones loaded (see
	/// `apply_newer_messages`).
	#[serde(rename = "loadNewer", default)]
	load_newer: bool,
	/// False only for a `get_messages_around` window that stops short of the
	/// newest message (see `AppState::messages_at_latest`).
	#[serde(rename = "isLatest", default = "default_true")]
	is_latest: bool,
}

fn default_true() -> bool {
	true
}

/// The payloads read straight into typed structs instead of a `Value` tree.
#[derive(Debug, PartialEq, Eq)]
enum FastPath {
	ConversationMessages,
	Chats,
	Other,
}

/// Which fast path `raw` takes, read without materializing the rest of the
/// document.
fn fast_path(raw: &str) -> FastPath {
	match serde_json::from_str::<TypeTag>(raw).as_ref().map(|t| t.ty.as_ref()) {
		Ok("conversation_messages") => FastPath::ConversationMessages,
		Ok("chats") => FastPath::Chats,
		_ => FastPath::Other,
	}
}

#[cfg(test)]
fn is_conversation_messages(raw: &str) -> bool {
	fast_path(raw) == FastPath::ConversationMessages
}

/// Parses a `conversation_messages` payload into its chat jid and messages,
/// or `None` if the envelope itself is malformed.
///
/// A message that fails to parse becomes a placeholder row carrying the error,
/// exactly as the old `Value`-walking path did, rather than failing the whole
/// chat - `ChatMessage`'s `id`/`text`/`timestamp` have no serde default, so one
/// bad row would otherwise blank the conversation.
fn parse_conversation_messages(raw: &str) -> Option<(String, bool, bool, bool, Vec<ChatMessage>)> {
	let envelope = serde_json::from_str::<ConversationMessagesEnvelope>(raw).ok()?;
	let parsed = envelope
		.data
		.messages
		.into_iter()
		.map(|raw_msg| {
			serde_json::from_str::<ChatMessage>(raw_msg.get()).unwrap_or_else(|e| ChatMessage {
				text: format!("Parse error: {} for json: {}", e, raw_msg.get()),
				..Default::default()
			})
		})
		.collect();
	Some((envelope.data.jid, envelope.data.load_more, envelope.data.load_newer, envelope.data.is_latest, parsed))
}

pub(crate) fn process_websocket_message(raw: String, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	// Opening a chat delivers its entire history as one payload. Routed through
	// the generic `Value` path below, a 26k-message chat built a tree of
	// roughly 26k objects x ~28 fields - about 1.5M short-lived heap
	// allocations - purely to be walked once and thrown away. Deserializing
	// straight into `Vec<ChatMessage>` skips the tree altogether, so the only
	// allocations are the messages the UI keeps.
	//
	// Reading the tag first costs one scan and no allocations: serde skips the
	// fields it is not asked for instead of materializing them.
	let path = fast_path(&raw);
	if path == FastPath::Chats {
		match serde_json::from_str::<ChatsEnvelope>(&raw) {
			Ok(ChatsEnvelope { data: Some(chats) }) => apply_chats(chats, state, parts),
			Ok(ChatsEnvelope { data: None }) => crate::ui_commands::links::open_pending_whatsapp_link(state),
			Err(e) => eprintln!("[LOG UI] Failed to parse a chats payload: {}", e),
		}
		return;
	}
	if path == FastPath::ConversationMessages {
		let parse_started = std::time::Instant::now();
		let parsed = parse_conversation_messages(&raw);
		ui_timing_log(&format!(
			"Parsed a {} KB conversation_messages payload in {} ms",
			raw.len() / 1024,
			parse_started.elapsed().as_millis()
		));
		// The parsed messages own their data; the JSON text (tens of MB for a
		// whole chat) is dead weight from here, and used to stay alive through
		// building and showing every row.
		drop(raw);
		match parsed {
			Some((jid, load_more, load_newer, is_latest, parsed_msgs)) => {
				// The chat may have been switched again while this request was
				// in flight - applying a stale response would silently populate
				// the wrong chat's history.
				if state.borrow().selected_chat.as_deref() == Some(jid.as_str()) {
					if load_newer {
						apply_newer_messages(parsed_msgs, is_latest, state, parts);
						return;
					}
					if !load_more {
						state.borrow_mut().newer_loading = false;
						state.borrow_mut().messages_at_latest = is_latest;
					}
					apply_conversation_messages(parsed_msgs, load_more, state, parts);
				}
			}
			None => eprintln!("[LOG UI] Failed to parse a conversation_messages payload"),
		}
		return;
	}

	if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
		if let Some(t) = val.get("type").and_then(|v| v.as_str()) {
			match t {
				"connected" => {
					// A reconnect (keepalive blip, brief network drop) also
					// fires this event, not just the first connect - don't
					// clobber an in-progress "Syncing history..." status
					// with plain "Connected" just because the socket
					// happened to bounce mid-sync.
					let mid_sync = state
						.borrow()
						.history_sync_last_update
						.map(|t| t.elapsed() < std::time::Duration::from_secs(5))
						.unwrap_or(false);
					if !mid_sync {
						parts.frame.set_status_text("Status: Connected", 0);
					}

					// Emitted on every IPC connect and on every whatsmeow
					// reconnect, not just at startup. Re-running the layout
					// and focus below on a mid-session reconnect is what
					// yanked focus back to the search box.
					let first_show = !state.borrow().main_ui_shown;
					if first_show {
						parts.pairing_dialog.show(false);
						parts.qrcode_dialog.show(false);
						parts.login_panel.show(false);
						parts.nav_panel.show(true);
						parts.content_panel.show(true);
						parts.main_panel.layout();
						parts.frame.show(true);
						parts.conversations_parts.search_field.set_focus();
						state.borrow_mut().main_ui_shown = true;

						// Now that we're logged in, install the tray icon if the
						// user wants it - it's gated on login so an unpaired app
						// never parks an icon in the tray (window.rs).
						install_tray_icon(&state.borrow(), parts);
					}

					let payload = serde_json::json!({
						"action": "get_chats"
					});
					let _ = state.borrow().ws_tx.send(payload.to_string());

					// Re-state our presence on every connect: a restarted backend
					// starts out unavailable and has no idea whether the window is
					// in use.
					{
						let mut s = state.borrow_mut();
						let in_use = crate::ui_commands::misc::desired_presence(&s);
						s.presence_sent = None;
						crate::ui_commands::misc::send_presence(&mut s, in_use);
					}
				}
				"needs_login" => {
					parts.frame.set_status_text("Status: Not logged in", 0);
					parts.nav_panel.show(false);
					parts.content_panel.show(false);
					parts.main_panel.layout();
					// The login panel is a Dialog owned by the frame (see
					// set_dialog_owner). Windows won't activate an owned window
					// whose owner is hidden, so with the frame no longer shown at
					// startup we reveal it here first - otherwise the login dialog
					// shows but never takes focus and NVDA lands on nothing. A
					// visible owner is what lets the owned dialog activate.
					// Keep the frame hidden so the login dialog is the only top-level
					// window while logged out. That way alt-tabbing to whaNTV activates
					// the dialog itself - a screen reader announces the window and its
					// focused field in one step - instead of landing on an empty frame
					// behind it and needing focus force-shifted into the dialog.
					parts.frame.show(false);
					parts.pairing_dialog.show(false);
					parts.qrcode_dialog.show(false);
					parts.login_panel.show(true);
					parts.login_panel.set_focus();
					state.borrow_mut().main_ui_shown = false;
					// Logged out: pull the tray icon so an unpaired app leaves
					// nothing behind in the tray (reinstalled on connect).
					parts.tray_icon.remove_icon();

					// A "reason" means WhatsApp itself ended the session (phone
					// unlinked it, session expired, etc.) rather than us just
					// having sent our own "Log Out" - worth telling the user
					// why they suddenly landed back on the login screen.
					if let Some(reason) = val.get("data").and_then(|d| d.get("reason")).and_then(|r| r.as_str()) {
						MessageDialog::builder(
							&parts.frame,
							&format!("whaNTV was logged out of WhatsApp: {reason}"),
							"Logged Out",
						)
						.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconWarning)
						.build()
						.show_modal();
					}
				}
				"pairing_code" => {
					if let Some(code) = val.get("data").and_then(|d| d.get("code")).and_then(|c| c.as_str()) {
						parts.pairing_code_display.set_value(code);
						println!("{{'data': {{'qrcode': {{'pairingCode': '{}'}}}}}}", code);
						// One setup screen at a time: the pairing-code screen
						// replaces the phone-entry and QR screens.
						parts.login_panel.show(false);
						parts.pairing_dialog.show(true);
						parts.qrcode_dialog.show(false);
						parts.pairing_code_display.select_all();
						parts.pairing_code_display.set_focus();
						unsafe {
							let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
								windows::Win32::UI::WindowsAndMessaging::MB_OK,
							);
						}
						crate::announce::announce(
							parts.pairing_announcer.clone(),
							&format!("Pairing code updated: {}", code),
						);
					}
				}
				"call_connected" => {
					if let Some(data) = val.get("data") {
						let call_id = data.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
						if let Some(audio_port) = data.get("audio_port").and_then(|p| p.as_u64()) {
							let mut s = state.borrow_mut();
							call_log(parts, &s, "connected", call_id);
							// Unlike call_ended/call_answer_failed, this used to act
							// on any call_connected regardless of call_id - a
							// call_connected arriving after we'd already given up on
							// (or left) that call, e.g. the answer watchdog above,
							// reopened the call UI and started a fresh audio stream
							// for a call the user was already told had ended.
							if s.active_call_id.as_deref() != Some(call_id) {
								return;
							}
							s.call_answer_deadline = None;
							s.incoming_call_caller = None;
							s.call_video_port =
								data.get("video_port").and_then(|p| p.as_u64()).map(|p| p as u16).filter(|&p| p != 0);
							stop_call_ringing_sound(&mut s);
							if let Ok(streams) = call_audio::start_audio_streams(
								audio_port as u16,
								s.settings.input_device_name.clone(),
								s.settings.output_device_name.clone(),
							) {
								s.active_call_audio = Some(streams);
								// Say so on a video call: the backend answers those with the
								// camera off, and silence about it would leave the caller
								// wondering whether they are being seen.
								let connected_msg = if s.call_video.is_video {
									"Video call connected. Your camera is off."
								} else {
									"Call connected."
								};
								announce_call(parts, connected_msg);

								if s.call_participants.is_empty() {
									parts.call_parts.status_label.set_label(connected_msg);
								} else {
									parts
										.call_parts
										.status_label
										.set_label(&group_call_status_text(&s.call_participants));
								}
								parts.call_parts.accept_btn.show(false);
								parts.call_parts.reject_btn.show(false);
								parts.call_parts.mute_btn.show(true);
								parts.call_parts.camera_btn.show(true);
								parts.call_parts.camera_btn.set_label(if s.call_video.local_camera_on {
									"Turn camera off"
								} else {
									"Turn camera on"
								});
								parts.call_parts.leave_btn.show(true);
								parts.call_parts.add_participant_btn.show(true);
								show_call_device_combos(&s, parts);
								parts.call_parts.frame.show(true);
								let autostart_camera = s.call_video.autostart_camera;
								s.call_video.autostart_camera = false;
								drop(s);
								if autostart_camera {
									crate::ui_commands::calls::toggle_camera(state, parts);
								}
							} else {
								announce_call(parts, "Failed to start audio stream.");
							}
						}
					}
				}
				"call_placed" => {
					if let Some(data) = val.get("data") {
						if let Some(call_id) = data.get("call_id").and_then(|c| c.as_str()) {
							let mut s = state.borrow_mut();
							call_log(parts, &s, "placed", call_id);
							s.call_placing = false;
							if s.call_pending_cancel {
								// Leave was pressed before we had a call_id to leave with (see
								// call_placing doc comment). Now that the backend has one, hang
								// up immediately instead of leaving the call ringing on the
								// peer's end with nothing on screen to show for it.
								s.call_pending_cancel = false;
								let msg = serde_json::json!({
									"action": "leave_call",
									"data": { "call_id": call_id }
								});
								let _ = s.ws_tx.send(msg.to_string());
							} else {
								s.active_call_id = Some(call_id.to_string());
								s.call_participants.clear();
								s.call_video = CallVideo::default();
								s.active_call_video_send = None;
								s.call_video_port = None;
								let self_phone = data.get("self_phone").and_then(|p| p.as_str()).unwrap_or("");
								start_call_ringback(&mut s, self_phone);
							}
						}
					}
				}
				"call_audio_health" => {
					if let Some(data) = val.get("data") {
						let call_id = data.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
						if state.borrow().active_call_id.as_deref() != Some(call_id) {
							return;
						}
						let stalled = data.get("stalled").and_then(|s| s.as_bool()).unwrap_or(false);
						let message = if stalled {
							"No sound is reaching you from the call. Check your internet connection."
						} else {
							"Sound from the call isn't getting through."
						};
						announce_call(parts, message);
					}
				}
				"call_participant_added" | "call_participant_add_failed" => {
					if let Some(data) = val.get("data") {
						let call_id = data.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
						if state.borrow().active_call_id.as_deref() != Some(call_id) {
							return;
						}
						let message = if t == "call_participant_added" {
							let name = data.get("name").and_then(|n| n.as_str()).unwrap_or("Contact");
							format!("Ringing {}.", name)
						} else {
							data.get("message")
								.and_then(|m| m.as_str())
								.unwrap_or("Couldn't add that person to the call.")
								.to_string()
						};
						announce_call(parts, &message);
					}
				}
				"call_participants" => {
					if let Some(data) = val.get("data") {
						let call_id = data.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
						let mut s = state.borrow_mut();
						if s.active_call_id.as_deref() != Some(call_id) {
							return;
						}
						let participants = parse_call_participants(data);
						// The first roster of a call is the starting line-up (who we're
						// ringing, or who's already in a call we're being invited to) -
						// the status label covers it, so only later changes are announced.
						let changes = if s.call_participants.is_empty() {
							Vec::new()
						} else {
							call_participant_changes(&s.call_participants, &participants)
						};
						s.call_participants = participants;
						// Leave the label alone while an incoming call is still ringing
						// or being answered - it's showing who's calling / "Answering...".
						if s.incoming_call_caller.is_none() && s.call_answer_deadline.is_none() {
							parts.call_parts.status_label.set_label(&group_call_status_text(&s.call_participants));
						}
						drop(s);
						if !changes.is_empty() {
							announce_call(parts, &changes.join(" "));
						}
					}
				}
				"call_video_state" => {
					if let Some(data) = val.get("data") {
						let call_id = data.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
						let event = data.get("peer_video").and_then(|e| e.as_str()).unwrap_or("");
						let mut s = state.borrow_mut();
						if s.active_call_id.as_deref() != Some(call_id) {
							return;
						}
						let announcement = peer_video_announcement(
							event,
							s.call_video.peer_camera_on,
							s.call_video.peer_name.as_deref(),
						);
						match event {
							"on" => s.call_video.peer_camera_on = Some(true),
							"off" => s.call_video.peer_camera_on = Some(false),
							_ => {}
						}
						drop(s);
						if let Some(text) = announcement {
							announce_call(parts, &text);
						}
					}
				}
				"call_video_keyframe_request" => {
					if let Some(data) = val.get("data") {
						let call_id = data.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
						let s = state.borrow();
						if s.active_call_id.as_deref() == Some(call_id) {
							if let Some(send) = &s.active_call_video_send {
								send.request_keyframe();
							}
						}
					}
				}
				"call_set_video_failed" => {
					if let Some(data) = val.get("data") {
						let call_id = data.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
						let mut s = state.borrow_mut();
						if s.active_call_id.as_deref() != Some(call_id) {
							return;
						}
						s.active_call_video_send = None;
						s.call_video.local_camera_on = false;
						parts.call_parts.camera_btn.set_label("Turn camera on");
						drop(s);
						announce_call(parts, "Couldn't turn the camera on for this call.");
					}
				}
				"call_failed" => {
					let mut s = state.borrow_mut();
					if s.call_placing {
						s.call_placing = false;
						s.call_pending_cancel = false;
						s.call_video = CallVideo::default();
						s.active_call_video_send = None;
						s.call_video_port = None;
						parts.call_parts.frame.show(false);
						stop_call_ringing_sound(&mut s);
						play_call_fail_sound(&mut s);
						let message = val
							.get("data")
							.and_then(|d| d.get("message"))
							.and_then(|m| m.as_str())
							.unwrap_or("Failed to place call.");
						announce_call(parts, message);
					}
				}
				"call_ended" => {
					if let Some(data) = val.get("data") {
						if let Some(call_id) = data.get("call_id").and_then(|c| c.as_str()) {
							let mut s = state.borrow_mut();
							call_log(parts, &s, "ended", call_id);
							if s.active_call_id.as_deref() == Some(call_id) {
								s.active_call_id = None;
								s.active_call_audio = None;
								s.call_participants.clear();
								s.call_video = CallVideo::default();
								s.active_call_video_send = None;
								s.call_video_port = None;
								s.call_answer_deadline = None;
								parts.call_parts.frame.show(false);
								stop_call_ringing_sound(&mut s);
								let reason = data.get("reason").and_then(|r| r.as_str()).unwrap_or("");
								// The person we called is on another call: WhatsApp
								// plays its busy tone instead of the hang-up sound.
								if reason == "busy" && s.incoming_call_caller.is_none() {
									crate::audio::play_busy_tone(&mut s);
								} else {
									play_call_fail_sound(&mut s);
								}
								// Still set here means an incoming call stopped ringing
								// without being answered here. A plain missed call stays
								// silent; one picked up or declined on another device
								// (the phone) says so, since otherwise the ringing just
								// stops for no audible reason. The call window is hidden
								// above, so this goes to the main window's announcer.
								let summary = data.get("summary").and_then(|s| s.as_str()).unwrap_or("");
								if s.incoming_call_caller.take().is_none() {
									announce_call(
										parts,
										if reason == "busy" { "Busy, on another call." } else { "Call ended." },
									);
								} else if summary.contains("on another device") {
									announce_call(parts, &format!("{}.", summary));
								}
							}
						}
					}
				}
				"call_answer_failed" => {
					if let Some(data) = val.get("data") {
						if let Some(call_id) = data.get("call_id").and_then(|c| c.as_str()) {
							let mut s = state.borrow_mut();
							if s.active_call_id.as_deref() == Some(call_id) {
								s.active_call_id = None;
								s.active_call_audio = None;
								s.call_participants.clear();
								s.call_video = CallVideo::default();
								s.active_call_video_send = None;
								s.call_video_port = None;
								s.call_answer_deadline = None;
								s.incoming_call_caller = None;
								parts.call_parts.frame.show(false);
								stop_call_ringing_sound(&mut s);
								play_call_fail_sound(&mut s);
								let message =
									data.get("message").and_then(|m| m.as_str()).unwrap_or("Failed to answer call.");
								announce_call(parts, message);
							}
						}
					}
				}
				"incoming_call" => {
					if let Some(data) = val.get("data") {
						let caller = data.get("caller").and_then(|c| c.as_str()).unwrap_or("Unknown");
						if let Some(call_id) = data.get("call_id").and_then(|c| c.as_str()) {
							let mut s = state.borrow_mut();
							call_log(parts, &s, "incoming", call_id);
							s.active_call_id = Some(call_id.to_string());
							s.call_answer_deadline = None;
							// Prefer the backend-resolved name: the caller JID is
							// typically a @lid, which s.contacts (keyed by phone JID)
							// cannot resolve.
							let name = data
								.get("caller_name")
								.and_then(|n| n.as_str())
								.filter(|n| !n.is_empty() && *n != caller)
								.map(|n| n.to_string())
								.or_else(|| s.contacts.get(caller).cloned())
								.unwrap_or_else(|| caller.to_string());
							s.incoming_call_caller = Some(name.clone());
							s.call_participants.clear();
							s.call_video = CallVideo::default();
							s.active_call_video_send = None;
							s.call_video_port = None;
							let is_group = data.get("is_group").and_then(|g| g.as_bool()).unwrap_or(false);
							let group_name = data
								.get("group_name")
								.and_then(|n| n.as_str())
								.filter(|n| !n.is_empty())
								.map(|n| n.to_string())
								.or_else(|| {
									let group_jid = data.get("group_jid").and_then(|j| j.as_str())?;
									s.display_names.get(group_jid).or_else(|| s.contacts.get(group_jid)).cloned()
								});
							let is_video = data.get("is_video").and_then(|v| v.as_bool()).unwrap_or(false);
							let description =
								incoming_call_description(is_group, is_video, &name, group_name.as_deref());
							s.call_video = CallVideo {
								is_video,
								peer_name: if is_group { None } else { Some(name.clone()) },
								peer_camera_on: Some(is_video),
								local_camera_on: false,
								autostart_camera: false,
							};
							parts.call_parts.frame.set_title(call_window_title(is_group, is_video));
							parts.call_parts.status_label.set_label(&format!("Incoming {}", description));
							parts.call_parts.accept_btn.show(true);
							parts.call_parts.reject_btn.show(true);
							parts.call_parts.mute_btn.show(false);
							parts.call_parts.camera_btn.show(false);
							parts.call_parts.leave_btn.show(false);
							parts.call_parts.add_participant_btn.show(false);
							parts.call_parts.devices_panel.show(false);
							// Pop the call dialog to the front the same way a second
							// launch instance activates the main window (see
							// UiCommand::ActivateWindow) - just calling show(true) is not
							// enough to steal focus if the app is minimized or behind
							// other windows.
							parts.call_parts.frame.show(true);
							parts.call_parts.frame.iconize(false);
							parts.call_parts.frame.raise();
							start_call_ringing_sound(&mut s, include_bytes!("../sounds/ringtone_incoming_call.ogg"));
							if s.settings.notifications_enabled {
								let mut toast_body = description.clone();
								toast_body[..1].make_ascii_uppercase();
								show_toast_notification("Incoming call", &toast_body);
							}
							// The call window was just raised, so this goes to its own
							// announcer (announce_call picks it while it's shown).
							announce_call(parts, &format!("Incoming {}", description));
							call_log(parts, &s, "incoming shown", call_id);
						}
					}
				}
				"qr" => {
					if state.borrow().is_pairing_active {
						return;
					}
					if let Some(code) = val.get("data").and_then(|d| d.get("code")).and_then(|c| c.as_str()) {
						if let Ok(qr) = qrcode::QrCode::new(code) {
							let image = qr.render::<image::Rgba<u8>>().quiet_zone(true).module_dimensions(5, 5).build();
							let width = image.width();
							let height = image.height();
							let data = image.into_raw();
							let mut png_bytes = Vec::new();
							let encoder = image::codecs::png::PngEncoder::new(&mut png_bytes);
							use image::ImageEncoder;
							if encoder.write_image(&data, width, height, image::ExtendedColorType::Rgba8).is_ok() {
								use base64::{Engine as _, engine::general_purpose};
								let b64 = general_purpose::STANDARD.encode(&png_bytes);
								println!("{{'data': {{'qrcode': {{'base64': 'data:image/png;base64,{}'}}}}}}", b64);
							}
							if let Some(bitmap) = wxdragon::bitmap::Bitmap::from_rgba(&data, width, height) {
								parts.qrcode_display.set_min_size(Size::new(width as i32, height as i32));
								parts.qrcode_display.set_bitmap(&bitmap);
								parts.qrcode_dialog.fit();
								parts.qrcode_dialog.layout();
							}
						}
						// One setup screen at a time: the QR screen replaces the
						// phone-entry and pairing screens rather than stacking
						// beside them.
						parts.login_panel.show(false);
						parts.pairing_dialog.show(false);
						parts.qrcode_dialog.show(true);
						parts.qrcode_cancel_btn.set_focus();
						unsafe {
							let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
								windows::Win32::UI::WindowsAndMessaging::MB_OK,
							);
						}
						crate::announce::announce(parts.qrcode_announcer.clone(), "QR code updated");
					}
				}
				"error" => {
					if let Some(msg) = val.get("data").and_then(|d| d.get("message")).and_then(|m| m.as_str()) {
						// The message row itself (if any) is already updated to
						// "failed" by its own "message" event, which the backend
						// always sends before this one; this is just the popup.
						parts.conversations_parts.play_voice_btn.set_label("Play voice message");
						MessageDialog::builder(&parts.frame, msg, "Error")
							.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
							.build()
							.show_modal();
					}
				}
				"connection_state" => {
					let data = val.get("data");
					let conn_state = data.and_then(|d| d.get("state")).and_then(|v| v.as_str()).unwrap_or("");
					let message = data.and_then(|d| d.get("message")).and_then(|v| v.as_str()).unwrap_or("");
					let retry_in = data.and_then(|d| d.get("retry_in")).and_then(|v| v.as_u64());

					let mut s = state.borrow_mut();
					s.conn_online = conn_state == "online";
					// Only speak on the edges of an outage (and for a manual
					// Reconnect Now): a quick keepalive blip that reconnects on
					// the first try says nothing, and retries during a long
					// outage don't repeat the same announcement every time.
					let announcement = match conn_state {
						"online" => {
							let mid_sync = s
								.history_sync_last_update
								.map(|t| t.elapsed() < std::time::Duration::from_secs(5))
								.unwrap_or(false);
							if !mid_sync {
								parts.frame.set_status_text("Status: Connected", 0);
							}
							let speak = s.conn_outage_announced || s.conn_manual_reconnect;
							s.conn_outage_announced = false;
							s.conn_manual_reconnect = false;
							speak.then(|| "Connected to WhatsApp.".to_string())
						}
						"connecting" => {
							let label = if message.is_empty() {
								"Status: Connecting...".to_string()
							} else {
								format!("Status: {message} Reconnecting...")
							};
							parts.frame.set_status_text(&label, 0);
							None
						}
						"offline" => {
							let reason = if message.is_empty() { "Can't connect to WhatsApp." } else { message };
							// A temp ban can report its retry hours away, not seconds.
							let label = match retry_in {
								Some(secs @ 1..=119) => {
									format!("Status: Offline. {reason} Retrying in {secs} seconds.")
								}
								Some(secs @ 120..=7199) => {
									format!("Status: Offline. {reason} Retrying in {} minutes.", secs.div_ceil(60))
								}
								Some(secs @ 7200..) => {
									format!("Status: Offline. {reason} Retrying in {} hours.", secs.div_ceil(3600))
								}
								_ => format!("Status: Offline. {reason} Retrying automatically."),
							};
							parts.frame.set_status_text(&label, 0);
							let speak = !s.conn_outage_announced || s.conn_manual_reconnect;
							s.conn_outage_announced = true;
							s.conn_manual_reconnect = false;
							speak.then(|| format!("{reason} whaNTV will keep trying to reconnect."))
						}
						"stopped" => {
							// The backend already raised an error dialog explaining
							// this one; the status bar just keeps it visible.
							parts.frame.set_status_text(&format!("Status: {message}"), 0);
							s.conn_outage_announced = true;
							s.conn_manual_reconnect = false;
							None
						}
						_ => None,
					};
					drop(s);
					if let Some(text) = announcement {
						crate::announce::announce(parts.conversations_parts.messages_announcer.clone(), &text);
					}
				}
				"history_sync" => {
					if let Some(data) = val.get("data") {
						let chats_done = data.get("chatsDone").and_then(|v| v.as_i64()).unwrap_or(0);
						let msgs_done = data.get("msgsDone").and_then(|v| v.as_i64()).unwrap_or(0);
						let label = format!(
							"Status: Connected (Syncing history... {msgs_done} messages, {chats_done} chats so far)"
						);
						parts.frame.set_status_text(&label, 0);
						{
							let mut s = state.borrow_mut();
							let now = std::time::Instant::now();
							s.history_sync_last_update = Some(now);
							s.history_sync_armed_until = Some(now + std::time::Duration::from_secs(60));
						}
					}
				}
				"history_access" => {
					// The phone limited what it shares with this computer.
					let none = val.get("data").and_then(|d| d.get("none")).and_then(|v| v.as_bool()).unwrap_or(false);
					let msg = if none {
						"Your phone didn't share any message history with whaNTV. New messages will still arrive."
					} else {
						"Your phone shared only your recent messages with whaNTV, not your full chat history. Older messages load from your phone when you scroll to the top of a chat."
					};
					MessageDialog::builder(&parts.frame, msg, "Chat history")
						.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
						.build()
						.show_modal();
				}
				"history_chats_synced" => {
					// Sent after every history-sync chunk - bulk and on-demand alike -
					// with the chats it just wrote messages for.
					let jids: Vec<&str> = val
						.get("data")
						.and_then(|d| d.get("jids"))
						.and_then(|j| j.as_array())
						.map(|a| a.iter().filter_map(|j| j.as_str()).collect())
						.unwrap_or_default();
					let mut s = state.borrow_mut();
					// A chat marked out of history (the phone answered too late, or
					// its history hadn't synced yet when it was scrolled to the top)
					// has more now - without this it stayed "complete" until whaNTV
					// was restarted.
					for jid in &jids {
						s.completed_history_chats.remove(*jid);
					}
					let Some(open_jid) = s.selected_chat.clone() else { return };
					if !jids.contains(&open_jid.as_str()) {
						return;
					}
					if s.phone_history_pending_jid.as_deref() == Some(open_jid.as_str()) {
						// The phone's answer, if any, is already in the local
						// database by the time this broadcast arrives - re-run the
						// same local page query rather than reloading the whole
						// chat. An empty result here (handled in
						// apply_conversation_messages) is what finally marks the jid
						// exhausted.
						let payload = crate::ui_commands::chat_list::older_page_request(&s.messages, &open_jid);
						let _ = s.ws_tx.send(payload.to_string());
					} else if !s.history_loading && s.quiet_reload_jid.is_none() {
						// Older messages for the open chat can page in locally again.
						s.no_more_messages = false;
						// Opened before any of its history had arrived: fill it in
						// now instead of leaving it blank until it's reopened.
						if whantv::real_message_count(&s.messages) == 0 && s.messages_at_latest {
							s.quiet_reload_jid = Some(open_jid.clone());
							let payload = serde_json::json!({
								"action": "get_messages",
								"data": { "jid": open_jid, "limit": crate::MESSAGE_PAGE_SIZE, "offset": 0 }
							});
							let _ = s.ws_tx.send(payload.to_string());
						}
					}
				}
				"chat_lock_verify_result" => {
					if let Some(data) = val.get("data") {
						let success = data.get("success").and_then(|s| s.as_bool()).unwrap_or(false);
						if success {
							let mut s = state.borrow_mut();
							s.chats_unlocked = true;
							s.search_query.clear(); // Clear the search bar
							parts.conversations_parts.search_field.set_value("");
							apply_chat_filter(&mut s, parts);
							crate::announce::announce(
								parts.conversations_parts.messages_announcer.clone(),
								"Locked chats shown.",
							);
						}
					}
				}
				"message" => {
					if let Some(data) = val.get("data") {
						let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
						let from = data.get("from").and_then(|f| f.as_str()).unwrap_or("");
						let text = data.get("text").and_then(|t| t.as_str()).unwrap_or("");
						// Group event lines ("X joined", "Y left", a new subject) never
						// alert or count as unread in the official apps, muted or not.
						let is_system = data.get("mediaType").and_then(|m| m.as_str()) == Some("system");

						let mut s = state.borrow_mut();

						// A window opened around a search hit stops short of the
						// newest messages; appending a live one under it would put
						// it right after an old message with a silent gap between.
						// Treat the chat as a background one until it's reloaded.
						if s.selected_chat.as_deref() == Some(jid) && s.messages_at_latest {
							let is_from_me = data
								.get("from_me")
								.and_then(|f| f.as_bool())
								.unwrap_or_else(|| from == "me" || from == "You");
							let is_group = jid.ends_with("@g.us");
							let ts = data
								.get("timestamp")
								.and_then(|t| t.as_i64().or_else(|| t.as_f64().map(|f| f as i64)))
								.unwrap_or_else(|| {
									std::time::SystemTime::now()
										.duration_since(std::time::UNIX_EPOCH)
										.unwrap_or_default()
										.as_secs() as i64
								});
							let msg_id = data.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();

							// Matches either the old client-side upload placeholder (by
							// its temp_upload_ prefix) or, now that the backend sends the
							// local echo itself with a real id from the start, a message
							// we already have that's transitioning status (pending/uploading
							// -> sent/failed). Either way this is an update-in-place, not a
							// new row.
							// An incoming message can also replace a row we already show:
							// the "Waiting for this message" placeholder the backend
							// stores when a message can't be decrypted yet is swapped
							// for the real one under the same id once it arrives.
							let mut temp_msg_idx = None;
							for (idx, msg) in s.messages.iter().enumerate() {
								if (!msg_id.is_empty() && msg.id == msg_id)
									|| (is_from_me && msg.id.starts_with("temp_upload_"))
								{
									temp_msg_idx = Some(idx);
									break;
								}
							}

							if let Some(idx) = temp_msg_idx {
								let pick_id = s.messages[idx].id.clone();
								let status = data
									.get("status")
									.and_then(|st| st.as_str())
									.filter(|st| !st.is_empty())
									.unwrap_or("sent");
								let field = |name: &str| data.get(name).and_then(|v| v.as_str()).unwrap_or("");
								// The quote fields have gone by either spelling.
								let quote_field = |snake: &str, camel: &str| {
									data.get(snake).or_else(|| data.get(camel)).and_then(|v| v.as_str()).unwrap_or("")
								};
								let target_di = update_loaded_message(&mut s, &pick_id, |m| {
									m.id = msg_id.clone();
									m.text = text.to_string();
									m.timestamp = ts;
									if is_from_me {
										m.sent_at = ts;
										m.status = status.to_string();
									}
									m.media_type = field("mediaType").to_string();
									m.media_filename = field("mediaFilename").to_string();
									// Our own file is already on disk, so it gets Open and Save
									// as rather than Download. Kept when an update leaves it out.
									let path = field("mediaPath");
									if !path.is_empty() {
										m.media_path = path.to_string();
									}
									m.quoted_id = quote_field("quoted_id", "quotedId").to_string();
									m.quoted_text = quote_field("quoted_text", "quotedText").to_string();
									m.quoted_sender = quote_field("quoted_sender", "quotedSender").to_string();
								});
								let updated_display = target_di
									.and_then(|di| s.display_items.get(di))
									.map(|item| item.format_display(is_group))
									.unwrap_or_default();

								if let Some(di) = target_di {
									parts.conversations_parts.messages_list.set_string(di as u32, &updated_display);
									crate::announce::announce(
										parts.conversations_parts.messages_announcer.clone(),
										&updated_display,
									);
									if parts.conversations_parts.messages_list.get_selection() == Some(di as u32) {
										refresh_voice_action_button(&s, &parts.conversations_parts);
									}
								}
							} else {
								let sender_name = if is_from_me {
									"You".to_string()
								} else {
									let mut clean_jid = from.to_string();
									if let Some(idx) = from.find(':') {
										clean_jid = format!("{}@s.whatsapp.net", &from[..idx]);
									}
									data.get("senderName")
										.and_then(|n| n.as_str())
										.filter(|n| !n.is_empty())
										.map(|n| n.to_string())
										.or_else(|| s.contacts.get(&clean_jid).cloned())
										.or_else(|| s.contacts.get(from).cloned())
										.unwrap_or(clean_jid)
								};

								let mut insert_date = false;
								let mut divider_label = String::new();
								if let Some(datetime) = chrono::DateTime::from_timestamp(ts, 0) {
									let local_datetime: chrono::DateTime<chrono::Local> = datetime.into();
									let day = whantv::date_key(local_datetime);
									if let Some(last_msg) = s.messages.last() {
										if !last_msg.is_date_divider {
											if let Some(last_dt) =
												chrono::DateTime::from_timestamp(last_msg.timestamp, 0)
											{
												let last_local: chrono::DateTime<chrono::Local> = last_dt.into();
												if day != whantv::date_key(last_local) {
													insert_date = true;
												}
											}
										}
									} else {
										insert_date = true;
									}
									if insert_date {
										divider_label = whantv::format_date_divider_label(local_datetime);
									}
								}

								let quoted_id = data
									.get("quoted_id")
									.or_else(|| data.get("quotedId"))
									.and_then(|q| q.as_str())
									.unwrap_or("")
									.to_string();
								let mut quoted_text = data
									.get("quoted_text")
									.or_else(|| data.get("quotedText"))
									.and_then(|q| q.as_str())
									.unwrap_or("")
									.to_string();
								let mut quoted_sender = data
									.get("quoted_sender")
									.or_else(|| data.get("quotedSender"))
									.and_then(|q| q.as_str())
									.unwrap_or("")
									.to_string();

								// The live push event doesn't always carry the quote preview
								// (e.g. server-side lookup races). Backfill it from the
								// already-loaded chat so reply styling doesn't depend on
								// closing and reopening the chat to pick it up on next load.
								if !quoted_id.is_empty() && (quoted_text.is_empty() || quoted_sender.is_empty()) {
									if let Some(original) = s.messages.iter().find(|m| m.id == quoted_id) {
										if quoted_text.is_empty() {
											quoted_text =
												if original.is_audio || original.text.starts_with("[Voice Message]") {
													"[Voice Message]".to_string()
												} else if original.text.is_empty() {
													"[Media]".to_string()
												} else {
													original.text.clone()
												};
										}
										if quoted_sender.is_empty() {
											quoted_sender = if original.from_me {
												"me".to_string()
											} else if !original.sender_name.is_empty() {
												original.sender_name.clone()
											} else {
												original.sender_jid.clone()
											};
										}
									}
								}

								if insert_date && !divider_label.is_empty() {
									let divider = whantv::Msg::new(ChatMessage {
										id: String::new(),
										text: divider_label,
										timestamp: ts,
										is_date_divider: true,
										..Default::default()
									});
									s.messages.push(divider.clone());
									s.display_items.push(whantv::DisplayItem::Single(divider.clone()));
									parts.conversations_parts.messages_list.append(&divider.format_display(is_group));
								}

								// Shared by `messages` and `display_items` (see `whantv::Msg`).
								let chat_msg = whantv::Msg::new(ChatMessage {
									id: msg_id.clone(),
									text: text.to_string(),
									timestamp: ts,
									from_me: is_from_me,
									sender_jid: from.to_string(),
									sender_name,
									is_audio: text.starts_with("[Voice Message]") || text.starts_with("[Audio]"),
									// The backend now tells us the real status up front for our
									// own sends (pending/uploading/sent/failed); fall back to an
									// out-of-order receipt, then "sent", for anything else.
									status: data
										.get("status")
										.and_then(|st| st.as_str())
										.filter(|st| !st.is_empty())
										.map(|st| st.to_string())
										.or_else(|| s.pending_receipts.remove(&msg_id))
										.unwrap_or_else(|| "sent".to_string()),
									sent_at: if is_from_me { ts } else { 0 },
									quoted_id,
									quoted_text,
									quoted_sender,
									media_type: data
										.get("mediaType")
										.and_then(|m| m.as_str())
										.unwrap_or("")
										.to_string(),
									media_filename: data
										.get("mediaFilename")
										.and_then(|m| m.as_str())
										.unwrap_or("")
										.to_string(),
									media_path: data
										.get("mediaPath")
										.and_then(|m| m.as_str())
										.unwrap_or("")
										.to_string(),
									media_size: data.get("mediaSize").and_then(|m| m.as_i64()).unwrap_or(0),
									album_id: data.get("albumId").and_then(|m| m.as_str()).unwrap_or("").to_string(),
									poll_results: data
										.get("pollResults")
										.and_then(|m| m.as_str())
										.unwrap_or("")
										.to_string(),
									..Default::default()
								});
								s.messages.push(chat_msg.clone());

								// Incremental display_items update instead of full O(n) regrouping
								let is_deleted_msg = chat_msg.is_deleted;
								let is_media_msg = !is_deleted_msg
									&& (chat_msg.media_type == "image"
										|| chat_msg.media_type == "video"
										|| chat_msg.text.starts_with("[Photo]")
										|| chat_msg.text.starts_with("[Image]")
										|| chat_msg.text.starts_with("[Video]"));

								let should_merge = if is_media_msg {
									if let Some(last_item) = s.display_items.last() {
										match last_item {
											whantv::DisplayItem::Single(prev_msg) => {
												let prev_is_media = !prev_msg.is_deleted
													&& (prev_msg.media_type == "image"
														|| prev_msg.media_type == "video"
														|| prev_msg.text.starts_with("[Photo]")
														|| prev_msg.text.starts_with("[Image]")
														|| prev_msg.text.starts_with("[Video]"));
												prev_is_media && whantv::same_album(prev_msg, &chat_msg)
											}
											whantv::DisplayItem::Album { messages, .. } => {
												messages.last().is_some_and(|m| whantv::same_album(m, &chat_msg))
											}
										}
									} else {
										false
									}
								} else {
									false
								};

								if should_merge {
									// Merging into the previous item only changes that one
									// row's label (e.g. "2 photos" -> "3 photos") - update it
									// in place instead of regrouping and re-appending the
									// whole (unvirtualized) list.
									let last_idx = s.display_items.len() - 1;
									let merged = match s.display_items.pop().unwrap() {
										whantv::DisplayItem::Single(prev_msg) => whantv::DisplayItem::Album {
											id: prev_msg.id.clone(),
											sender_jid: prev_msg.sender_jid.clone(),
											sender_name: prev_msg.sender_name.clone(),
											from_me: prev_msg.from_me,
											timestamp: prev_msg.timestamp,
											messages: vec![prev_msg, chat_msg.clone()],
										},
										whantv::DisplayItem::Album {
											id,
											sender_jid,
											sender_name,
											from_me,
											timestamp,
											mut messages,
										} => {
											messages.push(chat_msg.clone());
											whantv::DisplayItem::Album {
												id,
												sender_jid,
												sender_name,
												from_me,
												timestamp,
												messages,
											}
										}
									};
									parts
										.conversations_parts
										.messages_list
										.set_string(last_idx as u32, &merged.format_display(is_group));
									s.display_items.push(merged);
								} else {
									s.display_items.push(whantv::DisplayItem::Single(chat_msg.clone()));
									parts.conversations_parts.messages_list.append(&chat_msg.format_display(is_group));
								}
								refresh_selection_list(&mut s, parts);

								let count = parts.conversations_parts.messages_list.get_count();
								if count > 0 && is_from_me {
									parts.conversations_parts.messages_list.set_selection((count - 1) as u32, true);
									refresh_voice_action_button(&s, &parts.conversations_parts);
								}
								let formatted = chat_msg.format_display(is_group);
								crate::announce::announce(
									parts.conversations_parts.messages_announcer.clone(),
									&formatted,
								);

								if !is_from_me {
									let sounds_enabled =
										s.settings.sounds_enabled && !s.muted_jids.contains(jid) && !is_system;
									if s.window_focused {
										let mark_jid = jid.to_string();
										let mark_msg_id = msg_id.clone();
										let tx = s.ws_tx.clone();
										let payload = serde_json::json!({
											"action": "mark_read",
											"data": { "jid": mark_jid, "message_ids": vec![mark_msg_id] }
										});
										let _ = tx.send(payload.to_string());
									} else if !is_system {
										// The window isn't focused, so the user hasn't actually
										// seen this yet — don't send a read receipt. Queue it
										// for a batched mark_read once focus returns, and show
										// (or grow) an "N unread messages" divider right now so
										// it's already there when they look back at the chat —
										// updated/inserted in place rather than rebuilding the
										// whole list, so messages just quietly stack up in the
										// background like the official WhatsApp client.
										s.background_unread_message_ids.push(msg_id.clone());
										let is_group = jid.ends_with("@g.us");
										let existing_di = s.display_items.iter().position(
											|item| matches!(item, whantv::DisplayItem::Single(m) if m.is_unread_divider),
										);
										if let Some(di) = existing_di {
											s.unread_divider_count += 1;
											let count = s.unread_divider_count;
											let label = if count == 1 {
												"1 unread message".to_string()
											} else {
												format!("{} unread messages", count)
											};
											update_loaded_messages(
												&mut s,
												|m| m.is_unread_divider,
												|m| m.text = label.clone(),
											);
											parts.conversations_parts.messages_list.set_string(di as u32, &label);
										} else {
											s.unread_divider_jid = Some(jid.to_string());
											s.unread_divider_count = 1;
											// The message that just triggered this branch was
											// appended as the last entry above, so the divider
											// belongs immediately before it.
											let divider = whantv::Msg::new(ChatMessage {
												text: "1 unread message".to_string(),
												is_unread_divider: true,
												..Default::default()
											});
											let insert_at = s.messages.len().saturating_sub(1);
											s.messages.insert(insert_at, divider.clone());
											let di_insert_at = s.display_items.len().saturating_sub(1);
											s.display_items
												.insert(di_insert_at, whantv::DisplayItem::Single(divider.clone()));
											parts
												.conversations_parts
												.messages_list
												.insert(&divider.format_display(is_group), di_insert_at);
										}
									}
									play_notification_sound(
										NotificationSound::MessageCurrent,
										sounds_enabled,
										s.settings.output_device_name.clone(),
									);
								}
							}
						} else {
							let is_ignored = jid == "status@broadcast"
								|| jid.ends_with("@broadcast")
								|| jid.ends_with("@newsletter");
							if !is_ignored {
								let is_from_me = data
									.get("from_me")
									.and_then(|f| f.as_bool())
									.unwrap_or_else(|| from == "me" || from == "You");
								if !is_from_me && !is_system {
									// Archived chats default to silent and can be dialled back up per chat
									// from the chat-list context menu; muting still overrides both.
									let alert = whantv::incoming_alert(
										s.muted_jids.contains(jid),
										s.archived_jids.contains(jid),
										s.settings.archived_alert(jid),
									);
									if alert.sound && !crate::do_not_disturb::should_silence(&s.settings) {
										play_notification_sound(
											NotificationSound::MessageBackground,
											s.settings.sounds_enabled,
											s.settings.output_device_name.clone(),
										);
									}
									if alert.toast && s.settings.notifications_enabled {
										// The backend's "message" event carries no sender_name
										// field - resolve the display name from the contacts
										// map ourselves, same as everywhere else in the UI, so
										// the notification doesn't show a raw JID.
										let body = whantv::friendly_preview_text(text);
										let is_group = jid.ends_with("@g.us");
										// `from` can carry a WhatsApp device suffix (e.g.
										// "1234:12@s.whatsapp.net") which will not match the
										// contacts map keyed by the plain JID - strip it, same
										// as the in-window sender resolution above.
										let clean_from = match from.find(':') {
											Some(idx) => format!("{}@s.whatsapp.net", &from[..idx]),
											None => from.to_string(),
										};
										let provided_sender = data
											.get("senderName")
											.and_then(|n| n.as_str())
											.filter(|n| !n.is_empty())
											.map(|n| n.to_string());
										let provided_chat_name = data
											.get("chatName")
											.and_then(|n| n.as_str())
											.filter(|n| !n.is_empty())
											.map(|n| n.to_string());
										if is_group {
											let mut chat_name = provided_chat_name
												.or_else(|| s.contacts.get(jid).cloned())
												.unwrap_or_else(|| jid.to_string());
											if chat_name == *jid {
												chat_name = "New Group".to_string();
											}
											let sender_name = provided_sender
												.or_else(|| s.contacts.get(&clean_from).cloned())
												.or_else(|| s.contacts.get(from).cloned())
												.unwrap_or_else(|| display_name_for_jid(&clean_from));
											let toast_body = if sender_name.is_empty() {
												body.to_string()
											} else {
												format!("{}: {}", sender_name, body)
											};
											if let Some(n) = show_toast_notification(&chat_name, &toast_body) {
												crate::chat_ui::record_notification(&mut s, jid, n);
											}
										} else {
											let sender_name = provided_sender
												.or_else(|| s.contacts.get(&clean_from).cloned())
												.or_else(|| s.contacts.get(from).cloned())
												.unwrap_or_else(|| display_name_for_jid(&clean_from));
											if let Some(n) = show_toast_notification(&sender_name, &body) {
												crate::chat_ui::record_notification(&mut s, jid, n);
											}
										}
									}
								}
							}
						}
					}
				}
				"message_deleted" => {
					if let Some(data) = val.get("data") {
						let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
						let message_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("");
						let for_everyone = data.get("for_everyone").and_then(|f| f.as_bool()).unwrap_or(true);

						let mut s = state.borrow_mut();
						// A bulk delete (selection mode) fires one "message_deleted"
						// event per message; announcing each individually would spam
						// a screen reader with "Message deleted" N times back to
						// back, on top of the one summary announcement
						// DeleteSelectedMessages already made - suppress the
						// per-message announce while draining that batch's set.
						// Keyed by (jid, message_id) so an unrelated delete arriving
						// mid-batch (e.g. synced from another linked device) isn't
						// mistaken for part of this batch.
						let suppress_announce =
							s.bulk_delete_pending.remove(&(jid.to_string(), message_id.to_string()));
						if Some(jid.to_string()) == s.selected_chat {
							let is_group = jid.ends_with("@g.us");

							if !for_everyone {
								// "Delete for me": the row just disappears, like
								// official WhatsApp - no "This message was
								// deleted" tombstone, that's only for revokes.
								let had_message = s.messages.iter().any(|m| m.id == message_id);
								if had_message {
									s.messages.retain(|m| m.id != message_id);
									s.display_items = whantv::group_messages_into_display_items(&s.messages);
									s.prev_msg_count = s.display_items.len();

									// One call for every row, not one append each (see
									// `apply_conversation_messages`).
									let rows: Vec<String> =
										s.display_items.iter().map(|item| item.format_display(is_group)).collect();
									parts.conversations_parts.messages_list.freeze();
									parts
										.conversations_parts
										.messages_list
										.set_items(&rows.iter().map(String::as_str).collect::<Vec<_>>());
									parts.conversations_parts.messages_list.thaw();
									refresh_selection_list(&mut s, parts);
									let count = parts.conversations_parts.messages_list.get_count();
									if count > 0 {
										parts.conversations_parts.messages_list.set_selection(count - 1, true);
									}
									refresh_voice_action_button(&s, &parts.conversations_parts);
									if !suppress_announce {
										crate::announce::announce(
											parts.conversations_parts.messages_announcer.clone(),
											"Message deleted.",
										);
									}
								}
								return;
							}

							if let Some(di) = update_loaded_message(&mut s, message_id, |m| {
								m.is_deleted = true;
								m.text = String::new();
							}) {
								if let Some(item) = s.display_items.get(di) {
									parts
										.conversations_parts
										.messages_list
										.set_string(di as u32, &item.format_display(is_group));

									if let Some(sel_idx) = parts.conversations_parts.messages_list.get_selection() {
										if sel_idx as u32 == di as u32 {
											let mut need_layout =
												crate::update_action_buttons(&s, &parts.conversations_parts);
											if parts.conversations_parts.play_voice_btn.is_shown() {
												parts.conversations_parts.play_voice_btn.show(false);
												need_layout = true;
											}
											if parts.conversations_parts.audio_slider.is_shown() {
												parts.conversations_parts.audio_slider.show(false);
												parts.conversations_parts.audio_progress_label.show(false);
												parts.conversations_parts.audio_speed_btn.show(false);
												need_layout = true;
											}
											if need_layout {
												parts.conversations_parts.action_panel.layout();
												parts.conversations_parts.conversation_panel.layout();
											}
										}
									}
								}
								if !suppress_announce {
									crate::announce::announce(
										parts.conversations_parts.messages_announcer.clone(),
										"Message deleted.",
									);
								}
							}
						}
					}
				}
				"new_contact_status" => {
					if let Some(data) = val.get("data") {
						if let Some(status) = data.get("status").and_then(|s| s.as_str()) {
							if status == "success" {
								let payload = serde_json::json!({
									"action": "get_chats"
								});
								let _ = state.borrow().ws_tx.send(payload.to_string());
							} else if status == "error" {
								let err_msg = data.get("error").and_then(|e| e.as_str()).unwrap_or("Unknown error");
								unsafe {
									let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
										windows::Win32::UI::WindowsAndMessaging::MB_OK,
									);
								}
								MessageDialog::builder(&parts.new_contact_dialog.dialog, err_msg, "Error")
									.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
									.build()
									.show_modal();
							}
						}
					}
				}
				"contacts" => {
					if let Ok(rows) = serde_json::from_value::<Vec<serde_json::Value>>(
						val.get("data").cloned().unwrap_or(serde_json::Value::Null),
					) {
						let mut contacts: Vec<Contact> = rows
							.iter()
							.filter_map(|row| {
								let jid = row.get("jid").and_then(|j| j.as_str())?.to_string();
								if jid.is_empty() {
									return None;
								}
								let name = row.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
								let phone = row.get("phone").and_then(|p| p.as_str()).unwrap_or("").to_string();
								Some(Contact { jid, name, phone })
							})
							.collect();
						contacts.sort_by(|a, b| {
							let ka = if a.name.is_empty() { &a.phone } else { &a.name };
							let kb = if b.name.is_empty() { &b.phone } else { &b.name };
							ka.to_lowercase().cmp(&kb.to_lowercase())
						});
						let mut s = state.borrow_mut();
						s.all_contacts = contacts;
						// Refresh the pickers live if they're already open and the
						// backend response landed after the dialog was shown
						// with a possibly-empty or stale contact list.
						let new_conv_shown = parts.new_conv_dialog.dialog.is_shown();
						let new_group_shown = parts.new_group_dialog.dialog.is_shown();
						let add_members_shown = parts.add_members_dialog.dialog.is_shown();
						let attach_contact_shown = parts.attach_contact_dialog.dialog.is_shown();
						drop(s);
						if new_conv_shown {
							let query = parts.new_conv_dialog.search_field.get_value();
							apply_contact_picker_filter(&query, state, parts);
						}
						// Route a fresh `contacts` response through the same debounce
						// timer as keystrokes (rather than rebuilding the checklist
						// widget here immediately) - back-to-back rebuilds (this one
						// landing right after the dialog-open rebuild) race NVDA's
						// async accessibility queries and trip wx's "bad
						// wxCheckListBox index" assert. See the timer's own comment
						// in window.rs's bind_input_handlers.
						if new_group_shown {
							sync_new_group_contact_labels(state);
							parts.new_group_contacts_timer.stop();
							parts.new_group_contacts_timer.start(250, true);
						}
						if add_members_shown {
							sync_add_members_contact_labels(state);
							parts.add_members_contacts_timer.stop();
							parts.add_members_contacts_timer.start(250, true);
						}
						if attach_contact_shown {
							sync_attach_contact_labels(state);
							parts.attach_contact_timer.stop();
							parts.attach_contact_timer.start(250, true);
						}
					}
				}
				"create_group_status" | "new_group_status" => {
					if let Some(data) = val.get("data") {
						if let Some(status) = data.get("status").and_then(|s| s.as_str()) {
							if status == "success" {
								let payload = serde_json::json!({
									"action": "get_chats"
								});
								let _ = state.borrow().ws_tx.send(payload.to_string());
							} else if status == "error" {
								parts.new_group_dialog.ok_btn.enable(true);
								let err_msg = data.get("error").and_then(|e| e.as_str()).unwrap_or("Unknown error");
								unsafe {
									let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
										windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_STYLE(0x00000030),
									);
								}
								MessageDialog::builder(&parts.new_group_dialog.dialog, err_msg, "Error")
									.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
									.build()
									.show_modal();
							}
						}
					}
				}
				"no_more_history" => {
					// The backend refused to ask the phone again for history
					// before a message it already asked about - the phone had
					// nothing older then.
					let mut s = state.borrow_mut();
					s.history_loading = false;
					if let Some(jid) = val.get("data").and_then(|d| d.get("jid")).and_then(|j| j.as_str()) {
						if !jid.is_empty() {
							eprintln!("[LOG UI] End of history reached on phone for chat: {}", jid);
							s.completed_history_chats.insert(jid.to_string());
							if s.phone_history_pending_jid.as_deref() == Some(jid) {
								s.phone_history_pending_jid = None;
								s.phone_history_deadline = None;
							}
						}
					}
				}
				"chat_cleared" => {
					// Covers a clear-chat that happened on the phone or
					// another linked device - whantv's own clear_chat already
					// clears the open conversation view locally before this
					// broadcast ever comes back, so this is a no-op for that
					// case and the only path that matters for the other one.
					if let Some(jid) = val.get("data").and_then(|d| d.get("jid")).and_then(|j| j.as_str()) {
						let mut s = state.borrow_mut();
						if s.selected_chat.as_deref() == Some(jid) && !s.messages.is_empty() {
							s.messages.clear();
							s.display_items.clear();
							s.prev_msg_count = 0;
							drop(s);
							parts.conversations_parts.messages_list.clear();
							update_action_buttons(&state.borrow(), &parts.conversations_parts);
							crate::announce::announce(
								parts.conversations_parts.messages_announcer.clone(),
								"Chat cleared.",
							);
						}
					}
				}
				"play_audio_failed" => {
					eprintln!("[LOG UI] Voice message audio failed to load or download");
					crate::announce::announce(
						parts.conversations_parts.messages_announcer.clone(),
						"Could not load voice message",
					);
				}
				"message_info" => {
					if let Some(data) = val.get("data") {
						let fmt_ts = |ts: i64| -> String {
							if ts == 0 {
								return String::new();
							}
							let dt = chrono::DateTime::from_timestamp(ts, 0)
								.unwrap_or_default()
								.with_timezone(&chrono::Local);
							dt.format("%l:%M %p").to_string().trim().to_lowercase()
						};
						let msg_id = data.get("id").and_then(|v| v.as_str()).unwrap_or("");
						let mut sent_s = data.get("sent_at").and_then(|v| v.as_i64()).unwrap_or(0);
						let mut delivered_s = data.get("delivered_at").and_then(|v| v.as_i64()).unwrap_or(0);
						let mut read_s = data.get("read_at").and_then(|v| v.as_i64()).unwrap_or(0);
						let mut played_s = data.get("played_at").and_then(|v| v.as_i64()).unwrap_or(0);
						let mut edited_s = data.get("edited_at").and_then(|v| v.as_i64()).unwrap_or(0);
						let media_type = data.get("media_type").and_then(|v| v.as_str()).unwrap_or("");
						let is_voice = media_type == "voice" || media_type == "audio";

						{
							let s = state.borrow();
							if let Some(msg) = s.messages.iter().find(|m| m.id == msg_id) {
								if sent_s == 0 {
									sent_s = if msg.sent_at > 0 { msg.sent_at } else { msg.timestamp };
								}
								if delivered_s == 0 && msg.delivered_at > 0 {
									delivered_s = msg.delivered_at;
								}
								if read_s == 0 && msg.read_at > 0 {
									read_s = msg.read_at;
								}
								if played_s == 0 && msg.played_at > 0 {
									played_s = msg.played_at;
								}
								if edited_s == 0 && msg.edited_at > 0 {
									edited_s = msg.edited_at;
								}
							}
						}

						let mut lines = Vec::new();
						let sent_fmt = fmt_ts(sent_s);
						if !sent_fmt.is_empty() {
							lines.push(format!("Sent at {}", sent_fmt));
						}
						let del_fmt = fmt_ts(delivered_s);
						if !del_fmt.is_empty() {
							lines.push(format!("Delivered at {}", del_fmt));
						}
						let read_fmt = fmt_ts(read_s);
						if !read_fmt.is_empty() {
							lines.push(format!("Read at {}", read_fmt));
						}
						if is_voice {
							let played_fmt = fmt_ts(played_s);
							if !played_fmt.is_empty() {
								lines.push(format!("Played at {}", played_fmt));
							}
						}
						let edited_fmt = fmt_ts(edited_s);
						if !edited_fmt.is_empty() {
							lines.push(format!("Edited at {}", edited_fmt));
						}

						if let Some(participants) = data.get("participants").and_then(|p| p.as_array()) {
							if !participants.is_empty() {
								lines.retain(|l| l.starts_with("Sent at") || l.starts_with("Edited at"));

								let mut played = Vec::new();
								let mut read = Vec::new();
								let mut delivered = Vec::new();
								let mut sent = Vec::new();
								for p in participants {
									let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
									let pstatus = p.get("status").and_then(|v| v.as_str()).unwrap_or("sent");
									let pts = p.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0);
									let p_at = |key: &str| p.get(key).and_then(|v| v.as_i64()).unwrap_or(0);
									// Every state this member has reached, latest first, e.g.
									// "Asha — read 8:13 pm, delivered 8:07 pm".
									let mut times = Vec::new();
									if is_voice {
										let played = fmt_ts(p_at("played_at"));
										if !played.is_empty() {
											times.push(format!("played {}", played));
										}
									}
									let read_at = fmt_ts(p_at("read_at"));
									if !read_at.is_empty() {
										times.push(format!("read {}", read_at));
									}
									let delivered_at = fmt_ts(p_at("delivered_at"));
									if !delivered_at.is_empty() {
										times.push(format!("delivered {}", delivered_at));
									}
									if times.is_empty() {
										let ts_fmt = fmt_ts(pts);
										if !ts_fmt.is_empty() {
											times.push(ts_fmt);
										}
									}
									let entry = if times.is_empty() {
										name.to_string()
									} else {
										format!("{} — {}", name, times.join(", "))
									};
									match pstatus {
										"played" => played.push(entry),
										"read" => read.push(entry),
										"delivered" => delivered.push(entry),
										_ => sent.push(entry),
									}
								}

								let mut push_group = |label: &str, entries: &Vec<String>| {
									if entries.is_empty() {
										return;
									}
									lines.push(String::new());
									lines.push(label.to_string());
									for e in entries {
										lines.push(format!("  {}", e));
									}
								};
								push_group("Played by", &played);
								push_group("Read by", &read);
								push_group("Delivered to", &delivered);
								push_group("Sent to", &sent);
							}
						}

						// A poll: its results, then who voted for what.
						if let Some(poll) = data.get("poll") {
							let from_me = state.borrow().messages.iter().any(|m| m.id == msg_id && m.from_me);
							if !from_me {
								// Receipts are only ours to know for what we sent.
								lines.clear();
							}
							let mut poll_lines = Vec::new();
							let question = poll.get("question").and_then(|v| v.as_str()).unwrap_or("");
							poll_lines.push(format!("Poll: {}", question));
							let results = poll.get("results").and_then(|v| v.as_str()).unwrap_or("");
							if !results.is_empty() {
								poll_lines.push(results.to_string());
							}
							let voters = poll.get("voters").and_then(|v| v.as_array()).cloned().unwrap_or_default();
							let is_channel = poll.get("channel").and_then(|v| v.as_bool()).unwrap_or(false);
							poll_lines.push(String::new());
							if voters.is_empty() {
								poll_lines.push("No votes yet.".to_string());
							} else {
								poll_lines.push("Votes".to_string());
								for v in &voters {
									let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("");
									let options: Vec<&str> = v
										.get("options")
										.and_then(|x| x.as_array())
										.map(|a| a.iter().filter_map(|o| o.as_str()).collect())
										.unwrap_or_default();
									let when = fmt_ts(v.get("timestamp").and_then(|x| x.as_i64()).unwrap_or(0));
									let entry = format!("  {} voted {}", name, options.join(" and "));
									poll_lines.push(if when.is_empty() {
										entry
									} else {
										format!("{}, {}", entry, when)
									});
								}
							}
							if is_channel {
								poll_lines.push(
									"Channels show vote totals, not who voted. Admins can see voters from the Channel post menu.".to_string(),
								);
							}
							if !lines.is_empty() {
								poll_lines.push(String::new());
							}
							poll_lines.append(&mut lines);
							lines = poll_lines;
						}

						if lines.is_empty() {
							lines.push("No receipt information available.".to_string());
						}
						let info_text = lines.join("\n");

						let dlg = Dialog::builder(&parts.frame, "Message Info")
							.with_style(DialogStyle::DefaultDialogStyle)
							.build();
						let sizer = BoxSizer::builder(Orientation::Vertical).build();
						let txt = TextCtrl::builder(&dlg)
							.with_value(&info_text)
							.with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly | TextCtrlStyle::DontWrap)
							.build();
						sizer.add(&txt, 1, SizerFlag::Expand | SizerFlag::All, 10);
						let ok_btn = Button::builder(&dlg).with_label("OK").build();
						sizer.add(&ok_btn, 0, SizerFlag::AlignCentre | SizerFlag::Bottom | SizerFlag::All, 8);
						dlg.set_sizer_and_fit(sizer, true);
						let dlg_close = dlg.clone();
						ok_btn.on_click(move |_| {
							dlg_close.end_modal(wxdragon::id::ID_OK);
						});
						dlg.show_modal();
						// Unlike MessageDialog/FileDialog/TextEntryDialog, the base
						// `Dialog` has no `Drop` (see wxdragon's dialogs/mod.rs), so
						// without this every Message Info leaves a hidden dialog -
						// plus its TextCtrl, Button and bound closure - parented to
						// the main frame for the rest of the session. Destroying the
						// original exactly once also tears down `dlg_close`'s closure
						// with the button that owns it.
						dlg.destroy();
					}
				}
				"history_updated" => {
					eprintln!("[LOG UI] History sync completed on phone! Refreshing messages for active chat...");
					let s = state.borrow();
					if let Some(ref jid) = s.selected_chat.clone() {
						let limit = 9999999;
						let payload = serde_json::json!({
							"action": "get_messages",
							"data": { "jid": jid, "limit": limit, "offset": 0 }
						});
						let _ = s.ws_tx.send(payload.to_string());
					}
				}
				"export_progress" => {
					if let Some(data) = val.get("data") {
						let current = data.get("current").and_then(|c| c.as_i64()).unwrap_or(0);
						let total = data.get("total").and_then(|t| t.as_i64()).unwrap_or(0);
						parts.frame.set_status_text(&format!("Status: Exporting chat... {}/{}", current, total), 0);
					}
				}
				"chat_exported" => {
					if let Some(data) = val.get("data") {
						let path = data.get("path").and_then(|p| p.as_str()).unwrap_or("");
						parts.frame.set_status_text("Status: Connected", 0);
						if data.get("text_only").and_then(|t| t.as_bool()).unwrap_or(false) {
							let count = data.get("message_count").and_then(|m| m.as_i64()).unwrap_or(0);
							let msg = format!("Chat text exported to {}. {} messages.", path, count);
							crate::announce::announce(parts.conversations_parts.messages_announcer.clone(), &msg);
							return;
						}
						let media_exported = data.get("media_exported").and_then(|m| m.as_i64()).unwrap_or(0);
						let media_skipped = data.get("media_skipped").and_then(|m| m.as_i64()).unwrap_or(0);
						let msg = if media_skipped > 0 {
							format!(
								"Chat exported to {}. {} media files included, {} could not be downloaded and were skipped.",
								path, media_exported, media_skipped
							)
						} else {
							format!("Chat exported to {}. {} media files included.", path, media_exported)
						};
						crate::announce::announce(parts.conversations_parts.messages_announcer.clone(), &msg);
					}
				}
				"statuses" => {
					if let Some(data) = val.get("data") {
						if let Some(groups) = data.get("groups").and_then(|g| g.as_array()) {
							let mut parsed = Vec::new();
							for g in groups {
								let poster_jid = g.get("posterJid").and_then(|j| j.as_str()).unwrap_or("").to_string();
								let poster_name =
									g.get("posterName").and_then(|n| n.as_str()).unwrap_or("").to_string();
								let all_viewed = g.get("allViewed").and_then(|v| v.as_bool()).unwrap_or(false);
								let mut items = Vec::new();
								if let Some(statuses) = g.get("statuses").and_then(|s| s.as_array()) {
									for it in statuses {
										items.push(StatusItem {
											id: it.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
											text: it.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string(),
											media_type: it
												.get("mediaType")
												.and_then(|v| v.as_str())
												.unwrap_or("")
												.to_string(),
											timestamp: it.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0),
											viewed: it.get("viewed").and_then(|v| v.as_bool()).unwrap_or(false),
										});
									}
								}
								parsed.push(StatusGroup { poster_jid, poster_name, all_viewed, items });
							}
							let mut s = state.borrow_mut();
							s.status_groups = parsed;
							if s.active_tab == 1 {
								apply_chat_filter(&mut s, parts);
							}
						}
					}
				}
				"status_update" => {
					if let Some(data) = val.get("data") {
						let id = data.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
						let poster_jid = data.get("posterJid").and_then(|v| v.as_str()).unwrap_or("").to_string();
						if id.is_empty() || poster_jid.is_empty() {
							return;
						}
						let from_me = data.get("fromMe").and_then(|v| v.as_bool()).unwrap_or(false);
						let text = data.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
						let media_type = data.get("mediaType").and_then(|v| v.as_str()).unwrap_or("").to_string();
						let timestamp = data.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0);

						let mut s = state.borrow_mut();
						let poster_name = s
							.contacts
							.get(&poster_jid)
							.cloned()
							.unwrap_or_else(|| if from_me { "You".to_string() } else { poster_jid.clone() });
						let item = StatusItem { id, text, media_type, timestamp, viewed: from_me };
						if let Some(group) = s.status_groups.iter_mut().find(|g| g.poster_jid == poster_jid) {
							group.items.push(item);
							group.all_viewed = group.items.iter().all(|it| it.viewed);
						} else {
							s.status_groups.push(StatusGroup {
								poster_jid,
								poster_name,
								all_viewed: from_me,
								items: vec![item],
							});
						}
						if s.active_tab == 1 {
							apply_chat_filter(&mut s, parts);
						}
					}
				}
				"starred_messages" => {
					if let Some(data) = val.get("data") {
						if let Some(results) = data.get("results").and_then(|r| r.as_array()) {
							let mut s = state.borrow_mut();
							let mut entries = Vec::new();
							for item in results {
								let jid = item.get("jid").and_then(|j| j.as_str()).unwrap_or("").to_string();
								let message_id = item.get("id").and_then(|m| m.as_str()).unwrap_or("").to_string();
								if jid.is_empty() || message_id.is_empty() {
									continue;
								}
								let from_me = item.get("fromMe").and_then(|f| f.as_bool()).unwrap_or(false);
								let sender_name = item.get("senderName").and_then(|n| n.as_str()).unwrap_or("");
								let text = item.get("text").and_then(|t| t.as_str()).unwrap_or("");
								let quoted_text = item.get("quotedText").and_then(|t| t.as_str()).unwrap_or("");
								let media_type = item.get("mediaType").and_then(|t| t.as_str()).unwrap_or("");
								let timestamp = item.get("timestamp").and_then(|t| t.as_i64()).unwrap_or(0);

								let chat_name = s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
								let sender = if from_me {
									"You".to_string()
								} else if !sender_name.is_empty() {
									sender_name.to_string()
								} else {
									chat_name.clone()
								};
								let body = if !text.is_empty() {
									whantv::friendly_preview_text(text)
								} else if !quoted_text.is_empty() {
									whantv::friendly_preview_text(quoted_text)
								} else if !media_type.is_empty() {
									"Media".to_string()
								} else {
									"Message".to_string()
								};
								let date_str = if timestamp > 0 {
									chrono::DateTime::from_timestamp(timestamp, 0)
										.map(|dt| {
											let local_dt: chrono::DateTime<chrono::Local> = dt.into();
											local_dt.format("%b %e, %Y").to_string()
										})
										.unwrap_or_default()
								} else {
									String::new()
								};
								let label = if date_str.is_empty() {
									format!("{} ({}): {}", chat_name, sender, body)
								} else {
									format!("{} ({}): {}, {}", chat_name, sender, body, date_str)
								};

								entries.push(StarredEntry { jid, message_id, label });
							}
							s.starred_entries = entries;
							if s.active_tab == 2 {
								apply_chat_filter(&mut s, parts);
							}
						}
					}
				}
				"search_results" => {
					if let Some(data) = val.get("data") {
						let response_jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
						// The chat may have been switched again while this
						// search was in flight - applying a stale response
						// would silently populate the wrong chat's messages.
						if state.borrow().selected_chat.as_deref() != Some(response_jid) {
							return;
						}
						if let Some(results) = data.get("results").and_then(|r| r.as_array()) {
							let mut parsed_msgs: Vec<ChatMessage> = Vec::new();
							for msg_val in results {
								if let Ok(msg) = serde_json::from_value::<ChatMessage>(msg_val.clone()) {
									parsed_msgs.push(msg);
								}
							}

							let mut with_dates = Vec::new();
							let mut last_date: Option<chrono::NaiveDate> = None;
							for msg in parsed_msgs {
								if let Some(datetime) = chrono::DateTime::from_timestamp(msg.timestamp, 0) {
									let local_datetime: chrono::DateTime<chrono::Local> = datetime.into();
									let day = whantv::date_key(local_datetime);
									if last_date != Some(day) {
										last_date = Some(day);
										with_dates.push(whantv::Msg::new(ChatMessage {
											text: whantv::format_date_divider_label(local_datetime),
											timestamp: msg.timestamp,
											is_date_divider: true,
											..Default::default()
										}));
									}
								}
								with_dates.push(whantv::Msg::new(msg));
							}

							let mut s = state.borrow_mut();
							let is_group = s.selected_chat.as_ref().map(|j| j.ends_with("@g.us")).unwrap_or(false);
							s.messages = with_dates;
							s.display_items = whantv::group_messages_into_display_items(&s.messages);

							parts.conversations_parts.messages_list.freeze();
							parts.conversations_parts.messages_list.clear();
							for item in &s.display_items {
								parts.conversations_parts.messages_list.append(&item.format_display(is_group));
							}
							parts.conversations_parts.messages_list.thaw();
							refresh_selection_list(&mut s, parts);
							let count = parts.conversations_parts.messages_list.get_count();
							if count > 0 {
								parts.conversations_parts.messages_list.set_selection(0, true);
							}
							refresh_voice_action_button(&s, &parts.conversations_parts);
						}
					}
				}
				"typing" => {
					if let Some(data) = val.get("data") {
						let chat_jid =
							data.get("chat_jid").or_else(|| data.get("jid")).and_then(|j| j.as_str()).unwrap_or("");
						let mut sender_jid =
							data.get("sender_jid").or_else(|| data.get("jid")).and_then(|j| j.as_str()).unwrap_or("");
						if sender_jid.is_empty() {
							sender_jid = chat_jid;
						}
						if chat_jid.is_empty() || sender_jid.is_empty() {
							return;
						}
						let is_typing = data.get("typing").and_then(|t| t.as_bool()).unwrap_or(false);
						let is_recording = data.get("recording").and_then(|r| r.as_bool()).unwrap_or(false);

						let state_changed = {
							let mut s = state.borrow_mut();
							if is_typing || is_recording {
								s.typing_timestamps.insert(chat_jid.to_string(), std::time::Instant::now());
							} else {
								s.typing_timestamps.remove(chat_jid);
							}
							let new_kind: u8 = if is_recording {
								2
							} else if is_typing {
								1
							} else {
								0
							};
							let prev_kind = s.typing_announce_state.get(chat_jid).copied().unwrap_or(0);
							if new_kind == 0 {
								s.typing_announce_state.remove(chat_jid);
							} else {
								s.typing_announce_state.insert(chat_jid.to_string(), new_kind);
							}
							new_kind != prev_kind
						};

						let s = state.borrow();
						if Some(chat_jid.to_string()) == s.selected_chat {
							let sender_name =
								s.contacts.get(sender_jid).cloned().unwrap_or_else(|| display_name_for_jid(sender_jid));
							if !sender_name.is_empty() {
								let announcement = if is_recording && s.settings.announce_recording {
									Some(format!("{} is recording audio...", sender_name))
								} else if is_typing && s.settings.announce_typing {
									Some(format!("{} is typing...", sender_name))
								} else {
									None
								};
								if let Some(ann) = announcement {
									if state_changed {
										crate::announce::announce(
											parts.conversations_parts.messages_announcer.clone(),
											&ann,
										);
									}
								}
								let norm_chat_jid = if let Some((user, server)) = chat_jid.split_once('@') {
									let base = user.split('.').next().unwrap_or(user).split(':').next().unwrap_or(user);
									format!("{}@{}", base, server)
								} else {
									chat_jid.to_string()
								};
								let chat_name = s
									.contacts
									.get(chat_jid)
									.or_else(|| s.contacts.get(&norm_chat_jid))
									.cloned()
									.unwrap_or_else(|| display_name_for_jid(chat_jid));
								let presence_label = s
									.presence_labels
									.get(chat_jid)
									.or_else(|| s.presence_labels.get(&norm_chat_jid))
									.map(|l| l.as_str());
								let btn_label = format_conv_data_label(
									chat_jid,
									&chat_name,
									&sender_name,
									is_typing,
									is_recording,
									presence_label,
								);
								parts.conversations_parts.conv_data_btn.set_label(&btn_label);
							}
						}
					}
				}
				"presence" => {
					if let Some(data) = val.get("data") {
						let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
						if jid.is_empty() || jid.ends_with("@g.us") {
							return;
						}
						let clean_jid = if let Some((user, server)) = jid.split_once('@') {
							let base_user = user.split('.').next().unwrap_or(user).split(':').next().unwrap_or(user);
							format!("{}@{}", base_user, server)
						} else {
							jid.to_string()
						};
						let unavailable = data.get("unavailable").and_then(|u| u.as_bool()).unwrap_or(false);
						let last_seen = data.get("last_seen").and_then(|l| l.as_i64()).unwrap_or(0);
						let mut s = state.borrow_mut();

						let name = s
							.contacts
							.get(&clean_jid)
							.or_else(|| s.contacts.get(jid))
							.cloned()
							.unwrap_or_else(|| display_name_for_jid(&clean_jid));

						let btn_label = if unavailable {
							if last_seen > 0 {
								if let Some(datetime) = chrono::DateTime::from_timestamp(last_seen, 0) {
									let local_dt: chrono::DateTime<chrono::Local> = datetime.into();
									let now = chrono::Local::now();
									let time_str = local_dt.format("%l:%M %p").to_string().trim().to_string();
									let date_str = if local_dt.date_naive() == now.date_naive() {
										format!("today at {}", time_str)
									} else {
										local_dt.format("%b %e at %l:%M %p").to_string().replace("  ", " ")
									};
									format!("{} (last seen {})", name, date_str)
								} else {
									name.clone()
								}
							} else {
								name.clone()
							}
						} else {
							format!("{} (online)", name)
						};

						s.presence_labels.insert(clean_jid.clone(), btn_label.clone());
						s.presence_labels.insert(jid.to_string(), btn_label.clone());
						let was_online = s.contact_online.insert(clean_jid.clone(), !unavailable);
						if !unavailable && was_online != Some(true) {
							crate::ui_commands::messaging::contact_came_online(&mut s, &clean_jid);
						}

						let is_currently_typing = s
							.typing_timestamps
							.get(&clean_jid)
							.or_else(|| s.typing_timestamps.get(jid))
							.or_else(|| s.selected_chat.as_ref().and_then(|sel| s.typing_timestamps.get(sel)))
							.map_or(false, |t| t.elapsed().as_secs() < 5);

						if unavailable {
							s.typing_timestamps.remove(&clean_jid);
							s.typing_timestamps.remove(jid);
							s.typing_announce_state.remove(&clean_jid);
							s.typing_announce_state.remove(jid);
						}

						if Some(clean_jid.clone()) == s.selected_chat || Some(jid.to_string()) == s.selected_chat {
							let mut should_announce = true;
							if !s.initial_presence_received {
								s.initial_presence_received = true;
								should_announce = false;
							}

							let announcement = if unavailable {
								if last_seen > 0 {
									if let Some(datetime) = chrono::DateTime::from_timestamp(last_seen, 0) {
										let local_dt: chrono::DateTime<chrono::Local> = datetime.into();
										let now = chrono::Local::now();
										let time_str = local_dt.format("%l:%M %p").to_string().trim().to_string();
										let date_str = if local_dt.date_naive() == now.date_naive() {
											format!("today at {}", time_str)
										} else {
											local_dt.format("%b %e at %l:%M %p").to_string().replace("  ", " ")
										};
										format!("{}, last seen {}", name, date_str)
									} else {
										format!("{}, offline", name)
									}
								} else {
									format!("{}, offline", name)
								}
							} else {
								format!("{} is now online", name)
							};
							if should_announce {
								crate::announce::announce(
									parts.conversations_parts.messages_announcer.clone(),
									&announcement,
								);
							}
							if unavailable || !is_currently_typing {
								parts.conversations_parts.conv_data_btn.set_label(&btn_label);
							}
						}
					}
				}
				"mention_candidates" => {
					if let Some(data) = val.get("data") {
						crate::ui_commands::mentions::on_candidates(state, &parts.conversations_parts, data);
					}
				}
				"chat_limit_sharing" => {
					if let Some(data) = val.get("data") {
						let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or_default();
						let enabled = data.get("enabled").and_then(|e| e.as_bool()).unwrap_or(false);
						crate::ui_commands::mentions::on_limit_sharing(state, &parts.conversations_parts, jid, enabled);
					}
				}
				"group_info" => {
					if let Some(payload) = val.get("data") {
						let info = payload.get("Info").unwrap_or(payload);
						let viewer_is_admin = payload.get("ViewerIsAdmin").and_then(|v| v.as_bool()).unwrap_or(false);

						if let Some(group_jid) = info.get("JID").and_then(|j| j.get("User")).and_then(|u| u.as_str()) {
							let full_jid = format!("{}@g.us", group_jid);
							let mut s = state.borrow_mut();
							if viewer_is_admin {
								s.admin_group_jids.insert(full_jid);
							} else {
								s.admin_group_jids.remove(&full_jid);
							}
						}

						// Set visibility of admin buttons
						parts.group_data_dialog.add_member_btn.show(viewer_is_admin);
						parts.group_data_dialog.make_admin_btn.show(viewer_is_admin);
						parts.group_data_dialog.remove_member_btn.show(viewer_is_admin);
						// Only admins can see or reset a group's invite link.
						parts.group_data_dialog.invite_link_btn.show(viewer_is_admin);
						parts.group_data_dialog.save_settings_btn.show(viewer_is_admin);
						parts.group_data_dialog.announce_chk.enable(viewer_is_admin);
						parts.group_data_dialog.locked_chk.enable(viewer_is_admin);
						parts.group_data_dialog.forwarded_chk.enable(viewer_is_admin);
						parts.group_data_dialog.admin_reports_chk.enable(viewer_is_admin);
						parts.group_data_dialog.history_chk.enable(viewer_is_admin);
						parts.group_data_dialog.reported_btn.show(viewer_is_admin);
						let flag = |k: &str| info.get(k).and_then(|t| t.as_bool()).unwrap_or(false);
						parts.group_data_dialog.forwarded_chk.set_value(flag("NoFrequentlyForwarded"));
						parts.group_data_dialog.admin_reports_chk.set_value(flag("AllowAdminReports"));
						parts.group_data_dialog.history_chk.set_value(flag("HasGroupHistory"));

						// Only overwrite the tag field when it isn't being edited.
						let my_tag = payload.get("MyLabel").and_then(|t| t.as_str()).unwrap_or("");
						if !parts.group_data_dialog.my_tag_field.has_focus() {
							parts.group_data_dialog.my_tag_field.set_value(my_tag);
						}

						// Set basic settings
						if let Some(topic) = info.get("Topic").and_then(|t| t.as_str()) {
							parts.group_data_dialog.group_desc_field.set_value(topic);
						}
						if let Some(is_announce) = info.get("IsAnnounce").and_then(|t| t.as_bool()) {
							parts.group_data_dialog.announce_chk.set_value(is_announce);
						}
						if let Some(is_locked) = info.get("IsLocked").and_then(|t| t.as_bool()) {
							parts.group_data_dialog.locked_chk.set_value(is_locked);
						}
						// A community lists its groups; a group inside one lists
						// its community's groups, so the tab works from either.
						let is_community = info.get("IsParent").and_then(|t| t.as_bool()).unwrap_or(false);
						let in_community =
							info.get("LinkedParentJID").and_then(|t| t.as_str()).is_some_and(|p| !p.is_empty());
						parts.group_data_dialog.groups_list.clear();
						if is_community || in_community {
							let s = state.borrow();
							if let Some(ref jid) = s.selected_chat {
								let payload = serde_json::json!({
									"action": "get_community_subgroups",
									"data": { "jid": jid }
								});
								let _ = s.ws_tx.send(payload.to_string());
							}
						}

						// Parse members
						let s = state.borrow();
						let mut member_strings = Vec::new();
						let mut member_jids = Vec::new();
						let mut member_admin = Vec::new();
						if let Some(participants) = payload.get("Participants").and_then(|p| p.as_array()) {
							for p in participants {
								if let Some(jid) = p.get("JID").and_then(|j| j.as_str()) {
									// PhoneJID is the participant's phone-number JID
									// (backend-resolved from a @lid address when the
									// group hides phone numbers), or empty for someone
									// who hides their number from us - never format the
									// raw action JID as a number, a LID isn't one.
									let phone_jid = p.get("PhoneJID").and_then(|j| j.as_str()).unwrap_or("");
									let is_admin = p.get("IsAdmin").and_then(|a| a.as_bool()).unwrap_or(false);
									let is_super = p.get("IsSuperAdmin").and_then(|a| a.as_bool()).unwrap_or(false);

									let backend_name = p.get("Name").and_then(|n| n.as_str()).unwrap_or("");
									let tag = p.get("Label").and_then(|n| n.as_str()).unwrap_or("").trim();

									let phone = if phone_jid.is_empty() {
										String::new()
									} else {
										let raw_phone = phone_jid.split('@').next().unwrap_or(phone_jid);
										match phonenumber::parse(None, format!("+{}", raw_phone)) {
											Ok(parsed) => {
												parsed.format().mode(phonenumber::Mode::International).to_string()
											}
											Err(_) => raw_phone.to_string(),
										}
									};

									let local_name = if !backend_name.is_empty() {
										backend_name.to_string()
									} else if let Some(contact) =
										s.contacts.get(phone_jid).filter(|_| !phone_jid.is_empty())
									{
										contact.clone()
									} else if !phone.is_empty() {
										phone.clone()
									} else {
										"Unknown member".to_string()
									};

									let mut role = "";
									if is_super {
										role = " (Super Admin)";
									} else if is_admin {
										role = " (Admin)";
									}

									// The member tag reads right after the name, as
									// WhatsApp shows it under the name.
									let tagged = if tag.is_empty() {
										local_name.clone()
									} else {
										format!("{}, tag: {}", local_name, tag)
									};
									let display = if phone.is_empty() || local_name == phone {
										format!("{}{}", tagged, role)
									} else {
										format!("{}{} - {}", tagged, role, phone)
									};
									member_strings.push(display);
									member_jids.push(jid.to_string());
									member_admin.push(is_admin || is_super);
								}
							}
						}
						drop(s);
						let mut s_mut = state.borrow_mut();
						s_mut.group_data_members = member_strings;
						s_mut.group_data_member_jids = member_jids;
						s_mut.group_data_member_admin = member_admin;

						// Populate Media/Documents/Links
						s_mut.group_data_media = build_media_messages(&s_mut.messages);
						s_mut.group_data_documents = build_document_messages(&s_mut.messages);
						s_mut.group_data_links = build_link_messages(&s_mut.messages);
						populate_message_list(&parts.group_data_dialog.media_list, &s_mut.group_data_media, |m| {
							m.display_filename()
						});
						populate_message_list(
							&parts.group_data_dialog.documents_list,
							&s_mut.group_data_documents,
							|m| m.display_filename(),
						);
						populate_message_list(&parts.group_data_dialog.links_list, &s_mut.group_data_links, |m| {
							m.first_link().unwrap_or_default()
						});

						let query_members = parts.group_data_dialog.members_search.get_value().to_lowercase();
						parts.group_data_dialog.members_list.clear();
						let mut visible_jids = Vec::new();
						for (i, member) in s_mut.group_data_members.iter().enumerate() {
							if query_members.is_empty() || member.to_lowercase().contains(&query_members) {
								parts.group_data_dialog.members_list.append(member);
								visible_jids.push(s_mut.group_data_member_jids[i].clone());
							}
						}
						s_mut.group_data_visible_member_jids = visible_jids;
						drop(s_mut);
						// The list was just rebuilt from scratch, so there's no
						// selection yet - reset the label to the no-selection default.
						parts.group_data_dialog.make_admin_btn.set_label("Make admin");
					}
				}
				"community_subgroups" => {
					if let Some(data) = val.get("data") {
						let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
						let s = state.borrow();
						let is_relevant =
							s.selected_chat.as_deref() == Some(jid) && parts.group_data_dialog.dialog.is_shown();
						drop(s);
						if is_relevant {
							let community = data.get("community").and_then(|c| c.as_str()).unwrap_or(jid).to_string();
							let community_name = data.get("communityName").and_then(|n| n.as_str()).unwrap_or("");
							parts.group_data_dialog.groups_label.set_label(&if community_name.is_empty() {
								"Groups in this community".to_string()
							} else {
								format!("Groups in {}", community_name)
							});
							let mut subgroup_jids = Vec::new();
							let mut subgroup_joined = Vec::new();
							let mut labels = Vec::new();
							if let Some(groups) = data.get("groups").and_then(|g| g.as_array()) {
								for g in groups {
									let flag = |key: &str| g.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
									let joined = flag("Joined");
									let label = whantv::community_group_label(
										g.get("Name").and_then(|n| n.as_str()).unwrap_or(""),
										flag("IsDefaultSubGroup"),
										flag("IsGeneralChat"),
										joined,
										g.get("ParticipantCount").and_then(|c| c.as_u64()).unwrap_or(0),
									);
									labels.push(label);
									subgroup_jids.push(g.get("JID").and_then(|j| j.as_str()).unwrap_or("").to_string());
									subgroup_joined.push(joined);
								}
							}
							// A refresh (after joining one) keeps the user on the same
							// group, wherever the new order puts it.
							let groups_list = &parts.group_data_dialog.groups_list;
							let previous_jid = groups_list
								.get_selection()
								.and_then(|i| state.borrow().group_data_subgroup_jids.get(i as usize).cloned());
							groups_list.set_items(&labels.iter().map(String::as_str).collect::<Vec<_>>());
							if let Some(idx) =
								previous_jid.and_then(|prev| subgroup_jids.iter().position(|j| *j == prev))
							{
								groups_list.set_selection(idx as u32, true);
							}
							let mut s = state.borrow_mut();
							s.group_data_subgroup_jids = subgroup_jids;
							s.group_data_subgroup_joined = subgroup_joined;
							s.group_data_community = Some(community);
							drop(s);
							crate::ui_commands::contacts_groups::community_subgroup_selection_changed(state, parts);
						}
					}
				}
				"member_label" => {
					if let Some(data) = val.get("data") {
						let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
						let s = state.borrow();
						if data.get("mine").and_then(|m| m.as_bool()).unwrap_or(false) {
							let label = data.get("label").and_then(|l| l.as_str()).unwrap_or("");
							let text = if label.is_empty() {
								"Your tag was removed.".to_string()
							} else {
								format!("Your tag is now {}.", label)
							};
							crate::announce::announce(parts.conversations_parts.messages_announcer.clone(), &text);
						}
						// Show the change if this group's info is open.
						if s.selected_chat.as_deref() == Some(jid) && parts.group_data_dialog.dialog.is_shown() {
							let payload = serde_json::json!({
								"action": "get_group_info",
								"data": { "jid": jid }
							});
							let _ = s.ws_tx.send(payload.to_string());
						}
					}
				}
				"group_updated" => {
					if let Some(jid) = val.get("data").and_then(|d| d.get("jid")).and_then(|j| j.as_str()) {
						let s = state.borrow();
						let is_relevant =
							s.selected_chat.as_deref() == Some(jid) && parts.group_data_dialog.dialog.is_shown();
						if is_relevant {
							let payload = serde_json::json!({
								"action": "get_group_info",
								"data": { "jid": jid }
							});
							let _ = s.ws_tx.send(payload.to_string());
						}
					}
				}
				"community_subgroup_joined" => {
					if let Some(jid) = val.get("data").and_then(|d| d.get("jid")).and_then(|j| j.as_str()) {
						// Refresh the Groups tab so the group now reads as joined
						// and Enter opens it instead of joining again.
						{
							let s = state.borrow();
							if parts.group_data_dialog.dialog.is_shown() {
								if let Some(ref selected) = s.selected_chat {
									let payload = serde_json::json!({
										"action": "get_community_subgroups",
										"data": { "jid": selected }
									});
									let _ = s.ws_tx.send(payload.to_string());
								}
							}
						}
						// The chat list itself already self-corrects via the
						// broadcastChats() the backend fires right before this -
						// but silence otherwise leaves a screen-reader user unable
						// to tell "joined" apart from "nothing happened yet" (the
						// failure paths for this same action all announce via the
						// "error" event; this was the one success path that didn't).
						let name = state.borrow().contacts.get(jid).cloned().unwrap_or_else(|| jid.to_string());
						crate::announce::announce(
							parts.conversations_parts.messages_announcer.clone(),
							&format!("Joined {}.", name),
						);
					}
				}
				"group_invite_info" => {
					if let Some(data) = val.get("data") {
						let message_id = data.get("messageId").and_then(|v| v.as_str()).unwrap_or("").to_string();
						// Set when the invite came from a link rather than an
						// invite message.
						let code = data.get("code").and_then(|v| v.as_str()).unwrap_or("").to_string();
						let name = data.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
						let topic = data.get("topic").and_then(|v| v.as_str()).unwrap_or("");
						let size = data.get("size").and_then(|v| v.as_i64()).unwrap_or(0);
						let display_name = if name.is_empty() { "This group".to_string() } else { name };
						let mut body = format!("{display_name}\n{size} member(s)");
						if !topic.is_empty() {
							body.push_str(&format!("\n\n{topic}"));
						}
						let dlg = MessageDialog::builder(&parts.frame, &body, "Group Invite")
							.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
							.build();
						if dlg.show_modal() == wxdragon::id::ID_YES {
							let command = if code.is_empty() {
								UiCommand::JoinGroupInvite(message_id)
							} else {
								UiCommand::JoinGroupLink(code)
							};
							let _ = state.borrow().ui_tx.send(command);
						}
					}
				}
				"channel_directory" => {
					let directory = state.borrow().channel_directory.clone();
					if let (Some(directory), Some(data)) = (directory, val.get("data")) {
						directory.apply(data);
					}
				}
				"channel_info" => {
					if let Some(data) = val.get("data") {
						crate::ui_commands::links::show_channel_info(state, parts, data);
					}
				}
				"channel_admin_invite" => {
					if let Some(data) = val.get("data") {
						crate::ui_commands::channel_admin::channel_admin_invite(state, parts, data);
					}
				}
				"channel_notice" => {
					if let Some(data) = val.get("data") {
						crate::ui_commands::channel_admin::channel_notice(parts, data);
					}
				}
				"server_flags" => {
					if let Some(data) = val.get("data") {
						state.borrow_mut().channel_pinning =
							data.get("channelPinning").and_then(|v| v.as_bool()).unwrap_or(false);
					}
				}
				"channel_people" => {
					if let Some(data) = val.get("data") {
						crate::ui_commands::channel_admin::channel_people(parts, data);
					}
				}
				"channel_pending_invites" => {
					if let Some(data) = val.get("data") {
						crate::ui_commands::channel_admin::channel_pending_invites(state, parts, data);
					}
				}
				"similar_channels" => {
					let directory = state.borrow().channel_directory.clone();
					if let (Some(directory), Some(data)) = (directory, val.get("data")) {
						directory.apply(data);
					}
				}
				"channel_created" | "channel_updated" | "channel_deleted" | "channel_followers" => {
					if let Some(data) = val.get("data") {
						use crate::ui_commands::channel_admin as admin;
						match t {
							"channel_created" => admin::channel_created(state, parts, data),
							"channel_updated" => admin::channel_updated(state, parts, data),
							"channel_deleted" => admin::channel_deleted(state, parts, data),
							_ => admin::channel_followers(state, parts, data),
						}
					}
				}
				"channel_messages_updated" => {
					// New posts or counts for a channel: reload it quietly if it's
					// the one open.
					let jid = val.get("data").and_then(|d| d.get("jid")).and_then(|j| j.as_str()).unwrap_or("");
					let mut s = state.borrow_mut();
					if s.selected_chat.as_deref() == Some(jid) && s.quiet_reload_jid.is_none() && !s.history_loading {
						s.quiet_reload_jid = Some(jid.to_string());
						let payload = serde_json::json!({
							"action": "get_messages",
							"data": { "jid": jid, "limit": crate::MESSAGE_PAGE_SIZE, "offset": 0, "no_server_fetch": true }
						});
						let _ = s.ws_tx.send(payload.to_string());
					}
				}
				"channel_follow_changed" => {
					if let Some(data) = val.get("data") {
						let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
						let following = data.get("following").and_then(|f| f.as_bool()).unwrap_or(false);
						let directory = state.borrow().channel_directory.clone();
						if let Some(directory) = directory {
							directory.set_following(jid, following);
						}
						let name =
							state.borrow().contacts.get(jid).cloned().unwrap_or_else(|| "the channel".to_string());
						let message = if following {
							format!("Following {}. It's in the Channels tab, Alt+6.", name)
						} else {
							format!("Unfollowed {}.", name)
						};
						crate::announce::announce(parts.conversations_parts.messages_announcer.clone(), &message);
					}
				}
				"group_invite_link" => {
					if let Some(data) = val.get("data") {
						let jid = data.get("jid").and_then(|v| v.as_str()).unwrap_or("").to_string();
						let link = data.get("link").and_then(|v| v.as_str()).unwrap_or("").to_string();
						if !link.is_empty() {
							crate::ui_commands::links::show_group_invite_link(state, parts, jid, link);
						}
					}
				}
				"group_invite_joined" => {
					let name = val
						.get("data")
						.and_then(|d| d.get("jid"))
						.and_then(|j| j.as_str())
						.and_then(|jid| state.borrow().contacts.get(jid).cloned())
						.unwrap_or_else(|| "the group".to_string());
					crate::announce::announce(
						parts.conversations_parts.messages_announcer.clone(),
						&format!("Joined {}.", name),
					);
				}
				"about_updated" => {
					if let Some(jid) = val.get("data").and_then(|d| d.get("jid")).and_then(|j| j.as_str()) {
						let s = state.borrow();
						let is_relevant =
							s.selected_chat.as_deref() == Some(jid) && parts.conversation_data_dialog.dialog.is_shown();
						if is_relevant {
							let payload = serde_json::json!({
								"action": "get_user_info",
								"data": { "jid": jid }
							});
							let _ = s.ws_tx.send(payload.to_string());
						}
					}
				}
				"user_info" => {
					if let Some(data) = val.get("data") {
						let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
						let status = data.get("status").and_then(|s| s.as_str()).unwrap_or("");
						let push_name = data.get("pushName").and_then(|p| p.as_str()).unwrap_or("");
						let full_name = data.get("fullName").and_then(|f| f.as_str()).unwrap_or("");

						let s = state.borrow();
						if Some(jid.to_string()) == s.selected_chat {
							let local_name = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.to_string());
							let raw_phone = jid.split('@').next().unwrap_or(jid);
							let mut phone = raw_phone.to_string();
							if let Ok(parsed) = phonenumber::parse(None, format!("+{}", raw_phone)) {
								phone = parsed.format().mode(phonenumber::Mode::International).to_string();
							}

							let mut text = format!("Name: {}\nPhone: {}\n", local_name, phone);
							if !push_name.is_empty() {
								text.push_str(&format!("WhatsApp Name: {}\n", push_name));
							} else if !full_name.is_empty() {
								text.push_str(&format!("WhatsApp Name: {}\n", full_name));
							} else {
								text.push_str("WhatsApp Name: Not available\n");
							}

							if !status.is_empty() {
								text.push_str(&format!("About/Status: {}\n", status));
							} else {
								text.push_str("About/Status: Not set\n");
							}

							parts.conversation_data_dialog.info_field.set_value(&text);
							crate::announce::announce(
								parts.conversations_parts.messages_announcer.clone(),
								&format!("Conversation data loaded for {}", local_name),
							);
						}
					}
				}
				"own_profile" => {
					if let Some(data) = val.get("data") {
						let mut s = state.borrow_mut();
						if let Some(push_name) = data.get("pushName").and_then(|p| p.as_str()) {
							s.own_push_name = push_name.to_string();
						}
						if let Some(about) = data.get("about").and_then(|a| a.as_str()) {
							s.own_about = about.to_string();
						}
						let push_name = s.own_push_name.clone();
						let about = s.own_about.clone();
						drop(s);
						if parts.settings_dialog.dialog.is_shown() {
							parts.settings_dialog.name_field.set_value(&push_name);
							parts.settings_dialog.about_field.set_value(&about);
						}
					}
				}
				"privacy_settings" => {
					if let Some(data) = val.get("data") {
						let mut s = state.borrow_mut();
						for (key, ptr) in [
							("groupadd", "groupadd"),
							("last", "last"),
							("status", "status"),
							("profile", "profile"),
							("readreceipts", "readreceipts"),
							("online", "online"),
							("calladd", "calladd"),
						] {
							if let Some(v) = data.get(key).and_then(|v| v.as_str()) {
								s.privacy_settings.insert(ptr.to_string(), v.to_string());
							}
						}
						s.privacy_settings_loaded = true;
						let privacy_settings = s.privacy_settings.clone();
						drop(s);
						if parts.settings_dialog.dialog.is_shown() {
							use crate::ui::settings_dialog::*;
							parts.settings_dialog.ok_btn.enable(true);
							let get = |k: &str| privacy_settings.get(k).map(|s| s.as_str()).unwrap_or("all");
							set_combo_from_value(
								&parts.settings_dialog.last_seen_combo,
								VISIBILITY_CHOICES,
								get("last"),
							);
							set_combo_from_value(
								&parts.settings_dialog.profile_photo_combo,
								VISIBILITY_CHOICES,
								get("profile"),
							);
							set_combo_from_value(
								&parts.settings_dialog.about_privacy_combo,
								VISIBILITY_CHOICES,
								get("status"),
							);
							set_combo_from_value(
								&parts.settings_dialog.read_receipts_combo,
								READ_RECEIPTS_CHOICES,
								get("readreceipts"),
							);
							set_combo_from_value(
								&parts.settings_dialog.groups_combo,
								VISIBILITY_CHOICES,
								get("groupadd"),
							);
							set_combo_from_value(&parts.settings_dialog.calls_combo, CALL_CHOICES, get("calladd"));
							set_combo_from_value(&parts.settings_dialog.online_combo, ONLINE_CHOICES, get("online"));
						}
					}
				}
				kind @ ("message_status" | "receipt") => {
					if let Some(data) = val.get("data") {
						// A raw receipt in a group is one member's; the message's own
						// ticks only move once the whole group has it, which the
						// backend works out and sends as message_status.
						let chat = data.get("jid").or_else(|| data.get("chat_jid")).and_then(|j| j.as_str());
						if kind == "receipt" && chat.is_some_and(|j| j.ends_with("@g.us")) {
							return;
						}
						let chat_jid =
							data.get("jid").or_else(|| data.get("chat_jid")).and_then(|j| j.as_str()).unwrap_or("");
						let raw_status =
							data.get("status").or_else(|| data.get("type")).and_then(|s| s.as_str()).unwrap_or("");

						let mut msg_ids = Vec::new();
						if let Some(ids) = data.get("message_ids").and_then(|i| i.as_array()) {
							for id_val in ids {
								if let Some(id_str) = id_val.as_str() {
									msg_ids.push(id_str.to_string());
								}
							}
						} else if let Some(id_str) = data.get("message_id").and_then(|i| i.as_str()) {
							msg_ids.push(id_str.to_string());
						}

						// Normalize JID: strip :device suffix so "1234:0@s.whatsapp.net"
						// compares equal to "1234@s.whatsapp.net".
						let norm_jid_str = |jid: &str| -> String {
							let no_device = jid.split(':').next().unwrap_or(jid);
							if no_device.contains('@') {
								no_device.to_string()
							} else {
								jid.split('@').next().unwrap_or(jid).to_string()
							}
						};
						let norm_incoming = norm_jid_str(chat_jid);

						let mut s = state.borrow_mut();
						let is_match = if let Some(ref sel) = s.selected_chat {
							norm_incoming == norm_jid_str(sel)
						} else {
							false
						};

						if is_match {
							let is_group = chat_jid.ends_with("@g.us");
							let norm_status = match raw_status {
								"read-self" => "read",
								"sender" => "sent",
								"inactive" => "delivered",
								"" => "delivered",
								other => other,
							};
							let receipt_ts = data.get("timestamp").and_then(|t| t.as_i64()).unwrap_or(0);

							let ids: std::collections::HashSet<&str> = msg_ids.iter().map(String::as_str).collect();
							// Receipts for messages not loaded yet wait for them.
							let mut missing = ids.clone();
							for msg in &s.messages {
								missing.remove(msg.id.as_str());
							}
							let any_updated = missing.len() < ids.len();
							for id in missing {
								s.pending_receipts.insert(id.to_string(), norm_status.to_string());
							}
							let rows = update_loaded_messages(
								&mut s,
								|m| {
									ids.contains(m.id.as_str())
										&& get_status_rank(norm_status) >= get_status_rank(&m.status)
								},
								|m| {
									m.status = norm_status.to_string();
									if receipt_ts > 0 {
										let at = match norm_status {
											"delivered" => &mut m.delivered_at,
											"read" => &mut m.read_at,
											"played" => &mut m.played_at,
											_ => return,
										};
										if *at == 0 {
											*at = receipt_ts;
										}
									}
								},
							);
							for di in rows {
								if let Some(item) = s.display_items.get(di) {
									parts
										.conversations_parts
										.messages_list
										.set_string(di as u32, &item.format_display(is_group));
								}
							}
							if any_updated && (norm_status == "sent" || norm_status == "delivered") {
								// A reconnect or history sync can flush receipts for a
								// whole backlog of older outgoing messages at once; only
								// ding for messages we actually just sent, not every
								// backlogged ID that happens to be getting acked now.
								let now_ts = std::time::SystemTime::now()
									.duration_since(std::time::UNIX_EPOCH)
									.unwrap_or_default()
									.as_secs() as i64;
								// Only IDs that could still ring are worth remembering.
								// The recency gate means anything older than the window
								// can never produce a sound again, so admitting it to
								// the set was pure unbounded growth - and a reconnect
								// admits the entire backlog at once. Drop what has aged
								// out, and never take a backlogged ID in the first place.
								s.played_sent_sounds.retain(|_, acked_at| now_ts - *acked_at < SENT_SOUND_WINDOW_SECS);
								let mut should_play = false;
								// One pass over the chat, not one per acked id.
								let recent: Vec<String> = s
									.messages
									.iter()
									.filter(|m| {
										ids.contains(m.id.as_str()) && now_ts - m.sent_at < SENT_SOUND_WINDOW_SECS
									})
									.map(|m| m.id.clone())
									.collect();
								for target_id in recent {
									if !s.played_sent_sounds.contains_key(&target_id) {
										s.played_sent_sounds.insert(target_id, now_ts);
										should_play = true;
									}
								}
								if should_play {
									play_notification_sound(
										NotificationSound::MessageSent,
										s.settings.sounds_enabled,
										s.settings.output_device_name.clone(),
									);
								}
							}
						}
					}
				}
				"poll_details" => {
					if let Some(data) = val.get("data") {
						crate::ui_commands::polls::show_poll_vote_dialog(state, parts, data);
					}
				}
				"reported_messages" => {
					if let Some(data) = val.get("data") {
						crate::ui_commands::report::show_reported_messages(state, parts, data);
					}
				}
				"reported" => {
					if let Some(data) = val.get("data") {
						crate::ui_commands::report::reported(parts, data);
					}
				}
				"event_details" => {
					if let Some(data) = val.get("data") {
						crate::ui_commands::polls::show_event_rsvp_dialog(state, parts, data);
					}
				}
				"event_answered" => {
					if let Some(data) = val.get("data") {
						let results = data.get("pollResults").and_then(|m| m.as_str()).unwrap_or("");
						crate::announce::announce(
							parts.conversations_parts.messages_announcer.clone(),
							&format!("Answer sent. {}", results),
						);
					}
				}
				"poll_updated" => {
					if let Some(data) = val.get("data") {
						let id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("");
						let results = data.get("pollResults").and_then(|m| m.as_str()).unwrap_or("");
						crate::ui_commands::polls::update_poll_results(state, parts, id, results);
					}
				}
				"poll_voted" => {
					if let Some(data) = val.get("data") {
						let results = data.get("pollResults").and_then(|m| m.as_str()).unwrap_or("");
						let retracted = data.get("retracted").and_then(|m| m.as_bool()).unwrap_or(false);
						let lead = if retracted { "Vote removed." } else { "Voted." };
						crate::announce::announce(
							parts.conversations_parts.messages_announcer.clone(),
							&format!("{} {}", lead, results),
						);
					}
				}
				"message_edited" => {
					if let Some(data) = val.get("data") {
						let chat_jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
						let target_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("");
						let new_text = data.get("text").and_then(|t| t.as_str()).unwrap_or("");
						let edited_at = data.get("edited_at").and_then(|e| e.as_i64()).unwrap_or(0);
						// Meta AI (and other bots) stream a reply as several
						// edits of the same bubble while "typing"; the backend
						// marks all but the finished one `final: false` so
						// NVDA doesn't read out every growing partial draft.
						// Absent (a real edit, not a bot stream) means final.
						let is_final_edit = data.get("final").and_then(|f| f.as_bool()).unwrap_or(true);
						// The backend's own rewrite of a row (a view-once message
						// once opened), not an edit by its sender.
						let not_an_edit = data.get("not_edit").and_then(|f| f.as_bool()).unwrap_or(false);

						let norm_jid_edit = |jid: &str| -> String {
							if let Some((user, server)) = jid.split_once('@') {
								let base_user =
									user.split('.').next().unwrap_or(user).split(':').next().unwrap_or(user);
								format!("{}@{}", base_user, server)
							} else {
								jid.to_string()
							}
						};

						let mut s = state.borrow_mut();
						let is_match = if let Some(ref sel) = s.selected_chat {
							norm_jid_edit(chat_jid) == norm_jid_edit(sel)
						} else {
							false
						};

						if is_match {
							let is_group = chat_jid.ends_with("@g.us");
							let edited_row = update_loaded_message(&mut s, target_id, |m| {
								m.text = new_text.to_string();
								if not_an_edit {
									m.media_path.clear();
								} else {
									m.is_edited = true;
									m.edited_at = edited_at;
								}
							});
							if let Some(di) = edited_row {
								// Replies show the text they quote.
								let quoting_rows = update_loaded_messages(
									&mut s,
									|m| m.quoted_id == target_id,
									|m| m.quoted_text = new_text.to_string(),
								);
								if let Some(item) = s.display_items.get(di) {
									let updated_display = item.format_display(is_group);
									parts.conversations_parts.messages_list.set_string(di as u32, &updated_display);
									if is_final_edit && !not_an_edit {
										crate::announce::announce(
											parts.conversations_parts.messages_announcer.clone(),
											&format!("Message edited: {}", updated_display),
										);
									}
								}
								for row in quoting_rows.into_iter().filter(|&row| row != di) {
									if let Some(item) = s.display_items.get(row) {
										parts
											.conversations_parts
											.messages_list
											.set_string(row as u32, &item.format_display(is_group));
									}
								}
							}
						}
					}
				}
				"message_starred" => {
					if let Some(data) = val.get("data") {
						let chat_jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
						let target_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("");
						let starred = data.get("starred").and_then(|s| s.as_bool()).unwrap_or(false);

						let norm_jid_star = |jid: &str| -> String {
							if let Some((user, server)) = jid.split_once('@') {
								let base_user =
									user.split('.').next().unwrap_or(user).split(':').next().unwrap_or(user);
								format!("{}@{}", base_user, server)
							} else {
								jid.to_string()
							}
						};

						let mut s = state.borrow_mut();
						let is_match = if let Some(ref sel) = s.selected_chat {
							norm_jid_star(chat_jid) == norm_jid_star(sel)
						} else {
							false
						};

						if is_match {
							let is_group = chat_jid.ends_with("@g.us");
							if let Some(di) = update_loaded_message(&mut s, target_id, |m| m.starred = starred) {
								if let Some(item) = s.display_items.get(di) {
									let updated_display = item.format_display(is_group);
									parts.conversations_parts.messages_list.set_string(di as u32, &updated_display);
									crate::announce::announce(
										parts.conversations_parts.messages_announcer.clone(),
										if starred { "Message starred." } else { "Message unstarred." },
									);
								}
							}
						}
						if s.active_tab == 2 {
							let payload = serde_json::json!({ "action": "get_starred_messages" });
							let _ = s.ws_tx.send(payload.to_string());
						}
					}
				}
				"message_reaction" => {
					if let Some(data) = val.get("data") {
						let chat_jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
						let target_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("");
						let reactor = data.get("reactor").and_then(|r| r.as_str()).unwrap_or("");
						let emoji = data.get("emoji").and_then(|e| e.as_str()).unwrap_or("");
						let reactor_name_val = data.get("reactor_name").and_then(|n| n.as_str()).unwrap_or("");

						let norm_jid_react = |jid: &str| -> String {
							let no_device = jid.split(':').next().unwrap_or(jid);
							if no_device.contains('@') {
								no_device.to_string()
							} else {
								jid.split('@').next().unwrap_or(jid).to_string()
							}
						};

						let mut s = state.borrow_mut();
						let is_match = if let Some(ref sel) = s.selected_chat {
							norm_jid_react(chat_jid) == norm_jid_react(sel)
						} else {
							false
						};

						if is_match {
							let is_group = chat_jid.ends_with("@g.us");

							let is_own_reaction =
								reactor == "me" || reactor_name_val == "You" || reactor_name_val == "me";

							let resolved_name = if is_own_reaction {
								"You".to_string()
							} else if !reactor_name_val.is_empty() && !reactor_name_val.contains('@') {
								reactor_name_val.to_string()
							} else {
								s.contacts
									.get(norm_jid_react(reactor).as_str())
									.or_else(|| s.contacts.get(reactor))
									.cloned()
									.unwrap_or_else(|| "Contact".to_string())
							};

							let reacted_row = update_loaded_message(&mut s, target_id, |msg| {
								if emoji.is_empty() {
									msg.reactions.retain(|r| {
										if is_own_reaction {
											r.reactor_name != "You" && r.reactor_jid != "me" && r.reactor_name != "me"
										} else {
											r.reactor_jid != reactor && r.reactor_name != resolved_name
										}
									});
								} else {
									let mut found = false;
									for r in msg.reactions.iter_mut() {
										let r_is_own = r.reactor_name == "You" || r.reactor_jid == "me";
										if (is_own_reaction && r_is_own)
											|| (!is_own_reaction
												&& (r.reactor_jid == reactor || r.reactor_name == resolved_name))
										{
											r.emoji = emoji.to_string();
											r.reactor_name = resolved_name.clone();
											found = true;
											break;
										}
									}
									if !found {
										msg.reactions.push(whantv::ChatReaction {
											reactor_jid: if is_own_reaction {
												"me".to_string()
											} else {
												reactor.to_string()
											},
											reactor_name: resolved_name.clone(),
											emoji: emoji.to_string(),
											timestamp: std::time::SystemTime::now()
												.duration_since(std::time::UNIX_EPOCH)
												.unwrap_or_default()
												.as_secs()
												as i64,
										});
									}
								}
							});

							if let Some((di, item)) =
								reacted_row.and_then(|di| s.display_items.get(di).map(|item| (di, item)))
							{
								parts
									.conversations_parts
									.messages_list
									.set_string(di as u32, &item.format_display(is_group));
								let announce_msg = if emoji.is_empty() {
									format!("Reaction removed by {}", resolved_name)
								} else {
									format!("{} reacted with {}", resolved_name, emoji)
								};
								crate::announce::announce(
									parts.conversations_parts.messages_announcer.clone(),
									&announce_msg,
								);
							}
						} else if !emoji.is_empty() {
							// Reaction on a chat that isn't currently open - there's no
							// loaded ChatMessage to update, just surface it the way
							// WhatsApp itself does: a notification naming the reactor and
							// what they reacted to.
							let is_own_reaction =
								reactor == "me" || reactor_name_val == "You" || reactor_name_val == "me";
							if !is_own_reaction {
								let text = data.get("text").and_then(|t| t.as_str()).unwrap_or("");
								if !text.is_empty() {
									let resolved_name =
										if !reactor_name_val.is_empty() && !reactor_name_val.contains('@') {
											reactor_name_val.to_string()
										} else {
											s.contacts
												.get(norm_jid_react(reactor).as_str())
												.or_else(|| s.contacts.get(reactor))
												.cloned()
												.unwrap_or_else(|| "Contact".to_string())
										};
									let alert = whantv::incoming_alert(
										s.muted_jids.contains(chat_jid),
										s.archived_jids.contains(chat_jid),
										s.settings.archived_alert(chat_jid),
									);
									if alert.sound && !crate::do_not_disturb::should_silence(&s.settings) {
										play_notification_sound(
											NotificationSound::MessageBackground,
											s.settings.sounds_enabled,
											s.settings.output_device_name.clone(),
										);
									}
									if alert.toast && s.settings.notifications_enabled {
										if let Some(n) = show_toast_notification(
											&resolved_name,
											&format!(
												"Reacted {} to \"{}\"",
												emoji,
												whantv::friendly_preview_text(text)
											),
										) {
											crate::chat_ui::record_notification(&mut s, chat_jid, n);
										}
									}
								}
							}
						}
					}
				}
				"copied_album_to_clipboard" => {
					let ann_text = if let Some(data) = val.get("data") {
						let msg_ids: Vec<String> = data
							.get("message_ids")
							.and_then(|ids| ids.as_array())
							.map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
							.unwrap_or_default();

						let mut s = state.borrow_mut();
						let mut is_voice = false;
						for id in &msg_ids {
							s.pending_copy_announcements.remove(id);
							if let Some(m) = s.messages.iter().find(|msg| msg.id == *id) {
								if m.is_audio
									|| m.media_type == "voice"
									|| m.media_type == "audio"
									|| m.text.starts_with("[Voice Message]")
									|| m.text.starts_with("[Audio]")
								{
									is_voice = true;
								}
							}
						}
						if is_voice {
							"Copied voice message to clipboard.".to_string()
						} else if msg_ids.len() <= 1 {
							"Copied file to clipboard.".to_string()
						} else {
							format!("Copied {} files to clipboard.", msg_ids.len())
						}
					} else {
						"Copied file to clipboard.".to_string()
					};

					crate::announce::announce(parts.conversations_parts.messages_announcer.clone(), &ann_text);
				}
				"media_downloaded" => {
					if let Some(data) = val.get("data") {
						let msg_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("").to_string();
						// The backend already names cached files with a proper
						// extension; renaming one here left the database pointing
						// at a path that no longer existed.
						let path = data.get("path").and_then(|p| p.as_str()).unwrap_or("").to_string();
						let filename = data.get("filename").and_then(|f| f.as_str()).unwrap_or("").to_string();
						let clean_filename = ChatMessage::sanitize_filename(&filename);

						let mut s = state.borrow_mut();
						s.downloads_in_progress.remove(&msg_id);
						update_loaded_message(&mut s, &msg_id, |m| {
							m.media_path = path;
							if !clean_filename.is_empty() {
								m.media_filename = clean_filename;
							}
						});

						let download_had_focus = parts.conversations_parts.action_download_btn.has_focus();
						parts.conversations_parts.conversation_panel.freeze();
						if update_action_buttons(&s, &parts.conversations_parts) {
							parts.conversations_parts.action_panel.layout();
						}
						parts.conversations_parts.conversation_panel.thaw();

						if download_had_focus && !parts.conversations_parts.action_download_btn.is_shown() {
							if parts.conversations_parts.action_open_btn.is_shown() {
								parts.conversations_parts.action_open_btn.set_focus();
							} else if parts.conversations_parts.action_save_as_btn.is_shown() {
								parts.conversations_parts.action_save_as_btn.set_focus();
							}
						}

						let is_copy_pending = s.pending_copy_announcements.contains(&msg_id);
						let media_type = data.get("mediaType").and_then(|m| m.as_str()).unwrap_or("");
						if media_type != "voice" && !is_copy_pending {
							let announce_name = if filename.is_empty() { "File".to_string() } else { filename };
							crate::announce::announce(
								parts.conversations_parts.messages_announcer.clone(),
								&format!("{} downloaded", announce_name),
							);
						}
					}
				}
				"media_download_progress" => {
					if let Some(data) = val.get("data") {
						let msg_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("").to_string();
						let percent = data.get("percent").and_then(|p| p.as_i64()).unwrap_or(0);
						// Voice notes download on their own as they arrive; only
						// downloads the user asked for get spoken progress.
						let auto = data.get("auto").and_then(|a| a.as_bool()).unwrap_or(false);
						let mut s = state.borrow_mut();
						let prev = s.downloads_in_progress.insert(msg_id.clone(), percent);
						parts.conversations_parts.conversation_panel.freeze();
						if update_action_buttons(&s, &parts.conversations_parts) {
							parts.conversations_parts.action_panel.layout();
						}
						parts.conversations_parts.conversation_panel.thaw();
						for milestone in [25, 50, 75] {
							if !auto && prev.unwrap_or(0) < milestone && percent >= milestone {
								crate::announce::announce(
									parts.conversations_parts.messages_announcer.clone(),
									&format!("Download {}%", percent),
								);
								break;
							}
						}
					}
				}
				"media_download_cancelled" | "media_download_failed" => {
					if let Some(data) = val.get("data") {
						let msg_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("").to_string();
						let mut s = state.borrow_mut();
						s.downloads_in_progress.remove(&msg_id);
						parts.conversations_parts.conversation_panel.freeze();
						if update_action_buttons(&s, &parts.conversations_parts) {
							parts.conversations_parts.action_panel.layout();
						}
						parts.conversations_parts.conversation_panel.thaw();
						// A background voice-note download failing is no news to
						// anyone - playing it later simply tries again.
						let quiet = data.get("auto").and_then(|a| a.as_bool()).unwrap_or(false)
							&& s.playing_msg_id.as_deref() != Some(msg_id.as_str());
						if !quiet {
							let announcement = if t == "media_download_cancelled" {
								"Download cancelled".to_string()
							} else {
								let err = data.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error");
								format!("Download failed: {}", err)
							};
							crate::announce::announce(
								parts.conversations_parts.messages_announcer.clone(),
								&announcement,
							);
							parts.conversations_parts.play_voice_btn.set_label("Play voice message");
						}
					}
				}
				"media_saved" => {
					if let Some(data) = val.get("data") {
						let path = data.get("path").and_then(|p| p.as_str()).unwrap_or("");
						crate::announce::announce(
							parts.conversations_parts.messages_announcer.clone(),
							&format!("File saved to {}", path),
						);
					}
				}
				"play_audio" => {
					if let Some(data) = val.get("data") {
						if let Some(path) = data.get("path").and_then(|p| p.as_str()) {
							// Backend sends "message_id", not "id"
							let msg_id = data
								.get("message_id")
								.or_else(|| data.get("id"))
								.and_then(|i| i.as_str())
								.map(|s| s.to_string());
							eprintln!(
								"[LOG UI] Playing voice note: {} from path: {}",
								msg_id.as_deref().unwrap_or(""),
								path
							);
							let mut s = state.borrow_mut();
							// Keep playing_msg_id from the UI-side set (which used the correct msg.id),
							// only update it if the server echoes a non-empty id
							if let Some(ref id) = msg_id {
								if !id.is_empty() {
									s.playing_msg_id = Some(id.clone());
								}
							}
							// The user may have hit "pause" while this was still loading
							// (playing_msg_id is set immediately on click, before this
							// response arrives) - honor that instead of always starting
							// playback out from under them.
							let start_paused = s.is_audio_paused;
							s.audio_elapsed_secs = 0.0;
							s.audio_start_instant = None;
							s.audio_path = Some(path.to_string());
							if let Some(ref sink) = s.audio_sink {
								sink.stop();
							}
							s.audio_sink = None;

							// Duration strategy:
							// 1. Count samples via rodio (works for WAV/FLAC/MP3 but not opus)
							// 2. Parse "M:SS" from the message text e.g. "[Voice Message] (0:23)"
							// 3. Count samples by consuming the decoder (always works, small files)
							s.audio_total_secs = 0.0;

							// Try rodio total_duration first (fast, works for some formats)
							if let Ok(probe_file) = File::open(path) {
								if let Ok(probe_dec) = Decoder::new(BufReader::new(probe_file)) {
									use rodio::Source;
									if let Some(dur) = probe_dec.total_duration() {
										s.audio_total_secs = dur.as_secs_f32();
									}
								}
							}

							// Parse duration from message text: "[Voice Message] (M:SS)" or "[Audio] (M:SS)"
							if s.audio_total_secs == 0.0 {
								let playing_id = s.playing_msg_id.clone();
								if let Some(ref pid) = playing_id {
									if let Some(msg) = s.messages.iter().find(|m| &m.id == pid) {
										let text = &msg.text;
										// Find "(M:SS)" pattern inside the tag
										if let Some(open) = text.find('(') {
											if let Some(close) = text[open..].find(')') {
												let inner = &text[open + 1..open + close];
												// parse "M:SS"
												let parts_dur: Vec<&str> = inner.split(':').collect();
												if parts_dur.len() == 2 {
													if let (Ok(m), Ok(ss)) =
														(parts_dur[0].parse::<f32>(), parts_dur[1].parse::<f32>())
													{
														s.audio_total_secs = m * 60.0 + ss;
													}
												}
											}
										}
									}
								}
							}

							eprintln!("[LOG UI] Audio total duration: {}s", s.audio_total_secs);

							ensure_audio_stream(&mut s);
							let mut played = false;
							if let Some(ref stream_handle) = s.audio_stream {
								let sink = Player::connect_new(&stream_handle.mixer());
								if let Ok(file) = File::open(path) {
									if let Ok(decoder) = Decoder::new(BufReader::new(file)) {
										sink.append(decoder.wsola(s.audio_speed));
										if start_paused {
											sink.pause();
										} else {
											sink.play();
										}
										s.audio_sink = Some(sink);
										played = true;
										s.audio_start_instant =
											if start_paused { None } else { Some(std::time::Instant::now()) };
										parts.conversations_parts.play_voice_btn.set_label(if start_paused {
											"Play voice message"
										} else {
											"Pause voice message"
										});
									} else if let Some(opus_buf) = decode_opus_file(path) {
										sink.append(opus_buf.wsola(s.audio_speed));
										if start_paused {
											sink.pause();
										} else {
											sink.play();
										}
										s.audio_sink = Some(sink);
										played = true;
										s.audio_start_instant =
											if start_paused { None } else { Some(std::time::Instant::now()) };
										parts.conversations_parts.play_voice_btn.set_label(if start_paused {
											"Play voice message"
										} else {
											"Pause voice message"
										});
									}
								}
							}
							if !played {
								// Every format WhatsApp actually sends is covered above
								// (Ogg Opus by decode_opus_file, everything else by
								// symphonia), so landing here means the output device
								// couldn't be opened or the file is truncated/corrupt.
								// Reset back to "not playing" rather than leaving the
								// button stuck on "Pause" for audio that never started.
								eprintln!("[LOG UI] No decoder could play {}", path);
								s.playing_msg_id = None;
								s.audio_path = None;
								s.audio_start_instant = None;
								s.audio_elapsed_secs = 0.0;
								s.is_audio_paused = false;
								drop(s);
								parts.conversations_parts.play_voice_btn.set_label("Play voice message");
								parts.conversations_parts.audio_slider.show(false);
								parts.conversations_parts.audio_speed_btn.show(false);
								parts.conversations_parts.audio_progress_label.show(false);
								parts.conversations_parts.conversation_panel.layout();
								crate::announce::announce(
									parts.conversations_parts.messages_announcer.clone(),
									"Could not play voice message",
								);
								return;
							}
							s.is_audio_paused = start_paused;
							// Show slider UI and speed button
							parts.conversations_parts.audio_slider.show(true);
							parts.conversations_parts.audio_speed_btn.show(true);
							parts.conversations_parts.audio_progress_label.show(false);
							let total = s.audio_total_secs;
							let max_sec = (total as i32).max(1);
							parts.conversations_parts.audio_slider.set_range(0, max_sec);
							parts.conversations_parts.audio_slider.set_value(0);
							let fmt_dur = |v: f32| {
								if v > 0.0 {
									format!("{}:{:02}", v as u32 / 60, v as u32 % 60)
								} else {
									"?:??".to_string()
								}
							};
							parts
								.conversations_parts
								.audio_progress_label
								.set_label(&format!("0:00 / {}", fmt_dur(total)));
							parts.conversations_parts.conversation_panel.layout();
						}
					}
				}
				"attachment_uploading" => {
					// Percentage-level upload progress. The message row itself
					// (text/status) is kept in sync by the "message" event, which
					// the backend sends immediately with a real id and again on
					// completion/failure; this only drives the live announcement
					// and the (usually already-closed) attachment dialog's label.
					if let Some(data) = val.get("data") {
						let filename = data.get("filename").and_then(|f| f.as_str()).unwrap_or("file");
						let percent = data.get("percent").and_then(|p| p.as_i64());
						let msg = if let Some(p) = percent {
							format!("Uploading {} ({}%)...", filename, p)
						} else {
							format!("Uploading {}...", filename)
						};
						parts.attachment_dialog.attachment_info_label.set_label(&msg);
						crate::announce::announce(parts.conversations_parts.messages_announcer.clone(), &msg);
					}
				}
				"attachment_sent" => {
					// The message row itself is already updated to "sent" by its
					// own "message" event, which the backend always sends first.
					parts.attachment_dialog.attachment_info_label.set_label("");
				}
				_ => {}
			}
		}
	}
}

#[cfg(test)]
mod conversation_messages_tests {
	use super::*;

	#[test]
	fn chats_take_their_own_fast_path() {
		assert_eq!(fast_path(r#"{"type":"chats","data":[]}"#), FastPath::Chats);
		assert_eq!(fast_path(r#"{"type":"chat_cleared","data":{}}"#), FastPath::Other);
	}

	#[test]
	fn a_chats_payload_parses_with_escapes_and_missing_fields() {
		let raw = r#"{"type":"chats","data":[
			{"JID":"a@s.whatsapp.net","FullName":"A \"quoted\" name","UnreadCount":3,"Pinned":true,"Timestamp":5},
			{"JID":"g@g.us","IsCommunity":true,"ChannelRole":""}
		]}"#;
		let chats = serde_json::from_str::<ChatsEnvelope>(raw).unwrap().data.unwrap();
		assert_eq!(chats.len(), 2);
		assert_eq!(chats[0].full_name.as_deref(), Some("A \"quoted\" name"));
		assert_eq!(chats[0].unread_count, 3);
		assert!(chats[0].pinned && !chats[0].muted);
		assert_eq!(chats[1].full_name, None);
		assert!(chats[1].is_community);
	}

	#[test]
	fn a_chats_payload_without_data_has_none() {
		assert!(serde_json::from_str::<ChatsEnvelope>(r#"{"type":"chats"}"#).unwrap().data.is_none());
	}

	#[test]
	fn only_conversation_messages_takes_the_typed_fast_path() {
		assert!(is_conversation_messages(r#"{"type":"conversation_messages","data":{}}"#));
		// The tag can sit after a large payload and still be found.
		assert!(is_conversation_messages(r#"{"data":{"messages":[{"id":"a"}]},"type":"conversation_messages"}"#));
		assert!(!is_conversation_messages(r#"{"type":"message","data":{}}"#));
		assert!(!is_conversation_messages(r#"{"data":{}}"#));
		assert!(!is_conversation_messages("not json at all"));
	}

	#[test]
	fn parses_the_jid_and_every_message() {
		let raw = r#"{"type":"conversation_messages","data":{"jid":"a@s.whatsapp.net","messages":[
			{"id":"m1","text":"hi","timestamp":10,"fromMe":true,"mediaType":"image","albumId":"alb1"},
			{"id":"m2","text":"yo","timestamp":11,"sender_jid":"b@s.whatsapp.net"}
		]}}"#;
		let (jid, load_more, _, _, msgs) = parse_conversation_messages(raw).expect("envelope should parse");
		assert_eq!(jid, "a@s.whatsapp.net");
		assert!(!load_more);
		assert_eq!(msgs.len(), 2);
		assert_eq!(msgs[0].id, "m1");
		assert!(msgs[0].from_me);
		assert_eq!(msgs[0].media_type, "image");
		assert_eq!(msgs[0].album_id, "alb1");
		assert_eq!(msgs[1].id, "m2");
		assert_eq!(msgs[1].sender_jid, "b@s.whatsapp.net");
	}

	#[test]
	fn parses_the_load_more_flag() {
		let raw = r#"{"type":"conversation_messages","data":{"jid":"a","loadMore":true,"messages":[]}}"#;
		let (_, load_more, _, _, _) = parse_conversation_messages(raw).expect("envelope should parse");
		assert!(load_more);
	}

	#[test]
	fn parses_the_load_newer_flag() {
		let raw =
			r#"{"type":"conversation_messages","data":{"jid":"a","loadNewer":true,"isLatest":false,"messages":[]}}"#;
		let (_, load_more, load_newer, is_latest, _) = parse_conversation_messages(raw).expect("envelope should parse");
		assert!(!load_more);
		assert!(load_newer);
		assert!(!is_latest);
	}

	#[test]
	fn a_single_bad_message_becomes_a_placeholder_instead_of_failing_the_chat() {
		// The middle message is missing `timestamp`, which has no serde default.
		let raw = r#"{"type":"conversation_messages","data":{"jid":"a@s.whatsapp.net","messages":[
			{"id":"m1","text":"before","timestamp":10},
			{"id":"m2","text":"broken"},
			{"id":"m3","text":"after","timestamp":12}
		]}}"#;
		let (_, _, _, _, msgs) = parse_conversation_messages(raw).expect("one bad row must not fail the envelope");
		assert_eq!(msgs.len(), 3);
		assert_eq!(msgs[0].text, "before");
		assert!(msgs[1].text.starts_with("Parse error:"), "got {:?}", msgs[1].text);
		assert_eq!(msgs[1].id, "");
		assert_eq!(msgs[2].text, "after");
	}

	#[test]
	fn an_empty_or_absent_message_list_parses_to_nothing() {
		let (jid, _, _, _, msgs) =
			parse_conversation_messages(r#"{"type":"conversation_messages","data":{"jid":"a","messages":[]}}"#)
				.unwrap();
		assert_eq!(jid, "a");
		assert!(msgs.is_empty());

		let (jid, _, _, _, msgs) =
			parse_conversation_messages(r#"{"type":"conversation_messages","data":{"jid":"a"}}"#).unwrap();
		assert_eq!(jid, "a");
		assert!(msgs.is_empty());
	}

	#[test]
	fn a_malformed_envelope_is_rejected_rather_than_panicking() {
		assert!(parse_conversation_messages(r#"{"type":"conversation_messages"}"#).is_none());
		assert!(parse_conversation_messages("{").is_none());
	}

	#[test]
	fn direct_chat_typing_replaces_presence_label() {
		let label_online = format_conv_data_label(
			"919876543210@s.whatsapp.net",
			"Aman Kumar",
			"Aman Kumar",
			true,
			false,
			Some("Aman Kumar (online)"),
		);
		assert_eq!(label_online, "Aman Kumar (typing...)");

		let label_last_seen = format_conv_data_label(
			"919876543210@s.whatsapp.net",
			"Aman Kumar",
			"Aman Kumar",
			true,
			false,
			Some("Aman Kumar (last seen today at 6:19 AM)"),
		);
		assert_eq!(label_last_seen, "Aman Kumar (typing...)");
	}

	#[test]
	fn direct_chat_recording_replaces_presence_label() {
		let label = format_conv_data_label(
			"919876543210@s.whatsapp.net",
			"Aman Kumar",
			"Aman Kumar",
			false,
			true,
			Some("Aman Kumar (online)"),
		);
		assert_eq!(label, "Aman Kumar (recording audio...)");
	}

	#[test]
	fn direct_chat_idle_uses_presence_label() {
		let label_online = format_conv_data_label(
			"919876543210@s.whatsapp.net",
			"Aman Kumar",
			"Aman Kumar",
			false,
			false,
			Some("Aman Kumar (online)"),
		);
		assert_eq!(label_online, "Aman Kumar (online)");

		let label_last_seen = format_conv_data_label(
			"919876543210@s.whatsapp.net",
			"Aman Kumar",
			"Aman Kumar",
			false,
			false,
			Some("Aman Kumar (last seen today at 6:19 AM)"),
		);
		assert_eq!(label_last_seen, "Aman Kumar (last seen today at 6:19 AM)");

		let label_no_presence =
			format_conv_data_label("919876543210@s.whatsapp.net", "Aman Kumar", "Aman Kumar", false, false, None);
		assert_eq!(label_no_presence, "Aman Kumar");
	}

	#[test]
	fn group_chat_typing_appends_sender() {
		let label = format_conv_data_label("123456789@g.us", "Family Group", "Aman Kumar", true, false, None);
		assert_eq!(label, "Family Group, Aman Kumar is typing...");
	}

	#[test]
	fn group_chat_recording_appends_sender() {
		let label = format_conv_data_label("123456789@g.us", "Family Group", "Aman Kumar", false, true, None);
		assert_eq!(label, "Family Group, Aman Kumar is recording audio...");
	}

	#[test]
	fn group_chat_idle_shows_group_name() {
		let label = format_conv_data_label("123456789@g.us", "Family Group", "", false, false, None);
		assert_eq!(label, "Family Group");
	}
}
