//! Call links (Call > "Create Call Link..." and "Join Call Link...", and
//! call.whatsapp.com links), and the call window's "Raise hand" and
//! "React...", plus letting people in from a call link's waiting room. What
//! the backend answers: `call_link_created`, `call_hand`, `call_reaction`,
//! `call_waiting_room`, `call_action_done` and `call_action_failed`. Screen
//! sharing lives with the camera in `calls.rs`.

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState, CallVideo,
	call_helpers::{announce_call, call_window_title},
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

/// Until a call connects there's no hand to raise.
pub(crate) fn hide_call_buttons(parts: &WindowParts) {
	let c = &parts.call_parts;
	c.raise_hand_btn.show(false);
	c.react_btn.show(false);
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
	match whantv::parse_whatsapp_link(&link) {
		Some(whantv::WhatsAppLink::Call { token, video }) => join_call_link(state, parts, token, video),
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
	s.call_video = CallVideo { autostart_camera: video, ..Default::default() };
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
		"call_action_done" => {
			if text(data, "what").as_str() == "hand" {
				let mut s = state.borrow_mut();
				s.call_video.hand_raised = on;
				show_call_buttons(&s, parts);
				drop(s);
				announce_call(parts, if on { "Your hand is raised." } else { "You lowered your hand." });
			}
		}
		"call_action_failed" => announce_call(parts, &text(data, "message")),
		_ => {}
	}
}

/// We're waiting to be let in, or (as the link's admin) someone is.
fn waiting_room(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	if data.get("waiting").and_then(|v| v.as_bool()).unwrap_or(false) {
		parts.call_parts.status_label.set_label("Waiting for someone in the call to let you in...");
		announce_call(parts, "Waiting for someone in the call to let you in.");
		return;
	}
	if !data.get("is_admin").and_then(|v| v.as_bool()).unwrap_or(false) {
		return;
	}
	let users = data.get("users").and_then(|u| u.as_array()).cloned().unwrap_or_default();
	for user in users {
		let jid = text(&user, "jid");
		{
			let mut s = state.borrow_mut();
			if jid.is_empty() || s.call_video.admit_asked.contains(&jid) {
				continue;
			}
			s.call_video.admit_asked.push(jid.clone());
		}
		let dialog = MessageDialog::builder(
			&parts.call_parts.frame,
			&format!("{} wants to join the call.", text(&user, "name")),
			"Waiting to Join",
		)
		.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
		.build();
		dialog.set_yes_no_labels("Let in", "Don't let in");
		let admit = dialog.show_modal() == ID_YES;
		let s = state.borrow();
		if let Some(call_id) = s.active_call_id.clone() {
			send(&s, "admit_call_participant", serde_json::json!({ "call_id": call_id, "jid": jid, "on": admit }));
		}
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
		let copied = Clipboard::get().locker().is_some_and(|_locker| Clipboard::get().set_text(&url_copy));
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
		&& let Some(whantv::WhatsAppLink::Call { token, video }) = whantv::parse_whatsapp_link(&url)
	{
		join_call_link(state, parts, token, video);
	}
}
