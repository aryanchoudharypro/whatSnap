use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState,
	ui::{
		message_display::{BUILT_IN_CHAT_FILTERS, apply_chat_filter},
		theme,
		window::WindowParts,
	},
};

pub(crate) struct ChatList {
	pub(crate) id: String,
	pub(crate) name: String,
	pub(crate) jids: Vec<String>,
}

fn send(state: &Rc<RefCell<AppState>>, action: &str, data: serde_json::Value) {
	let payload = serde_json::json!({ "action": action, "data": data });
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

fn announce(state: &Rc<RefCell<AppState>>, parts: &WindowParts, message: &str) {
	let announcer = state.borrow().list_manager.as_ref().map(|m| m.announcer());
	let announcer = announcer.unwrap_or(parts.conversations_parts.messages_announcer);
	crate::platform::announce::announce(announcer, message);
}

fn text(data: &serde_json::Value, key: &str) -> String {
	data.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn strings(data: &serde_json::Value, key: &str) -> Vec<String> {
	data.get(key)
		.and_then(|v| v.as_array())
		.map(|a| a.iter().filter_map(|j| j.as_str().map(str::to_string)).collect())
		.unwrap_or_default()
}

fn chat_name(s: &AppState, jid: &str) -> String {
	s.contacts.get(jid).cloned().unwrap_or_else(|| crate::ui::contact_picker::display_name_for_jid(jid))
}

fn ask_name(parent: &impl WxWidget, prompt: &str, title: &str, value: &str) -> Option<String> {
	let dialog = TextEntryDialog::builder(parent, prompt, title).with_default_value(value).build();
	if dialog.show_modal() != wxdragon::id::ID_OK {
		return None;
	}
	Some(dialog.get_value().unwrap_or_default().trim().to_string()).filter(|n| !n.is_empty())
}

pub(crate) fn set_filter(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, idx: usize) {
	let mut s = state.borrow_mut();
	s.chat_filter = idx;
	apply_chat_filter(&mut s, parts);
	// Only the chats: not headings, contacts or messages a search found.
	let count = s
		.filtered_chats
		.iter()
		.zip(&s.filtered_message_ids)
		.filter(|(jid, message_id)| message_id.is_empty() && s.chats.contains(*jid))
		.count();
	drop(s);
	crate::ui_commands::chat_list::search_all_messages(state);
	announce(state, parts, &count_chats(count));
}

pub(crate) fn set_favorite(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>, jid: String, favorite: bool) {
	send(state, "set_favorite", serde_json::json!({ "jid": jid, "favorite": favorite }));
}

pub(crate) fn choose_lists(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String) {
	let (name, lists): (String, Vec<(String, String, bool)>) = {
		let s = state.borrow();
		let lists = s.chat_lists.iter().map(|l| (l.id.clone(), l.name.clone(), l.jids.contains(&jid))).collect();
		(chat_name(&s, &jid), lists)
	};
	if lists.is_empty() {
		new_list(state, parts, Some(&jid), &name);
		return;
	}

	let dialog = Dialog::builder(&parts.frame, crate::i18n::tr("Lists"))
		.with_size(380, 420)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();
	let label_text =
		crate::i18n::tr_fmt("Lists {} is in", &[&name]);
	let label = StaticText::builder(&panel).with_label(&label_text).build();
	theme::style_muted_label(&label);
	sizer.add(&label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let list = CheckListBox::builder(&panel).build();
	list.set_font(&theme::Fonts::body());
	for (idx, (_, list_name, member)) in lists.iter().enumerate() {
		list.append(list_name);
		list.check(idx as u32, *member);
	}
	list.set_selection(0, true);
	sizer.add(&list, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);
	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let done_btn = Button::builder(&panel).with_id(ID_OK).with_label(crate::i18n::tr("Done")).build();
	theme::style_primary_button(&done_btn);
	let new_btn = Button::builder(&panel).with_label(crate::i18n::tr("New List...")).build();
	let cancel_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label(crate::i18n::tr("Cancel")).build();
	btn_sizer.add(&done_btn, 0, SizerFlag::Right, 8);
	btn_sizer.add(&new_btn, 0, SizerFlag::Right, 8);
	btn_sizer.add_stretch_spacer(1);
	btn_sizer.add(&cancel_btn, 0, SizerFlag::Right, 8);
	sizer.add_sizer(&btn_sizer, 0, SizerFlag::Expand | SizerFlag::All, 10);
	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	const NEW_LIST: i32 = 1;
	list.on_key_down(crate::ui::keys::escape_cancels(dialog));
	done_btn.on_click(move |_| dialog.end_modal(ID_OK));
	new_btn.on_click(move |_| dialog.end_modal(NEW_LIST));
	cancel_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));
	list.set_focus();
	let result = dialog.show_modal();
	let checked: Vec<bool> = (0..lists.len()).map(|i| list.is_checked(i as u32)).collect();
	dialog.destroy();

	if result == NEW_LIST {
		new_list(state, parts, Some(&jid), &name);
	} else if result == ID_OK {
		for ((id, _, member), now) in lists.iter().zip(checked) {
			if *member != now {
				send(state, "set_list_member", serde_json::json!({ "id": id, "jid": jid, "member": now }));
			}
		}
	}
}

fn new_list(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: Option<&str>, chat: &str) {
	let prompt = match jid {
		Some(_) => {
			crate::i18n::tr_fmt("Name a new list for {}:", &[&chat])
		}
		None => crate::i18n::tr("List name:").to_string(),
	};
	let Some(name) = ask_name(&parts.frame, &prompt, crate::i18n::tr("New List"), "") else { return };
	let jids: Vec<&str> = jid.into_iter().collect();
	send(state, "create_list", serde_json::json!({ "name": name, "jids": jids }));
}

pub(crate) struct ListManager {
	dialog: Dialog,
	list: ListBox,
	list_label: StaticText,
	rename_btn: Button,
	chats_btn: Button,
	delete_btn: Button,
	announcer: StaticText,
	ids: RefCell<Vec<String>>,
	chats: RefCell<Option<Rc<ListChats>>>,
}

struct ListChats {
	dialog: Dialog,
	list: ListBox,
	list_label: StaticText,
	remove_btn: Button,
	announcer: StaticText,
	list_id: String,
	jids: RefCell<Vec<String>>,
}

fn count_chats(count: usize) -> String {
	if crate::i18n::is_indonesian() {
		format!("{count} chat")
	} else if count == 1 {
		"1 chat".to_string()
	} else {
		format!("{count} chats")
	}
}

fn keep_selection(list: &ListBox, idx: usize, buttons: &[&Button]) {
	let count = list.get_count() as usize;
	if count > 0 {
		list.set_selection(idx.min(count - 1) as u32, true);
	} else if buttons.iter().any(|b| b.has_focus()) {
		list.set_focus();
	}
	for b in buttons {
		b.enable(count > 0);
	}
}

impl ListManager {
	fn apply(&self, s: &AppState) {
		let previous = self.selected_id();
		let previous_idx = self.list.get_selection().unwrap_or(0) as usize;
		let labels: Vec<String> =
			s.chat_lists.iter().map(|l| format!("{}, {}", l.name, count_chats(l.jids.len()))).collect();
		let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
		self.list.set_items(&refs);
		*self.ids.borrow_mut() = s.chat_lists.iter().map(|l| l.id.clone()).collect();
		self.list_label.set_label(&if crate::i18n::is_indonesian() {
			match s.chat_lists.len() {
				0 => "Daftar Anda: belum ada".to_string(),
				n => format!("Daftar Anda ({n})"),
			}
		} else {
			match s.chat_lists.len() {
				0 => "Your lists: none yet".to_string(),
				n => format!("Your lists ({n})"),
			}
		});
		let idx = previous.and_then(|id| self.ids.borrow().iter().position(|i| *i == id)).unwrap_or(previous_idx);
		keep_selection(&self.list, idx, &[&self.rename_btn, &self.chats_btn, &self.delete_btn]);
		let chats = self.chats.borrow().clone();
		if let Some(chats) = chats {
			match s.chat_lists.iter().find(|l| l.id == chats.list_id) {
				Some(l) => chats.apply(s, l),
				None => chats.dialog.end_modal(ID_CANCEL),
			}
		}
	}

	fn select(&self, id: &str) {
		if let Some(idx) = self.ids.borrow().iter().position(|i| i == id) {
			self.list.set_selection(idx as u32, true);
		}
	}

	fn selected_id(&self) -> Option<String> {
		let idx = self.list.get_selection()? as usize;
		self.ids.borrow().get(idx).cloned()
	}

	fn announcer(&self) -> StaticText {
		self.chats.borrow().as_ref().map_or(self.announcer, |c| c.announcer)
	}
}

impl ListChats {
	fn apply(&self, s: &AppState, l: &ChatList) {
		let previous_idx = self.list.get_selection().unwrap_or(0) as usize;
		let labels: Vec<String> = l.jids.iter().map(|j| chat_name(s, j)).collect();
		let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
		self.list.set_items(&refs);
		let title =
			crate::i18n::tr_fmt("Chats in {}", &[&l.name]);
		self.dialog.set_label(&title);
		self.list_label.set_label(&if crate::i18n::is_indonesian() {
			match l.jids.len() {
				0 => format!("Chat di {}: belum ada", l.name),
				n => format!("Chat di {} ({})", l.name, n),
			}
		} else {
			match l.jids.len() {
				0 => format!("Chats in {}: none yet", l.name),
				n => format!("Chats in {} ({})", l.name, n),
			}
		});
		keep_selection(&self.list, previous_idx, &[&self.remove_btn]);
		*self.jids.borrow_mut() = l.jids.clone();
	}

	fn selected(&self) -> Option<String> {
		let idx = self.list.get_selection()? as usize;
		self.jids.borrow().get(idx).cloned()
	}
}

fn list_dialog(
	parent: &impl WxWidget,
	title: &str,
	labels: &[&str],
) -> (Dialog, StaticText, ListBox, StaticText, Vec<Button>) {
	let dialog = Dialog::builder(parent, title)
		.with_size(480, 420)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();
	let list_label = StaticText::builder(&panel).with_label(title).build();
	theme::style_muted_label(&list_label);
	sizer.add(&list_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let list = ListBox::builder(&panel).build();
	list.set_font(&theme::Fonts::body());
	sizer.add(&list, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);
	let announcer = StaticText::builder(&panel).with_label("").build();
	live_region::set_live_region(&announcer);
	theme::style_muted_label(&announcer);
	sizer.add(&announcer, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let row = BoxSizer::builder(Orientation::Horizontal).build();
	let mut buttons: Vec<Button> = labels.iter().map(|l| Button::builder(&panel).with_label(l).build()).collect();
	for b in &buttons {
		row.add(b, 0, SizerFlag::Right, 8);
	}
	row.add_stretch_spacer(1);
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label(crate::i18n::tr("Close")).build();
	row.add(&close_btn, 0, SizerFlag::empty(), 0);
	sizer.add_sizer(&row, 0, SizerFlag::Expand | SizerFlag::All, 10);
	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();
	close_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));
	buttons.push(close_btn);
	(dialog, list_label, list, announcer, buttons)
}

type Action = Rc<dyn Fn()>;

fn on_press(button: &Button, action: impl Fn() + 'static) -> Action {
	let action: Action = Rc::new(action);
	let on_click = action.clone();
	button.on_click(move |_| on_click());
	action
}

fn list_keys(dialog: Dialog, list: ListBox, delete: Action, rename: Option<Action>, open: Option<Action>) {
	list.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key) = event {
			let action = match key.get_key_code() {
				Some(WXK_DELETE | WXK_NUMPAD_DELETE) => Some(&delete),
				Some(WXK_F2) => rename.as_ref(),
				_ => None,
			};
			if let Some(action) = action {
				if list.get_selection().is_some() {
					action();
				}
				event.skip(false);
				return;
			}
		}
		crate::ui::keys::escape_cancels(dialog)(event);
	});
	if let Some(open) = open {
		crate::ui::keys::on_enter(&list, Rc::default(), move || {
			if list.get_selection().is_some() {
				open();
			}
		});
	}
}

