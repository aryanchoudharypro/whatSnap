use wxdragon::prelude::*;
use wxdragon::event::{WxEvtHandler, EventType};

use crate::UiCommand;
use crate::ui_wake::UiCommandSender;

pub struct WindowParts {
	pub frame: Frame,
	pub nav_list: ListBox,
	pub content_panel: Panel,
	pub login_panel: Panel,
	pub qrcode_dialog: Dialog,
	pub qrcode_display: StaticBitmap,
	pub qrcode_announcer: StaticText,
	pub qrcode_cancel_btn: Button,
	pub phone_panel: Panel,
	pub country_combo: ComboBox,
	pub phone_field: TextCtrl,
	pub pair_btn: Button,
	pub switch_to_qr_btn: Button,
	pub pairing_dialog: Dialog,
	pub pairing_code_display: TextCtrl,
	pub pairing_announcer: StaticText,
	pub pairing_cancel_btn: Button,
	pub new_conv_dialog: crate::ui::new_conversation::NewConversationDialogParts,
	pub new_contact_dialog: crate::ui::new_contact::NewContactDialogParts,
	pub new_group_dialog: crate::ui::new_group::NewGroupDialogParts,
	pub attachment_dialog: crate::ui::attachment_dialog::AttachmentDialogParts,
	pub conversations_parts: crate::ui::conversations::ConversationsPanelParts,
}

