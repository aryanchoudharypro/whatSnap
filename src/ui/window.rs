use std::rc::Rc;

use wxdragon::{
	event::{EventType, WxEvtHandler},
	prelude::*,
};

use crate::{UiCommand, ui::theme, ui_wake::UiCommandSender};

/// Strips characters Windows can't have in a filename, for the default
/// export filename built from a chat/contact name.
fn sanitize_export_filename(name: &str) -> String {
	let cleaned: String = name.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) { '_' } else { c }).collect();
	let trimmed = cleaned.trim();
	if trimmed.is_empty() { "chat".to_string() } else { trimmed.to_string() }
}

/// Confirms deleting `count` message(s). If every one of them is your own
/// (`all_from_me`), offers a choice between revoking for everyone and
/// removing them locally only ("delete for me"); if even one belongs to
/// someone else, the whole batch can only ever be removed locally (matching
/// official WhatsApp: "delete for everyone" is unavailable the moment a
/// selection mixes in someone else's messages), so it's a plain yes/no
/// confirm. Returns `None` if the user cancelled, else `Some(for_everyone)`.
fn confirm_delete_choice(parent: &impl WxWidget, count: usize, all_from_me: bool) -> Option<bool> {
	let plural = if count == 1 { "this message" } else { "these messages" };
	if all_from_me {
		let choices: Vec<&str> = vec!["Delete for Everyone", "Delete for Me"];
		let dialog = SingleChoiceDialog::builder(
			parent,
			&format!("Delete {plural} for everyone, or just remove {} from your own device?", plural),
			"Delete Message",
			&choices,
		)
		.build();
		if dialog.show_modal() != wxdragon::id::ID_OK {
			return None;
		}
		match dialog.get_string_selection().as_deref() {
			Some("Delete for Everyone") => Some(true),
			Some("Delete for Me") => Some(false),
			_ => None,
		}
	} else {
		let dlg = MessageDialog::builder(
			parent,
			&format!("Are you sure you want to delete {plural} from your device?"),
			"Confirm Delete",
		)
		.with_style(MessageDialogStyle::OK | MessageDialogStyle::Cancel | MessageDialogStyle::IconQuestion)
		.build();
		if dlg.show_modal() == wxdragon::id::ID_OK { Some(false) } else { None }
	}
}

/// wxDialog::Create() on MSW doesn't set the native window's owner
/// (GWLP_HWNDPARENT) from the `parent` it's given, so every dialog we build
/// ends up as an unowned top-level window - Windows then gives each one its
/// own Alt-Tab/taskbar entry instead of grouping it under the main frame
/// like a proper child dialog. Setting the owner directly via the Win32 API
/// right after construction fixes that.
fn set_dialog_owner(dialog: &impl WxWidget, owner: &impl WxWidget) {
	let dialog_handle = dialog.get_handle();
	let owner_handle = owner.get_handle();
	if dialog_handle.is_null() || owner_handle.is_null() {
		return;
	}
	unsafe {
		windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(
			windows::Win32::Foundation::HWND(dialog_handle),
			windows::Win32::UI::WindowsAndMessaging::GWLP_HWNDPARENT,
			owner_handle as isize,
		);
	}
}

/// Wires a ListBox in one of the group/conversation "data" dialogs so that
/// pressing Enter on a row closes the dialog and jumps to the message it
/// came from (like a search result), and - for lists of actual media files -
/// so the context menu (right-click or the Menu/Shift+F10 key) offers
/// Download/Open/Save As, mirroring the actions available for a selected
/// media message in the main conversation view.
fn wire_media_list(
	list: &ListBox,
	dialog: &Dialog,
	ui_tx: &UiCommandSender,
	state: &std::rc::Rc<std::cell::RefCell<crate::AppState>>,
	get_message: impl Fn(&crate::AppState, usize) -> Option<whantv::ChatMessage> + Clone + 'static,
	with_media_actions: bool,
) {
	let list_enter = list.clone();
	let dialog_enter = dialog.clone();
	let tx_enter = ui_tx.clone();
	let state_enter = state.clone();
	let get_message_enter = get_message.clone();
	list.on_key_up(move |event| {
		if let WindowEventData::Keyboard(ref key_event) = event {
			let code = key_event.get_key_code();
			if code == Some(13) || code == Some(370) {
				// Enter or numpad Enter
				if let Some(idx) = list_enter.get_selection() {
					if let Some(msg) = get_message_enter(&state_enter.borrow(), idx as usize) {
						dialog_enter.end_modal(0);
						let _ = tx_enter
							.send(UiCommand::JumpToSearchResult { message_id: msg.id, message_text: String::new() });
					}
				}
				event.skip(false);
				return;
			}
		}
		event.skip(true);
	});

	if with_media_actions {
		let list_ctx = list.clone();
		let tx_ctx = ui_tx.clone();
		let state_ctx = state.clone();
		list.bind_internal(EventType::CONTEXT_MENU, move |_event| {
			let Some(idx) = list_ctx.get_selection() else { return };
			let Some(msg) = get_message(&state_ctx.borrow(), idx as usize) else { return };

			let download_id = 4101;
			let open_id = 4102;
			let save_id = 4103;
			let is_downloaded = msg.is_media_downloaded();
			let mut menu_builder = Menu::builder();
			if is_downloaded {
				menu_builder = menu_builder.append_item(open_id, "&Open", "Open this file").append_item(
					save_id,
					"&Save as...",
					"Save this file to a chosen location",
				);
			} else {
				menu_builder = menu_builder.append_item(download_id, "&Download", "Download this file");
			}
			let mut popup_menu = menu_builder.build();

			let tx_menu = tx_ctx.clone();
			let message_id = msg.id.clone();
			popup_menu.on_selected(move |menu_evt| {
				let id = menu_evt.get_id();
				let cmd = if id == download_id {
					Some(UiCommand::DownloadMediaMessage(message_id.clone()))
				} else if id == open_id {
					Some(UiCommand::OpenMediaMessage(message_id.clone()))
				} else if id == save_id {
					Some(UiCommand::SaveMediaMessageAs(message_id.clone()))
				} else {
					None
				};
				if let Some(cmd) = cmd {
					let _ = tx_menu.send(cmd);
				}
			});

			list_ctx.popup_menu(&mut popup_menu, None);
			popup_menu.destroy_menu();
		});
	}
}

/// Every constructed widget is kept here for consistency even if not all of
/// them are read back after construction yet (e.g. phone_panel).
#[allow(dead_code)]
pub struct WindowParts {
	pub frame: Frame,
	pub nav_list: ListBox,
	pub content_panel: Panel,
	pub login_panel: Dialog,
	pub main_panel: Panel,
	pub nav_panel: Panel,
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
	pub call_parts: crate::ui::call_window::CallWindowParts,
	pub conversation_data_dialog: crate::ui::conversation_data::ConversationDataDialogParts,
	pub group_data_dialog: crate::ui::group_data::GroupDataDialogParts,
	pub conversations_parts: crate::ui::conversations::ConversationsPanelParts,
	pub settings_dialog: crate::ui::settings_dialog::SettingsDialogParts,
	pub tray_icon: Rc<TaskBarIcon>,
}

/// IDs for the frame's menu bar (Account menu). Not app-global command IDs -
/// just needs to be outside wx's reserved/auto-assigned ranges.
pub const MENU_ID_SETTINGS: i32 = 6001;
pub const MENU_ID_LOGOUT: i32 = 6002;

