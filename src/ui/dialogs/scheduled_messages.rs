use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::ui::{dialog_look, theme};

/// One of the open chat's scheduled messages, as WA Web's
/// WAWebScheduledMessagesList shows it.
#[derive(Clone)]
pub struct ScheduledEntry {
	pub id: String,
	pub text: String,
	pub kind: String,
	pub when: String,
	pub failed: bool,
	pub past: bool,
}

impl ScheduledEntry {
	pub fn from_json(item: &serde_json::Value) -> Option<Self> {
		let text = |key: &str| item.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
		let id = text("id");
		if id.is_empty() {
			return None;
		}
		Some(ScheduledEntry {
			id,
			text: text("text"),
			kind: text("kind"),
			when: text("when"),
			failed: item.get("failed").and_then(|v| v.as_bool()).unwrap_or(false),
			past: item.get("past").and_then(|v| v.as_bool()).unwrap_or(false),
		})
	}

	fn content(&self) -> String {
		let text = self.text.split_whitespace().collect::<Vec<_>>().join(" ");
		let media = match self.kind.as_str() {
			"image" => crate::i18n::tr("Photo"),
			"video" => crate::i18n::tr("Video"),
			_ => return text,
		};
		if text.is_empty() { media.to_string() } else { format!("{media}: {text}") }
	}

	/// WA Web's "Scheduled for today at {time}", and its failure dialog's
	/// "Your message did not send." / "Your message can't be scheduled."
	pub fn label(&self) -> String {
		let when = crate::i18n::translate_backend_message(&self.when);
		match (self.failed, self.past) {
			(false, _) => {
				crate::i18n::tr_fmt("Scheduled for {}: {}", &[&when, &self.content()])
			}
			(true, true) => {
				crate::i18n::tr_fmt("Did not send, {}: {}", &[&when, &self.content()])
			}
			(true, false) => {
				crate::i18n::tr_fmt("Couldn't be scheduled, {}: {}", &[&when, &self.content()])
			}
		}
	}
}

pub struct ScheduledList {
	pub dialog: Dialog,
	pub jid: String,
	list: ListBox,
	list_label: StaticText,
	send_now_btn: Button,
	delete_btn: Button,
	copy_btn: Button,
	announcer: StaticText,
	entries: RefCell<Vec<ScheduledEntry>>,
}

impl ScheduledList {
	pub fn apply(&self, entries: &[ScheduledEntry], pending: u64, limit: u64) {
		let previous = self.list.get_selection().unwrap_or(0) as usize;
		let labels: Vec<String> = entries.iter().map(ScheduledEntry::label).collect();
		let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
		self.list.set_items(&label_refs);
		self.list_label.set_label(&match entries.len() {
			0 => crate::i18n::tr("Scheduled messages: none").to_string(),
			n => {
				crate::i18n::tr_fmt("Scheduled messages ({}, {} of {} waiting)", &[&n, &pending, &limit])
			}
		});
		let any = !entries.is_empty();
		if any {
			self.list.set_selection(previous.min(entries.len() - 1) as u32, true);
		} else if self.send_now_btn.has_focus() || self.delete_btn.has_focus() || self.copy_btn.has_focus() {
			self.list.set_focus();
		}
		for button in [&self.send_now_btn, &self.delete_btn, &self.copy_btn] {
			button.enable(any);
		}
		let had_some = !self.entries.borrow().is_empty();
		*self.entries.borrow_mut() = entries.to_vec();
		if had_some && !any {
			self.announce(crate::i18n::tr("All scheduled messages have been sent or deleted"));
		}
	}

	pub fn announce(&self, message: &str) {
		crate::platform::announce::announce(self.announcer, message);
	}

	fn selected(&self) -> Option<ScheduledEntry> {
		let idx = self.list.get_selection()? as usize;
		self.entries.borrow().get(idx).cloned()
	}
}

fn send(ws_tx: &std::sync::mpsc::Sender<String>, action: &str, jid: &str, id: &str) {
	let payload = serde_json::json!({ "action": action, "data": { "jid": jid, "message_id": id } });
	let _ = ws_tx.send(payload.to_string());
}

fn confirm(parent: &Dialog, message: &str, title: &str) -> bool {
	MessageDialog::builder(parent, message, title)
		.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
		.build()
		.show_modal()
		== ID_YES
}

