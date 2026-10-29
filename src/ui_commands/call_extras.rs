//! Call links (Call > "Create Call Link..." and "Join Call Link...", and
//! call.whatsapp.com links), and the call window's "Raise hand" and
//! "React...", plus its people list: who's in a group call and who's waiting
//! to be let in through a call link, with "Let in", "Don't let in" and
//! "Approve new participants". What the backend answers: `call_link_created`,
//! `call_hand`, `call_reaction`, `call_waiting_room`, `call_action_done` and
//! `call_action_failed`. Screen sharing lives with the camera in `calls.rs`.

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState, CallParticipant, CallVideo,
	call_helpers::{announce_call, call_window_title, is_ringing_state},
	ui::window::WindowParts,
};

/// The reactions offered in a call, as WA Web's call reaction tray has them.
const CALL_REACTIONS: [&str; 6] = ["👍", "❤️", "😂", "😮", "😢", "🙏"];

fn send(s: &AppState, action: &str, data: serde_json::Value) {
	let payload = serde_json::json!({ "action": action, "data": data });
	let _ = s.ws_tx.send(payload.to_string());
}

fn text(data: &serde_json::Value, key: &str) -> String {
	data.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn this_call(s: &AppState, data: &serde_json::Value) -> bool {
	s.active_call_id.as_deref() == Some(text(data, "call_id").as_str())
}

/// Raising a hand is a group call thing.
pub(crate) fn show_call_buttons(s: &AppState, parts: &WindowParts) {
	let group = s.call_video.peer_name.is_none();
	let c = &parts.call_parts;
	c.raise_hand_btn.set_label(if s.call_video.hand_raised { "Lower hand" } else { "Raise hand" });
	c.raise_hand_btn.show(group);
	c.react_btn.show(true);
	c.frame.layout();
}

/// Until a call connects there's no hand to raise, and until a group roster
/// arrives no one to list.
pub(crate) fn hide_call_buttons(parts: &WindowParts) {
	let c = &parts.call_parts;
	c.raise_hand_btn.show(false);
	c.react_btn.show(false);
	c.people_panel.show(false);
	c.caller_info.show(false);
}

/// One row of the call window's people list.
struct PersonRow {
	jid: String,
	name: String,
	label: String,
	waiting: bool,
	connected: bool,
}

fn people_rows(s: &AppState) -> Vec<PersonRow> {
	let mut rows: Vec<PersonRow> = s
		.call_video
		.waiting
		.iter()
		.map(|p| PersonRow {
			jid: p.jid.clone(),
			name: p.name.clone(),
			label: format!("{}, wants to join", p.name),
			waiting: true,
			connected: false,
		})
		.collect();
	for p in &s.call_participants {
		let state = match p.state.as_str() {
			"connected" => "in the call",
			"invited" => "invited",
			"rejected" => "declined",
			st if is_ringing_state(st) => "ringing",
			"" => "joining",
			other => other,
		};
		rows.push(PersonRow {
			jid: p.jid.clone(),
			name: p.name.clone(),
			label: format!("{}, {}", p.name, state),
			waiting: false,
			connected: p.state == "connected",
		});
	}
	rows
}

/// Fills the people list from the latest roster and waiting room, keeping
/// the selected person selected. Shown once a group call has a roster or
/// we run a call link.
pub(crate) fn refresh_people(s: &AppState, parts: &WindowParts) {
	let c = &parts.call_parts;
	let v = &s.call_video;
	if !v.roster_seen && !v.link_admin && v.waiting.is_empty() {
		c.people_panel.show(false);
		return;
	}
	let rows = people_rows(s);
	let selected = c.people_list.get_selection().and_then(|i| people_rows_jid_at(c.people_list.get_string(i), &rows));
	let labels: Vec<&str> = if rows.is_empty() {
		vec!["No one else is here yet."]
	} else {
		rows.iter().map(|r| r.label.as_str()).collect()
	};
	// Rewriting identical rows would make a screen reader re-read the list.
	let current: Vec<String> = (0..c.people_list.get_count()).filter_map(|i| c.people_list.get_string(i)).collect();
	if current.iter().map(String::as_str).ne(labels.iter().copied()) {
		c.people_list.set_items(&labels);
		let index = selected.and_then(|jid| rows.iter().position(|r| r.jid == jid)).unwrap_or(0);
		c.people_list.set_selection(index as u32, true);
	}
	person_buttons(s, parts);
	let admitting = v.link_admin && !v.waiting.is_empty();
	c.let_in_btn.show(admitting);
	c.keep_out_btn.show(admitting);
	c.approval_check.show(v.link_admin);
	c.approval_check.set_value(v.approval_required);
	c.people_panel.show(true);
	c.people_panel.layout();
	c.frame.layout();
}

/// Which person a list row was for, by its label - the list is rebuilt from
/// state, so the label is the stable thing to match on.
fn people_rows_jid_at(label: Option<String>, rows: &[PersonRow]) -> Option<String> {
	let label = label?;
	rows.iter().find(|r| r.label == label).map(|r| r.jid.clone())
}

/// The person selected under People, if they're in the call.
fn selected_person(s: &AppState, parts: &WindowParts) -> Option<PersonRow> {
	let rows = people_rows(s);
	let list = &parts.call_parts.people_list;
	let jid = list.get_selection().and_then(|i| people_rows_jid_at(list.get_string(i), &rows))?;
	rows.into_iter().find(|r| r.jid == jid && r.connected)
}

/// "Mute {name}" for whoever is selected and in the call, and "Remove
/// {name}" too when we can remove people.
pub(crate) fn person_buttons(s: &AppState, parts: &WindowParts) {
	let c = &parts.call_parts;
	let person = selected_person(s, parts);
	// A call link's admins can remove people too; the waiting room says
	// we're one before the next roster does.
	let can_remove = s.call_video.can_remove || s.call_video.link_admin;
	match &person {
		Some(p) => {
			c.mute_person_btn.set_label(&format!("Mute {}", p.name));
			c.remove_person_btn.set_label(&format!("Remove {}", p.name));
		}
		None => {
			c.mute_person_btn.set_label("Mute");
			c.remove_person_btn.set_label("Remove");
		}
	}
	c.mute_person_btn.show(person.is_some());
	c.remove_person_btn.show(person.is_some() && can_remove);
	c.people_panel.layout();
}

/// "Mute {name}" (remove false) and "Remove {name}" (remove true).
pub(crate) fn act_on_person(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, remove: bool) {
	let s = state.borrow();
	let Some(call_id) = s.active_call_id.clone() else { return };
	let Some(person) = selected_person(&s, parts) else {
		drop(s);
		announce_call(parts, "Select someone in the call first.");
		return;
	};
	let action = if remove { "remove_call_participant" } else { "mute_call_participant" };
	send(&s, action, serde_json::json!({ "call_id": call_id, "jid": person.jid }));
	drop(s);
	let what = if remove { "Removing" } else { "Asking" };
	let tail = if remove { "from the call." } else { "to mute." };
	announce_call(parts, &format!("{} {} {}", what, person.name, tail));
}

/// Someone else in the call muted us: meowcaller has already told the call,
/// so only our microphone and the Mute button need to follow.
fn muted_by(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let s = state.borrow();
	if let Some(audio) = &s.active_call_audio {
		audio.is_muted.store(true, std::sync::atomic::Ordering::Relaxed);
	}
	parts.call_parts.mute_btn.set_label("Unmute microphone");
	drop(s);
	let name = text(data, "name");
	if name.is_empty() {
		announce_call(parts, "Someone in the call muted you.");
	} else {
		announce_call(parts, &format!("{} muted you.", name));
	}
}

/// "Let in" and "Don't let in": the selected person, or the only one waiting.
pub(crate) fn admit_selected(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, admit: bool) {
	let s = state.borrow();
	let Some(call_id) = s.active_call_id.clone() else { return };
	let rows = people_rows(&s);
	let list = &parts.call_parts.people_list;
	let chosen = list
		.get_selection()
		.and_then(|i| people_rows_jid_at(list.get_string(i), &rows))
		.and_then(|jid| rows.iter().find(|r| r.jid == jid && r.waiting))
		.or_else(|| {
			let mut waiting = rows.iter().filter(|r| r.waiting);
			match (waiting.next(), waiting.next()) {
				(Some(only), None) => Some(only),
				_ => None,
			}
		});
	let Some(row) = chosen else {
		drop(s);
		announce_call(parts, "Select someone who wants to join first.");
		return;
	};
	send(&s, "admit_call_participant", serde_json::json!({ "call_id": call_id, "jid": row.jid, "on": admit }));
}

/// "Approve new participants": whether people need letting in.
pub(crate) fn set_approval(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>, on: bool) {
	let s = state.borrow();
	let Some(call_id) = s.active_call_id.clone() else { return };
	send(&s, "set_call_approval", serde_json::json!({ "call_id": call_id, "on": on }));
}

/// Call > "Create Call Link...": a voice or video link to share.
pub(crate) fn create_call_link(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let choice =
		SingleChoiceDialog::builder(&parts.frame, "What kind of call?", "Create Call Link", &["Voice", "Video"])
			.build();
	if choice.show_modal() != wxdragon::id::ID_OK {
		return;
	}
	send(&state.borrow(), "create_call_link", serde_json::json!({ "video": choice.get_selection() == 1 }));
}

/// Call > "Join Call Link...": paste a link someone shared.
pub(crate) fn ask_call_link(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let dialog = TextEntryDialog::builder(&parts.frame, "Call link:", "Join Call Link").build();
	if dialog.show_modal() != wxdragon::id::ID_OK {
		return;
	}
	let link = dialog.get_value().unwrap_or_default();
	match whatsnap::parse_whatsapp_link(&link) {
		Some(whatsnap::WhatsAppLink::Call { token, video }) => join_call_link(state, parts, token, video),
		_ => {
			MessageDialog::builder(&parts.frame, "That isn't a WhatsApp call link.", "Join Call Link")
				.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconWarning)
				.build()
				.show_modal();
		}
	}
}