pub fn build_main_window() -> WindowParts {
	let frame = Frame::builder().with_title("whantv").with_size(Size::new(1180, 760)).build();
	frame.set_min_size(Size::new(900, 600));
	wxdragon::app::set_top_window(&frame);
	frame.create_status_bar(1, 0, wxdragon::id::ID_ANY as i32, "status_bar");
	frame.set_status_text("Status: Initializing...", 0);
	theme::style_content(&frame);

	let main_panel = Panel::builder(&frame).build();
	theme::style_content(&main_panel);
	let main_sizer = BoxSizer::builder(Orientation::Horizontal).build();

	let nav_panel = Panel::builder(&main_panel).build();
	theme::style_sidebar(&nav_panel);
	nav_panel.set_min_size(Size::new(200, -1));
	let nav_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let nav_label = StaticText::builder(&nav_panel).with_label("Main Navigation").build();
	theme::style_sidebar_heading(&nav_label);
	let nav_list = ListBox::builder(&nav_panel)
		.with_choices(vec![
			"Conversations".to_string(),
			"Status".to_string(),
			"Starred Messages".to_string(),
			"Archived Chats".to_string(),
			"Communities".to_string(),
		])
		.build();
	nav_list.set_font(&theme::Fonts::body_strong());
	theme::style_sidebar(&nav_list);
	nav_list.set_selection(0_u32, true);
	nav_sizer.add(&nav_label, 0, SizerFlag::All, 10);
	nav_sizer.add(&nav_list, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom, 10);

	nav_panel.set_sizer(nav_sizer, true);

	let account_menu = Menu::builder()
		.append_item(MENU_ID_SETTINGS, "&Settings...", "Open settings")
		.append_separator()
		.append_item(MENU_ID_LOGOUT, "Log &Out", "Log out of WhatsApp on this device")
		.build();
	let menu_bar = MenuBar::builder().append(account_menu, "&Account").build();
	frame.set_menu_bar(menu_bar);

	let content_panel = Panel::builder(&main_panel).build();
	theme::style_content(&content_panel);
	let content_sizer = BoxSizer::builder(Orientation::Vertical).build();

	let login_panel = Dialog::builder(&frame, "Login").with_size(500, 320).build();
	theme::style_content(&login_panel);
	let login_sizer = BoxSizer::builder(Orientation::Vertical).build();

	let qrcode_dialog = Dialog::builder(&frame, "QR Code").with_size(550, 550).build();
	theme::style_content(&qrcode_dialog);
	let qr_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let qrcode_instructions =
		StaticText::builder(&qrcode_dialog).with_label("Scan this QR Code with WhatsApp:").build();
	theme::style_heading(&qrcode_instructions);
	let empty_bitmap = wxdragon::bitmap::Bitmap::from_rgba(&[255, 255, 255, 255], 1, 1).unwrap();
	let qrcode_display = StaticBitmap::builder(&qrcode_dialog)
		.with_bitmap(Some(empty_bitmap))
		.with_scale_mode(Some(ScaleMode::AspectFit))
		.build();

	let qrcode_announcer = StaticText::builder(&qrcode_dialog).with_label("").build();
	live_region::set_live_region(&qrcode_announcer);
	theme::style_muted_label(&qrcode_announcer);

	let qrcode_cancel_btn =
		Button::builder(&qrcode_dialog).with_id(wxdragon::id::ID_CANCEL).with_label("Cancel QR Code").build();
	qrcode_cancel_btn.set_font(&theme::Fonts::body());
	qr_sizer.add(&qrcode_instructions, 0, SizerFlag::All | SizerFlag::AlignCentre, 10);
	qr_sizer.add(&qrcode_display, 0, SizerFlag::Expand | SizerFlag::All, 10);
	qr_sizer.add(&qrcode_announcer, 0, SizerFlag::Left, 0);
	qr_sizer.add(&qrcode_cancel_btn, 0, SizerFlag::All | SizerFlag::AlignCentre, 10);
	qrcode_dialog.set_sizer(qr_sizer, true);

	// Phone Panel
	let phone_panel = Panel::builder(&login_panel).build();
	theme::style_content(&phone_panel);
	let phone_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let country_label = StaticText::builder(&phone_panel).with_label("Country:").build();
	theme::style_muted_label(&country_label);
	let mut choices = Vec::new();
	for &(name, _) in crate::countries::COUNTRIES {
		choices.push(name.to_string());
	}
	let country_combo =
		ComboBox::builder(&phone_panel).with_choices(choices).with_style(ComboBoxStyle::ReadOnly).build();
	country_combo.set_selection(0_u32);
	country_combo.set_font(&theme::Fonts::body());

	let phone_label = StaticText::builder(&phone_panel).with_label("Enter phone number:").build();
	theme::style_muted_label(&phone_label);
	let phone_field = TextCtrl::builder(&phone_panel).with_value("+55 ").build();
	phone_field.set_font(&theme::Fonts::body());
	let pair_btn = Button::builder(&phone_panel).with_label("Continue").build();
	theme::style_primary_button(&pair_btn);
	let switch_to_qr_btn = Button::builder(&phone_panel).with_label("Connect with QR Code").build();
	switch_to_qr_btn.set_font(&theme::Fonts::body());

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
	let pairing_dialog = Dialog::builder(&frame, "Pairing Code").with_size(320, 170).build();
	theme::style_content(&pairing_dialog);
	let p_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let pairing_instructions =
		StaticText::builder(&pairing_dialog).with_label("Enter this pairing code in WhatsApp on your phone:").build();
	theme::style_heading(&pairing_instructions);
	let pairing_code_display =
		TextCtrl::builder(&pairing_dialog).with_style(TextCtrlStyle::ReadOnly | TextCtrlStyle::Centre).build();
	pairing_code_display.set_font(&theme::Fonts::heading());

	let pairing_announcer = StaticText::builder(&pairing_dialog).with_label("").build();
	live_region::set_live_region(&pairing_announcer);

	let pairing_cancel_btn =
		Button::builder(&pairing_dialog).with_id(wxdragon::id::ID_CANCEL).with_label("Cancel").build();
	pairing_cancel_btn.set_font(&theme::Fonts::body());
	p_sizer.add(&pairing_instructions, 0, SizerFlag::All | SizerFlag::AlignCentre, 10);
	p_sizer.add(&pairing_code_display, 0, SizerFlag::Expand | SizerFlag::All, 10);
	p_sizer.add(&pairing_announcer, 0, SizerFlag::Left, 0); // hidden visually but in DOM
	pairing_announcer.show(false);
	p_sizer.add(&pairing_cancel_btn, 0, SizerFlag::All | SizerFlag::AlignCentre, 10);
	pairing_dialog.set_sizer(p_sizer, true);

	let pairing_dlg_esc = pairing_dialog.clone();
	pairing_code_display.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key) = event {
			if key.get_key_code() == Some(27) {
				pairing_dlg_esc.end_modal(wxdragon::id::ID_CANCEL);
				event.skip(false);
				return;
			}
		}
		event.skip(true);
	});

	let new_conv_dialog = crate::ui::new_conversation::build_new_conversation_dialog(&frame);
	let new_contact_dialog = crate::ui::new_contact::build_new_contact_dialog(&frame);
	let new_group_dialog = crate::ui::new_group::build_new_group_dialog(&frame);
	let attachment_dialog = crate::ui::attachment_dialog::build_attachment_dialog(&frame);
	let call_parts = crate::ui::call_window::build_call_window();
	let conversation_data_dialog = crate::ui::conversation_data::build_conversation_data_dialog(&frame);
	let group_data_dialog = crate::ui::group_data::build_group_data_dialog(&frame);
	let settings_dialog = crate::ui::settings_dialog::build_settings_dialog(&frame);
	let tray_icon = Rc::new(TaskBarIcon::builder().build());

	let conv_panel = crate::ui::conversations::build_conversations_panel(&content_panel);

	content_sizer.add(&conv_panel.panel, 1, SizerFlag::Expand, 0);
	conv_panel.panel.show(true);

	// Start with login shown, main app hidden
	nav_panel.show(false);
	content_panel.show(false);
	login_panel.show(true);

	content_panel.set_sizer(content_sizer, true);

	main_sizer.add(&nav_panel, 0, SizerFlag::Expand | SizerFlag::All, 5);
	main_sizer.add(&content_panel, 1, SizerFlag::Expand | SizerFlag::All, 5);
	main_panel.set_sizer(main_sizer, true);

	let frame_sizer = BoxSizer::builder(Orientation::Vertical).build();
	frame_sizer.add(&main_panel, 1, SizerFlag::Expand, 0);
	frame.set_sizer(frame_sizer, true);

	// Group every dialog under the main frame's Alt-Tab/taskbar entry
	// instead of letting each show up as its own independent window (see
	// set_dialog_owner). The call window is deliberately left out - it's a
	// Frame the user is meant to be able to switch to independently while a
	// call is up, not a transient dialog.
	set_dialog_owner(&login_panel, &frame);
	set_dialog_owner(&qrcode_dialog, &frame);
	set_dialog_owner(&pairing_dialog, &frame);
	set_dialog_owner(&new_conv_dialog.dialog, &frame);
	set_dialog_owner(&new_contact_dialog.dialog, &frame);
	set_dialog_owner(&new_group_dialog.dialog, &frame);
	set_dialog_owner(&attachment_dialog.dialog, &frame);
	set_dialog_owner(&conversation_data_dialog.dialog, &frame);
	set_dialog_owner(&group_data_dialog.dialog, &frame);
	set_dialog_owner(&settings_dialog.dialog, &frame);

	WindowParts {
		frame,
		nav_list,
		content_panel,
		login_panel,
		main_panel,
		nav_panel,
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
		call_parts,
		conversation_data_dialog,
		group_data_dialog,
		conversations_parts: conv_panel,
		settings_dialog,
		tray_icon,
	}
}

