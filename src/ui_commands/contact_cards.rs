//! Received contact cards: the View contact dialog and the actions WhatsApp
//! Web offers on a contact card message (Message, Save contact, View
//! business, Add to group, Download).

use std::{cell::RefCell, rc::Rc};

use serde::{Deserialize, Deserializer};
use wxdragon::prelude::*;

use crate::{
	AppState, UiCommand,
	ui::{keys, theme, window::WindowParts},
};

/// Reads a JSON null as the empty value, which `#[serde(default)]` alone
/// doesn't: a Go nil slice arrives as null.
fn null_default<'de, D: Deserializer<'de>, T: Default + Deserialize<'de>>(d: D) -> Result<T, D::Error> {
	Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
struct CardNumber {
	jid: String,
	digits: String,
	label: String,
}

impl CardNumber {
	fn shown(&self) -> String {
		whatsnap::phone::format_international(&self.digits).unwrap_or_else(|| format!("+{}", self.digits))
	}
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
struct CardDetail {
	kind: String,
	value: String,
	label: String,
	link: String,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
struct Card {
	name: String,
	org: String,
	first_name: String,
	last_name: String,
	#[serde(deserialize_with = "null_default")]
	numbers: Vec<CardNumber>,
	lid: String,
	lid_chat: String,
	#[serde(deserialize_with = "null_default")]
	details: Vec<CardDetail>,
	saved: bool,
	me: bool,
	business: bool,
	can_save: bool,
	add_to_group: bool,
}

impl Card {
	/// WA Web's Message target: the LID when the card has no phone numbers,
	/// otherwise its WhatsApp numbers (a choice when there are several).
	fn message_targets(&self) -> Vec<(String, String)> {
		if !self.lid.is_empty() {
			let jid = if self.lid_chat.is_empty() { format!("{}@lid", self.lid) } else { self.lid_chat.clone() };
			return vec![(jid, self.name.clone())];
		}
		self.numbers.iter().map(|n| (n.jid.clone(), format!("{}, {}", n.shown(), n.label))).collect()
	}

