//! Group member menu items that need the backend: Verify security code and
//! Send message history (WA Web's participant menu in Group Info).

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{AppState, GroupMember, ui::window::WindowParts};

thread_local! {
	/// The member whose security code was asked for: (jid asked with, name).
	static PENDING_CODE: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
	/// The member message history is being sent to, for the result message.
	static PENDING_HISTORY: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn owner(parts: &WindowParts) -> Dialog {
	parts.group_data_dialog.dialog
}

fn tell(parts: &WindowParts, text: &str, title: &str, error: bool) {
	let icon = if error { MessageDialogStyle::IconError } else { MessageDialogStyle::IconInformation };
	MessageDialog::builder(&owner(parts), text, title).with_style(MessageDialogStyle::OK | icon).build().show_modal();
}

pub(crate) fn request_security_code(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>, member: &GroupMember) {
	let jid = member.chat_jid().to_string();
	PENDING_CODE.with(|p| *p.borrow_mut() = Some((jid.clone(), member.name.clone())));
	let payload = serde_json::json!({ "action": "get_security_code", "data": { "jid": jid } });
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

/// The 60 digits as WA Web shows them: 12 groups of 5, four to a line.
fn format_code(digits: &str) -> String {
	let groups: Vec<&str> = digits.as_bytes().chunks(5).filter_map(|c| std::str::from_utf8(c).ok()).collect();
	groups.chunks(4).map(|line| line.join(" ")).collect::<Vec<_>>().join("\n")
}

fn from_hex(hex: &str) -> Option<Vec<u8>> {
	(0..hex.len()).step_by(2).map(|i| hex.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok())).collect()
}

pub(crate) fn show_security_code(_state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or_default();
	let Some((_, name)) = PENDING_CODE.with(|p| p.borrow_mut().take_if(|(asked, _)| asked == jid)) else { return };
	if let Some(error) = data.get("error").and_then(|e| e.as_str()) {
		let err_tr = crate::i18n::translate_backend_message(error);
		let msg = crate::i18n::tr_fmt("Couldn't get the security code with {}. {}", &[&name, &err_tr]);
		tell(parts, &msg, crate::i18n::tr("Verify Security Code"), true);
		return;
	}
	let digits = data.get("digits").and_then(|d| d.as_str()).unwrap_or_default().to_string();
	let qr = data.get("qr").and_then(|q| q.as_str()).and_then(from_hex);

	let dialog = Dialog::builder(&owner(parts), crate::i18n::tr("Verify Security Code")).build();
	let panel = Panel::builder(&dialog).build();
	let sizer = BoxSizer::builder(Orientation::Vertical).build();
	let qr_row = BoxSizer::builder(Orientation::Horizontal).build();
	sizer.add_sizer(&qr_row, 0, SizerFlag::AlignCenterHorizontal | SizerFlag::Top, 8);
	let label_text = crate::i18n::tr_fmt("Security code with {}", &[&name]);
	let label = StaticText::builder(&panel).with_label(&label_text).build();
	sizer.add(&label, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 8);
	let code_field = TextCtrl::builder(&panel)
		.with_value(&format_code(&digits))
		.with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly)
		.with_size(Size::new(300, 80))
		.build();
	sizer.add(&code_field, 0, SizerFlag::Expand | SizerFlag::All, 8);
	let about_text = crate::i18n::tr_fmt("To check that your messages and calls with {} are end-to-end encrypted, compare these 60 \
			 numbers with the ones on their phone, or let them scan the QR code with their phone. The code is \
			 the same on all your devices.", &[&name]);
	let about = StaticText::builder(&panel).with_label(&about_text).build();
	about.wrap(320);
	sizer.add(&about, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 8);
	let buttons = BoxSizer::builder(Orientation::Horizontal).build();
	let copy_btn = Button::builder(&panel).with_label(crate::i18n::tr("Copy code")).build();
	buttons.add(&copy_btn, 0, SizerFlag::Right, 8);
	buttons.add_stretch_spacer(1);
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label(crate::i18n::tr("Close")).build();
	buttons.add(&close_btn, 0, SizerFlag::Right, 0);
	sizer.add_sizer(&buttons, 0, SizerFlag::Expand | SizerFlag::All, 8);
	// Made last, so it never names a control and isn't in Tab's way.
	if let Some(qr) = qr.and_then(|bytes| qrcode::QrCode::new(bytes).ok()) {
		let (width, height, rgba) = crate::backend::events::qr_rgba(&qr);
		if let Some(bitmap) = wxdragon::bitmap::Bitmap::from_rgba(&rgba, width, height) {
			let image = StaticBitmap::builder(&panel).with_bitmap(Some(bitmap)).build();
			image.set_min_size(Size::new(width as i32, height as i32));
			qr_row.add(&image, 0, SizerFlag::AlignCenterVertical, 0);
		}
	}
	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer_and_fit(outer, true);
	dialog.center();

	let copied_digits = digits.clone();
	let copy_parent = dialog;
	copy_btn.on_click(move |_| {
		let text = if crate::copy_text(&copied_digits) {
			crate::i18n::tr("Code copied.")
		} else {
			crate::i18n::tr("Couldn't copy the code.")
		};
		MessageDialog::builder(&copy_parent, text, crate::i18n::tr("Copy code"))
			.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
			.build()
			.show_modal();
	});
	let close = dialog;
	close_btn.on_click(move |_| close.end_modal(ID_CANCEL));
	code_field.set_focus();
	dialog.show_modal();
	dialog.destroy();
}