pub(crate) fn manage_lists(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let (dialog, list_label, list, announcer, buttons) = list_dialog(
		&parts.frame,
		crate::i18n::tr("Manage Lists"),
		&[
			crate::i18n::tr("&Add list..."),
			crate::i18n::tr("&Rename..."),
			crate::i18n::tr("&Chats..."),
			crate::i18n::tr("&Delete"),
		],
	);
	let [add_btn, rename_btn, chats_btn, delete_btn, _close] = buttons[..] else { return };
	let manager = Rc::new(ListManager {
		dialog,
		list,
		list_label,
		rename_btn,
		chats_btn,
		delete_btn,
		announcer,
		ids: RefCell::new(Vec::new()),
		chats: RefCell::new(None),
	});
	manager.apply(&state.borrow());
	state.borrow_mut().list_manager = Some(manager.clone());

	let state_add = state.clone();
	on_press(&add_btn, move || {
		if let Some(name) = ask_name(&dialog, crate::i18n::tr("List name:"), crate::i18n::tr("New List"), "") {
			send(&state_add, "create_list", serde_json::json!({ "name": name, "jids": [] }));
		}
	});

	let (state_rename, manager_rename) = (state.clone(), manager.clone());
	let rename = on_press(&rename_btn, move || {
		let Some(id) = manager_rename.selected_id() else { return };
		let Some(name) = list_name(&state_rename, &id) else { return };
		if let Some(new_name) = ask_name(&dialog, crate::i18n::tr("List name:"), crate::i18n::tr("Rename List"), &name)
			&& new_name != name
		{
			send(&state_rename, "rename_list", serde_json::json!({ "id": id, "name": new_name }));
		}
	});

	let (state_delete, manager_delete) = (state.clone(), manager.clone());
	let delete = on_press(&delete_btn, move || {
		let Some(id) = manager_delete.selected_id() else { return };
		let Some(name) = list_name(&state_delete, &id) else { return };
		let msg = crate::i18n::tr_fmt("Delete the list {}? The chats in it stay as they are.", &[&name]);
		let confirm = MessageDialog::builder(&dialog, &msg, crate::i18n::tr("Delete List"))
			.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
			.build();
		confirm.set_yes_no_labels(crate::i18n::tr("Delete"), crate::i18n::tr("Cancel"));
		if confirm.show_modal() == ID_YES {
			send(&state_delete, "delete_list", serde_json::json!({ "id": id }));
		}
	});

	let (state_chats, manager_chats) = (state.clone(), manager.clone());
	let open = on_press(&chats_btn, move || {
		if let Some(id) = manager_chats.selected_id() {
			show_list_chats(&state_chats, &manager_chats, id);
		}
	});

	list_keys(dialog, list, delete, Some(rename), Some(open));
	list.set_focus();
	dialog.show_modal();
	state.borrow_mut().list_manager = None;
	dialog.destroy();
}

