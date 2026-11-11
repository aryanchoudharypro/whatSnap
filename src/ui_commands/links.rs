use std::{cell::RefCell, rc::Rc};

use whatsnap::WhatsAppLink;
use wxdragon::prelude::*;

use crate::{
	AppState,
	ui::{
		chat_ui::open_chat_for_jid,
		contact_picker::contact_picker_label,
		dialogs::send_to::{SendTarget, show_send_to_dialog},
		window::WindowParts,
	},
};

fn handle_app_link(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, link: crate::platform::app_link::AppLink) {
	match link {
		crate::platform::app_link::AppLink::Open { jid, .. }
		| crate::platform::app_link::AppLink::Reply { jid, .. } => open_chat_for_jid(&jid, state, parts),
		crate::platform::app_link::AppLink::MarkRead { jid, .. } => {
			if state.borrow().locked_jids.contains(&jid) && !state.borrow().chats_unlocked {
				return;
			}
			crate::ui::chat_ui::clear_chat_notifications(&jid);
			state.borrow_mut().notification_reads.insert(jid.clone());
			crate::handle_ui_command(crate::UiCommand::MarkRead { jid, message_ids: Vec::new() }, state, parts);
		}
	}
}

pub(crate) fn open_whatsapp_link(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, link: String) {
	let app_link = crate::platform::app_link::parse(&link);
	let parsed = if app_link.is_some() { None } else { whatsnap::parse_whatsapp_link(&link) };
	if app_link.is_none() && parsed.is_none() {
		return;
	}
	// A notification from a linked account that isn't on screen: mark read
	// goes straight to that account; open switches to it first.
	if let Some(link_account) = app_link.as_ref().and_then(|l| match l {
		crate::platform::app_link::AppLink::Open { account, .. }
		| crate::platform::app_link::AppLink::MarkRead { account, .. }
		| crate::platform::app_link::AppLink::Reply { account, .. } => account.clone(),
	}) && !crate::ui_commands::accounts::is_active(state, &link_account)
	{
		match &app_link {
			Some(crate::platform::app_link::AppLink::MarkRead { jid, .. }) => {
				crate::ui_commands::accounts::mark_read_in_background(state, &link_account, jid);
				return;
			}
			_ => {
				if !crate::ui_commands::accounts::switch_to(state, parts, &link_account) {
					return;
				}
			}
		}
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
			send_text_to_chosen_chats(state, parts, crate::i18n::tr("Send to"), &text);
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

/// Sends what was typed into a notification's reply box, then marks the chat
/// read, as replying from a notification does on the phone. Nothing opens or
/// takes focus. When it can't be sent, a notification says why.
pub(crate) fn notification_reply(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	jid: String,
	account: Option<String>,
	text: String,
) {
	if let Some(account) = account.filter(|a| !crate::ui_commands::accounts::is_active(state, a)) {
		crate::ui_commands::accounts::reply_in_background(state, &account, &jid, &text);
		return;
	}
	let refusal = {
		let mut s = state.borrow_mut();
		if !s.main_ui_shown || s.chats.is_empty() {
			s.pending_notification_replies.push((jid, text));
			return;
		}
		if s.locked_jids.contains(&jid) && !s.chats_unlocked {
			return;
		}
		let name =
			s.contacts.get(&jid).cloned().unwrap_or_else(|| crate::ui::contact_picker::display_name_for_jid(&jid));
		if crate::ui_commands::blocking::is_blocked(&s, &jid) {
			Some(if crate::i18n::is_indonesian() {
				format!("Balasan Anda kepada {name} tidak terkirim karena Anda memblokirnya.")
			} else {
				format!("Your reply to {name} wasn't sent because you blocked them.")
			})
		} else {
			crate::ui::chat_ui::group_send_block_reason(&s, &jid).map(|reason| {
				if crate::i18n::is_indonesian() {
					format!("Balasan Anda kepada {name} tidak terkirim. {reason}.")
				} else {
					format!("Your reply to {name} wasn't sent. {reason}.")
				}
			})
		}
	};
	if let Some(refusal) = refusal {
		crate::ui::chat_ui::show_toast_notification(whatsnap::TRAY_APP_NAME, &refusal);
		return;
	}
	{
		let s = state.borrow();
		let payload = serde_json::json!({
			"action": "send",
			"data": { "to": jid, "text": text, "reply_to_id": "", "reply_to_sender": "" }
		});
		let _ = s.ws_tx.send(payload.to_string());
	}
	handle_app_link(state, parts, crate::platform::app_link::AppLink::MarkRead { jid, account: None });
}

pub(crate) fn open_pending_whatsapp_link(state: &Rc<RefCell<AppState>>) {
	let mut s = state.borrow_mut();
	if !s.main_ui_shown || s.chats.is_empty() {
		return;
	}
	for (jid, text) in std::mem::take(&mut s.pending_notification_replies) {
		let _ = s.ui_tx.send(crate::UiCommand::NotificationReply { jid, account: None, text });
	}
	if let Some(link) = s.pending_whatsapp_link.take() {
		let _ = s.ui_tx.send(crate::UiCommand::OpenWhatsAppLink(link));
	}
	if !s.pending_shared_files.is_empty() {
		let paths = std::mem::take(&mut s.pending_shared_files);
		let _ = s.ui_tx.send(crate::UiCommand::ShareFiles(paths));
	}
}

pub(crate) fn share_files(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, paths: Vec<String>) {
	{
		let mut s = state.borrow_mut();
		if !s.main_ui_shown || s.chats.is_empty() {
			s.pending_shared_files.extend(paths);
			return;
		}
	}
	let mut paths: Vec<String> = paths.into_iter().filter(|p| std::path::Path::new(p).is_file()).collect();
	if paths.is_empty() {
		return;
	}
	let max = crate::ui_commands::attachments::MAX_BATCH_FILES;
	let left_out = paths.len().saturating_sub(max);
	paths.truncate(max);
	let title = match paths.as_slice() {
		[one] => {
			let fname = std::path::Path::new(one).file_name().unwrap_or_default().to_string_lossy();
			if crate::i18n::is_indonesian() { format!("Kirim {fname}") } else { format!("Send {fname}") }
		}
		many => {
			if crate::i18n::is_indonesian() {
				format!("Kirim {} file", many.len())
			} else {
				format!("Send {} files", many.len())
			}
		}
	};
	let targets = send_targets(&state.borrow());
	let Some(choice) = show_send_to_dialog(&parts.frame, &title, targets, "", &paths) else { return };
	let s = state.borrow();
	let kind = whatsnap::AttachKind::Pasted;
	for jid in &choice.jids {
		crate::ui_commands::attachments::send_files(&s.ws_tx, jid, &paths, kind, choice.text.trim(), "", "", false);
	}
	let names: Vec<String> =
		choice.jids.iter().map(|jid| s.contacts.get(jid).cloned().unwrap_or_else(|| jid.clone())).collect();
	drop(s);
	let names_joined = names.join(", ");
	let mut message = if crate::i18n::is_indonesian() {
		format!("Mengirim ke {names_joined}.")
	} else {
		format!("Sending to {names_joined}.")
	};
	if left_out > 0 {
		if crate::i18n::is_indonesian() {
			message.push_str(&format!(
				" Anda dapat mengirim maksimal {max} file sekaligus; {left_out} file tidak dikirim."
			));
		} else {
			message.push_str(&format!(
				" You can send up to {max} files at once; {left_out} file{} weren't sent.",
				if left_out == 1 { "" } else { "s" }
			));
		}
	}
	crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &message);
}

pub(crate) fn send_targets(s: &AppState) -> Vec<SendTarget> {
	let mut seen = std::collections::HashSet::new();
	let mut targets = Vec::new();
	for jid in &s.chats {
		if jid.ends_with("@broadcast") || s.community_jids.contains(jid) || s.locked_jids.contains(jid) {
			continue;
		}
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

pub(crate) fn send_text_to_chosen_chats(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	title: &str,
	text: &str,
) {
	let targets = send_targets(&state.borrow());
	let Some(choice) = show_send_to_dialog(&parts.frame, title, targets, text, &[]) else { return };
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
	let names_joined = names.join(", ");
	let spoken = if crate::i18n::is_indonesian() {
		format!("Terkirim ke {names_joined}.")
	} else {
		format!("Sent to {names_joined}.")
	};
	crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &spoken);
}

pub(crate) fn show_group_invite_link(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	group_jid: String,
	link: String,
) {
	let group_name =
		state.borrow().contacts.get(&group_jid).cloned().unwrap_or_else(|| crate::i18n::tr("this group").to_string());
	let parent: &dyn WxWidget =
		if parts.group_data_dialog.dialog.is_shown() { &parts.group_data_dialog.dialog } else { &parts.frame };
	let dialog = Dialog::builder(parent, crate::i18n::tr("Group invite link"))
		.with_size(460, 240)
		.with_style(DialogStyle::DefaultDialogStyle)
		.build();
	crate::ui::theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	crate::ui::theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let intro_text = if crate::i18n::is_indonesian() {
		format!("Siapa saja yang memiliki tautan ini dapat bergabung ke {group_name}.")
	} else {
		format!("Anyone with this link can join {group_name}.")
	};
	let intro = StaticText::builder(&panel).with_label(&intro_text).build();
	sizer.add(&intro, 0, SizerFlag::All, 10);
	let link_label = StaticText::builder(&panel).with_label(crate::i18n::tr("Invite link")).build();
	crate::ui::theme::style_muted_label(&link_label);
	sizer.add(&link_label, 0, SizerFlag::Left | SizerFlag::Right, 10);
	let link_field = TextCtrl::builder(&panel).with_style(TextCtrlStyle::ReadOnly).build();
	link_field.set_value(&link);
	sizer.add(&link_field, 0, SizerFlag::Expand | SizerFlag::All, 10);

	let buttons = BoxSizer::builder(Orientation::Horizontal).build();
	let send_btn = Button::builder(&panel).with_label(crate::i18n::tr("Send link...")).build();
	let copy_btn = Button::builder(&panel).with_label(crate::i18n::tr("Copy link")).build();
	let reset_btn = Button::builder(&panel).with_label(crate::i18n::tr("Reset link")).build();
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label(crate::i18n::tr("Close")).build();
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
		let copied = crate::copy_text(&link_copy);
		let message = if copied { crate::i18n::tr("Link copied.") } else { crate::i18n::tr("Couldn't copy the link.") };
		crate::platform::announce::announce(announcer_copy, message);
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
			crate::i18n::tr("Reset this group's invite link? The current link will stop working, and anyone who has it won't be able to join with it."),
			crate::i18n::tr("Reset link"),
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
			let text = if crate::i18n::is_indonesian() {
				format!("Ikuti tautan ini untuk bergabung ke grup WhatsApp saya: {link}")
			} else {
				format!("Follow this link to join my WhatsApp group: {link}")
			};
			send_text_to_chosen_chats(state, parts, crate::i18n::tr("Send invite link"), &text);
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

pub(crate) fn request_group_invite_link(state: &Rc<RefCell<AppState>>) {
	let s = state.borrow();
	let Some(jid) = s.selected_chat.clone().filter(|j| j.ends_with("@g.us")) else { return };
	let payload = serde_json::json!({
		"action": "get_group_invite_link",
		"data": { "jid": jid, "reset": false }
	});
	let _ = s.ws_tx.send(payload.to_string());
}

pub(crate) fn can_forward_to(s: &AppState, jid: &str) -> bool {
	!jid.ends_with("@newsletter") || can_post_in_channel(s, jid)
}

pub(crate) fn can_post_in_channel(s: &AppState, jid: &str) -> bool {
	matches!(s.channel_roles.get(jid).map(String::as_str), Some("owner" | "admin"))
}

pub(crate) fn show_channel_info(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, info: &serde_json::Value) {
	let text = |key: &str| info.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
	let flag = |key: &str| info.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
	let jid = text("jid");
	if jid.is_empty() {
		return;
	}
	let name = Some(text("name")).filter(|n| !n.is_empty()).unwrap_or_else(|| crate::i18n::tr("Channel").to_string());
	let followers = info.get("followers").and_then(|v| v.as_u64()).unwrap_or(0);
	let following = flag("following");
	let muted = flag("muted");
	let link = text("link");
	let is_open = state.borrow().selected_chat.as_deref() == Some(jid.as_str());

	let mut details = vec![name.clone()];
	if flag("verified") {
		details.push(crate::i18n::tr("Verified channel").to_string());
	}
	details.push(match followers {
		1 => crate::i18n::tr("1 follower").to_string(),
		n => {
			if crate::i18n::is_indonesian() {
				format!("{n} pengikut")
			} else {
				format!("{n} followers")
			}
		}
	});
	match text("role").as_str() {
		"owner" => details.push(crate::i18n::tr("You own this channel").to_string()),
		"admin" => details.push(crate::i18n::tr("You're an admin of this channel").to_string()),
		_ if following => details.push(crate::i18n::tr("You follow this channel").to_string()),
		_ => details.push(crate::i18n::tr("You don't follow this channel").to_string()),
	}
	if matches!(text("state").as_str(), "suspended" | "geosuspended") {
		details.push(crate::i18n::tr("This channel is suspended").to_string());
	}
	let role = text("role");
	let is_owner = role == "owner";
	let can_manage = is_owner || role == "admin";
	let description = text("description");
	if !description.is_empty() {
		details.push(String::new());
		details.push(description.clone());
	}

	let directory = state.borrow().channel_directory.clone();
	let parent: &dyn WxWidget = match &directory {
		Some(directory) => &directory.dialog,
		None => &parts.frame,
	};
	let dialog = Dialog::builder(parent, crate::i18n::tr("Channel info"))
		.with_size(480, 380)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	crate::ui::theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	crate::ui::theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();
	let details_label = StaticText::builder(&panel).with_label(crate::i18n::tr("About this channel")).build();
	crate::ui::theme::style_muted_label(&details_label);
	sizer.add(&details_label, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Right, 10);
	let details_field =
		TextCtrl::builder(&panel).with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly).build();
	details_field.set_value(&details.join("\n"));
	sizer.add(&details_field, 1, SizerFlag::Expand | SizerFlag::All, 10);

	let buttons = BoxSizer::builder(Orientation::Horizontal).build();
	let follow_btn = Button::builder(&panel)
		.with_label(if following { crate::i18n::tr("Unfollow") } else { crate::i18n::tr("Follow") })
		.build();
	let mute_btn = Button::builder(&panel)
		.with_label(if muted { crate::i18n::tr("Unmute") } else { crate::i18n::tr("Mute") })
		.build();
	let open_btn = Button::builder(&panel).with_label(crate::i18n::tr("Open channel")).build();
	let copy_btn = Button::builder(&panel).with_label(crate::i18n::tr("Copy link")).build();
	let similar_btn = Button::builder(&panel).with_label(crate::i18n::tr("Similar channels...")).build();
	let manage_btn = Button::builder(&panel).with_label(crate::i18n::tr("Manage channel...")).build();
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label(crate::i18n::tr("Close")).build();
	for button in [&manage_btn, &follow_btn, &mute_btn, &open_btn, &copy_btn, &similar_btn] {
		buttons.add(button, 0, SizerFlag::Right, 8);
	}
	buttons.add_stretch_spacer(1);
	buttons.add(&close_btn, 0, SizerFlag::empty(), 0);
	sizer.add_sizer(&buttons, 0, SizerFlag::Expand | SizerFlag::All, 10);
	mute_btn.show(following);
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
				crate::i18n::tr("Unfollow this channel? Its posts will be removed from whatSnap."),
				crate::i18n::tr("Unfollow channel"),
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
		let copied = crate::copy_text(&link_copy);
		crate::platform::announce::announce(
			announcer,
			if copied { crate::i18n::tr("Link copied.") } else { crate::i18n::tr("Couldn't copy the link.") },
		);
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

pub(crate) fn show_channel_directory(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	similar_to: Option<(String, String)>,
) {
	let ws_tx = state.borrow().ws_tx.clone();
	let state_store = state.clone();
	crate::ui::dialogs::channel_directory::show_channel_directory(&parts.frame, ws_tx, similar_to, move |dir| {
		state_store.borrow_mut().channel_directory = dir;
	});
}