pub(crate) fn send_history(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>, group: &str, member: &GroupMember) {
	PENDING_HISTORY.with(|p| *p.borrow_mut() = Some(member.name.clone()));
	let payload = serde_json::json!({
		"action": "send_group_history",
		"data": { "jid": group, "participant": member.jid }
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

pub(crate) fn group_history_result(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let name = PENDING_HISTORY.with(|p| p.borrow_mut().take()).unwrap_or_else(|| crate::i18n::tr("them").to_string());
	if data.get("ok").and_then(|o| o.as_bool()) == Some(true) {
		let count = data.get("count").and_then(|c| c.as_u64()).unwrap_or(0);
		let msg = crate::i18n::tr_fmt("Message history sent to {}: {}.", &[&name, &crate::i18n::count_noun(count as usize, "message", "messages")]);
		tell(parts, &msg, crate::i18n::tr("Send Message History"), false);
		// Refresh the members so the menu no longer offers it.
		let group = state.borrow().selected_chat.clone();
		if let Some(jid) = group.filter(|_| parts.group_data_dialog.dialog.is_shown()) {
			let payload = serde_json::json!({ "action": "get_group_info", "data": { "jid": jid } });
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
	} else {
		let message = data
			.get("message")
			.and_then(|m| m.as_str())
			.map(crate::i18n::translate_backend_message)
			.unwrap_or_else(|| crate::i18n::tr("Couldn't send the message history.").to_string());
		tell(parts, &message, crate::i18n::tr("Send Message History"), true);
	}
}

pub(crate) fn request_member_changes(state: &Rc<RefCell<AppState>>) {
	let s = state.borrow();
	let Some(jid) = s.selected_chat.clone().filter(|j| j.ends_with("@g.us")) else { return };
	let payload = serde_json::json!({ "action": "get_member_changes", "data": { "jid": jid } });
	let _ = s.ws_tx.send(payload.to_string());
}

pub(crate) fn show_member_changes(_state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let changes = data.get("changes").and_then(|c| c.as_array()).cloned().unwrap_or_default();
	let owner = &parts.group_data_dialog.dialog;
	if changes.is_empty() {
		MessageDialog::builder(
			owner,
			crate::i18n::tr("No one has joined, left or been added or removed."),
			crate::i18n::tr("Member changes"),
		)
		.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
		.build()
		.show_modal();
		return;
	}
	let lines: Vec<String> = changes
		.iter()
		.map(|c| {
			let text = crate::i18n::translate_system_message(c.get("text").and_then(|t| t.as_str()).unwrap_or(""));
			let when = c
				.get("timestamp")
				.and_then(|t| t.as_i64())
				.and_then(whatsnap::local_time::LocalTime::from_unix)
				.map(|t| t.format("%Y-%m-%d %H:%M"))
				.unwrap_or_default();
			format!("{when}: {text}")
		})
		.collect();
	let dialog = Dialog::builder(owner, crate::i18n::tr("Member changes"))
		.with_size(520, 400)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	let panel = Panel::builder(&dialog).build();
	let sizer = BoxSizer::builder(Orientation::Vertical).build();
	let label = StaticText::builder(&panel).with_label(crate::i18n::tr("Member changes, newest first")).build();
	sizer.add(&label, 0, SizerFlag::All, 8);
	let list = ListBox::builder(&panel).with_choices(lines).build();
	list.set_selection(0_u32, true);
	sizer.add(&list, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 8);
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label(crate::i18n::tr("Close")).build();
	sizer.add(&close_btn, 0, SizerFlag::All, 8);
	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();
	list.set_focus();
	dialog.show_modal();
	dialog.destroy();
}
