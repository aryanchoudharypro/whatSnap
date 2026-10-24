//! WhatsApp links opened from outside the app (whatSnap is the whatsapp://
//! handler), and sending a group's invite link.

use std::{cell::RefCell, rc::Rc};

use whatsnap::WhatsAppLink;
use wxdragon::prelude::*;

use crate::{
	AppState,
	chat_ui::open_chat_for_jid,
	contact_picker::contact_picker_label,
	ui::{
		send_to::{SendTarget, show_send_to_dialog},
		window::WindowParts,
	},
};

/// Acts on a `whatsnap://` link from one of our notifications: a click opens the
/// chat, and "Mark as read" marks it read. Either way the chat's toasts go.
fn handle_app_link(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, link: crate::app_link::AppLink) {
	match link {
		crate::app_link::AppLink::Open { jid } => open_chat_for_jid(&jid, state, parts),
		crate::app_link::AppLink::MarkRead { jid } => {
			crate::chat_ui::clear_chat_notifications(&jid);
			// The chat shows as read only once the server has the receipts:
			// see the "chat_updated" and "mark_read_failed" events.
			state.borrow_mut().notification_reads.insert(jid.clone());
			crate::handle_ui_command(crate::UiCommand::MarkRead { jid, message_ids: Vec::new() }, state, parts);
		}
	}
}

