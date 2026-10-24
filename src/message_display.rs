//! Chat-list filtering/rendering and per-message action-button visibility
//! logic shared across the conversation view, group data dialog, and search.

use whatsnap::ChatMessage;
use wxdragon::prelude::*;

use crate::{AppState, CallTarget};

/// Changes every loaded message `pick` selects, each one exactly once, and
/// returns the display rows holding a changed message, in order. See
/// [`whatsnap::update_shared_messages`] for why this, and not `Rc::make_mut` on
/// each list in turn.
pub(crate) fn update_loaded_messages(
	s: &mut AppState,
	pick: impl FnMut(&ChatMessage) -> bool,
	change: impl FnMut(&mut ChatMessage),
) -> Vec<usize> {
	whatsnap::update_shared_messages(&mut s.messages, &mut s.display_items, &mut s.selection_rows, pick, change)
}

/// [`update_loaded_messages`] for the one message with this id: its display
/// row, if it's loaded.
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
			// Ids are unique; stop at the first.
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

// These share the loaded messages rather than copying them (see `whatsnap::Msg`).
pub(crate) fn build_media_messages(messages: &[whatsnap::Msg]) -> Vec<whatsnap::Msg> {
	messages.iter().filter(|m| m.is_photo_or_video()).cloned().collect()
}

pub(crate) fn build_document_messages(messages: &[whatsnap::Msg]) -> Vec<whatsnap::Msg> {
	messages.iter().filter(|m| m.is_document()).cloned().collect()
}

pub(crate) fn build_link_messages(messages: &[whatsnap::Msg]) -> Vec<whatsnap::Msg> {
	messages.iter().filter(|m| m.first_link().is_some()).cloned().collect()
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
		// Channels have their own tab, as in the official apps.
		0 => !archived.contains(jid) && !communities.contains(jid) && !channels.contains(jid),
		3 => archived.contains(jid),
		4 => communities.contains(jid) || parents.get(jid).is_some_and(|p| communities.contains(p)),
		5 => channels.contains(jid),
		_ => true,
	}
}

