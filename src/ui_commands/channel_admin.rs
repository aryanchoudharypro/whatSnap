use std::{
	cell::{Cell, RefCell},
	rc::Rc,
};

use wxdragon::prelude::*;

use crate::{
	AppState,
	ui::{
		chat_ui::open_chat_for_jid,
		dialogs::{
			channel_admin::{
				ChannelFollower, FollowerAction, ask_channel_details, pick_channel_picture, show_channel_followers,
				show_channel_people, show_pending_admin_invites,
			},
			send_to::{SendTarget, show_send_to_dialog},
		},
		window::WindowParts,
	},
};

const ADMIN_INVITE_TEXT: &str = "I'd like you to be an admin of my WhatsApp channel.";

fn admin_invite_targets(s: &AppState) -> Vec<SendTarget> {
	let person = |jid: &str| jid.ends_with("@s.whatsapp.net") || jid.ends_with("@lid");
	let mut seen = std::collections::HashSet::new();
	let mut targets = Vec::new();
	for jid in s.chats.iter().filter(|j| person(j)) {
		if seen.insert(jid.clone()) {
			let label = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.clone());
			targets.push(SendTarget { jid: jid.clone(), label });
		}
	}
	for contact in s.all_contacts.iter().filter(|c| person(&c.jid)) {
		if seen.insert(contact.jid.clone()) {
			targets.push(SendTarget {
				jid: contact.jid.clone(),
				label: crate::ui::contact_picker::contact_picker_label(contact),
			});
		}
	}
	targets
}

pub(crate) fn open_channel_admin_invite(state: &Rc<RefCell<AppState>>, message_id: String) {
	send(state, "get_channel_admin_invite", serde_json::json!({ "message_id": message_id }));
}

pub(crate) fn channel_admin_invite(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let text = |key: &str| data.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
	let flag = |key: &str| data.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
	let jid = text("jid");
	let channel =
		Some(text("name")).filter(|n| !n.is_empty()).unwrap_or_else(|| crate::i18n::tr("a channel").to_string());
	let user_name = text("userName");
	if jid.is_empty() {
		return;
	}
	if flag("fromMe") {
		let msg = if crate::i18n::is_indonesian() {
			format!("Anda mengundang {user_name} menjadi admin di {channel}. Batalkan undangan ini?")
		} else {
			format!("You invited {user_name} to be an admin of {channel}. Cancel this invite?")
		};
		let dlg = MessageDialog::builder(&parts.frame, &msg, crate::i18n::tr("Channel admin invite"))
			.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
			.build();
		if dlg.show_modal() == ID_YES {
			send(state, "revoke_channel_admin_invite", serde_json::json!({ "jid": jid, "user": text("user") }));
		}
		return;
	}
	if flag("expired") {
		let msg = if crate::i18n::is_indonesian() {
			format!("Undangan untuk menjadi admin di {channel} ini telah kedaluwarsa.")
		} else {
			format!("This invite to be an admin of {channel} has expired.")
		};
		MessageDialog::builder(&parts.frame, &msg, crate::i18n::tr("Channel admin invite"))
			.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
			.build()
			.show_modal();
		return;
	}
	let mut body = if crate::i18n::is_indonesian() {
		format!("{user_name} mengundang Anda menjadi admin saluran {channel}.")
	} else {
		format!("{user_name} invited you to be an admin of the channel {channel}.")
	};
	let caption = text("caption");
	if !caption.is_empty() {
		body.push_str(&format!("\n\n{}", caption));
	}
	body.push_str(crate::i18n::tr("\n\nAccept? As an admin you can post updates and change the channel's details."));
	let dlg = MessageDialog::builder(&parts.frame, &body, crate::i18n::tr("Channel admin invite"))
		.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
		.build();
	if dlg.show_modal() == ID_YES {
		send(state, "accept_channel_admin_invite", serde_json::json!({ "jid": jid }));
		announce(parts, crate::i18n::tr("Accepting..."));
	}
}

pub(crate) fn channel_notice(parts: &Rc<WindowParts>, data: &serde_json::Value) {
	if let Some(message) = data.get("message").and_then(|m| m.as_str()) {
		let translated = crate::i18n::translate_backend_message(message);
		let localized = if translated == message { crate::i18n::tr_str(message) } else { translated };
		announce(parts, &localized);
	}
}

#[derive(Clone, Copy)]
pub(crate) enum ChannelManage {
	InviteAdmins,
	PendingInvites,
	Edit,
	ChangePicture,
	RemovePicture,
	Followers,
	Delete,
}

