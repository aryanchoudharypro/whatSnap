use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState, Contact, StatusGroup, StatusItem,
	ui::{message_display::apply_chat_filter, window::WindowParts},
};

const VIEW: i32 = 4300;
const REPLY: i32 = 4301;
const REACT: i32 = 4302;
const MUTE: i32 = 4303;
const VIEWERS: i32 = 4304;
const DELETE: i32 = 4305;
const PRIVACY: i32 = 4306;
const POST_TEXT: i32 = 4307;
const POST_MEDIA: i32 = 4308;

pub(crate) const STATUS_REACTIONS: [&str; 8] = ["😍", "😂", "😮", "😢", "🙏", "👏", "🎉", "💯"];
/// WA Web's status like (WAWebStatusLikeEmojis.STATUS_LIKE_EMOJI): a real
/// reaction, sent only to the poster, unlike the quick replies above.
pub(crate) const STATUS_LIKE: &str = "💚";

fn send(state: &Rc<RefCell<AppState>>, action: &str, data: serde_json::Value) {
	let payload = serde_json::json!({ "action": action, "data": data });
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

fn announce(parts: &WindowParts, message: &str) {
	crate::platform::announce::announce(parts.conversations_parts.messages_announcer, message);
}

/// The parts of the Status tab, in WA Web's order: your own updates, then
/// people with updates you haven't seen ("Recent"), people whose updates
/// you've all seen ("Viewed"), and muted people last, whatever they posted.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Section {
	Mine,
	Recent,
	Viewed,
	Muted,
}

impl Section {
	pub(crate) fn heading(self) -> Option<&'static str> {
		match self {
			Section::Mine => None,
			Section::Recent => Some(crate::i18n::tr("Recent updates")),
			Section::Viewed => Some(crate::i18n::tr("Viewed updates")),
			Section::Muted => Some(crate::i18n::tr("Muted updates")),
		}
	}
}

pub(crate) fn section_of(group: &StatusGroup) -> Section {
	if group.from_me {
		Section::Mine
	} else if group.muted {
		Section::Muted
	} else if group.items.iter().any(|it| !it.viewed) {
		Section::Recent
	} else {
		Section::Viewed
	}
}

fn latest(group: &StatusGroup) -> i64 {
	group.items.iter().map(|it| it.timestamp).max().unwrap_or(0)
}

/// Each person's updates as one row, like WA Web's status list: sorted by
/// section, then newest update first within a section. Empty groups (all
/// their updates deleted) are left out.
pub(crate) fn sorted_groups(groups: &[StatusGroup]) -> Vec<(Section, &StatusGroup)> {
	let mut sorted: Vec<(Section, &StatusGroup)> =
		groups.iter().filter(|g| !g.items.is_empty()).map(|g| (section_of(g), g)).collect();
	let rank = |s: Section| s as u8;
	sorted.sort_by(|(sa, a), (sb, b)| rank(*sa).cmp(&rank(*sb)).then_with(|| latest(b).cmp(&latest(a))));
	sorted
}

/// Where a person's row opens the viewer, as WA Web does: at their first
/// update you haven't seen, or their first update when you've seen them all.
pub(crate) fn start_item(group: &StatusGroup) -> Option<&StatusItem> {
	group.items.iter().find(|it| !it.viewed && !group.from_me).or_else(|| group.items.first())
}

struct Row {
	poster_jid: String,
	poster_name: String,
	from_me: bool,
	muted: bool,
	items: Vec<StatusItem>,
}

fn row_at(s: &AppState, idx: usize) -> Option<Row> {
	let poster_jid = s.filtered_chats.get(idx).filter(|j| !j.is_empty())?;
	if poster_jid == "me" {
		let group = s.status_groups.iter().find(|g| g.from_me);
		return Some(Row {
			poster_jid: "me".to_string(),
			poster_name: crate::i18n::tr("You").to_string(),
			from_me: true,
			muted: false,
			items: group.map(|g| g.items.clone()).unwrap_or_default(),
		});
	}
	s.filtered_status_message_ids.get(idx).filter(|id| !id.is_empty())?;
	let group = s.status_groups.iter().find(|g| &g.poster_jid == poster_jid)?;
	Some(Row {
		poster_jid: poster_jid.clone(),
		poster_name: group.poster_name.clone(),
		from_me: group.from_me,
		muted: group.muted,
		items: group.items.clone(),
	})
}

