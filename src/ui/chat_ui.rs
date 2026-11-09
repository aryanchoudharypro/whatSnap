use std::{
	cell::RefCell,
	io::{BufRead, BufReader, Write},
	rc::Rc,
	thread,
};

use wxdragon::prelude::*;

use crate::{
	AppState, UiCommand,
	ui::{
		contact_picker::{display_name_for_jid, refresh_selection_list},
		window::WindowParts,
	},
};

/// Shows a plain desktop notification (the catch-up summary and incoming
/// calls, which have no chat to file under). Must be called on the wx
/// main/GUI thread - wx (and the underlying WinRT toast COM calls) aren't
/// thread-safe. Message toasts go through [`show_chat_notification`].
pub(crate) fn show_toast_notification(title: &str, body: &str) {
	static LAST_TOAST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
	let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
	let last = LAST_TOAST.swap(now, std::sync::atomic::Ordering::Relaxed);
	if now - last < 1000 {
		return;
	}

	match wxdragon::prelude::NotificationMessage::builder()
		.with_title(title)
		.with_message(body)
		.with_generic(false)
		.build()
	{
		Ok(notification) => {
			notification.show(wxdragon::prelude::TIMEOUT_AUTO);
		}
		Err(err) => {
			eprintln!("[NOTIFY] Failed to build notification: {:?}", err);
		}
	}
}

const CATCH_UP_QUIET: std::time::Duration = std::time::Duration::from_secs(2);

pub(crate) fn record_catch_up_alert(s: &mut AppState, jid: &str, missed_call: bool, alert: whatsnap::IncomingAlert) {
	if missed_call {
		s.catch_up_alerts.record_missed_call(jid, alert);
	} else {
		s.catch_up_alerts.record_message(jid, alert);
	}
	if !s.catch_up_alerts.is_empty() {
		s.catch_up_last_at = Some(std::time::Instant::now());
	}
}

pub(crate) fn flush_catch_up_alerts(s: &mut AppState) {
	match s.catch_up_last_at {
		Some(last) if last.elapsed() >= CATCH_UP_QUIET => {}
		_ => return,
	}
	s.catch_up_last_at = None;
	let tally = std::mem::take(&mut s.catch_up_alerts);
	if tally.is_empty() {
		return;
	}
	if tally.sound() {
		crate::audio::play_message_tone(
			s.settings.message_tone,
			&s.settings.message_tone_file,
			s.settings.sounds_enabled,
			s.settings.output_device_name.clone(),
		);
	}
	if tally.toast() {
		show_toast_notification(whatsnap::TRAY_APP_NAME, &tally.summary());
	}
}

pub(crate) fn show_chat_notification(jid: &str, title: &str, body: &str, is_locked: bool) {
	show_chat_notification_for(jid, title, body, None, is_locked);
}

/// A chat notification for a linked account that isn't the one on screen.
pub(crate) fn show_chat_notification_for(jid: &str, title: &str, body: &str, account: Option<&str>, is_locked: bool) {
	if let Err(err) = crate::platform::toast::show_chat_toast(jid, title, body, account, is_locked) {
		eprintln!("[NOTIFY] Chat toast failed, showing a plain notification: {err:?}");
		show_toast_notification(title, body);
	}
}

pub(crate) fn clear_chat_notifications(jid: &str) {
	crate::platform::toast::clear_chat_toasts(jid);
}

