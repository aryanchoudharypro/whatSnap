//! Misc chat-view UI helpers: toast notifications, backend shutdown
//! signaling, and switching the conversation panel between chats (unread
//! divider, composer draft, pending reply, opening a jid).

use std::{
	cell::RefCell,
	io::{BufRead, BufReader, Write},
	rc::Rc,
	thread,
};

use interprocess::{
	TryClone,
	local_socket::{GenericNamespaced, ToNsName, prelude::*},
};
use wxdragon::prelude::*;

use crate::{
	AppState, UiCommand,
	contact_picker::{display_name_for_jid, refresh_selection_list},
	ui::window::WindowParts,
};

/// Shows a native desktop notification. Must be called on the wx main/GUI
/// thread - wx (and the underlying WinRT toast COM calls) aren't thread-safe.
/// Returns the shown notification (kept alive by the caller and closeable
/// later via `NotificationMessage::close`, e.g. once the user opens the chat
/// it was for) so it can be dismissed from Action Center on demand, the way
/// official WhatsApp clears a chat's notifications the moment you read it.
pub(crate) fn show_toast_notification(title: &str, body: &str) -> Option<wxdragon::prelude::NotificationMessage> {
	// Debounce toast notifications globally to max 1 per second to avoid flooding on startup
	static LAST_TOAST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
	let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
	let last = LAST_TOAST.swap(now, std::sync::atomic::Ordering::Relaxed);
	if now - last < 1000 {
		return None;
	}

	// `with_generic(false)` opts into the native backend, which uses the modern
	// WinRT toast (registered via `msw_use_toasts` above) rather than wxdragon's
	// default generic popup on Windows.
	match wxdragon::prelude::NotificationMessage::builder()
		.with_title(title)
		.with_message(body)
		.with_generic(false)
		.build()
	{
		Ok(notification) => {
			notification.show(wxdragon::prelude::TIMEOUT_AUTO);
			Some(notification)
		}
		Err(err) => {
			eprintln!("[NOTIFY] Failed to build notification: {:?}", err);
			None
		}
	}
}

/// Toasts kept per chat so opening it can dismiss them.
///
/// Set well above what Windows itself retains in Action Center for one app: a
/// toast Windows has already dropped cannot be dismissed anyway, so a lower cap
/// buys no memory back but does leave real, still-visible notifications
/// stranded for a chat the user has just read - noise for anyone reviewing
/// Action Center with a screen reader. The cap exists only so a chat that is
/// never opened cannot pin native objects for the whole session.
const MAX_TRACKED_NOTIFICATIONS_PER_CHAT: usize = 40;
/// Chats tracked at once, for the same reason.
const MAX_TRACKED_NOTIFICATION_CHATS: usize = 128;

/// Files a just-shown toast under `jid` so `open_chat_for_jid` can dismiss it,
/// evicting older entries to keep the bookkeeping bounded.
///
/// Evicted entries are dropped rather than `close`d: dropping frees the native
/// wrapper while leaving the toast itself in Action Center, whereas closing
/// would yank a notification the user may not have read yet.
pub(crate) fn record_notification(s: &mut AppState, jid: &str, notification: wxdragon::prelude::NotificationMessage) {
	if s.active_notifications.len() >= MAX_TRACKED_NOTIFICATION_CHATS && !s.active_notifications.contains_key(jid) {
		// Any chat but the one we are filing under - it is the one most
		// likely to be opened next, so it is the one worth keeping.
		let evict = s.active_notifications.keys().find(|k| k.as_str() != jid).cloned();
		if let Some(key) = evict {
			s.active_notifications.remove(&key);
		}
	}
	let entry = s.active_notifications.entry(jid.to_string()).or_default();
	entry.push(notification);
	if entry.len() > MAX_TRACKED_NOTIFICATIONS_PER_CHAT {
		let overflow = entry.len() - MAX_TRACKED_NOTIFICATIONS_PER_CHAT;
		entry.drain(0..overflow);
	}
}

pub(crate) fn request_backend_shutdown(pipe_id: &str) {
	let Ok(name) = pipe_id.to_ns_name::<GenericNamespaced>() else {
		return;
	};
	let Ok(mut socket) = LocalSocketStream::connect(name) else {
		return;
	};
	let Ok(drain_socket) = socket.try_clone() else {
		return;
	};

	// The backend greets every new connection with a connected event and the
	// full chat list, writing it before it reads anything. Left undrained the
	// pipe fills, the backend blocks mid-write, and it never reaches our
	// command -- so the reader has to run concurrently with the write.
	thread::spawn(move || {
		let mut reader = BufReader::new(drain_socket);
		let mut buf = String::new();
		while matches!(reader.read_line(&mut buf), Ok(n) if n > 0) {
			buf.clear();
		}
	});

	let _ = socket.write_all(b"{\"action\":\"shutdown\"}\n");
	let _ = socket.flush();
}

