//! The Conversations filter (All chats, Unread, Favorites, Groups and the
//! user's own lists, as WA Web's filter chips have them), adding a chat to
//! the favorites or a list from its context menu, Chat menu > "Manage
//! Lists...", and contact info's groups in common. What the backend answers:
//! `chat_lists`, `favorite_saved`, `list_saved`, `list_deleted`,
//! `list_member_saved` and `common_groups`.

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState,
	message_display::{BUILT_IN_CHAT_FILTERS, apply_chat_filter},
	ui::{theme, window::WindowParts},
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

fn announce(parts: &WindowParts, message: &str) {
	crate::announce::announce(parts.conversations_parts.messages_announcer.clone(), message);
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
	s.contacts.get(jid).cloned().unwrap_or_else(|| jid.to_string())
}

fn ask_name(parent: &impl WxWidget, prompt: &str, title: &str, value: &str) -> Option<String> {
	let dialog = TextEntryDialog::builder(parent, prompt, title).with_default_value(value).build();
	if dialog.show_modal() != wxdragon::id::ID_OK {
		return None;
	}
	Some(dialog.get_value().unwrap_or_default().trim().to_string()).filter(|n| !n.is_empty())
}

/// The filter dropdown changed: show those chats, and say how many there are.
pub(crate) fn set_filter(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, idx: usize) {
	let mut s = state.borrow_mut();
	s.chat_filter = idx;
	apply_chat_filter(&mut s, parts);
	let count = s.filtered_chats.len();
	drop(s);
	announce(parts, &if count == 1 { "1 chat".to_string() } else { format!("{} chats", count) });
}

pub(crate) fn set_favorite(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>, jid: String, favorite: bool) {
	send(state, "set_favorite", serde_json::json!({ "jid": jid, "favorite": favorite }));
}

/// A chat's context menu > "Add to list...": tick the lists it should be in.
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

	let dialog = Dialog::builder(&parts.frame, "Lists")
		.with_size(380, 420)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();
	let label = StaticText::builder(&panel).with_label(&format!("Lists {} is in", name)).build();
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
	let done_btn = Button::builder(&panel).with_id(ID_OK).with_label("Done").build();
	theme::style_primary_button(&done_btn);
	let new_btn = Button::builder(&panel).with_label("New List...").build();
	let cancel_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Cancel").build();
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

	// "New List..." closes this with its own code, then asks for the name.
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

/// Asks for a new list's name, and makes it, with `jid` in it if given.
fn new_list(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: Option<&str>, chat: &str) {
	let prompt = match jid {
		Some(_) => format!("Name a new list for {}:", chat),
		None => "List name:".to_string(),
	};
	let Some(name) = ask_name(&parts.frame, &prompt, "New List", "") else { return };
	let jids: Vec<&str> = jid.into_iter().collect();
	send(state, "create_list", serde_json::json!({ "name": name, "jids": jids }));
}

/// Chat menu > "Manage Lists...": make a list, or rename or delete one.
pub(crate) fn manage_lists(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let lists: Vec<(String, String, usize)> =
		state.borrow().chat_lists.iter().map(|l| (l.id.clone(), l.name.clone(), l.jids.len())).collect();
	let mut labels = vec!["New list...".to_string()];
	for (_, name, count) in &lists {
		labels.push(format!(
			"{}, {}",
			name,
			if *count == 1 { "1 chat".to_string() } else { format!("{} chats", count) }
		));
	}
	let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
	let choice = SingleChoiceDialog::builder(&parts.frame, "Your lists:", "Manage Lists", &refs).build();
	if choice.show_modal() != wxdragon::id::ID_OK {
		return;
	}
	let Ok(picked) = usize::try_from(choice.get_selection()) else { return };
	if picked == 0 {
		new_list(state, parts, None, "");
		return;
	}
	let Some((id, name, _)) = lists.get(picked - 1).cloned() else { return };
	let what = SingleChoiceDialog::builder(&parts.frame, &name, "Manage Lists", &["Rename...", "Delete..."]).build();
	if what.show_modal() != wxdragon::id::ID_OK {
		return;
	}
	if what.get_selection() == 0 {
		if let Some(new_name) = ask_name(&parts.frame, "List name:", "Rename List", &name) {
			if new_name != name {
				send(state, "rename_list", serde_json::json!({ "id": id, "name": new_name }));
			}
		}
		return;
	}
	let dialog = MessageDialog::builder(
		&parts.frame,
		&format!("Delete the list {}? The chats in it stay as they are.", name),
		"Delete List",
	)
	.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
	.build();
	dialog.set_yes_no_labels("Delete", "Cancel");
	if dialog.show_modal() == ID_YES {
		send(state, "delete_list", serde_json::json!({ "id": id }));
	}
}