pub(crate) fn show_status_menu(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, idx: usize) {
	let Some(row) = row_at(&state.borrow(), idx) else { return };
	let mut builder = Menu::builder();
	if row.from_me {
		if row.items.is_empty() {
			builder = builder
				.append_item(
					POST_TEXT,
					crate::i18n::tr("Post &text status update..."),
					crate::i18n::tr("Compose a new text status update"),
				)
				.append_item(
					POST_MEDIA,
					crate::i18n::tr("Post &photo/video status update..."),
					crate::i18n::tr("Choose a photo or video to post as status"),
				);
		} else {
			builder = builder
				.append_item(VIEW, crate::i18n::tr("&View status"), crate::i18n::tr("View your status updates"))
				.append_item(
					POST_TEXT,
					crate::i18n::tr("Add &text status update..."),
					crate::i18n::tr("Compose a new text status update"),
				)
				.append_item(
					POST_MEDIA,
					crate::i18n::tr("Add &photo/video status update..."),
					crate::i18n::tr("Choose a photo or video to post as status"),
				)
				.append_item(
					VIEWERS,
					crate::i18n::tr("&Viewed by..."),
					crate::i18n::tr("See who has seen one of your status updates, and their reactions"),
				)
				.append_item(
					DELETE,
					crate::i18n::tr("&Delete..."),
					crate::i18n::tr("Delete one of your status updates for everyone"),
				);
		}
	} else {
		let mute = if row.muted {
			crate::i18n::tr_fmt("Un&mute {}'s status updates", &[&row.poster_name])
		} else { crate::i18n::tr_fmt("&Mute {}'s status updates", &[&row.poster_name]) };
		builder = builder
			.append_item(VIEW, crate::i18n::tr("&View status"), crate::i18n::tr("View their status updates"))
			.append_item(
				REPLY,
				crate::i18n::tr("&Reply..."),
				crate::i18n::tr("Reply to their latest status update in a chat with them"),
			)
			.append_item(REACT, crate::i18n::tr("Rea&ct..."), crate::i18n::tr("React to their latest status update"))
			.append_item(MUTE, &mute, crate::i18n::tr("Muted status updates are listed last"));
	}
	builder = builder.append_item(
		PRIVACY,
		crate::i18n::tr("Status &privacy..."),
		crate::i18n::tr("Choose who sees your status updates"),
	);
	let mut menu = builder.build();
	let state_menu = state.clone();
	let parts_menu = parts.clone();
	menu.on_selected(move |evt| on_menu_item(&state_menu, &parts_menu, evt.get_id(), &row));
	parts.conversations_parts.conversations_list.popup_menu(&mut menu, None);
	menu.destroy_menu();
	crate::platform::tray::enter_release_guard().set(true);
}