fn list_name(state: &Rc<RefCell<AppState>>, id: &str) -> Option<String> {
	state.borrow().chat_lists.iter().find(|l| l.id == id).map(|l| l.name.clone())
}

fn show_list_chats(state: &Rc<RefCell<AppState>>, manager: &Rc<ListManager>, id: String) {
	let Some(name) = list_name(state, &id) else { return };
	let title = crate::i18n::tr_fmt("Chats in {}", &[&name]);
	let (dialog, list_label, list, announcer, buttons) =
		list_dialog(&manager.dialog, &title, &[crate::i18n::tr("&Add chats..."), crate::i18n::tr("&Remove")]);
	let [add_btn, remove_btn, _close] = buttons[..] else { return };
	let chats = Rc::new(ListChats {
		dialog,
		list,
		list_label,
		remove_btn,
		announcer,
		list_id: id.clone(),
		jids: RefCell::new(Vec::new()),
	});
	{
		let s = state.borrow();
		if let Some(l) = s.chat_lists.iter().find(|l| l.id == id) {
			chats.apply(&s, l);
		}
	}
	*manager.chats.borrow_mut() = Some(chats.clone());

	let (state_add, chats_add, id_add) = (state.clone(), chats.clone(), id.clone());
	on_press(&add_btn, move || {
		let offered: Vec<crate::Contact> = {
			let s = state_add.borrow();
			let members = chats_add.jids.borrow();
			s.chats
				.iter()
				.filter(|j| !members.contains(j))
				.map(|j| crate::Contact {
					jid: j.clone(),
					name: s.contacts.get(j).cloned().unwrap_or_default(),
					phone: String::new(),
					saved: false,
				})
				.collect()
		};
		let name = list_name(&state_add, &id_add).unwrap_or_default();
		let words = crate::ui::dialogs::privacy_list_picker::PickerWords {
			search: "Search chats",
			on: "chosen",
			off: "not chosen",
			one: "chat",
			many: "chats",
			require_one: None,
		};
		let title = crate::i18n::tr_fmt("Add Chats to {}", &[&name]);
		let picked = crate::ui::dialogs::privacy_list_picker::pick_with(
			&dialog,
			&title,
			crate::i18n::tr("Check the chats to add"),
			&offered,
			&[],
			&words,
		);
		if let Some(jids) = picked.filter(|j| !j.is_empty()) {
			send(&state_add, "set_list_member", serde_json::json!({ "id": id_add, "jids": jids, "member": true }));
		}
	});

	let (state_remove, chats_remove) = (state.clone(), chats.clone());
	let remove = on_press(&remove_btn, move || {
		if let Some(jid) = chats_remove.selected() {
			send(
				&state_remove,
				"set_list_member",
				serde_json::json!({ "id": chats_remove.list_id, "jid": jid, "member": false }),
			);
		}
	});

	list_keys(dialog, list, remove, None, None);
	list.set_focus();
	dialog.show_modal();
	*manager.chats.borrow_mut() = None;
	dialog.destroy();
}