/// Joins the call a link is for, as an outgoing group call.
pub(crate) fn join_call_link(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, token: String, video: bool) {
	let mut s = state.borrow_mut();
	if s.active_call_id.is_some() || s.call_placing {
		drop(s);
		announce_call(parts, "You're already in a call.");
		return;
	}
	send(&s, "join_call_link", serde_json::json!({ "link": token, "video": video }));
	s.call_placing = true;
	s.call_pending_cancel = false;
	s.call_video = CallVideo { autostart_camera: video, video_call: video, ..Default::default() };
	let c = &parts.call_parts;
	c.frame.set_title(call_window_title(true, video));
	c.status_label.set_label("Joining call...");
	c.accept_btn.show(false);
	c.reject_btn.show(false);
	c.mute_btn.show(true);
	c.camera_btn.set_label("Turn camera on");
	c.camera_btn.show(false);
	c.add_participant_btn.show(false);
	hide_call_buttons(parts);
	c.leave_btn.show(true);
	c.frame.show(true);
	drop(s);
	announce_call(parts, "Joining call...");
}

pub(crate) fn toggle_hand(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>) {
	let s = state.borrow();
	let Some(call_id) = s.active_call_id.clone() else { return };
	send(&s, "raise_hand", serde_json::json!({ "call_id": call_id, "on": !s.call_video.hand_raised }));
}