pub(crate) fn request_backend_shutdown(pipe_id: &str) {
	let Ok(mut socket) = crate::backend::pipe::PipeStream::connect(pipe_id) else {
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

pub(crate) fn clear_unread_divider_for_open_chat(s: &mut AppState, parts: &WindowParts, jid: &str) {
	if s.unread_divider_jid.as_deref() != Some(jid) {
		return;
	}
	s.unread_divider_jid = None;
	s.unread_divider_count = 0;
	s.background_unread_message_ids.clear();
	s.messages.retain(|m| !m.is_unread_divider);
	if let Some(di) =
		s.display_items.iter().position(|item| matches!(item, whatsnap::DisplayItem::Single(m) if m.is_unread_divider))
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

pub(crate) fn update_composer_buttons(s: &AppState, cp: &crate::ui::conversations::ConversationsPanelParts) {
	let Some(jid) = s.selected_chat.as_deref() else { return };
	let is_blocked = crate::ui_commands::blocking::is_blocked(s, jid);
	let is_channel_readonly = jid.ends_with("@newsletter") && !crate::ui_commands::links::can_post_in_channel(s, jid);
	let is_group_not_member = jid.ends_with("@g.us") && s.not_member_jids.contains(jid);
	let is_group_admins_only =
		jid.ends_with("@g.us") && s.admins_only_jids.contains(jid) && !s.admin_group_jids.contains(jid);

	let can_write = !is_blocked && !is_channel_readonly && !is_group_not_member && !is_group_admins_only;
	if !can_write {
		let mut changed = false;
		if is_blocked {
			if !cp.blocked_banner_btn.is_shown() {
				cp.blocked_banner_btn.show(true);
				changed = true;
			}
			if cp.message_label.is_shown() {
				cp.message_label.show(false);
				changed = true;
			}
		} else {
			if cp.blocked_banner_btn.is_shown() {
				cp.blocked_banner_btn.show(false);
				changed = true;
			}
			if !cp.message_label.is_shown() {
				cp.message_label.show(true);
				changed = true;
			}
			let label = if is_channel_readonly {
				"Channels are read-only. Only admins can post."
			} else if is_group_not_member {
				"You can't send messages to this group because you're no longer a member."
			} else if is_group_admins_only {
				"Only admins can send messages to this group."
			} else {
				"You cannot send messages to this chat."
			};
			if cp.message_label.get_label() != label {
				cp.message_label.set_label(label);
			}
		}
		if cp.message_field.is_shown() {
			cp.message_field.show(false);
			changed = true;
		}
		if cp.add_attachment_btn.is_shown() {
			cp.add_attachment_btn.show(false);
			changed = true;
		}
		if cp.emoji_btn.is_shown() {
			cp.emoji_btn.show(false);
			changed = true;
		}
		if cp.send_message_btn.is_shown() {
			cp.send_message_btn.show(false);
			changed = true;
		}
		if cp.record_voice_btn.is_shown() {
			cp.record_voice_btn.show(false);
			changed = true;
		}
		if cp.cancel_edit_btn.is_shown() {
			cp.cancel_edit_btn.show(false);
			changed = true;
		}
		if cp.remove_quote_btn.is_shown() {
			cp.remove_quote_btn.show(false);
			changed = true;
		}
		if changed {
			cp.conversation_panel.layout();
		}
		return;
	}

	let is_editing = s.editing_message_id.is_some();
	let has_text = !cp.message_field.get_value().trim().is_empty();
	let mut changed = false;

	if cp.blocked_banner_btn.is_shown() {
		cp.blocked_banner_btn.show(false);
		changed = true;
	}
	if !cp.message_label.is_shown() {
		cp.message_label.show(true);
		changed = true;
	}
	if !cp.message_field.is_shown() {
		cp.message_field.show(true);
		changed = true;
	}
	if !cp.emoji_btn.is_shown() {
		cp.emoji_btn.show(true);
		changed = true;
	}

	if is_editing {
		if cp.add_attachment_btn.is_shown() {
			cp.add_attachment_btn.show(false);
			changed = true;
		}
		if !cp.cancel_edit_btn.is_shown() {
			cp.cancel_edit_btn.show(true);
			changed = true;
		}
		if cp.remove_quote_btn.is_shown() {
			cp.remove_quote_btn.show(false);
			changed = true;
		}
		if cp.record_voice_btn.is_shown() {
			cp.record_voice_btn.show(false);
			changed = true;
		}
		if cp.send_message_btn.is_shown() != has_text {
			cp.send_message_btn.show(has_text);
			changed = true;
		}
		if cp.message_label.get_label() != "Edit message" {
			cp.message_label.set_label("Edit message");
		}
	} else {
		if !cp.add_attachment_btn.is_shown() {
			cp.add_attachment_btn.show(true);
			changed = true;
		}
		if cp.cancel_edit_btn.is_shown() {
			cp.cancel_edit_btn.show(false);
			changed = true;
		}
		let show_quote = s.pending_reply_id.is_some();
		if cp.remove_quote_btn.is_shown() != show_quote {
			cp.remove_quote_btn.show(show_quote);
			changed = true;
		}
		if cp.send_message_btn.is_shown() != has_text {
			cp.send_message_btn.show(has_text);
			changed = true;
		}
		let show_voice = !has_text;
		if cp.record_voice_btn.is_shown() != show_voice {
			cp.record_voice_btn.show(show_voice);
			changed = true;
		}
		// The box is named "Replying to Name" only: the quoted text was
		// already announced, and a long message or link as the field's name
		// is read out every time it gets focus.
		let target_label = if let Some(ref preview) = s.pending_reply_preview {
			preview.split_once(": ").map_or(preview.as_str(), |(who, _)| who)
		} else {
			group_send_block_reason(s, jid).unwrap_or(if jid.ends_with("@newsletter") {
				"Type an update"
			} else {
				"Type a message"
			})
		};
		if cp.message_label.get_label() != target_label {
			cp.message_label.set_label(target_label);
		}
	}
	if changed {
		cp.conversation_panel.layout();
	}
}

/// What "Replying to Name: ..." says about the quoted message: its first line,
/// cut at a word near 100 characters, like the short quote WhatsApp Web shows.
/// A long message or link isn't read out in full.
pub(crate) fn reply_snippet(text: &str) -> String {
	const MAX: usize = 100;
	let friendly = whatsnap::friendly_preview_text(text);
	let first = friendly.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
	let more_lines = friendly.trim().lines().count() > 1;
	if first.chars().count() <= MAX {
		return if more_lines { format!("{first}…") } else { first.to_string() };
	}
	let cut: String = first.chars().take(MAX).collect();
	let cut = match cut.rfind(' ') {
		Some(i) if i > MAX / 2 => &cut[..i],
		_ => cut.as_str(),
	};
	format!("{}…", cut.trim_end())
}

pub(crate) fn clear_pending_reply(s: &mut AppState, parts: &WindowParts) {
	s.pending_reply_id = None;
	s.pending_reply_sender = None;
	s.pending_reply_preview = None;
	update_composer_buttons(s, &parts.conversations_parts);
}

pub(crate) fn chat_status_label(s: &AppState, jid: &str) -> String {
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
		.is_some_and(|t| t.elapsed().as_secs() < 5);
	let kind = if is_typing_active {
		s.typing_announce_state.get(jid).or_else(|| s.typing_announce_state.get(&norm_jid)).copied().unwrap_or(1)
	} else {
		0
	};
	crate::backend::events::format_conv_data_label(jid, &name, &name, kind == 1, kind == 2, presence_label)
}

pub(crate) fn open_chat_for_jid(jid: &str, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	{
		let mut s = state.borrow_mut();
		if s.locked_jids.contains(jid) && !s.chats_unlocked {
			s.pending_locked_chat = Some(jid.to_string());
			drop(s);
			crate::ui_commands::chat_list::prompt_locked_chats(state, parts);
			return;
		}
	}
	let mut s = state.borrow_mut();
	// An in-progress edit is tied to the message (and chat) it was started
	// from - switching away must drop that link rather than leaving
	// `editing_message_id` pointing at a message that isn't even on screen
	// anymore.
	if s.editing_message_id.take().is_some() {
		s.editing_message_original_text = None;
		parts.conversations_parts.cancel_edit_btn.show(false);
		parts.conversations_parts.add_attachment_btn.show(true);
		parts.conversations_parts.emoji_btn.show(true);
		parts.conversations_parts.message_label.set_label("Type a message");
		parts.conversations_parts.message_field.set_value("");
	}
	save_composer_draft(&mut s, parts);
	// Leaving a chat with unsent text in the composer must not leave the
	// recipient staring at a "typing…" indicator that never clears - real
	// WhatsApp Web sends "paused" the moment you navigate away, not just
	// when the text is deleted or sent.
	// Only if we actually told that chat we were typing.
	if let Some(prev_jid) = s.composing.as_ref().map(|c| c.jid.clone())
		&& prev_jid != jid
	{
		crate::ui_commands::messaging::send_chatstate(&mut s, &prev_jid, "paused", "");
	}
	if s.selected_chat.as_deref() != Some(jid) {
		clear_pending_reply(&mut s, parts);
		crate::ui_commands::voice::stop_voice_message(&mut s, parts);
	}
	s.selected_chat = Some(jid.to_string());
	crate::ui::window::refresh_menu_bar(&s, &parts.frame);
	clear_chat_notifications(jid);
	let unread = s.unread_counts.get(jid).copied().unwrap_or(0);
	s.unread_divider_jid = if unread > 0 { Some(jid.to_string()) } else { None };
	s.unread_divider_count = unread;
	s.background_unread_message_ids.clear();
	s.initial_presence_received = false;
	crate::ui_commands::chat_list::release_conversation_buffers(&mut s, parts);
	s.loading_messages = false;
	s.no_more_messages = false;
	s.history_loading = false;
	s.phone_history_pending_jid = None;
	s.phone_history_deadline = None;
	s.history_start_chats.remove(jid);
	s.quiet_reload_jid = None;
	s.newer_loading = false;
	s.older_unloaded = 0;
	s.newer_unloaded = 0;
	s.land_on_first = false;
	s.pending_attachments.clear();
	if s.selection_mode {
		s.selection_mode = false;
		s.selected_message_ids.clear();
		s.selection_rows.clear();
		parts.conversations_parts.messages_select_list.show(false);
		parts.conversations_parts.selection_bar_panel.show(false);
		parts.conversations_parts.messages_list.show(true);
	}
	parts.conversations_parts.conversation_panel.show(true);
	parts.conversations_parts.play_voice_btn.show(false);
	parts.conversations_parts.action_panel.show(false);

	crate::ui::owner_draw::set_header_chat(jid);
	parts.conversations_parts.conv_data_btn.set_label(&chat_status_label(&s, jid));

	parts.attachment_dialog.attachment_info_label.set_label("");
	// The message list was already emptied by release_conversation_buffers
	// above; clearing it a second time is dead work on a control a screen
	// reader is watching.
	crate::ui_commands::calls::apply_call_buttons(&s, parts, jid);
	apply_composer_access(&s, parts, jid);
	parts.conversations_parts.top_btns_panel.layout();
	let draft = s.draft_messages.get(jid).cloned().unwrap_or_default();
	s.composer_mentions = s.draft_mentions.get(jid).cloned().unwrap_or_default();
	parts.conversations_parts.message_field.set_value(&draft);
	update_composer_buttons(&s, &parts.conversations_parts);
	parts.conversations_parts.panel.layout();
	let payload = match s.search_target_id.clone() {
		Some(target) => crate::ui_commands::chat_list::around_request(jid, &target, ""),
		None => {
			let limit = (unread as usize + 50).clamp(crate::MESSAGE_PAGE_SIZE as usize, crate::MAX_LOADED_MESSAGES);
			serde_json::json!({
				"action": "get_messages",
				"data": { "jid": jid, "limit": limit, "offset": 0 }
			})
		}
	};
	let _ = s.ws_tx.send(payload.to_string());
	drop(s);
	crate::handle_ui_command(UiCommand::SubscribePresence(jid.to_string()), state, parts);
	crate::handle_ui_command(UiCommand::MarkRead { jid: jid.to_string(), message_ids: Vec::new() }, state, parts);
	parts.conversations_parts.messages_list.set_focus();
}

pub(crate) fn group_send_block_reason(s: &AppState, jid: &str) -> Option<&'static str> {
	if !jid.ends_with("@g.us") {
		None
	} else if s.not_member_jids.contains(jid) {
		Some("You can't send messages to this group because you're no longer a member")
	} else if s.admins_only_jids.contains(jid) && !s.admin_group_jids.contains(jid) {
		Some("Only admins can send messages to this group")
	} else {
		None
	}
}

/// Disables the composer for a chat that can't take messages, with the
/// reason as the message field's label so a screen reader says why, and
/// enables it again everywhere else.
pub(crate) fn apply_composer_access(s: &AppState, parts: &WindowParts, jid: &str) {
	let is_blocked = crate::ui_commands::blocking::is_blocked(s, jid);
	let is_channel_readonly = jid.ends_with("@newsletter") && !crate::ui_commands::links::can_post_in_channel(s, jid);
	let reason = if is_blocked {
		Some("You blocked this contact. Click to unblock.")
	} else if is_channel_readonly {
		Some("Channels are read-only. Only admins can post updates.")
	} else {
		group_send_block_reason(s, jid)
	};
	let enabled = reason.is_none();
	let cp = &parts.conversations_parts;
	cp.message_field.enable(enabled);
	cp.send_message_btn.enable(enabled);
	cp.add_attachment_btn.enable(enabled);
	cp.emoji_btn.enable(enabled);
	cp.record_voice_btn.enable(enabled);
	if is_blocked {
		cp.blocked_banner_btn.show(true);
		cp.message_field.show(false);
		cp.message_label.show(false);
		cp.add_attachment_btn.show(false);
		cp.emoji_btn.show(false);
		cp.send_message_btn.show(false);
		cp.record_voice_btn.show(false);
	} else {
		cp.blocked_banner_btn.show(false);
		if s.editing_message_id.is_none() && s.pending_reply_preview.is_none() {
			let label =
				reason.unwrap_or(if jid.ends_with("@newsletter") { "Type an update" } else { "Type a message" });
			if cp.message_label.get_label() != label {
				cp.message_label.set_label(label);
			}
		}
	}
}