pub(crate) fn on_event(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, t: &str, data: &serde_json::Value) {
	match t {
		"chat_lists" => update_lists(state, parts, data),
		"favorite_saved" => {
			let name = chat_name(&state.borrow(), &text(data, "jid"));
			let favorite = data.get("favorite").and_then(|f| f.as_bool()).unwrap_or(false);
			let msg = if crate::i18n::is_indonesian() {
				if favorite {
					format!("{name} ditambahkan ke favorit.")
				} else {
					format!("{name} dihapus dari favorit.")
				}
			} else if favorite {
				format!("{name} added to favorites.")
			} else {
				format!("{name} removed from favorites.")
			};
			announce(state, parts, &msg);
		}
		"list_saved" => {
			let created = data.get("created").and_then(|c| c.as_bool()).unwrap_or(false);
			let verb = if created {
				crate::i18n::tr("created")
			} else { crate::i18n::tr("renamed") };
			let manager = state.borrow().list_manager.clone();
			if let Some(manager) = manager.filter(|_| created) {
				manager.select(&text(data, "id"));
			}
			let list_name = text(data, "name");
			let msg = crate::i18n::tr_fmt("List {} {}.", &[&list_name, &verb]);
			announce(state, parts, &msg);
		}
		"list_deleted" => {
			let list_name = text(data, "name");
			let msg = crate::i18n::tr_fmt("List {} deleted.", &[&list_name]);
			announce(state, parts, &msg);
		}
		"list_member_saved" => {
			let name = chat_name(&state.borrow(), &text(data, "jid"));
			let member = data.get("member").and_then(|m| m.as_bool()).unwrap_or(false);
			let count = data.get("count").and_then(|c| c.as_u64()).unwrap_or(1);
			let list = text(data, "name");
			let who = if count > 1 { count_chats(count as usize) } else { name };
			let msg = if crate::i18n::is_indonesian() {
				if member { format!("{who} ditambahkan ke {list}.") } else { format!("{who} dihapus dari {list}.") }
			} else if member {
				format!("{who} added to {list}.")
			} else {
				format!("{who} removed from {list}.")
			};
			announce(state, parts, &msg);
		}
		"common_groups" => show_common_groups(state, parts, data),
		_ => {}
	}
}