/// How many chats the Chats tab would show as unread: the same chats the
/// Unread filter can reach, so the navigation count agrees with the list.
pub(crate) fn unread_chats_on_chats_tab(s: &AppState) -> usize {
	whatsnap::unread_chat_count(&s.chats, &s.unread_counts, |jid| {
		chat_visible_in_tab(
			0,
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

/// The chat list in the order the active tab shows it: the usual order, except
/// that the Communities tab puts each community's groups under it.
/// Borrowed rather than copied: this runs on every chat-list refresh and
/// filter keystroke.
pub(crate) fn chats_in_tab_order(s: &AppState) -> Vec<&String> {
	if s.active_tab == 4 {
		whatsnap::community_tab_order(&s.chats, &s.community_jids, &s.chat_parents, &s.announcement_jids)
	} else {
		s.chats.iter().collect()
	}
}

/// If `new` is exactly `old` with one element removed and a (possibly
/// different) element inserted at the front, returns the index that element
/// held in `old`. Used to detect "one chat bumped to the top" without an
/// O(n^2) scan, so `apply_chat_filter` can patch the list in place instead
/// of rebuilding it on every message sent or received.
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

/// The Conversations tab's filters that are always there; the user's own
/// lists follow them in the dropdown.
pub(crate) const BUILT_IN_CHAT_FILTERS: [&str; 4] = ["All chats", "Unread", "Favorites", "Groups"];

/// Whether the chat filter picked above the list lets this chat through.
/// It only applies to Conversations, as WA Web's filter chips do.
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
	// Archiving, locking and deleting move chats in and out of the Chats tab
	// without touching their unread counts, so the navigation count is
	// refreshed here as well as where the counts change.
	crate::tray::refresh_chats_nav_label(s, parts);
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
	// The chat the user is on in the list (not necessarily the open one), so
	// a reorder can put them back on the same chat rather than the same row.
	let list_selected_jid = parts
		.conversations_parts
		.conversations_list
		.get_selection()
		.and_then(|i| s.filtered_chats.get(i as usize).cloned());

	let mut filtered = Vec::new();
	let mut labels = Vec::new();
	let mut filtered_starred_ids = Vec::new();
	let mut filtered_status_ids = Vec::new();

	if s.active_tab == 1 {
		// Status: a flat list, one row per status item (not grouped by
		// contact) - own statuses first, then everyone else newest-post-first
		// (status_groups is already sorted that way by the backend).
		// filtered_chats holds the poster's jid per row, filtered_status_ids
		// the status message id, so OpenConversation can look either up by idx.
		for group in &s.status_groups {
			for item in &group.items {
				let viewed_tag = match (item.viewed, group.muted) {
					(true, false) => "Viewed",
					(false, false) => "New",
					(true, true) => "Viewed, muted",
					(false, true) => "New, muted",
				};
				let preview = if !item.text.is_empty() {
					item.text.clone()
				} else if !item.media_type.is_empty() {
					"Media".to_string()
				} else {
					String::new()
				};
				let time_str = if item.timestamp > 0 {
					whatsnap::LocalTime::from_unix(item.timestamp)
						.map(|local_dt| local_dt.format("%I:%M %p"))
						.unwrap_or_default()
				} else {
					String::new()
				};
				let mut label = if time_str.is_empty() {
					format!("{} [{}]: {}", group.poster_name, viewed_tag, preview)
				} else {
					format!("{} [{}]: {}, {}", group.poster_name, viewed_tag, preview, time_str)
				};
				if group.from_me && item.viewers > 0 {
					label.push_str(&format!(", {} view{}", item.viewers, if item.viewers == 1 { "" } else { "s" }));
				}
				if !item.my_reaction.is_empty() {
					label.push_str(&format!(", you reacted {}", item.my_reaction));
				}
				if lower_query.is_empty() || label.to_lowercase().contains(&lower_query) {
					labels.push(label);
					filtered.push(group.poster_jid.clone());
					filtered_status_ids.push(item.id.clone());
				}
			}
		}
	} else if s.active_tab == 2 {
		// Starred Messages: a flat, cross-chat list built from starred_entries
		// rather than s.chats. filtered_chats still holds one jid per row so
		// OpenConversation's existing index -> jid lookup keeps working.
		for entry in &s.starred_entries {
			if lower_query.is_empty() || entry.label.to_lowercase().contains(&lower_query) {
				labels.push(entry.label.clone());
				filtered.push(entry.jid.clone());
				filtered_starred_ids.push(entry.message_id.clone());
			}
		}
	} else if s.active_tab == 6 {
		// Calls, laid out like the phone's: frequent contacts, then recent
		// calls, then every contact A to Z. A section with nothing to show is
		// left out, and its heading with it. Heading rows have no jid, which is
		// how a call or Enter on one knows to do nothing.
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
		let mut push_section = |heading: &str, rows: Vec<(String, String)>| {
			if rows.is_empty() {
				return;
			}
			labels.push(heading.to_string());
			filtered.push(String::new());
			for (label, jid) in rows {
				labels.push(label);
				filtered.push(jid);
			}
		};

		// Group calls going on that we can join come first, as on the phone.
		let ongoing: Vec<(String, String)> = s
			.joinable_calls
			.iter()
			.map(|c| (crate::ui_commands::calls::joinable_call_label(s, c), c))
			.filter(|(label, _)| lower_query.is_empty() || label.to_lowercase().contains(&lower_query))
			.map(|(label, c)| (label, format!("{}{}", crate::ui_commands::calls::JOIN_ROW_PREFIX, c.call_id)))
			.collect();
		push_section("Going on now", ongoing);

		let frequent: Vec<(String, String)> =
			s.call_frequent.iter().filter(|t| matches_target(t)).map(|t| (target_label(t), t.jid.clone())).collect();
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
			// A call still going on joins on Enter rather than calling back.
			let mut row_jid = entry.jid.clone();
			if s.joinable_calls.iter().any(|c| c.call_id == entry.id) {
				label.push_str(crate::ui_commands::calls::ONGOING_CALL_SUFFIX);
				row_jid = format!("{}{}", crate::ui_commands::calls::JOIN_ROW_PREFIX, entry.id);
			}

			if lower_query.is_empty() || label.to_lowercase().contains(&lower_query) || entry.jid.contains(&lower_query)
			{
				recent.push((label, row_jid));
			}
		}
		push_section("Recent calls", recent);

		let all: Vec<(String, String)> =
			s.call_contacts.iter().filter(|t| matches_target(t)).map(|t| (target_label(t), t.jid.clone())).collect();
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

	// Clearing and refilling the list resets the focused item and makes the
	// screen reader re-announce, so leave the control untouched when the
	// backend re-sends chats that produce identical contents.
	if labels == s.displayed_chat_labels {
		return;
	}

	let list = &parts.conversations_parts.conversations_list;
	let had_focus = list.has_focus();

	// The overwhelmingly common reason the label set changes at all is one
	// chat (whichever was just messaged) jumping to the front, with every
	// other row's relative order and text untouched. Detect that shape and
	// apply it as a single delete+insert instead of tearing down and
	// rebuilding the whole native list control - on a chat list with
	// hundreds of entries, a full clear()+append() per row on every single
	// message sent or received was the visible "UI freezes for a bit" hitch.
	let moved_from = find_single_removal_offset(&s.displayed_chat_labels, &labels);

	list.freeze();
	if let Some(idx) = moved_from {
		list.delete(idx as u32);
		list.insert(&labels[0], 0);
	} else {
		// One native call for the whole list rather than one per row.
		list.set_items(&labels.iter().map(String::as_str).collect::<Vec<_>>());
	}
	list.thaw();

	// Stay on the chat the user was on in the list. Re-selecting the open
	// chat instead made the list jump away from wherever they were arrowing
	// every time a message moved some other chat to the top.
	let target = list_selected_jid
		.as_ref()
		.and_then(|jid| s.filtered_chats.iter().position(|j| j == jid))
		.or_else(|| s.selected_chat.as_ref().and_then(|sel| s.filtered_chats.iter().position(|j| j == sel)));
	if let Some(idx) = target {
		let count = list.get_count();
		if count > 0 && (idx as u32) < count && list.get_selection() != Some(idx as u32) {
			list.set_selection(idx as u32, true);
		}
	}

	// Filling the list clears its selection. If the list has focus and nothing
	// is selected (first launch: Alt+1 arrived before any chats had loaded),
	// select the first row so Enter opens it without a Space first.
	if (had_focus || list.has_focus()) && list.get_selection().is_none() && list.get_count() > 0 {
		list.set_selection(0_u32, true);
	}

	if had_focus {
		list.set_focus();
	}

	s.displayed_chat_labels = labels;
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
					return need_layout | show_send_state_buttons(parts, is_failed);
				}
				need_layout |= set_shown(&parts.action_retry_btn, false);

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
						return need_layout | show_send_state_buttons(parts, is_failed);
					}
					need_layout |= set_shown(&parts.action_retry_btn, false);

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

/// Shows or hides `btn`, returning whether that changed anything (and so
/// needs a layout).
fn set_shown(btn: &Button, show: bool) -> bool {
	if btn.is_shown() == show {
		return false;
	}
	btn.show(show);
	true
}

/// Action buttons for an outgoing message that failed to send (Retry is the
/// only thing to do) or is still uploading (nothing to do yet). Returns
/// whether a layout is needed.
fn show_send_state_buttons(parts: &crate::ui::conversations::ConversationsPanelParts, failed: bool) -> bool {
	let mut need_layout = set_shown(&parts.action_retry_btn, failed);
	need_layout |= set_shown(&parts.action_open_btn, false);
	need_layout |= set_shown(&parts.action_save_as_btn, false);
	need_layout |= set_shown(&parts.action_download_btn, false);
	need_layout
}

/// Syncs `play_voice_btn` / `audio_slider` / `action_panel` visibility to
/// whichever row `messages_list` currently has selected.
///
/// This duplicates the logic in `messages_list`'s `on_selection_changed`
/// handler (ui/window.rs) because a *programmatic* `set_selection()` - used
/// throughout this file to land on a particular row after rebuilding the
/// list (e.g. the newest message after a send, the message just jumped to
/// from search) - doesn't fire that wx event. Without this, those buttons
/// keep reflecting whatever row was selected *before* the rebuild: e.g. if
/// the user had a voice message selected, then sent a plain text message,
/// `play_voice_btn` stayed visible and labeled for the old voice message
/// even though the newly-selected row is the text message they just sent.
/// Call this after any `messages_list.set_selection(...)` that isn't
/// immediately followed by a real user-driven selection change.
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

/// Formats a call record as a spoken, screen-reader friendly line.
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