	fn main_jid(&self) -> Option<String> {
		self.message_targets().into_iter().next().map(|(jid, _)| jid)
	}
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CardEvent {
	jid: String,
	intent: String,
	multi: bool,
	title: String,
	group_name: String,
	#[serde(deserialize_with = "null_default")]
	cards: Vec<Card>,
	vcard: String,
	file_name: String,
}

#[derive(Clone)]
enum RowAction {
	None,
	MessageCard,
	Message(String),
	Link(String, &'static str),
}

struct Row {
	card: usize,
	text: String,
	copy: String,
	action: RowAction,
}

fn card_rows(cards: &[Card], multi: bool) -> Vec<Row> {
	let mut rows = Vec::new();
	for (i, card) in cards.iter().enumerate() {
		let mut head = if card.name.is_empty() { crate::i18n::tr("Contact").to_string() } else { card.name.clone() };
		if !card.org.is_empty() {
			head = format!("{head}, {}", card.org);
		}
		if multi {
			head = crate::i18n::tr_fmt("{} ({} of {})", &[&head, &(i + 1), &cards.len()]);
		}
		let action = if card.message_targets().is_empty() { RowAction::None } else { RowAction::MessageCard };
		rows.push(Row { card: i, text: head, copy: card.name.clone(), action });
		for number in &card.numbers {
			let shown = number.shown();
			rows.push(Row {
				card: i,
				text: crate::i18n::tr_fmt("{}, {}, on WhatsApp", &[&shown, &number.label]),
				copy: shown,
				action: RowAction::Message(number.jid.clone()),
			});
		}
		for detail in &card.details {
			let action = match detail.kind.as_str() {
				"email" => RowAction::Link(detail.link.clone(), crate::i18n::tr("Send &Email")),
				"address" => RowAction::Link(detail.link.clone(), crate::i18n::tr("Open in &Maps")),
				_ => RowAction::None,
			};
			rows.push(Row {
				card: i,
				text: format!("{}, {}", detail.value, crate::i18n::tr_str(&detail.label)),
				copy: detail.value.clone(),
				action,
			});
		}
	}
	rows
}

fn send(state: &Rc<RefCell<AppState>>, action: &str, data: serde_json::Value) {
	let payload = serde_json::json!({ "action": action, "data": data });
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

fn announce(parts: &WindowParts, message: &str) {
	crate::platform::announce::announce(parts.conversations_parts.messages_announcer, message);
}

/// Asks the backend for a contact card message. `intent` is "view" (Enter
/// or View contact), "message" (Message contact) or "download".
pub(crate) fn request_contact_card(state: &Rc<RefCell<AppState>>, message_id: String, intent: &str) {
	let Some(jid) = state.borrow().selected_chat.clone() else { return };
	send(state, "get_contact_card", serde_json::json!({ "jid": jid, "message_id": message_id, "intent": intent }));
}

pub(crate) fn on_contact_card(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let event = match CardEvent::deserialize(data) {
		Ok(event) => event,
		Err(err) => {
			eprintln!("[LOG UI] Couldn't read a contact card: {err}");
			return;
		}
	};
	if state.borrow().selected_chat.as_deref() != Some(event.jid.as_str()) {
		return;
	}
	match event.intent.as_str() {
		"download" => save_vcard_file(parts, &event),
		"message" if !event.multi && event.cards.len() == 1 && !event.cards[0].message_targets().is_empty() => {
			message_card(state, &parts.frame, &event.cards[0]);
		}
		_ if event.cards.is_empty() => {
			// WA Web: a card it can't read can still be downloaded.
			let ask = MessageDialog::builder(
				&parts.frame,
				crate::i18n::tr(
					"This contact can't be displayed in whatSnap. Download it to open it with another app on your computer?",
				),
				crate::i18n::tr("Download contact?"),
			)
			.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
			.build();
			ask.set_yes_no_labels(crate::i18n::tr("Download"), crate::i18n::tr("Cancel"));
			if ask.show_modal() == wxdragon::id::ID_YES {
				save_vcard_file(parts, &event);
			}
		}
		_ => show_contact_dialog(state, parts, event),
	}
}

fn save_vcard_file(parts: &Rc<WindowParts>, event: &CardEvent) {
	let chooser = FileDialog::builder(&parts.frame)
		.with_message(crate::i18n::tr("Download contact"))
		.with_default_file(&event.file_name)
		.with_wildcard(crate::i18n::tr("Contact cards (*.vcf)|*.vcf|All files (*.*)|*.*"))
		.with_style(FileDialogStyle::Save | FileDialogStyle::OverwritePrompt)
		.build();
	let path = if chooser.show_modal() == ID_OK { chooser.get_path() } else { None };
	chooser.destroy();
	let Some(path) = path else { return };
	let spoken = match std::fs::write(&path, event.vcard.as_bytes()) {
		Ok(()) => crate::i18n::tr("Contact saved.").to_string(),
		Err(err) => crate::i18n::tr_fmt("Couldn't save the contact: {}", &[&err]),
	};
	announce(parts, &spoken);
}

/// WA Web's Message: one number opens the chat, several ask which.
fn message_card(state: &Rc<RefCell<AppState>>, parent: &impl WxWidget, card: &Card) {
	if let Some(jid) = pick_target(parent, card) {
		let _ = state.borrow().ui_tx.send(UiCommand::OpenChatByJid(jid));
	}
}

fn pick_target(parent: &impl WxWidget, card: &Card) -> Option<String> {
	let targets = card.message_targets();
	if targets.len() > 1 {
		let labels: Vec<&str> = targets.iter().map(|(_, label)| label.as_str()).collect();
		let chooser = SingleChoiceDialog::builder(
			parent,
			crate::i18n::tr("Choose a phone number"),
			crate::i18n::tr("Message"),
			&labels,
		)
		.build();
		chooser.set_selection(0);
		if chooser.show_modal() != wxdragon::id::ID_OK {
			return None;
		}
		let i = chooser.get_selection();
		return targets.get(i as usize).map(|(jid, _)| jid.clone());
	}
	targets.into_iter().next().map(|(jid, _)| jid)
}

fn save_contact(parts: &Rc<WindowParts>, card: &Card) {
	let Some(number) = card.numbers.first() else { return };
	let title = if card.saved { crate::i18n::tr("Edit Contact") } else { crate::i18n::tr("Add Contact") };
	crate::ui_commands::contacts_groups::open_contact_form(
		parts,
		title,
		crate::i18n::tr("Save"),
		&card.first_name,
		&card.last_name,
		&format!("+{}", number.digits),
		false,
	);
}

fn add_to_group(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, group: &str, group_name: &str, card: &Card) {
	let Some(person) = card.main_jid() else { return };
	let name = state.borrow().contacts.get(&person).cloned().filter(|n| !n.is_empty()).unwrap_or(card.name.clone());
	let question = crate::i18n::tr_fmt("Add {} to \"{}\" group?", &[&name, &group_name]);
	let confirm = MessageDialog::builder(&parts.frame, &question, crate::i18n::tr("Add to group"))
		.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
		.build();
	confirm.set_yes_no_labels(crate::i18n::tr("Add member"), crate::i18n::tr("Cancel"));
	if confirm.show_modal() != wxdragon::id::ID_YES {
		return;
	}
	send(
		state,
		"update_group_participants",
		serde_json::json!({ "jid": group, "participants": [person], "action": "add" }),
	);
	announce(parts, &crate::i18n::tr_fmt("Adding {}...", &[&name]));
}

#[derive(Clone, Copy, PartialEq)]
enum After {
	Close,
	Message(usize),
	Save(usize),
	Business,
	AddToGroup,
}

fn show_contact_dialog(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, event: CardEvent) {
	let title = if event.multi {
		match event.cards.len() {
			1 => crate::i18n::tr("1 contact").to_string(),
			n => crate::i18n::tr_fmt("{} contacts", &[&n]),
		}
	} else {
		crate::i18n::tr("View contact").to_string()
	};
	let rows = card_rows(&event.cards, event.multi);
	let dialog = Dialog::builder(&parts.frame, &title)
		.with_size(480, 420)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let list_label =
		StaticText::builder(&panel).with_label(&if event.multi { title.clone() } else { event.title.clone() }).build();
	theme::style_muted_label(&list_label);
	sizer.add(&list_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let list = ListBox::builder(&panel).build();
	list.set_font(&theme::Fonts::body());
	let row_texts: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
	list.set_items(&row_texts);
	sizer.add(&list, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let single = (!event.multi).then(|| event.cards.first()).flatten();
	let buttons = BoxSizer::builder(Orientation::Horizontal).build();
	let action_btn = Button::builder(&panel).with_label(crate::i18n::tr("&Message")).build();
	theme::style_primary_button(&action_btn);
	let copy_btn = Button::builder(&panel).with_label(crate::i18n::tr("&Copy")).build();
	let save_btn = Button::builder(&panel).with_label(crate::i18n::tr("&Save Contact")).build();
	buttons.add(&action_btn, 0, SizerFlag::Right, 8);
	buttons.add(&copy_btn, 0, SizerFlag::Right, 8);
	buttons.add(&save_btn, 0, SizerFlag::Right, 8);
	let business_btn = single.filter(|c| c.business).map(|_| {
		let btn = Button::builder(&panel).with_label(crate::i18n::tr("View &Business")).build();
		buttons.add(&btn, 0, SizerFlag::Right, 8);
		btn
	});
	let group_btn = single.filter(|c| c.add_to_group).map(|_| {
		let btn = Button::builder(&panel).with_label(crate::i18n::tr("&Add to Group")).build();
		buttons.add(&btn, 0, SizerFlag::Right, 8);
		btn
	});
	buttons.add_stretch_spacer(1);
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label(crate::i18n::tr("Close")).build();
	buttons.add(&close_btn, 0, SizerFlag::empty(), 0);
	sizer.add_sizer(&buttons, 0, SizerFlag::Expand | SizerFlag::All, 10);
	let announcer = StaticText::builder(&panel).with_label("").build();
	live_region::set_live_region(&announcer);
	theme::style_muted_label(&announcer);
	sizer.add(&announcer, 0, SizerFlag::Left | SizerFlag::Bottom, 10);
	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let rows = Rc::new(rows);
	let cards = Rc::new(event.cards.clone());
	let refresh = {
		let (rows, cards) = (rows.clone(), cards.clone());
		move || {
			let Some(row) = list.get_selection().and_then(|i| rows.get(i as usize)) else { return };
			let label = match &row.action {
				RowAction::Link(_, label) => label,
				_ => crate::i18n::tr("&Message"),
			};
			action_btn.set_label(label);
			action_btn.enable(!matches!(row.action, RowAction::None));
			copy_btn.enable(!row.copy.is_empty());
			let card = &cards[row.card];
			save_btn.set_label(if card.saved {
				crate::i18n::tr("&Edit Contact")
			} else {
				crate::i18n::tr("&Save Contact")
			});
			save_btn.enable(card.can_save);
		}
	};
	if !rows.is_empty() {
		list.set_selection(0, true);
	}
	refresh();
	list.on_selection_changed(move |_| refresh());

	let after: Rc<RefCell<(After, String)>> = Rc::new(RefCell::new((After::Close, String::new())));
	let act = {
		let (rows, after) = (rows.clone(), after.clone());
		move || {
			let Some(row) = list.get_selection().and_then(|i| rows.get(i as usize)) else { return };
			match &row.action {
				RowAction::None => {}
				RowAction::MessageCard => {
					*after.borrow_mut() = (After::Message(row.card), String::new());
					dialog.end_modal(ID_OK);
				}
				RowAction::Message(jid) => {
					*after.borrow_mut() = (After::Message(row.card), jid.clone());
					dialog.end_modal(ID_OK);
				}
				RowAction::Link(url, _) => {
					wxdragon::utils::launch_default_browser(url, wxdragon::utils::BrowserLaunchFlags::Default);
				}
			}
		}
	};
	let act = Rc::new(act);
	let act_btn = act.clone();
	action_btn.on_click(move |_| act_btn());
	keys::on_enter(&list, Rc::default(), move || act());
	let copy_rows = rows.clone();
	copy_btn.on_click(move |_| {
		let Some(row) = list.get_selection().and_then(|i| copy_rows.get(i as usize)) else { return };
		if crate::platform::clipboard::copy_text(&row.copy) {
			crate::platform::announce::announce(announcer, &crate::i18n::tr_fmt("Copied: {}", &[&row.copy]));
		}
	});
	let (save_rows, save_after) = (rows.clone(), after.clone());
	save_btn.on_click(move |_| {
		let Some(row) = list.get_selection().and_then(|i| save_rows.get(i as usize)) else { return };
		*save_after.borrow_mut() = (After::Save(row.card), String::new());
		dialog.end_modal(ID_OK);
	});
	if let Some(btn) = business_btn {
		let after = after.clone();
		btn.on_click(move |_| {
			*after.borrow_mut() = (After::Business, String::new());
			dialog.end_modal(ID_OK);
		});
	}
	if let Some(btn) = group_btn {
		let after = after.clone();
		btn.on_click(move |_| {
			*after.borrow_mut() = (After::AddToGroup, String::new());
			dialog.end_modal(ID_OK);
		});
	}
	close_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));
	keys::escape_cancels_anywhere(dialog);

	list.set_focus();
	dialog.show_modal();
	dialog.destroy();

	let (what, jid) = after.borrow().clone();
	match what {
		After::Close => {}
		After::Message(_) if !jid.is_empty() => {
			let _ = state.borrow().ui_tx.send(UiCommand::OpenChatByJid(jid));
		}
		After::Message(i) => {
			if let Some(card) = cards.get(i) {
				message_card(state, &parts.frame, card);
			}
		}
		After::Save(i) => {
			if let Some(card) = cards.get(i) {
				save_contact(parts, card);
			}
		}
		After::Business => {
			// WA Web opens the chat and its contact info.
			if let Some(card) = cards.first()
				&& let Some(jid) = pick_target(&parts.frame, card)
			{
				let tx = state.borrow().ui_tx.clone();
				let _ = tx.send(UiCommand::OpenChatByJid(jid.clone()));
				let _ = tx.send(UiCommand::ShowContactInfo(jid));
			}
		}
		After::AddToGroup => {
			if let Some(card) = cards.first() {
				add_to_group(state, parts, &event.jid, &event.group_name, card);
			}
		}
	}
}
