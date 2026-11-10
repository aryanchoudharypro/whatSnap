use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState,
	ui::{
		dialogs::{schedule_message, scheduled_messages},
		window::WindowParts,
	},
};

fn chat_name(s: &AppState, jid: &str) -> String {
	s.contacts.get(jid).cloned().unwrap_or_else(|| jid.to_string())
}

fn announce(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, message: &str) {
	let list = state.borrow().scheduled_list.clone();
	match list {
		Some(list) => list.announce(message),
		None => crate::platform::announce::announce(parts.conversations_parts.messages_announcer, message),
	}
}

pub(crate) fn schedulable(s: &AppState, jid: &str) -> bool {
	let person_or_group = jid.ends_with("@s.whatsapp.net") || jid.ends_with("@lid") || jid.ends_with("@g.us");
	person_or_group && s.scheduled_messages_enabled
}

/// "Schedule Message...": asks the backend for the picker's days and window,
/// then shows the dialog (see `schedule_options_received`).
pub(crate) fn open_schedule_dialog(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	if crate::ui_commands::blocking::stop_if_cant_send(state, parts) {
		return;
	}
	let mut s = state.borrow_mut();
	let Some(jid) = s.selected_chat.clone() else { return };
	if !s.scheduled_messages_enabled {
		drop(s);
		let style = MessageDialogStyle::OK | MessageDialogStyle::IconWarning;
		MessageDialog::builder(
			&parts.frame,
			"WhatsApp hasn't turned on scheduling messages for your account yet.",
			"Schedule message",
		)
		.with_style(style)
		.build()
		.show_modal();
		announce(state, parts, "WhatsApp hasn't turned on scheduling messages for your account.");
		return;
	}
	if !schedulable(&s, &jid) {
		drop(s);
		let style = MessageDialogStyle::OK | MessageDialogStyle::IconWarning;
		MessageDialog::builder(
			&parts.frame,
			"Messages can only be scheduled in one-on-one chats and groups.",
			"Schedule message",
		)
		.with_style(style)
		.build()
		.show_modal();
		announce(state, parts, "Messages can only be scheduled in one-on-one chats and groups.");
		return;
	}
	let text = parts.conversations_parts.message_field.get_value();
	s.pending_schedule = Some((jid.clone(), text));
	let payload = serde_json::json!({ "action": "get_schedule_options", "data": { "jid": jid } });
	let _ = s.ws_tx.send(payload.to_string());
}

pub(crate) fn schedule_options_received(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	data: &serde_json::Value,
) {
	let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("").to_string();
	let text = {
		let mut s = state.borrow_mut();
		match s.pending_schedule.take() {
			Some((pending_jid, text)) if pending_jid == jid && s.selected_chat.as_deref() == Some(jid.as_str()) => text,
			_ => return,
		}
	};
	if !data.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true) {
		let style = MessageDialogStyle::OK | MessageDialogStyle::IconWarning;
		MessageDialog::builder(
			&parts.frame,
			"WhatsApp hasn't turned on scheduling messages for your account yet.",
			"Schedule message",
		)
		.with_style(style)
		.build()
		.show_modal();
		announce(state, parts, "WhatsApp hasn't turned on scheduling messages for your account.");
		return;
	}
	let options = schedule_message::ScheduleOptions::from_json(data);
	if options.days.is_empty() {
		announce(state, parts, "Couldn't work out when this message can be scheduled.");
		return;
	}
	let Some(choice) = schedule_message::show_schedule_dialog(&parts.frame, &options, &text) else {
		parts.conversations_parts.message_field.set_focus();
		return;
	};
	let mut s = state.borrow_mut();
	let mentions = crate::ui_commands::mentions::mentions_payload(&s, &choice.text);
	let payload = serde_json::json!({
		"action": "schedule_message",
		"data": { "jid": jid, "text": choice.text, "date": choice.date, "time": choice.time, "mentions": mentions }
	});
	let _ = s.ws_tx.send(payload.to_string());
	// The message box's text was what got scheduled: clear it like a send.
	if !text.is_empty() && s.selected_chat.as_deref() == Some(jid.as_str()) {
		s.draft_messages.remove(&jid);
		s.draft_mentions.remove(&jid);
		s.composer_mentions.clear();
		crate::ui_commands::mentions::close(&mut s, &parts.conversations_parts);
		parts.conversations_parts.message_field.set_value("");
		crate::ui::chat_ui::update_composer_buttons(&s, &parts.conversations_parts);
	}
	drop(s);
	parts.conversations_parts.message_field.set_focus();
	announce(state, parts, "Scheduling message...");
}

