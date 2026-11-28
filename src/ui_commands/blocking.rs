use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState, ContactPickerMode,
	ui::{dialogs::blocked_contacts::BlockedEntry, window::WindowParts},
};

pub(crate) fn is_blocked(s: &AppState, jid: &str) -> bool {
	s.blocked_jids.contains(jid)
}

fn can_block(jid: &str) -> bool {
	jid.ends_with("@s.whatsapp.net") || jid.ends_with("@lid")
}

fn chat_name(state: &AppState, jid: &str) -> String {
	state.contacts.get(jid).cloned().unwrap_or_else(|| jid.to_string())
}

fn announce(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, message: &str) {
	let dialog = state.borrow().blocked_contacts.clone();
	match dialog {
		Some(dialog) => dialog.announce(message),
		None => crate::platform::announce::announce(parts.conversations_parts.messages_announcer, message),
	}
}

fn send(state: &Rc<RefCell<AppState>>, action: &str, jid: &str) {
	let payload = serde_json::json!({ "action": action, "data": { "jid": jid } });
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

fn active_modal_parent(state: &AppState, parts: &WindowParts) -> Option<Dialog> {
	if let Some(ref blocked) = state.blocked_contacts {
		if blocked.dialog.is_shown() {
			return Some(blocked.dialog);
		}
	}
	if parts.conversation_data_dialog.details_dialog.is_shown() {
		return Some(parts.conversation_data_dialog.details_dialog);
	}
	if parts.conversation_data_dialog.dialog.is_shown() {
		return Some(parts.conversation_data_dialog.dialog);
	}
	if parts.group_data_dialog.dialog.is_shown() {
		return Some(parts.group_data_dialog.dialog);
	}
	if parts.settings_dialog.dialog.is_shown() {
		return Some(parts.settings_dialog.dialog);
	}
	None
}

struct BlockConfirmResult {
	confirmed: bool,
	report: bool,
}

fn ask_block_confirm(parent: &impl WxWidget, name: &str, is_business: bool) -> BlockConfirmResult {
	let title = crate::i18n::tr_fmt("Block {}?", &[&name]);
	let dialog = Dialog::builder(parent, &title)
		.with_size(440, 240)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	crate::ui::theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	crate::ui::theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let question_text = if is_business {
		crate::i18n::tr_fmt("{} won't be able to message or call you.", &[&name])
	} else {
		crate::i18n::tr(
			"This person won't be able to message or call you. They won't know you blocked or reported them.",
		)
		.to_string()
	};
	let question = StaticText::builder(&panel).with_label(&question_text).build();
	question.set_font(&crate::ui::theme::Fonts::body());
	question.wrap(400);
	sizer.add(&question, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 16);

	let report_check = CheckBox::builder(&panel).with_label(crate::i18n::tr("Report to WhatsApp")).build();
	crate::ui::dialog_look::switch(&report_check, crate::ui::theme::Palette::CONTENT_BG());
	sizer.add(&report_check, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);

	let report_hint_text = if is_business {
		crate::i18n::tr("WhatsApp receives the last message this business sent you.")
	} else {
		crate::i18n::tr("WhatsApp receives the last message this person sent you.")
	};
	let report_hint = StaticText::builder(&panel).with_label(report_hint_text).build();
	crate::ui::theme::style_muted_label(&report_hint);
	report_hint.wrap(400);
	sizer.add(&report_hint, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 4);

	let block_btn = Button::builder(&panel).with_id(ID_OK).with_label(crate::i18n::tr("Block")).build();
	crate::ui::dialog_look::pill_button(&block_btn, true, crate::ui::theme::Palette::CONTENT_BG());

	let cancel_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label(crate::i18n::tr("Cancel")).build();
	crate::ui::dialog_look::pill_button(&cancel_btn, false, crate::ui::theme::Palette::CONTENT_BG());

	crate::ui::dialog_look::button_row(&sizer, &[&block_btn, &cancel_btn]);
	panel.set_sizer(sizer, true);

	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();
	crate::ui::keys::escape_cancels_anywhere(dialog);
	block_btn.on_click(move |_| dialog.end_modal(ID_OK));
	cancel_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));
	block_btn.set_focus();

	let confirmed = dialog.show_modal() == ID_OK;
	let report = report_check.is_checked();
	dialog.destroy();
	BlockConfirmResult { confirmed, report }
}

