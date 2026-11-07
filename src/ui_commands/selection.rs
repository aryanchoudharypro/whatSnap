//! The buttons under the message list in selection mode, and what they do.
//!
//! Which buttons show, and when they're available, follows WhatsApp Web's own
//! select bar (`WAWebMultiSelectBar` and the `WAWebMultiSelect*` buttons, with
//! the rules from `WAWebMsgActionCapability`):
//!
//! - Copy: every selected message is text.
//! - Star, or Unstar when every selected message is starred: every message
//!   can be starred. In place of Star, Keep (Unkeep when all are kept) when
//!   every message is a disappearing one that can be kept. A selection that
//!   mixes the two shows neither.
//! - Delete: always, except a channel post you can't delete. The dialog offers
//!   "Delete for everyone" only when every message can still be deleted for
//!   everyone - your own within 2.5 days, or, as a group admin, one other
//!   person's message at a time.
//! - Forward: every message can be forwarded.
//! - Save as: every message has a file.
//!
//! Every action ends selection mode, as on WhatsApp Web.

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{AppState, ui::window::WindowParts};

/// WhatsApp's revoke window (`WAWebRevokeMsgConstants.REVOKE_WINDOW`).
const REVOKE_WINDOW_SECS: i64 = 60 * 60 * 60;

fn now() -> i64 {
	std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}

fn announce(parts: &WindowParts, text: &str) {
	crate::platform::announce::announce(parts.conversations_parts.messages_announcer, text);
}

fn plural(count: usize, one: &str, many: &str) -> String {
	if count == 1 { format!("1 {one}") } else { format!("{count} {many}") }
}

/// The selected messages, oldest first. An album counts as each of its
/// photos and videos.
pub(crate) fn selected_messages(s: &AppState) -> Vec<whatsnap::Msg> {
	let mut msgs: Vec<whatsnap::Msg> = s
		.selection_rows
		.iter()
		.filter(|item| s.selected_message_ids.contains(item.id()))
		.flat_map(|item| item.messages())
		.cloned()
		.collect();
	msgs.sort_by_key(|m| m.timestamp);
	msgs
}

fn is_view_once(m: &whatsnap::ChatMessage) -> bool {
	m.text.starts_with("[View once]") || m.text.to_lowercase().contains("view once message")
}

fn is_unsent(m: &whatsnap::ChatMessage) -> bool {
	m.from_me && (m.status == "pending" || m.status == "failed")
}

fn is_sticker(m: &whatsnap::ChatMessage) -> bool {
	m.media_type == "sticker" || m.text.starts_with("[Sticker]")
}

/// Something someone wrote, as opposed to a date, a group notice or a call.
fn is_user_message(m: &whatsnap::ChatMessage) -> bool {
	!m.is_date_divider && !m.is_unread_divider && !m.is_system() && !m.is_call() && !m.id.is_empty()
}

fn is_channel(jid: &str) -> bool {
	jid.ends_with("@newsletter")
}

/// A text message, whose text Copy puts on the clipboard (`canCopyMessage`).
fn is_copyable_text(m: &whatsnap::ChatMessage) -> bool {
	is_user_message(m) && !m.is_deleted && m.media_type.is_empty() && !m.text.starts_with('[') && !m.text.is_empty()
}

/// A message with a file, which Copy puts on the clipboard as that file -
/// unlike WhatsApp Web, which only copies text.
fn is_copyable_file(m: &whatsnap::ChatMessage) -> bool {
	is_user_message(m)
		&& !m.is_deleted
		&& !is_view_once(m)
		&& (m.is_media_downloaded()
			|| matches!(m.media_type.as_str(), "image" | "video" | "audio" | "voice" | "document" | "sticker"))
}

/// Copy takes all text or all files; the clipboard can't sensibly hold a mix.
fn can_copy(msgs: &[whatsnap::Msg]) -> bool {
	!msgs.is_empty() && (msgs.iter().all(|m| is_copyable_text(m)) || msgs.iter().all(|m| is_copyable_file(m)))
}

/// `canStarMsg`.
fn can_star(chat: &str, m: &whatsnap::ChatMessage) -> bool {
	!is_channel(chat)
		&& is_user_message(m)
		&& !m.is_deleted
		&& !is_view_once(m)
		&& !is_unsent(m)
		&& (!m.expiring || m.starred || is_sticker(m))
}