/// Drops the "N unread messages" divider from `jid`'s message list right
/// away, if `jid` is the chat currently open and it has one - shared by
/// replying (obviously seen) and explicitly marking the chat read from the
/// chat-list context menu. Removing just the divider row in place is O(1)
/// against the widget rather than rebuilding+re-appending the whole
/// (unvirtualized) list, which visibly froze the UI on long chats.
pub(crate) fn clear_unread_divider_for_open_chat(s: &mut AppState, parts: &WindowParts, jid: &str) {
	if s.unread_divider_jid.as_deref() != Some(jid) {
		return;
	}
	s.unread_divider_jid = None;
	s.unread_divider_count = 0;
	s.background_unread_message_ids.clear();
	s.messages.retain(|m| !m.is_unread_divider);
	if let Some(di) =
		s.display_items.iter().position(|item| matches!(item, whantv::DisplayItem::Single(m) if m.is_unread_divider))
	{
		s.display_items.remove(di);
		parts.conversations_parts.messages_list.delete(di as u32);
	}
	refresh_selection_list(s, parts);
}

/// Saves whatever is currently sitting in the composer into `draft_messages`
/// under the chat that's about to be left, if any - so switching away (to
/// another chat, or closing the conversation entirely) never silently
/// discards an unsent message. A no-op if the composer is empty or no chat
/// is currently open.
pub(crate) fn save_composer_draft(s: &mut AppState, parts: &WindowParts) {
	let Some(prev_jid) = s.selected_chat.clone() else { return };
	let text = parts.conversations_parts.message_field.get_value();
	let mentions = std::mem::take(&mut s.composer_mentions);
	crate::ui_commands::mentions::close(s, &parts.conversations_parts);
	if text.trim().is_empty() {
		s.draft_messages.remove(&prev_jid);
		s.draft_mentions.remove(&prev_jid);
	} else {
		s.draft_messages.insert(prev_jid.clone(), text);
		s.draft_mentions.insert(prev_jid, mentions);
	}
}

/// Drops a pending "reply to" quote and hides its Remove Quote button. A
/// quote belongs to the chat it was picked in: every send path reads
/// `pending_reply_id` against whatever chat is open *now*, so leaving it set
/// across a chat switch sent the next message in the other chat as a reply
/// to a message that isn't even there. Official WhatsApp cancels it too.
pub(crate) fn clear_pending_reply(s: &mut AppState, parts: &WindowParts) {
	s.pending_reply_id = None;
	s.pending_reply_sender = None;
	s.pending_reply_preview = None;
	parts.conversations_parts.remove_quote_btn.show(false);
}

