use wxdragon::prelude::*;

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
	pub settings_dialog: crate::ui::settings::SettingsDialogParts,
	pub conversations_parts: crate::ui::conversations::ConversationsPanelParts,
}

pub fn build_main_window() -> WindowParts {
	let frame = Frame::builder().with_title("WinZapp Rust").with_size(Size::new(900, 600)).build();
	wxdragon::app::set_top_window(&frame);

	let main_panel = Panel::builder(&frame).build();
	let main_sizer = BoxSizer::builder(Orientation::Horizontal).build();

	let nav_panel = Panel::builder(&main_panel).build();
	let nav_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let nav_label = StaticText::builder(&nav_panel).with_label("Main Navigation").build();
	let nav_list = ListBox::builder(&nav_panel).with_choices(vec![
		"Conversations".to_string(),
		"Archived Chats".to_string(),
		"Settings".to_string(),
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
	let settings_dialog = crate::ui::settings::build_settings_dialog(&frame);
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
		settings_dialog,
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
	parts.new_conv_dialog.new_contact_btn.on_click(move |_| {
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
		let _ = tx_new_contact.send(UiCommand::NewContact {
			name: full_name,
			phone: clean_phone,
		});
	});

	let tx_prepare = ui_tx.clone();
	parts.new_conv_dialog.new_group_btn.on_click(move |_| {
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
			MessageDialog::builder(&group_dialog, "Nome do grupo é obrigatório.", "Erro").with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError).build().show_modal();
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
			MessageDialog::builder(&group_dialog, "Pelo menos um participante deve ser selecionado.", "Erro").with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError).build().show_modal();
			contacts_listbox_ok.set_focus();
			return;
		}
		ok_btn.enable(false);
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
				let state_str = if checked { "selecionado" } else { "não selecionado" };
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

	let settings_dialog_ok = parts.settings_dialog.dialog;
	let tx_save = ui_tx.clone();
	parts.settings_dialog.ok_btn.on_click(move |_| {
		let _ = tx_save.send(UiCommand::SaveSettings);
		settings_dialog_ok.end_modal(wxdragon::id::ID_OK);
	});

	let settings_dialog_cancel = parts.settings_dialog.dialog;
	parts.settings_dialog.cancel_btn.on_click(move |_| {
		settings_dialog_cancel.end_modal(wxdragon::id::ID_CANCEL);
	});

	let tx_apply = ui_tx.clone();
	parts.settings_dialog.apply_btn.on_click(move |_| {
		let _ = tx_apply.send(UiCommand::SaveSettings);
	});

	let conv_new_conv_dialog = parts.new_conv_dialog.dialog;
	parts.conversations_parts.new_conv_btn.on_click(move |_| {
		conv_new_conv_dialog.show_modal();
	});

	let tx_conv_send = ui_tx.clone();
	let conv_msg_field = parts.conversations_parts.message_field;
	parts.conversations_parts.send_message_btn.on_click(move |_| {
		let text = conv_msg_field.get_value();
		if !text.is_empty() {
			let _ = tx_conv_send.send(UiCommand::SendMessage(text));
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

	let tx_toggle_search = ui_tx.clone();
	parts.conversations_parts.search_in_conv_btn.on_click(move |_| {
		let _ = tx_toggle_search.send(UiCommand::ToggleSearchPanel);
	});

	let _tx_close_search = ui_tx.clone();
	let search_panel_ref = parts.conversations_parts.search_panel;
	let conv_panel_ref = parts.conversations_parts.conversation_panel;
	parts.conversations_parts.search_close_btn.on_click(move |_| {
		search_panel_ref.show(false);
		conv_panel_ref.layout();
	});

	let tx_search_text = ui_tx.clone();
	let search_field_ref = parts.conversations_parts.search_conv_field;
	search_field_ref.on_text_changed(move |_| {
		let query = search_field_ref.get_value();
		if !query.is_empty() {
			let _ = tx_search_text.send(UiCommand::SearchInConversation(query));
		}
	});

	let tx_search_next = ui_tx.clone();
	parts.conversations_parts.search_next_btn.on_click(move |_| {
		let _ = tx_search_next.send(UiCommand::SearchNextResult);
	});

	let tx_search_prev = ui_tx.clone();
	parts.conversations_parts.search_prev_btn.on_click(move |_| {
		let _ = tx_search_prev.send(UiCommand::SearchPrevResult);
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

	let tx_send_attach = ui_tx.clone();
	let caption_field_ref = parts.conversations_parts.caption_field;
	parts.conversations_parts.send_attachment_btn.on_click(move |_| {
		let caption = caption_field_ref.get_value();
		let _ = tx_send_attach.send(UiCommand::SendAttachment { caption });
	});

	let tx_add_more = ui_tx.clone();
	parts.conversations_parts.add_more_btn.on_click(move |_| {
		let _ = tx_add_more.send(UiCommand::AddAttachment);
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
}
