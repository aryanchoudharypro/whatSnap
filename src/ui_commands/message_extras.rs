//! Pinning and keeping messages, and advanced chat privacy: the message
//! menu's Pin and Keep items, the chat menu's "Pinned Messages..." and
//! "Advanced Chat Privacy...", and what the backend answers
//! (`message_pinned`, `message_kept`, `pinned_messages`, `chat_privacy`).

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{AppState, message_display::update_loaded_message, types::UiCommand, ui::window::WindowParts};

const PIN: i32 = 3401;
const UNPIN: i32 = 3402;
const KEEP: i32 = 3403;
const UNKEEP: i32 = 3404;
const STICKER_FAVORITE: i32 = 3405;

/// How long a pin lasts, as WhatsApp offers it.
const PIN_DURATIONS: [(&str, u32); 3] = [("24 hours", 86_400), ("7 days", 604_800), ("30 days", 2_592_000)];

fn send(state: &Rc<RefCell<AppState>>, action: &str, data: serde_json::Value) {
	let payload = serde_json::json!({ "action": action, "data": data });
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

fn announce(parts: &WindowParts, message: &str) {
	crate::announce::announce(parts.conversations_parts.messages_announcer, message);
}

/// Adds Pin/Unpin and Keep/Unkeep to a message's context menu when they
/// apply, and returns the message they act on.
pub(crate) fn append_message_actions(
	menu: &Menu,
	state: &Rc<RefCell<AppState>>,
	item: Option<&whatsnap::DisplayItem>,
) -> Option<whatsnap::ChatMessage> {
	let Some(whatsnap::DisplayItem::Single(m)) = item else { return None };
	if m.is_date_divider || m.is_unread_divider || m.is_system() || m.is_deleted || m.id.is_empty() {
		return None;
	}
	// Only a message WhatsApp has can be pinned or kept.
	if m.from_me && (m.status == "pending" || m.status == "failed") {
		return None;
	}
	let s = state.borrow();
	let jid = s.selected_chat.as_ref()?;
	if jid.ends_with("@newsletter") || jid.ends_with("@broadcast") {
		return None;
	}
	// Like WA Web, a group where only admins can send only lets admins pin.
	let can_pin = !s.admins_only_jids.contains(jid) || s.admin_group_jids.contains(jid);
	if m.pinned {
		menu.append(UNPIN, "Unpi&n", "Unpin this message for everyone in the chat", ItemKind::Normal);
	} else if can_pin {
		menu.append(PIN, "Pi&n...", "Pin this message at the top of the chat for everyone", ItemKind::Normal);
	}
	if m.kept {
		menu.append(UNKEEP, "Un&keep", "Let this message disappear like the others", ItemKind::Normal);
	} else if m.expiring {
		menu.append(KEEP, "&Keep in chat", "Keep this message from disappearing, for everyone", ItemKind::Normal);
	}
	if m.media_type == "sticker" {
		if m.sticker_favorite {
			menu.append(
				STICKER_FAVORITE,
				"Remove from &favorite stickers",
				"Take this sticker off your favorites",
				ItemKind::Normal,
			);
		} else {
			menu.append(
				STICKER_FAVORITE,
				"Add to &favorite stickers",
				"Keep this sticker in your favorites to send it again",
				ItemKind::Normal,
			);
		}
	}
	Some((**m).clone())
}

/// Runs one of the items `append_message_actions` added. False when `id`
/// isn't one of them.
pub(crate) fn handle_message_action(
	id: i32,
	state: &Rc<RefCell<AppState>>,
	frame: &Frame,
	msg: &whatsnap::ChatMessage,
) -> bool {
	let Some(jid) = state.borrow().selected_chat.clone() else { return false };
	let target = serde_json::json!({ "jid": jid, "message_id": msg.id });
	match id {
		PIN => {
			let labels: Vec<&str> = PIN_DURATIONS.iter().map(|(label, _)| *label).collect();
			let dialog =
				SingleChoiceDialog::builder(frame, "Keep this message pinned for:", "Pin Message", &labels).build();
			// WhatsApp's own default.
			dialog.set_selection(1);
			if dialog.show_modal() != wxdragon::id::ID_OK {
				return true;
			}
			let Some(choice) = dialog.get_string_selection() else { return true };
			let Some(&(_, seconds)) = PIN_DURATIONS.iter().find(|(label, _)| *label == choice) else { return true };
			let mut data = target;
			data["pin"] = true.into();
			data["duration"] = seconds.into();
			send(state, "pin_message", data);
		}
		UNPIN => {
			let mut data = target;
			data["pin"] = false.into();
			send(state, "pin_message", data);
		}
		KEEP | UNKEEP => {
			let mut data = target;
			data["keep"] = (id == KEEP).into();
			send(state, "keep_message", data);
		}
		STICKER_FAVORITE => crate::ui_commands::stickers::set_favorite(state, &msg.id, !msg.sticker_favorite),
		_ => return false,
	}
	true
}

/// Whether two JIDs name the same chat, ignoring device parts.
fn same_chat(a: &str, b: &str) -> bool {
	let base = |jid: &str| match jid.split_once('@') {
		Some((user, server)) => format!("{}@{}", user.split([':', '.']).next().unwrap_or(user), server),
		None => jid.to_string(),
	};
	base(a) == base(b)
}

/// Rewrites one message's row in the open chat after `change`, and says
/// `spoken` if it was there.
fn update_open_message(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	data: &serde_json::Value,
	spoken: &str,
	change: impl FnOnce(&mut whatsnap::ChatMessage),
) {
	let text = |key: &str| data.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
	let (jid, message_id) = (text("jid"), text("message_id"));
	let mut s = state.borrow_mut();
	if !s.selected_chat.as_deref().is_some_and(|sel| same_chat(sel, &jid)) {
		return;
	}
	let is_group = jid.ends_with("@g.us");
	if let Some(row) = update_loaded_message(&mut s, &message_id, change)
		&& let Some(item) = s.display_items.get(row)
	{
		parts.conversations_parts.messages_list.set_string(row as u32, &item.format_display(is_group));
		drop(s);
		announce(parts, spoken);
	}
}

/// A message was pinned or unpinned, by us or someone else.
pub(crate) fn message_pinned(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let pinned = data.get("pinned").and_then(|v| v.as_bool()).unwrap_or(false);
	let spoken = if pinned { "Message pinned." } else { "Message unpinned." };
	update_open_message(state, parts, data, spoken, |m| m.pinned = pinned);
}

/// A message was kept or unkept.
pub(crate) fn message_kept(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let kept = data.get("kept").and_then(|v| v.as_bool()).unwrap_or(false);
	let spoken = if kept { "Message kept." } else { "Message unkept." };
	update_open_message(state, parts, data, spoken, |m| m.kept = kept);
}

/// Chat menu > "Pinned Messages...": ask the backend for the open chat's.
pub(crate) fn show_pinned_messages(state: &Rc<RefCell<AppState>>) {
	let Some(jid) = state.borrow().selected_chat.clone() else { return };
	send(state, "get_pinned_messages", serde_json::json!({ "jid": jid }));
}

/// The backend listed a chat's pinned messages: pick one to go to it.
pub(crate) fn pinned_messages(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let jid = data.get("jid").and_then(|v| v.as_str()).unwrap_or("");
	if !state.borrow().selected_chat.as_deref().is_some_and(|sel| same_chat(sel, jid)) {
		return;
	}
	let pins: Vec<(String, String)> = data
		.get("messages")
		.and_then(|m| m.as_array())
		.map(|list| {
			list.iter()
				.map(|p| {
					let field = |key: &str| p.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
					let text = whatsnap::friendly_preview_text(&field("text"));
					(field("id"), format!("{}: {}", field("sender"), text))
				})
				.collect()
		})
		.unwrap_or_default();
	if pins.is_empty() {
		MessageDialog::builder(&parts.frame, "No messages are pinned in this chat.", "Pinned Messages")
			.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
			.build()
			.show_modal();
		return;
	}
	let labels: Vec<&str> = pins.iter().map(|(_, label)| label.as_str()).collect();
	let dialog =
		SingleChoiceDialog::builder(&parts.frame, "Go to a pinned message:", "Pinned Messages", &labels).build();
	if dialog.show_modal() != wxdragon::id::ID_OK {
		return;
	}
	let Some((id, _)) = usize::try_from(dialog.get_selection()).ok().and_then(|i| pins.get(i)) else { return };
	let _ = state
		.borrow()
		.ui_tx
		.send(UiCommand::JumpToSearchResult { message_id: id.clone(), message_text: String::new() });
}

/// Chat menu or contact info > "Advanced Chat Privacy...": find out whether
/// it's on first. Contact info may be showing a chat that isn't open.
pub(crate) fn show_chat_privacy(state: &Rc<RefCell<AppState>>) {
	let Some(jid) = ({
		let s = state.borrow();
		s.info_chat.clone().or_else(|| s.selected_chat.clone())
	}) else {
		return;
	};
	if jid.ends_with("@newsletter") || jid.ends_with("@broadcast") {
		return;
	}
	send(state, "get_chat_privacy", serde_json::json!({ "jid": jid }));
}

/// The backend said whether advanced chat privacy is on: offer to switch it.
/// Restricted chat, the newer kind, can only be switched on a phone, so
/// that's only explained.
pub(crate) fn chat_privacy(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let jid = data.get("jid").and_then(|v| v.as_str()).unwrap_or("").to_string();
	let asked = {
		let s = state.borrow();
		[&s.info_chat, &s.selected_chat].into_iter().flatten().any(|chat| same_chat(chat, &jid))
	};
	if !asked {
		return;
	}
	// Over the contact info page when it was asked from there.
	let info = &parts.conversation_data_dialog.dialog;
	let parent: &dyn WxWidget = if info.is_shown() { info } else { &parts.frame };
	if data.get("restricted").and_then(|v| v.as_bool()).unwrap_or(false) {
		MessageDialog::builder(
			parent,
			"Restricted chat is on in this chat. It keeps the chat on the phone that turned it on: new messages \
			 don't come to linked devices like this one, nobody in the chat can export it or save its media \
			 automatically, and its messages can't be used with AI features. It can only be turned off from a phone.",
			"Restricted Chat",
		)
		.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
		.build()
		.show_modal();
		return;
	}
	let enabled = data.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
	// WhatsApp's own description of the setting.
	let what = "When it's on, people in this chat can't export it, media isn't saved to their devices automatically, \
	            and messages can't be used with AI features.";
	let (question, action) = if enabled {
		(format!("Advanced chat privacy is on in this chat. {} Turn it off?", what), "Turn off")
	} else {
		(format!("Advanced chat privacy is off in this chat. {} Turn it on?", what), "Turn on")
	};
	let dialog = MessageDialog::builder(parent, &question, "Advanced Chat Privacy")
		.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
		.build();
	dialog.set_yes_no_labels(action, "Cancel");
	if dialog.show_modal() == ID_YES {
		send(state, "set_chat_privacy", serde_json::json!({ "jid": jid, "enabled": !enabled }));
	}
}