pub(crate) fn on_event(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, t: &str, data: &serde_json::Value) {
	match t {
		"chat_lists" => update_lists(state, parts, data),
		"favorite_saved" => {
			let name = chat_name(&state.borrow(), &text(data, "jid"));
			let favorite = data.get("favorite").and_then(|f| f.as_bool()).unwrap_or(false);
			announce(
				parts,
				&if favorite {
					format!("{} added to favorites.", name)
				} else {
					format!("{} removed from favorites.", name)
				},
			);
		}
		"list_saved" => {
			let created = data.get("created").and_then(|c| c.as_bool()).unwrap_or(false);
			let verb = if created { "created" } else { "renamed" };
			announce(parts, &format!("List {} {}.", text(data, "name"), verb));
		}
		"list_deleted" => announce(parts, &format!("List {} deleted.", text(data, "name"))),
		"list_member_saved" => {
			let name = chat_name(&state.borrow(), &text(data, "jid"));
			let member = data.get("member").and_then(|m| m.as_bool()).unwrap_or(false);
			let list = text(data, "name");
			announce(
				parts,
				&if member {
					format!("{} added to {}.", name, list)
				} else {
					format!("{} removed from {}.", name, list)
				},
			);
		}
		"common_groups" => show_common_groups(state, parts, data),
		_ => {}
	}
}

/// The favorites or the lists changed: keep the filter on the same list if
/// it's still there, and refilter.
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
			combo.append(name);
		}
		for l in &s.chat_lists {
			combo.append(&l.name);
		}
	}
	if combo.get_selection() != Some(s.chat_filter as u32) {
		combo.set_selection(s.chat_filter as u32);
	}
	apply_chat_filter(&mut s, parts);
}

fn show_common_groups(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let jid = text(data, "jid");
	let mut s = state.borrow_mut();
	if s.selected_chat.as_deref() != Some(jid.as_str()) || !parts.conversation_data_dialog.dialog.is_shown() {
		return;
	}
	let groups = data.get("groups").and_then(|g| g.as_array()).cloned().unwrap_or_default();
	let list = &parts.conversation_data_dialog.common_groups_list;
	list.clear();
	s.common_group_jids = groups.iter().map(|g| text(g, "jid")).collect();
	if groups.is_empty() {
		list.append("No groups in common");
	}
	for g in &groups {
		list.append(&text(g, "name"));
	}
	let label = match groups.len() {
		0 => "Groups in common".to_string(),
		1 => "1 group in common".to_string(),
		n => format!("{} groups in common", n),
	};
	parts.conversation_data_dialog.common_groups_label.set_label(&label);
}

/// Enter or a double click on one of contact info's groups in common: open it.
pub(crate) fn open_common_group(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, idx: usize) {
	let Some(jid) = state.borrow().common_group_jids.get(idx).cloned() else { return };
	parts.conversation_data_dialog.dialog.end_modal(wxdragon::id::ID_CANCEL);
	crate::chat_ui::open_chat_for_jid(&jid, state, parts);
}