/// Acts on a WhatsApp link, the way the official app does: a link naming a
/// number opens that chat with the text ready to send, a link with only text
/// asks who to send it to, and an invite link offers to join the group.
/// Before we're logged in and have the chat list, the link waits.
pub(crate) fn open_whatsapp_link(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, link: String) {
	let app_link = crate::app_link::parse(&link);
	let parsed = if app_link.is_some() { None } else { whatsnap::parse_whatsapp_link(&link) };
	if app_link.is_none() && parsed.is_none() {
		return;
	}
	{
		let mut s = state.borrow_mut();
		if !s.main_ui_shown || s.chats.is_empty() {
			s.pending_whatsapp_link = Some(link);
			return;
		}
	}
	if let Some(app_link) = app_link {
		handle_app_link(state, parts, app_link);
		return;
	}
	let Some(parsed) = parsed else { return };
	match parsed {
		WhatsAppLink::Send { phone: Some(phone), text } => {
			let jid = format!("{}@s.whatsapp.net", phone);
			open_chat_for_jid(&jid, state, parts);
			let field = &parts.conversations_parts.message_field;
			if !text.is_empty() {
				field.set_value(&text);
				let end = field.get_last_position();
				field.set_selection(end, end);
			}
			field.set_focus();
		}
		WhatsAppLink::Send { phone: None, text } => {
			send_text_to_chosen_chats(state, parts, "Send to", &text);
		}
		WhatsAppLink::Channel { code } => {
			let payload = serde_json::json!({
				"action": "get_channel_info",
				"data": { "code": code }
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		WhatsAppLink::Call { token, video } => {
			crate::ui_commands::call_extras::join_call_link(state, parts, token, video);
		}
		WhatsAppLink::Invite { code } => {
			let payload = serde_json::json!({
				"action": "get_group_link_info",
				"data": { "code": code }
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
	}
}

/// Runs a link that arrived before the chat list was ready.
pub(crate) fn open_pending_whatsapp_link(state: &Rc<RefCell<AppState>>) {
	let mut s = state.borrow_mut();
	if !s.main_ui_shown || s.chats.is_empty() {
		return;
	}
	if let Some(link) = s.pending_whatsapp_link.take() {
		let _ = s.ui_tx.send(crate::UiCommand::OpenWhatsAppLink(link));
	}
}

/// Everyone a message can go to: chats first, most recent first, then
/// contacts we have no chat with.
fn send_targets(s: &AppState) -> Vec<SendTarget> {
	let mut seen = std::collections::HashSet::new();
	let mut targets = Vec::new();
	for jid in &s.chats {
		if jid.ends_with("@broadcast") || s.community_jids.contains(jid) || s.locked_jids.contains(jid) {
			continue;
		}
		// Only a channel's admins can post in it.
		if s.channel_jids.contains(jid) && !can_post_in_channel(s, jid) {
			continue;
		}
		let label = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.clone());
		seen.insert(jid.clone());
		targets.push(SendTarget { jid: jid.clone(), label });
	}
	for contact in &s.all_contacts {
		if seen.insert(contact.jid.clone()) {
			targets.push(SendTarget { jid: contact.jid.clone(), label: contact_picker_label(contact) });
		}
	}
	targets
}

/// Asks which chats (up to five) to send `text` to, then sends it.
pub(crate) fn send_text_to_chosen_chats(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	title: &str,
	text: &str,
) {
	let targets = send_targets(&state.borrow());
	let Some(choice) = show_send_to_dialog(&parts.frame, title, targets, text) else { return };
	let s = state.borrow();
	for jid in &choice.jids {
		let payload = serde_json::json!({
			"action": "send",
			"data": { "to": jid, "text": choice.text, "reply_to_id": "", "reply_to_sender": "" }
		});
		let _ = s.ws_tx.send(payload.to_string());
	}
	let names: Vec<String> =
		choice.jids.iter().map(|jid| s.contacts.get(jid).cloned().unwrap_or_else(|| jid.clone())).collect();
	drop(s);
	crate::announce::announce(parts.conversations_parts.messages_announcer, &format!("Sent to {}.", names.join(", ")));
}

/// Shows a group's invite link, with ways to copy it, send it, or replace it
/// with a new one. `link` is the link the backend just fetched.
pub(crate) fn show_group_invite_link(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	group_jid: String,
	link: String,
) {
	let group_name = state.borrow().contacts.get(&group_jid).cloned().unwrap_or_else(|| "this group".to_string());
	let parent: &dyn WxWidget =
		if parts.group_data_dialog.dialog.is_shown() { &parts.group_data_dialog.dialog } else { &parts.frame };
	let dialog = Dialog::builder(parent, "Group invite link")
		.with_size(460, 240)
		.with_style(DialogStyle::DefaultDialogStyle)
		.build();
	crate::ui::theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	crate::ui::theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let intro =
		StaticText::builder(&panel).with_label(&format!("Anyone with this link can join {}.", group_name)).build();
	sizer.add(&intro, 0, SizerFlag::All, 10);
	let link_label = StaticText::builder(&panel).with_label("Invite link").build();
	crate::ui::theme::style_muted_label(&link_label);
	sizer.add(&link_label, 0, SizerFlag::Left | SizerFlag::Right, 10);
	let link_field = TextCtrl::builder(&panel).with_style(TextCtrlStyle::ReadOnly).build();
	link_field.set_value(&link);
	sizer.add(&link_field, 0, SizerFlag::Expand | SizerFlag::All, 10);

	let buttons = BoxSizer::builder(Orientation::Horizontal).build();
	let send_btn = Button::builder(&panel).with_label("Send link...").build();
	let copy_btn = Button::builder(&panel).with_label("Copy link").build();
	let reset_btn = Button::builder(&panel).with_label("Reset link").build();
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Close").build();
	for button in [&send_btn, &copy_btn, &reset_btn] {
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

	// What the dialog was closed to do next.
	#[derive(Clone, Copy, PartialEq)]
	enum Next {
		Nothing,
		Send,
		Reset,
	}
	let next = Rc::new(RefCell::new(Next::Nothing));

	let link_copy = link.clone();
	let announcer_copy = announcer;
	copy_btn.on_click(move |_| {
		let copied = Clipboard::get().locker().is_some_and(|_locker| Clipboard::get().set_text(&link_copy));
		let message = if copied { "Link copied." } else { "Couldn't copy the link." };
		crate::announce::announce(announcer_copy, message);
	});
	let next_send = next.clone();
	send_btn.on_click(move |_| {
		*next_send.borrow_mut() = Next::Send;
		dialog.end_modal(ID_OK);
	});
	let next_reset = next.clone();
	reset_btn.on_click(move |_| {
		let confirm = MessageDialog::builder(
			&dialog,
			"Reset this group's invite link? The current link will stop working, and anyone who has it won't be able to join with it.",
			"Reset link",
		)
		.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconWarning)
		.build();
		if confirm.show_modal() == ID_YES {
			*next_reset.borrow_mut() = Next::Reset;
			dialog.end_modal(ID_OK);
		}
	});
	close_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));

	link_field.set_focus();
	link_field.set_selection(0, link_field.get_last_position());
	dialog.show_modal();
	dialog.destroy();

	let next = *next.borrow();
	match next {
		Next::Send => {
			let text = format!("Follow this link to join my WhatsApp group: {}", link);
			send_text_to_chosen_chats(state, parts, "Send invite link", &text);
		}
		Next::Reset => {
			let payload = serde_json::json!({
				"action": "get_group_invite_link",
				"data": { "jid": group_jid, "reset": true }
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		Next::Nothing => {}
	}
}

/// Asks the backend for a group's invite link; the answer opens
/// `show_group_invite_link`.
pub(crate) fn request_group_invite_link(state: &Rc<RefCell<AppState>>) {
	let s = state.borrow();
	let Some(jid) = s.selected_chat.clone().filter(|j| j.ends_with("@g.us")) else { return };
	let payload = serde_json::json!({
		"action": "get_group_invite_link",
		"data": { "jid": jid, "reset": false }
	});
	let _ = s.ws_tx.send(payload.to_string());
}

/// Whether we can post in a channel: only its owner and admins can.
/// Whether a message can be forwarded to jid: anywhere but a channel we
/// only follow.
pub(crate) fn can_forward_to(s: &AppState, jid: &str) -> bool {
	!jid.ends_with("@newsletter") || can_post_in_channel(s, jid)
}

pub(crate) fn can_post_in_channel(s: &AppState, jid: &str) -> bool {
	matches!(s.channel_roles.get(jid).map(String::as_str), Some("owner" | "admin"))
}

/// The channel info dialog: what the channel is, and follow, mute, open and
/// copy-link actions. `info` is the backend's `channel_info` event data.
pub(crate) fn show_channel_info(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, info: &serde_json::Value) {
	let text = |key: &str| info.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
	let flag = |key: &str| info.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
	let jid = text("jid");
	if jid.is_empty() {
		return;
	}
	let name = Some(text("name")).filter(|n| !n.is_empty()).unwrap_or_else(|| "Channel".to_string());
	let followers = info.get("followers").and_then(|v| v.as_u64()).unwrap_or(0);
	let following = flag("following");
	let muted = flag("muted");
	let link = text("link");
	let is_open = state.borrow().selected_chat.as_deref() == Some(jid.as_str());

	let mut details = vec![name.clone()];
	if flag("verified") {
		details.push("Verified channel".to_string());
	}
	details.push(match followers {
		1 => "1 follower".to_string(),
		n => format!("{} followers", n),
	});
	match text("role").as_str() {
		"owner" => details.push("You own this channel".to_string()),
		"admin" => details.push("You're an admin of this channel".to_string()),
		_ if following => details.push("You follow this channel".to_string()),
		_ => details.push("You don't follow this channel".to_string()),
	}
	if matches!(text("state").as_str(), "suspended" | "geosuspended") {
		details.push("This channel is suspended".to_string());
	}
	let role = text("role");
	let is_owner = role == "owner";
	let can_manage = is_owner || role == "admin";
	let description = text("description");
	if !description.is_empty() {
		details.push(String::new());
		details.push(description.clone());
	}

	// Over "Find channels" when it's open, so it isn't lost behind it.
	let directory = state.borrow().channel_directory.clone();
	let parent: &dyn WxWidget = match &directory {
		Some(directory) => &directory.dialog,
		None => &parts.frame,
	};
	let dialog = Dialog::builder(parent, "Channel info")
		.with_size(480, 380)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	crate::ui::theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	crate::ui::theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();
	// A read-only text field rather than labels, so the whole description can
	// be read line by line and copied.
	let details_label = StaticText::builder(&panel).with_label("About this channel").build();
	crate::ui::theme::style_muted_label(&details_label);
	sizer.add(&details_label, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Right, 10);
	let details_field =
		TextCtrl::builder(&panel).with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly).build();
	details_field.set_value(&details.join("\n"));
	sizer.add(&details_field, 1, SizerFlag::Expand | SizerFlag::All, 10);

	let buttons = BoxSizer::builder(Orientation::Horizontal).build();
	let follow_btn = Button::builder(&panel).with_label(if following { "Unfollow" } else { "Follow" }).build();
	let mute_btn = Button::builder(&panel).with_label(if muted { "Unmute" } else { "Mute" }).build();
	let open_btn = Button::builder(&panel).with_label("Open channel").build();
	let copy_btn = Button::builder(&panel).with_label("Copy link").build();
	let similar_btn = Button::builder(&panel).with_label("Similar channels...").build();
	let manage_btn = Button::builder(&panel).with_label("Manage channel...").build();
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Close").build();
	for button in [&manage_btn, &follow_btn, &mute_btn, &open_btn, &copy_btn, &similar_btn] {
		buttons.add(button, 0, SizerFlag::Right, 8);
	}
	buttons.add_stretch_spacer(1);
	buttons.add(&close_btn, 0, SizerFlag::empty(), 0);
	sizer.add_sizer(&buttons, 0, SizerFlag::Expand | SizerFlag::All, 10);
	mute_btn.show(following);
	// An owner can't unfollow their own channel, only delete it.
	follow_btn.show(!is_owner);
	manage_btn.show(can_manage);
	open_btn.show(!is_open);
	copy_btn.show(!link.is_empty());
	let announcer = StaticText::builder(&panel).with_label("").build();
	live_region::set_live_region(&announcer);
	sizer.add(&announcer, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom, 10);
	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	#[derive(Clone, Copy)]
	enum Next {
		Nothing,
		Follow,
		Unfollow,
		Mute(bool),
		Open,
		Similar,
		Manage(crate::ui_commands::channel_admin::ChannelManage),
	}
	let next = Rc::new(RefCell::new(Next::Nothing));
	let next_follow = next.clone();
	follow_btn.on_click(move |_| {
		if following {
			let confirm = MessageDialog::builder(
				&dialog,
				"Unfollow this channel? Its posts will be removed from whatSnap.",
				"Unfollow channel",
			)
			.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
			.build();
			if confirm.show_modal() != ID_YES {
				return;
			}
		}
		*next_follow.borrow_mut() = if following { Next::Unfollow } else { Next::Follow };
		dialog.end_modal(ID_OK);
	});
	let next_mute = next.clone();
	mute_btn.on_click(move |_| {
		*next_mute.borrow_mut() = Next::Mute(!muted);
		dialog.end_modal(ID_OK);
	});
	let next_open = next.clone();
	open_btn.on_click(move |_| {
		*next_open.borrow_mut() = Next::Open;
		dialog.end_modal(ID_OK);
	});
	let next_similar = next.clone();
	similar_btn.on_click(move |_| {
		*next_similar.borrow_mut() = Next::Similar;
		dialog.end_modal(ID_OK);
	});
	let link_copy = link.clone();
	copy_btn.on_click(move |_| {
		let copied = Clipboard::get().locker().is_some_and(|_locker| Clipboard::get().set_text(&link_copy));
		crate::announce::announce(announcer, if copied { "Link copied." } else { "Couldn't copy the link." });
	});
	let next_manage = next.clone();
	manage_btn.on_click(move |_| {
		if let Some(action) = crate::ui_commands::channel_admin::choose_manage_action(&manage_btn, &dialog, is_owner) {
			*next_manage.borrow_mut() = Next::Manage(action);
			dialog.end_modal(ID_OK);
		}
	});
	close_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));

	details_field.set_focus();
	dialog.show_modal();
	dialog.destroy();

	let next = *next.borrow();
	let send = |action: &str, data: serde_json::Value| {
		let payload = serde_json::json!({ "action": action, "data": data });
		let _ = state.borrow().ws_tx.send(payload.to_string());
	};
	match next {
		Next::Follow => send("follow_channel", serde_json::json!({ "jid": jid })),
		Next::Unfollow => send("unfollow_channel", serde_json::json!({ "jid": jid })),
		Next::Mute(mute) => {
			send("set_chat_muted", serde_json::json!({ "jid": jid, "muted": mute, "duration_seconds": 0 }))
		}
		Next::Open => {
			if let Some(directory) = &directory {
				directory.dialog.end_modal(ID_CANCEL);
			}
			// A channel we don't follow isn't in the chat list; name it so
			// the open chat's heading isn't a raw ID.
			state.borrow_mut().contacts.entry(jid.clone()).or_insert(name);
			open_chat_for_jid(&jid, state, parts);
		}
		Next::Similar => match &directory {
			// Find channels is already open under this: list them there.
			Some(directory) => directory.request_similar(&jid, &name),
			None => show_channel_directory(state, parts, Some((jid.clone(), name.clone()))),
		},
		Next::Manage(action) => {
			if let Some(directory) = &directory {
				directory.dialog.end_modal(ID_CANCEL);
			}
			crate::ui_commands::channel_admin::manage_channel(state, parts, &jid, &name, &description, action);
		}
		Next::Nothing => {}
	}
}

/// Opens "Find channels", on the directory or on the channels like one.
pub(crate) fn show_channel_directory(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	similar_to: Option<(String, String)>,
) {
	let ws_tx = state.borrow().ws_tx.clone();
	let state_store = state.clone();
	crate::ui::channel_directory::show_channel_directory(&parts.frame, ws_tx, similar_to, move |dir| {
		state_store.borrow_mut().channel_directory = dir;
	});
}
