//! Account, Accounts...: the linked WhatsApp accounts, to switch between,
//! add, rename and remove, and how each alerts while it isn't on screen.

use std::{cell::Cell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	platform::accounts::{CallAlerts, MessageAlerts},
	ui::{dialog_look, theme},
};

/// One row of the list.
pub struct AccountRow {
	pub id: String,
	pub label: String,
}

/// What the user chose. The dialog closes for each, and the caller acts on it.
pub enum AccountsAction {
	Switch(String),
	Add,
	Rename(String),
	Remove(String),
	Notifications(String),
}

pub fn show(parent: &impl WxWidget, rows: &[AccountRow], active: &str, focus: Option<&str>) -> Option<AccountsAction> {
	let dialog = Dialog::builder(parent, crate::i18n::tr("Accounts"))
		.with_size(460, 400)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	// The label goes right before the list, so the list is named by it.
	let list_label = StaticText::builder(&panel).with_label(crate::i18n::tr("Linked accounts")).build();
	theme::style_muted_label(&list_label);
	sizer.add(&list_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let list = ListBox::builder(&panel).build();
	list.set_font(&theme::Fonts::body());
	sizer.add(&list, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let switch_btn = Button::builder(&panel).with_id(ID_OK).with_label(crate::i18n::tr("&Switch")).build();
	theme::style_primary_button(&switch_btn);
	let add_btn = Button::builder(&panel).with_label(crate::i18n::tr("&Add Account...")).build();
	let rename_btn = Button::builder(&panel).with_label(crate::i18n::tr("Re&name...")).build();
	let notify_btn = Button::builder(&panel).with_label(crate::i18n::tr("N&otifications...")).build();
	let remove_btn = Button::builder(&panel).with_label(crate::i18n::tr("&Remove...")).build();
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label(crate::i18n::tr("Close")).build();
	for btn in [&switch_btn, &add_btn, &rename_btn, &notify_btn, &remove_btn] {
		btn_sizer.add(btn, 0, SizerFlag::Right, 8);
	}
	btn_sizer.add_stretch_spacer(1);
	btn_sizer.add(&close_btn, 0, SizerFlag::empty(), 0);
	sizer.add_sizer(&btn_sizer, 0, SizerFlag::Expand | SizerFlag::All, 10);
	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	for row in rows {
		list.append(&row.label);
	}
	let start = focus
		.and_then(|id| rows.iter().position(|r| r.id == id))
		.or_else(|| rows.iter().position(|r| r.id == active))
		.unwrap_or(0);
	if !rows.is_empty() {
		list.set_selection(start as u32, true);
	}

	let ids: Rc<Vec<String>> = Rc::new(rows.iter().map(|r| r.id.clone()).collect());
	// Switch is only there for an account that isn't on screen, so Tab never
	// stops on a button that would do nothing.
	let update_switch = {
		let ids = ids.clone();
		let active = active.to_string();
		move || {
			let other = list.get_selection().and_then(|i| ids.get(i as usize)).is_some_and(|id| *id != active);
			if switch_btn.is_shown() != other {
				switch_btn.show(other);
				panel.layout();
			}
		}
	};
	update_switch();
	list.on_selection_changed(move |_| update_switch());
	// 0 none, 1 switch, 2 add, 3 rename, 4 remove, 5 notifications
	let choice = Rc::new(Cell::new(0u8));
	let chosen_index = Rc::new(Cell::new(None::<usize>));

	let pick = {
		let choice = choice.clone();
		let chosen_index = chosen_index.clone();
		move |what: u8| {
			chosen_index.set(list.get_selection().map(|i| i as usize));
			choice.set(what);
			dialog.end_modal(ID_OK);
		}
	};
	let p = pick.clone();
	switch_btn.on_click(move |_| p(1));
	let p = pick.clone();
	add_btn.on_click(move |_| p(2));
	let p = pick.clone();
	rename_btn.on_click(move |_| p(3));
	let p = pick.clone();
	remove_btn.on_click(move |_| p(4));
	let p = pick.clone();
	notify_btn.on_click(move |_| p(5));
	close_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));
	let p = pick.clone();
	// The dialog may have been opened with Enter on its menu item; the release
	// of that Enter lands in the list and must not press Switch.
	crate::ui::keys::on_enter(&list, Rc::new(Cell::new(true)), move || p(1));
	crate::ui::keys::escape_cancels_anywhere(dialog);

	list.set_focus();
	let result = dialog.show_modal();
	dialog.destroy();
	if result != ID_OK {
		return None;
	}
	let id = chosen_index.get().and_then(|i| ids.get(i).cloned());
	match (choice.get(), id) {
		(2, _) => Some(AccountsAction::Add),
		(1, Some(id)) if id != active => Some(AccountsAction::Switch(id)),
		(3, Some(id)) => Some(AccountsAction::Rename(id)),
		(4, Some(id)) => Some(AccountsAction::Remove(id)),
		(5, Some(id)) => Some(AccountsAction::Notifications(id)),
		_ => None,
	}
}