/// `canShowKeepOrUnkeepOption`.
fn can_keep(s: &AppState, chat: &str, m: &whatsnap::ChatMessage) -> bool {
	let keepable = !is_channel(chat) && is_user_message(m) && m.expiring && !m.is_deleted && !is_view_once(m);
	keepable && (m.kept || (!m.starred && !is_sticker(m) && !s.not_member_jids.contains(chat)))
}

/// `canForwardMsg`.
fn can_forward(chat: &str, m: &whatsnap::ChatMessage) -> bool {
	!chat.ends_with("@broadcast")
		&& is_user_message(m)
		&& !m.is_deleted
		&& !is_view_once(m)
		&& !is_unsent(m)
		&& !m.is_unreadable()
		&& !m.is_event()
		&& !m.is_order()
		&& !m.text.starts_with("[Payment]")
		&& !m.text.starts_with("[Group Invite]")
		&& !m.text.starts_with("[Live Location]")
		&& !(m.is_poll() && !is_channel(chat))
}

/// `canDownloadMsg`: anything with a file, but not stickers.
fn can_save(m: &whatsnap::ChatMessage) -> bool {
	is_user_message(m)
		&& !m.is_deleted
		&& !is_view_once(m)
		&& !is_sticker(m)
		&& (m.is_media_downloaded()
			|| matches!(m.media_type.as_str(), "image" | "video" | "audio" | "voice" | "document"))
}

#[derive(Clone, Copy, PartialEq)]
enum Revoker {
	Sender,
	Admin,
}

/// Who may delete `m` for everyone (`canSenderRevokeMsg`,
/// `canAdminRevokeMsg`, `canRevokeNewsletterMsg`).
fn revoker(s: &AppState, chat: &str, m: &whatsnap::ChatMessage, now: i64) -> Option<Revoker> {
	if !is_user_message(m) || m.is_deleted || is_unsent(m) || chat.ends_with("@broadcast") {
		return None;
	}
	if is_channel(chat) {
		return crate::ui_commands::links::can_post_in_channel(s, chat).then_some(Revoker::Sender);
	}
	if s.not_member_jids.contains(chat) || now - m.timestamp > REVOKE_WINDOW_SECS {
		return None;
	}
	if m.from_me {
		Some(Revoker::Sender)
	} else if chat.ends_with("@g.us") && s.admin_group_jids.contains(chat) {
		Some(Revoker::Admin)
	} else {
		None
	}
}

/// `WAWebMultiSelectDelete`: deleting is only ruled out by a channel post you
/// can't delete.
fn can_delete(s: &AppState, chat: &str, m: &whatsnap::ChatMessage) -> bool {
	if is_channel(chat) { crate::ui_commands::links::can_post_in_channel(s, chat) && !m.is_deleted } else { true }
}

enum StarSlot {
	Star { enabled: bool, unstar: bool },
	Keep { enabled: bool, unkeep: bool },
	Neither,
}

struct BarState {
	count: usize,
	copy: bool,
	slot: StarSlot,
	delete: bool,
	forward: bool,
	save: bool,
}

fn bar_state(s: &AppState) -> BarState {
	let msgs = selected_messages(s);
	let chat = s.selected_chat.clone().unwrap_or_default();
	let count = msgs.len();
	let any = count > 0;
	let all = |f: &dyn Fn(&whatsnap::ChatMessage) -> bool| any && msgs.iter().all(|m| f(m));
	let keepable = msgs.iter().filter(|m| can_keep(s, &chat, m)).count();
	let slot = if keepable > 0 && keepable < count {
		StarSlot::Neither
	} else if any && keepable == count {
		StarSlot::Keep { enabled: true, unkeep: msgs.iter().all(|m| m.kept) }
	} else {
		StarSlot::Star { enabled: all(&|m| can_star(&chat, m)), unstar: all(&|m| m.starred) }
	};
	BarState {
		count,
		copy: can_copy(&msgs),
		slot,
		delete: all(&|m| can_delete(s, &chat, m)),
		forward: all(&|m| can_forward(&chat, m)),
		save: all(&can_save),
	}
}