pub(crate) fn react(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let Some(call_id) = state.borrow().active_call_id.clone() else { return };
	let choice = SingleChoiceDialog::builder(&parts.call_parts.frame, "React with:", "React", &CALL_REACTIONS).build();
	if choice.show_modal() != wxdragon::id::ID_OK {
		return;
	}
	let Some(emoji) = usize::try_from(choice.get_selection()).ok().and_then(|i| CALL_REACTIONS.get(i)) else { return };
	send(&state.borrow(), "call_reaction", serde_json::json!({ "call_id": call_id, "emoji": emoji }));
	announce_call(parts, &format!("You reacted {}.", emoji));
}

pub(crate) fn on_event(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, t: &str, data: &serde_json::Value) {
	if t == "call_link_created" {
		show_call_link(state, parts, text(data, "url"), data.get("video").and_then(|v| v.as_bool()).unwrap_or(false));
		return;
	}
	if !this_call(&state.borrow(), data) {
		return;
	}
	let on = data.get("on").and_then(|v| v.as_bool()).unwrap_or(false);
	match t {
		"call_hand" => {
			let raised = data.get("raised").and_then(|v| v.as_bool()).unwrap_or(false);
			let verb = if raised { "raised" } else { "lowered" };
			announce_call(parts, &format!("{} {} their hand.", text(data, "name"), verb));
		}
		"call_reaction" => announce_call(parts, &format!("{} reacted {}.", text(data, "name"), text(data, "emoji"))),
		"call_waiting_room" => waiting_room(state, parts, data),
		"call_muted_by" => muted_by(state, parts, data),
		"call_action_done" => match text(data, "what").as_str() {
			"hand" => {
				let mut s = state.borrow_mut();
				s.call_video.hand_raised = on;
				show_call_buttons(&s, parts);
				drop(s);
				announce_call(parts, if on { "Your hand is raised." } else { "You lowered your hand." });
			}
			"approval" => {
				let mut s = state.borrow_mut();
				s.call_video.approval_required = on;
				refresh_people(&s, parts);
				drop(s);
				announce_call(
					parts,
					if on {
						"New people now wait for you to let them in."
					} else {
						"Anyone with the link can now join straight away."
					},
				);
			}
			"admit" => announce_call(parts, if on { "They're being let in." } else { "They won't be let in." }),
			_ => {}
		},
		"call_action_failed" => {
			if text(data, "what").as_str() == "approval" {
				// The switch moved before the server agreed; put it back.
				let s = state.borrow();
				parts.call_parts.approval_check.set_value(s.call_video.approval_required);
			}
			announce_call(parts, &text(data, "message"));
		}
		_ => {}
	}
}

