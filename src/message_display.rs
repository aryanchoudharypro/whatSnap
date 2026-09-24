//! Chat-list filtering/rendering and per-message action-button visibility
//! logic shared across the conversation view, group data dialog, and search.

use whantv::ChatMessage;
use wxdragon::prelude::*;

use crate::AppState;

pub(crate) fn build_media_messages(messages: &[whantv::Msg]) -> Vec<ChatMessage> {
	messages.iter().filter(|m| m.is_photo_or_video()).map(|m| (**m).clone()).collect()
}

pub(crate) fn build_document_messages(messages: &[whantv::Msg]) -> Vec<ChatMessage> {
	messages.iter().filter(|m| m.is_document()).map(|m| (**m).clone()).collect()
}

pub(crate) fn build_link_messages(messages: &[whantv::Msg]) -> Vec<ChatMessage> {
	messages.iter().filter(|m| m.first_link().is_some()).map(|m| (**m).clone()).collect()
}

pub(crate) fn populate_message_list(list: &ListBox, messages: &[ChatMessage], label: impl Fn(&ChatMessage) -> String) {
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

/// The chat list in the order the active tab shows it: the usual order, except
/// that the Communities tab puts each community's groups under it.
pub(crate) fn chats_in_tab_order(s: &AppState) -> Vec<String> {
	if s.active_tab == 4 {
		whantv::community_tab_order(&s.chats, &s.community_jids, &s.chat_parents, &s.announcement_jids)
	} else {
		s.chats.clone()
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

pub(crate) fn apply_chat_filter(s: &mut AppState, parts: &crate::ui::window::WindowParts) {
	let show_locked_btn = s.active_tab == 0 && !s.locked_jids.is_empty();
	if parts.conversations_parts.locked_chats_btn.is_shown() != show_locked_btn {
		parts.conversations_parts.locked_chats_btn.show(show_locked_btn);
		parts.conversations_parts.panel.layout();
	}

	let lower_query = s.search_query.to_lowercase();

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
				let viewed_tag = if item.viewed { "Viewed" } else { "New" };
				let preview = if !item.text.is_empty() {
					item.text.clone()
				} else if !item.media_type.is_empty() {
					"Media".to_string()
				} else {
					String::new()
				};
				let time_str = if item.timestamp > 0 {
					chrono::DateTime::from_timestamp(item.timestamp, 0)
						.map(|dt| {
							let local_dt: chrono::DateTime<chrono::Local> = dt.into();
							local_dt.format("%I:%M %p").to_string()
						})
						.unwrap_or_default()
				} else {
					String::new()
				};
				let label = if time_str.is_empty() {
					format!("{} [{}]: {}", group.poster_name, viewed_tag, preview)
				} else {
					format!("{} [{}]: {}, {}", group.poster_name, viewed_tag, preview, time_str)
				};
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
	} else {
		for jid in &chats_in_tab_order(s) {
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

			let name = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.clone());
			if lower_query.is_empty() || name.to_lowercase().contains(&lower_query) {
				labels.push(s.display_names.get(jid).cloned().unwrap_or(name));
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
		list.clear();
		for label in &labels {
			list.append(label);
		}
	}
	list.thaw();

	if let Some(idx) = s.selected_chat.as_ref().and_then(|sel| s.filtered_chats.iter().position(|j| j == sel)) {
		let count = list.get_count();
		if count > 0 && (idx as u32) < count {
			list.set_selection(idx as u32, true);
		}
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

	if let Some(idx) = sel {
		if let Some(item) = s.display_items.get(idx as usize) {
			match item {
				whantv::DisplayItem::Single(msg) => {
					let is_deleted = msg.is_deleted;
					let is_voice =
						msg.is_audio || msg.text.starts_with("[Voice Message]") || msg.text.starts_with("[Audio]");
					let is_media = !is_deleted
						&& !is_voice && !msg.is_system()
						&& !msg.is_call() && (!msg.media_type.is_empty()
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
							} else if let Some(size) = whantv::format_file_size(msg.media_size) {
								format!("Download ({})", size)
							} else {
								"Download".to_string()
							};
							parts.action_download_btn.set_label(&label);
						}
						return need_layout;
					}
				}
				whantv::DisplayItem::Album { messages, .. } => {
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

							let uncollected: Vec<_> =
								valid_messages.iter().filter(|m| !m.is_media_downloaded()).collect();
							let is_dl_in_progress =
								valid_messages.iter().any(|m| s.downloads_in_progress.contains_key(&m.id));
							let dl_label = if is_dl_in_progress {
								format!("Downloading album ({} items remaining)", uncollected.len())
							} else {
								let total_size: i64 = uncollected.iter().map(|m| m.media_size).sum();
								match whantv::format_file_size(total_size) {
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
			whantv::DisplayItem::Single(m) => Some(m.clone()),
			whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(),
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
			let is_paused = s.audio_sink.as_ref().map_or(true, |sink| sink.is_paused() || sink.empty());
			parts.play_voice_btn.set_label(if is_paused { "Play voice message" } else { "Pause voice message" });
			if !parts.audio_slider.is_shown() {
				parts.audio_slider.show(true);
				parts.audio_speed_btn.show(true);
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