fn on_menu_item(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, id: i32, row: &Row) {
	match id {
		VIEW => {
			let start = row.items.iter().find(|it| !it.viewed && !row.from_me).or_else(|| row.items.first());
			if let Some(item) = start {
				let outcome = crate::ui::status_viewer::show(state, parts, &item.id);
				crate::platform::tray::enter_release_guard().set(true);
				apply_chat_filter(&mut state.borrow_mut(), parts);
				if let crate::ui::status_viewer::ViewerOutcome::OpenChat(jid) = outcome {
					crate::ui_commands::chat_list::nav_selected(state, parts, 0);
					crate::ui::chat_ui::open_chat_for_jid(&jid, state, parts);
				}
			}
		}
		POST_TEXT => {
			crate::ui::dialogs::status_composer::show_text_status_composer(&parts.frame, state, parts);
		}
		POST_MEDIA => {
			crate::ui::dialogs::status_composer::show_media_status_composer(&parts.frame, state, parts);
		}
		REPLY => {
			if let Some(item) = row.items.last() {
				reply(state, parts, row, item);
			}
		}
		REACT => {
			let Some(item) = row.items.last() else { return };
			let mut labels: Vec<String> = STATUS_REACTIONS.iter().map(|e| e.to_string()).collect();
			if !item.my_reaction.is_empty() {
				let label = crate::i18n::tr_fmt("Remove my reaction ({})", &[&item.my_reaction]);
				labels.push(label);
			}
			let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
			let dialog = SingleChoiceDialog::builder(
				&parts.frame,
				crate::i18n::tr("React with:"),
				crate::i18n::tr("React to Status"),
				&refs,
			)
			.build();
			if dialog.show_modal() != wxdragon::id::ID_OK {
				return;
			}
			let Ok(choice) = usize::try_from(dialog.get_selection()) else { return };
			let emoji = STATUS_REACTIONS.get(choice).copied().unwrap_or("");
			send(state, "react_status", serde_json::json!({ "id": item.id, "emoji": emoji }));
		}
		MUTE => send(state, "set_status_mute", serde_json::json!({ "jid": row.poster_jid, "muted": !row.muted })),
		VIEWERS => {
			if let Some(item) =
				pick_update(parts, row, crate::i18n::tr("Viewed By"), crate::i18n::tr("See who has seen:"))
			{
				send(state, "get_status_viewers", serde_json::json!({ "id": item.id }));
			}
		}
		DELETE => {
			let Some(item) = pick_update(
				parts,
				row,
				crate::i18n::tr("Delete Status"),
				crate::i18n::tr("Delete which status update?"),
			) else {
				return;
			};
			let dialog = MessageDialog::builder(
				&parts.frame,
				crate::i18n::tr("Delete this status update? It will be deleted for everyone who received it."),
				crate::i18n::tr("Delete Status"),
			)
			.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
			.build();
			dialog.set_yes_no_labels(crate::i18n::tr("Delete"), crate::i18n::tr("Cancel"));
			if dialog.show_modal() == ID_YES {
				send(state, "delete_status", serde_json::json!({ "id": item.id }));
			}
		}
		PRIVACY => send(state, "get_status_privacy", serde_json::json!({})),
		_ => {}
	}
}

/// What a status update is, in a few words: its text, or what kind of media.
pub(crate) fn describe(item: &StatusItem) -> String {
	match (item.media_type.as_str(), item.text.is_empty()) {
		("", _) => item.text.clone(),
		("video", true) => crate::i18n::tr("Video").to_string(),
		("video", false) => format!("{}: {}", crate::i18n::tr("Video"), item.text),
		("image", true) => crate::i18n::tr("Photo").to_string(),
		("image", false) => format!("{}: {}", crate::i18n::tr("Photo"), item.text),
		(_, true) => crate::i18n::tr("Media").to_string(),
		(_, false) => format!("{}: {}", crate::i18n::tr("Media"), item.text),
	}
}

fn when(item: &StatusItem) -> String {
	whatsnap::LocalTime::from_unix(item.timestamp).map(whatsnap::time_or_day).unwrap_or_default()
}

/// Your own row holds all your updates; with more than one, ask which,
/// newest first like WA Web's My status list.
fn pick_update<'a>(parts: &Rc<WindowParts>, row: &'a Row, title: &str, prompt: &str) -> Option<&'a StatusItem> {
	if row.items.len() <= 1 {
		return row.items.first();
	}
	let newest_first: Vec<&StatusItem> = row.items.iter().rev().collect();
	let labels: Vec<String> = newest_first
		.iter()
		.map(|it| {
			let views = if crate::i18n::is_indonesian() {
				format!("{} dilihat", it.viewers)
			} else if it.viewers == 1 {
				"1 view".to_string()
			} else {
				format!("{} views", it.viewers)
			};
			format!("{}, {}, {}", describe(it), when(it), views)
		})
		.collect();
	let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
	let dialog = SingleChoiceDialog::builder(&parts.frame, prompt, title, &refs).build();
	if dialog.show_modal() != wxdragon::id::ID_OK {
		return None;
	}
	usize::try_from(dialog.get_selection()).ok().and_then(|i| newest_first.get(i).copied())
}

fn reply(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, row: &Row, item: &StatusItem) {
	crate::ui::chat_ui::open_chat_for_jid(&row.poster_jid, state, parts);
	let what = if !item.text.is_empty() {
		item.text.clone()
	} else if item.media_type == "video" {
		crate::i18n::tr("Video").to_string()
	} else {
		crate::i18n::tr("Photo").to_string()
	};
	let preview = crate::i18n::tr_fmt("Replying to {}'s status: {}", &[&row.poster_name, &what]);
	{
		let mut s = state.borrow_mut();
		s.pending_reply_id = Some(item.id.clone());
		s.pending_reply_sender = Some(row.poster_jid.clone());
		s.pending_reply_preview = Some(preview.clone());
	}
	let c = &parts.conversations_parts;
	c.remove_quote_btn.show(true);
	c.conversation_panel.layout();
	c.message_field.set_focus();
	let spoken = crate::i18n::tr_fmt("{}. Type your reply and press Enter.", &[&preview]);
	announce(parts, &spoken);
}