/// Shows, hides, names and enables the select bar's buttons for what is
/// selected now.
pub(crate) fn update_bar(s: &AppState, cp: &crate::ui::conversations::ConversationsPanelParts) {
	let bar = bar_state(s);
	cp.selection_count_label.set_label(&crate::ui::window::selection_text(bar.count));
	cp.selection_copy_btn.enable(bar.copy);
	match bar.slot {
		StarSlot::Star { enabled, unstar } => {
			let label = match (unstar, bar.count) {
				(true, 1) => "Unstar message",
				(true, _) => "Unstar messages",
				_ => "Star",
			};
			cp.selection_star_btn.set_label(label);
			cp.selection_star_btn.enable(enabled);
			cp.selection_star_btn.show(true);
			cp.selection_keep_btn.show(false);
		}
		StarSlot::Keep { enabled, unkeep } => {
			cp.selection_keep_btn.set_label(if unkeep { "Unkeep" } else { "Keep" });
			cp.selection_keep_btn.enable(enabled);
			cp.selection_keep_btn.show(true);
			cp.selection_star_btn.show(false);
		}
		StarSlot::Neither => {
			cp.selection_star_btn.show(false);
			cp.selection_keep_btn.show(false);
		}
	}
	cp.selection_delete_btn.enable(bar.delete);
	cp.selection_forward_btn.enable(bar.forward);
	cp.selection_save_btn.enable(bar.save);
	cp.selection_bar_panel.layout();
}

/// Leaves selection mode, after one of the bar's actions.
fn finish(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	if state.borrow().selection_mode {
		crate::ui_commands::chat_list::set_selection_mode(state, parts, false, false);
	}
}

pub(crate) fn copy_selected(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let msgs = selected_messages(&state.borrow());
	if !can_copy(&msgs) {
		return;
	}
	if msgs.iter().all(|m| is_copyable_file(m)) {
		finish(state, parts);
		let ids = msgs.iter().map(|m| m.id.clone()).collect();
		crate::ui_commands::attachments::copy_album_to_clipboard(state, parts, ids);
		return;
	}
	let text = msgs.iter().map(|m| m.text.as_str()).collect::<Vec<_>>().join("\n");
	finish(state, parts);
	if crate::platform::clipboard::copy_text(&text) {
		announce(parts, &format!("Copied {} to clipboard.", plural(msgs.len(), "message", "messages")));
	} else {
		announce(parts, "Couldn't copy to clipboard.");
	}
}

pub(crate) fn star_selected(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let (chat, msgs, star) = {
		let s = state.borrow();
		let Some(chat) = s.selected_chat.clone() else { return };
		let msgs = selected_messages(&s);
		if msgs.is_empty() || !msgs.iter().all(|m| can_star(&chat, m)) {
			return;
		}
		let star = !msgs.iter().all(|m| m.starred);
		(chat, msgs, star)
	};
	{
		let mut s = state.borrow_mut();
		for m in msgs.iter().filter(|m| m.starred != star) {
			s.bulk_quiet_pending.insert(m.id.clone());
			let payload = serde_json::json!({
				"action": "star_message",
				"data": {
					"jid": chat,
					"message_id": m.id,
					"sender_jid": if m.from_me { "me".to_string() } else { m.sender_jid.clone() },
					"from_me": m.from_me,
					"starred": star
				}
			});
			let _ = s.ws_tx.send(payload.to_string());
		}
	}
	finish(state, parts);
	let what = plural(msgs.len(), "message", "messages");
	announce(parts, &if star { format!("{what} starred.") } else { format!("{what} unstarred.") });
}

pub(crate) fn keep_selected(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let (chat, msgs, keep) = {
		let s = state.borrow();
		let Some(chat) = s.selected_chat.clone() else { return };
		let msgs = selected_messages(&s);
		if msgs.is_empty() || !msgs.iter().all(|m| can_keep(&s, &chat, m)) {
			return;
		}
		let keep = !msgs.iter().all(|m| m.kept);
		(chat, msgs, keep)
	};
	{
		let mut s = state.borrow_mut();
		for m in msgs.iter().filter(|m| m.kept != keep) {
			s.bulk_quiet_pending.insert(m.id.clone());
			let payload = serde_json::json!({
				"action": "keep_message",
				"data": { "jid": chat, "message_id": m.id, "keep": keep }
			});
			let _ = s.ws_tx.send(payload.to_string());
		}
	}
	finish(state, parts);
	let what = plural(msgs.len(), "message", "messages");
	announce(parts, &if keep { format!("{what} kept.") } else { format!("{what} unkept.") });
}

pub(crate) fn delete_selected(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let msgs = selected_messages(&state.borrow());
	ask_delete(state, parts, msgs, true);
}