fn send(state: &Rc<RefCell<AppState>>, action: &str, data: serde_json::Value) {
	let payload = serde_json::json!({ "action": action, "data": data });
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

fn announce(parts: &WindowParts, message: &str) {
	crate::platform::announce::announce(parts.conversations_parts.messages_announcer, message);
}

pub(crate) fn create_channel(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let Some(details) = ask_channel_details(&parts.frame, true, "", "") else { return };
	send(
		state,
		"create_channel",
		serde_json::json!({
			"name": details.name,
			"description": details.description,
			"picture": details.picture.unwrap_or_default(),
		}),
	);
	announce(parts, crate::i18n::tr("Creating channel..."));
}

pub(crate) fn choose_manage_action(button: &Button, dialog: &Dialog, is_owner: bool) -> Option<ChannelManage> {
	const EDIT: i32 = 3201;
	const PICTURE: i32 = 3202;
	const REMOVE_PICTURE: i32 = 3203;
	const FOLLOWERS: i32 = 3204;
	const DELETE: i32 = 3205;
	const INVITE_ADMINS: i32 = 3206;
	const PENDING_INVITES: i32 = 3207;
	let mut builder = Menu::builder()
		.append_item(
			EDIT,
			crate::i18n::tr("&Edit name and description..."),
			crate::i18n::tr("Change the channel's name and description"),
		)
		.append_item(
			PICTURE,
			crate::i18n::tr("Change &picture..."),
			crate::i18n::tr("Choose a new picture for the channel"),
		)
		.append_item(
			REMOVE_PICTURE,
			crate::i18n::tr("&Remove picture"),
			crate::i18n::tr("Remove the channel's picture"),
		)
		.append_item(
			FOLLOWERS,
			crate::i18n::tr("&Followers and admins..."),
			crate::i18n::tr("See who follows the channel, and manage its admins"),
		);
	if is_owner {
		builder = builder
			.append_item(
				INVITE_ADMINS,
				crate::i18n::tr("&Invite admins..."),
				crate::i18n::tr("Invite people to be admins of the channel"),
			)
			.append_item(
				PENDING_INVITES,
				crate::i18n::tr("Pe&nding admin invites..."),
				crate::i18n::tr("See and cancel admin invites not accepted yet"),
			)
			.append_item(
				DELETE,
				crate::i18n::tr("&Delete channel..."),
				crate::i18n::tr("Delete the channel for everyone"),
			);
	}
	let mut menu = builder.build();
	let chosen = Rc::new(Cell::new(0));
	let chosen_select = chosen.clone();
	menu.on_selected(move |event| chosen_select.set(event.get_id()));
	button.popup_menu(&mut menu, None);
	menu.destroy_menu();
	match chosen.get() {
		EDIT => Some(ChannelManage::Edit),
		PICTURE => Some(ChannelManage::ChangePicture),
		REMOVE_PICTURE => Some(ChannelManage::RemovePicture),
		FOLLOWERS => Some(ChannelManage::Followers),
		INVITE_ADMINS => Some(ChannelManage::InviteAdmins),
		PENDING_INVITES => Some(ChannelManage::PendingInvites),
		DELETE => {
			let confirm = MessageDialog::builder(
				dialog,
				crate::i18n::tr(
					"Delete this channel? It will be removed for all its followers, along with all its updates. This can't be undone.",
				),
				crate::i18n::tr("Delete channel"),
			)
			.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconWarning)
			.build();
			(confirm.show_modal() == ID_YES).then_some(ChannelManage::Delete)
		}
		_ => None,
	}
}

pub(crate) fn manage_channel(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	jid: &str,
	name: &str,
	description: &str,
	action: ChannelManage,
) {
	match action {
		ChannelManage::InviteAdmins => {
			let targets = admin_invite_targets(&state.borrow());
			let Some(choice) = show_send_to_dialog(
				&parts.frame,
				crate::i18n::tr("Invite admins"),
				targets,
				crate::i18n::tr(ADMIN_INVITE_TEXT),
				&[],
			) else {
				return;
			};
			send(
				state,
				"invite_channel_admins",
				serde_json::json!({ "jid": jid, "users": choice.jids, "caption": choice.text }),
			);
			announce(parts, crate::i18n::tr("Sending admin invites..."));
		}
		ChannelManage::PendingInvites => {
			send(state, "get_channel_pending_invites", serde_json::json!({ "jid": jid }));
			announce(parts, crate::i18n::tr("Loading pending admin invites..."));
		}
		ChannelManage::Edit => {
			let Some(details) = ask_channel_details(&parts.frame, false, name, description) else { return };
			let mut data = serde_json::json!({ "jid": jid });
			if details.name != name {
				data["name"] = details.name.into();
			}
			if details.description != description {
				data["description"] = details.description.into();
			}
			if data.as_object().is_some_and(|d| d.len() > 1) {
				send(state, "update_channel", data);
				announce(parts, crate::i18n::tr("Saving..."));
			}
		}
		ChannelManage::ChangePicture => {
			let Some(path) = pick_channel_picture(&parts.frame) else { return };
			send(state, "update_channel", serde_json::json!({ "jid": jid, "picture": path }));
			announce(parts, crate::i18n::tr("Changing the channel picture..."));
		}
		ChannelManage::RemovePicture => {
			send(state, "update_channel", serde_json::json!({ "jid": jid, "removePicture": true }));
		}
		ChannelManage::Followers => {
			send(state, "get_channel_followers", serde_json::json!({ "jid": jid }));
			announce(parts, crate::i18n::tr("Loading followers..."));
		}
		ChannelManage::Delete => {
			send(state, "delete_channel", serde_json::json!({ "jid": jid }));
			announce(parts, crate::i18n::tr("Deleting channel..."));
		}
	}
}