fn block_with_confirm(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: &str) {
	let (name, is_biz) = {
		let s = state.borrow();
		(chat_name(&s, jid), s.business_jids.contains(jid))
	};
	// A borrow in the match scrutinee lives through the modal; the window
	// regaining focus when it closes then borrows state mutably and panics.
	let parent = active_modal_parent(&state.borrow(), parts);
	let result = match parent {
		Some(dlg) => ask_block_confirm(&dlg, &name, is_biz),
		None => ask_block_confirm(&parts.frame, &name, is_biz),
	};
	if !result.confirmed {
		return;
	}
	if result.report {
		let payload = serde_json::json!({
			"action": "report",
			"data": { "jid": jid, "message_id": "", "block": false, "leave": false }
		});
		let _ = state.borrow().ws_tx.send(payload.to_string());
	}
	send(state, "block_contact", jid);
	let msg = crate::i18n::tr_fmt("Blocking {}...", &[&name]);
	announce(state, parts, &msg);
}

pub(crate) fn toggle_block(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String) {
	if !can_block(&jid) {
		announce(state, parts, crate::i18n::tr("Only contacts can be blocked."));
		return;
	}
	if is_blocked(&state.borrow(), &jid) {
		let name = chat_name(&state.borrow(), &jid);
		send(state, "unblock_contact", &jid);
		let msg = crate::i18n::tr_fmt("Unblocking {}...", &[&name]);
		announce(state, parts, &msg);
	} else {
		block_with_confirm(state, parts, &jid);
	}
}

pub(crate) fn block_picked_contact(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String) {
	if is_blocked(&state.borrow(), &jid) {
		let name = chat_name(&state.borrow(), &jid);
		let msg = crate::i18n::tr_fmt("{} is already blocked.", &[&name]);
		announce(state, parts, &msg);
		return;
	}
	block_with_confirm(state, parts, &jid);
}

fn parse_blocklist(data: &serde_json::Value) -> Option<(std::collections::HashSet<String>, Vec<BlockedEntry>)> {
	#[derive(serde::Deserialize)]
	struct Entry {
		jid: String,
		#[serde(default)]
		alts: Vec<String>,
		#[serde(default)]
		name: String,
		#[serde(default)]
		phone: String,
	}
	#[derive(serde::Deserialize)]
	struct List {
		entries: Vec<Entry>,
	}
	let list: List = serde_json::from_value(data.clone()).ok()?;
	let mut jids = std::collections::HashSet::new();
	let mut entries = Vec::new();
	for e in list.entries {
		if !can_block(&e.jid) {
			return None;
		}
		for alt in e.alts {
			if can_block(&alt) {
				jids.insert(alt);
			}
		}
		jids.insert(e.jid.clone());
		entries.push(BlockedEntry { jid: e.jid, name: e.name, phone: e.phone });
	}
	Some((jids, entries))
}