/// The Delete key or menu item on one row of the message list: the same
/// dialog as the select bar's Delete. An album is all of its photos.
pub(crate) fn delete_row(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, ids: Vec<String>) {
	let msgs: Vec<whatsnap::Msg> = {
		let s = state.borrow();
		ids.iter().filter_map(|id| s.messages.iter().find(|m| m.id == *id).cloned()).collect()
	};
	ask_delete(state, parts, msgs, false);
}

/// `WAWebDeleteRevokeMsgFlow`: "Delete for everyone" only when every message
/// can be, with at most one deleted as an admin (that one is named, and
/// confirmed separately). Then one `delete_messages` request: the backend
/// sends a revoke per message, or every "delete for me" in one app-state
/// patch, as WhatsApp Web does.
fn ask_delete(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, msgs: Vec<whatsnap::Msg>, from_selection: bool) {
	let (chat, revokable, admin_msg) = {
		let s = state.borrow();
		let Some(chat) = s.selected_chat.clone() else { return };
		if msgs.is_empty() || !msgs.iter().all(|m| can_delete(&s, &chat, m)) {
			return;
		}
		let now = now();
		let revokers: Vec<Option<Revoker>> = msgs.iter().map(|m| revoker(&s, &chat, m, now)).collect();
		let admin_count = revokers.iter().filter(|r| **r == Some(Revoker::Admin)).count();
		let revokable = revokers.iter().all(Option::is_some) && admin_count <= 1;
		let admin_msg = revokers.iter().position(|r| *r == Some(Revoker::Admin)).map(|i| msgs[i].clone());
		(chat, revokable, admin_msg)
	};
	let admin_name = admin_msg.as_ref().filter(|_| revokable).map(|m| sender_name(state, m));
	let title = match &admin_name {
		Some(name) => format!("Delete message from {name}?"),
		None if msgs.len() == 1 => "Delete message?".to_string(),
		None => "Delete messages?".to_string(),
	};
	let for_everyone = if revokable {
		let dialog = MessageDialog::builder(&parts.frame, &title, "Delete")
			.with_style(
				MessageDialogStyle::YesNo
					| MessageDialogStyle::Cancel
					| MessageDialogStyle::CancelDefault
					| MessageDialogStyle::IconWarning,
			)
			.build();
		dialog.set_yes_no_labels("Delete for everyone", "Delete for me");
		match dialog.show_modal() {
			id if id == wxdragon::id::ID_YES => {
				if let Some(name) = &admin_name {
					let confirm = MessageDialog::builder(
						&parts.frame,
						&format!(
							"As an admin, you are deleting {name}'s message for everyone in this chat. They will see that you deleted the message."
						),
						"Delete for everyone",
					)
					.with_style(MessageDialogStyle::OK | MessageDialogStyle::Cancel | MessageDialogStyle::IconWarning)
					.build();
					confirm.set_ok_cancel_labels("Delete for everyone", "Cancel");
					if confirm.show_modal() != wxdragon::id::ID_OK {
						return;
					}
				}
				true
			}
			id if id == wxdragon::id::ID_NO => false,
			_ => return,
		}
	} else {
		let dialog = MessageDialog::builder(&parts.frame, &title, "Delete")
			.with_style(MessageDialogStyle::OK | MessageDialogStyle::Cancel | MessageDialogStyle::IconWarning)
			.build();
		dialog.set_ok_cancel_labels("Delete for me", "Cancel");
		if dialog.show_modal() != wxdragon::id::ID_OK {
			return;
		}
		false
	};
	{
		let mut s = state.borrow_mut();
		let items: Vec<serde_json::Value> = msgs
			.iter()
			.map(|m| {
				serde_json::json!({
					"message_id": m.id,
					"sender_jid": if m.from_me { "me".to_string() } else { m.sender_jid.clone() },
				})
			})
			.collect();
		// One message keeps its own "Message deleted." when the backend
		// confirms; several are announced once, here.
		if msgs.len() > 1 {
			for m in &msgs {
				s.bulk_delete_pending.insert((chat.clone(), m.id.clone()));
			}
		}
		let payload = serde_json::json!({
			"action": "delete_messages",
			"data": { "jid": chat, "messages": items, "for_everyone": for_everyone }
		});
		let _ = s.ws_tx.send(payload.to_string());
	}
	if from_selection {
		finish(state, parts);
	}
	if msgs.len() > 1 {
		announce(parts, &format!("{} deleted.", plural(msgs.len(), "message", "messages")));
	}
}