pub(crate) fn channel_created(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("").to_string();
	let name = data.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
	if jid.is_empty() {
		return;
	}
	{
		let mut s = state.borrow_mut();
		s.contacts.insert(jid.clone(), name.clone());
		s.channel_jids.insert(jid.clone());
		s.channel_roles.insert(jid.clone(), "owner".to_string());
	}
	open_chat_for_jid(&jid, state, parts);
	let msg = if crate::i18n::is_indonesian() {
		format!("Saluran {name} dibuat.")
	} else {
		format!("Channel {name} created.")
	};
	announce(parts, &msg);
}

pub(crate) fn channel_updated(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
	let message = data.get("message").and_then(|m| m.as_str()).unwrap_or("Channel updated.");
	let translated = crate::i18n::translate_backend_message(message);
	let localized = if translated == message { crate::i18n::tr_str(message) } else { translated };
	announce(parts, &localized);
	if !jid.is_empty() {
		send(state, "get_channel_info", serde_json::json!({ "jid": jid }));
	}
}

pub(crate) fn channel_deleted(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("").to_string();
	let name = state.borrow().contacts.get(&jid).cloned().unwrap_or_else(|| crate::i18n::tr("The channel").to_string());
	if state.borrow().selected_chat.as_deref() == Some(jid.as_str()) {
		crate::handle_ui_command(crate::UiCommand::CloseConversation, state, parts);
	}
	{
		let mut s = state.borrow_mut();
		s.channel_jids.remove(&jid);
		s.channel_roles.remove(&jid);
	}
	let msg =
		if crate::i18n::is_indonesian() { format!("{name} telah dihapus.") } else { format!("{name} was deleted.") };
	announce(parts, &msg);
}

pub(crate) fn channel_followers(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let text = |v: &serde_json::Value, key: &str| v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string();
	let jid = text(data, "jid");
	let followers: Vec<ChannelFollower> = data
		.get("followers")
		.and_then(|f| f.as_array())
		.map(|list| {
			list.iter()
				.map(|f| ChannelFollower { jid: text(f, "jid"), name: text(f, "name"), role: text(f, "role") })
				.filter(|f| !f.jid.is_empty())
				.collect()
		})
		.unwrap_or_default();
	let (channel_name, is_owner) = {
		let s = state.borrow();
		(
			s.contacts.get(&jid).cloned().unwrap_or_else(|| crate::i18n::tr("channel").to_string()),
			s.channel_roles.get(&jid).map(String::as_str) == Some("owner"),
		)
	};
	match show_channel_followers(&parts.frame, &channel_name, followers, is_owner) {
		Some(FollowerAction::InviteAdmin { jid: user, name }) => {
			send(state, "invite_channel_admins", serde_json::json!({ "jid": jid, "users": [user], "caption": "" }));
			let msg = if crate::i18n::is_indonesian() {
				format!("Mengundang {name} menjadi admin...")
			} else {
				format!("Inviting {name} to be an admin...")
			};
			announce(parts, &msg);
		}
		Some(FollowerAction::RemoveAdmin { jid: user, name }) => {
			send(state, "demote_channel_admin", serde_json::json!({ "jid": jid, "user": user }));
			let msg = if crate::i18n::is_indonesian() {
				format!("Menghapus {name} sebagai admin...")
			} else {
				format!("Removing {name} as admin...")
			};
			announce(parts, &msg);
		}
		Some(FollowerAction::MakeOwner { jid: user, name }) => {
			send(state, "change_channel_owner", serde_json::json!({ "jid": jid, "user": user }));
			let msg = if crate::i18n::is_indonesian() {
				format!("Menjadikan {name} pemilik saluran...")
			} else {
				format!("Making {name} the owner...")
			};
			announce(parts, &msg);
		}
		None => {}
	}
}