/// Switches the conversation panel over to `jid`, requesting its message
/// history. Shared by opening a chat from the sidebar (`OpenConversation`)
/// and jumping straight to a phone number found in message text
/// (`OpenChatByPhone`) — the latter's jid may not be in `filtered_chats` yet.
pub(crate) fn open_chat_for_jid(jid: &str, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let mut s = state.borrow_mut();
	// An in-progress edit is tied to the message (and chat) it was started
	// from - switching away must drop that link rather than leaving
	// `editing_message_id` pointing at a message that isn't even on screen
	// anymore.
	if s.editing_message_id.take().is_some() {
		s.editing_message_original_text = None;
		parts.conversations_parts.cancel_edit_btn.show(false);
		parts.conversations_parts.add_attachment_btn.show(true);
		parts.conversations_parts.message_label.set_label("Type a message");
		// An edit-in-progress is not a new message someone was composing -
		// clear it rather than letting `save_composer_draft` below pick up
		// the message's own (edited) text as if it were an ordinary unsent
		// draft for the chat being left.
		parts.conversations_parts.message_field.set_value("");
	}
	save_composer_draft(&mut s, parts);
	// Leaving a chat with unsent text in the composer must not leave the
	// recipient staring at a "typing…" indicator that never clears - real
	// WhatsApp Web sends "paused" the moment you navigate away, not just
	// when the text is deleted or sent.
	// Only if we actually told that chat we were typing.
	if let Some(prev_jid) = s.composing.as_ref().map(|c| c.jid.clone()) {
		if prev_jid != jid {
			crate::ui_commands::messaging::send_chatstate(&mut s, &prev_jid, "paused", "");
		}
	}
	// Re-opening the same chat keeps the quote; any other chat drops it.
	if s.selected_chat.as_deref() != Some(jid) {
		clear_pending_reply(&mut s, parts);
	}
	s.selected_chat = Some(jid.to_string());
	// Mirrors official WhatsApp: opening a chat clears any of its
	// notifications still sitting in Action Center.
	if let Some(notifications) = s.active_notifications.remove(jid) {
		for notification in notifications {
			notification.close();
		}
	}
	let unread = s.unread_counts.get(jid).copied().unwrap_or(0);
	s.unread_divider_jid = if unread > 0 { Some(jid.to_string()) } else { None };
	s.unread_divider_count = unread;
	s.background_unread_message_ids.clear();
	s.initial_presence_received = false;
	// Releases the outgoing chat's messages, their display_items copies and the
	// info dialogs' media snapshots. `s.messages.clear()` alone used to leave
	// display_items holding a full second copy of the previous chat - both a
	// retention leak and a source of stale row lookups in the window between
	// the switch and the new conversation_messages response landing.
	crate::ui_commands::chat_list::release_conversation_buffers(&mut s, parts);
	s.loading_messages = false;
	s.no_more_messages = false;
	s.history_loading = false;
	// Each time a chat is opened, Home at its top may ask the phone for
	// older history once more (and at most once, until it's reopened) - a
	// chat marked out of history by an earlier visit gets that one try back.
	s.completed_history_chats.remove(jid);
	s.phone_history_pending_jid = None;
	s.phone_history_deadline = None;
	s.quiet_reload_jid = None;
	s.newer_loading = false;
	s.pending_attachments.clear();
	// Selection mode's row list is tied to the chat it was built from -
	// switching chats mid-selection would leave it pointing at messages that
	// no longer belong to the now-displayed conversation.
	if s.selection_mode {
		s.selection_mode = false;
		s.selected_message_ids.clear();
		s.selection_rows.clear();
		parts.conversations_parts.messages_select_list.show(false);
		parts.conversations_parts.selection_bar_panel.show(false);
		parts.conversations_parts.messages_list.show(true);
		parts.conversations_parts.message_field.show(true);
		parts.conversations_parts.add_attachment_btn.show(true);
		parts.conversations_parts.record_voice_btn.show(true);
	}
	parts.conversations_parts.conversation_panel.show(true);
	parts.conversations_parts.play_voice_btn.show(false);
	parts.conversations_parts.action_panel.show(false);

	let norm_jid = if let Some((user, server)) = jid.split_once('@') {
		let base = user.split('.').next().unwrap_or(user).split(':').next().unwrap_or(user);
		format!("{}@{}", base, server)
	} else {
		jid.to_string()
	};
	let name =
		s.contacts.get(jid).or_else(|| s.contacts.get(&norm_jid)).cloned().unwrap_or_else(|| display_name_for_jid(jid));
	let presence_label = s.presence_labels.get(jid).or_else(|| s.presence_labels.get(&norm_jid)).map(|s| s.as_str());
	let is_typing_active = s
		.typing_timestamps
		.get(jid)
		.or_else(|| s.typing_timestamps.get(&norm_jid))
		.map_or(false, |t| t.elapsed().as_secs() < 5);
	let kind = if is_typing_active {
		s.typing_announce_state.get(jid).or_else(|| s.typing_announce_state.get(&norm_jid)).copied().unwrap_or(1)
	} else {
		0
	};
	let label =
		crate::websocket_events::format_conv_data_label(jid, &name, &name, kind == 1, kind == 2, presence_label);
	parts.conversations_parts.conv_data_btn.set_label(&label);

	parts.attachment_dialog.attachment_info_label.set_label("");
	// The message list was already emptied by release_conversation_buffers
	// above; clearing it a second time is dead work on a control a screen
	// reader is watching.
	// A channel is read-only unless we're one of its admins: no composer.
	let can_write = !jid.ends_with("@newsletter") || crate::ui_commands::links::can_post_in_channel(&s, jid);
	parts.conversations_parts.message_field.show(can_write);
	parts.conversations_parts.add_attachment_btn.show(can_write);
	parts.conversations_parts.record_voice_btn.show(can_write);
	let can_call = crate::ui_commands::calls::can_call(&s, jid);
	parts.conversations_parts.place_call_btn.show(can_call);
	parts.conversations_parts.place_video_call_btn.show(can_call);
	parts.conversations_parts.top_btns_panel.layout();
	let draft = s.draft_messages.get(jid).cloned().unwrap_or_default();
	s.composer_mentions = s.draft_mentions.get(jid).cloned().unwrap_or_default();
	parts.conversations_parts.message_field.set_value(&draft);
	parts.conversations_parts.send_message_btn.show(!draft.trim().is_empty());
	parts.conversations_parts.panel.layout();
	// The whole chat (see `MESSAGE_PAGE_SIZE`). Opening onto a starred
	// message needs nothing extra: it's in there, and
	// `apply_conversation_messages` selects it via `search_target_id`.
	let payload = serde_json::json!({
		"action": "get_messages",
		"data": { "jid": jid, "limit": crate::MESSAGE_PAGE_SIZE, "offset": 0 }
	});
	let _ = s.ws_tx.send(payload.to_string());
	drop(s);
	crate::handle_ui_command(UiCommand::SubscribePresence(jid.to_string()), state, parts);
	crate::handle_ui_command(UiCommand::MarkRead { jid: jid.to_string(), message_ids: Vec::new() }, state, parts);
	parts.conversations_parts.messages_list.set_focus();
}
