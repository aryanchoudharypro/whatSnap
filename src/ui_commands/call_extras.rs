use std::{cell::RefCell, rc::Rc};

use live_region::Priority;
use wxdragon::prelude::*;

use crate::{
	AppState, CallParticipant, CallVideo,
	calls::helpers::{announce_call, call_window_title, is_ringing_state},
	ui::window::WindowParts,
};

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

pub(crate) fn show_call_buttons(s: &AppState, parts: &WindowParts) {
	let group = s.call_video.peer_name.is_none();
	let c = &parts.call_parts;
	c.raise_hand_btn.set_label(crate::i18n::tr(if s.call_video.hand_raised { "Lower hand" } else { "Raise hand" }));
	c.raise_hand_btn.show(group);
	c.react_btn.show(true);
	c.frame.layout();
}

pub(crate) fn hide_call_buttons(parts: &WindowParts) {
	let c = &parts.call_parts;
	c.raise_hand_btn.show(false);
	c.react_btn.show(false);
	c.people_panel.show(false);
}

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
			label: crate::i18n::tr_fmt("{}, wants to join", &[&p.name]),
			waiting: true,
			connected: false,
		})
		.collect();
	for p in &s.call_participants {
		let state = match p.state.as_str() {
			"connected" => crate::i18n::tr("in the call"),
			"invited" => crate::i18n::tr("invited"),
			"rejected" => crate::i18n::tr("declined"),
			st if is_ringing_state(st) => crate::i18n::tr("ringing"),
			"" => crate::i18n::tr("joining"),
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
		vec![crate::i18n::tr("No one else is here yet.")]
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

fn people_rows_jid_at(label: Option<String>, rows: &[PersonRow]) -> Option<String> {
	let label = label?;
	rows.iter().find(|r| r.label == label).map(|r| r.jid.clone())
}

fn selected_person(s: &AppState, parts: &WindowParts) -> Option<PersonRow> {
	let rows = people_rows(s);
	let list = &parts.call_parts.people_list;
	let jid = list.get_selection().and_then(|i| people_rows_jid_at(list.get_string(i), &rows))?;
	rows.into_iter().find(|r| r.jid == jid && r.connected)
}

pub(crate) fn person_buttons(s: &AppState, parts: &WindowParts) {
	let c = &parts.call_parts;
	let person = selected_person(s, parts);
	let can_remove = s.call_video.can_remove || s.call_video.link_admin;
	match &person {
		Some(p) => {
			if crate::i18n::is_indonesian() {
				c.mute_person_btn.set_label(&format!("Bisukan {}", p.name));
				c.remove_person_btn.set_label(&format!("Keluarkan {}", p.name));
			} else {
				c.mute_person_btn.set_label(&format!("Mute {}", p.name));
				c.remove_person_btn.set_label(&format!("Remove {}", p.name));
			}
		}
		None => {
			c.mute_person_btn.set_label(crate::i18n::tr("Mute"));
			c.remove_person_btn.set_label(crate::i18n::tr("Remove"));
		}
	}
	c.mute_person_btn.show(person.is_some());
	c.remove_person_btn.show(person.is_some() && can_remove);
	c.people_panel.layout();
}

pub(crate) fn act_on_person(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, remove: bool) {
	let s = state.borrow();
	let Some(call_id) = s.active_call_id.clone() else { return };
	let Some(person) = selected_person(&s, parts) else {
		drop(s);
		announce_call(parts, crate::i18n::tr("Select someone in the call first."), Priority::High);
		return;
	};
	let action = if remove { "remove_call_participant" } else { "mute_call_participant" };
	send(&s, action, serde_json::json!({ "call_id": call_id, "jid": person.jid }));
	drop(s);
	let msg = if crate::i18n::is_indonesian() {
		if remove {
			format!("Mengeluarkan {} dari panggilan.", person.name)
		} else {
			format!("Meminta {} untuk membisukan.", person.name)
		}
	} else {
		let what = if remove { "Removing" } else { "Asking" };
		let tail = if remove { "from the call." } else { "to mute." };
		format!("{} {} {}", what, person.name, tail)
	};
	announce_call(parts, &msg, Priority::High);
}

fn muted_by(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let s = state.borrow();
	if let Some(audio) = &s.active_call_audio {
		audio.is_muted.store(true, std::sync::atomic::Ordering::Relaxed);
	}
	parts.call_parts.mute_btn.set_label(crate::i18n::tr("Unmute microphone"));
	drop(s);
	let name = text(data, "name");
	if name.is_empty() {
		announce_call(parts, crate::i18n::tr("Someone in the call muted you."), Priority::High);
	} else {
		let msg = crate::i18n::tr_fmt("{} muted you.", &[&name]);
		announce_call(parts, &msg, Priority::High);
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
		announce_call(parts, crate::i18n::tr("Select someone who wants to join first."), Priority::High);
		return;
	};
	send(&s, "admit_call_participant", serde_json::json!({ "call_id": call_id, "jid": row.jid, "on": admit }));
}

pub(crate) fn set_approval(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>, on: bool) {
	let s = state.borrow();
	let Some(call_id) = s.active_call_id.clone() else { return };
	send(&s, "set_call_approval", serde_json::json!({ "call_id": call_id, "on": on }));
}

pub(crate) fn create_call_link(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let choices = if crate::i18n::is_indonesian() { ["Suara", "Video"] } else { ["Voice", "Video"] };
	let choice = SingleChoiceDialog::builder(
		&parts.frame,
		crate::i18n::tr("What kind of call?"),
		crate::i18n::tr("Create Call Link"),
		&choices,
	)
	.build();
	if choice.show_modal() != wxdragon::id::ID_OK {
		return;
	}
	send(&state.borrow(), "create_call_link", serde_json::json!({ "video": choice.get_selection() == 1 }));
}

pub(crate) fn ask_call_link(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let dialog =
		TextEntryDialog::builder(&parts.frame, crate::i18n::tr("Call link:"), crate::i18n::tr("Join Call Link"))
			.build();
	if dialog.show_modal() != wxdragon::id::ID_OK {
		return;
	}
	let link = dialog.get_value().unwrap_or_default();
	match whatsnap::parse_whatsapp_link(&link) {
		Some(whatsnap::WhatsAppLink::Call { token, video }) => join_call_link(state, parts, token, video),
		_ => {
			MessageDialog::builder(
				&parts.frame,
				crate::i18n::tr("That isn't a WhatsApp call link."),
				crate::i18n::tr("Join Call Link"),
			)
			.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconWarning)
			.build()
			.show_modal();
		}
	}
}

pub(crate) fn join_call_link(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, token: String, video: bool) {
	let mut s = state.borrow_mut();
	if s.active_call_id.is_some() || s.call_placing {
		drop(s);
		announce_call(parts, crate::i18n::tr("You're already in a call."), Priority::High);
		return;
	}
	send(&s, "join_call_link", serde_json::json!({ "link": token, "video": video }));
	s.call_placing = true;
	s.call_pending_cancel = false;
	s.call_video = CallVideo { autostart_camera: video, video_call: video, ..Default::default() };
	let c = &parts.call_parts;
	c.frame.set_title(crate::i18n::tr(call_window_title(true, video)));
	c.status_label.set_label(crate::i18n::tr("Joining call..."));
	c.accept_btn.show(false);
	c.reject_btn.show(false);
	c.mute_btn.show(true);
	c.camera_btn.set_label(crate::i18n::tr("Turn camera on"));
	c.camera_btn.show(false);
	c.add_participant_btn.show(false);
	hide_call_buttons(parts);
	c.leave_btn.show(true);
	c.frame.show(true);
	drop(s);
	announce_call(parts, crate::i18n::tr("Joining call..."), Priority::High);
}

pub(crate) fn toggle_hand(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>) {
	let s = state.borrow();
	let Some(call_id) = s.active_call_id.clone() else { return };
	send(&s, "raise_hand", serde_json::json!({ "call_id": call_id, "on": !s.call_video.hand_raised }));
}

pub(crate) fn react(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let Some(call_id) = state.borrow().active_call_id.clone() else { return };
	let choice = SingleChoiceDialog::builder(
		&parts.call_parts.frame,
		crate::i18n::tr("React with:"),
		crate::i18n::tr("React"),
		&CALL_REACTIONS,
	)
	.build();
	if choice.show_modal() != wxdragon::id::ID_OK {
		return;
	}
	let Some(emoji) = usize::try_from(choice.get_selection()).ok().and_then(|i| CALL_REACTIONS.get(i)) else { return };
	send(&state.borrow(), "call_reaction", serde_json::json!({ "call_id": call_id, "emoji": emoji }));
	let msg = if crate::i18n::is_indonesian() {
		format!("Anda memberi reaksi {emoji}.")
	} else {
		format!("You reacted {emoji}.")
	};
	announce_call(parts, &msg, Priority::High);
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
			let name = text(data, "name");
			let msg = if crate::i18n::is_indonesian() {
				if raised { format!("{name} mengangkat tangan.") } else { format!("{name} menurunkan tangan.") }
			} else {
				let verb = if raised { "raised" } else { "lowered" };
				format!("{name} {verb} their hand.")
			};
			announce_call(parts, &msg, Priority::High);
		}
		"call_reaction" => {
			let name = text(data, "name");
			let emoji = text(data, "emoji");
			let msg = crate::i18n::tr_fmt("{} reacted {}.", &[&name, &emoji]);
			announce_call(parts, &msg, Priority::High);
		}
		"call_waiting_room" => waiting_room(state, parts, data),
		"call_muted_by" => muted_by(state, parts, data),
		"call_action_done" => match text(data, "what").as_str() {
			"hand" => {
				let mut s = state.borrow_mut();
				s.call_video.hand_raised = on;
				show_call_buttons(&s, parts);
				drop(s);
				announce_call(
					parts,
					crate::i18n::tr(if on { "Your hand is raised." } else { "You lowered your hand." }),
					Priority::High,
				);
			}
			"approval" => {
				let mut s = state.borrow_mut();
				s.call_video.approval_required = on;
				refresh_people(&s, parts);
				drop(s);
				announce_call(
					parts,
					crate::i18n::tr(if on {
						"New people now wait for you to let them in."
					} else {
						"Anyone with the link can now join straight away."
					}),
					Priority::High,
				);
			}
			"admit" => {
				announce_call(
					parts,
					crate::i18n::tr(if on { "They're being let in." } else { "They won't be let in." }),
					Priority::High,
				);
			}
			_ => {}
		},
		"call_action_failed" => {
			if text(data, "what").as_str() == "approval" {
				let s = state.borrow();
				parts.call_parts.approval_check.set_value(s.call_video.approval_required);
			}
			let err_msg = crate::i18n::translate_backend_message(&text(data, "message"));
			announce_call(parts, &err_msg, Priority::High);
		}
		_ => {}
	}
}