/// The open chat's scheduled messages, with WA Web's actions on them: delete
/// (unschedule), send now and copy.
pub fn show_scheduled_messages(
	parent: &impl WxWidget,
	chat_name: &str,
	jid: &str,
	ws_tx: std::sync::mpsc::Sender<String>,
	store: impl Fn(Option<Rc<ScheduledList>>),
) {
	let title = crate::i18n::tr_fmt("Scheduled messages for {}", &[&chat_name]);
	let dialog = Dialog::builder(parent, &title)
		.with_size(520, 520)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let info_label = StaticText::builder(&panel)
		.with_label(crate::i18n::tr("WhatsApp sends these at the time shown, even when this computer is off."))
		.build();
	theme::style_muted_label(&info_label);
	sizer.add(&info_label, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);

	let list_label = StaticText::builder(&panel).with_label(crate::i18n::tr("Scheduled messages: loading")).build();
	list_label.set_font(&theme::Fonts::body_strong());
	sizer.add(&list_label, 0, SizerFlag::Left | SizerFlag::Top, 12);
	let list = ListBox::builder(&panel).build();
	list.set_font(&theme::Fonts::body());
	sizer.add(&list, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let announcer = StaticText::builder(&panel).with_label("").build();
	live_region::set_live_region(&announcer);
	theme::style_muted_label(&announcer);
	sizer.add(&announcer, 0, SizerFlag::Left | SizerFlag::Top, 8);

	let buttons = BoxSizer::builder(Orientation::Horizontal).build();
	let send_now_btn = Button::builder(&panel).with_label(crate::i18n::tr("Send now")).build();
	dialog_look::pill_button(&send_now_btn, true, theme::Palette::CONTENT_BG());
	let delete_btn = Button::builder(&panel).with_label(crate::i18n::tr("Delete")).build();
	dialog_look::pill_button(&delete_btn, false, theme::Palette::CONTENT_BG());
	let copy_btn = Button::builder(&panel).with_label(crate::i18n::tr("Copy text")).build();
	dialog_look::pill_button(&copy_btn, false, theme::Palette::CONTENT_BG());
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label(crate::i18n::tr("Close")).build();
	dialog_look::pill_button(&close_btn, false, theme::Palette::CONTENT_BG());
	buttons.add(&send_now_btn, 0, SizerFlag::Right, 8);
	buttons.add(&delete_btn, 0, SizerFlag::Right, 8);
	buttons.add(&copy_btn, 0, SizerFlag::Right, 8);
	buttons.add_stretch_spacer(1);
	buttons.add(&close_btn, 0, SizerFlag::empty(), 0);
	sizer.add_sizer(&buttons, 0, SizerFlag::Expand | SizerFlag::All, 12);
	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();
	for button in [&send_now_btn, &delete_btn, &copy_btn] {
		button.enable(false);
	}

	let scheduled = Rc::new(ScheduledList {
		dialog,
		jid: jid.to_string(),
		list,
		list_label,
		send_now_btn,
		delete_btn,
		copy_btn,
		announcer,
		entries: RefCell::new(Vec::new()),
	});
	store(Some(scheduled.clone()));
	send(&ws_tx, "get_scheduled_messages", jid, "");

	let delete: Rc<dyn Fn()> = {
		let (scheduled, ws_tx) = (scheduled.clone(), ws_tx.clone());
		Rc::new(move || {
			let Some(entry) = scheduled.selected() else { return };
			if !confirm(
				&scheduled.dialog,
				crate::i18n::tr("This message will not be sent. This action cannot be undone."),
				crate::i18n::tr("Delete scheduled message?"),
			) {
				return;
			}
			send(&ws_tx, "unschedule_message", &scheduled.jid, &entry.id);
			scheduled.announce(crate::i18n::tr("Deleting scheduled message..."));
		})
	};
	let delete_click = delete.clone();
	delete_btn.on_click(move |_| delete_click());

	let (scheduled_send, ws_tx_send) = (scheduled.clone(), ws_tx.clone());
	send_now_btn.on_click(move |_| {
		let Some(entry) = scheduled_send.selected() else { return };
		if !(entry.failed && entry.past)
			&& !confirm(
				&scheduled_send.dialog,
				crate::i18n::tr("Send this message now? It won't be sent again at the scheduled time."),
				crate::i18n::tr("Send now"),
			) {
			return;
		}
		send(&ws_tx_send, "send_scheduled_now", &scheduled_send.jid, &entry.id);
		scheduled_send.announce(crate::i18n::tr("Sending..."));
	});

	let scheduled_copy = scheduled.clone();
	copy_btn.on_click(move |_| {
		let Some(entry) = scheduled_copy.selected() else { return };
		if entry.text.is_empty() {
			scheduled_copy.announce(crate::i18n::tr("No text to copy."));
		} else if crate::platform::clipboard::copy_text(&entry.text) {
			scheduled_copy.announce(crate::i18n::tr("Message text copied to clipboard."));
		} else {
			scheduled_copy.announce(crate::i18n::tr("Couldn't copy to clipboard."));
		}
	});
	close_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));

	list.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key) = event
			&& crate::ui::keys::is_delete(key.get_key_code())
		{
			if list.get_selection().is_some() {
				delete();
			}
			event.skip(false);
			return;
		}
		crate::ui::keys::escape_cancels(dialog)(event);
	});
	crate::ui::keys::escape_cancels_anywhere(dialog);

	list.set_focus();
	dialog.show_modal();
	store(None);
	dialog.destroy();
}

#[cfg(test)]
mod tests {
	use super::*;

	fn entry(kind: &str, text: &str, failed: bool, past: bool) -> ScheduledEntry {
		ScheduledEntry {
			id: "A".into(),
			text: text.into(),
			kind: kind.into(),
			when: "today at 18:30".into(),
			failed,
			past,
		}
	}

	#[test]
	fn labels_say_when_and_what() {
		assert_eq!(entry("text", "see you\nsoon", false, false).label(), "Scheduled for today at 18:30: see you soon");
		assert_eq!(entry("image", "", false, false).label(), "Scheduled for today at 18:30: Photo");
		assert_eq!(entry("video", "clip", true, true).label(), "Did not send, today at 18:30: Video: clip");
		assert_eq!(entry("text", "hi", true, false).label(), "Couldn't be scheduled, today at 18:30: hi");
	}
}
