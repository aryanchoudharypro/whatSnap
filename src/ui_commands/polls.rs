//! Voting in polls: `V` on a poll asks the backend for it, and its answer
//! (`poll_details`) opens the voting dialog here.

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState,
	ui::{theme, window::WindowParts},
};

/// `V` on a message: if it's a poll, fetch it to vote in.
pub(crate) fn open_poll_vote(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, message_id: String) {
	let s = state.borrow();
	let Some(jid) = s.selected_chat.clone() else {
		return;
	};
	let is_poll = s.messages.iter().any(|m| m.id == message_id && m.is_poll() && !m.is_deleted);
	if !is_poll {
		crate::announce::announce(parts.conversations_parts.messages_announcer.clone(), "Not a poll.");
		return;
	}
	let payload = serde_json::json!({
		"action": "get_poll",
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
	if options.is_empty() {
		return;
	}

	let dialog = Dialog::builder(&parts.frame, "Vote in poll")
		.with_size(420, 380)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let hint = if multiple { "Choose one or more answers" } else { "Choose one answer" };
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
	let vote_btn = Button::builder(&panel).with_id(ID_OK).with_label("Vote").build();
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