fn waiting_room(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	if data.get("waiting").and_then(|v| v.as_bool()).unwrap_or(false) {
		parts.call_parts.status_label.set_label(crate::i18n::tr("Waiting for someone in the call to let you in..."));
		announce_call(parts, crate::i18n::tr("Waiting for someone in the call to let you in."), Priority::High);
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
		let msg = if crate::i18n::is_indonesian() {
			let joined = match names.as_slice() {
				[] => String::new(),
				[one] => (*one).to_string(),
				[rest @ .., last] => format!("{} dan {}", rest.join(", "), last),
			};
			format!("{joined} ingin bergabung ke panggilan. Pilih Izinkan masuk atau Tolak masuk di bagian Orang.")
		} else {
			let verb = if arrived.len() == 1 { "wants" } else { "want" };
			format!(
				"{} {} to join the call. Choose Let in or Don't let in under People.",
				crate::calls::helpers::join_names(&names),
				verb
			)
		};
		announce_call(parts, &msg, Priority::High);
	}
}

fn show_call_link(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, url: String, video: bool) {
	let kind = if video { "video" } else { "voice" };
	let dialog = Dialog::builder(&parts.frame, crate::i18n::tr("Call Link")).with_size(460, 220).build();
	crate::ui::theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	crate::ui::theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();
	let intro_text = if crate::i18n::is_indonesian() {
		let kind_id = if video { "video" } else { "suara" };
		format!(
			"Siapa saja yang memiliki WhatsApp dapat menggunakan tautan ini untuk bergabung ke panggilan {kind_id} Anda."
		)
	} else {
		format!("Anyone with WhatsApp can use this link to join your {kind} call.")
	};
	let intro = StaticText::builder(&panel).with_label(&intro_text).build();
	sizer.add(&intro, 0, SizerFlag::All, 10);
	let link_field = TextCtrl::builder(&panel).with_style(TextCtrlStyle::ReadOnly).build();
	link_field.set_value(&url);
	sizer.add(&link_field, 0, SizerFlag::Expand | SizerFlag::All, 10);
	let buttons = BoxSizer::builder(Orientation::Horizontal).build();
	let copy_btn = Button::builder(&panel).with_label(crate::i18n::tr("Copy link")).build();
	let send_btn = Button::builder(&panel).with_label(crate::i18n::tr("Send link...")).build();
	let join_btn = Button::builder(&panel).with_label(crate::i18n::tr("Join call")).build();
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label(crate::i18n::tr("Close")).build();
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
		crate::platform::announce::announce(
			announcer,
			crate::i18n::tr(if copied { "Link copied." } else { "Couldn't copy the link." }),
		);
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
		let text = if crate::i18n::is_indonesian() {
			let kind_id = if video { "video" } else { "suara" };
			format!("Gabung ke panggilan {kind_id} WhatsApp saya: {url}")
		} else {
			format!("Join my WhatsApp {kind} call: {url}")
		};
		crate::ui_commands::links::send_text_to_chosen_chats(state, parts, crate::i18n::tr("Send call link"), &text);
	} else if result == JOIN
		&& let Some(whatsnap::WhatsAppLink::Call { token, video }) = whatsnap::parse_whatsapp_link(&url)
	{
		join_call_link(state, parts, token, video);
	}
}
