//! The "Send to" dialog: pick up to five chats and send them a message. Used
//! for WhatsApp links opened from elsewhere ("Share on WhatsApp" buttons
//! and whatsapp://send links), for files sent from Explorer's Send to menu,
//! and for sending a group's invite link.

use std::{cell::RefCell, collections::HashSet, rc::Rc};

use wxdragon::prelude::*;

use crate::ui::{
	dialog_look::{self, Field},
	theme,
};

/// WhatsApp lets one message go to at most five chats at once, when
/// forwarding or sharing into the app.
pub const MAX_RECIPIENTS: usize = 5;

/// One chat or contact the dialog offers.
#[derive(Clone)]
pub struct SendTarget {
	pub jid: String,
	pub label: String,
}

/// What the user chose in the dialog.
pub struct SendToChoice {
	pub jids: Vec<String>,
	pub text: String,
}

struct PickerState {
	all: Vec<SendTarget>,
	/// JIDs of the rows currently shown, in list order.
	shown: Vec<String>,
	/// In the order they were checked, so the send order is predictable.
	checked: Vec<String>,
}

/// Shows the dialog modally and returns what to send, or None if cancelled.
/// With `files`, the dialog sends those and the message is their optional
/// caption; without, the message is what's sent and can't be empty.
pub fn show_send_to_dialog(
	parent: &impl WxWidget,
	title: &str,
	targets: Vec<SendTarget>,
	initial_text: &str,
	files: &[String],
) -> Option<SendToChoice> {
	let dialog = Dialog::builder(parent, title)
		.with_size(440, 560)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	if !files.is_empty() {
		let names: Vec<String> = files
			.iter()
			.map(|p| std::path::Path::new(p).file_name().unwrap_or_default().to_string_lossy().to_string())
			.collect();
		let files_text = StaticText::builder(&panel).with_label(&format!("Sending: {}", names.join(", "))).build();
		files_text.wrap(400);
		sizer.add(&files_text, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);
	}

	let search_label = StaticText::builder(&panel).with_label("Search chats and contacts").build();
	theme::style_muted_label(&search_label);
	sizer.add(&search_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let search_field = TextCtrl::builder(&panel).with_style(dialog_look::field_style(TextCtrlStyle::Default)).build();
	dialog_look::add_field(&sizer, &search_field, Field::Search, 0);

	let list_label = StaticText::builder(&panel).with_label(&format!("Send to (up to {})", MAX_RECIPIENTS)).build();
	theme::style_muted_label(&list_label);
	sizer.add(&list_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let list = CheckListBox::builder(&panel).build();
	list.set_font(&theme::Fonts::body());
	sizer.add(&list, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	// Checking someone and then searching again hides them from the list, so
	// a running total confirms the choice is still there.
	let announcer = StaticText::builder(&panel).with_label("").build();
	live_region::set_live_region(&announcer);
	theme::style_muted_label(&announcer);
	sizer.add(&announcer, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let message_label =
		StaticText::builder(&panel).with_label(if files.is_empty() { "Message" } else { "Caption (optional)" }).build();
	theme::style_muted_label(&message_label);
	sizer.add(&message_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let message_field =
		TextCtrl::builder(&panel).with_style(dialog_look::field_style(TextCtrlStyle::MultiLine)).build();
	message_field.set_value(initial_text);
	message_field.set_min_size(Size::new(-1, 90));
	dialog_look::add_field(&sizer, &message_field, Field::Box, 0);

	let send_btn = Button::builder(&panel).with_id(ID_OK).with_label("Send").build();
	dialog_look::pill_button(&send_btn, true, theme::Palette::CONTENT_BG);
	let cancel_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Cancel").build();
	dialog_look::pill_button(&cancel_btn, false, theme::Palette::CONTENT_BG);
	dialog_look::button_row(&sizer, &[&send_btn, &cancel_btn]);
	panel.set_sizer(sizer, true);
	dialog_look::paint_fields(&panel, &[(search_field, Field::Search), (message_field, Field::Box)]);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let state = Rc::new(RefCell::new(PickerState {
		shown: targets.iter().map(|t| t.jid.clone()).collect(),
		all: targets,
		checked: Vec::new(),
	}));
	let state_rows = state.clone();
	dialog_look::contact_check_rows(&list, move |index| {
		state_rows.try_borrow().ok().and_then(|s| s.shown.get(index).cloned())
	});
	{
		let s = state.borrow();
		for target in &s.all {
			list.append(&target.label);
		}
		if !s.all.is_empty() {
			list.set_selection(0, true);
		}
	}

	// Rebuilt only once typing pauses: rebuilding a check list on every
	// keystroke leaves NVDA querying rows that no longer exist (see the New
	// Group dialog's search in window.rs).
	let search_timer = Rc::new(Timer::new(&dialog));
	let state_search = state.clone();
	let announcer_search = announcer;
	search_timer.on_tick(move |_| {
		let query = search_field.get_value().to_lowercase().trim().to_string();
		let mut s = state_search.borrow_mut();
		let shown: Vec<SendTarget> =
			s.all.iter().filter(|t| query.is_empty() || t.label.to_lowercase().contains(&query)).cloned().collect();
		let checked: HashSet<String> = s.checked.iter().cloned().collect();
		list.clear();
		for target in &shown {
			list.append(&target.label);
		}
		for (idx, target) in shown.iter().enumerate() {
			if checked.contains(&target.jid) {
				list.check(idx as u32, true);
			}
		}
		if !shown.is_empty() {
			list.set_selection(0, true);
		}
		s.shown = shown.into_iter().map(|t| t.jid).collect();
		crate::announce::announce(
			announcer_search,
			&format!("{} shown. {}", s.shown.len(), selected_summary(s.checked.len())),
		);
	});
	let timer_for_typing = search_timer.clone();
	search_field.on_text_changed(move |_| {
		timer_for_typing.stop();
		timer_for_typing.start(250, true);
	});

	let state_toggle = state.clone();
	let announcer_toggle = announcer;
	list.on_toggled(move |event| {
		let Some(idx) = event.get_selection() else { return };
		let mut s = state_toggle.borrow_mut();
		let Some(jid) = s.shown.get(idx as usize).cloned() else { return };
		let label = list.get_string(idx as usize).unwrap_or_default();
		if list.is_checked(idx) {
			if s.checked.len() >= MAX_RECIPIENTS {
				list.check(idx, false);
				crate::announce::announce(
					announcer_toggle,
					&format!("You can only send to {} chats at once. {} not selected.", MAX_RECIPIENTS, label),
				);
				return;
			}
			if !s.checked.contains(&jid) {
				s.checked.push(jid);
			}
			crate::announce::announce(
				announcer_toggle,
				&format!("{}: selected. {}", label, selected_summary(s.checked.len())),
			);
		} else {
			s.checked.retain(|j| *j != jid);
			crate::announce::announce(
				announcer_toggle,
				&format!("{}: not selected. {}", label, selected_summary(s.checked.len())),
			);
		}
	});

	let escape_closes = crate::ui::keys::escape_cancels(dialog);
	search_field.on_key_down(escape_closes);
	list.on_key_down(escape_closes);

	let caption_only = !files.is_empty();
	let state_send = state.clone();
	let announcer_send = announcer;
	send_btn.on_click(move |_| {
		let s = state_send.borrow();
		if s.checked.is_empty() {
			crate::announce::announce(announcer_send, "Choose at least one chat to send to.");
			return;
		}
		if !caption_only && message_field.get_value().trim().is_empty() {
			crate::announce::announce(announcer_send, "The message is empty.");
			return;
		}
		dialog.end_modal(ID_OK);
	});
	cancel_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));

	search_field.set_focus();
	let result = dialog.show_modal();
	let choice = (result == ID_OK)
		.then(|| SendToChoice { jids: state.borrow().checked.clone(), text: message_field.get_value() });
	search_timer.stop();
	// A dialog built per use must be destroyed, or it and every closure bound
	// to it live on hidden.
	dialog.destroy();
	choice
}

fn selected_summary(count: usize) -> String {
	match count {
		0 => "No chats selected.".to_string(),
		1 => "1 chat selected.".to_string(),
		n => format!("{} chats selected.", n),
	}
}
