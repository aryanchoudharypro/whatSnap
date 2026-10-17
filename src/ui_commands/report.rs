//! Reporting, as in WhatsApp: a message (from its menu) or a chat, to
//! WhatsApp, optionally blocking the contact or exiting the group; and in a
//! group, sending a message to its admins to review.

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{AppState, ui::window::WindowParts};

fn chat_name(state: &AppState, jid: &str) -> String {
	state.contacts.get(jid).cloned().unwrap_or_else(|| jid.to_string())
}

/// Asks how to report, and sends the report. `message_id` is None for a
/// whole chat, which WhatsApp receives with the last message they sent.
fn confirm_and_report(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String, message_id: Option<String>) {
	if jid.ends_with("@newsletter") || jid.ends_with("@broadcast") {
		return;
	}
	let is_group = jid.ends_with("@g.us");
	let name = chat_name(&state.borrow(), &jid);
	let question = match (&message_id, is_group) {
		(Some(_), false) => {
			format!("Report this message to WhatsApp? WhatsApp receives this message. {name} won't be notified.")
		}
		(Some(_), true) => {
			"Report this message to WhatsApp? WhatsApp receives this message. The sender and the group won't be notified."
				.to_string()
		}
		(None, false) => format!(
			"Report {name} to WhatsApp? WhatsApp receives the last message they sent you. They won't be notified."
		),
		(None, true) => format!(
			"Report {name} to WhatsApp? WhatsApp receives the most recent message someone else sent here. The group won't be notified."
		),
	};
	let also = if is_group { "Report and exit group".to_string() } else { format!("Report and block {name}") };
	let choices: Vec<&str> = vec!["Report", also.as_str()];
	let dialog = SingleChoiceDialog::builder(&parts.frame, &question, "Report", &choices).build();
	if dialog.show_modal() != wxdragon::id::ID_OK {
		return;
	}
	let Some(choice) = dialog.get_string_selection() else { return };
	let extra = choice != "Report";
	let payload = serde_json::json!({
		"action": "report",
		"data": {
			"jid": jid,
			"message_id": message_id.unwrap_or_default(),
			"block": extra && !is_group,
			"leave": extra && is_group,
		}
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	crate::announce::announce(parts.conversations_parts.messages_announcer.clone(), "Reporting...");
}

/// "Report..." on a message in the open chat.
pub(crate) fn report_message(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, message_id: String) {
	let Some(jid) = state.borrow().selected_chat.clone() else {
		return;
	};
	confirm_and_report(state, parts, jid, Some(message_id));
}

/// "Report..." on a chat.
pub(crate) fn report_chat(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String) {
	confirm_and_report(state, parts, jid, None);
}

/// "Report to group admins" on a message in the open group.
pub(crate) fn report_to_admins(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, message_id: String) {
	let Some(jid) = state.borrow().selected_chat.clone().filter(|j| j.ends_with("@g.us")) else {
		return;
	};
	let confirm = MessageDialog::builder(
		&parts.frame,
		"Send this message to the group's admins to review? The sender won't be told who reported it.",
		"Report to group admins",
	)
	.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
	.build();
	if confirm.show_modal() != ID_YES {
		return;
	}
	let payload = serde_json::json!({
		"action": "report_to_admins",
		"data": { "jid": jid, "message_ids": [message_id] }
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	crate::announce::announce(parts.conversations_parts.messages_announcer.clone(), "Sending to the admins...");
}

/// The backend's answer to a report.
pub(crate) fn reported(parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let flag = |k: &str| data.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
	let mut text = if data.get("admins").is_some() { "Sent to the group's admins." } else { "Reported." }.to_string();
	if flag("blocked") {
		text.push_str(" Contact blocked.");
	}
	if flag("left") {
		text.push_str(" You left the group.");
	}
	crate::announce::announce(parts.conversations_parts.messages_announcer.clone(), &text);
}

/// Saves Group Info's moderation switches; the backend only sends the ones
/// that changed.
pub(crate) fn update_group_moderation(
	state: &Rc<RefCell<AppState>>,
	no_frequently_forwarded: bool,
	allow_admin_reports: bool,
	group_history: bool,
	admins_add_only: bool,
	approve_members: bool,
) {
	let s = state.borrow();
	let Some(jid) = s.selected_chat.clone().filter(|j| j.ends_with("@g.us")) else {
		return;
	};
	let payload = serde_json::json!({
		"action": "set_group_moderation",
		"data": {
			"jid": jid,
			"no_frequently_forwarded": no_frequently_forwarded,
			"allow_admin_reports": allow_admin_reports,
			"group_history": group_history,
			"admins_add_only": admins_add_only,
			"approve_members": approve_members,
		}
	});
	let _ = s.ws_tx.send(payload.to_string());
}

/// Group Info's "Reported messages...": asks the backend for the list.
pub(crate) fn request_reported_messages(state: &Rc<RefCell<AppState>>) {
	let s = state.borrow();
	let Some(jid) = s.selected_chat.clone().filter(|j| j.ends_with("@g.us")) else {
		return;
	};
	let payload = serde_json::json!({ "action": "get_reported_messages", "data": { "jid": jid } });
	let _ = s.ws_tx.send(payload.to_string());
}

/// What members reported to the group's admins, one line per message: who
/// sent it, what it says, and who reported it. Enter or "Go to message"
/// closes Group Info and opens the chat at that message.
pub(crate) fn show_reported_messages(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let reports = data.get("reports").and_then(|r| r.as_array()).cloned().unwrap_or_default();
	if reports.is_empty() {
		MessageDialog::builder(
			&parts.group_data_dialog.dialog,
			"No messages have been reported to the admins.",
			"Reported messages",
		)
		.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
		.build()
		.show_modal();
		return;
	}
	let str_of = |r: &serde_json::Value, k: &str| r.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
	let mut lines = Vec::with_capacity(reports.len());
	let mut ids = Vec::with_capacity(reports.len());
	for r in &reports {
		let reporters: Vec<String> = r
			.get("reporters")
			.and_then(|v| v.as_array())
			.map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
			.unwrap_or_default();
		let stored = r.get("stored").and_then(|v| v.as_bool()).unwrap_or(false);
		let text = if stored {
			whantv::friendly_preview_text(&str_of(r, "text"))
		} else {
			"A message that isn't on this computer".to_string()
		};
		let sender = str_of(r, "sender_name");
		let from = if sender.is_empty() { String::new() } else { format!("{sender}: ") };
		lines.push(format!("{from}{text}. Reported by {}", reporters.join(", ")));
		ids.push(stored.then(|| str_of(r, "message_id")));
	}

	let dialog = Dialog::builder(&parts.group_data_dialog.dialog, "Reported messages")
		.with_size(520, 400)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	let panel = Panel::builder(&dialog).build();
	let sizer = BoxSizer::builder(Orientation::Vertical).build();
	let label = StaticText::builder(&panel).with_label("Messages members reported to the admins").build();
	sizer.add(&label, 0, SizerFlag::All, 8);
	let list = ListBox::builder(&panel).with_choices(lines).build();
	list.set_selection(0_u32, true);
	sizer.add(&list, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 8);
	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let go_btn = Button::builder(&panel).with_id(ID_OK).with_label("Go to message").build();
	btn_sizer.add(&go_btn, 0, SizerFlag::Right, 8);
	btn_sizer.add_stretch_spacer(1);
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Close").build();
	btn_sizer.add(&close_btn, 0, SizerFlag::Right, 8);
	sizer.add_sizer(&btn_sizer, 0, SizerFlag::Expand | SizerFlag::All, 8);
	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let dlg_go = dialog.clone();
	go_btn.on_click(move |_| dlg_go.end_modal(ID_OK));
	let dlg_enter = dialog.clone();
	list.on_item_double_clicked(move |_| dlg_enter.end_modal(ID_OK));
	let dlg_close = dialog.clone();
	close_btn.on_click(move |_| dlg_close.end_modal(ID_CANCEL));
	list.set_focus();

	let go = dialog.show_modal() == ID_OK;
	let chosen = list.get_selection().and_then(|i| ids.get(i as usize).cloned().flatten());
	dialog.destroy();
	if !go {
		return;
	}
	let Some(message_id) = chosen else {
		crate::announce::announce(
			parts.conversations_parts.messages_announcer.clone(),
			"That message isn't on this computer, so it can't be shown.",
		);
		return;
	};
	parts.group_data_dialog.dialog.end_modal(ID_OK);
	crate::ui_commands::chat_list::jump_to_search_result(state, parts, message_id, String::new());
}