pub(crate) fn channel_post_menu(
	state: &Rc<RefCell<AppState>>,
	channel_jid: &str,
	post: &whatsnap::ChatMessage,
) -> Menu {
	const PIN: i32 = 3301;
	const REACTIONS: i32 = 3302;
	const VOTERS: i32 = 3303;
	let is_poll = post.text.starts_with("[Poll]") || post.text.starts_with("[Quiz]");
	let mut builder = Menu::builder();
	if post.channel_pinned || state.borrow().channel_pinning {
		builder = builder.append_item(
			PIN,
			crate::i18n::tr(if post.channel_pinned { "&Unpin post" } else { "&Pin post" }),
			crate::i18n::tr("Pin or unpin this post at the top of the channel"),
		);
	}
	builder = builder.append_item(
		REACTIONS,
		crate::i18n::tr("Who &reacted..."),
		crate::i18n::tr("See who reacted to this post"),
	);
	if is_poll {
		builder = builder.append_item(
			VOTERS,
			crate::i18n::tr("Who &voted..."),
			crate::i18n::tr("See who voted for each option"),
		);
	}
	let menu = builder.build();
	let state = state.clone();
	let channel_jid = channel_jid.to_string();
	let message_id = post.id.clone();
	let pinned = post.channel_pinned;
	menu.on_selected(move |event| {
		let post_ref = serde_json::json!({ "jid": channel_jid, "message_id": message_id });
		match event.get_id() {
			PIN => {
				let mut data = post_ref;
				data["pin"] = (!pinned).into();
				send(&state, "pin_channel_post", data);
			}
			REACTIONS => send(&state, "get_channel_reactions", post_ref),
			VOTERS => send(&state, "get_channel_poll_voters", post_ref),
			_ => {}
		}
	});
	menu
}

pub(crate) fn channel_people(parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let text = |v: &serde_json::Value, key: &str| v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string();
	let groups: Vec<(String, Vec<String>)> = data
		.get("groups")
		.and_then(|g| g.as_array())
		.map(|groups| {
			groups
				.iter()
				.map(|g| {
					let names = g
						.get("names")
						.and_then(|n| n.as_array())
						.map(|n| n.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
						.unwrap_or_default();
					(text(g, "label"), names)
				})
				.collect()
		})
		.unwrap_or_default();
	let people: usize = groups.iter().map(|(_, names)| names.len()).sum();
	let summary = if crate::i18n::is_indonesian() {
		match (text(data, "kind").as_str(), people) {
			("votes", 0) => "Belum ada yang memilih di polling ini.".to_string(),
			("votes", n) => format!("{n} suara"),
			(_, 0) => "Belum ada yang bereaksi ke postingan ini.".to_string(),
			(_, n) => format!("{n} reaksi"),
		}
	} else {
		match (text(data, "kind").as_str(), people) {
			("votes", 0) => "No one has voted in this poll yet.".to_string(),
			("votes", 1) => "1 vote".to_string(),
			("votes", n) => format!("{} votes", n),
			(_, 0) => "No one has reacted to this post yet.".to_string(),
			(_, 1) => "1 reaction".to_string(),
			(_, n) => format!("{} reactions", n),
		}
	};
	let raw_title = text(data, "title");
	let title = if crate::i18n::is_indonesian() {
		if let Some(q) = raw_title.strip_prefix("Votes: ") {
			format!("Suara: {q}")
		} else {
			crate::i18n::tr_str(&raw_title)
		}
	} else {
		raw_title
	};
	show_channel_people(&parts.frame, &title, &summary, &groups);
}

pub(crate) fn channel_pending_invites(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	data: &serde_json::Value,
) {
	let text = |v: &serde_json::Value, key: &str| v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string();
	let jid = text(data, "jid");
	let invites: Vec<(String, String)> = data
		.get("invites")
		.and_then(|i| i.as_array())
		.map(|list| list.iter().map(|i| (text(i, "jid"), text(i, "name"))).filter(|(j, _)| !j.is_empty()).collect())
		.unwrap_or_default();
	let channel_name =
		state.borrow().contacts.get(&jid).cloned().unwrap_or_else(|| crate::i18n::tr("channel").to_string());
	if let Some((user, name)) = show_pending_admin_invites(&parts.frame, &channel_name, invites) {
		send(state, "revoke_channel_admin_invite", serde_json::json!({ "jid": jid, "user": user }));
		let msg = if crate::i18n::is_indonesian() {
			format!("Membatalkan undangan untuk {name}...")
		} else {
			format!("Cancelling the invite for {name}...")
		};
		announce(parts, &msg);
	}
}