pub(crate) fn blocklist_received(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let Some((jids, entries)) = parse_blocklist(data) else { return };
	let (selected_changed, info_changed) = {
		let s = state.borrow();
		let changed = |jid: &str| s.blocked_jids.contains(jid) != jids.contains(jid);
		(s.selected_chat.as_deref().is_some_and(changed), s.info_chat.as_deref().is_some_and(changed))
	};
	let dialog = {
		let mut s = state.borrow_mut();
		s.blocked_jids = jids;
		s.blocked_entries = entries.clone();
		s.blocked_contacts.clone()
	};
	if let Some(dialog) = dialog {
		dialog.apply(&entries);
	}
	let s = state.borrow();
	if selected_changed && let Some(jid) = s.selected_chat.as_deref() {
		let cp = &parts.conversations_parts;
		let composer_focused = cp.message_field.has_focus() || cp.blocked_banner_btn.has_focus();
		crate::ui::chat_ui::apply_composer_access(&s, parts, jid);
		crate::ui::chat_ui::update_composer_buttons(&s, cp);
		crate::ui_commands::calls::apply_call_buttons(&s, parts, jid);
		cp.top_btns_panel.layout();
		cp.conversation_panel.layout();
		crate::ui::window::refresh_menu_bar(&s, &parts.frame);
		if composer_focused {
			if is_blocked(&s, jid) {
				cp.blocked_banner_btn.set_focus();
			} else {
				cp.message_field.set_focus();
			}
		}
	}
	if info_changed && let Some(jid) = s.info_chat.as_deref() {
		let d = &parts.conversation_data_dialog;
		crate::ui::chat_overview::refresh_block_state(&s, &d.overview, jid);
		let text = update_info_block_line(&d.info_field.get_value(), blocked_info_line(), is_blocked(&s, jid));
		d.info_field.set_value(&text);
		let callable = crate::ui_commands::calls::can_call(&s, jid);
		d.voice_call_btn.show(callable);
		d.video_call_btn.show(callable);
		d.info_panel.layout();
	}
}

pub(crate) fn block_result(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let name = data.get("name").and_then(|n| n.as_str()).unwrap_or_else(|| crate::i18n::tr("Contact"));
	let blocked = data.get("blocked").and_then(|b| b.as_bool()).unwrap_or(false);
	let Some(error) = data.get("error").and_then(|e| e.as_str()).filter(|e| !e.is_empty()) else {
		// Only the authoritative blocklist event changes state. Applying the
		// command's original JID here leaves alternate LIDs blocked on unblock,
		// and a late result can undo a newer snapshot.
		let msg = crate::i18n::tr_fmt(if blocked { "{} blocked." } else { "{} unblocked." }, &[&name]);
		announce(state, parts, &msg);
		return;
	};
	let err_text = crate::i18n::translate_backend_message(error);
	let message = crate::i18n::tr_fmt(
		if blocked { "Couldn't block {}: {}" } else { "Couldn't unblock {}: {}" },
		&[&name, &err_text],
	);
	let title = crate::i18n::tr("Error");
	let style = MessageDialogStyle::OK | MessageDialogStyle::IconError;
	let parent = active_modal_parent(&state.borrow(), parts);
	if let Some(dialog) = parent {
		let owner = windows::Win32::Foundation::HWND(dialog.get_handle());
		crate::platform::focus::show_modal_and_refocus(owner, false, || {
			MessageDialog::builder(&dialog, &message, title).with_style(style).build().show_modal()
		});
	} else {
		let owner = windows::Win32::Foundation::HWND(parts.frame.get_handle());
		crate::platform::focus::show_modal_and_refocus(owner, false, || {
			MessageDialog::builder(&parts.frame, &message, title).with_style(style).build().show_modal()
		});
	}
}

pub(crate) fn show_blocked_contacts(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let (ws_tx, entries) = {
		let s = state.borrow();
		(s.ws_tx.clone(), s.blocked_entries.clone())
	};
	let _ = ws_tx.send(serde_json::json!({ "action": "get_blocklist" }).to_string());
	let (state_add, parts_add) = (state.clone(), parts.clone());
	let state_store = state.clone();
	let add = move || {
		crate::ui_commands::contacts_groups::prepare_contact_picker(
			&state_add,
			&parts_add,
			ContactPickerMode::BlockContact,
		);
	};
	let store = move |dialog| state_store.borrow_mut().blocked_contacts = dialog;
	// Opened from Settings, it goes over Settings, so focus comes back there.
	let settings = parts.settings_dialog.dialog;
	if settings.is_shown() {
		crate::ui::dialogs::blocked_contacts::show_blocked_contacts(&settings, &entries, ws_tx, add, store);
	} else {
		crate::ui::dialogs::blocked_contacts::show_blocked_contacts(&parts.frame, &entries, ws_tx, add, store);
	}
}