/// We're waiting to be let in, or (as the link's admin) someone is. New
/// arrivals are announced and listed under People, where "Let in" admits them.
fn waiting_room(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	if data.get("waiting").and_then(|v| v.as_bool()).unwrap_or(false) {
		parts.call_parts.status_label.set_label("Waiting for someone in the call to let you in...");
		announce_call(parts, "Waiting for someone in the call to let you in.");
		return;
	}
	let mut s = state.borrow_mut();
	s.call_video.link_admin = data.get("is_admin").and_then(|v| v.as_bool()).unwrap_or(false);
	s.call_video.approval_required = data.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
	let users = data.get("users").and_then(|u| u.as_array()).cloned().unwrap_or_default();
	let waiting: Vec<CallParticipant> = users
		.iter()
		.filter(|u| !text(u, "jid").is_empty())
		.map(|u| CallParticipant { jid: text(u, "jid"), name: text(u, "name"), state: text(u, "state") })
		.collect();
	let arrived: Vec<String> = waiting
		.iter()
		.filter(|p| !s.call_video.waiting.iter().any(|old| old.jid == p.jid))
		.map(|p| p.name.clone())
		.collect();
	s.call_video.waiting = waiting;
	refresh_people(&s, parts);
	let admin = s.call_video.link_admin;
	drop(s);
	if admin && !arrived.is_empty() {
		let names: Vec<&str> = arrived.iter().map(String::as_str).collect();
		let verb = if arrived.len() == 1 { "wants" } else { "want" };
		announce_call(
			parts,
			&format!(
				"{} {} to join the call. Choose Let in or Don't let in under People.",
				crate::call_helpers::join_names(&names),
				verb
			),
		);
	}
}

/// A new call link: copy it, send it to chats, or join the call now.
fn show_call_link(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, url: String, video: bool) {
	let kind = if video { "video" } else { "voice" };
	let dialog = Dialog::builder(&parts.frame, "Call Link").with_size(460, 220).build();
	crate::ui::theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	crate::ui::theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();
	let intro = StaticText::builder(&panel)
		.with_label(&format!("Anyone with WhatsApp can use this link to join your {} call.", kind))
		.build();
	sizer.add(&intro, 0, SizerFlag::All, 10);
	let link_field = TextCtrl::builder(&panel).with_style(TextCtrlStyle::ReadOnly).build();
	link_field.set_value(&url);
	sizer.add(&link_field, 0, SizerFlag::Expand | SizerFlag::All, 10);
	let buttons = BoxSizer::builder(Orientation::Horizontal).build();
	let copy_btn = Button::builder(&panel).with_label("Copy link").build();
	let send_btn = Button::builder(&panel).with_label("Send link...").build();
	let join_btn = Button::builder(&panel).with_label("Join call").build();
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Close").build();
	for button in [&copy_btn, &send_btn, &join_btn] {
		buttons.add(button, 0, SizerFlag::Right, 8);
	}
	buttons.add_stretch_spacer(1);
	buttons.add(&close_btn, 0, SizerFlag::empty(), 0);
	sizer.add_sizer(&buttons, 0, SizerFlag::Expand | SizerFlag::All, 10);
	let announcer = StaticText::builder(&panel).with_label("").build();
	live_region::set_live_region(&announcer);
	sizer.add(&announcer, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom, 10);
	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	const SEND: i32 = 1;
	const JOIN: i32 = 2;
	let url_copy = url.clone();
	copy_btn.on_click(move |_| {
		let copied = crate::copy_text(&url_copy);
		crate::announce::announce(announcer, if copied { "Link copied." } else { "Couldn't copy the link." });
	});
	send_btn.on_click(move |_| dialog.end_modal(SEND));
	join_btn.on_click(move |_| dialog.end_modal(JOIN));
	close_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));
	link_field.on_key_down(crate::ui::keys::escape_cancels(dialog));
	link_field.set_focus();
	link_field.set_selection(0, link_field.get_last_position());
	let result = dialog.show_modal();
	dialog.destroy();

	if result == SEND {
		let text = format!("Join my WhatsApp {} call: {}", kind, url);
		crate::ui_commands::links::send_text_to_chosen_chats(state, parts, "Send call link", &text);
	} else if result == JOIN
		&& let Some(whatsnap::WhatsAppLink::Call { token, video }) = whatsnap::parse_whatsapp_link(&url)
	{
		join_call_link(state, parts, token, video);
	}
}