fn sender_name(state: &Rc<RefCell<AppState>>, m: &whatsnap::ChatMessage) -> String {
	let s = state.borrow();
	Some(m.sender_name.clone())
		.filter(|n| !n.is_empty() && *n != m.sender_jid)
		.or_else(|| s.contacts.get(&m.sender_jid).cloned())
		.unwrap_or_else(|| crate::ui::contact_picker::display_name_for_jid(&m.sender_jid))
}

/// Forwards the selection, oldest first, to up to five chats.
pub(crate) fn forward_selected(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let (ids, targets) = {
		let s = state.borrow();
		let Some(chat) = s.selected_chat.clone() else { return };
		let msgs = selected_messages(&s);
		if msgs.is_empty() || !msgs.iter().all(|m| can_forward(&chat, m)) {
			return;
		}
		let ids: Vec<String> = msgs.iter().map(|m| m.id.clone()).collect();
		(ids, crate::ui_commands::links::send_targets(&s))
	};
	let title = if ids.len() == 1 { "Forward message to" } else { "Forward messages to" };
	let Some(to_jids) = crate::ui::dialogs::send_to::show_forward_dialog(&parts.frame, title, targets) else {
		return;
	};
	if to_jids.is_empty() {
		return;
	}
	let payload = serde_json::json!({
		"action": "forward_message",
		"data": { "message_ids": ids, "to_jids": to_jids }
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	finish(state, parts);
	let names: Vec<String> = {
		let s = state.borrow();
		to_jids.iter().map(|jid| s.contacts.get(jid).cloned().unwrap_or_else(|| jid.clone())).collect()
	};
	announce(parts, &format!("{} forwarded to {}.", plural(ids.len(), "message", "messages"), names.join(", ")));
}

/// One file: a Save As box. Several: a folder, as WhatsApp Web on Windows
/// does with "Save as...".
pub(crate) fn save_selected(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let (chat, msgs) = {
		let s = state.borrow();
		let Some(chat) = s.selected_chat.clone() else { return };
		let msgs = selected_messages(&s);
		if msgs.is_empty() || !msgs.iter().all(|m| can_save(m)) {
			return;
		}
		(chat, msgs)
	};
	if msgs.len() == 1 {
		finish(state, parts);
		crate::ui_commands::attachments::save_media_message_as(state, parts, msgs[0].id.clone());
		return;
	}
	let dir_dialog = wxdragon::DirDialog::builder(&parts.frame, "Choose a folder to save the files into", "").build();
	if dir_dialog.show_modal() != wxdragon::id::ID_OK {
		return;
	}
	let Some(dir) = dir_dialog.get_path() else { return };
	finish(state, parts);
	let dir = std::path::PathBuf::from(dir);
	let mut used_names = std::collections::HashMap::new();
	let (mut copied, mut fetching, mut failed) = (0, 0, 0);
	for m in &msgs {
		let name = crate::platform::clipboard::unique_clipboard_filename(&m.display_filename(), &mut used_names);
		let dest = dir.join(name);
		if m.is_media_downloaded() {
			if std::fs::copy(&m.media_path, &dest).is_ok() {
				copied += 1;
			} else {
				failed += 1;
			}
		} else {
			let payload = serde_json::json!({
				"action": "save_media_as",
				"data": { "jid": chat, "message_id": m.id, "dest_path": dest.to_string_lossy() }
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
			fetching += 1;
		}
	}
	let mut text = format!("Saved {} to {}.", plural(copied, "file", "files"), dir.display());
	if fetching > 0 {
		text.push_str(&format!(" Downloading {} more.", fetching));
	}
	if failed > 0 {
		text.push_str(&format!(" {} couldn't be saved.", plural(failed, "file", "files")));
	}
	announce(parts, &text);
}

/// The select bar's buttons.
#[derive(Clone, Copy, Debug)]
pub enum BarAction {
	Copy,
	Star,
	Keep,
	Delete,
	Forward,
	Save,
}

pub(crate) fn run(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, action: BarAction) {
	match action {
		BarAction::Copy => copy_selected(state, parts),
		BarAction::Star => star_selected(state, parts),
		BarAction::Keep => keep_selected(state, parts),
		BarAction::Delete => delete_selected(state, parts),
		BarAction::Forward => forward_selected(state, parts),
		BarAction::Save => save_selected(state, parts),
	}
}