/// "Scheduled Messages...": the open chat's scheduled messages.
pub(crate) fn show_scheduled_messages(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let (jid, name, ws_tx) = {
		let s = state.borrow();
		let Some(jid) = s.selected_chat.clone() else { return };
		if !s.scheduled_messages_enabled {
			drop(s);
			let style = MessageDialogStyle::OK | MessageDialogStyle::IconWarning;
			MessageDialog::builder(
				&parts.frame,
				"WhatsApp hasn't turned on scheduling messages for your account yet.",
				"Scheduled messages",
			)
			.with_style(style)
			.build()
			.show_modal();
			announce(state, parts, "WhatsApp hasn't turned on scheduling messages for your account.");
			return;
		}
		if !schedulable(&s, &jid) {
			drop(s);
			let style = MessageDialogStyle::OK | MessageDialogStyle::IconWarning;
			MessageDialog::builder(
				&parts.frame,
				"Messages can only be scheduled in one-on-one chats and groups.",
				"Scheduled messages",
			)
			.with_style(style)
			.build()
			.show_modal();
			announce(state, parts, "This chat has no scheduled messages.");
			return;
		}
		let name = chat_name(&s, &jid);
		(jid, name, s.ws_tx.clone())
	};
	let state_store = state.clone();
	let store = move |list| state_store.borrow_mut().scheduled_list = list;
	scheduled_messages::show_scheduled_messages(&parts.frame, &name, &jid, ws_tx, store);
}

pub(crate) fn update_scheduled_button(s: &AppState, parts: &WindowParts, jid: &str) {
	let count = s.scheduled_counts.get(jid).copied().unwrap_or(0);
	let c = &parts.conversations_parts;
	if count > 0 && s.scheduled_messages_enabled {
		let label = if count == 1 { "1 scheduled".to_string() } else { format!("{count} scheduled") };
		c.scheduled_btn.set_label(&label);
		c.scheduled_btn.set_tooltip(&format!("{label} messages"));
		let text_ext = c.scheduled_btn.get_text_extent(&label);
		let line = c.scheduled_btn.get_char_height();
		c.scheduled_btn.set_min_size(Size::new((text_ext.width + line * 2 + 8).max(line * 5), line * 2 + 8));
		c.scheduled_btn.show(true);
	} else {
		c.scheduled_btn.show(false);
	}
	c.top_btns_panel.layout();
}

pub(crate) fn scheduled_messages_received(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	data: &serde_json::Value,
) {
	let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("").to_string();
	let pending = data.get("pending").and_then(|v| v.as_u64()).unwrap_or(0);
	if !jid.is_empty() {
		state.borrow_mut().scheduled_counts.insert(jid.clone(), pending);
		if state.borrow().selected_chat.as_deref() == Some(&jid) {
			update_scheduled_button(&state.borrow(), parts, &jid);
		}
	}
	let Some(list) = state.borrow().scheduled_list.clone() else { return };
	if list.jid != jid {
		return;
	}
	let entries: Vec<scheduled_messages::ScheduledEntry> = data
		.get("messages")
		.and_then(|m| m.as_array())
		.into_iter()
		.flatten()
		.filter_map(scheduled_messages::ScheduledEntry::from_json)
		.collect();
	let limit = data.get("limit").and_then(|v| v.as_u64()).unwrap_or(30);
	list.apply(&entries, pending, limit);
}

/// The backend's answer to schedule, unschedule or send now: said in the
/// list's live region when it's open, and an error is shown in a box too.
pub(crate) fn scheduled_result(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let ok = data.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
	let message = data.get("message").and_then(|m| m.as_str()).unwrap_or("").to_string();
	let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("").to_string();
	let action = data.get("action").and_then(|a| a.as_str()).unwrap_or("");
	let text = data.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string();
	if ok {
		if !message.is_empty() {
			announce(state, parts, &message);
		}
		return;
	}
	// Restore typed draft if scheduling failed so user text is never lost
	if action == "schedule" && !text.is_empty() && !jid.is_empty() {
		let mut s = state.borrow_mut();
		if s.selected_chat.as_deref() == Some(jid.as_str()) {
			parts.conversations_parts.message_field.set_value(&text);
			let end = text.chars().count() as i64;
			parts.conversations_parts.message_field.set_selection(end, end);
			crate::ui::chat_ui::update_composer_buttons(&s, &parts.conversations_parts);
		} else {
			s.draft_messages.insert(jid, text);
		}
	}
	let message = if message.is_empty() { "Something went wrong.".to_string() } else { message };
	let list = state.borrow().scheduled_list.clone();
	let style = MessageDialogStyle::OK | MessageDialogStyle::IconError;
	match list {
		Some(list) => {
			list.announce(&message);
			MessageDialog::builder(&list.dialog, &message, "Scheduled messages").with_style(style).build().show_modal();
		}
		None => {
			let owner = windows::Win32::Foundation::HWND(parts.frame.get_handle());
			crate::platform::focus::show_modal_and_refocus(owner, false, || {
				MessageDialog::builder(&parts.frame, &message, "Scheduled messages")
					.with_style(style)
					.build()
					.show_modal()
			});
		}
	}
}