fn update_lists(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let lists: Vec<ChatList> = data
		.get("lists")
		.and_then(|l| l.as_array())
		.map(|a| {
			a.iter().map(|l| ChatList { id: text(l, "id"), name: text(l, "name"), jids: strings(l, "jids") }).collect()
		})
		.unwrap_or_default();
	let mut s = state.borrow_mut();
	let built_in = BUILT_IN_CHAT_FILTERS.len();
	let selected_id = s.chat_filter.checked_sub(built_in).and_then(|i| s.chat_lists.get(i)).map(|l| l.id.clone());
	let names_changed =
		lists.len() != s.chat_lists.len() || lists.iter().zip(&s.chat_lists).any(|(a, b)| a.name != b.name);
	if let Some(id) = selected_id {
		s.chat_filter = lists.iter().position(|l| l.id == id).map_or(0, |i| i + built_in);
	}
	s.favorite_jids = strings(data, "favorites");
	s.chat_lists = lists;

	let combo = &parts.conversations_parts.chat_filter_combo;
	if names_changed {
		combo.clear();
		for name in BUILT_IN_CHAT_FILTERS {
			combo.append(crate::i18n::tr(name));
		}
		for l in &s.chat_lists {
			combo.append(&l.name);
		}
	}
	if combo.get_selection() != Some(s.chat_filter as u32) {
		combo.set_selection(s.chat_filter as u32);
	}
	apply_chat_filter(&mut s, parts);
	drop(s);
	let s = state.borrow();
	if let Some(manager) = &s.list_manager {
		manager.apply(&s);
	}
}