pub fn build_main_window() -> WindowParts {
	let frame = Frame::builder().with_title("whantv").with_size(Size::new(900, 600)).build();
	wxdragon::app::set_top_window(&frame);

	let main_panel = Panel::builder(&frame).build();
	let main_sizer = BoxSizer::builder(Orientation::Horizontal).build();

	let nav_panel = Panel::builder(&main_panel).build();
	let nav_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let nav_label = StaticText::builder(&nav_panel).with_label("Main Navigation").build();
	let nav_list = ListBox::builder(&nav_panel).with_choices(vec![
		"Conversations".to_string(),
		"Archived Chats".to_string(),
	]).build();
	nav_list.set_selection(0_u32, true);
	nav_sizer.add(&nav_label, 0, SizerFlag::All, 5);
	nav_sizer.add(&nav_list, 1, SizerFlag::Expand | SizerFlag::All, 5);
	nav_panel.set_sizer(nav_sizer, true);

	let content_panel = Panel::builder(&main_panel).build();
	let content_sizer = BoxSizer::builder(Orientation::Vertical).build();

	let login_panel = Panel::builder(&content_panel).build();
	let login_sizer = BoxSizer::builder(Orientation::Vertical).build();
	
	let qrcode_dialog = Dialog::builder(&frame, "QR Code").with_size(550, 550).build();
	let qr_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let qrcode_instructions = StaticText::builder(&qrcode_dialog).with_label("Scan this QR Code with WhatsApp:").build();
	let empty_bitmap = wxdragon::bitmap::Bitmap::from_rgba(&[255, 255, 255, 255], 1, 1).unwrap();
	let qrcode_display = StaticBitmap::builder(&qrcode_dialog).with_bitmap(Some(empty_bitmap)).with_scale_mode(Some(ScaleMode::AspectFit)).build();
	
	let qrcode_announcer = StaticText::builder(&qrcode_dialog).with_label("").build();
	live_region::set_live_region(&qrcode_announcer);
	
	let qrcode_cancel_btn = Button::builder(&qrcode_dialog).with_label("Cancel QR Code").build();
	qr_sizer.add(&qrcode_instructions, 0, SizerFlag::All | SizerFlag::AlignCentre, 10);
	qr_sizer.add(&qrcode_display, 0, SizerFlag::Expand | SizerFlag::All, 10);
	qr_sizer.add(&qrcode_announcer, 0, SizerFlag::Left, 0);
	qr_sizer.add(&qrcode_cancel_btn, 0, SizerFlag::All | SizerFlag::AlignCentre, 10);
	qrcode_dialog.set_sizer(qr_sizer, true);

	// Phone Panel
	let phone_panel = Panel::builder(&login_panel).build();
	let phone_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let country_label = StaticText::builder(&phone_panel).with_label("Country:").build();
	let mut choices = Vec::new();
	for &(name, _) in crate::countries::COUNTRIES {
		choices.push(name.to_string());
	}
	let country_combo = ComboBox::builder(&phone_panel).with_choices(choices).with_style(ComboBoxStyle::ReadOnly).build();
	country_combo.set_selection(0_u32);

	let phone_label = StaticText::builder(&phone_panel).with_label("Enter phone number:").build();
	let phone_field = TextCtrl::builder(&phone_panel).with_value("+55 ").build();
	let pair_btn = Button::builder(&phone_panel).with_label("Continue").build();
	let switch_to_qr_btn = Button::builder(&phone_panel).with_label("Connect with QR Code").build();

	phone_sizer.add(&country_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	phone_sizer.add(&country_combo, 0, SizerFlag::Expand | SizerFlag::All, 10);
	phone_sizer.add(&phone_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	phone_sizer.add(&phone_field, 0, SizerFlag::Expand | SizerFlag::All, 10);
	phone_sizer.add(&pair_btn, 0, SizerFlag::All | SizerFlag::AlignCentre, 10);
	phone_sizer.add(&switch_to_qr_btn, 0, SizerFlag::All | SizerFlag::AlignCentre, 10);
	phone_panel.set_sizer(phone_sizer, true);

	login_sizer.add(&phone_panel, 1, SizerFlag::Expand | SizerFlag::All, 5);
	phone_panel.show(true);
	login_panel.set_sizer(login_sizer, true);
	
	// Pairing Dialog
	let pairing_dialog = Dialog::builder(&frame, "Pairing Code").with_size(300, 150).build();
	let p_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let pairing_instructions = StaticText::builder(&pairing_dialog).with_label("Enter this pairing code in WhatsApp on your phone:").build();
	let pairing_code_display = TextCtrl::builder(&pairing_dialog).with_style(TextCtrlStyle::ReadOnly | TextCtrlStyle::Centre).build();
	
	let pairing_announcer = StaticText::builder(&pairing_dialog).with_label("").build();
	live_region::set_live_region(&pairing_announcer);
	
	let pairing_cancel_btn = Button::builder(&pairing_dialog).with_label("Cancel").build();
	p_sizer.add(&pairing_instructions, 0, SizerFlag::All | SizerFlag::AlignCentre, 10);
	p_sizer.add(&pairing_code_display, 0, SizerFlag::Expand | SizerFlag::All, 10);
	p_sizer.add(&pairing_announcer, 0, SizerFlag::Left, 0); // hidden visually but in DOM
	pairing_announcer.show(false);
	p_sizer.add(&pairing_cancel_btn, 0, SizerFlag::All | SizerFlag::AlignCentre, 10);
	pairing_dialog.set_sizer(p_sizer, true);

	let new_conv_dialog = crate::ui::new_conversation::build_new_conversation_dialog(&frame);
	let new_contact_dialog = crate::ui::new_contact::build_new_contact_dialog(&frame);
	let new_group_dialog = crate::ui::new_group::build_new_group_dialog(&frame);
	let attachment_dialog = crate::ui::attachment_dialog::build_attachment_dialog(&frame);
	let conversations_parts = crate::ui::conversations::build_conversations_panel(&content_panel);

	content_sizer.add(&conversations_parts.panel, 1, SizerFlag::Expand, 0);
	content_sizer.add(&login_panel, 1, SizerFlag::Expand, 0);
	conversations_parts.panel.show(false);
	login_panel.show(true);
	content_panel.set_sizer(content_sizer, true);

	main_sizer.add(&nav_panel, 0, SizerFlag::Expand | SizerFlag::All, 5);
	main_sizer.add(&content_panel, 1, SizerFlag::Expand | SizerFlag::All, 5);
	main_panel.set_sizer(main_sizer, true);

	let frame_sizer = BoxSizer::builder(Orientation::Vertical).build();
	frame_sizer.add(&main_panel, 1, SizerFlag::Expand, 0);
	frame.set_sizer(frame_sizer, true);
	WindowParts {
		frame,
		nav_list,
		content_panel,
		login_panel,
		qrcode_dialog,
		qrcode_display,
		qrcode_announcer,
		qrcode_cancel_btn,
		phone_panel,
		country_combo,
		phone_field,
		pair_btn,
		switch_to_qr_btn,
		pairing_dialog,
		pairing_code_display,
		pairing_announcer,
		pairing_cancel_btn,
		new_conv_dialog,
		new_contact_dialog,
		new_group_dialog,
		attachment_dialog,
		conversations_parts,
	}
}

pub fn bind_input_handlers(parts: &WindowParts, ui_tx: UiCommandSender, state: &std::rc::Rc<std::cell::RefCell<crate::AppState>>) {
	let tx_pair = ui_tx.clone();
	let phone_ctrl = parts.phone_field;
	let combo = parts.country_combo;
	combo.on_selection_changed(move |event| {
		if let Some(idx) = event.get_selection() {
			let code = crate::countries::COUNTRIES[idx as usize].1;
			phone_ctrl.set_value(&format!("+{} ", code));
		}
	});

	let phone_ctrl2 = parts.phone_field;
	parts.pair_btn.on_click(move |_| {
		let phone = phone_ctrl2.get_value();
		let clean_phone: String = phone.chars().filter(|c| c.is_digit(10)).collect();
		if !clean_phone.is_empty() {
			let _ = tx_pair.send(UiCommand::RequestPairing(clean_phone));
		}
	});
	
	let tx_qr = ui_tx.clone();
	parts.switch_to_qr_btn.on_click(move |_| {
		let _ = tx_qr.send(UiCommand::RequestQr);
	});

	let qr_dialog = parts.qrcode_dialog;
	let tx_cancel_qr = ui_tx.clone();
	parts.qrcode_cancel_btn.on_click(move |_| {
		qr_dialog.show(false);
		let _ = tx_cancel_qr.send(UiCommand::CancelQr);
	});
	
	let pairing_dialog = parts.pairing_dialog;
	let tx_cancel_pairing = ui_tx.clone();
	parts.pairing_cancel_btn.on_click(move |_| {
		pairing_dialog.show(false);
		let _ = tx_cancel_pairing.send(UiCommand::CancelPairing);
	});

	let tx_send = ui_tx.clone();
	let msg_ctrl = parts.conversations_parts.message_field;
	parts.conversations_parts.send_message_btn.on_click(move |_| {
		let text = msg_ctrl.get_value();
		if !text.is_empty() {
			let _ = tx_send.send(UiCommand::SendMessage(text));
		}
	});

	let tx_typing = ui_tx.clone();
	let msg_ctrl_typing = parts.conversations_parts.message_field;
	msg_ctrl_typing.on_text_changed(move |_| {
		let text = msg_ctrl_typing.get_value();
		let typing_state = if text.is_empty() { "paused" } else { "composing" };
		let _ = tx_typing.send(UiCommand::SendTyping {
			jid: String::new(),
			state: typing_state.to_string(),
			media: String::new(),
		});
	});

	let tx_select = ui_tx.clone();
	parts.conversations_parts.conversations_list.on_selection_changed(move |event| {
		if let Some(idx) = event.get_selection() {
			let _ = tx_select.send(UiCommand::ConversationSelectionChanged(idx as usize));
		}
	});

	let new_conv_dialog = parts.new_conv_dialog.dialog;
	parts.conversations_parts.new_conv_btn.on_click(move |_| {
		new_conv_dialog.show_modal();
	});

	let new_conv_dialog_close = parts.new_conv_dialog.dialog;
	parts.new_conv_dialog.close_btn.on_click(move |_| {
		new_conv_dialog_close.end_modal(0);
	});

	let new_contact_dialog = parts.new_contact_dialog.dialog;
	let new_conv_dlg_for_contact = parts.new_conv_dialog.dialog;
	parts.new_conv_dialog.new_contact_btn.on_click(move |_| {
		new_conv_dlg_for_contact.end_modal(0);
		new_contact_dialog.show_modal();
	});

	let new_contact_dialog_cancel = parts.new_contact_dialog.dialog;
	parts.new_contact_dialog.cancel_btn.on_click(move |_| {
		new_contact_dialog_cancel.end_modal(0);
	});

	let tx_new_contact = ui_tx.clone();
	let dlg = parts.new_contact_dialog.dialog;
	let name_field = parts.new_contact_dialog.name_field;
	let surname_field = parts.new_contact_dialog.surname_field;
	let phone_field = parts.new_contact_dialog.phone_field;
	parts.new_contact_dialog.ok_btn.on_click(move |_| {
		let name = name_field.get_value().trim().to_string();
		let surname = surname_field.get_value().trim().to_string();
		let phone = phone_field.get_value();
		if name.is_empty() {
			unsafe {
				let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(windows::Win32::UI::WindowsAndMessaging::MB_OK);
			}
			MessageDialog::builder(&dlg, "Name is required.", "Error").with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError).build().show_modal();
			name_field.set_focus();
			return;
		}
		let clean_phone: String = phone.chars().filter(|c| c.is_digit(10)).collect();
		if clean_phone.len() < 7 {
			unsafe {
				let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(windows::Win32::UI::WindowsAndMessaging::MB_OK);
			}
			MessageDialog::builder(&dlg, "Phone number must have at least 7 digits.", "Error").with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError).build().show_modal();
			phone_field.set_focus();
			return;
		}
		let full_name = format!("{} {}", name, surname).trim().to_string();
		dlg.end_modal(wxdragon::id::ID_OK);
		let _ = tx_new_contact.send(UiCommand::NewContact {
			name: full_name,
			phone: clean_phone,
		});
	});

	let tx_prepare = ui_tx.clone();
	let new_conv_dlg_for_group = parts.new_conv_dialog.dialog;
	parts.new_conv_dialog.new_group_btn.on_click(move |_| {
		new_conv_dlg_for_group.end_modal(0);
		let _ = tx_prepare.send(UiCommand::PrepareNewGroup);
	});

	let tx_create = ui_tx.clone();
	let group_dialog = parts.new_group_dialog.dialog;
	let name_ctrl = parts.new_group_dialog.name_field;
	let extra_ctrl = parts.new_group_dialog.extra_number_field;
	let contacts_listbox_ok = parts.new_group_dialog.contacts_listbox;
	let state_ok = state.clone();
	let ok_btn = parts.new_group_dialog.ok_btn;

	parts.new_group_dialog.ok_btn.on_click(move |_| {
		let name = name_ctrl.get_value().trim().to_string();
		if name.is_empty() {
			unsafe {
				let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_STYLE(0x00000030));
			}
			MessageDialog::builder(&group_dialog, "Group name is required.", "Error").with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError).build().show_modal();
			name_ctrl.select_all();
			name_ctrl.set_focus();
			return;
		}
		let mut s = state_ok.borrow_mut();
		let gs = &mut s.group_state;
		let count = contacts_listbox_ok.get_count();
		for idx in 0..count {
			if idx < gs.current_jids.len() as u32 {
				let jid = gs.current_jids[idx as usize].clone();
				if contacts_listbox_ok.is_checked(idx) {
					gs.checked_jids.insert(jid);
				} else {
					gs.checked_jids.remove(&jid);
				}
			}
		}
		let mut participants = Vec::new();
		for jid in &gs.checked_jids {
			participants.push(jid.clone());
		}
		let extra_raw = extra_ctrl.get_value();
		for part in extra_raw.split(|c| c == ',' || c == '\n') {
			let digits: String = part.chars().filter(|c| c.is_digit(10)).collect();
			if digits.len() >= 7 {
				participants.push(digits);
			}
		}
		let mut seen = std::collections::HashSet::new();
		let mut unique_participants = Vec::new();
		for p in participants {
			if seen.insert(p.clone()) {
				unique_participants.push(p);
			}
		}
		if unique_participants.is_empty() {
			unsafe {
				let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_STYLE(0x00000030));
			}
			MessageDialog::builder(&group_dialog, "At least one participant must be selected.", "Error").with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError).build().show_modal();
			contacts_listbox_ok.set_focus();
			return;
		}
		ok_btn.enable(false);
		group_dialog.end_modal(wxdragon::id::ID_OK);
		let _ = tx_create.send(UiCommand::CreateGroup {
			name,
			participants: unique_participants,
		});
	});

	let new_group_cancel = parts.new_group_dialog.dialog;
	parts.new_group_dialog.cancel_btn.on_click(move |_| {
		new_group_cancel.end_modal(0);
	});

	let search_field = parts.new_group_dialog.search_field;
	let contacts_listbox = parts.new_group_dialog.contacts_listbox;
	let state_search = state.clone();
	search_field.on_text_changed(move |_| {
		let query = search_field.get_value().to_lowercase().trim().to_string();
		let mut s = state_search.borrow_mut();
		let gs = &mut s.group_state;
		let count = contacts_listbox.get_count();
		for idx in 0..count {
			if idx < gs.current_jids.len() as u32 {
				let jid = gs.current_jids[idx as usize].clone();
				if contacts_listbox.is_checked(idx) {
					gs.checked_jids.insert(jid);
				} else {
					gs.checked_jids.remove(&jid);
				}
			}
		}
		let mut filtered_labels = Vec::new();
		let mut filtered_jids = Vec::new();
		for (lbl, jid) in gs.all_contact_labels.iter().zip(gs.all_contact_jids.iter()) {
			if query.is_empty() || lbl.to_lowercase().contains(&query) {
				filtered_labels.push(lbl.clone());
				filtered_jids.push(jid.clone());
			}
		}
		contacts_listbox.clear();
		for lbl in &filtered_labels {
			contacts_listbox.append(lbl);
		}
		gs.current_jids = filtered_jids;
		for (idx, jid) in gs.current_jids.iter().enumerate() {
			if gs.checked_jids.contains(jid) {
				contacts_listbox.check(idx as u32, true);
			}
		}
	});

	let contacts_listbox_toggle = parts.new_group_dialog.contacts_listbox;
	let state_toggle = state.clone();
	contacts_listbox_toggle.on_toggled(move |event| {
		if let Some(idx) = event.get_selection() {
			let mut s = state_toggle.borrow_mut();
			let gs = &mut s.group_state;
			if idx < gs.current_jids.len() as u32 {
				let jid = gs.current_jids[idx as usize].clone();
				let checked = contacts_listbox_toggle.is_checked(idx);
				if checked {
					gs.checked_jids.insert(jid);
				} else {
					gs.checked_jids.remove(&jid);
				}
				let label = contacts_listbox_toggle.get_string(idx as usize).unwrap_or_default();
				let state_str = if checked { "selected" } else { "not selected" };
				println!("{}: {}", label, state_str);
			}
		}
	});

	let tx_nav = ui_tx.clone();
	parts.nav_list.on_selection_changed(move |event| {
		if let Some(idx) = event.get_selection() {
			let _ = tx_nav.send(UiCommand::NavSelected(idx as usize));
		}
	});



	let tx_send_field = ui_tx.clone();
	let conv_msg_field_esc = parts.conversations_parts.message_field;
	conv_msg_field_esc.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key_event) = event {
			let code = key_event.get_key_code();
			if code == Some(13) || code == Some(370) { // Enter key
				if key_event.shift_down() {
					event.skip(true); // Shift+Enter -> insert newline in multiline text box
				} else {
					let text = conv_msg_field_esc.get_value();
					if !text.is_empty() {
						let _ = tx_send_field.send(UiCommand::SendMessage(text));
					}
					event.skip(false); // Suppress default Enter newline insertion
				}
			} else if code == Some(27) { // Escape key
				let _ = tx_send_field.send(UiCommand::CloseConversation);
				event.skip(false);
			} else {
				event.skip(true);
			}
		} else {
			event.skip(true);
		}
	});

	let tx_filter = ui_tx.clone();
	let conv_search = parts.conversations_parts.search_field;
	conv_search.on_text_changed(move |_| {
		let query = conv_search.get_value();
		let _ = tx_filter.send(UiCommand::FilterConversations(query));
	});

	let tx_open = ui_tx.clone();
	parts.conversations_parts.conversations_list.on_item_double_clicked(move |event| {
		if let Some(idx) = event.get_selection() {
			let _ = tx_open.send(UiCommand::OpenConversation(idx as usize));
		}
	});

	let tx_open_enter = ui_tx.clone();
	let conv_list_enter = parts.conversations_parts.conversations_list.clone();
	parts.conversations_parts.conversations_list.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key_event) = event {
			if key_event.get_key_code() == Some(13) {
				if let Some(idx) = conv_list_enter.get_selection() {
					let _ = tx_open_enter.send(UiCommand::OpenConversation(idx as usize));
				}
				event.skip(false);
			} else {
				event.skip(true);
			}
		} else {
			event.skip(true);
		}
	});

	let tx_open_enter_up = ui_tx.clone();
	let conv_list_enter_up = parts.conversations_parts.conversations_list;
	parts.conversations_parts.conversations_list.on_key_up(move |event| {
		if let WindowEventData::Keyboard(ref key_event) = event {
			if key_event.get_key_code() == Some(13) || key_event.get_key_code() == Some(370) {
				if let Some(idx) = conv_list_enter_up.get_selection() {
					let _ = tx_open_enter_up.send(UiCommand::OpenConversation(idx as usize));
				}
				event.skip(false);
			} else {
				event.skip(true);
			}
		} else {
			event.skip(true);
		}
	});

	let tx_play_media = ui_tx.clone();
	let msgs_list_play = parts.conversations_parts.messages_list.clone();
	let state_play = state.clone();
	let play_voice_btn_space = parts.conversations_parts.play_voice_btn.clone();
	let tx_close_list = ui_tx.clone();
	let parent_frame = parts.frame.clone();
	let msgs_announcer = parts.conversations_parts.messages_announcer.clone();
	let msg_input_field = parts.conversations_parts.message_field.clone();
	let search_panel_key = parts.conversations_parts.search_panel;
	let _conv_panel_key = parts.conversations_parts.conversation_panel;
	let last_left_arrow = std::rc::Rc::new(std::cell::Cell::new(std::time::Instant::now() - std::time::Duration::from_secs(10)));
	parts.conversations_parts.messages_list.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key_event) = event {
			let code = key_event.get_key_code();
			if code == Some(314) || code == Some(37) { // Left Arrow
				let now = std::time::Instant::now();
				let prev = last_left_arrow.get();
				if now.duration_since(prev) < std::time::Duration::from_millis(600) {
					last_left_arrow.set(now - std::time::Duration::from_secs(10));
					// If search is active, double-left-arrow behaves like normal chats:
					// if the message is a reply, jump to the quoted message; otherwise just close search
					if search_panel_key.is_shown() {
						if let Some(idx) = msgs_list_play.get_selection() {
							let s = state_play.borrow();
							let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item { whantv::DisplayItem::Single(m) => Some(m.clone()), whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(), }); if let Some(msg) = msg_opt {
								drop(s);
								if !msg.is_date_divider && (!msg.quoted_id.is_empty() || !msg.quoted_text.is_empty()) {
									// Jump to the quoted message — pass both id and text so handler can try both
									let _ = tx_close_list.send(UiCommand::JumpToSearchResult {
										message_id: msg.quoted_id,
										message_text: msg.quoted_text,
									});
								} else {
									let _ = tx_close_list.send(UiCommand::CloseSearch);
								}
							} else {
								drop(s);
								let _ = tx_close_list.send(UiCommand::CloseSearch);
							}
						} else {
							let _ = tx_close_list.send(UiCommand::CloseSearch);
						}
						event.skip(false);
					} else if let Some(idx) = msgs_list_play.get_selection() {
						let s = state_play.borrow();
						let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item { whantv::DisplayItem::Single(m) => Some(m.clone()), whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(), });
						if let Some(msg) = msg_opt {
							if !msg.quoted_id.is_empty() || !msg.quoted_text.is_empty() {
								let mut target = if !msg.quoted_id.is_empty() {
									s.display_items.iter().position(|item| match item {
										whantv::DisplayItem::Single(m) => m.id == msg.quoted_id,
										whantv::DisplayItem::Album { messages, .. } => messages.iter().any(|m| m.id == msg.quoted_id),
									})
								} else {
									None
								};

								if target.is_none() && !msg.quoted_text.is_empty() {
									let is_generic = msg.quoted_text == "[Voice Message]"
										|| msg.quoted_text == "[Audio]"
										|| msg.quoted_text == "Voice message"
										|| msg.quoted_text == "Audio"
										|| msg.quoted_text == "[Media]";
									if !is_generic && (idx as usize) < s.display_items.len() {
										target = s.display_items[..idx as usize].iter().rposition(|item| {
											match item {
												whantv::DisplayItem::Single(m) => m.text == msg.quoted_text || (!m.text.is_empty() && m.text.contains(&msg.quoted_text)),
												whantv::DisplayItem::Album { messages, .. } => messages.iter().any(|m| m.text == msg.quoted_text || (!m.text.is_empty() && m.text.contains(&msg.quoted_text))),
											}
										});
									}
								}

								if let Some(target_idx) = target {
									msgs_list_play.set_selection(target_idx as u32, true);
									unsafe {
										let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(windows::Win32::UI::WindowsAndMessaging::MB_OK);
									}
								} else {
									unsafe {
										let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(windows::Win32::UI::WindowsAndMessaging::MB_ICONASTERISK);
									}
								}
							}
						}
					}
					event.skip(false);
				} else {
					last_left_arrow.set(now);
					event.skip(false); // Stay silent on single press
				}
			} else if key_event.get_key_code() == Some(27) { // Escape
				let _ = tx_close_list.send(UiCommand::CloseConversation);
				event.skip(false);
			} else if key_event.get_key_code() == Some(313) || key_event.get_key_code() == Some(384) { // WXK_HOME (313) -> Oldest message at top
				let count = msgs_list_play.get_count();
				if count > 0 {
					msgs_list_play.set_selection(0, true);
				}
				event.skip(false);
			} else if key_event.get_key_code() == Some(312) || key_event.get_key_code() == Some(385) { // WXK_END (312) -> Newest message at bottom
				let count = msgs_list_play.get_count();
				if count > 0 {
					msgs_list_play.set_selection((count - 1) as u32, true);
				}
				event.skip(false);
			} else if key_event.get_key_code() == Some(13) { // Enter key
				let is_search_active = search_panel_key.is_shown();
				if is_search_active {
					if let Some(idx) = msgs_list_play.get_selection() {
						let s = state_play.borrow();
						let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item { whantv::DisplayItem::Single(m) => Some(m.clone()), whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(), }); if let Some(msg) = msg_opt {
							drop(s);
							if !msg.is_date_divider {
								let _ = tx_close_list.send(UiCommand::JumpToSearchResult { message_id: msg.id, message_text: String::new() });
							} else {
								let _ = tx_close_list.send(UiCommand::CloseSearch);
							}
						} else {
							drop(s);
							let _ = tx_close_list.send(UiCommand::CloseSearch);
						}
					} else {
						let _ = tx_close_list.send(UiCommand::CloseSearch);
					}
					event.skip(false);
				} else {
					if let Some(idx) = msgs_list_play.get_selection() {
						let s = state_play.borrow();
						let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item { whantv::DisplayItem::Single(m) => Some(m.clone()), whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(), });
						if let Some(msg) = msg_opt {
							let mut urls = Vec::new();
							let text = &msg.text;
							let mut start_idx = 0;
							while let Some(idx) = text[start_idx..].find("http") {
								let actual_idx = start_idx + idx;
								if text[actual_idx..].starts_with("http://") || text[actual_idx..].starts_with("https://") {
									let space_idx = text[actual_idx..].find(|c: char| c.is_whitespace()).unwrap_or(text.len() - actual_idx);
									urls.push(text[actual_idx..actual_idx + space_idx].to_string());
									start_idx = actual_idx + space_idx;
								} else {
									start_idx = actual_idx + 4;
								}
							}
							
							if urls.len() == 1 {
								wxdragon::utils::launch_default_browser(&urls[0], wxdragon::utils::BrowserLaunchFlags::Default);
							} else if urls.len() > 1 {
								let url_strs: Vec<&str> = urls.iter().map(|s| s.as_str()).collect();
								let dialog = wxdragon::prelude::SingleChoiceDialog::builder(
									&msgs_list_play,
									"Select a link to open:",
									"Open Link",
									&url_strs
								).build();
								if dialog.show_modal() == wxdragon::id::ID_OK {
									if let Some(selection) = dialog.get_string_selection() {
										wxdragon::utils::launch_default_browser(&selection, wxdragon::utils::BrowserLaunchFlags::Default);
									}
								}
							}
						}
					}
					event.skip(false);
				}
			} else if key_event.get_key_code() == Some(32) || key_event.get_key_code() == Some(' ' as i32) { // Space
				use std::io::Write;
				if let Some(idx) = msgs_list_play.get_selection() {
					let mut s = state_play.borrow_mut();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item { whantv::DisplayItem::Single(m) => Some(m.clone()), whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(), }); if let Some(msg) = msg_opt {
						let is_voice = msg.is_audio || msg.text.starts_with("[Voice Message]") || msg.text.starts_with("[Audio]");
						if !is_voice {
							event.skip(true);
							return;
						}
						let is_currently_playing = s.playing_msg_id.as_deref() == Some(&msg.id);
						if is_currently_playing {
							if s.is_audio_paused {
								// Resume
								if let Some(ref sink) = s.audio_sink {
									sink.play();
								}
								if let Some(ref mut stdin) = s.audio_stdin {
									let _ = writeln!(stdin, "RESUME");
									let _ = stdin.flush();
								}
								s.is_audio_paused = false;
								s.audio_start_instant = Some(std::time::Instant::now());
								play_voice_btn_space.set_label("Pause voice message");
							} else {
								// Pause — save elapsed
								if let Some(inst) = s.audio_start_instant.take() {
									s.audio_elapsed_secs += inst.elapsed().as_secs_f32();
								}
								if let Some(ref sink) = s.audio_sink {
									sink.pause();
								}
								if let Some(ref mut stdin) = s.audio_stdin {
									let _ = writeln!(stdin, "PAUSE");
									let _ = stdin.flush();
								}
								s.is_audio_paused = true;
								play_voice_btn_space.set_label("Play voice message");
							}
						} else {
							if let Some(ref sink) = s.audio_sink {
								sink.stop();
							}
							s.audio_sink = None;
							s.audio_elapsed_secs = 0.0;
							s.audio_start_instant = None;
							s.is_audio_paused = false;
							s.playing_msg_id = Some(msg.id.clone());
							play_voice_btn_space.set_label("Loading voice message...");
							let _ = tx_play_media.send(UiCommand::PlayVoiceMessage(msg.id));
						}
					}
				}
				event.skip(false);
			} else if code == Some('r' as i32) || code == Some('R' as i32) || code == Some(82) { // 'r' / 'R' -> Reply
				if let Some(idx) = msgs_list_play.get_selection() {
					let mut s = state_play.borrow_mut();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item { whantv::DisplayItem::Single(m) => Some(m.clone()), whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(), }); if let Some(msg) = msg_opt {
						if !msg.is_date_divider {
							let sender = if msg.from_me {
								"You".to_string()
							} else if !msg.sender_name.is_empty() {
								msg.sender_name.clone()
							} else {
								msg.sender_jid.clone()
							};
							let preview = if msg.is_audio || msg.text.starts_with("[Voice Message]") {
								"Voice message".to_string()
							} else if msg.text.is_empty() {
								"Media".to_string()
							} else {
								msg.text.clone()
							};

							s.pending_reply_id = Some(msg.id.clone());
							s.pending_reply_sender = Some(if msg.from_me { "me".to_string() } else { msg.sender_jid.clone() });
							s.pending_reply_preview = Some(format!("Replying to {}: {}", sender, preview));

							let status_msg = format!("Replying to {}: {}. Type your reply and press Enter.", sender, preview);
							live_region::announce(msgs_announcer.clone(), &status_msg);
							msg_input_field.set_focus();
							unsafe {
								let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(windows::Win32::UI::WindowsAndMessaging::MB_OK);
							}
						}
					}
				}
				event.skip(false);
			} else if code == Some('e' as i32) || code == Some('E' as i32) || code == Some(69) { // 'e' / 'E' -> Edit
				if let Some(idx) = msgs_list_play.get_selection() {
					let s = state_play.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item { whantv::DisplayItem::Single(m) => Some(m.clone()), whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(), }); if let Some(msg) = msg_opt {
						if !msg.is_date_divider {
							let can_edit = msg.from_me && !msg.is_deleted && !msg.is_audio && !msg.text.starts_with("[Voice Message]") && !msg.text.starts_with("[Audio]");
							if !can_edit {
								unsafe {
									let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(windows::Win32::UI::WindowsAndMessaging::MB_ICONHAND);
								}
								live_region::announce(msgs_announcer.clone(), "Message can't be edited.");
							} else {
								let dlg = TextEntryDialog::builder(&parent_frame, "Enter updated message text:", "Edit Message")
									.with_default_value(&msg.text)
									.build();
								if dlg.show_modal() == wxdragon::id::ID_OK {
									if let Some(val) = dlg.get_value() {
										let new_text = val.trim().to_string();
										if !new_text.is_empty() && new_text != msg.text {
											let _ = tx_play_media.send(UiCommand::EditMessage { message_id: msg.id.clone(), new_text });
										}
									}
								}
							}
						}
					}
				}
				event.skip(false);
			} else if code == Some(127) || code == Some(390) { // Delete key -> Delete
				if let Some(idx) = msgs_list_play.get_selection() {
					let s = state_play.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item { whantv::DisplayItem::Single(m) => Some(m.clone()), whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(), }); if let Some(msg) = msg_opt {
						if !msg.is_date_divider {
							let prompt = if msg.from_me {
								"Are you sure you want to delete this message for everyone?"
							} else {
								"Are you sure you want to delete this message from your device?"
							};
							let dlg = MessageDialog::builder(&parent_frame, prompt, "Confirm Delete")
								.with_style(MessageDialogStyle::OK | MessageDialogStyle::Cancel | MessageDialogStyle::IconQuestion)
								.build();
							if dlg.show_modal() == wxdragon::id::ID_OK {
								let sender_jid = if msg.from_me { "me".to_string() } else { msg.sender_jid.clone() };
								let _ = tx_play_media.send(UiCommand::DeleteMessage {
									message_id: msg.id.clone(),
									sender_jid,
									for_everyone: msg.from_me,
								});
							}
						}
					}
				}
			} else if !key_event.meta_down() && (code == Some(349) || code == Some(395) || code == Some(93) || (key_event.shift_down() && code == Some(358))) { // Context Menu key (WXK_WINDOWS_MENU=395, VK_APPS=93) or Shift+F10
				if let Some(idx) = msgs_list_play.get_selection() {
					let reply_id = 3001;
					let edit_id = 3002;
					let delete_id = 3003;
					let info_id = 3004;
					let copy_all_id = 3005;

					let s = state_play.borrow();
					let display_item = s.display_items.get(idx as usize).cloned();
					drop(s);

					let is_from_me = display_item.as_ref().map(|item| match item { whantv::DisplayItem::Single(m) => m.from_me && !m.is_date_divider, whantv::DisplayItem::Album { messages, .. } => messages.first().map(|m| m.from_me).unwrap_or(false) }).unwrap_or(false);
					let is_photo_album = display_item.as_ref().map(|item| match item {
						whantv::DisplayItem::Album { messages, .. } => messages.iter().all(|m| m.media_type == "image"),
						_ => false,
					}).unwrap_or(false);

					let mut menu_builder = Menu::builder();
					menu_builder = menu_builder.append_item(reply_id, "&Reply", "Reply to message")
						.append_item(edit_id, "&Edit", "Edit message")
						.append_item(delete_id, "&Delete", "Delete message");
					if is_from_me {
						menu_builder = menu_builder.append_item(info_id, "Message &Info", "Show message delivery info");
					}
					if is_photo_album {
						menu_builder = menu_builder.append_item(copy_all_id, "Copy all p&hotos", "Copy all photos to clipboard");
					}
					let mut popup_menu = menu_builder.build();

					let state_clone = state_play.clone();
					let parent_frame_clone = parent_frame.clone();
					let msgs_announcer_clone = msgs_announcer.clone();
					let msg_input_field_clone = msg_input_field.clone();
					let tx_play_media_clone = tx_play_media.clone();

					popup_menu.on_selected(move |menu_evt| {
						let menu_id = menu_evt.get_id();
						if menu_id == reply_id {
							let mut s = state_clone.borrow_mut();
							let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item { whantv::DisplayItem::Single(m) => Some(m.clone()), whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(), }); if let Some(msg) = msg_opt {
								if !msg.is_date_divider {
									let sender = if msg.from_me {
										"You".to_string()
									} else if !msg.sender_name.is_empty() {
										msg.sender_name.clone()
									} else {
										msg.sender_jid.clone()
									};
									let preview = if msg.is_audio || msg.text.starts_with("[Voice Message]") {
										"Voice message".to_string()
									} else if msg.text.is_empty() {
										"Media".to_string()
									} else {
										msg.text.clone()
									};
									s.pending_reply_id = Some(msg.id.clone());
									s.pending_reply_sender = Some(if msg.from_me { "me".to_string() } else { msg.sender_jid.clone() });
									s.pending_reply_preview = Some(format!("Replying to {}: {}", sender, preview));

									let status_msg = format!("Replying to {}: {}. Type your reply and press Enter.", sender, preview);
									live_region::announce(msgs_announcer_clone.clone(), &status_msg);
									msg_input_field_clone.set_focus();
									unsafe {
										let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(windows::Win32::UI::WindowsAndMessaging::MB_OK);
									}
								}
							}
						} else if menu_id == edit_id {
							let s = state_clone.borrow();
							let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item { whantv::DisplayItem::Single(m) => Some(m.clone()), whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(), }); if let Some(msg) = msg_opt {
								if !msg.is_date_divider {
									let can_edit = msg.from_me && !msg.is_deleted && !msg.is_audio && !msg.text.starts_with("[Voice Message]") && !msg.text.starts_with("[Audio]");
									if !can_edit {
										unsafe {
											let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(windows::Win32::UI::WindowsAndMessaging::MB_ICONHAND);
										}
										live_region::announce(msgs_announcer_clone.clone(), "Message can't be edited.");
									} else {
										let dlg = TextEntryDialog::builder(&parent_frame_clone, "Enter updated message text:", "Edit Message")
											.with_default_value(&msg.text)
											.build();
										if dlg.show_modal() == wxdragon::id::ID_OK {
											if let Some(val) = dlg.get_value() {
												let new_text = val.trim().to_string();
												if !new_text.is_empty() && new_text != msg.text {
													let _ = tx_play_media_clone.send(UiCommand::EditMessage { message_id: msg.id.clone(), new_text });
												}
											}
										}
									}
								}
							}
						} else if menu_id == delete_id {
							let s = state_clone.borrow();
							let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item { whantv::DisplayItem::Single(m) => Some(m.clone()), whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(), }); if let Some(msg) = msg_opt {
								if !msg.is_date_divider {
									let prompt = if msg.from_me {
										"Are you sure you want to delete this message for everyone?"
									} else {
										"Are you sure you want to delete this message from your device?"
									};
									let dlg = MessageDialog::builder(&parent_frame_clone, prompt, "Confirm Delete")
										.with_style(MessageDialogStyle::OK | MessageDialogStyle::Cancel | MessageDialogStyle::IconQuestion)
										.build();
									if dlg.show_modal() == wxdragon::id::ID_OK {
										let sender_jid = if msg.from_me { "me".to_string() } else { msg.sender_jid.clone() };
										let _ = tx_play_media_clone.send(UiCommand::DeleteMessage {
											message_id: msg.id.clone(),
											sender_jid,
											for_everyone: msg.from_me,
										});
									}
								}
							}
						} else if menu_id == info_id {
							let s = state_clone.borrow();
							let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item { whantv::DisplayItem::Single(m) => Some(m.clone()), whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(), }); if let Some(msg) = msg_opt {
								if msg.from_me && !msg.is_date_divider {
									let _ = tx_play_media_clone.send(UiCommand::GetMessageInfo { message_id: msg.id.clone() });
								}
							}
						} else if menu_id == copy_all_id {
							let s = state_clone.borrow();
							if let Some(whantv::DisplayItem::Album { messages, .. }) = s.display_items.get(idx as usize) {
								let msg_ids: Vec<String> = messages.iter().map(|m| m.id.clone()).collect();
								if !msg_ids.is_empty() {
									let _ = tx_play_media_clone.send(UiCommand::CopyAlbumToClipboard { message_ids: msg_ids });
								}
							}
						}
					});

					msgs_list_play.popup_menu(&mut popup_menu, None);
					popup_menu.destroy_menu();
				}
				event.skip(false);
			} else {
				event.skip(true);
			}
		} else {
			event.skip(true);
		}
	});

	// on_key_up catches Enter on ListBox — on_key_down misses it because the OS consumes it
	let tx_jump_enter = ui_tx.clone();
	let msgs_list_enter = parts.conversations_parts.messages_list.clone();
	let state_enter = state.clone();
	let search_panel_enter = parts.conversations_parts.search_panel;
	parts.conversations_parts.messages_list.on_key_up(move |event| {
		if let WindowEventData::Keyboard(ref key_event) = event {
			let code = key_event.get_key_code();
			if code == Some(13) || code == Some(370) { // Enter or numpad Enter
				if search_panel_enter.is_shown() {
					if let Some(idx) = msgs_list_enter.get_selection() {
						let s = state_enter.borrow();
						let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item { whantv::DisplayItem::Single(m) => Some(m.clone()), whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(), }); if let Some(msg) = msg_opt {
							drop(s);
							if !msg.is_date_divider {
								let _ = tx_jump_enter.send(UiCommand::JumpToSearchResult { message_id: msg.id, message_text: String::new() });
							} else {
								let _ = tx_jump_enter.send(UiCommand::CloseSearch);
							}
						} else {
							drop(s);
							let _ = tx_jump_enter.send(UiCommand::CloseSearch);
						}
					} else {
						let _ = tx_jump_enter.send(UiCommand::CloseSearch);
					}
					event.skip(false);
				} else {
					event.skip(true);
				}
			}
		}
	});

	let msgs_list_ctx = parts.conversations_parts.messages_list.clone();
	let state_ctx = state.clone();
	let parent_frame_ctx = parts.frame.clone();
	let msgs_announcer_ctx = parts.conversations_parts.messages_announcer.clone();
	let msg_input_field_ctx = parts.conversations_parts.message_field.clone();
	let tx_play_media_ctx = ui_tx.clone();

	parts.conversations_parts.messages_list.bind_internal(EventType::CONTEXT_MENU, move |_event| {
		if let Some(idx) = msgs_list_ctx.get_selection() {
			let reply_id = 3001;
			let edit_id = 3002;
			let delete_id = 3003;
			let info_id = 3004;
			let copy_all_id = 3005;

			let s = state_ctx.borrow();
			let display_item = s.display_items.get(idx as usize).cloned();
			drop(s);

			let is_from_me = display_item.as_ref().map(|item| match item { whantv::DisplayItem::Single(m) => m.from_me && !m.is_date_divider, whantv::DisplayItem::Album { messages, .. } => messages.first().map(|m| m.from_me).unwrap_or(false) }).unwrap_or(false);
			let is_photo_album = display_item.as_ref().map(|item| match item {
				whantv::DisplayItem::Album { messages, .. } => messages.iter().all(|m| m.media_type == "image"),
				_ => false,
			}).unwrap_or(false);

			let mut menu_builder = Menu::builder();
			menu_builder = menu_builder.append_item(reply_id, "&Reply", "Reply to message")
				.append_item(edit_id, "&Edit", "Edit message")
				.append_item(delete_id, "&Delete", "Delete message");
			if is_from_me {
				menu_builder = menu_builder.append_item(info_id, "Message &Info", "View message delivery info");
			}
			if is_photo_album {
				menu_builder = menu_builder.append_item(copy_all_id, "Copy all p&hotos", "Copy all photos to clipboard");
			}
			let mut popup_menu = menu_builder.build();

			let state_clone = state_ctx.clone();
			let parent_frame_clone = parent_frame_ctx.clone();
			let msgs_announcer_clone = msgs_announcer_ctx.clone();
			let msg_input_field_clone = msg_input_field_ctx.clone();
			let tx_play_media_clone = tx_play_media_ctx.clone();

			popup_menu.on_selected(move |menu_evt| {
				let id = menu_evt.get_id();
				if id == reply_id {
					let mut s = state_clone.borrow_mut();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item { whantv::DisplayItem::Single(m) => Some(m.clone()), whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(), }); if let Some(msg) = msg_opt {
						if !msg.is_date_divider {
							let sender = if msg.from_me {
								"You".to_string()
							} else if !msg.sender_name.is_empty() {
								msg.sender_name.clone()
							} else {
								msg.sender_jid.clone()
							};
							let preview = if msg.is_audio || msg.text.starts_with("[Voice Message]") {
								"Voice message".to_string()
							} else if msg.text.is_empty() {
								"Media".to_string()
							} else {
								msg.text.clone()
							};
							s.pending_reply_id = Some(msg.id.clone());
							s.pending_reply_sender = Some(if msg.from_me { "me".to_string() } else { msg.sender_jid.clone() });
							s.pending_reply_preview = Some(format!("Replying to {}: {}", sender, preview));

							let status_msg = format!("Replying to {}: {}. Type your reply and press Enter.", sender, preview);
							live_region::announce(msgs_announcer_clone.clone(), &status_msg);
							msg_input_field_clone.set_focus();
							unsafe {
								let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(windows::Win32::UI::WindowsAndMessaging::MB_OK);
							}
						}
					}
				} else if id == edit_id {
					let s = state_clone.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item { whantv::DisplayItem::Single(m) => Some(m.clone()), whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(), }); if let Some(msg) = msg_opt {
						if !msg.is_date_divider {
							let can_edit = msg.from_me && !msg.is_deleted && !msg.is_audio && !msg.text.starts_with("[Voice Message]") && !msg.text.starts_with("[Audio]");
							if !can_edit {
								unsafe {
									let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(windows::Win32::UI::WindowsAndMessaging::MB_ICONHAND);
								}
								live_region::announce(msgs_announcer_clone.clone(), "Message can't be edited.");
							} else {
								let dlg = TextEntryDialog::builder(&parent_frame_clone, "Enter updated message text:", "Edit Message")
									.with_default_value(&msg.text)
									.build();
								if dlg.show_modal() == wxdragon::id::ID_OK {
									if let Some(val) = dlg.get_value() {
										let new_text = val.trim().to_string();
										if !new_text.is_empty() && new_text != msg.text {
											let _ = tx_play_media_clone.send(UiCommand::EditMessage { message_id: msg.id.clone(), new_text });
										}
									}
								}
							}
						}
					}
				} else if id == delete_id {
					let s = state_clone.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item { whantv::DisplayItem::Single(m) => Some(m.clone()), whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(), }); if let Some(msg) = msg_opt {
						if !msg.is_date_divider {
							let prompt = if msg.from_me {
								"Are you sure you want to delete this message for everyone?"
							} else {
								"Are you sure you want to delete this message from your device?"
							};
							let dlg = MessageDialog::builder(&parent_frame_clone, prompt, "Confirm Delete")
								.with_style(MessageDialogStyle::OK | MessageDialogStyle::Cancel | MessageDialogStyle::IconQuestion)
								.build();
							if dlg.show_modal() == wxdragon::id::ID_OK {
								let sender_jid = if msg.from_me { "me".to_string() } else { msg.sender_jid.clone() };
								let _ = tx_play_media_clone.send(UiCommand::DeleteMessage {
									message_id: msg.id.clone(),
									sender_jid,
									for_everyone: msg.from_me,
								});
							}
						}
					}
				} else if id == info_id {
					let s = state_clone.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item { whantv::DisplayItem::Single(m) => Some(m.clone()), whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(), }); if let Some(msg) = msg_opt {
						if msg.from_me && !msg.is_date_divider {
							let _ = tx_play_media_clone.send(UiCommand::GetMessageInfo { message_id: msg.id.clone() });
						}
					}
				} else if id == copy_all_id {
					let s = state_clone.borrow();
					if let Some(whantv::DisplayItem::Album { messages, .. }) = s.display_items.get(idx as usize) {
						let msg_ids: Vec<String> = messages.iter().map(|m| m.id.clone()).collect();
						if !msg_ids.is_empty() {
							let _ = tx_play_media_clone.send(UiCommand::CopyAlbumToClipboard { message_ids: msg_ids });
						}
					}
				}
			});

			msgs_list_ctx.popup_menu(&mut popup_menu, None);
			popup_menu.destroy_menu();
		}
	});

	let tx_play_btn = ui_tx.clone();
	let msgs_list_btn = parts.conversations_parts.messages_list.clone();
	let state_play_btn = state.clone();
	let play_voice_btn_ref = parts.conversations_parts.play_voice_btn.clone();
	parts.conversations_parts.play_voice_btn.on_click(move |_| {
		use std::io::Write;
		if let Some(idx) = msgs_list_btn.get_selection() {
			let mut s = state_play_btn.borrow_mut();
			let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item { whantv::DisplayItem::Single(m) => Some(m.clone()), whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(), }); if let Some(msg) = msg_opt {
				let is_currently_playing = s.playing_msg_id.as_deref() == Some(&msg.id);
				if is_currently_playing {
					if s.is_audio_paused {
						// Resume
						if let Some(ref sink) = s.audio_sink {
							sink.play();
						}
						if let Some(ref mut stdin) = s.audio_stdin {
							let _ = writeln!(stdin, "RESUME");
							let _ = stdin.flush();
						}
						s.is_audio_paused = false;
						s.audio_start_instant = Some(std::time::Instant::now());
						play_voice_btn_ref.set_label("Pause voice message");
						return;
					} else {
						// Pause — save elapsed time
						if let Some(inst) = s.audio_start_instant.take() {
							s.audio_elapsed_secs += inst.elapsed().as_secs_f32();
						}
						if let Some(ref sink) = s.audio_sink {
							sink.pause();
						}
						if let Some(ref mut stdin) = s.audio_stdin {
							let _ = writeln!(stdin, "PAUSE");
							let _ = stdin.flush();
						}
						s.is_audio_paused = true;
						play_voice_btn_ref.set_label("Play voice message");
						return;
					}
				}

				if let Some(ref sink) = s.audio_sink {
					sink.stop();
				}
				s.audio_sink = None;
				s.audio_elapsed_secs = 0.0;
				s.audio_start_instant = None;
				s.is_audio_paused = false;
				s.playing_msg_id = Some(msg.id.clone());
				play_voice_btn_ref.set_label("Loading voice message...");
				let _ = tx_play_btn.send(UiCommand::PlayVoiceMessage(msg.id));
			}
		}
	});

	let state_sel = state.clone();
	let play_voice_btn_sel = parts.conversations_parts.play_voice_btn.clone();
	let audio_slider_sel = parts.conversations_parts.audio_slider.clone();
	let audio_progress_label_sel = parts.conversations_parts.audio_progress_label.clone();
	let conv_parts_sel = parts.conversations_parts.clone();
	let last_layout_time = std::cell::Cell::new(std::time::Instant::now());
	parts.conversations_parts.messages_list.on_selection_changed(move |event| {
		if let Some(idx) = event.get_selection() {
			let s = state_sel.borrow();
			conv_parts_sel.action_panel.freeze();
			let mut need_layout = crate::update_action_buttons(&s, &conv_parts_sel);

			let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item { whantv::DisplayItem::Single(m) => Some(m.clone()), whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(), });
			if let Some(msg) = msg_opt {
				let is_voice = match s.display_items.get(idx as usize) {
					Some(whantv::DisplayItem::Single(m)) => m.is_audio || m.text.starts_with("[Voice Message]") || m.text.starts_with("[Audio]"),
					_ => false,
				};
				if play_voice_btn_sel.is_shown() != is_voice {
					play_voice_btn_sel.show(is_voice);
					need_layout = true;
				}
				if is_voice {
					let is_playing = s.playing_msg_id.as_deref() == Some(&msg.id);
					if is_playing {
						if let Some(ref sink) = s.audio_sink {
							if !sink.is_paused() && !sink.empty() {
								play_voice_btn_sel.set_label("Pause voice message");
							} else {
								play_voice_btn_sel.set_label("Play voice message");
							}
						} else if s.audio_process.is_some() {
							play_voice_btn_sel.set_label("Pause voice message");
						} else {
							play_voice_btn_sel.set_label("Play voice message");
						}
						if !audio_slider_sel.is_shown() {
							audio_slider_sel.show(true);
							audio_progress_label_sel.show(true);
							need_layout = true;
						}
					} else {
						play_voice_btn_sel.set_label("Play voice message");
						if audio_slider_sel.is_shown() {
							audio_slider_sel.show(false);
							audio_progress_label_sel.show(false);
							need_layout = true;
						}
					}
				} else if audio_slider_sel.is_shown() {
					audio_slider_sel.show(false);
					audio_progress_label_sel.show(false);
					need_layout = true;
				}
			} else {
				if play_voice_btn_sel.is_shown() { play_voice_btn_sel.show(false); need_layout = true; }
				if audio_slider_sel.is_shown() { audio_slider_sel.show(false); audio_progress_label_sel.show(false); need_layout = true; }
			}

			// Show/hide action_panel based on whether any action buttons are visible
			let any_action_visible = play_voice_btn_sel.is_shown()
				|| conv_parts_sel.action_open_btn.is_shown()
				|| conv_parts_sel.action_save_as_btn.is_shown()
				|| conv_parts_sel.action_download_btn.is_shown()
				|| audio_slider_sel.is_shown();
			if conv_parts_sel.action_panel.is_shown() != any_action_visible {
				conv_parts_sel.action_panel.show(any_action_visible);
				need_layout = true;
			}

			conv_parts_sel.action_panel.thaw();

			if need_layout {
				let now = std::time::Instant::now();
				if now.duration_since(last_layout_time.get()) > std::time::Duration::from_millis(100) {
					conv_parts_sel.action_panel.layout();
					last_layout_time.set(now);
				}
			}
		} else {
			let s = state_sel.borrow();
			conv_parts_sel.action_panel.freeze();
			let mut need_layout = crate::update_action_buttons(&s, &conv_parts_sel);
			if play_voice_btn_sel.is_shown() { play_voice_btn_sel.show(false); need_layout = true; }
			if audio_slider_sel.is_shown() { audio_slider_sel.show(false); audio_progress_label_sel.show(false); need_layout = true; }

			// Hide action_panel when no buttons are visible
			if conv_parts_sel.action_panel.is_shown() {
				conv_parts_sel.action_panel.show(false);
				need_layout = true;
			}

			conv_parts_sel.action_panel.thaw();

			if need_layout {
				let now = std::time::Instant::now();
				if now.duration_since(last_layout_time.get()) > std::time::Duration::from_millis(100) {
					conv_parts_sel.action_panel.layout();
					last_layout_time.set(now);
				}
			}
		}
	});

	let tx_toggle_search = ui_tx.clone();
	parts.conversations_parts.search_in_conv_btn.on_click(move |_| {
		let _ = tx_toggle_search.send(UiCommand::ToggleSearchPanel);
	});

	let tx_close_search = ui_tx.clone();
	parts.conversations_parts.search_close_btn.on_click(move |_| {
		let _ = tx_close_search.send(UiCommand::CloseSearch);
	});

	let tx_search_text = ui_tx.clone();
	let search_field_ref = parts.conversations_parts.search_conv_field;
	let msgs_list_search_focus = parts.conversations_parts.messages_list.clone();
	search_field_ref.on_text_changed(move |_| {
		let query = search_field_ref.get_value();
		if !query.is_empty() {
			let _ = tx_search_text.send(UiCommand::SearchInConversation(query));
		}
	});
	search_field_ref.on_text_enter(move |_| {
		msgs_list_search_focus.set_focus();
		if msgs_list_search_focus.get_count() > 0 {
			msgs_list_search_focus.set_selection(0, true);
		}
	});

	let tx_record = ui_tx.clone();
	parts.conversations_parts.record_voice_btn.on_click(move |_| {
		let _ = tx_record.send(UiCommand::StartRecording);
	});

	let tx_pause = ui_tx.clone();
	parts.conversations_parts.pause_resume_btn.on_click(move |_| {
		let _ = tx_pause.send(UiCommand::TogglePauseRecording);
	});

	let tx_send_voice = ui_tx.clone();
	parts.conversations_parts.send_voice_btn.on_click(move |_| {
		let _ = tx_send_voice.send(UiCommand::SendVoiceMessage);
	});

	let tx_discard_voice = ui_tx.clone();
	parts.conversations_parts.discard_voice_btn.on_click(move |_| {
		let _ = tx_discard_voice.send(UiCommand::DiscardVoiceMessage);
	});

	let tx_attach = ui_tx.clone();
	parts.conversations_parts.add_attachment_btn.on_click(move |_| {
		let _ = tx_attach.send(UiCommand::AddAttachment);
	});

	// Attachment dialog — Send button
	let tx_send_attach = ui_tx.clone();
	let caption_field_ref = parts.attachment_dialog.caption_field;
	parts.attachment_dialog.send_btn.on_click(move |_| {
		let caption = caption_field_ref.get_value();
		let _ = tx_send_attach.send(UiCommand::SendAttachment { caption });
	});

	// Attachment dialog — Add more files button
	let tx_add_more = ui_tx.clone();
	parts.attachment_dialog.add_more_btn.on_click(move |_| {
		let _ = tx_add_more.send(UiCommand::AddAttachment);
	});

	// Attachment dialog — Discard button
	let tx_discard_attach = ui_tx.clone();
	parts.attachment_dialog.discard_btn.on_click(move |_| {
		let _ = tx_discard_attach.send(UiCommand::DiscardAttachment);
	});

	let tx_download = ui_tx.clone();
	parts.conversations_parts.action_download_btn.on_click(move |_| {
		let _ = tx_download.send(UiCommand::DownloadMedia);
	});

	let tx_open_media = ui_tx.clone();
	parts.conversations_parts.action_open_btn.on_click(move |_| {
		let _ = tx_open_media.send(UiCommand::OpenMedia);
	});

	let tx_save_as = ui_tx.clone();
	parts.conversations_parts.action_save_as_btn.on_click(move |_| {
		let _ = tx_save_as.send(UiCommand::SaveMediaAs);
	});

	let tx_cancel_edit = ui_tx.clone();
	parts.conversations_parts.cancel_edit_btn.on_click(move |_| {
		let _ = tx_cancel_edit.send(UiCommand::CancelEdit);
	});

	let tx_remove_quote = ui_tx.clone();
	parts.conversations_parts.remove_quote_btn.on_click(move |_| {
		let _ = tx_remove_quote.send(UiCommand::RemoveQuote);
	});

	let tx_read_more = ui_tx.clone();
	parts.conversations_parts.read_more_btn.on_click(move |_| {
		let _ = tx_read_more.send(UiCommand::LoadMoreMessages);
	});

	// Wire audio slider for seeking (drag/mouse)
	let tx_slider = ui_tx.clone();
	parts.conversations_parts.audio_slider.on_slider(move |event| {
		let val = event.get_value();
		let _ = tx_slider.send(UiCommand::SeekAudioAbsolute(val));
	});

	// Wire audio slider keyboard seeking (Left/Right arrow ±5 seconds)
	let tx_slider_key = ui_tx.clone();
	parts.conversations_parts.audio_slider.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key) = event {
			let code = key.get_key_code();
			if code == Some(314) { // Left arrow
				let _ = tx_slider_key.send(UiCommand::SeekAudio(-5.0));
				event.skip(false);
			} else if code == Some(316) { // Right arrow
				let _ = tx_slider_key.send(UiCommand::SeekAudio(5.0));
				event.skip(false);
			} else {
				event.skip(true);
			}
		} else {
			event.skip(true);
		}
	});
}