fn text(data: &serde_json::Value, key: &str) -> String {
	data.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn update_item(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	id: &str,
	change: impl FnOnce(&mut crate::StatusItem),
) {
	let mut s = state.borrow_mut();
	if let Some(item) = s.status_groups.iter_mut().flat_map(|g| g.items.iter_mut()).find(|it| it.id == id) {
		change(item);
		if s.active_tab == 1 {
			apply_chat_filter(&mut s, parts);
		}
	}
}

pub(crate) fn on_event(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, t: &str, data: &serde_json::Value) {
	if crate::ui::status_viewer::on_event(t, data) {
		return;
	}
	match t {
		"status_viewers" => show_viewers(state, parts, data),
		"status_privacy" => choose_privacy(state, parts, data),
		"status_privacy_saved" => announce(parts, crate::i18n::tr("Status privacy saved.")),
		"status_deleted" => {
			let id = text(data, "id");
			let mut s = state.borrow_mut();
			for group in s.status_groups.iter_mut() {
				group.items.retain(|it| it.id != id);
			}
			s.status_groups.retain(|g| !g.items.is_empty());
			if s.active_tab == 1 {
				apply_chat_filter(&mut s, parts);
			}
		}
		"status_muted" => {
			let (jid, muted) = (text(data, "jid"), data.get("muted").and_then(|v| v.as_bool()).unwrap_or(false));
			let mut s = state.borrow_mut();
			let Some(group) = s.status_groups.iter_mut().find(|g| g.poster_jid == jid) else { return };
			group.muted = muted;
			let name = group.poster_name.clone();
			if s.active_tab == 1 {
				apply_chat_filter(&mut s, parts);
			}
			drop(s);
			let spoken = if crate::i18n::is_indonesian() {
				if muted {
					format!("Pembaruan status {name} dibisukan.")
				} else {
					format!("Pembaruan status {name} dibunyikan.")
				}
			} else {
				let verb = if muted { "muted" } else { "unmuted" };
				format!("{name}'s status updates {verb}.")
			};
			announce(parts, &spoken);
		}
		"status_viewed" => update_item(state, parts, &text(data, "id"), |it| it.viewers += 1),
		"status_reaction" => {
			let reactor = text(data, "reactor");
			let emoji = text(data, "emoji");
			let spoken = crate::i18n::tr_fmt("{} reacted {} to your status.", &[&reactor, &emoji]);
			announce(parts, &spoken);
		}
		"status_reacted" => {
			let emoji = text(data, "emoji");
			let spoken = if data.get("like").and_then(|v| v.as_bool()).unwrap_or(false) {
				if emoji.is_empty() {
					crate::i18n::tr("Like removed.").to_string()
				} else {
					crate::i18n::tr("Liked.").to_string()
				}
			} else if emoji.is_empty() {
				crate::i18n::tr("Reaction removed.").to_string()
			} else if crate::i18n::is_indonesian() {
				format!("Anda memberi reaksi {emoji}.")
			} else {
				format!("You reacted {emoji}.")
			};
			update_item(state, parts, &text(data, "id"), |it| it.my_reaction = emoji);
			announce(parts, &spoken);
		}
		_ => {}
	}
}

fn show_viewers(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let viewers = data.get("viewers").and_then(|v| v.as_array()).cloned().unwrap_or_default();
	if viewers.is_empty() {
		MessageDialog::builder(
			&parts.frame,
			crate::i18n::tr("No one has seen this status update yet."),
			crate::i18n::tr("Viewed By"),
		)
		.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
		.build()
		.show_modal();
		return;
	}
	let mut items = Vec::new();
	let mut labels: Vec<String> = Vec::new();
	for v in &viewers {
		let jid = text(v, "jid");
		let name = text(v, "name");
		let time = v
			.get("timestamp")
			.and_then(|t| t.as_i64())
			.and_then(whatsnap::LocalTime::from_unix)
			.map(whatsnap::time_or_day)
			.unwrap_or_default();
		let reaction = text(v, "reaction");
		let mut label = format!("{name}, {time}");
		if !reaction.is_empty() {
			if crate::i18n::is_indonesian() {
				label.push_str(&format!(", bereaksi {reaction}"));
			} else {
				label.push_str(&format!(", reacted {reaction}"));
			}
		}
		items.push((jid, name));
		labels.push(label);
	}
	let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
	let count = labels.len();
	let title = if crate::i18n::is_indonesian() {
		format!("Dilihat oleh {count} orang")
	} else {
		format!("Viewed by {count} {}", if count == 1 { "person" } else { "people" })
	};
	let dialog = SingleChoiceDialog::builder(&parts.frame, &title, crate::i18n::tr("Viewed By"), &refs).build();
	if dialog.show_modal() == wxdragon::id::ID_OK
		&& let Ok(choice) = usize::try_from(dialog.get_selection())
		&& let Some((jid, name)) = items.get(choice)
		&& !jid.is_empty()
	{
		let question = crate::i18n::tr_fmt("Open chat with {}?", &[&name]);
		let confirm = MessageDialog::builder(&parts.frame, &question, crate::i18n::tr("Open Chat"))
			.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
			.build();
		if confirm.show_modal() == ID_YES {
			crate::ui_commands::chat_list::nav_selected(state, parts, 0);
			crate::ui::chat_ui::open_chat_for_jid(jid, state, parts);
		}
	}
}

fn choose_privacy(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	const MODES: [(&str, &str); 3] =
		[("contacts", "My contacts"), ("except", "My contacts except..."), ("only", "Only share with...")];
	let current = text(data, "mode");
	let listed: Vec<Contact> = data
		.get("people")
		.and_then(|p| p.as_array())
		.map(|people| {
			people
				.iter()
				.map(|p| Contact { jid: text(p, "jid"), name: text(p, "name"), phone: String::new(), saved: false })
				.collect()
		})
		.unwrap_or_default();
	let labels: Vec<&str> = MODES.iter().map(|(_, label)| crate::i18n::tr(label)).collect();
	let dialog = SingleChoiceDialog::builder(
		&parts.frame,
		crate::i18n::tr("Who can see your status updates:"),
		crate::i18n::tr("Status Privacy"),
		&labels,
	)
	.build();
	dialog.set_selection(MODES.iter().position(|(mode, _)| *mode == current).unwrap_or(0) as i32);
	if dialog.show_modal() != wxdragon::id::ID_OK {
		return;
	}
	let Some(&(mode, _)) = usize::try_from(dialog.get_selection()).ok().and_then(|i| MODES.get(i)) else { return };
	let mut jids: Vec<String> = Vec::new();
	if mode != "contacts" {
		let mut contacts = state.borrow().all_contacts.clone();
		if contacts.is_empty() {
			for (jid, name) in &state.borrow().contacts {
				if !jid.ends_with("@g.us") && !jid.ends_with("@broadcast") && !jid.ends_with("@newsletter") {
					contacts.push(Contact { jid: jid.clone(), name: name.clone(), phone: String::new(), saved: false });
				}
			}
			contacts.sort_by(|a, b| a.name.cmp(&b.name));
		}
		let preset: &[Contact] = if mode == current { &listed } else { &[] };
		let (title, hint) = if mode == "except" {
			(
				crate::i18n::tr("Status: My Contacts Except..."),
				crate::i18n::tr("Checked contacts won't see your status updates"),
			)
		} else {
			(
				crate::i18n::tr("Status: Only Share With..."),
				crate::i18n::tr("Only checked contacts will see your status updates"),
			)
		};
		let Some(picked) =
			crate::ui::dialogs::privacy_list_picker::pick_contacts(&parts.frame, title, hint, &contacts, preset)
		else {
			return;
		};
		if mode == "only" && picked.is_empty() {
			announce(parts, crate::i18n::tr("Choose at least one contact to share with."));
			return;
		}
		jids = picked;
	}
	send(state, "set_status_privacy", serde_json::json!({ "mode": mode, "jids": jids }));
	announce(parts, crate::i18n::tr("Saving who sees your status..."));
}