fn show_common_groups(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let jid = text(data, "jid");
	let mut s = state.borrow_mut();
	if s.info_chat.as_deref() != Some(jid.as_str()) || !parts.conversation_data_dialog.dialog.is_shown() {
		return;
	}
	let groups = data.get("groups").and_then(|g| g.as_array()).cloned().unwrap_or_default();
	let list = &parts.conversation_data_dialog.common_groups_list;
	list.clear();
	s.common_group_jids = groups.iter().map(|g| text(g, "jid")).collect();
	if groups.is_empty() {
		list.append(crate::i18n::tr("No groups in common"));
	}
	for g in &groups {
		list.append(&text(g, "name"));
	}
	let label = if crate::i18n::is_indonesian() {
		match groups.len() {
			0 => "Grup bersama".to_string(),
			n => format!("{n} grup bersama"),
		}
	} else {
		match groups.len() {
			0 => "Groups in common".to_string(),
			1 => "1 group in common".to_string(),
			n => format!("{} groups in common", n),
		}
	};
	parts.conversation_data_dialog.common_groups_label.set_label(&label);
}

pub(crate) fn open_common_group(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, idx: usize) {
	let Some(jid) = state.borrow().common_group_jids.get(idx).cloned() else { return };
	parts.conversation_data_dialog.dialog.end_modal(wxdragon::id::ID_CANCEL);
	crate::ui::chat_ui::open_chat_for_jid(&jid, state, parts);
}