pub(crate) fn stop_if_blocked(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) -> bool {
	let Some(jid) = state.borrow().selected_chat.clone() else { return false };
	stop_if_blocked_jid(state, parts, &jid)
}

pub(crate) fn stop_if_blocked_jid(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: &str) -> bool {
	let name = {
		let s = state.borrow();
		if !is_blocked(&s, jid) {
			return false;
		}
		chat_name(&s, jid)
	};
	let question = crate::i18n::tr_fmt("You blocked {}. Unblock them to message or call them?", &[&name]);
	let title = crate::i18n::tr("Blocked contact");
	let style = MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion;
	let parent = active_modal_parent(&state.borrow(), parts);
	let answer = if let Some(dialog) = parent {
		MessageDialog::builder(&dialog, &question, title).with_style(style).build().show_modal()
	} else {
		MessageDialog::builder(&parts.frame, &question, title).with_style(style).build().show_modal()
	};
	if answer == ID_YES {
		send(state, "unblock_contact", jid);
		let msg = crate::i18n::tr_fmt("Unblocking {}...", &[&name]);
		announce(state, parts, &msg);
	}
	true
}

pub(crate) fn stop_if_cant_send(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) -> bool {
	if stop_if_blocked(state, parts) {
		return true;
	}
	let reason = {
		let s = state.borrow();
		s.selected_chat.as_deref().and_then(|jid| crate::ui::chat_ui::group_send_block_reason(&s, jid))
	};
	if let Some(reason) = reason {
		announce(state, parts, reason);
		return true;
	}
	false
}

pub(crate) fn info_line(s: &AppState, jid: &str) -> &'static str {
	if is_blocked(s, jid) { blocked_info_line() } else { "" }
}

fn blocked_info_line() -> &'static str {
	let line = crate::i18n::tr("Blocked: Yes\n");
	if crate::i18n::is_indonesian() && line == "Blocked: Yes\n" { "Diblokir: Ya\n" } else { line }
}

// The details field starts with Name and Phone; change only the block line
// immediately after them, preserving the loaded About and WhatsApp name.
fn update_info_block_line(text: &str, line: &str, blocked: bool) -> String {
	let mut lines = text.splitn(3, '\n');
	let (Some(name), Some(phone), Some(rest)) = (lines.next(), lines.next(), lines.next()) else {
		return text.to_string();
	};
	let rest = rest.strip_prefix(line).unwrap_or(rest);
	format!("{name}\n{phone}\n{}{rest}", if blocked { line } else { "" })
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn snapshots_include_every_address_and_empty_is_distinct_from_invalid() {
		let data = serde_json::json!({ "entries": [
			{ "jid": "1@s.whatsapp.net", "alts": ["2@lid", "3@lid"], "name": "A", "phone": "+1" }
		] });
		let (jids, entries) = parse_blocklist(&data).unwrap();
		assert_eq!(jids.len(), 3);
		assert!(jids.contains("1@s.whatsapp.net") && jids.contains("2@lid") && jids.contains("3@lid"));
		assert_eq!(entries[0].phone, "+1");
		assert!(parse_blocklist(&serde_json::json!({ "entries": [] })).unwrap().0.is_empty());
		for invalid in [
			serde_json::json!({}),
			serde_json::json!({ "entries": null }),
			serde_json::json!({ "entries": [{ "jid": "1@g.us" }] }),
		] {
			assert!(parse_blocklist(&invalid).is_none());
		}
	}

	#[test]
	fn block_info_updates_preserve_metadata_and_do_not_duplicate_the_line() {
		let original = "Name: A\nPhone: +1\nWhatsApp Name: B\nAbout/Status: Blocked: Yes\n";
		let blocked = update_info_block_line(original, "Blocked: Yes\n", true);
		assert_eq!(blocked, "Name: A\nPhone: +1\nBlocked: Yes\nWhatsApp Name: B\nAbout/Status: Blocked: Yes\n");
		assert_eq!(update_info_block_line(&blocked, "Blocked: Yes\n", true), blocked);
		assert_eq!(update_info_block_line(&blocked, "Blocked: Yes\n", false), original);
	}
}
