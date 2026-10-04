//! Voting in polls and quizzes: `V` on a poll asks the backend for it, and
//! its answer (`poll_details`) opens the voting dialog here. `V` on an event
//! answers it instead (going, maybe, not going).

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState,
	ui::{theme, window::WindowParts},
};

/// `V` on a message: if it's a poll, fetch it to vote in; if it's an event,
/// fetch it to answer.
pub(crate) fn open_poll_vote(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, message_id: String) {
	let s = state.borrow();
	let Some(jid) = s.selected_chat.clone() else {
		return;
	};
	let Some(m) = s.messages.iter().find(|m| m.id == message_id && !m.is_deleted) else {
		return;
	};
	let action = if m.is_poll() {
		"get_poll"
	} else if m.is_event() {
		"get_event"
	} else {
		crate::announce::announce(parts.conversations_parts.messages_announcer.clone(), "Not a poll or event.");
		return;
	};
	let payload = serde_json::json!({
		"action": action,
		"data": { "jid": jid, "message_id": message_id }
	});
	let _ = s.ws_tx.send(payload.to_string());
}

/// The voting dialog: radio buttons when the poll takes one answer, check
/// boxes when it takes several, your current vote preselected. Vote sends it;
/// in a multiple-answer poll, clearing every box takes your vote back.
pub(crate) fn show_poll_vote_dialog(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let str_of = |k: &str| data.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
	let list_of = |k: &str| -> Vec<String> {
		data.get(k)
			.and_then(|v| v.as_array())
			.map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
			.unwrap_or_default()
	};
	let (jid, message_id, question) = (str_of("jid"), str_of("message_id"), str_of("question"));
	let options = list_of("options");
	let selected = list_of("selected");
	let multiple = data.get("multiple").and_then(|v| v.as_bool()).unwrap_or(true);
	let quiz = data.get("quiz").and_then(|v| v.as_bool()).unwrap_or(false);
	if options.is_empty() {
		return;
	}
	// A quiz is answered once, for good, as in WhatsApp.
	if quiz && !selected.is_empty() {
		crate::announce::announce(
			parts.conversations_parts.messages_announcer.clone(),
			&format!("You've already answered this quiz: {}.", selected.join(", ")),
		);
		return;
	}

	let dialog = Dialog::builder(&parts.frame, if quiz { "Answer quiz" } else { "Vote in poll" })
		.with_size(420, 380)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let hint = if quiz {
		"Quiz, choose one answer; you can't change it afterwards"
	} else if multiple {
		"Choose one or more answers"
	} else {
		"Choose one answer"
	};
	let question_label = StaticText::builder(&panel).with_label(&format!("{question} ({hint})")).build();
	question_label.set_font(&theme::Fonts::body());
	sizer.add(&question_label, 0, SizerFlag::Expand | SizerFlag::All, 10);

	let option_refs: Vec<&str> = options.iter().map(String::as_str).collect();
	let checks = multiple.then(|| {
		let list = CheckListBox::builder(&panel).build();
		list.set_font(&theme::Fonts::body());
		for (i, opt) in options.iter().enumerate() {
			list.append(opt);
			list.check(i as u32, selected.contains(opt));
		}
		list.set_selection(0, true);
		sizer.add(&list, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 10);
		list
	});
	let radios = (!multiple).then(|| {
		let radio = RadioBox::builder(&panel, &option_refs)
			.with_label(&question)
			.with_major_dimension(1)
			.with_style(RadioBoxStyle::SpecifyCols)
			.build();
		radio.set_font(&theme::Fonts::body());
		if let Some(i) = options.iter().position(|o| selected.contains(o)) {
			radio.set_selection(i as i32);
		}
		sizer.add(&radio, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 10);
		radio
	});

	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let vote_btn = Button::builder(&panel).with_id(ID_OK).with_label(if quiz { "Answer" } else { "Vote" }).build();
	theme::style_primary_button(&vote_btn);
	btn_sizer.add(&vote_btn, 0, SizerFlag::Right, 8);
	btn_sizer.add_stretch_spacer(1);
	let cancel_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Cancel").build();
	btn_sizer.add(&cancel_btn, 0, SizerFlag::Right, 8);
	sizer.add_sizer(&btn_sizer, 0, SizerFlag::Expand | SizerFlag::All, 10);
	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let dlg_ok = dialog.clone();
	vote_btn.on_click(move |_| dlg_ok.end_modal(ID_OK));
	let dlg_cancel = dialog.clone();
	cancel_btn.on_click(move |_| dlg_cancel.end_modal(ID_CANCEL));

	if let Some(ref list) = checks {
		list.set_focus();
	} else if let Some(ref radio) = radios {
		radio.set_focus();
	}

	let voted = dialog.show_modal() == ID_OK;
	let choice: Vec<String> = if let Some(ref list) = checks {
		(0..options.len()).filter(|&i| list.is_checked(i as u32)).map(|i| options[i].clone()).collect()
	} else if let Some(ref radio) = radios {
		let i = radio.get_selection();
		options.get(i.max(0) as usize).cloned().into_iter().collect()
	} else {
		Vec::new()
	};
	dialog.destroy();
	if !voted {
		return;
	}
	if choice.is_empty() && selected.is_empty() {
		crate::announce::announce(parts.conversations_parts.messages_announcer.clone(), "No answer chosen.");
		return;
	}
	let payload = serde_json::json!({
		"action": "send_poll_vote",
		"data": { "jid": jid, "message_id": message_id, "options": choice }
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	crate::announce::announce(parts.conversations_parts.messages_announcer.clone(), "Sending vote...");
}

/// Updates a poll's results line wherever that poll is shown.
pub(crate) fn update_poll_results(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	message_id: &str,
	results: &str,
) {
	let mut s = state.borrow_mut();
	let is_group = s.selected_chat.as_deref().is_some_and(|j| j.ends_with("@g.us"));
	let row = crate::message_display::update_loaded_message(&mut s, message_id, |m| {
		m.poll_results = results.to_string();
	})
	.and_then(|i| s.display_items.get(i).map(|item| (i, item.format_display(is_group))));
	if let Some((i, text)) = row {
		parts.conversations_parts.messages_list.set_string(i as u32, &text);
	}
}

/// Answering an event (`event_details` from the backend): going, maybe or
/// not going, your current answer preselected, and how many guests you bring
/// when the event allows guests.
pub(crate) fn show_event_rsvp_dialog(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	const ANSWERS: [(&str, &str); 3] = [("Going", "going"), ("Maybe", "maybe"), ("Not going", "not_going")];
	let str_of = |k: &str| data.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
	let (jid, message_id, name) = (str_of("jid"), str_of("message_id"), str_of("name"));
	let (when, location, current, results) =
		(str_of("when"), str_of("location"), str_of("response"), str_of("results"));
	let guests_allowed = data.get("extra_guests_allowed").and_then(|v| v.as_bool()).unwrap_or(false);
	let current_guests = data.get("extra_guests").and_then(|v| v.as_i64()).unwrap_or(0) as i32;

	let dialog = Dialog::builder(&parts.frame, "Answer event")
		.with_size(420, 360)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let mut about = name.clone();
	for detail in [&when, &location, &results] {
		if !detail.is_empty() {
			about.push_str(". ");
			about.push_str(detail);
		}
	}
	let about_label = StaticText::builder(&panel).with_label(&about).build();
	about_label.set_font(&theme::Fonts::body());
	sizer.add(&about_label, 0, SizerFlag::Expand | SizerFlag::All, 10);

	let labels: Vec<&str> = ANSWERS.iter().map(|(label, _)| *label).collect();
	let radio = RadioBox::builder(&panel, &labels)
		.with_label("Your answer")
		.with_major_dimension(1)
		.with_style(RadioBoxStyle::SpecifyCols)
		.build();
	radio.set_font(&theme::Fonts::body());
	if let Some(i) = ANSWERS.iter().position(|(_, key)| *key == current) {
		radio.set_selection(i as i32);
	}
	sizer.add(&radio, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 10);

	let guests = guests_allowed.then(|| {
		let label = StaticText::builder(&panel).with_label("Guests you're bringing").build();
		theme::style_muted_label(&label);
		sizer.add(&label, 0, SizerFlag::Left | SizerFlag::Top, 10);
		let spin = SpinCtrl::builder(&panel).with_range(0, 20).build();
		spin.set_value(current_guests.clamp(0, 20));
		spin.set_font(&theme::Fonts::body());
		sizer.add(&spin, 0, SizerFlag::Left | SizerFlag::Top, 10);
		spin
	});

	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let ok_btn = Button::builder(&panel).with_id(ID_OK).with_label("Send").build();
	theme::style_primary_button(&ok_btn);
	btn_sizer.add(&ok_btn, 0, SizerFlag::Right, 8);
	btn_sizer.add_stretch_spacer(1);
	let cancel_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Cancel").build();
	btn_sizer.add(&cancel_btn, 0, SizerFlag::Right, 8);
	sizer.add_sizer(&btn_sizer, 0, SizerFlag::Expand | SizerFlag::All, 10);
	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let dlg_ok = dialog.clone();
	ok_btn.on_click(move |_| dlg_ok.end_modal(ID_OK));
	let dlg_cancel = dialog.clone();
	cancel_btn.on_click(move |_| dlg_cancel.end_modal(ID_CANCEL));
	radio.set_focus();

	let answered = dialog.show_modal() == ID_OK;
	let choice = ANSWERS.get(radio.get_selection().max(0) as usize).map(|(_, key)| *key).unwrap_or("going");
	let extra_guests = if choice == "going" { guests.as_ref().map(|g| g.value()).unwrap_or(0) } else { 0 };
	dialog.destroy();
	if !answered {
		return;
	}
	let payload = serde_json::json!({
		"action": "send_event_response",
		"data": { "jid": jid, "message_id": message_id, "response": choice, "extra_guests": extra_guests }
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	crate::announce::announce(parts.conversations_parts.messages_announcer.clone(), "Sending your answer...");
}

/// "Event" in the attach menu: name, description, when, where, and whether
/// people may bring guests. The backend checks the date and time.
pub(crate) fn open_event_dialog(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	if crate::ui_commands::blocking::stop_if_blocked(state, parts) {
		return;
	}
	let Some(jid) = state.borrow().selected_chat.clone() else {
		return;
	};
	let dialog = Dialog::builder(&parts.frame, "Create event")
		.with_size(440, 520)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();
	let field = |label: &str, multi_line: bool| {
		let text = StaticText::builder(&panel).with_label(label).build();
		theme::style_muted_label(&text);
		sizer.add(&text, 0, SizerFlag::Left | SizerFlag::Top, 10);
		let ctrl = if multi_line {
			TextCtrl::builder(&panel).with_style(TextCtrlStyle::MultiLine).build()
		} else {
			TextCtrl::builder(&panel).build()
		};
		ctrl.set_font(&theme::Fonts::body());
		let grow = if multi_line { 1 } else { 0 };
		sizer.add(&ctrl, grow, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);
		ctrl
	};
	let name_field = field("Event name", false);
	let description_field = field("Description (optional)", true);
	let date_field = field("Date, like 2026-10-05 (optional)", false);
	let time_field = field("Start time, like 18:30 (optional)", false);
	let location_field = field("Location (optional)", false);
	let guests_check = CheckBox::builder(&panel).with_label("Allow guests").build();
	guests_check.set_font(&theme::Fonts::body());
	sizer.add(&guests_check, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let send_btn = Button::builder(&panel).with_label("Send").build();
	theme::style_primary_button(&send_btn);
	btn_sizer.add(&send_btn, 0, SizerFlag::Right, 8);
	btn_sizer.add_stretch_spacer(1);
	let cancel_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Cancel").build();
	btn_sizer.add(&cancel_btn, 0, SizerFlag::Right, 8);
	sizer.add_sizer(&btn_sizer, 0, SizerFlag::Expand | SizerFlag::All, 10);
	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();
	crate::ui::keys::escape_cancels_anywhere(dialog);

	let dlg_send = dialog.clone();
	let name_check = name_field;
	send_btn.on_click(move |_| {
		if name_check.get_value().trim().is_empty() {
			MessageDialog::builder(&dlg_send, "Type a name for the event.", "Event")
				.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
				.build()
				.show_modal();
			name_check.set_focus();
			return;
		}
		dlg_send.end_modal(ID_OK);
	});
	let dlg_cancel = dialog.clone();
	cancel_btn.on_click(move |_| dlg_cancel.end_modal(ID_CANCEL));
	name_field.set_focus();

	let send = dialog.show_modal() == ID_OK;
	let data = serde_json::json!({
		"jid": jid,
		"name": name_field.get_value().trim(),
		"description": description_field.get_value().trim(),
		"date": date_field.get_value().trim(),
		"time": time_field.get_value().trim(),
		"location": location_field.get_value().trim(),
		"extra_guests_allowed": guests_check.get_value(),
	});
	dialog.destroy();
	if !send {
		return;
	}
	let mut data = data;
	{
		let mut s = state.borrow_mut();
		data["reply_to_id"] = serde_json::json!(s.pending_reply_id.take().unwrap_or_default());
		data["reply_to_sender"] = serde_json::json!(s.pending_reply_sender.take().unwrap_or_default());
		s.pending_reply_preview = None;
	}
	parts.conversations_parts.remove_quote_btn.show(false);
	let payload = serde_json::json!({ "action": "send_event", "data": data });
	let _ = state.borrow().ws_tx.send(payload.to_string());
	crate::announce::announce(parts.conversations_parts.messages_announcer.clone(), "Sending event...");
}
