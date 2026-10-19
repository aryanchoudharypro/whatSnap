//! "Blocked contacts": everyone the account has blocked, with a way to
//! unblock them or block someone else. The list changes while the dialog is
//! open (`blocklist` events), so its parts live in AppState for as long as
//! it's shown.

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::ui::theme;

/// One blocked person.
#[derive(Clone)]
pub struct BlockedEntry {
	pub jid: String,
	pub name: String,
	pub phone: String,
}

impl BlockedEntry {
	fn label(&self) -> String {
		// Someone who isn't a saved contact is named by their number already.
		if self.phone.is_empty() || self.name.starts_with('+') {
			self.name.clone()
		} else if self.name.is_empty() {
			self.phone.clone()
		} else {
			format!("{}, {}", self.name, self.phone)
		}
	}
}

/// The open "Blocked contacts" dialog.
pub struct BlockedContacts {
	pub dialog: Dialog,
	list: ListBox,
	list_label: StaticText,
	unblock_btn: Button,
	announcer: StaticText,
	entries: RefCell<Vec<BlockedEntry>>,
}

impl BlockedContacts {
	/// Shows the list as it now stands, keeping the selection in place.
	pub fn apply(&self, entries: &[BlockedEntry]) {
		let previous = self.list.get_selection().unwrap_or(0) as usize;
		let labels: Vec<String> = entries.iter().map(BlockedEntry::label).collect();
		let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
		self.list.set_items(&label_refs);
		self.list_label.set_label(&match entries.len() {
			0 => "Blocked contacts: none".to_string(),
			n => format!("Blocked contacts ({})", n),
		});
		if !entries.is_empty() {
			self.list.set_selection(previous.min(entries.len() - 1) as u32, true);
		} else if self.unblock_btn.has_focus() {
			// A disabled button can't keep the focus.
			self.list.set_focus();
		}
		self.unblock_btn.enable(!entries.is_empty());
		*self.entries.borrow_mut() = entries.to_vec();
	}

	pub fn announce(&self, message: &str) {
		crate::announce::announce(self.announcer, message);
	}

	fn selected(&self) -> Option<BlockedEntry> {
		let idx = self.list.get_selection()? as usize;
		self.entries.borrow().get(idx).cloned()
	}
}

/// Builds and shows the dialog, and returns once it's closed. `store` keeps
/// it reachable for list changes while it's open; `add` starts blocking
/// someone new.
pub fn show_blocked_contacts(
	parent: &impl WxWidget,
	entries: &[BlockedEntry],
	ws_tx: std::sync::mpsc::Sender<String>,
	add: impl Fn() + 'static,
	store: impl Fn(Option<Rc<BlockedContacts>>),
) {
	let dialog = Dialog::builder(parent, "Blocked contacts")
		.with_size(440, 480)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let list_label = StaticText::builder(&panel).with_label("Blocked contacts").build();
	theme::style_muted_label(&list_label);
	sizer.add(&list_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let list = ListBox::builder(&panel).build();
	list.set_font(&theme::Fonts::body());
	sizer.add(&list, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let announcer = StaticText::builder(&panel).with_label("").build();
	live_region::set_live_region(&announcer);
	theme::style_muted_label(&announcer);
	sizer.add(&announcer, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let buttons = BoxSizer::builder(Orientation::Horizontal).build();
	let unblock_btn = Button::builder(&panel).with_label("Unblock").build();
	let add_btn = Button::builder(&panel).with_label("Block a contact...").build();
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Close").build();
	buttons.add(&unblock_btn, 0, SizerFlag::Right, 8);
	buttons.add(&add_btn, 0, SizerFlag::Right, 8);
	buttons.add_stretch_spacer(1);
	buttons.add(&close_btn, 0, SizerFlag::empty(), 0);
	sizer.add_sizer(&buttons, 0, SizerFlag::Expand | SizerFlag::All, 10);
	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let blocked = Rc::new(BlockedContacts {
		dialog,
		list,
		list_label,
		unblock_btn,
		announcer,
		entries: RefCell::new(Vec::new()),
	});
	blocked.apply(entries);
	store(Some(blocked.clone()));

	let blocked_unblock = blocked.clone();
	unblock_btn.on_click(move |_| {
		if let Some(entry) = blocked_unblock.selected() {
			let payload = serde_json::json!({ "action": "unblock_contact", "data": { "jid": entry.jid } });
			let _ = ws_tx.send(payload.to_string());
			blocked_unblock.announce(&format!("Unblocking {}...", entry.label()));
		}
	});
	add_btn.on_click(move |_| add());
	close_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));

	let escape_closes = crate::ui::keys::escape_cancels(dialog);
	list.on_key_down(escape_closes);

	list.set_focus();
	dialog.show_modal();
	store(None);
	dialog.destroy();
}