const MESSAGE_CHOICES: [(&str, MessageAlerts); 4] = [
	("Notification and sound", MessageAlerts::Full),
	("Notification only, no sound", MessageAlerts::NoSound),
	("Sound only, no notification", MessageAlerts::SoundOnly),
	("Off", MessageAlerts::Off),
];

const CALL_CHOICES: [(&str, CallAlerts); 3] = [
	("Switch to this account and ring", CallAlerts::Switch),
	("Notification only", CallAlerts::Notify),
	("Off", CallAlerts::Off),
];

/// Asks how an account alerts while another account is on screen.
pub fn ask_notifications(
	parent: &impl WxWidget,
	account: &str,
	messages: MessageAlerts,
	calls: CallAlerts,
) -> Option<(MessageAlerts, CallAlerts)> {
	let title = crate::i18n::tr_fmt("Notifications for {}", &[&account]);
	let dialog = Dialog::builder(parent, &title)
		.with_size(440, 330)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let note = StaticText::builder(&panel)
		.with_label(crate::i18n::tr(
			"These apply while another account is on screen. Each chat's own notification settings still count, and the account on screen always uses the normal settings.",
		))
		.build();
	theme::style_muted_label(&note);
	sizer.add(&note, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);

	// Each label goes right before its list, so the list is named by it.
	let combo = |label: &str, choices: Vec<&str>, at: usize| {
		let text = StaticText::builder(&panel).with_label(label).build();
		text.set_font(&theme::Fonts::body());
		sizer.add(&text, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);
		let combo = ComboBox::builder(&panel).with_string_choices(&choices).with_style(ComboBoxStyle::ReadOnly).build();
		combo.set_font(&theme::Fonts::body());
		combo.set_selection(at as u32);
		sizer.add(&combo, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);
		combo
	};
	let message_combo = combo(
		crate::i18n::tr("When a message comes in"),
		MESSAGE_CHOICES.iter().map(|(label, _)| crate::i18n::tr(label)).collect(),
		MESSAGE_CHOICES.iter().position(|(_, v)| *v == messages).unwrap_or(0),
	);
	let call_combo = combo(
		crate::i18n::tr("When someone calls"),
		CALL_CHOICES.iter().map(|(label, _)| crate::i18n::tr(label)).collect(),
		CALL_CHOICES.iter().position(|(_, v)| *v == calls).unwrap_or(0),
	);

	let save_btn = Button::builder(&panel).with_id(ID_OK).with_label(crate::i18n::tr("Save")).build();
	dialog_look::pill_button(&save_btn, true, theme::Palette::CONTENT_BG());
	let cancel_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label(crate::i18n::tr("Cancel")).build();
	dialog_look::pill_button(&cancel_btn, false, theme::Palette::CONTENT_BG());
	dialog_look::button_row(&sizer, &[&save_btn, &cancel_btn]);
	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();
	crate::ui::keys::escape_cancels_anywhere(dialog);
	message_combo.set_focus();

	let saved = dialog.show_modal() == ID_OK;
	let messages =
		message_combo.get_selection().and_then(|i| MESSAGE_CHOICES.get(i as usize)).map_or(messages, |c| c.1);
	let calls = call_combo.get_selection().and_then(|i| CALL_CHOICES.get(i as usize)).map_or(calls, |c| c.1);
	dialog.destroy();
	saved.then_some((messages, calls))
}