pub fn bind_input_handlers(
	parts: &WindowParts,
	ui_tx: UiCommandSender,
	state: &std::rc::Rc<std::cell::RefCell<crate::AppState>>,
) {
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
	let tx_post_status = ui_tx.clone();
	let state_new_conv = state.clone();
	let frame_new_conv = parts.frame.clone();
	parts.conversations_parts.new_conv_btn.on_click(move |_| {
		if state_new_conv.borrow().active_tab == 1 {
			let dlg = TextEntryDialog::builder(&frame_new_conv, "What's on your mind?", "Post Status Update").build();
			if dlg.show_modal() == wxdragon::id::ID_OK {
				if let Some(val) = dlg.get_value() {
					let text = val.trim().to_string();
					if !text.is_empty() {
						let _ = tx_post_status.send(UiCommand::PostStatus { text });
					}
				}
			}
		} else {
			new_conv_dialog.show_modal();
		}
	});

	let tx_post_media_status = ui_tx.clone();
	parts.conversations_parts.post_media_status_btn.on_click(move |_| {
		let _ = tx_post_media_status.send(UiCommand::PostMediaStatus);
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

	let tx_conv_data = ui_tx.clone();
	parts.conversations_parts.conv_data_btn.on_click(move |_| {
		let _ = tx_conv_data.send(UiCommand::ShowConversationData);
	});

	let tx_place_call = ui_tx.clone();
	parts.conversations_parts.place_call_btn.on_click(move |_| {
		let _ = tx_place_call.send(UiCommand::PlaceCall);
	});

	let tx_answer = ui_tx.clone();
	parts.call_parts.accept_btn.on_click(move |_| {
		let _ = tx_answer.send(UiCommand::AnswerCall);
	});
	let tx_reject = ui_tx.clone();
	parts.call_parts.reject_btn.on_click(move |_| {
		let _ = tx_reject.send(UiCommand::RejectCall);
	});
	let tx_mute = ui_tx.clone();
	parts.call_parts.mute_btn.on_click(move |_| {
		let _ = tx_mute.send(UiCommand::MuteCall);
	});
	let tx_leave = ui_tx.clone();
	parts.call_parts.leave_btn.on_click(move |_| {
		let _ = tx_leave.send(UiCommand::LeaveCall);
	});
	let tx_add_participant = ui_tx.clone();
	parts.call_parts.add_participant_btn.on_click(move |_| {
		let _ = tx_add_participant.send(UiCommand::AddParticipant);
	});

	let conv_data_dialog_back = parts.conversation_data_dialog.dialog.clone();
	parts.conversation_data_dialog.back_btn.on_click(move |_| {
		conv_data_dialog_back.end_modal(0);
	});

	let group_data_dialog_back = parts.group_data_dialog.dialog.clone();
	parts.group_data_dialog.back_btn.on_click(move |_| {
		group_data_dialog_back.end_modal(0);
	});

	// ── Tray icon ────────────────────────────────────────────────────────────
	// The icon bitmap and popup menu are set up once; whether it's actually
	// installed (and thus whether closing the window hides to it) tracks the
	// `show_tray_icon` setting, reconciled below and whenever Settings is saved.
	let tray_icon = parts.tray_icon.clone();
	let tray_bitmap = theme::build_app_icon(32);

	let tray_open_id = 5001;
	let tray_exit_id = 5002;
	let mut tray_menu = Menu::builder()
		.append_item(tray_open_id, "&Open whaNTV", "Show the main window")
		.append_item(tray_exit_id, "E&xit", "Quit whaNTV")
		.build();
	let frame_tray_menu = parts.frame;
	let tx_tray_menu = ui_tx.clone();
	tray_icon.on_menu(move |event| {
		let id = event.get_id();
		if id == tray_open_id {
			frame_tray_menu.show(true);
			frame_tray_menu.iconize(false);
			let _ = tx_tray_menu.send(UiCommand::SetPresence(true));
		} else if id == tray_exit_id {
			// Closing just the main frame doesn't end the app on its own: wx only
			// auto-exits once every top-level window it knows about is destroyed,
			// and the call window (and every dialog built up front in this
			// function) is created once at startup and only ever shown/hidden,
			// never destroyed - so it, and them, would otherwise keep the process
			// (and the backend it owns) alive forever in the background after
			// "Exit". Ending the loop explicitly sidesteps that entirely.
			frame_tray_menu.close(true);
			if let Some(app) = wxdragon::get_app_instance() {
				app.exit_main_loop();
			}
		}
	});
	tray_icon.set_popup_menu(&mut tray_menu);
	std::mem::forget(tray_menu);

	let frame_tray_restore = parts.frame;
	let tx_tray_restore = ui_tx.clone();
	tray_icon.on_left_double_click(move |_| {
		frame_tray_restore.show(true);
		frame_tray_restore.iconize(false);
		frame_tray_restore.raise();
		let _ = tx_tray_restore.send(UiCommand::SetPresence(true));
	});

	// A single left click also restores the window — this is what Explorer
	// sends when the icon is "selected" via keyboard (Tab to it, then press
	// Space/Enter) or touch, not a double-click, so without this the tray
	// icon only responds to its context menu from the keyboard.
	let frame_tray_select = parts.frame;
	let tx_tray_select = ui_tx.clone();
	tray_icon.on_left_up(move |_| {
		frame_tray_select.show(true);
		frame_tray_select.iconize(false);
		frame_tray_select.raise();
		let _ = tx_tray_select.send(UiCommand::SetPresence(true));
	});

	let apply_tray_setting = {
		let tray_icon = tray_icon.clone();
		let tray_bitmap = tray_bitmap.clone();
		move |enabled: bool| {
			if enabled {
				tray_icon.set_icon(&tray_bitmap, "whaNTV");
			} else {
				tray_icon.remove_icon();
			}
		}
	};
	apply_tray_setting(state.borrow().settings.show_tray_icon);

	// Closing the window hides it to the tray instead of quitting, as long as
	// the tray icon is actually installed to bring it back with. A forced
	// close (the tray menu's Exit, or Windows shutting the app down) can't be
	// vetoed, so `can_veto()` tells the two cases apart.
	let frame_close = parts.frame;
	let tray_icon_close = tray_icon.clone();
	let tx_close = ui_tx.clone();
	parts.frame.on_close(move |event| {
		if let WindowEventData::General(ref ev) = event {
			if ev.can_veto() && tray_icon_close.is_icon_installed() {
				ev.veto();
				frame_close.show(false);
				let _ = tx_close.send(UiCommand::SetPresence(false));
			} else {
				// A real close (tray icon disabled in Settings, so the X button
				// actually quits) - see the tray Exit handler above for why this
				// explicit ExitMainLoop is needed rather than relying on wx's
				// default "last frame closed" behavior.
				if let Some(app) = wxdragon::get_app_instance() {
					app.exit_main_loop();
				}
			}
		}
	});

	// The window is only "actively in use" while it's focused at the OS
	// level; losing focus (alt-tabbing away, minimizing, or hiding to tray)
	// should stop broadcasting an online presence to WhatsApp, matching what
	// the official clients do.
	let tx_activate = ui_tx.clone();
	parts.frame.on_activate(move |event| {
		if let WindowEventData::Activate(ref ev) = event {
			let _ = tx_activate.send(UiCommand::SetPresence(ev.is_active()));
		}
	});

	let settings_dialog = parts.settings_dialog.dialog;
	let sounds_check = parts.settings_dialog.sounds_check;
	let notifications_check = parts.settings_dialog.notifications_check;
	let announce_typing_check = parts.settings_dialog.announce_typing_check;
	let announce_recording_check = parts.settings_dialog.announce_recording_check;
	let tray_icon_check = parts.settings_dialog.tray_icon_check;
	let autostart_check = parts.settings_dialog.autostart_check;
	let name_field_open = parts.settings_dialog.name_field.clone();
	let about_field_open = parts.settings_dialog.about_field.clone();
	let last_seen_combo_open = parts.settings_dialog.last_seen_combo.clone();
	let profile_photo_combo_open = parts.settings_dialog.profile_photo_combo.clone();
	let about_privacy_combo_open = parts.settings_dialog.about_privacy_combo.clone();
	let read_receipts_combo_open = parts.settings_dialog.read_receipts_combo.clone();
	let groups_combo_open = parts.settings_dialog.groups_combo.clone();
	let calls_combo_open = parts.settings_dialog.calls_combo.clone();
	let online_combo_open = parts.settings_dialog.online_combo.clone();
	let input_device_combo_open = parts.settings_dialog.input_device_combo.clone();
	let output_device_combo_open = parts.settings_dialog.output_device_combo.clone();
	let noise_suppression_check_open = parts.settings_dialog.voice_message_noise_suppression_check.clone();
	let state_settings_open = state.clone();
	parts.frame.on_menu_selected(move |evt| {
		if evt.get_id() != MENU_ID_SETTINGS {
			return;
		}
		use crate::ui::settings_dialog::*;
		let s = state_settings_open.borrow();
		sounds_check.set_value(s.settings.sounds_enabled);
		notifications_check.set_value(s.settings.notifications_enabled);
		announce_typing_check.set_value(s.settings.announce_typing);
		announce_recording_check.set_value(s.settings.announce_recording);
		tray_icon_check.set_value(s.settings.show_tray_icon);
		noise_suppression_check_open.set_value(s.settings.voice_message_noise_suppression);

		name_field_open.set_value(&s.own_push_name);
		about_field_open.set_value(&s.own_about);
		let get = |k: &str| s.privacy_settings.get(k).map(|v| v.as_str()).unwrap_or("all");
		set_combo_from_value(&last_seen_combo_open, VISIBILITY_CHOICES, get("last"));
		set_combo_from_value(&profile_photo_combo_open, VISIBILITY_CHOICES, get("profile"));
		set_combo_from_value(&about_privacy_combo_open, VISIBILITY_CHOICES, get("status"));
		set_combo_from_value(&read_receipts_combo_open, READ_RECEIPTS_CHOICES, get("readreceipts"));
		set_combo_from_value(&groups_combo_open, VISIBILITY_CHOICES, get("groupadd"));
		set_combo_from_value(&calls_combo_open, CALL_CHOICES, get("calladd"));
		set_combo_from_value(&online_combo_open, ONLINE_CHOICES, get("online"));

		// Re-enumerated on every open since WASAPI device lists can change
		// (USB headset plugged/unplugged, etc) while the app is running.
		set_device_combo(
			&input_device_combo_open,
			&crate::audio_devices::input_device_names(),
			s.settings.input_device_name.as_deref(),
		);
		set_device_combo(
			&output_device_combo_open,
			&crate::audio_devices::output_device_names(),
			s.settings.output_device_name.as_deref(),
		);

		let _ = s.ws_tx.send(serde_json::json!({"action": "get_own_profile"}).to_string());
		let _ = s.ws_tx.send(serde_json::json!({"action": "get_privacy_settings"}).to_string());
		drop(s);
		autostart_check.set_value(crate::autostart::is_enabled());
		settings_dialog.show_modal();
	});

	let frame_logout = parts.frame;
	let state_logout = state.clone();
	parts.frame.on_menu_selected(move |evt| {
		if evt.get_id() != MENU_ID_LOGOUT {
			return;
		}
		let dlg = MessageDialog::builder(
			&frame_logout,
			"Log out of WhatsApp on this device? You'll need to scan a QR code or re-enter your phone number to use whaNTV again.",
			"Log Out",
		)
		.with_style(MessageDialogStyle::OK | MessageDialogStyle::Cancel | MessageDialogStyle::IconQuestion)
		.build();
		if dlg.show_modal() != wxdragon::id::ID_OK {
			return;
		}
		let s = state_logout.borrow();
		let _ = s.ws_tx.send(serde_json::json!({"action": "logout"}).to_string());
	});

	let settings_dialog_cancel = parts.settings_dialog.dialog;
	parts.settings_dialog.cancel_btn.on_click(move |_| {
		settings_dialog_cancel.end_modal(wxdragon::id::ID_CANCEL);
	});

	let settings_dialog_ok = parts.settings_dialog.dialog;
	let sounds_check_ok = parts.settings_dialog.sounds_check;
	let notifications_check_ok = parts.settings_dialog.notifications_check;
	let announce_typing_check_ok = parts.settings_dialog.announce_typing_check;
	let announce_recording_check_ok = parts.settings_dialog.announce_recording_check;
	let tray_icon_check_ok = parts.settings_dialog.tray_icon_check;
	let autostart_check_ok = parts.settings_dialog.autostart_check;
	let name_field_ok = parts.settings_dialog.name_field.clone();
	let about_field_ok = parts.settings_dialog.about_field.clone();
	let last_seen_combo_ok = parts.settings_dialog.last_seen_combo.clone();
	let profile_photo_combo_ok = parts.settings_dialog.profile_photo_combo.clone();
	let about_privacy_combo_ok = parts.settings_dialog.about_privacy_combo.clone();
	let read_receipts_combo_ok = parts.settings_dialog.read_receipts_combo.clone();
	let groups_combo_ok = parts.settings_dialog.groups_combo.clone();
	let calls_combo_ok = parts.settings_dialog.calls_combo.clone();
	let online_combo_ok = parts.settings_dialog.online_combo.clone();
	let input_device_combo_ok = parts.settings_dialog.input_device_combo.clone();
	let output_device_combo_ok = parts.settings_dialog.output_device_combo.clone();
	let noise_suppression_check_ok = parts.settings_dialog.voice_message_noise_suppression_check.clone();
	let state_settings_save = state.clone();
	parts.settings_dialog.ok_btn.on_click(move |_| {
		use crate::ui::settings_dialog::*;
		let mut s = state_settings_save.borrow_mut();
		s.settings.sounds_enabled = sounds_check_ok.is_checked();
		s.settings.notifications_enabled = notifications_check_ok.is_checked();
		s.settings.announce_typing = announce_typing_check_ok.is_checked();
		s.settings.announce_recording = announce_recording_check_ok.is_checked();
		s.settings.show_tray_icon = tray_icon_check_ok.is_checked();
		s.settings.voice_message_noise_suppression = noise_suppression_check_ok.is_checked();
		s.settings.input_device_name = device_combo_value(&input_device_combo_ok);
		let new_output_device = device_combo_value(&output_device_combo_ok);
		if new_output_device != s.settings.output_device_name {
			s.settings.output_device_name = new_output_device;
			// Drop the cached playback stream so the next sound/voice-message
			// play reopens it against the newly selected device instead of
			// the one that was live when the dialog opened.
			s.audio_stream = None;
		}
		s.settings.save();

		let new_name = name_field_ok.get_value();
		if new_name != s.own_push_name {
			s.own_push_name = new_name.clone();
			let _ =
				s.ws_tx.send(serde_json::json!({"action": "set_push_name", "data": {"name": new_name}}).to_string());
		}
		let new_about = about_field_ok.get_value();
		if new_about != s.own_about {
			s.own_about = new_about.clone();
			let _ = s.ws_tx.send(serde_json::json!({"action": "set_about", "data": {"text": new_about}}).to_string());
		}
		for (name, value) in [
			("last", combo_value(&last_seen_combo_ok, VISIBILITY_CHOICES)),
			("profile", combo_value(&profile_photo_combo_ok, VISIBILITY_CHOICES)),
			("status", combo_value(&about_privacy_combo_ok, VISIBILITY_CHOICES)),
			("readreceipts", combo_value(&read_receipts_combo_ok, READ_RECEIPTS_CHOICES)),
			("groupadd", combo_value(&groups_combo_ok, VISIBILITY_CHOICES)),
			("calladd", combo_value(&calls_combo_ok, CALL_CHOICES)),
			("online", combo_value(&online_combo_ok, ONLINE_CHOICES)),
		] {
			if s.privacy_settings.get(name).map(|v| v.as_str()) != Some(value.as_str()) {
				s.privacy_settings.insert(name.to_string(), value.clone());
				let _ = s.ws_tx.send(
					serde_json::json!({"action": "set_privacy_setting", "data": {"name": name, "value": value}})
						.to_string(),
				);
			}
		}
		drop(s);
		apply_tray_setting(tray_icon_check_ok.is_checked());
		if crate::autostart::is_enabled() != autostart_check_ok.is_checked() {
			crate::autostart::set_enabled(autostart_check_ok.is_checked());
		}
		settings_dialog_ok.end_modal(wxdragon::id::ID_OK);
	});

	let tx_group_settings = ui_tx.clone();
	let group_name_field = parts.group_data_dialog.group_name_field.clone();
	let group_desc_field = parts.group_data_dialog.group_desc_field.clone();
	let announce_chk = parts.group_data_dialog.announce_chk.clone();
	let locked_chk = parts.group_data_dialog.locked_chk.clone();
	parts.group_data_dialog.save_settings_btn.on_click(move |_| {
		let name = group_name_field.get_value();
		let topic = group_desc_field.get_value();
		let announce = announce_chk.is_checked();
		let locked = locked_chk.is_checked();
		let _ = tx_group_settings.send(UiCommand::UpdateGroupSettings { name, topic, announce, locked });
	});

	let tx_group_members_search = ui_tx.clone();
	let members_search = parts.group_data_dialog.members_search.clone();
	members_search.on_text_changed(move |_| {
		let query = members_search.get_value();
		let _ = tx_group_members_search.send(UiCommand::FilterGroupMembers(query));
	});

	let tx_add_member = ui_tx.clone();
	parts.group_data_dialog.add_member_btn.on_click(move |_| {
		let _ = tx_add_member.send(UiCommand::AddGroupMember);
	});

	let tx_make_admin = ui_tx.clone();
	let members_list_admin = parts.group_data_dialog.members_list.clone();
	parts.group_data_dialog.make_admin_btn.on_click(move |_| {
		if let Some(idx) = members_list_admin.get_selection() {
			let _ = tx_make_admin.send(UiCommand::ToggleGroupMemberAdmin(idx));
		}
	});

	let tx_remove_member = ui_tx.clone();
	let members_list_remove = parts.group_data_dialog.members_list.clone();
	parts.group_data_dialog.remove_member_btn.on_click(move |_| {
		if let Some(idx) = members_list_remove.get_selection() {
			let _ = tx_remove_member.send(UiCommand::RemoveGroupMember(idx));
		}
	});

	let tx_join_group = ui_tx.clone();
	let groups_list_join = parts.group_data_dialog.groups_list.clone();
	parts.group_data_dialog.join_group_btn.on_click(move |_| {
		if let Some(idx) = groups_list_join.get_selection() {
			let _ = tx_join_group.send(UiCommand::JoinCommunitySubgroup(idx));
		}
	});

	wire_media_list(
		&parts.group_data_dialog.media_list,
		&parts.group_data_dialog.dialog,
		&ui_tx,
		state,
		|s, i| s.group_data_media.get(i).cloned(),
		true,
	);
	wire_media_list(
		&parts.group_data_dialog.documents_list,
		&parts.group_data_dialog.dialog,
		&ui_tx,
		state,
		|s, i| s.group_data_documents.get(i).cloned(),
		true,
	);
	wire_media_list(
		&parts.group_data_dialog.links_list,
		&parts.group_data_dialog.dialog,
		&ui_tx,
		state,
		|s, i| s.group_data_links.get(i).cloned(),
		false,
	);

	let conv_data_dialog_esc = parts.conversation_data_dialog.dialog.clone();
	parts.conversation_data_dialog.info_field.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key_event) = event {
			if key_event.get_key_code() == Some(27) {
				conv_data_dialog_esc.end_modal(0);
				event.skip(false);
			} else {
				event.skip(true);
			}
		} else {
			event.skip(true);
		}
	});

	wire_media_list(
		&parts.conversation_data_dialog.media_list,
		&parts.conversation_data_dialog.dialog,
		&ui_tx,
		state,
		|s, i| s.conversation_data_media.get(i).cloned(),
		true,
	);
	wire_media_list(
		&parts.conversation_data_dialog.documents_list,
		&parts.conversation_data_dialog.dialog,
		&ui_tx,
		state,
		|s, i| s.conversation_data_documents.get(i).cloned(),
		true,
	);
	wire_media_list(
		&parts.conversation_data_dialog.links_list,
		&parts.conversation_data_dialog.dialog,
		&ui_tx,
		state,
		|s, i| s.conversation_data_links.get(i).cloned(),
		false,
	);

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
				let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
					windows::Win32::UI::WindowsAndMessaging::MB_OK,
				);
			}
			MessageDialog::builder(&dlg, "Name is required.", "Error")
				.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
				.build()
				.show_modal();
			name_field.set_focus();
			return;
		}
		let clean_phone: String = phone.chars().filter(|c| c.is_digit(10)).collect();
		if clean_phone.len() < 7 {
			unsafe {
				let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
					windows::Win32::UI::WindowsAndMessaging::MB_OK,
				);
			}
			MessageDialog::builder(&dlg, "Phone number must have at least 7 digits.", "Error")
				.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
				.build()
				.show_modal();
			phone_field.set_focus();
			return;
		}
		let full_name = format!("{} {}", name, surname).trim().to_string();
		dlg.end_modal(wxdragon::id::ID_OK);
		let _ = tx_new_contact.send(UiCommand::NewContact { name: full_name, phone: clean_phone });
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
				let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
					windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_STYLE(0x00000030),
				);
			}
			MessageDialog::builder(&group_dialog, "Group name is required.", "Error")
				.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
				.build()
				.show_modal();
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
				let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
					windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_STYLE(0x00000030),
				);
			}
			MessageDialog::builder(&group_dialog, "At least one participant must be selected.", "Error")
				.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
				.build()
				.show_modal();
			contacts_listbox_ok.set_focus();
			return;
		}
		ok_btn.enable(false);
		group_dialog.end_modal(wxdragon::id::ID_OK);
		let _ = tx_create.send(UiCommand::CreateGroup { name, participants: unique_participants });
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
			if code == Some(13) || code == Some(370) {
				// Enter key
				if key_event.shift_down() {
					event.skip(true); // Shift+Enter -> insert newline in multiline text box
				} else {
					let text = conv_msg_field_esc.get_value();
					if !text.is_empty() {
						let _ = tx_send_field.send(UiCommand::SendMessage(text));
					}
					event.skip(false); // Suppress default Enter newline insertion
				}
			} else if code == Some(27) {
				// Escape key
				let _ = tx_send_field.send(UiCommand::CloseConversation);
				event.skip(false);
			} else if code == Some(86) && key_event.control_down() {
				// Ctrl+V
				let mut has_files = false;
				if let Some(_locker) = Clipboard::get().locker() {
					let mut fdo = FileDataObject::new();
					if Clipboard::get().get_data(&mut fdo) && fdo.get_file_count() > 0 {
						has_files = true;
					}
				}
				if has_files {
					let _ = tx_send_field.send(UiCommand::PasteAttachment);
					event.skip(false);
				} else {
					event.skip(true);
				}
			} else {
				event.skip(true);
			}
		} else {
			event.skip(true);
		}
	});

	// Escape only closed the conversation when the message list or composer
	// itself had focus - hitting it from any other control in the
	// conversation panel (a toolbar button, the "Conversation data" field,
	// etc.) did nothing. CHAR_HOOK fires on the frame for every keypress
	// before it reaches whichever control has focus, so it catches Escape
	// regardless of what's focused. Left alone when the in-conversation
	// search panel is showing (Escape closes that first, like official
	// WhatsApp) or selection mode is active (its own list has dedicated
	// Escape handling to exit selection instead).
	let tx_escape_hook = ui_tx.clone();
	let conv_panel_hook = parts.conversations_parts.conversation_panel;
	let search_panel_hook = parts.conversations_parts.search_panel;
	let state_hook = state.clone();
	parts.frame.bind_internal(wxdragon::event::EventType::CHAR_HOOK, move |event| {
		let data = WindowEventData::new(event);
		if let WindowEventData::Keyboard(ref key_event) = data {
			if key_event.get_key_code() == Some(27)
				&& conv_panel_hook.is_shown()
				&& !search_panel_hook.is_shown()
				&& !state_hook.borrow().selection_mode
			{
				let _ = tx_escape_hook.send(UiCommand::CloseConversation);
				data.skip(false);
				return;
			}
		}
		data.skip(true);
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

	let convs_list_ctx = parts.conversations_parts.conversations_list.clone();
	let state_chat_ctx = state.clone();
	let ui_tx_chat_ctx = ui_tx.clone();
	let parent_frame_chat_ctx = parts.frame;

	let open_chat_context_menu = Rc::new(move || {
		if let Some(idx) = convs_list_ctx.get_selection() {
			let pin_id = 4001;
			let archive_id = 4002;
			let mute_id = 4003;
			let read_id = 4004;
			let delete_id = 4005;
			let export_id = 4006;

			let s = state_chat_ctx.borrow();
			let jid_opt = s.filtered_chats.get(idx as usize).cloned();
			if let Some(jid) = jid_opt {
				let is_pinned = s.pinned_jids.contains(&jid);
				let is_archived = s.archived_jids.contains(&jid);
				let is_muted = s.muted_jids.contains(&jid);
				let unread_count = s.unread_counts.get(&jid).copied().unwrap_or(0);
				let is_unread = unread_count > 0;
				let chat_name = s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
				drop(s);

				let mut menu_builder = Menu::builder();

				let pin_label = if is_pinned { "Un&pin chat" } else { "&Pin chat" };
				let archive_label = if is_archived { "&Unarchive chat" } else { "&Archive chat" };
				let mute_label = if is_muted { "&Unmute notifications" } else { "&Mute notifications" };
				let read_label = if is_unread { "Mark as &read" } else { "Mark as &unread" };

				menu_builder = menu_builder
					.append_item(pin_id, pin_label, "Pin or unpin chat")
					.append_item(archive_id, archive_label, "Archive or unarchive chat")
					.append_item(mute_id, mute_label, "Mute or unmute notifications")
					.append_item(read_id, read_label, "Mark chat as read or unread")
					.append_item(export_id, "&Export chat...", "Export this chat's transcript and media")
					.append_item(delete_id, "&Delete chat...", "Delete chat history");

				let mut popup_menu = menu_builder.build();

				let ui_tx_clone = ui_tx_chat_ctx.clone();
				let jid_clone = jid.clone();
				let parent_clone = parent_frame_chat_ctx.clone();

				popup_menu.on_selected(move |menu_evt| {
					let id = menu_evt.get_id();
					if id == pin_id {
						let _ = ui_tx_clone.send(UiCommand::TogglePinChat { jid: jid_clone.clone(), pin: !is_pinned });
					} else if id == archive_id {
						let _ = ui_tx_clone
							.send(UiCommand::ToggleArchiveChat { jid: jid_clone.clone(), archive: !is_archived });
					} else if id == mute_id {
						let _ = ui_tx_clone.send(UiCommand::ToggleMuteChat { jid: jid_clone.clone(), mute: !is_muted });
					} else if id == read_id {
						let _ =
							ui_tx_clone.send(UiCommand::ToggleReadChat { jid: jid_clone.clone(), read: !is_unread });
					} else if id == delete_id {
						let dialog = MessageDialog::builder(
							&parent_clone,
							&format!("Are you sure you want to delete chat with {}?", chat_name),
							"Confirm Delete",
						)
						.with_style(
							MessageDialogStyle::OK | MessageDialogStyle::Cancel | MessageDialogStyle::IconQuestion,
						)
						.build();
						if dialog.show_modal() == wxdragon::id::ID_OK {
							let _ = ui_tx_clone.send(UiCommand::DeleteChat { jid: jid_clone.clone() });
						}
					} else if id == export_id {
						let default_name = format!("{}.zip", sanitize_export_filename(&chat_name));
						let dialog = FileDialog::builder(&parent_clone)
							.with_message("Export chat as")
							.with_default_file(&default_name)
							.with_wildcard("Zip archive (*.zip)|*.zip")
							.with_style(FileDialogStyle::Save | FileDialogStyle::OverwritePrompt)
							.build();
						if dialog.show_modal() == wxdragon::id::ID_OK {
							if let Some(path) = dialog.get_path() {
								let _ =
									ui_tx_clone.send(UiCommand::ExportChat { jid: jid_clone.clone(), dest_path: path });
							}
						}
					}
				});

				convs_list_ctx.popup_menu(&mut popup_menu, None);
				popup_menu.destroy_menu();
			}
		}
	});

	let open_chat_menu_key = open_chat_context_menu.clone();
	let conv_list_key = parts.conversations_parts.conversations_list.clone();
	let tx_open_enter = ui_tx.clone();
	parts.conversations_parts.conversations_list.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key_event) = event {
			let code = key_event.get_key_code();
			if code == Some(13) {
				if let Some(idx) = conv_list_key.get_selection() {
					let _ = tx_open_enter.send(UiCommand::OpenConversation(idx as usize));
				}
				event.skip(false);
			} else if !key_event.meta_down()
				&& (code == Some(395) || code == Some(93) || (key_event.shift_down() && code == Some(349)))
			{
				open_chat_menu_key.as_ref()();
				event.skip(false);
			} else {
				event.skip(true);
			}
		} else {
			event.skip(true);
		}
	});

	let open_chat_menu_cb = open_chat_context_menu.clone();
	parts.conversations_parts.conversations_list.bind_internal(EventType::CONTEXT_MENU, move |_event| {
		open_chat_menu_cb.as_ref()();
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
	let remove_quote_btn_play = parts.conversations_parts.remove_quote_btn;
	let conv_panel_play = parts.conversations_parts.conversation_panel;
	let _conv_panel_key = parts.conversations_parts.conversation_panel;
	let last_left_arrow =
		std::rc::Rc::new(std::cell::Cell::new(std::time::Instant::now() - std::time::Duration::from_secs(10)));
	let msgs_list_ctx = parts.conversations_parts.messages_list.clone();
	let state_ctx = state.clone();
	let parent_frame_ctx = parts.frame.clone();
	let msgs_announcer_ctx = parts.conversations_parts.messages_announcer.clone();
	let msg_input_field_ctx = parts.conversations_parts.message_field.clone();
	let tx_play_media_ctx = ui_tx.clone();
	let remove_quote_btn_ctx = parts.conversations_parts.remove_quote_btn;
	let conv_panel_ctx = parts.conversations_parts.conversation_panel;

	let state_copy = state.clone();
	let tx_copy = ui_tx.clone();
	let msgs_announcer_copy = parts.conversations_parts.messages_announcer.clone();
	let msgs_list_copy = parts.conversations_parts.messages_list.clone();

	let perform_copy_selected = std::rc::Rc::new(move || {
		if let Some(idx) = msgs_list_copy.get_selection() {
			let s = state_copy.borrow();
			if let Some(display_item) = s.display_items.get(idx as usize).cloned() {
				drop(s);
				match display_item {
					whantv::DisplayItem::Single(msg) => {
						let is_voice = msg.is_audio
							|| msg.media_type == "voice"
							|| msg.media_type == "audio"
							|| msg.text.starts_with("[Voice Message]")
							|| msg.text.starts_with("[Audio]");
						let is_media = is_voice
							|| !msg.media_type.is_empty()
							|| msg.text.starts_with("[Image]")
							|| msg.text.starts_with("[Photo]")
							|| msg.text.starts_with("[Video]")
							|| msg.text.starts_with("[Document]")
							|| msg.text.starts_with("[Sticker]");

						if is_media {
							if msg.is_media_downloaded()
								&& !msg.media_path.is_empty()
								&& std::path::Path::new(&msg.media_path).exists()
							{
								let display_name = msg.display_filename();
								let temp_dir = std::env::temp_dir().join("whantv_clipboard");
								let _ = std::fs::create_dir_all(&temp_dir);
								let temp_path = temp_dir.join(&display_name);
								let path_to_copy = if std::fs::copy(&msg.media_path, &temp_path).is_ok() {
									temp_path.to_string_lossy().to_string()
								} else {
									msg.media_path.clone()
								};
								if crate::copy_files_to_clipboard(&[path_to_copy]) {
									let msg_kind = if is_voice { "voice message" } else { "file" };
									live_region::announce(
										msgs_announcer_copy.clone(),
										&format!("Copied {} to clipboard.", msg_kind),
									);
								} else {
									live_region::announce(
										msgs_announcer_copy.clone(),
										"Failed to copy file to clipboard.",
									);
								}
							} else {
								let _ =
									tx_copy.send(UiCommand::CopyAlbumToClipboard { message_ids: vec![msg.id.clone()] });
							}
						} else {
							if let Some(_locker) = Clipboard::get().locker() {
								if Clipboard::get().set_text(&msg.text) {
									live_region::announce(
										msgs_announcer_copy.clone(),
										"Copied message text to clipboard.",
									);
								}
							}
						}
					}
					whantv::DisplayItem::Album { messages, .. } => {
						let msg_ids: Vec<String> = messages.iter().map(|m| m.id.clone()).collect();
						if !msg_ids.is_empty() {
							let _ = tx_copy.send(UiCommand::CopyAlbumToClipboard { message_ids: msg_ids });
						}
					}
				}
			}
		}
	});

	let copy_for_ctx = perform_copy_selected.clone();
	let copy_for_key = perform_copy_selected.clone();

	let open_msg_context_menu = Rc::new(move || {
		if let Some(idx) = msgs_list_ctx.get_selection() {
			let reply_id = 3001;
			let edit_id = 3002;
			let delete_id = 3003;
			let info_id = 3004;
			let copy_all_id = 3005;
			let forward_id = 3006;
			let copy_id = 3007;
			let save_voice_id = 3008;
			let react_dialog_id = 3010;
			let retry_id = 3011;
			let star_id = 3012;
			let resend_id = 3013;
			let copy_number_id = 3014;
			let message_number_id = 3015;

			let s = state_ctx.borrow();
			let display_item = s.display_items.get(idx as usize).cloned();
			drop(s);

			let is_starred = display_item
				.as_ref()
				.map(|item| match item {
					whantv::DisplayItem::Single(m) => m.starred && !m.is_date_divider,
					whantv::DisplayItem::Album { .. } => false,
				})
				.unwrap_or(false);

			let is_from_me = display_item
				.as_ref()
				.map(|item| match item {
					whantv::DisplayItem::Single(m) => m.from_me && !m.is_date_divider,
					whantv::DisplayItem::Album { messages, .. } => messages.first().map(|m| m.from_me).unwrap_or(false),
				})
				.unwrap_or(false);
			let is_photo_album = display_item
				.as_ref()
				.map(|item| match item {
					whantv::DisplayItem::Album { messages, .. } => messages.iter().all(|m| m.media_type == "image"),
					_ => false,
				})
				.unwrap_or(false);
			let is_voice_msg = display_item
				.as_ref()
				.map(|item| match item {
					whantv::DisplayItem::Single(m) => {
						m.is_audio
							|| m.media_type == "voice"
							|| m.media_type == "audio"
							|| m.text.starts_with("[Voice Message]")
							|| m.text.starts_with("[Audio]")
					}
					_ => false,
				})
				.unwrap_or(false);
			let can_retry = display_item
				.as_ref()
				.map(|item| match item {
					whantv::DisplayItem::Single(m) => {
						m.from_me && !m.is_date_divider && (m.status == "pending" || m.status == "failed")
					}
					whantv::DisplayItem::Album { messages, .. } => messages
						.first()
						.map(|m| m.from_me && (m.status == "pending" || m.status == "failed"))
						.unwrap_or(false),
				})
				.unwrap_or(false);

			let is_unreadable = display_item
				.as_ref()
				.map(|item| match item {
					whantv::DisplayItem::Single(m) => m.is_unreadable(),
					_ => false,
				})
				.unwrap_or(false);

			let phone_match = display_item.as_ref().and_then(|item| match item {
				whantv::DisplayItem::Single(m) => m.first_phone_number(),
				whantv::DisplayItem::Album { .. } => None,
			});

			let mut menu_builder = Menu::builder();
			menu_builder = menu_builder
				.append_item(copy_id, "&Copy\tCtrl+C", "Copy message text or file to clipboard")
				.append_item(reply_id, "&Reply", "Reply to message")
				.append_item(forward_id, "&Forward...", "Forward message to another contact")
				.append_item(edit_id, "&Edit", "Edit message")
				.append_item(delete_id, "&Delete", "Delete message")
				.append_item(react_dialog_id, "&React...", "React to message");
			let is_single_message =
				matches!(display_item, Some(whantv::DisplayItem::Single(ref m)) if !m.is_date_divider);
			if is_single_message {
				let star_label = if is_starred { "&Unstar" } else { "&Star" };
				menu_builder = menu_builder.append_item(star_id, star_label, "Star or unstar this message");
			}
			if can_retry {
				menu_builder = menu_builder.append_item(retry_id, "Retry &send", "Retry sending this message");
			}
			if is_unreadable {
				menu_builder =
					menu_builder.append_item(resend_id, "Re&quest again", "Ask your phone to send this message again");
			}
			if is_voice_msg {
				menu_builder =
					menu_builder.append_item(save_voice_id, "&Save voice message...", "Save voice message to device");
			}
			if is_from_me {
				menu_builder = menu_builder.append_item(info_id, "Message &Info", "View message delivery info");
			}
			if is_photo_album {
				menu_builder =
					menu_builder.append_item(copy_all_id, "Copy all p&hotos", "Copy all photos to clipboard");
			}
			if phone_match.is_some() {
				menu_builder = menu_builder
					.append_item(copy_number_id, "Copy Nu&mber", "Copy phone number to clipboard")
					.append_item(message_number_id, "Message Num&ber", "Start a chat with this phone number");
			}
			let mut popup_menu = menu_builder.build();

			let state_clone = state_ctx.clone();
			let parent_frame_clone = parent_frame_ctx.clone();
			let msgs_announcer_clone = msgs_announcer_ctx.clone();
			let msg_input_field_clone = msg_input_field_ctx.clone();
			let tx_play_media_clone = tx_play_media_ctx.clone();
			let phone_match_clone = phone_match.clone();
			let copy_for_menu = copy_for_ctx.clone();

			popup_menu.on_selected(move |menu_evt| {
				let id = menu_evt.get_id();
				if id == reply_id {
					let mut s = state_clone.borrow_mut();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whantv::DisplayItem::Single(m) => Some(m.clone()),
						whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(),
					});
					if let Some(msg) = msg_opt {
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
							s.pending_reply_sender =
								Some(if msg.from_me { "me".to_string() } else { msg.sender_jid.clone() });
							s.pending_reply_preview = Some(format!("Replying to {}: {}", sender, preview));

							let status_msg =
								format!("Replying to {}: {}. Type your reply and press Enter.", sender, preview);
							live_region::announce(msgs_announcer_clone.clone(), &status_msg);
							msg_input_field_clone.set_focus();
							remove_quote_btn_ctx.show(true);
							conv_panel_ctx.layout();
							unsafe {
								let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
									windows::Win32::UI::WindowsAndMessaging::MB_OK,
								);
							}
						}
					}
				} else if id == forward_id {
					let s = state_clone.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whantv::DisplayItem::Single(m) => Some(m.clone()),
						whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(),
					});
					if let Some(msg) = msg_opt {
						if !msg.is_date_divider {
							let mut labels = Vec::new();
							let mut jids = Vec::new();
							for jid in &s.chats {
								if !jid.ends_with("@broadcast") {
									let name = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.clone());
									labels.push(name);
									jids.push(jid.clone());
								}
							}
							for (jid, name) in &s.contacts {
								if !jid.ends_with("@broadcast") && !jids.contains(jid) {
									labels.push(name.clone());
									jids.push(jid.clone());
								}
							}
							drop(s);

							if !jids.is_empty() {
								let dlg = Dialog::builder(&parent_frame_clone, "Forward Message")
									.with_size(400, 450)
									.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
									.build();
								let dlg_panel = Panel::builder(&dlg).build();
								let dlg_sizer = BoxSizer::builder(Orientation::Vertical).build();

								let search_label =
									StaticText::builder(&dlg_panel).with_label("Search contacts or chats:").build();
								dlg_sizer.add(&search_label, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Right, 8);

								let search_ctrl = TextCtrl::builder(&dlg_panel).build();
								dlg_sizer.add(&search_ctrl, 0, SizerFlag::Expand | SizerFlag::All, 8);

								let contacts_listbox =
									ListBox::builder(&dlg_panel).with_choices(labels.clone()).build();
								dlg_sizer.add(
									&contacts_listbox,
									1,
									SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right,
									8,
								);

								let btn_panel = Panel::builder(&dlg_panel).build();
								let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
								let ok_btn = Button::builder(&btn_panel)
									.with_id(wxdragon::id::ID_OK)
									.with_label("Forward")
									.build();
								let cancel_btn = Button::builder(&btn_panel)
									.with_id(wxdragon::id::ID_CANCEL)
									.with_label("Cancel")
									.build();
								btn_sizer.add(&ok_btn, 0, SizerFlag::Right, 5);
								btn_sizer.add(&cancel_btn, 0, SizerFlag::empty(), 0);
								btn_panel.set_sizer(btn_sizer, true);

								dlg_sizer.add(&btn_panel, 0, SizerFlag::AlignRight | SizerFlag::All, 8);
								dlg_panel.set_sizer(dlg_sizer, true);

								let dlg_esc_search = dlg.clone();
								search_ctrl.on_key_down(move |event| {
									if let WindowEventData::Keyboard(ref key) = event {
										if key.get_key_code() == Some(27) {
											dlg_esc_search.end_modal(wxdragon::id::ID_CANCEL);
											event.skip(false);
											return;
										}
									}
									event.skip(true);
								});

								let dlg_esc_list = dlg.clone();
								contacts_listbox.on_key_down(move |event| {
									if let WindowEventData::Keyboard(ref key) = event {
										if key.get_key_code() == Some(27) {
											dlg_esc_list.end_modal(wxdragon::id::ID_CANCEL);
											event.skip(false);
											return;
										}
									}
									event.skip(true);
								});

								let current_labels = std::rc::Rc::new(std::cell::RefCell::new(labels.clone()));
								let current_jids = std::rc::Rc::new(std::cell::RefCell::new(jids.clone()));

								let all_labels = labels;
								let all_jids = jids;
								let current_labels_search = current_labels.clone();
								let current_jids_search = current_jids.clone();
								let listbox_search = contacts_listbox;

								search_ctrl.on_text_changed(move |_| {
									let q = search_ctrl.get_value().to_lowercase().trim().to_string();
									let mut fl = Vec::new();
									let mut fj = Vec::new();
									for (lbl, jid) in all_labels.iter().zip(all_jids.iter()) {
										if q.is_empty() || lbl.to_lowercase().contains(&q) {
											fl.push(lbl.clone());
											fj.push(jid.clone());
										}
									}
									listbox_search.clear();
									for l in &fl {
										listbox_search.append(l);
									}
									*current_labels_search.borrow_mut() = fl;
									*current_jids_search.borrow_mut() = fj;
								});

								let dlg_cancel = dlg.clone();
								cancel_btn.on_click(move |_| {
									dlg_cancel.end_modal(0);
								});

								let dlg_ok = dlg.clone();
								let tx_fwd = tx_play_media_clone.clone();
								let msg_id = msg.id.clone();
								let current_jids_ok = current_jids;
								ok_btn.on_click(move |_| {
									if let Some(sel_idx) = contacts_listbox.get_selection() {
										let cj = current_jids_ok.borrow();
										if (sel_idx as usize) < cj.len() {
											let target_jid = cj[sel_idx as usize].clone();
											dlg_ok.end_modal(wxdragon::id::ID_OK);
											let _ = tx_fwd.send(UiCommand::ForwardMessage {
												message_id: msg_id.clone(),
												to_jids: vec![target_jid],
											});
										}
									}
								});

								search_ctrl.set_focus();
								dlg.show_modal();
							}
						}
					}
				} else if id == edit_id {
					let s = state_clone.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whantv::DisplayItem::Single(m) => Some(m.clone()),
						whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(),
					});
					drop(s);
					if let Some(msg) = msg_opt {
						if !msg.is_date_divider {
							let can_edit = msg.from_me
								&& !msg.is_deleted && !msg.is_audio
								&& !msg.text.starts_with("[Voice Message]")
								&& !msg.text.starts_with("[Audio]");
							if !can_edit {
								unsafe {
									let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
										windows::Win32::UI::WindowsAndMessaging::MB_ICONHAND,
									);
								}
								live_region::announce(msgs_announcer_clone.clone(), "Message can't be edited.");
							} else {
								let dlg = TextEntryDialog::builder(
									&parent_frame_clone,
									"Enter updated message text:",
									"Edit Message",
								)
								.with_default_value(&msg.text)
								.build();
								if dlg.show_modal() == wxdragon::id::ID_OK {
									if let Some(val) = dlg.get_value() {
										let new_text = val.trim().to_string();
										if !new_text.is_empty() && new_text != msg.text {
											let _ = tx_play_media_clone
												.send(UiCommand::EditMessage { message_id: msg.id.clone(), new_text });
										}
									}
								}
							}
						}
					}
				} else if id == delete_id {
					let s = state_clone.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whantv::DisplayItem::Single(m) => Some(m.clone()),
						whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(),
					});
					drop(s);
					if let Some(msg) = msg_opt {
						if !msg.is_date_divider {
							if let Some(for_everyone) = confirm_delete_choice(&parent_frame_clone, 1, msg.from_me) {
								let sender_jid = if msg.from_me { "me".to_string() } else { msg.sender_jid.clone() };
								let _ = tx_play_media_clone.send(UiCommand::DeleteMessage {
									message_id: msg.id.clone(),
									sender_jid,
									for_everyone,
								});
							}
						}
					}
				} else if id == star_id {
					let s = state_clone.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whantv::DisplayItem::Single(m) => Some(m.clone()),
						whantv::DisplayItem::Album { .. } => None,
					});
					drop(s);
					if let Some(msg) = msg_opt {
						if !msg.is_date_divider {
							let sender_jid = if msg.from_me { "me".to_string() } else { msg.sender_jid.clone() };
							let _ = tx_play_media_clone.send(UiCommand::StarMessage {
								message_id: msg.id.clone(),
								sender_jid,
								from_me: msg.from_me,
								starred: !msg.starred,
							});
						}
					}
				} else if id == info_id {
					let s = state_clone.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whantv::DisplayItem::Single(m) => Some(m.clone()),
						whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(),
					});
					if let Some(msg) = msg_opt {
						if msg.from_me && !msg.is_date_divider {
							let _ = tx_play_media_clone.send(UiCommand::GetMessageInfo { message_id: msg.id.clone() });
						}
					}
				} else if id == copy_id {
					copy_for_menu.as_ref()();
				} else if id == retry_id {
					let s = state_clone.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whantv::DisplayItem::Single(m) => Some(m.clone()),
						whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(),
					});
					drop(s);
					if let Some(msg) = msg_opt {
						let _ = tx_play_media_clone.send(UiCommand::RetryMessage { message_id: msg.id.clone() });
					}
				} else if id == resend_id {
					let s = state_clone.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whantv::DisplayItem::Single(m) => Some(m.clone()),
						_ => None,
					});
					drop(s);
					if let Some(msg) = msg_opt {
						let _ =
							tx_play_media_clone.send(UiCommand::RequestMessageResend { message_id: msg.id.clone() });
					}
				} else if id == save_voice_id {
					let _ = tx_play_media_clone.send(UiCommand::SaveMediaAs);
				} else if id == copy_all_id {
					let s = state_clone.borrow();
					if let Some(whantv::DisplayItem::Album { messages, .. }) = s.display_items.get(idx as usize) {
						let msg_ids: Vec<String> = messages.iter().map(|m| m.id.clone()).collect();
						if !msg_ids.is_empty() {
							let _ = tx_play_media_clone.send(UiCommand::CopyAlbumToClipboard { message_ids: msg_ids });
						}
					}
				} else if id == copy_number_id {
					if let Some((_, digits)) = phone_match_clone.as_ref() {
						if let Some(_locker) = Clipboard::get().locker() {
							if Clipboard::get().set_text(&format!("+{}", digits)) {
								live_region::announce(
									msgs_announcer_clone.clone(),
									"Copied phone number to clipboard.",
								);
							}
						}
					}
				} else if id == message_number_id {
					if let Some((_, digits)) = phone_match_clone.as_ref() {
						let _ = tx_play_media_clone.send(UiCommand::OpenChatByPhone(digits.clone()));
					}
				} else if id == react_dialog_id {
					let s = state_clone.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whantv::DisplayItem::Single(m) => Some(m.clone()),
						whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(),
					});
					drop(s);
					if let Some(msg) = msg_opt {
						let dlg = Dialog::builder(&parent_frame_clone, "React to message").with_size(300, 400).build();
						let dlg_panel = Panel::builder(&dlg).build();
						let dlg_sizer = BoxSizer::builder(Orientation::Vertical).build();

						let hint_label = StaticText::builder(&dlg_panel).with_label("Select a reaction emoji:").build();
						dlg_sizer.add(&hint_label, 0, SizerFlag::All, 8);

						let emojis = vec![
							"👍".to_string(),
							"❤️".to_string(),
							"😂".to_string(),
							"😮".to_string(),
							"😢".to_string(),
							"🙏".to_string(),
							"🔥".to_string(),
							"🎉".to_string(),
							"💯".to_string(),
							"😎".to_string(),
							"🥰".to_string(),
							"Other emoji...".to_string(),
							"Remove reaction".to_string(),
						];

						let emoji_list = ListBox::builder(&dlg_panel).with_choices(emojis.clone()).build();
						dlg_sizer.add(&emoji_list, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 8);

						dlg_panel.set_sizer(dlg_sizer, true);

						let dlg_ref = dlg.clone();
						let tx_react = tx_play_media_clone.clone();
						let msg_id = msg.id.clone();
						let sender_jid = if msg.from_me { "me".to_string() } else { msg.sender_jid.clone() };
						let parent_ref = parent_frame_clone.clone();

						let on_select = Rc::new(move |choice_idx: usize| {
							dlg_ref.end_modal(wxdragon::id::ID_OK);
							match choice_idx {
								0..=10 => {
									let selected = emojis[choice_idx].clone();
									let _ = tx_react.send(UiCommand::ReactToMessage {
										message_id: msg_id.clone(),
										sender_jid: sender_jid.clone(),
										emoji: selected,
									});
								}
								11 => {
									let custom_dlg = TextEntryDialog::builder(
										&parent_ref,
										"Enter or paste an emoji:",
										"React with Emoji",
									)
									.build();
									if custom_dlg.show_modal() == wxdragon::id::ID_OK {
										if let Some(val) = custom_dlg.get_value() {
											let emoji = val.trim().to_string();
											if !emoji.is_empty() {
												let _ = tx_react.send(UiCommand::ReactToMessage {
													message_id: msg_id.clone(),
													sender_jid: sender_jid.clone(),
													emoji,
												});
											}
										}
									}
								}
								12 => {
									let _ = tx_react.send(UiCommand::ReactToMessage {
										message_id: msg_id.clone(),
										sender_jid: sender_jid.clone(),
										emoji: String::new(),
									});
								}
								_ => {}
							}
						});

						let on_select_dbl = on_select.clone();
						emoji_list.on_item_double_clicked(move |event| {
							if let Some(sel) = event.get_selection() {
								on_select_dbl(sel as usize);
							}
						});

						let on_select_key = on_select.clone();
						let list_ref = emoji_list.clone();
						let dlg_esc_down = dlg.clone();
						emoji_list.on_key_down(move |event| {
							if let WindowEventData::Keyboard(ref key) = event {
								let code = key.get_key_code();
								if code == Some(13) || code == Some(370) {
									if let Some(sel) = list_ref.get_selection() {
										on_select_key(sel as usize);
										event.skip(false);
										return;
									}
								} else if code == Some(27) {
									dlg_esc_down.end_modal(wxdragon::id::ID_CANCEL);
									event.skip(false);
									return;
								}
							}
							event.skip(true);
						});

						let on_select_enter = on_select.clone();
						let list_ref_up = emoji_list.clone();
						emoji_list.on_key_up(move |event| {
							if let WindowEventData::Keyboard(ref key) = event {
								let code = key.get_key_code();
								if code == Some(13) || code == Some(370) {
									if let Some(sel) = list_ref_up.get_selection() {
										on_select_enter(sel as usize);
										event.skip(false);
										return;
									}
								}
							}
							event.skip(true);
						});

						emoji_list.set_focus();
						dlg.show_modal();
					}
				}
			});

			msgs_list_ctx.popup_menu(&mut popup_menu, None);
			popup_menu.destroy_menu();
		}
	});

	let open_menu_key = open_msg_context_menu.clone();
	parts.conversations_parts.messages_list.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key_event) = event {
			let code = key_event.get_key_code();

			if code == Some(314) || code == Some(37) {
				// Left Arrow
				let now = std::time::Instant::now();
				let prev = last_left_arrow.get();
				if now.duration_since(prev) < std::time::Duration::from_millis(600) {
					last_left_arrow.set(now - std::time::Duration::from_secs(10));
					// If search is active, double-left-arrow behaves like normal chats:
					// if the message is a reply, jump to the quoted message; otherwise just close search
					if search_panel_key.is_shown() {
						if let Some(idx) = msgs_list_play.get_selection() {
							let s = state_play.borrow();
							let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
								whantv::DisplayItem::Single(m) => Some(m.clone()),
								whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(),
							});
							if let Some(msg) = msg_opt {
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
						let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
							whantv::DisplayItem::Single(m) => Some(m.clone()),
							whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(),
						});
						if let Some(msg) = msg_opt {
							if !msg.quoted_id.is_empty() || !msg.quoted_text.is_empty() {
								let mut target = if !msg.quoted_id.is_empty() {
									s.display_items.iter().position(|item| match item {
										whantv::DisplayItem::Single(m) => m.id == msg.quoted_id,
										whantv::DisplayItem::Album { messages, .. } => {
											messages.iter().any(|m| m.id == msg.quoted_id)
										}
									})
								} else {
									None
								};

								if target.is_none() && !msg.quoted_text.is_empty() {
									let is_generic = msg.quoted_text == "[Voice Message]"
										|| msg.quoted_text == "[Audio]"
										|| msg.quoted_text == "Voice message"
										|| msg.quoted_text == "Audio" || msg.quoted_text == "[Media]";
									if !is_generic && (idx as usize) < s.display_items.len() {
										target = s.display_items[..idx as usize].iter().rposition(|item| match item {
											whantv::DisplayItem::Single(m) => {
												m.text == msg.quoted_text
													|| (!m.text.is_empty() && m.text.contains(&msg.quoted_text))
											}
											whantv::DisplayItem::Album { messages, .. } => messages.iter().any(|m| {
												m.text == msg.quoted_text
													|| (!m.text.is_empty() && m.text.contains(&msg.quoted_text))
											}),
										});
									}
								}

								if let Some(target_idx) = target {
									msgs_list_play.set_selection(target_idx as u32, true);
									unsafe {
										let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
											windows::Win32::UI::WindowsAndMessaging::MB_OK,
										);
									}
								} else {
									unsafe {
										let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
											windows::Win32::UI::WindowsAndMessaging::MB_ICONASTERISK,
										);
									}
								}
							}
						}
					}
					event.skip(false);
				} else {
					last_left_arrow.set(now);
					if state_play.borrow().playing_msg_id.is_some() {
						let _ = tx_close_list.send(UiCommand::SeekAudio(-5.0));
					}
					event.skip(false);
				}
			} else if code == Some(316) || code == Some(39) {
				if state_play.borrow().playing_msg_id.is_some() {
					let _ = tx_close_list.send(UiCommand::SeekAudio(5.0));
					event.skip(false);
				} else {
					event.skip(true);
				}
			} else if code == Some('j' as i32) || code == Some('J' as i32) || code == Some(74) || code == Some(106) {
				if state_play.borrow().playing_msg_id.is_some() {
					let _ = tx_close_list.send(UiCommand::SeekAudio(-5.0));
					event.skip(false);
				} else {
					event.skip(true);
				}
			} else if code == Some('l' as i32) || code == Some('L' as i32) || code == Some(76) || code == Some(108) {
				if state_play.borrow().playing_msg_id.is_some() {
					let _ = tx_close_list.send(UiCommand::SeekAudio(5.0));
					event.skip(false);
				} else {
					event.skip(true);
				}
			} else if key_event.get_key_code() == Some(27) {
				// Escape
				let _ = tx_close_list.send(UiCommand::CloseConversation);
				event.skip(false);
			} else if key_event.get_key_code() == Some(313) || key_event.get_key_code() == Some(384) {
				// WXK_HOME (313) -> Oldest message at top
				let count = msgs_list_play.get_count();
				if count > 0 {
					msgs_list_play.set_selection(0, true);
				}
				event.skip(false);
			} else if key_event.get_key_code() == Some(312) || key_event.get_key_code() == Some(385) {
				// WXK_END (312) -> Newest message at bottom
				let count = msgs_list_play.get_count();
				if count > 0 {
					msgs_list_play.set_selection((count - 1) as u32, true);
				}
				event.skip(false);
			} else if (code == Some(67) || code == Some('c' as i32) || code == Some('C' as i32))
				&& key_event.control_down()
			{
				copy_for_key.as_ref()();
				event.skip(false);
			} else if key_event.get_key_code() == Some(86) && key_event.control_down() {
				// Ctrl+V
				let mut has_files = false;
				if let Some(_locker) = Clipboard::get().locker() {
					let mut fdo = FileDataObject::new();
					if Clipboard::get().get_data(&mut fdo) && fdo.get_file_count() > 0 {
						has_files = true;
					}
				}
				if has_files {
					let _ = tx_close_list.send(UiCommand::PasteAttachment);
					event.skip(false);
				} else {
					event.skip(true);
				}
			} else if key_event.get_key_code() == Some(13) {
				// Enter key
				event.skip(true);
			} else if (key_event.get_key_code() == Some(32) || key_event.get_key_code() == Some(' ' as i32))
				&& key_event.control_down()
			{
				// Ctrl+Space: enter/exit selection mode.
				let _ = tx_close_list.send(UiCommand::ToggleSelectionMode);
				event.skip(false);
			} else if key_event.get_key_code() == Some(32) || key_event.get_key_code() == Some(' ' as i32) {
				// Space
				use std::io::Write;
				if let Some(idx) = msgs_list_play.get_selection() {
					let mut s = state_play.borrow_mut();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whantv::DisplayItem::Single(m) => Some(m.clone()),
						whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(),
					});
					if let Some(msg) = msg_opt {
						let is_voice =
							msg.is_audio || msg.text.starts_with("[Voice Message]") || msg.text.starts_with("[Audio]");
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
			} else if code == Some('r' as i32) || code == Some('R' as i32) || code == Some(82) {
				// 'r' / 'R' -> Reply
				if let Some(idx) = msgs_list_play.get_selection() {
					let mut s = state_play.borrow_mut();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whantv::DisplayItem::Single(m) => Some(m.clone()),
						whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(),
					});
					if let Some(msg) = msg_opt {
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
							s.pending_reply_sender =
								Some(if msg.from_me { "me".to_string() } else { msg.sender_jid.clone() });
							s.pending_reply_preview = Some(format!("Replying to {}: {}", sender, preview));

							let status_msg =
								format!("Replying to {}: {}. Type your reply and press Enter.", sender, preview);
							live_region::announce(msgs_announcer.clone(), &status_msg);
							msg_input_field.set_focus();
							remove_quote_btn_play.show(true);
							conv_panel_play.layout();
							unsafe {
								let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
									windows::Win32::UI::WindowsAndMessaging::MB_OK,
								);
							}
						}
					}
				}
				event.skip(false);
			} else if code == Some('e' as i32) || code == Some('E' as i32) || code == Some(69) {
				// 'e' / 'E' -> Edit
				if let Some(idx) = msgs_list_play.get_selection() {
					let s = state_play.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whantv::DisplayItem::Single(m) => Some(m.clone()),
						whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(),
					});
					drop(s);
					if let Some(msg) = msg_opt {
						if !msg.is_date_divider {
							let can_edit = msg.from_me
								&& !msg.is_deleted && !msg.is_audio
								&& !msg.text.starts_with("[Voice Message]")
								&& !msg.text.starts_with("[Audio]");
							if !can_edit {
								unsafe {
									let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
										windows::Win32::UI::WindowsAndMessaging::MB_ICONHAND,
									);
								}
								live_region::announce(msgs_announcer.clone(), "Message can't be edited.");
							} else {
								let dlg = TextEntryDialog::builder(
									&parent_frame,
									"Enter updated message text:",
									"Edit Message",
								)
								.with_default_value(&msg.text)
								.build();
								if dlg.show_modal() == wxdragon::id::ID_OK {
									if let Some(val) = dlg.get_value() {
										let new_text = val.trim().to_string();
										if !new_text.is_empty() && new_text != msg.text {
											let _ = tx_play_media
												.send(UiCommand::EditMessage { message_id: msg.id.clone(), new_text });
										}
									}
								}
							}
						}
					}
				}
				event.skip(false);
			} else if code == Some('s' as i32) || code == Some('S' as i32) || code == Some(83) || code == Some(115) {
				// 's' / 'S' -> Star/Unstar
				if let Some(idx) = msgs_list_play.get_selection() {
					let s = state_play.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whantv::DisplayItem::Single(m) => Some(m.clone()),
						whantv::DisplayItem::Album { .. } => None,
					});
					drop(s);
					if let Some(msg) = msg_opt {
						if !msg.is_date_divider {
							let sender_jid = if msg.from_me { "me".to_string() } else { msg.sender_jid.clone() };
							let _ = tx_play_media.send(UiCommand::StarMessage {
								message_id: msg.id.clone(),
								sender_jid,
								from_me: msg.from_me,
								starred: !msg.starred,
							});
						}
					}
				}
				event.skip(false);
			} else if code == Some('f' as i32) || code == Some('F' as i32) || code == Some(70) || code == Some(102) {
				// 'f' / 'F' -> Forward
				if let Some(idx) = msgs_list_play.get_selection() {
					let s = state_play.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whantv::DisplayItem::Single(m) => Some(m.clone()),
						whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(),
					});
					if let Some(msg) = msg_opt {
						if !msg.is_date_divider {
							let mut labels = Vec::new();
							let mut jids = Vec::new();
							for jid in &s.chats {
								if !jid.ends_with("@broadcast") {
									let name = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.clone());
									labels.push(name);
									jids.push(jid.clone());
								}
							}
							for (jid, name) in &s.contacts {
								if !jid.ends_with("@broadcast") && !jids.contains(jid) {
									labels.push(name.clone());
									jids.push(jid.clone());
								}
							}
							drop(s);

							if !jids.is_empty() {
								let dlg = Dialog::builder(&parent_frame, "Forward Message")
									.with_size(400, 450)
									.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
									.build();
								let dlg_panel = Panel::builder(&dlg).build();
								let dlg_sizer = BoxSizer::builder(Orientation::Vertical).build();

								let search_label =
									StaticText::builder(&dlg_panel).with_label("Search contacts or chats:").build();
								dlg_sizer.add(&search_label, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Right, 8);

								let search_ctrl = TextCtrl::builder(&dlg_panel).build();
								dlg_sizer.add(&search_ctrl, 0, SizerFlag::Expand | SizerFlag::All, 8);

								let contacts_listbox =
									ListBox::builder(&dlg_panel).with_choices(labels.clone()).build();
								dlg_sizer.add(
									&contacts_listbox,
									1,
									SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right,
									8,
								);

								let btn_panel = Panel::builder(&dlg_panel).build();
								let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
								let ok_btn = Button::builder(&btn_panel)
									.with_id(wxdragon::id::ID_OK)
									.with_label("Forward")
									.build();
								let cancel_btn = Button::builder(&btn_panel)
									.with_id(wxdragon::id::ID_CANCEL)
									.with_label("Cancel")
									.build();
								btn_sizer.add(&ok_btn, 0, SizerFlag::Right, 5);
								btn_sizer.add(&cancel_btn, 0, SizerFlag::empty(), 0);
								btn_panel.set_sizer(btn_sizer, true);

								dlg_sizer.add(&btn_panel, 0, SizerFlag::AlignRight | SizerFlag::All, 8);
								dlg_panel.set_sizer(dlg_sizer, true);

								let dlg_esc_search = dlg.clone();
								search_ctrl.on_key_down(move |event| {
									if let WindowEventData::Keyboard(ref key) = event {
										if key.get_key_code() == Some(27) {
											dlg_esc_search.end_modal(wxdragon::id::ID_CANCEL);
											event.skip(false);
											return;
										}
									}
									event.skip(true);
								});

								let dlg_esc_list = dlg.clone();
								contacts_listbox.on_key_down(move |event| {
									if let WindowEventData::Keyboard(ref key) = event {
										if key.get_key_code() == Some(27) {
											dlg_esc_list.end_modal(wxdragon::id::ID_CANCEL);
											event.skip(false);
											return;
										}
									}
									event.skip(true);
								});

								let current_labels = std::rc::Rc::new(std::cell::RefCell::new(labels.clone()));
								let current_jids = std::rc::Rc::new(std::cell::RefCell::new(jids.clone()));

								let all_labels = labels;
								let all_jids = jids;
								let current_labels_search = current_labels.clone();
								let current_jids_search = current_jids.clone();
								let listbox_search = contacts_listbox;

								search_ctrl.on_text_changed(move |_| {
									let q = search_ctrl.get_value().to_lowercase().trim().to_string();
									let mut fl = Vec::new();
									let mut fj = Vec::new();
									for (lbl, jid) in all_labels.iter().zip(all_jids.iter()) {
										if q.is_empty() || lbl.to_lowercase().contains(&q) {
											fl.push(lbl.clone());
											fj.push(jid.clone());
										}
									}
									listbox_search.clear();
									for l in &fl {
										listbox_search.append(l);
									}
									*current_labels_search.borrow_mut() = fl;
									*current_jids_search.borrow_mut() = fj;
								});

								let dlg_cancel = dlg.clone();
								cancel_btn.on_click(move |_| {
									dlg_cancel.end_modal(0);
								});

								let dlg_ok = dlg.clone();
								let tx_fwd = tx_close_list.clone();
								let msg_id = msg.id.clone();
								let current_jids_ok = current_jids;
								ok_btn.on_click(move |_| {
									if let Some(sel_idx) = contacts_listbox.get_selection() {
										let cj = current_jids_ok.borrow();
										if (sel_idx as usize) < cj.len() {
											let target_jid = cj[sel_idx as usize].clone();
											dlg_ok.end_modal(wxdragon::id::ID_OK);
											let _ = tx_fwd.send(UiCommand::ForwardMessage {
												message_id: msg_id.clone(),
												to_jids: vec![target_jid],
											});
										}
									}
								});

								search_ctrl.set_focus();
								dlg.show_modal();
							}
						}
					}
				}
				event.skip(false);
			} else if code == Some(127) || code == Some(390) {
				// Delete key -> Delete
				if let Some(idx) = msgs_list_play.get_selection() {
					let s = state_play.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whantv::DisplayItem::Single(m) => Some(m.clone()),
						whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(),
					});
					drop(s);
					if let Some(msg) = msg_opt {
						if !msg.is_date_divider {
							if let Some(for_everyone) = confirm_delete_choice(&parent_frame, 1, msg.from_me) {
								let sender_jid = if msg.from_me { "me".to_string() } else { msg.sender_jid.clone() };
								let _ = tx_play_media.send(UiCommand::DeleteMessage {
									message_id: msg.id.clone(),
									sender_jid,
									for_everyone,
								});
							}
						}
					}
				}
			} else if !key_event.meta_down()
				&& (code == Some(395) || code == Some(93) || (key_event.shift_down() && code == Some(349)))
			{
				// Context Menu key or Shift+F10
				open_menu_key.as_ref()();
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
	let tx_phone_enter = ui_tx.clone();
	let msgs_list_enter = parts.conversations_parts.messages_list.clone();
	let state_enter = state.clone();
	let search_panel_enter = parts.conversations_parts.search_panel;
	let msgs_announcer_enter = parts.conversations_parts.messages_announcer.clone();
	parts.conversations_parts.messages_list.on_key_up(move |event| {
		if let WindowEventData::Keyboard(ref key_event) = event {
			let code = key_event.get_key_code();
			if code == Some(13) || code == Some(370) {
				// Enter or numpad Enter
				if search_panel_enter.is_shown() {
					if let Some(idx) = msgs_list_enter.get_selection() {
						let s = state_enter.borrow();
						let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
							whantv::DisplayItem::Single(m) => Some(m.clone()),
							whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(),
						});
						if let Some(msg) = msg_opt {
							drop(s);
							if !msg.is_date_divider {
								let _ = tx_jump_enter.send(UiCommand::JumpToSearchResult {
									message_id: msg.id,
									message_text: String::new(),
								});
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
					if let Some(idx) = msgs_list_enter.get_selection() {
						let s = state_enter.borrow();
						let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
							whantv::DisplayItem::Single(m) => Some(m.clone()),
							whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(),
						});
						drop(s);
						if let Some(msg) = msg_opt {
							let mut urls = Vec::new();
							let text = &msg.text;
							let mut start_idx = 0;
							while let Some(idx) = text[start_idx..].find("http") {
								let actual_idx = start_idx + idx;
								if text[actual_idx..].starts_with("http://")
									|| text[actual_idx..].starts_with("https://")
								{
									let space_idx = text[actual_idx..]
										.find(|c: char| c.is_whitespace())
										.unwrap_or(text.len() - actual_idx);
									let mut clean_url = &text[actual_idx..actual_idx + space_idx];
									while clean_url.ends_with(|c: char| c.is_ascii_punctuation()) {
										clean_url = &clean_url[..clean_url.len() - 1];
									}
									urls.push(clean_url.to_string());
									start_idx = actual_idx + space_idx;
								} else {
									start_idx = actual_idx + 4;
								}
							}

							if urls.len() == 1 {
								wxdragon::utils::launch_default_browser(
									&urls[0],
									wxdragon::utils::BrowserLaunchFlags::Default,
								);
							} else if urls.len() > 1 {
								let url_strs: Vec<&str> = urls.iter().map(|s| s.as_str()).collect();
								let dialog = wxdragon::prelude::SingleChoiceDialog::builder(
									&msgs_list_enter,
									"Select a link to open:",
									"Open Link",
									&url_strs,
								)
								.build();
								if dialog.show_modal() == wxdragon::id::ID_OK {
									if let Some(selection) = dialog.get_string_selection() {
										wxdragon::utils::launch_default_browser(
											&selection,
											wxdragon::utils::BrowserLaunchFlags::Default,
										);
									}
								}
							} else if let Some((_, digits)) = msg.first_phone_number() {
								let copy_num_id = 9001;
								let msg_num_id = 9002;
								let mut phone_menu = Menu::builder()
									.append_item(copy_num_id, "&Copy Number", "Copy phone number to clipboard")
									.append_item(msg_num_id, "&Message Number", "Start a chat with this phone number")
									.build();
								let tx_phone = tx_phone_enter.clone();
								let msgs_announcer_phone = msgs_announcer_enter.clone();
								phone_menu.on_selected(move |menu_evt| {
									let sel_id = menu_evt.get_id();
									if sel_id == copy_num_id {
										if let Some(_locker) = Clipboard::get().locker() {
											if Clipboard::get().set_text(&format!("+{}", digits)) {
												live_region::announce(
													msgs_announcer_phone.clone(),
													"Copied phone number to clipboard.",
												);
											}
										}
									} else if sel_id == msg_num_id {
										let _ = tx_phone.send(UiCommand::OpenChatByPhone(digits.clone()));
									}
								});
								msgs_list_enter.popup_menu(&mut phone_menu, None);
								phone_menu.destroy_menu();
							}
						}
					}
					event.skip(false);
				}
			}
		}
	});

	let open_menu_cb = open_msg_context_menu.clone();
	parts.conversations_parts.messages_list.bind_internal(EventType::CONTEXT_MENU, move |_event| {
		open_menu_cb.as_ref()();
	});

	let tx_play_btn = ui_tx.clone();
	let msgs_list_btn = parts.conversations_parts.messages_list.clone();
	let state_play_btn = state.clone();
	let play_voice_btn_ref = parts.conversations_parts.play_voice_btn.clone();
	parts.conversations_parts.play_voice_btn.on_click(move |_| {
		use std::io::Write;
		if let Some(idx) = msgs_list_btn.get_selection() {
			let mut s = state_play_btn.borrow_mut();
			let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
				whantv::DisplayItem::Single(m) => Some(m.clone()),
				whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(),
			});
			if let Some(msg) = msg_opt {
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
						// Pause — save elapsed time (played back at the
						// current speed since the last seek/start, so the
						// wall-clock delta must be scaled the same way the
						// resume/tick/seek paths compute position)
						if let Some(inst) = s.audio_start_instant.take() {
							s.audio_elapsed_secs += inst.elapsed().as_secs_f32() * s.audio_speed;
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
	let tx_sel = ui_tx.clone();
	// The 100ms throttle below skips layout() on some events in a rapid
	// selection burst (e.g. holding an arrow key down through messages) so
	// the toolbar doesn't visibly freeze while it repaints on every row. But
	// if the burst's last event lands inside the throttle window, that final
	// show()/hide() never gets laid out - the toolbar is stuck showing the
	// previous row's buttons until some unrelated event happens to trigger
	// another layout. This one-shot timer, (re)armed on every skipped
	// layout, guarantees a trailing layout once the burst actually settles.
	let action_layout_timer = Timer::new(&parts.frame);
	let action_panel_for_timer = parts.conversations_parts.action_panel.clone();
	action_layout_timer.on_tick(move |_| {
		action_panel_for_timer.layout();
	});
	parts.conversations_parts.messages_list.on_selection_changed(move |event| {
		if let Some(idx) = event.get_selection() {
			if idx == 0 {
				let _ = tx_sel.send(UiCommand::LoadMoreMessages);
			}
			let s = state_sel.borrow();
			conv_parts_sel.action_panel.freeze();
			let mut need_layout = crate::update_action_buttons(&s, &conv_parts_sel);

			let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
				whantv::DisplayItem::Single(m) => Some(m.clone()),
				whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(),
			});
			if let Some(msg) = msg_opt {
				let is_voice = match s.display_items.get(idx as usize) {
					Some(whantv::DisplayItem::Single(m)) => {
						let is_deleted = m.is_deleted
							|| m.text == "[This message was deleted]"
							|| m.text == "[You deleted this message]";
						!is_deleted
							&& (m.is_audio || m.text.starts_with("[Voice Message]") || m.text.starts_with("[Audio]"))
					}
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
				if play_voice_btn_sel.is_shown() {
					play_voice_btn_sel.show(false);
					need_layout = true;
				}
				if audio_slider_sel.is_shown() {
					audio_slider_sel.show(false);
					audio_progress_label_sel.show(false);
					need_layout = true;
				}
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
				} else {
					action_layout_timer.start(120, true);
				}
			}
		} else {
			let s = state_sel.borrow();
			conv_parts_sel.action_panel.freeze();
			let mut need_layout = crate::update_action_buttons(&s, &conv_parts_sel);
			if play_voice_btn_sel.is_shown() {
				play_voice_btn_sel.show(false);
				need_layout = true;
			}
			if audio_slider_sel.is_shown() {
				audio_slider_sel.show(false);
				audio_progress_label_sel.show(false);
				need_layout = true;
			}

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
				} else {
					action_layout_timer.start(120, true);
				}
			}
		}
	});

	let tx_toggle_search = ui_tx.clone();
	parts.conversations_parts.search_in_conv_btn.on_click(move |_| {
		let _ = tx_toggle_search.send(UiCommand::ToggleSearchPanel);
	});

	let tx_cancel_selection = ui_tx.clone();
	parts.conversations_parts.selection_cancel_btn.on_click(move |_| {
		let _ = tx_cancel_selection.send(UiCommand::ToggleSelectionMode);
	});

	let selection_list_toggle = parts.conversations_parts.messages_select_list;
	let selection_count_label_toggle = parts.conversations_parts.selection_count_label;
	let state_selection_toggle = state.clone();
	selection_list_toggle.on_toggled(move |event| {
		if let Some(idx) = event.get_selection() {
			let mut s = state_selection_toggle.borrow_mut();
			if let Some(item) = s.selection_rows.get(idx as usize).cloned() {
				if selection_list_toggle.is_checked(idx) {
					s.selected_message_ids.insert(item.id().to_string());
				} else {
					s.selected_message_ids.remove(item.id());
				}
				let count = s.selected_message_ids.len();
				selection_count_label_toggle.set_label(&format!("{} selected", count));
			}
		}
	});

	// `messages_select_list` (not `messages_list`) has focus while selection
	// mode is active, so the Ctrl+Space/Escape handling on `messages_list`
	// above never fires here - bind the same exit shortcuts on it directly.
	let tx_selection_list_keys = ui_tx.clone();
	parts.conversations_parts.messages_select_list.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key_event) = event {
			let code = key_event.get_key_code();
			let ctrl_space = (code == Some(32) || code == Some(' ' as i32)) && key_event.control_down();
			if ctrl_space || code == Some(27) {
				let _ = tx_selection_list_keys.send(UiCommand::ToggleSelectionMode);
				event.skip(false);
				return;
			}
		}
		event.skip(true);
	});

	let tx_delete_selection = ui_tx.clone();
	let parent_frame_selection = parts.frame.clone();
	let state_delete_selection = state.clone();
	parts.conversations_parts.selection_delete_btn.on_click(move |_| {
		let (count, all_from_me) = {
			let s = state_delete_selection.borrow();
			let ids = &s.selected_message_ids;
			let msgs: Vec<whantv::ChatMessage> = s
				.selection_rows
				.iter()
				.filter(|item| ids.contains(item.id()))
				.flat_map(|item| item.messages())
				.collect();
			(msgs.len(), !msgs.is_empty() && msgs.iter().all(|m| m.from_me))
		};
		if count == 0 {
			return;
		}
		if let Some(for_everyone) = confirm_delete_choice(&parent_frame_selection, count, all_from_me) {
			let _ = tx_delete_selection.send(UiCommand::DeleteSelectedMessages { for_everyone });
		}
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
	let state_search_focus = state.clone();
	search_field_ref.on_text_enter(move |_| {
		msgs_list_search_focus.set_focus();
		let count = msgs_list_search_focus.get_count();
		if count > 0 {
			let s = state_search_focus.borrow();
			let mut selected_idx = 0;
			for (i, item) in s.display_items.iter().enumerate() {
				match item {
					whantv::DisplayItem::Single(m) => {
						if !m.is_date_divider {
							selected_idx = i as u32;
							break;
						}
					}
					whantv::DisplayItem::Album { .. } => {
						selected_idx = i as u32;
						break;
					}
				}
			}
			msgs_list_search_focus.set_selection(selected_idx, true);
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

	let tx_speed = ui_tx.clone();
	parts.conversations_parts.audio_speed_btn.on_click(move |_| {
		let _ = tx_speed.send(UiCommand::ToggleAudioSpeed);
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
			if code == Some(314) || code == Some(37) || code == Some(317) || code == Some(40) {
				let _ = tx_slider_key.send(UiCommand::SeekAudio(-5.0));
				event.skip(false);
			} else if code == Some(316) || code == Some(39) || code == Some(315) || code == Some(38) {
				let _ = tx_slider_key.send(UiCommand::SeekAudio(5.0));
				event.skip(false);
			} else if code == Some(312) || code == Some(385) {
				let _ = tx_slider_key.send(UiCommand::SeekAudio(5.0));
				event.skip(false);
			} else if code == Some(313) || code == Some(384) {
				let _ = tx_slider_key.send(UiCommand::SeekAudio(-5.0));
				event.skip(false);
			} else {
				event.skip(true);
			}
		} else {
			event.skip(true);
		}
	});
}
