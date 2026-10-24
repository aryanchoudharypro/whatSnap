use std::rc::Rc;

use wxdragon::{
	event::{EventType, WxEvtHandler},
	prelude::*,
};

use crate::{
	UiCommand,
	keymap::{default_menu_label, message_list},
	ui::{keys, theme},
	ui_wake::UiCommandSender,
};

/// Whether Enter (main or numpad) is physically down right now.
fn enter_key_held() -> bool {
	use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_RETURN};
	let state = unsafe { GetAsyncKeyState(i32::from(VK_RETURN.0)) };
	state as u16 & 0x8000 != 0
}

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
fn confirm_delete_choice(parent: &impl WxWidget, count: usize, can_delete_for_everyone: bool) -> Option<bool> {
	let plural = if count == 1 { "this message" } else { "these messages" };
	if can_delete_for_everyone {
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
	get_message: impl Fn(&crate::AppState, usize) -> Option<whatsnap::Msg> + Clone + 'static,
	with_media_actions: bool,
) {
	let list_enter = *list;
	let dialog_enter = *dialog;
	let tx_enter = ui_tx.clone();
	let state_enter = state.clone();
	let get_message_enter = get_message.clone();
	keys::on_enter(list, Rc::default(), move || {
		if let Some(idx) = list_enter.get_selection()
			&& let Some(msg) = get_message_enter(&state_enter.borrow(), idx as usize)
		{
			dialog_enter.end_modal(0);
			let _ = tx_enter
				.send(UiCommand::JumpToSearchResult { message_id: msg.id.clone(), message_text: String::new() });
		}
	});

	if with_media_actions {
		let list_ctx = *list;
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
	pub poll_dialog: crate::ui::poll_dialog::PollDialogParts,
	pub new_group_dialog: crate::ui::new_group::NewGroupDialogParts,
	pub add_members_dialog: crate::ui::add_members::AddMembersDialogParts,
	pub attach_contact_dialog: crate::ui::attach_contact::AttachContactDialogParts,
	/// Shared with the search-field debounce in `bind_input_handlers` so a
	/// `contacts` response landing while a picker is open coalesces with any
	/// in-flight keystroke rebuild instead of firing an immediate rebuild
	/// back-to-back with it - see the "bad wxCheckListBox index" comments in
	/// `bind_input_handlers` and in main.rs's `refresh_*_contacts_list`.
	pub new_group_contacts_timer: Rc<Timer<Dialog>>,
	pub add_members_contacts_timer: Rc<Timer<Dialog>>,
	pub attach_contact_timer: Rc<Timer<Dialog>>,
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
pub const MENU_ID_RECONNECT: i32 = 6003;
pub const MENU_ID_NAV_CONVERSATIONS: i32 = 6010;
pub const MENU_ID_NAV_STATUS: i32 = 6011;
pub const MENU_ID_NAV_COMMUNITIES: i32 = 6012;
pub const MENU_ID_NAV_STARRED: i32 = 6013;
pub const MENU_ID_NAV_ARCHIVED: i32 = 6014;
pub const MENU_ID_NAV_CHANNELS: i32 = 6015;
pub const MENU_ID_NAV_CALLS: i32 = 6016;
pub const MENU_ID_NEW_CHAT: i32 = 6020;
pub const MENU_ID_NEW_GROUP: i32 = 6021;
pub const MENU_ID_SEARCH_GLOBAL: i32 = 6022;
pub const MENU_ID_SEARCH_CHAT: i32 = 6023;
pub const MENU_ID_MUTE_CHAT: i32 = 6024;
pub const MENU_ID_ARCHIVE_CHAT: i32 = 6025;
pub const MENU_ID_PIN_CHAT: i32 = 6026;
pub const MENU_ID_MARK_UNREAD: i32 = 6027;
pub const MENU_ID_CHAT_INFO: i32 = 6028;
pub const MENU_ID_STAR_MESSAGE: i32 = 6029;
pub const MENU_ID_START_RECORDING: i32 = 6030;
pub const MENU_ID_PAUSE_RECORDING: i32 = 6031;
pub const MENU_ID_SEND_PTT: i32 = 6032;
pub const MENU_ID_CALL_MUTE: i32 = 6033;
pub const MENU_ID_CALL_END: i32 = 6034;
pub const MENU_ID_LOCK_CHAT: i32 = 6035;
pub const MENU_ID_DISAPPEARING: i32 = 6036;
pub const MENU_ID_REPORT_CHAT: i32 = 6037;
pub const MENU_ID_BLOCK_CHAT: i32 = 6038;
pub const MENU_ID_BLOCKED_CONTACTS: i32 = 6039;
pub const MENU_ID_PINNED_MESSAGES: i32 = 6040;
pub const MENU_ID_NEW_COMMUNITY: i32 = 6042;
pub const MENU_ID_MANAGE_LISTS: i32 = 6043;
pub const MENU_ID_CREATE_CALL_LINK: i32 = 6044;
pub const MENU_ID_JOIN_CALL_LINK: i32 = 6045;
pub const MENU_ID_FOCUS_MESSAGE_FIELD: i32 = 6046;
pub const MENU_ID_ANNOUNCE_STATUS: i32 = 6047;
pub const MENU_ID_FOCUS_MESSAGES_LIST: i32 = 6048;
pub const MENU_ID_EMOJI_PANEL: i32 = 6049;
pub const MENU_ID_STICKER_PANEL: i32 = 6050;
pub const MENU_ID_CHAT_PRIVACY: i32 = 6041;

/// The Conversation menu's items, which all act on the open chat.
const CONVERSATION_MENU_IDS: [i32; 18] = [
	MENU_ID_CHAT_INFO,
	MENU_ID_SEARCH_CHAT,
	MENU_ID_PINNED_MESSAGES,
	MENU_ID_MUTE_CHAT,
	MENU_ID_ARCHIVE_CHAT,
	MENU_ID_PIN_CHAT,
	MENU_ID_LOCK_CHAT,
	MENU_ID_MARK_UNREAD,
	MENU_ID_DISAPPEARING,
	MENU_ID_CHAT_PRIVACY,
	MENU_ID_REPORT_CHAT,
	MENU_ID_BLOCK_CHAT,
	MENU_ID_STAR_MESSAGE,
	MENU_ID_FOCUS_MESSAGE_FIELD,
	MENU_ID_FOCUS_MESSAGES_LIST,
	MENU_ID_ANNOUNCE_STATUS,
	MENU_ID_EMOJI_PANEL,
	MENU_ID_STICKER_PANEL,
];

/// The main panes Alt+E can come back to.
#[derive(Clone, Copy)]
enum ReturnPane {
	Nav,
	Chats,
	Messages,
}

/// Where Alt+E found the user before moving them into the message box: a main
/// pane with its selected row (and which chat was open, since a row index means
/// nothing once another chat is showing), or any other control, by its window
/// handle.
enum ReturnSpot {
	Pane { pane: ReturnPane, index: Option<u32>, chat: Option<String> },
	Control(usize),
}

/// Gives keyboard focus back to the window with this handle, if it still exists.
fn restore_focus(handle: usize) -> bool {
	let hwnd = windows::Win32::Foundation::HWND(handle as *mut std::ffi::c_void);
	unsafe {
		windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(hwnd)).as_bool()
			&& windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(hwnd)).is_ok()
	}
}

/// Shows the selected message's own content (no sender, time or reply wrapper:
/// see `ChatMessage::body_text`) in a read-only multi-line field. A list row's
/// text can be read by NVDA but not selected or copied, so this gives a normal
/// field to browse and Ctrl+C from without letting anyone change the message.
///
/// Uses `Rich2` and `DontWrap` with CRLF newlines so screen readers (NVDA/JAWS)
/// can navigate paragraph by paragraph with Ctrl+Up/Down and lines aren't
/// artificially split by word wrapping.
fn show_message_text(parent: &Frame, list: &ListBox, state: &std::cell::RefCell<crate::AppState>) {
	let Some(idx) = list.get_selection() else { return };
	let text = state.borrow().display_items.get(idx as usize).map(|item| item.body_text());
	let Some(text) = text else { return };
	let text = text.replace("\r\n", "\n").replace('\n', "\r\n");
	let dlg = Dialog::builder(parent, "Message Text")
		.with_size(480, 320)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	let dlg_panel = Panel::builder(&dlg).build();
	let dlg_sizer = BoxSizer::builder(Orientation::Vertical).build();

	let text_view = TextCtrl::builder(&dlg_panel)
		.with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly | TextCtrlStyle::Rich2 | TextCtrlStyle::DontWrap)
		.build();
	text_view.set_value(&text);
	dlg_sizer.add(&text_view, 1, SizerFlag::Expand | SizerFlag::All, 8);

	let close_btn = Button::builder(&dlg_panel).with_id(wxdragon::id::ID_OK).with_label("Close").build();
	dlg_sizer.add(&close_btn, 0, SizerFlag::AlignRight | SizerFlag::All, 8);
	dlg_panel.set_sizer(dlg_sizer, true);

	let dlg_esc = dlg;
	text_view.on_key_down(keys::escape_cancels(dlg_esc));

	text_view.set_focus();
	text_view.set_selection(0, 0);
	dlg.show_modal();
	// The base Dialog has no Drop, so a dialog built per use must be
	// destroyed or it (and every closure bound to it) lives on hidden.
	dlg.destroy();
}

/// The chat the Conversation menu acts on: the open one, unless the Status
/// or Starred tab is showing, whose rows aren't chats.
pub(crate) fn conversation_menu_chat(s: &crate::AppState) -> Option<&str> {
	if s.active_tab == 1 || s.active_tab == 2 {
		return None;
	}
	s.selected_chat.as_deref()
}

/// Greys out what doesn't apply right now. A disabled item's shortcut does
/// nothing either, so this runs whenever the tab or the open chat changes,
/// not only when a menu opens.
pub(crate) fn refresh_menu_bar(s: &crate::AppState, frame: &Frame) {
	let Some(menu_bar) = frame.get_menu_bar() else { return };
	let chat = conversation_menu_chat(s);
	for id in CONVERSATION_MENU_IDS {
		menu_bar.enable_item(id, chat.is_some());
	}
	let Some(jid) = chat else { return };
	let is_contact = jid.ends_with("@s.whatsapp.net") || jid.ends_with("@lid");
	let is_channel = jid.ends_with("@newsletter");
	let is_broadcast = jid.ends_with("@broadcast");
	menu_bar.enable_item(MENU_ID_BLOCK_CHAT, is_contact);
	menu_bar.enable_item(MENU_ID_REPORT_CHAT, !is_channel && !is_broadcast);
	menu_bar.enable_item(MENU_ID_DISAPPEARING, !is_channel);
	menu_bar.enable_item(MENU_ID_CHAT_PRIVACY, !is_channel && !is_broadcast);
}

pub fn build_main_window() -> WindowParts {
	let frame = Frame::builder().with_title("whatSnap").with_size(Size::new(1180, 760)).build();
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
	let tab_order = crate::settings::Settings::load().tab_order();
	let nav_list = ListBox::builder(&nav_panel)
		.with_choices(tab_order.iter().map(|&t| crate::settings::TAB_NAMES[t].to_string()).collect())
		.build();
	nav_list.set_font(&theme::Fonts::body_strong());
	theme::style_sidebar(&nav_list);
	// Conversations is the tab the app starts on, wherever it was moved to.
	nav_list.set_selection(tab_order.iter().position(|&t| t == 0).unwrap_or(0) as u32, true);
	nav_sizer.add(&nav_label, 0, SizerFlag::All, 10);
	nav_sizer.add(&nav_list, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom, 10);

	nav_panel.set_sizer(nav_sizer, true);

	let account_menu = Menu::builder()
		.append_item(MENU_ID_SETTINGS, &default_menu_label(MENU_ID_SETTINGS), "Open settings")
		.append_item(
			MENU_ID_RECONNECT,
			&default_menu_label(MENU_ID_RECONNECT),
			"Retry connecting to WhatsApp right away",
		)
		.append_item(MENU_ID_BLOCKED_CONTACTS, "&Blocked Contacts...", "See and change who you have blocked")
		.append_separator()
		.append_item(MENU_ID_LOGOUT, "Log &Out", "Log out of WhatsApp on this device")
		.build();
	// Alt+1/2/3 match official WhatsApp's Chats/Status/Communities shortcuts;
	// Alt+4/5 extend the pattern to whatSnap's Starred/Archived tabs, which
	// WhatsApp doesn't have. These need to live on real menu items - wx only
	// builds an app-wide accelerator table (honored no matter which control
	// has focus) from the "\tAlt+1" accelerator syntax on a menu label; a
	// plain key-down handler on a widget loses to that table for Alt-combos.
	let go_menu = Menu::builder()
		.append_item(MENU_ID_NAV_CONVERSATIONS, &default_menu_label(MENU_ID_NAV_CONVERSATIONS), "Jump to the chat list")
		.append_item(MENU_ID_NAV_STATUS, &default_menu_label(MENU_ID_NAV_STATUS), "Jump to Status")
		.append_item(MENU_ID_NAV_COMMUNITIES, &default_menu_label(MENU_ID_NAV_COMMUNITIES), "Jump to Communities")
		.append_item(MENU_ID_NAV_STARRED, &default_menu_label(MENU_ID_NAV_STARRED), "Jump to Starred Messages")
		.append_item(MENU_ID_NAV_ARCHIVED, &default_menu_label(MENU_ID_NAV_ARCHIVED), "Jump to Archived Chats")
		.append_item(MENU_ID_NAV_CHANNELS, &default_menu_label(MENU_ID_NAV_CHANNELS), "Jump to the channels you follow")
		.append_item(MENU_ID_NAV_CALLS, &default_menu_label(MENU_ID_NAV_CALLS), "Jump to Calls")
		.build();
	// Matches official WhatsApp's shortcuts, minus Alt on every combo that
	// also has Ctrl (kept only on the Alt-only ones) - see the Alt+1..5
	// comment above for why these have to be real menu items.
	let chats_menu = Menu::builder()
		.append_item(MENU_ID_NEW_CHAT, &default_menu_label(MENU_ID_NEW_CHAT), "Start a new chat")
		.append_item(MENU_ID_NEW_GROUP, &default_menu_label(MENU_ID_NEW_GROUP), "Start a new group")
		.append_item(MENU_ID_NEW_COMMUNITY, "New C&ommunity...", "Start a community to bring groups together")
		.append_separator()
		.append_item(MENU_ID_SEARCH_GLOBAL, &default_menu_label(MENU_ID_SEARCH_GLOBAL), "Search chats")
		.append_item(MENU_ID_MANAGE_LISTS, "Manage &Lists...", "Make, rename or delete your chat lists")
		.build();
	// Everything here acts on the open chat, so `refresh_menu_bar` greys it
	// out on the Status and Starred tabs and when no chat is open.
	let conversation_menu = Menu::builder()
		.append_item(MENU_ID_CHAT_INFO, &default_menu_label(MENU_ID_CHAT_INFO), "Open info for the open chat")
		.append_item(
			MENU_ID_FOCUS_MESSAGE_FIELD,
			&default_menu_label(MENU_ID_FOCUS_MESSAGE_FIELD),
			"Move focus to the message box, or back to where you were",
		)
		.append_item(
			MENU_ID_FOCUS_MESSAGES_LIST,
			&default_menu_label(MENU_ID_FOCUS_MESSAGES_LIST),
			"Move focus to the messages list",
		)
		.append_item(
			MENU_ID_ANNOUNCE_STATUS,
			&default_menu_label(MENU_ID_ANNOUNCE_STATUS),
			"Say the open chat's status: online, typing and so on",
		)
		.append_item(
			MENU_ID_EMOJI_PANEL,
			&default_menu_label(MENU_ID_EMOJI_PANEL),
			"Pick an emoji to put in the message box",
		)
		.append_item(
			MENU_ID_STICKER_PANEL,
			&default_menu_label(MENU_ID_STICKER_PANEL),
			"Pick a sticker to send: favorites, recent ones or WhatsApp's own packs",
		)
		.append_item(MENU_ID_SEARCH_CHAT, &default_menu_label(MENU_ID_SEARCH_CHAT), "Search within the open chat")
		.append_item(MENU_ID_PINNED_MESSAGES, "Pi&nned Messages...", "See and go to the open chat's pinned messages")
		.append_separator()
		.append_item(MENU_ID_MUTE_CHAT, &default_menu_label(MENU_ID_MUTE_CHAT), "Mute the open chat")
		.append_item(MENU_ID_ARCHIVE_CHAT, &default_menu_label(MENU_ID_ARCHIVE_CHAT), "Archive the open chat")
		.append_item(MENU_ID_PIN_CHAT, &default_menu_label(MENU_ID_PIN_CHAT), "Pin the open chat")
		.append_item(MENU_ID_LOCK_CHAT, &default_menu_label(MENU_ID_LOCK_CHAT), "Lock or unlock the open chat")
		.append_item(
			MENU_ID_MARK_UNREAD,
			&default_menu_label(MENU_ID_MARK_UNREAD),
			"Toggle read/unread on the open chat",
		)
		.append_separator()
		.append_item(
			MENU_ID_DISAPPEARING,
			"&Disappearing Messages...",
			"Turn disappearing messages on or off for the open chat",
		)
		.append_item(
			MENU_ID_CHAT_PRIVACY,
			"Ad&vanced Chat Privacy...",
			"Turn advanced chat privacy on or off for the open chat",
		)
		.append_separator()
		.append_item(MENU_ID_REPORT_CHAT, "&Report...", "Report the open chat's contact or group to WhatsApp")
		.append_item(MENU_ID_BLOCK_CHAT, "&Block or Unblock Contact", "Block or unblock the open chat's contact")
		.append_separator()
		.append_item(MENU_ID_STAR_MESSAGE, &default_menu_label(MENU_ID_STAR_MESSAGE), "Star the selected message")
		.build();
	let voice_menu = Menu::builder()
		.append_item(
			MENU_ID_START_RECORDING,
			&default_menu_label(MENU_ID_START_RECORDING),
			"Start a voice message recording",
		)
		.append_item(
			MENU_ID_PAUSE_RECORDING,
			&default_menu_label(MENU_ID_PAUSE_RECORDING),
			"Pause/resume the voice recording",
		)
		.append_item(MENU_ID_SEND_PTT, &default_menu_label(MENU_ID_SEND_PTT), "Send the recorded voice message")
		.build();
	let call_menu = Menu::builder()
		.append_item(
			MENU_ID_CALL_MUTE,
			&default_menu_label(MENU_ID_CALL_MUTE),
			"Mute/unmute the microphone during a call",
		)
		.append_item(MENU_ID_CALL_END, &default_menu_label(MENU_ID_CALL_END), "End the active call")
		.append_separator()
		.append_item(MENU_ID_CREATE_CALL_LINK, "&Create Call Link...", "Make a link anyone can use to join a call")
		.append_item(MENU_ID_JOIN_CALL_LINK, "&Join Call Link...", "Join a call from a link someone shared")
		.build();
	let menu_bar = MenuBar::builder()
		.append(account_menu, "&Account")
		.append(go_menu, "&Go")
		.append(chats_menu, "&Chats")
		.append(conversation_menu, "C&onversation")
		.append(voice_menu, "&Voice")
		.append(call_menu, "Ca&ll")
		.build();
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

	let pairing_dlg_esc = pairing_dialog;
	pairing_code_display.on_key_down(keys::escape_cancels(pairing_dlg_esc));

	let new_conv_dialog = crate::ui::new_conversation::build_new_conversation_dialog(&frame);
	let new_contact_dialog = crate::ui::new_contact::build_new_contact_dialog(&frame);
	let poll_dialog = crate::ui::poll_dialog::build_poll_dialog(&frame);
	let new_group_dialog = crate::ui::new_group::build_new_group_dialog(&frame);
	let add_members_dialog = crate::ui::add_members::build_add_members_dialog(&frame);
	let attach_contact_dialog = crate::ui::attach_contact::build_attach_contact_dialog(&frame);
	let new_group_contacts_timer = Rc::new(Timer::new(&new_group_dialog.dialog));
	let add_members_contacts_timer = Rc::new(Timer::new(&add_members_dialog.dialog));
	let attach_contact_timer = Rc::new(Timer::new(&attach_contact_dialog.dialog));
	let attachment_dialog = crate::ui::attachment_dialog::build_attachment_dialog(&frame);
	let call_parts = crate::ui::call_window::build_call_window();
	let conversation_data_dialog = crate::ui::conversation_data::build_conversation_data_dialog(&frame);
	let group_data_dialog = crate::ui::group_data::build_group_data_dialog(&frame);
	let settings_dialog = crate::ui::settings_dialog::build_settings_dialog(&frame);
	let tray_icon = Rc::new(TaskBarIcon::builder().build());

	let conv_panel = crate::ui::conversations::build_conversations_panel(&content_panel);

	content_sizer.add(&conv_panel.panel, 1, SizerFlag::Expand, 0);
	conv_panel.panel.show(true);

	// Start everything hidden until the backend reports login state. Showing
	// the login panel here unconditionally made it flash on screen for a few
	// seconds at every launch - even when a saved session was about to log us
	// straight in - because it was the default startup screen rather than a
	// choice gated on whether we're actually logged in. The "connected" /
	// "needs_login" events (see main.rs) now pick the right screen instead.
	nav_panel.show(false);
	content_panel.show(false);
	login_panel.show(false);

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
	set_dialog_owner(&poll_dialog.dialog, &frame);
	set_dialog_owner(&new_group_dialog.dialog, &frame);
	set_dialog_owner(&add_members_dialog.dialog, &frame);
	set_dialog_owner(&attach_contact_dialog.dialog, &frame);
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
		poll_dialog,
		new_group_dialog,
		add_members_dialog,
		attach_contact_dialog,
		new_group_contacts_timer,
		add_members_contacts_timer,
		attach_contact_timer,
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
		let clean_phone: String = phone.chars().filter(|c| c.is_ascii_digit()).collect();
		if !clean_phone.is_empty() {
			let _ = tx_pair.send(UiCommand::RequestPairing(clean_phone));
		}
	});

	let tx_qr = ui_tx.clone();
	parts.switch_to_qr_btn.on_click(move |_| {
		let _ = tx_qr.send(UiCommand::RequestQr);
	});

	let qr_dialog = parts.qrcode_dialog;
	let login_from_qr = parts.login_panel;
	let tx_cancel_qr = ui_tx.clone();
	parts.qrcode_cancel_btn.on_click(move |_| {
		// Back to the phone-entry screen, keeping the one-screen-at-a-time flow.
		qr_dialog.show(false);
		login_from_qr.show(true);
		login_from_qr.set_focus();
		let _ = tx_cancel_qr.send(UiCommand::CancelQr);
	});

	let pairing_dialog = parts.pairing_dialog;
	let login_from_pairing = parts.login_panel;
	let tx_cancel_pairing = ui_tx.clone();
	parts.pairing_cancel_btn.on_click(move |_| {
		pairing_dialog.show(false);
		login_from_pairing.show(true);
		login_from_pairing.set_focus();
		let _ = tx_cancel_pairing.send(UiCommand::CancelPairing);
	});

	let tx_send = ui_tx.clone();
	let msg_ctrl = parts.conversations_parts.message_field;
	let state_send_btn = state.clone();
	parts.conversations_parts.send_message_btn.on_click(move |_| {
		let text = msg_ctrl.get_value();
		if let Some(message_id) = state_send_btn.borrow().editing_message_id.clone() {
			let _ = tx_send.send(UiCommand::EditMessage { message_id, new_text: text });
		} else if !text.is_empty() {
			let _ = tx_send.send(UiCommand::SendMessage(text));
		}
	});

	let tx_typing = ui_tx.clone();
	let msg_ctrl_typing = parts.conversations_parts.message_field;
	let state_typing = state.clone();
	let conv_parts_typing = parts.conversations_parts;
	msg_ctrl_typing.on_text_changed(move |_| {
		crate::ui_commands::mentions::refresh(&state_typing, &conv_parts_typing);
		let text = msg_ctrl_typing.get_value();
		// set_value() fires this too - restoring a chat's saved draft on open,
		// or entering edit mode, is not the user typing, and mustn't tell the
		// chat "composing". Clearing the field still counts (it's how a
		// "composing" we did send gets taken back).
		if !text.is_empty() && !msg_ctrl_typing.has_focus() {
			return;
		}
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

	let tx_post_status = ui_tx.clone();
	let tx_new_conv = ui_tx.clone();
	let state_new_conv = state.clone();
	let frame_new_conv = parts.frame;
	parts.conversations_parts.new_conv_btn.on_click(move |_| {
		if state_new_conv.borrow().active_tab == 5 {
			let _ = tx_new_conv.send(UiCommand::FindChannels);
		} else if state_new_conv.borrow().active_tab == 1 {
			let dlg = TextEntryDialog::builder(&frame_new_conv, "What's on your mind?", "Post Status Update").build();
			if dlg.show_modal() == wxdragon::id::ID_OK
				&& let Some(val) = dlg.get_value()
			{
				let text = val.trim().to_string();
				if !text.is_empty() {
					let _ = tx_post_status.send(UiCommand::PostStatus { text });
				}
			}
		} else {
			let _ = tx_new_conv.send(UiCommand::PrepareContactPicker(crate::ContactPickerMode::OpenChat));
		}
	});

	let tx_post_media_status = ui_tx.clone();
	let state_post_media_status = state.clone();
	parts.conversations_parts.post_media_status_btn.on_click(move |_| {
		// The Channels tab borrows this second button for "Create channel".
		if state_post_media_status.borrow().active_tab == 5 {
			let _ = tx_post_media_status.send(UiCommand::CreateChannel);
		} else {
			let _ = tx_post_media_status.send(UiCommand::PostMediaStatus);
		}
	});

	let tx_locked_chats = ui_tx.clone();
	let state_locked_chats = state.clone();
	parts.conversations_parts.locked_chats_btn.on_click(move |_| {
		if state_locked_chats.borrow().chats_unlocked {
			let _ = tx_locked_chats.send(UiCommand::CloseLockedChats);
		} else {
			let _ = tx_locked_chats.send(UiCommand::PromptLockedChats);
		}
	});

	let new_conv_dialog_close = parts.new_conv_dialog.dialog;
	parts.new_conv_dialog.close_btn.on_click(move |_| {
		new_conv_dialog_close.end_modal(0);
	});

	let tx_new_contact_form = ui_tx.clone();
	let new_conv_dlg_for_contact = parts.new_conv_dialog.dialog;
	parts.new_conv_dialog.new_contact_btn.on_click(move |_| {
		new_conv_dlg_for_contact.end_modal(0);
		let _ = tx_new_contact_form.send(UiCommand::OpenNewContactForm);
	});

	let new_conv_dlg_for_meta_ai = parts.new_conv_dialog.dialog;
	let tx_meta_ai = ui_tx.clone();
	parts.new_conv_dialog.message_meta_ai_btn.on_click(move |_| {
		new_conv_dlg_for_meta_ai.end_modal(0);
		let _ = tx_meta_ai.send(UiCommand::OpenChatByJid(crate::types::META_AI_JID.to_string()));
	});

	let tx_picker_filter = ui_tx.clone();
	let picker_search_field = parts.new_conv_dialog.search_field;
	picker_search_field.on_text_changed(move |_| {
		let query = picker_search_field.get_value();
		let _ = tx_picker_filter.send(UiCommand::FilterContactPicker(query));
	});

	// Enter in the search field jumps straight to the top match, so picking
	// a contact never requires touching the results list at all.
	let tx_picker_select_from_search = ui_tx.clone();
	let picker_results_list_from_search = parts.new_conv_dialog.results_list;
	picker_search_field.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key_event) = event {
			let code = key_event.get_key_code();
			if keys::is_enter(code) && picker_results_list_from_search.get_count() > 0 {
				let _ = tx_picker_select_from_search.send(UiCommand::SelectContactPickerResult(0));
				event.skip(false);
				return;
			}
		}
		event.skip(true);
	});

	let tx_picker_select = ui_tx.clone();
	let picker_results_list = parts.new_conv_dialog.results_list;
	picker_results_list.on_item_double_clicked(move |event| {
		if let Some(idx) = event.get_selection() {
			let _ = tx_picker_select.send(UiCommand::SelectContactPickerResult(idx as usize));
		}
	});

	let tx_picker_select_key = ui_tx.clone();
	let picker_results_list_key = parts.new_conv_dialog.results_list;
	let picker_dialog_esc = parts.new_conv_dialog.dialog;
	// Plain Enter on a ListBox row never reaches on_key_down here - this
	// dialog's IsDialogMessage handling takes it for the default button (only
	// Alt+Enter got through) - which is what on_enter's key-up half is for.
	keys::on_enter(&picker_results_list_key, Rc::default(), move || {
		if let Some(idx) = picker_results_list_key.get_selection() {
			let _ = tx_picker_select_key.send(UiCommand::SelectContactPickerResult(idx as usize));
		}
	});
	picker_results_list_key.on_key_down(keys::escape_cancels(picker_dialog_esc));

	let tx_conv_data = ui_tx.clone();
	parts.conversations_parts.conv_data_btn.on_click(move |_| {
		let _ = tx_conv_data.send(UiCommand::ShowConversationData);
	});

	let tx_enter_call = ui_tx.clone();
	let tx_place_call = ui_tx.clone();
	parts.conversations_parts.place_call_btn.on_click(move |_| {
		let _ = tx_place_call.send(UiCommand::PlaceCall);
	});
	let tx_place_video_call = ui_tx.clone();
	parts.conversations_parts.place_video_call_btn.on_click(move |_| {
		let _ = tx_place_video_call.send(UiCommand::PlaceVideoCall);
	});

	let tx_calls_call = ui_tx.clone();
	parts.conversations_parts.calls_call_btn.on_click(move |_| {
		let _ = tx_calls_call.send(UiCommand::PlaceCall);
	});
	let tx_calls_video = ui_tx.clone();
	parts.conversations_parts.calls_video_btn.on_click(move |_| {
		let _ = tx_calls_video.send(UiCommand::PlaceVideoCall);
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
	let tx_camera = ui_tx.clone();
	parts.call_parts.camera_btn.on_click(move |_| {
		let _ = tx_camera.send(UiCommand::ToggleCamera);
	});
	let tx_screen = ui_tx.clone();
	parts.call_parts.screen_btn.on_click(move |_| {
		let _ = tx_screen.send(UiCommand::ToggleScreenShare);
	});
	let tx_call_input = ui_tx.clone();
	let call_input_combo = parts.call_parts.input_device_combo;
	parts.call_parts.input_device_combo.on_selection_changed(move |_| {
		use crate::ui::settings_dialog::device_combo_value;
		let _ = tx_call_input.send(UiCommand::SetCallInputDevice(device_combo_value(&call_input_combo)));
	});
	let tx_call_output = ui_tx.clone();
	let call_output_combo = parts.call_parts.output_device_combo;
	parts.call_parts.output_device_combo.on_selection_changed(move |_| {
		use crate::ui::settings_dialog::device_combo_value;
		let _ = tx_call_output.send(UiCommand::SetCallOutputDevice(device_combo_value(&call_output_combo)));
	});
	let tx_add_participant = ui_tx.clone();
	parts.call_parts.add_participant_btn.on_click(move |_| {
		let _ = tx_add_participant.send(UiCommand::PrepareContactPicker(crate::ContactPickerMode::AddCallParticipant));
	});
	let tx_raise_hand = ui_tx.clone();
	parts.call_parts.raise_hand_btn.on_click(move |_| {
		let _ = tx_raise_hand.send(UiCommand::ToggleCallHand);
	});
	let tx_call_react = ui_tx.clone();
	parts.call_parts.react_btn.on_click(move |_| {
		let _ = tx_call_react.send(UiCommand::CallReact);
	});
	let tx_let_in = ui_tx.clone();
	parts.call_parts.let_in_btn.on_click(move |_| {
		let _ = tx_let_in.send(UiCommand::AdmitCallPerson(true));
	});
	let tx_keep_out = ui_tx.clone();
	parts.call_parts.keep_out_btn.on_click(move |_| {
		let _ = tx_keep_out.send(UiCommand::AdmitCallPerson(false));
	});
	let tx_approval = ui_tx.clone();
	parts.call_parts.approval_check.on_toggled(move |event| {
		let _ = tx_approval.send(UiCommand::SetCallApproval(event.is_checked()));
	});
	let tx_call_camera = ui_tx.clone();
	let call_camera_combo = parts.call_parts.camera_device_combo;
	parts.call_parts.camera_device_combo.on_selection_changed(move |_| {
		use crate::ui::settings_dialog::device_combo_value;
		let _ = tx_call_camera.send(UiCommand::SetCallCameraDevice(device_combo_value(&call_camera_combo)));
	});
	// Closing the call window (Alt+F4, its close box) must never destroy it:
	// it's built once and reused for every call, so a destroyed one left
	// every later call - incoming or outgoing - with no window at all until
	// the app restarted. Closing hides it and does what the button on
	// screen would: decline a call that's ringing, otherwise hang up.
	let tx_call_close = ui_tx.clone();
	let call_frame_close = parts.call_parts.frame;
	let state_call_close = state.clone();
	parts.call_parts.frame.on_close(move |event| {
		if let WindowEventData::General(ref ev) = event {
			if !ev.can_veto() {
				return;
			}
			ev.veto();
			call_frame_close.show(false);
			let s = state_call_close.borrow();
			let ringing_in = s.incoming_call_caller.is_some() && s.active_call_audio.is_none();
			let in_call = s.active_call_id.is_some() || s.call_placing;
			drop(s);
			if ringing_in {
				let _ = tx_call_close.send(UiCommand::RejectCall);
			} else if in_call {
				let _ = tx_call_close.send(UiCommand::LeaveCall);
			}
		}
	});

	// Ctrl+Shift+V toggles the camera and Ctrl+Shift+S the screen share from
	// anywhere in the call window, matching their buttons.
	let tx_camera_key = ui_tx.clone();
	parts.call_parts.frame.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key_event) = event
			&& key_event.control_down()
			&& key_event.shift_down()
		{
			let command = if keys::is_letter(key_event.get_key_code(), 'V') {
				Some(UiCommand::ToggleCamera)
			} else if keys::is_letter(key_event.get_key_code(), 'S') {
				Some(UiCommand::ToggleScreenShare)
			} else {
				None
			};
			if let Some(command) = command {
				let _ = tx_camera_key.send(command);
				event.skip(false);
				return;
			}
		}
		event.skip(true);
	});

	let tx_contact = ui_tx.clone();
	parts.conversation_data_dialog.contact_btn.on_click(move |_| {
		let _ = tx_contact.send(UiCommand::AddOrEditContact);
	});
	let tx_delete_contact = ui_tx.clone();
	parts.conversation_data_dialog.delete_contact_btn.on_click(move |_| {
		let _ = tx_delete_contact.send(UiCommand::DeleteContact);
	});
	let tx_info_privacy = ui_tx.clone();
	parts.conversation_data_dialog.chat_privacy_btn.on_click(move |_| {
		let _ = tx_info_privacy.send(UiCommand::ShowChatPrivacy);
	});

	let conv_data_dialog_back = parts.conversation_data_dialog.dialog;
	parts.conversation_data_dialog.back_btn.on_click(move |_| {
		conv_data_dialog_back.end_modal(0);
	});

	let group_data_dialog_back = parts.group_data_dialog.dialog;
	parts.group_data_dialog.back_btn.on_click(move |_| {
		group_data_dialog_back.end_modal(0);
	});

	// ── Tray icon ────────────────────────────────────────────────────────────
	// The icon bitmap and popup menu are set up once; whether it's actually
	// installed (and thus whether closing the window hides to it) tracks the
	// `show_tray_icon` setting, reconciled below and whenever Settings is saved.
	let tray_icon = parts.tray_icon.clone();
	let tray_bitmap = theme::build_app_icon(32);
	// Whether an Enter release still to come has already been dealt with, so
	// the chat list mustn't open a chat on it. See its uses on the chat list.
	let enter_release_handled = crate::tray::enter_release_guard();

	let tray_open_id = 5001;
	let tray_exit_id = 5002;
	let mut tray_menu = Menu::builder()
		.append_item(tray_open_id, "&Open whatSnap", "Show the main window")
		.append_item(tray_exit_id, "E&xit", "Quit whatSnap")
		.build();
	let frame_tray_menu = parts.frame;
	let tx_tray_menu = ui_tx.clone();
	tray_icon.on_menu(move |event| {
		let id = event.get_id();
		if id == tray_open_id {
			crate::tray::restore_main_window(&frame_tray_menu);
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

	// A click on the icon, or Space/Enter on it from the keyboard, restores the
	// window. See `tray::enable_keyboard_select` for why this isn't wx's
	// on_left_up.
	let frame_tray_select = parts.frame;
	let tx_tray_select = ui_tx.clone();
	crate::tray::set_on_select(move || {
		crate::tray::restore_main_window(&frame_tray_select);
		let _ = tx_tray_select.send(UiCommand::SetPresence(true));
	});

	let apply_tray_setting = Rc::new({
		let tray_icon = tray_icon.clone();
		let tray_bitmap = tray_bitmap.clone();
		let state = state.clone();
		move |enabled: bool| {
			if enabled {
				// Two calls on purpose - see `install_tray_icon` in main.rs. The
				// icon is added under the app name so that becomes the item's
				// accessible name, and the count is written over it straight after
				// so switching the icon back on in Settings doesn't leave a bare
				// name until the next chats broadcast refreshes it.
				let s = state.borrow();
				tray_icon.set_icon(&tray_bitmap, whatsnap::TRAY_APP_NAME);
				let unread = whatsnap::unread_notification_total(
					&s.unread_counts,
					&s.muted_jids,
					&s.archived_jids,
					&s.settings.archived_alert_levels,
				);
				tray_icon.set_icon(&tray_bitmap, &whatsnap::tray_tooltip_text(unread));
				crate::tray::enable_keyboard_select();
			} else {
				tray_icon.remove_icon();
			}
		}
	});
	// When Explorer restarts, or the taskbar is recreated after a display or
	// DPI change, wx adds the icon back with its last tooltip - the count - and
	// that becomes the icon's name. Add it back under the app name instead.
	{
		let apply_tray_setting = apply_tray_setting.clone();
		let tray_icon = tray_icon.clone();
		let state = state.clone();
		crate::tray::set_on_taskbar_created(move || {
			let Ok(s) = state.try_borrow() else {
				return false;
			};
			let shown = s.settings.show_tray_icon && s.main_ui_shown;
			drop(s);
			if shown {
				tray_icon.remove_icon();
				apply_tray_setting(true);
			}
			true
		});
	}
	// Only ever install the tray icon while logged in - there's nothing to
	// keep running in the background for otherwise, and a tray icon for an
	// unpaired app is just clutter (and one more stray thing for a screen
	// reader to trip over). main_ui_shown is false until "connected" lands, so
	// this installs nothing at startup; the connected/needs_login handlers in
	// main.rs reconcile it on every login state change thereafter.
	let show_tray_now = {
		let s = state.borrow();
		s.settings.show_tray_icon && s.main_ui_shown
	};
	apply_tray_setting(show_tray_now);

	// Closing the window hides it to the tray instead of quitting, as long as
	// the tray icon is actually installed to bring it back with. A forced
	// close (the tray menu's Exit, or Windows shutting the app down) can't be
	// vetoed, so `can_veto()` tells the two cases apart.
	let frame_close = parts.frame;
	let tray_icon_close = tray_icon.clone();
	let tx_close = ui_tx.clone();
	let state_close = state.clone();
	parts.frame.on_close(move |event| {
		if let WindowEventData::General(ref ev) = event {
			// Only hide-to-tray while logged in - a signed-in session is worth
			// keeping alive in the background to receive messages/calls. When
			// we're not logged in there's nothing to stay running for, so
			// Alt+F4 should actually quit instead of parking a stray (and, to a
			// screen reader, oddly-announced) window in the tray. main_ui_shown
			// is true only once "connected" has shown the chat list.
			let logged_in = state_close.borrow().main_ui_shown;
			if logged_in && ev.can_veto() && tray_icon_close.is_icon_installed() {
				ev.veto();
				crate::tray::hide_main_window(&frame_close);
				let _ = tx_close.send(UiCommand::WindowHidden);
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

	// The login dialog is only ever up while we're logged out, so Alt+F4 (or
	// its close box) there should quit outright too. Without this the close
	// could land on the dialog rather than the frame and fall through to wx's
	// default hide, leaving an invisible, still-running app behind.
	parts.login_panel.on_close(move |event| {
		if let WindowEventData::General(_) = event
			&& let Some(app) = wxdragon::get_app_instance()
		{
			app.exit_main_loop();
		}
	});

	// The window is only "actively in use" while it's focused at the OS
	// level; losing focus (alt-tabbing away, minimizing, or hiding to tray)
	// should stop broadcasting an online presence to WhatsApp, matching what
	// the official clients do.
	let tx_activate = ui_tx.clone();
	parts.frame.on_activate(move |event| {
		if let WindowEventData::Activate(ref ev) = event {
			// Something else activating the window while it's put away in the
			// tray isn't the user coming back - see tray::hide_main_window.
			if ev.is_active() && crate::tray::is_hidden_to_tray() {
				return;
			}
			let _ = tx_activate.send(UiCommand::SetPresence(ev.is_active()));
		}
	});

	let keymap = Rc::new(crate::keymap::Keymap::from_saved(&state.borrow().settings.shortcuts));
	keymap.apply(&parts.frame);
	state.borrow_mut().keymap = keymap;
	// Edits from the Keyboard Shortcuts dialog wait here until Settings is
	// confirmed, so Cancel on Settings throws them away like everything else.
	let pending_shortcuts: Rc<std::cell::RefCell<Option<crate::keymap::Keymap>>> = Rc::default();

	let settings_dialog = parts.settings_dialog.dialog;
	let sounds_check = parts.settings_dialog.sounds_check;
	let notifications_check = parts.settings_dialog.notifications_check;
	let follow_dnd_check_open = parts.settings_dialog.follow_dnd_check;
	let announce_typing_check = parts.settings_dialog.announce_typing_check;
	let announce_recording_check = parts.settings_dialog.announce_recording_check;
	let tray_icon_check = parts.settings_dialog.tray_icon_check;
	let autostart_check = parts.settings_dialog.autostart_check;
	let start_maximized_check_open = parts.settings_dialog.start_maximized_check;
	let meta_ai_enabled_check_open = parts.settings_dialog.meta_ai_enabled_check;
	let verbose_logging_check_open = parts.settings_dialog.verbose_backend_logging_check;
	let name_field_open = parts.settings_dialog.name_field;
	let settings_parts_open = parts.settings_dialog;
	let input_device_combo_open = parts.settings_dialog.input_device_combo;
	let output_device_combo_open = parts.settings_dialog.output_device_combo;
	let noise_suppression_check_open = parts.settings_dialog.voice_message_noise_suppression_check;
	let dual_stream_check_open = parts.settings_dialog.dual_stream_screen_share_check;
	let state_settings_open = state.clone();
	let pending_shortcuts_open = pending_shortcuts.clone();
	parts.frame.on_menu_selected(move |evt| {
		if evt.get_id() != MENU_ID_SETTINGS {
			return;
		}
		pending_shortcuts_open.borrow_mut().take();
		use crate::ui::settings_dialog::*;
		let mut s = state_settings_open.borrow_mut();
		sounds_check.set_value(s.settings.sounds_enabled);
		notifications_check.set_value(s.settings.notifications_enabled);
		follow_dnd_check_open.set_value(s.settings.follow_windows_do_not_disturb);
		announce_typing_check.set_value(s.settings.announce_typing);
		announce_recording_check.set_value(s.settings.announce_recording);
		tray_icon_check.set_value(s.settings.show_tray_icon);
		meta_ai_enabled_check_open.set_value(s.settings.meta_ai_enabled);
		verbose_logging_check_open.set_value(s.settings.verbose_backend_logging);
		start_maximized_check_open.set_value(s.settings.start_maximized);
		noise_suppression_check_open.set_value(s.settings.voice_message_noise_suppression);
		dual_stream_check_open.set_value(s.settings.dual_stream_screen_share);

		name_field_open.set_value(&s.own_push_name);
		// Everything on the privacy tab is fetched fresh on every open, and
		// stays disabled until it arrives: a value cached from an earlier
		// open (or a dropdown's placeholder) may not be what the phone has
		// set since, and saving it would overwrite the real one.
		s.own_about_loaded = false;
		s.default_timer = None;
		settings_parts_open.default_timer_combo.enable(false);
		s.privacy_settings_loaded = false;
		s.privacy_lists.clear();
		s.privacy_list_edits.clear();
		settings_parts_open.show_about(None);
		settings_parts_open.show_privacy(None);

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
		let _ = s.ws_tx.send(serde_json::json!({"action": "get_default_timer"}).to_string());
		let _ = s.ws_tx.send(serde_json::json!({"action": "get_privacy_settings"}).to_string());
		for control in settings_parts_open.privacy_controls() {
			if control.except_btn.is_some() {
				let payload = serde_json::json!({"action": "get_privacy_list", "data": {"name": control.name}});
				let _ = s.ws_tx.send(payload.to_string());
			}
		}
		// The excluded-contacts picker offers every contact.
		let _ = s.ws_tx.send(serde_json::json!({"action": "get_contacts"}).to_string());
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
			"Log out of WhatsApp on this device? You'll need to scan a QR code or re-enter your phone number to use whatSnap again.",
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

	let state_reconnect = state.clone();
	let reconnect_announcer = parts.conversations_parts.messages_announcer;
	parts.frame.on_menu_selected(move |evt| {
		if evt.get_id() != MENU_ID_RECONNECT {
			return;
		}
		let mut s = state_reconnect.borrow_mut();
		// The backend answers with a "connection_state" either way (even if
		// already online), and this flag makes sure that answer is spoken.
		s.conn_manual_reconnect = true;
		let online = s.conn_online;
		let _ = s.ws_tx.send(serde_json::json!({"action": "reconnect"}).to_string());
		drop(s);
		if !online {
			crate::announce::announce(reconnect_announcer, "Reconnecting...");
		}
	});

	let nav_list_for_menu = parts.nav_list;
	let state_nav_menu = state.clone();
	let nav_announcer = parts.conversations_parts.messages_announcer;
	let tx_nav_menu = ui_tx.clone();
	// One-shot: moves focus into the chat list once the tab name has been spoken.
	let nav_focus_timer = std::rc::Rc::new(Timer::new(&parts.frame));
	let conv_list_for_nav_timer = parts.conversations_parts.conversations_list;
	nav_focus_timer.on_tick(move |_| {
		// Focus may already be there (Alt+1 pressed from inside the list); then
		// this is a no-op and the tab name is all that's spoken.
		conv_list_for_nav_timer.set_focus();
		// The list was just refilled, so nothing is selected. Pre-select the
		// first row so Enter opens it without a Space first.
		if conv_list_for_nav_timer.get_selection().is_none() && conv_list_for_nav_timer.get_count() > 0 {
			conv_list_for_nav_timer.set_selection(0_u32, true);
		}
	});
	// Alt+E's "where you were": each main pane records itself when it takes
	// focus, and the row is captured at the moment Alt+E leaves for the box.
	let return_pane = std::rc::Rc::new(std::cell::Cell::new(ReturnPane::Messages));
	let return_spot: std::rc::Rc<std::cell::RefCell<Option<ReturnSpot>>> =
		std::rc::Rc::new(std::cell::RefCell::new(None));
	let rp_nav = return_pane.clone();
	parts.nav_list.on_set_focus(move |event| {
		rp_nav.set(ReturnPane::Nav);
		event.skip(true);
	});
	let rp_chats = return_pane.clone();
	parts.conversations_parts.conversations_list.on_set_focus(move |event| {
		rp_chats.set(ReturnPane::Chats);
		event.skip(true);
	});
	// Tab (or any other route) into the chat list with nothing selected: pre-select
	// the first row so Enter opens it without a Space first.
	let conv_list_focus_select = parts.conversations_parts.conversations_list;
	parts.conversations_parts.conversations_list.on_set_focus(move |event| {
		if conv_list_focus_select.get_selection().is_none() && conv_list_focus_select.get_count() > 0 {
			conv_list_focus_select.set_selection(0_u32, true);
		}
		event.skip(true);
	});
	let rp_messages = return_pane.clone();
	parts.conversations_parts.messages_list.on_set_focus(move |event| {
		rp_messages.set(ReturnPane::Messages);
		event.skip(true);
	});
	// Leaving the box any other way (Tab, a click) forgets the spot, so a later
	// Alt+E from the box doesn't jump back to a position from long ago.
	let spot_on_leave = return_spot.clone();
	parts.conversations_parts.message_field.on_kill_focus(move |event| {
		spot_on_leave.borrow_mut().take();
		event.skip(true);
	});
	parts.frame.on_menu_selected(move |evt| {
		let tab = match evt.get_id() {
			MENU_ID_NAV_CONVERSATIONS => 0,
			MENU_ID_NAV_STATUS => 1,
			MENU_ID_NAV_STARRED => 2,
			MENU_ID_NAV_ARCHIVED => 3,
			MENU_ID_NAV_COMMUNITIES => 4,
			MENU_ID_NAV_CHANNELS => 5,
			MENU_ID_NAV_CALLS => 6,
			_ => return,
		};
		// The shortcut names a tab; the list may have it anywhere.
		let idx = state_nav_menu.borrow().settings.tab_order().iter().position(|&t| t == tab).unwrap_or(tab) as u32;
		// set_selection's bool means select-vs-deselect, not "fire the change
		// event" - wxListBox::SetSelection never generates one, so switching
		// the highlighted tab needs an explicit NavSelected to actually swap
		// the (shared) list's content, same as a mouse click would trigger.
		nav_list_for_menu.set_selection(idx, true);
		let _ = tx_nav_menu.send(UiCommand::NavSelected(tab));
		// Say the tab name now, then move focus into the chat list shortly
		// after. Focus speech is not a live-region announcement, so it isn't
		// queued behind one: moving focus straight away would talk over the
		// tab name. The timer (below) waits long enough for it to finish.
		let name = nav_list_for_menu.get_string(idx as u32).unwrap_or_default();
		if !name.is_empty() {
			crate::announce::announce(nav_announcer, &name);
		}
		nav_focus_timer.start(700, true);
	});

	let state_chat_menu = state.clone();
	let tx_chat_menu = ui_tx.clone();
	let messages_list_for_menu = parts.conversations_parts.messages_list;
	let search_field_for_menu = parts.conversations_parts.search_field;
	let msg_field_for_menu = parts.conversations_parts.message_field;
	let msgs_announcer_for_menu = parts.conversations_parts.messages_announcer;
	let nav_list_for_focus_menu = parts.nav_list;
	let conv_list_for_focus_menu = parts.conversations_parts.conversations_list;
	let return_pane_for_menu = return_pane.clone();
	let return_spot_for_menu = return_spot.clone();
	parts.frame.on_menu_selected(move |evt| {
		let id = evt.get_id();
		// Belt and braces for the greying out: never act on a chat that
		// isn't the one being looked at.
		if CONVERSATION_MENU_IDS.contains(&id) && conversation_menu_chat(&state_chat_menu.borrow()).is_none() {
			return;
		}
		if id == MENU_ID_NEW_CHAT {
			let _ = tx_chat_menu.send(UiCommand::PrepareContactPicker(crate::ContactPickerMode::OpenChat));
		} else if id == MENU_ID_NEW_GROUP {
			let _ = tx_chat_menu.send(UiCommand::PrepareNewGroup);
		} else if id == MENU_ID_NEW_COMMUNITY {
			let _ = tx_chat_menu.send(UiCommand::NewCommunity);
		} else if id == MENU_ID_MANAGE_LISTS {
			let _ = tx_chat_menu.send(UiCommand::ManageChatLists);
		} else if id == MENU_ID_SEARCH_GLOBAL {
			search_field_for_menu.set_focus();
		} else if id == MENU_ID_SEARCH_CHAT {
			let _ = tx_chat_menu.send(UiCommand::ToggleSearchPanel);
		} else if id == MENU_ID_FOCUS_MESSAGE_FIELD {
			if !msg_field_for_menu.is_shown() {
				crate::announce::announce(msgs_announcer_for_menu, "This chat has no message box.");
			} else if msg_field_for_menu.has_focus() {
				let spot = return_spot_for_menu.borrow_mut().take();
				let chat = state_chat_menu.borrow().selected_chat.clone();
				let restored = match &spot {
					Some(ReturnSpot::Control(handle)) => restore_focus(*handle),
					_ => false,
				};
				if !restored {
					let (pane, index) = match spot {
						Some(ReturnSpot::Pane { pane, index, chat: spot_chat }) => {
							// Rows of another chat's messages aren't where the user was.
							let index =
								if matches!(pane, ReturnPane::Messages) && spot_chat != chat { None } else { index };
							(pane, index)
						}
						_ => (return_pane_for_menu.get(), None),
					};
					let list = match pane {
						ReturnPane::Nav => &nav_list_for_focus_menu,
						ReturnPane::Chats => &conv_list_for_focus_menu,
						ReturnPane::Messages => &messages_list_for_menu,
					};
					if let Some(index) = index.filter(|&i| i < list.get_count()) {
						list.set_selection(index, true);
						list.ensure_visible(index as i32);
					}
					list.set_focus();
				}
			} else {
				let chat = state_chat_menu.borrow().selected_chat.clone();
				let focused = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetFocus() }.0 as usize;
				let panes = [
					(ReturnPane::Nav, &nav_list_for_focus_menu),
					(ReturnPane::Chats, &conv_list_for_focus_menu),
					(ReturnPane::Messages, &messages_list_for_menu),
				];
				let spot =
					if let Some((pane, list)) = panes.iter().find(|(_, list)| list.get_handle() as usize == focused) {
						ReturnSpot::Pane { pane: *pane, index: list.get_selection(), chat }
					} else if focused != 0 {
						ReturnSpot::Control(focused)
					} else {
						let pane = return_pane_for_menu.get();
						let list = match pane {
							ReturnPane::Nav => &nav_list_for_focus_menu,
							ReturnPane::Chats => &conv_list_for_focus_menu,
							ReturnPane::Messages => &messages_list_for_menu,
						};
						ReturnSpot::Pane { pane, index: list.get_selection(), chat }
					};
				*return_spot_for_menu.borrow_mut() = Some(spot);
				msg_field_for_menu.set_focus();
			}
		} else if id == MENU_ID_FOCUS_MESSAGES_LIST {
			// Jump straight to the messages list when a conversation is open.
			if messages_list_for_menu.is_shown() {
				messages_list_for_menu.set_focus();
			}
		} else if id == MENU_ID_ANNOUNCE_STATUS {
			let label = {
				let s = state_chat_menu.borrow();
				s.selected_chat.clone().map(|jid| crate::chat_ui::chat_status_label(&s, &jid))
			};
			if let Some(label) = label {
				crate::announce::announce(msgs_announcer_for_menu, &label);
			}
		} else if id == MENU_ID_EMOJI_PANEL || id == MENU_ID_STICKER_PANEL {
			let _ = tx_chat_menu.send(UiCommand::OpenEmojiPanel { stickers: id == MENU_ID_STICKER_PANEL });
		} else if id == MENU_ID_CHAT_INFO {
			let _ = tx_chat_menu.send(UiCommand::ShowConversationData);
		} else if id == MENU_ID_DISAPPEARING {
			let jid = state_chat_menu.borrow().selected_chat.clone();
			if let Some(jid) = jid {
				let _ = tx_chat_menu.send(UiCommand::ChooseDisappearingTimer { jid });
			}
		} else if id == MENU_ID_REPORT_CHAT {
			let jid = state_chat_menu.borrow().selected_chat.clone();
			if let Some(jid) = jid {
				let _ = tx_chat_menu.send(UiCommand::ReportChat { jid });
			}
		} else if id == MENU_ID_BLOCK_CHAT {
			let jid = state_chat_menu.borrow().selected_chat.clone();
			if let Some(jid) = jid {
				let _ = tx_chat_menu.send(UiCommand::ToggleBlockChat { jid });
			}
		} else if id == MENU_ID_BLOCKED_CONTACTS {
			let _ = tx_chat_menu.send(UiCommand::ShowBlockedContacts);
		} else if id == MENU_ID_PINNED_MESSAGES {
			crate::ui_commands::message_extras::show_pinned_messages(&state_chat_menu);
		} else if id == MENU_ID_CHAT_PRIVACY {
			crate::ui_commands::message_extras::show_chat_privacy(&state_chat_menu);
		} else if id == MENU_ID_MUTE_CHAT
			|| id == MENU_ID_ARCHIVE_CHAT
			|| id == MENU_ID_PIN_CHAT
			|| id == MENU_ID_LOCK_CHAT
			|| id == MENU_ID_MARK_UNREAD
		{
			let s = state_chat_menu.borrow();
			let Some(jid) = s.selected_chat.clone() else { return };
			if id == MENU_ID_MUTE_CHAT {
				let mute = !s.muted_jids.contains(&jid);
				drop(s);
				let _ = tx_chat_menu.send(UiCommand::ToggleMuteChat { jid, mute });
			} else if id == MENU_ID_ARCHIVE_CHAT {
				let archive = !s.archived_jids.contains(&jid);
				drop(s);
				let _ = tx_chat_menu.send(UiCommand::ToggleArchiveChat { jid, archive });
			} else if id == MENU_ID_PIN_CHAT {
				let pin = !s.pinned_jids.contains(&jid);
				drop(s);
				let _ = tx_chat_menu.send(UiCommand::TogglePinChat { jid, pin });
			} else if id == MENU_ID_LOCK_CHAT {
				let lock = !s.locked_jids.contains(&jid);
				drop(s);
				let _ = tx_chat_menu.send(UiCommand::ToggleLockChat { jid, lock });
			} else {
				let read = s.unread_counts.get(&jid).copied().unwrap_or(0) == 0;
				drop(s);
				let _ = tx_chat_menu.send(UiCommand::ToggleReadChat { jid, read: !read });
			}
		} else if id == MENU_ID_STAR_MESSAGE
			&& let Some(idx) = messages_list_for_menu.get_selection()
		{
			let s = state_chat_menu.borrow();
			let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
				whatsnap::DisplayItem::Single(m) => Some(m.clone()),
				whatsnap::DisplayItem::Album { .. } => None,
			});
			drop(s);
			if let Some(msg) = msg_opt
				&& !msg.is_date_divider
			{
				let sender_jid = if msg.from_me { "me".to_string() } else { msg.sender_jid.clone() };
				let _ = tx_chat_menu.send(UiCommand::StarMessage {
					message_id: msg.id.clone(),
					sender_jid,
					from_me: msg.from_me,
					starred: !msg.starred,
				});
			}
		}
	});

	let tx_voice_menu = ui_tx.clone();
	parts.frame.on_menu_selected(move |evt| {
		let id = evt.get_id();
		if id == MENU_ID_START_RECORDING {
			let _ = tx_voice_menu.send(UiCommand::StartRecording);
		} else if id == MENU_ID_PAUSE_RECORDING {
			let _ = tx_voice_menu.send(UiCommand::TogglePauseRecording);
		} else if id == MENU_ID_SEND_PTT {
			let _ = tx_voice_menu.send(UiCommand::SendVoiceMessage);
		}
	});

	let tx_call_menu = ui_tx.clone();
	parts.frame.on_menu_selected(move |evt| {
		let id = evt.get_id();
		if id == MENU_ID_CALL_MUTE {
			let _ = tx_call_menu.send(UiCommand::MuteCall);
		} else if id == MENU_ID_CALL_END {
			let _ = tx_call_menu.send(UiCommand::LeaveCall);
		} else if id == MENU_ID_CREATE_CALL_LINK {
			let _ = tx_call_menu.send(UiCommand::CreateCallLink);
		} else if id == MENU_ID_JOIN_CALL_LINK {
			let _ = tx_call_menu.send(UiCommand::AskCallLink);
		}
	});

	let settings_dialog_shortcuts = parts.settings_dialog.dialog;
	let state_shortcuts = state.clone();
	let pending_shortcuts_edit = pending_shortcuts.clone();
	parts.settings_dialog.shortcuts_btn.on_click(move |_| {
		let current =
			pending_shortcuts_edit.borrow().clone().unwrap_or_else(|| (*state_shortcuts.borrow().keymap).clone());
		if let Some(updated) = wx_utils::shortcuts::prompt_for_shortcuts(&settings_dialog_shortcuts, &current) {
			*pending_shortcuts_edit.borrow_mut() = Some(updated);
		}
	});

	let settings_dialog_cancel = parts.settings_dialog.dialog;
	parts.settings_dialog.cancel_btn.on_click(move |_| {
		settings_dialog_cancel.end_modal(wxdragon::id::ID_CANCEL);
	});

	for control in parts.settings_dialog.privacy_controls() {
		let Some(except_btn) = control.except_btn else { continue };
		let settings_parts = parts.settings_dialog;
		let name = control.name;
		let find = move || settings_parts.privacy_controls().into_iter().find(|c| c.name == name);
		// Picking "My Contacts Except..." only enables the button: a combo
		// fires this for every arrow-key step through it, so opening the
		// picker from here would pop it up on the way past.
		control.combo.on_selection_changed(move |_| {
			if let Some(control) = find() {
				except_btn.enable(control.is_except());
			}
		});
		let state_except = state.clone();
		except_btn.on_click(move |_| {
			if let Some(control) = find() {
				choose_excluded_contacts(&state_except, settings_parts.dialog, &control);
			}
		});
	}

	let settings_dialog_ok = parts.settings_dialog.dialog;
	let sounds_check_ok = parts.settings_dialog.sounds_check;
	let notifications_check_ok = parts.settings_dialog.notifications_check;
	let follow_dnd_check_ok = parts.settings_dialog.follow_dnd_check;
	let announce_typing_check_ok = parts.settings_dialog.announce_typing_check;
	let announce_recording_check_ok = parts.settings_dialog.announce_recording_check;
	let tray_icon_check_ok = parts.settings_dialog.tray_icon_check;
	let autostart_check_ok = parts.settings_dialog.autostart_check;
	let start_maximized_check_ok = parts.settings_dialog.start_maximized_check;
	let meta_ai_enabled_check_ok = parts.settings_dialog.meta_ai_enabled_check;
	let verbose_logging_check_ok = parts.settings_dialog.verbose_backend_logging_check;
	let name_field_ok = parts.settings_dialog.name_field;
	let settings_parts_ok = parts.settings_dialog;
	let input_device_combo_ok = parts.settings_dialog.input_device_combo;
	let output_device_combo_ok = parts.settings_dialog.output_device_combo;
	let noise_suppression_check_ok = parts.settings_dialog.voice_message_noise_suppression_check;
	let dual_stream_check_ok = parts.settings_dialog.dual_stream_screen_share_check;
	let state_settings_save = state.clone();
	let frame_shortcuts_save = parts.frame;
	parts.settings_dialog.ok_btn.on_click(move |_| {
		use crate::ui::settings_dialog::*;
		let mut s = state_settings_save.borrow_mut();
		// "My Contacts Except..." needs someone to exclude: checked before
		// anything is saved, so Save either saves everything or nothing.
		if s.privacy_settings_loaded {
			for control in settings_parts_ok.privacy_controls() {
				let already_except = s.privacy_settings.get(control.name).map(String::as_str) == Some(EXCEPT_VALUE);
				if control.is_except() && !already_except && !s.privacy_list_edits.contains_key(control.name) {
					drop(s);
					MessageDialog::builder(
						&settings_dialog_ok,
						&format!(
							"Choose the contacts excluded from {} first, with the button under it.",
							control.label
						),
						"Settings",
					)
					.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
					.build()
					.show_modal();
					if let Some(btn) = control.except_btn {
						btn.set_focus();
					}
					return;
				}
			}
		}
		if let Some(keymap) = pending_shortcuts.borrow_mut().take() {
			keymap.apply(&frame_shortcuts_save);
			s.settings.shortcuts = keymap.saved();
			s.keymap = Rc::new(keymap);
		}
		s.settings.sounds_enabled = sounds_check_ok.is_checked();
		s.settings.notifications_enabled = notifications_check_ok.is_checked();
		s.settings.follow_windows_do_not_disturb = follow_dnd_check_ok.is_checked();
		s.settings.announce_typing = announce_typing_check_ok.is_checked();
		s.settings.announce_recording = announce_recording_check_ok.is_checked();
		s.settings.show_tray_icon = tray_icon_check_ok.is_checked();
		s.settings.meta_ai_enabled = meta_ai_enabled_check_ok.is_checked();
		s.settings.verbose_backend_logging = verbose_logging_check_ok.is_checked();
		s.settings.start_maximized = start_maximized_check_ok.is_checked();
		s.settings.voice_message_noise_suppression = noise_suppression_check_ok.is_checked();
		s.settings.dual_stream_screen_share = dual_stream_check_ok.is_checked();
		crate::ui_commands::calls::send_call_options(&s);
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
		// The About and privacy settings are only saved once they've loaded,
		// and only what was changed is sent.
		if s.own_about_loaded {
			let new_about = settings_parts_ok.about_field.get_value();
			let picked = combo_value(&settings_parts_ok.about_duration_combo, ABOUT_DURATION_CHOICES);
			// A time the dropdown can't show is kept unless another is picked.
			let duration = if picked == shown_value(ABOUT_DURATION_CHOICES, &s.own_about_duration.to_string()) {
				s.own_about_duration
			} else {
				picked.parse().unwrap_or(-1)
			};
			if new_about != s.own_about || duration != s.own_about_duration {
				s.own_about = new_about.clone();
				s.own_about_duration = duration;
				let payload =
					serde_json::json!({"action": "set_about", "data": {"text": new_about, "duration": duration}});
				let _ = s.ws_tx.send(payload.to_string());
			}
		}
		if let Some(current) = s.default_timer {
			let picked = combo_value(&settings_parts_ok.default_timer_combo, DEFAULT_TIMER_CHOICES);
			if picked != shown_value(DEFAULT_TIMER_CHOICES, &current.to_string()) {
				let seconds: i64 = picked.parse().unwrap_or(0);
				s.default_timer = Some(seconds);
				let payload = serde_json::json!({"action": "set_default_timer", "data": {"seconds": seconds}});
				let _ = s.ws_tx.send(payload.to_string());
			}
		}
		if s.privacy_settings_loaded {
			for control in settings_parts_ok.privacy_controls() {
				let server = s.privacy_settings.get(control.name).cloned().unwrap_or_default();
				if control.is_except() {
					// Sent whole; the backend works out who to add and remove.
					if let Some(jids) = s.privacy_list_edits.get(control.name) {
						let payload = serde_json::json!({"action": "set_privacy_list", "data": {"name": control.name, "jids": jids}});
						let _ = s.ws_tx.send(payload.to_string());
					}
					continue;
				}
				let value = combo_value(&control.combo, control.choices);
				if value != shown_value(control.choices, &server) {
					s.privacy_settings.insert(control.name.to_string(), value.clone());
					let payload = serde_json::json!({"action": "set_privacy_setting", "data": {"name": control.name, "value": value}});
					let _ = s.ws_tx.send(payload.to_string());
				}
			}
		}
		let logged_in = s.main_ui_shown;
		drop(s);
		// Respect the setting, but never surface the icon while logged out.
		apply_tray_setting(tray_icon_check_ok.is_checked() && logged_in);
		if crate::autostart::is_enabled() != autostart_check_ok.is_checked() {
			crate::autostart::set_enabled(autostart_check_ok.is_checked());
		}
		settings_dialog_ok.end_modal(wxdragon::id::ID_OK);
	});

	// Your own profile photo, changed or removed straight away.
	let state_photo = state.clone();
	let settings_dialog_photo = parts.settings_dialog.dialog;
	parts.settings_dialog.change_photo_btn.on_click(move |_| {
		let file_dialog = FileDialog::builder(&settings_dialog_photo)
			.with_message("Choose a profile photo")
			.with_wildcard("Pictures (*.jpg;*.jpeg;*.png;*.webp;*.bmp;*.gif)|*.jpg;*.jpeg;*.png;*.webp;*.bmp;*.gif")
			.with_style(FileDialogStyle::Open | FileDialogStyle::FileMustExist)
			.build();
		if file_dialog.show_modal() != wxdragon::id::ID_OK {
			return;
		}
		let Some(path) = file_dialog.get_path() else { return };
		let payload = serde_json::json!({"action": "set_own_photo", "data": {"path": path}});
		let _ = state_photo.borrow().ws_tx.send(payload.to_string());
	});
	let state_photo_remove = state.clone();
	parts.settings_dialog.remove_photo_btn.on_click(move |_| {
		let dialog = MessageDialog::builder(&settings_dialog_photo, "Remove your profile photo?", "Profile Photo")
			.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
			.build();
		dialog.set_yes_no_labels("Remove", "Cancel");
		if dialog.show_modal() == ID_YES {
			let payload = serde_json::json!({"action": "set_own_photo", "data": {"remove": true}});
			let _ = state_photo_remove.borrow().ws_tx.send(payload.to_string());
		}
	});

	let tx_group_settings = ui_tx.clone();
	let group_name_field = parts.group_data_dialog.group_name_field;
	let group_desc_field = parts.group_data_dialog.group_desc_field;
	let announce_chk = parts.group_data_dialog.announce_chk;
	let locked_chk = parts.group_data_dialog.locked_chk;
	let forwarded_chk = parts.group_data_dialog.forwarded_chk;
	let admin_reports_chk = parts.group_data_dialog.admin_reports_chk;
	let history_chk = parts.group_data_dialog.history_chk;
	let admins_add_chk = parts.group_data_dialog.admins_add_chk;
	let approve_members_chk = parts.group_data_dialog.approve_members_chk;
	parts.group_data_dialog.save_settings_btn.on_click(move |_| {
		let name = group_name_field.get_value();
		let topic = group_desc_field.get_value();
		let announce = announce_chk.is_checked();
		let locked = locked_chk.is_checked();
		let _ = tx_group_settings.send(UiCommand::UpdateGroupSettings { name, topic, announce, locked });
		let _ = tx_group_settings.send(UiCommand::UpdateGroupModeration {
			no_frequently_forwarded: forwarded_chk.is_checked(),
			allow_admin_reports: admin_reports_chk.is_checked(),
			group_history: history_chk.is_checked(),
			admins_add_only: admins_add_chk.is_checked(),
			approve_members: approve_members_chk.is_checked(),
		});
	});

	let gd = &parts.group_data_dialog;
	for (button, action) in [
		(gd.join_requests_btn, "join_requests"),
		(gd.link_group_btn, "link_group"),
		(gd.unlink_group_btn, "unlink_group"),
		(gd.deactivate_btn, "deactivate_community"),
	] {
		let tx = ui_tx.clone();
		button.on_click(move |_| {
			let _ = tx.send(UiCommand::GroupAction(action));
		});
	}

	let tx_set_tag = ui_tx.clone();
	let tag_field = parts.group_data_dialog.my_tag_field;
	let send_tag = move || {
		let _ = tx_set_tag.send(UiCommand::SetMemberLabel(tag_field.get_value().trim().to_string()));
	};
	let send_tag_enter = send_tag.clone();
	parts.group_data_dialog.set_tag_btn.on_click(move |_| send_tag());
	parts.group_data_dialog.my_tag_field.on_text_enter(move |_| send_tag_enter());

	let tx_reported = ui_tx.clone();
	parts.group_data_dialog.reported_btn.on_click(move |_| {
		let _ = tx_reported.send(UiCommand::ShowReportedMessages);
	});

	let tx_group_members_search = ui_tx.clone();
	let members_search = parts.group_data_dialog.members_search;
	members_search.on_text_changed(move |_| {
		let query = members_search.get_value();
		let _ = tx_group_members_search.send(UiCommand::FilterGroupMembers(query));
	});

	let tx_add_member = ui_tx.clone();
	parts.group_data_dialog.add_member_btn.on_click(move |_| {
		let _ = tx_add_member.send(UiCommand::PrepareAddMembers);
	});

	let tx_make_admin = ui_tx.clone();
	let members_list_admin = parts.group_data_dialog.members_list;
	parts.group_data_dialog.make_admin_btn.on_click(move |_| {
		if let Some(idx) = members_list_admin.get_selection() {
			let _ = tx_make_admin.send(UiCommand::ToggleGroupMemberAdmin(idx));
		}
	});

	// Keeps "Make admin" reading as "Dismiss as admin" when the selected
	// member already is one, matching official WhatsApp's per-member toggle
	// instead of one static label regardless of who's selected.
	let tx_member_selection = ui_tx.clone();
	parts.group_data_dialog.members_list.on_selection_changed(move |_| {
		let _ = tx_member_selection.send(UiCommand::GroupMemberSelectionChanged);
	});

	let tx_remove_member = ui_tx.clone();
	let members_list_remove = parts.group_data_dialog.members_list;
	parts.group_data_dialog.remove_member_btn.on_click(move |_| {
		if let Some(idx) = members_list_remove.get_selection() {
			let _ = tx_remove_member.send(UiCommand::RemoveGroupMember(idx));
		}
	});

	let tx_invite_link = ui_tx.clone();
	parts.group_data_dialog.invite_link_btn.on_click(move |_| {
		let _ = tx_invite_link.send(UiCommand::ShowGroupInviteLink);
	});

	let tx_join_group = ui_tx.clone();
	let groups_list_join = parts.group_data_dialog.groups_list;
	parts.group_data_dialog.join_group_btn.on_click(move |_| {
		if let Some(idx) = groups_list_join.get_selection() {
			let _ = tx_join_group.send(UiCommand::JoinCommunitySubgroup(idx));
		}
	});

	let tx_common_click = ui_tx.clone();
	parts.conversation_data_dialog.common_groups_list.on_item_double_clicked(move |event| {
		if let Some(idx) = event.get_selection() {
			let _ = tx_common_click.send(UiCommand::OpenCommonGroup(idx as usize));
		}
	});
	let tx_common_enter = ui_tx.clone();
	let common_list_enter = parts.conversation_data_dialog.common_groups_list;
	keys::on_enter(&parts.conversation_data_dialog.common_groups_list, Rc::default(), move || {
		if let Some(idx) = common_list_enter.get_selection() {
			let _ = tx_common_enter.send(UiCommand::OpenCommonGroup(idx as usize));
		}
	});
	let tx_chat_filter = ui_tx.clone();
	parts.conversations_parts.chat_filter_combo.on_selection_changed(move |event| {
		if let Some(idx) = event.get_selection() {
			let _ = tx_chat_filter.send(UiCommand::SetChatFilter(idx as usize));
		}
	});

	let tx_group_selection = ui_tx.clone();
	parts.group_data_dialog.groups_list.on_selection_changed(move |_| {
		let _ = tx_group_selection.send(UiCommand::CommunitySubgroupSelectionChanged);
	});
	let tx_group_double_click = ui_tx.clone();
	parts.group_data_dialog.groups_list.on_item_double_clicked(move |event| {
		if let Some(idx) = event.get_selection() {
			let _ = tx_group_double_click.send(UiCommand::JoinCommunitySubgroup(idx as u32));
		}
	});
	// Enter on the row opens or joins it. Caught on key-up: the dialog eats
	// Enter's key-down for its default button (see the contact picker).
	let tx_group_enter = ui_tx.clone();
	let groups_list_enter = parts.group_data_dialog.groups_list;
	keys::on_enter(&parts.group_data_dialog.groups_list, Rc::default(), move || {
		if let Some(idx) = groups_list_enter.get_selection() {
			let _ = tx_group_enter.send(UiCommand::JoinCommunitySubgroup(idx));
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

	let poll_dialog_cancel = parts.poll_dialog.dialog;
	parts.poll_dialog.cancel_btn.on_click(move |_| {
		poll_dialog_cancel.end_modal(ID_CANCEL);
	});
	let tx_poll = ui_tx.clone();
	let poll_dlg = parts.poll_dialog.dialog;
	let poll_question = parts.poll_dialog.question_field;
	let poll_options = parts.poll_dialog.options_field;
	let poll_multiple = parts.poll_dialog.multiple_check;
	let poll_quiz = parts.poll_dialog.quiz_check;
	parts.poll_dialog.send_btn.on_click(move |_| {
		let question = poll_question.get_value();
		match whatsnap::validate_poll(&question, &poll_options.get_value()) {
			Ok(options) => {
				let quiz = poll_quiz.get_value();
				let correct_answer = if quiz {
					let labels: Vec<&str> = options.iter().map(String::as_str).collect();
					let choose =
						SingleChoiceDialog::builder(&poll_dlg, "Which answer is correct?", "Quiz", &labels).build();
					if choose.show_modal() != ID_OK {
						return;
					}
					let Some(answer) = choose.get_string_selection() else { return };
					Some(answer)
				} else {
					None
				};
				poll_dlg.end_modal(ID_OK);
				let _ = tx_poll.send(UiCommand::SendPoll {
					question: question.trim().to_string(),
					options,
					multiple_answers: !quiz && poll_multiple.get_value(),
					correct_answer,
				});
			}
			Err(problem) => {
				unsafe {
					let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
						windows::Win32::UI::WindowsAndMessaging::MB_OK,
					);
				}
				MessageDialog::builder(&poll_dlg, problem, "Poll")
					.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
					.build()
					.show_modal();
				if question.trim().is_empty() || question.trim().chars().count() > whatsnap::POLL_MAX_QUESTION_CHARS {
					poll_question.set_focus();
				} else {
					poll_options.set_focus();
				}
			}
		}
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
	let sync_check = parts.new_contact_dialog.sync_check;
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
		let clean_phone: String = phone.chars().filter(|c| c.is_ascii_digit()).collect();
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
		let sync = sync_check.get_value();
		dlg.end_modal(wxdragon::id::ID_OK);
		// The number goes as typed: without a + it's read as one from your own
		// country, as WhatsApp's own add-contact form does.
		let _ = tx_new_contact.send(UiCommand::NewContact {
			name: full_name,
			first_name: name,
			phone: phone.trim().to_string(),
			sync,
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
		// Let go of the state before the error box below: its modal loop
		// handles incoming backend events, which borrow the state too.
		drop(s);
		let extra_raw = extra_ctrl.get_value();
		for part in extra_raw.split([',', '\n']) {
			let digits: String = part.chars().filter(|c| c.is_ascii_digit()).collect();
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
	let selection_announcer_search = parts.new_group_dialog.selection_announcer;
	let state_search = state.clone();
	// wxCheckListBox::Check() fires an OBJECT_STATECHANGE WinEvent per item
	// (checklst.cpp), which NVDA processes asynchronously. Rebuilding the
	// whole list (clear() + append() + re-Check()) on every single keystroke
	// queued events referencing items that a following keystroke's rebuild
	// had already replaced by the time NVDA got to them, landing NVDA on an
	// index past the new, shorter list and tripping wx's "bad
	// wxCheckListBox index" assert in IsChecked(). Debouncing so the list
	// only rebuilds once typing pauses keeps rebuilds far enough apart for
	// NVDA to finish querying the previous one first. This timer is shared
	// (via `parts.new_group_contacts_timer`) with a fresh `contacts` response
	// landing while the dialog is open, for the same reason - see the
	// "contacts" case in main.rs's websocket message handler.
	let group_search_timer = parts.new_group_contacts_timer.clone();
	let timer_for_tick = group_search_timer.clone();
	timer_for_tick.on_tick(move |_| {
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
		// wxListBox::SetSelection asserts/crashes if given index 0 on an
		// empty list (only wxNOT_FOUND, which wxdragon's u32 API can't
		// express, is valid there) - so there's nothing to reset when the
		// filter has no matches, only when it has at least one.
		if !gs.current_jids.is_empty() {
			contacts_listbox.set_selection(0, true);
		}
		if !gs.checked_jids.is_empty() {
			let count = gs.checked_jids.len();
			crate::announce::announce(
				selection_announcer_search,
				&format!(
					"{} shown. {} contact{} selected.",
					gs.current_jids.len(),
					count,
					if count == 1 { "" } else { "s" }
				),
			);
		}
	});
	search_field.on_text_changed(move |_| {
		group_search_timer.stop();
		group_search_timer.start(250, true);
	});

	let contacts_listbox_toggle = parts.new_group_dialog.contacts_listbox;
	let selection_announcer_toggle = parts.new_group_dialog.selection_announcer;
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
				// Checking someone and then filtering them out of view with
				// a new search reads as "my selection got lost" with no way
				// to otherwise confirm it didn't - this running total is how
				// they can tell it's still there.
				let count = gs.checked_jids.len();
				crate::announce::announce(
					selection_announcer_toggle,
					&format!(
						"{}: {}. {} contact{} selected.",
						label,
						state_str,
						count,
						if count == 1 { "" } else { "s" }
					),
				);
			}
		}
	});

	// Add Members dialog - mirrors the New Group dialog's checkbox-list
	// wiring above (including the debounced search, shared with a fresh
	// `contacts` response, and the crash fixes that motivated it), but
	// confirms with an explicit Add button instead of creating anything.
	let add_members_search_field = parts.add_members_dialog.search_field;
	let add_members_listbox = parts.add_members_dialog.contacts_listbox;
	let add_members_selection_announcer = parts.add_members_dialog.selection_announcer;
	let state_add_members_search = state.clone();
	let add_members_search_timer = parts.add_members_contacts_timer.clone();
	let add_members_timer_tick = add_members_search_timer.clone();
	add_members_timer_tick.on_tick(move |_| {
		let query = add_members_search_field.get_value().to_lowercase().trim().to_string();
		let mut s = state_add_members_search.borrow_mut();
		let ams = &mut s.add_members_state;
		let count = add_members_listbox.get_count();
		for idx in 0..count {
			if idx < ams.current_jids.len() as u32 {
				let jid = ams.current_jids[idx as usize].clone();
				if add_members_listbox.is_checked(idx) {
					ams.checked_jids.insert(jid);
				} else {
					ams.checked_jids.remove(&jid);
				}
			}
		}
		let mut filtered_labels = Vec::new();
		let mut filtered_jids = Vec::new();
		for (lbl, jid) in ams.all_contact_labels.iter().zip(ams.all_contact_jids.iter()) {
			if query.is_empty() || lbl.to_lowercase().contains(&query) {
				filtered_labels.push(lbl.clone());
				filtered_jids.push(jid.clone());
			}
		}
		add_members_listbox.clear();
		for lbl in &filtered_labels {
			add_members_listbox.append(lbl);
		}
		ams.current_jids = filtered_jids;
		for (idx, jid) in ams.current_jids.iter().enumerate() {
			if ams.checked_jids.contains(jid) {
				add_members_listbox.check(idx as u32, true);
			}
		}
		if !ams.current_jids.is_empty() {
			add_members_listbox.set_selection(0, true);
		}
		if !ams.checked_jids.is_empty() {
			let count = ams.checked_jids.len();
			crate::announce::announce(
				add_members_selection_announcer,
				&format!(
					"{} shown. {} contact{} selected.",
					ams.current_jids.len(),
					count,
					if count == 1 { "" } else { "s" }
				),
			);
		}
	});
	add_members_search_field.on_text_changed(move |_| {
		add_members_search_timer.stop();
		add_members_search_timer.start(250, true);
	});

	let add_members_listbox_toggle = parts.add_members_dialog.contacts_listbox;
	let add_members_selection_announcer_toggle = parts.add_members_dialog.selection_announcer;
	let state_add_members_toggle = state.clone();
	add_members_listbox_toggle.on_toggled(move |event| {
		if let Some(idx) = event.get_selection() {
			let mut s = state_add_members_toggle.borrow_mut();
			let ams = &mut s.add_members_state;
			if idx < ams.current_jids.len() as u32 {
				let jid = ams.current_jids[idx as usize].clone();
				let checked = add_members_listbox_toggle.is_checked(idx);
				if checked {
					ams.checked_jids.insert(jid);
				} else {
					ams.checked_jids.remove(&jid);
				}
				let label = add_members_listbox_toggle.get_string(idx as usize).unwrap_or_default();
				let state_str = if checked { "selected" } else { "not selected" };
				let count = ams.checked_jids.len();
				crate::announce::announce(
					add_members_selection_announcer_toggle,
					&format!(
						"{}: {}. {} contact{} selected.",
						label,
						state_str,
						count,
						if count == 1 { "" } else { "s" }
					),
				);
			}
		}
	});

	let tx_add_members_confirm = ui_tx.clone();
	let add_members_listbox_ok = parts.add_members_dialog.contacts_listbox;
	let add_members_dialog_ok = parts.add_members_dialog.dialog;
	let state_add_members_ok = state.clone();
	parts.add_members_dialog.add_btn.on_click(move |_| {
		let mut s = state_add_members_ok.borrow_mut();
		let ams = &mut s.add_members_state;
		let count = add_members_listbox_ok.get_count();
		for idx in 0..count {
			if idx < ams.current_jids.len() as u32 {
				let jid = ams.current_jids[idx as usize].clone();
				if add_members_listbox_ok.is_checked(idx) {
					ams.checked_jids.insert(jid);
				} else {
					ams.checked_jids.remove(&jid);
				}
			}
		}
		let participants: Vec<String> = ams.checked_jids.iter().cloned().collect();
		drop(s);
		if participants.is_empty() {
			unsafe {
				let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
					windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_STYLE(0x00000030),
				);
			}
			MessageDialog::builder(&add_members_dialog_ok, "At least one contact must be selected.", "Error")
				.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
				.build()
				.show_modal();
			add_members_listbox_ok.set_focus();
			return;
		}
		add_members_dialog_ok.end_modal(wxdragon::id::ID_OK);
		let _ = tx_add_members_confirm.send(UiCommand::AddGroupMembers(participants));
	});

	let add_members_cancel = parts.add_members_dialog.dialog;
	parts.add_members_dialog.cancel_btn.on_click(move |_| {
		add_members_cancel.end_modal(0);
	});

	// Attach Contact dialog - mirrors the Add Members dialog's checkbox-list
	// wiring above (debounced search, live-region running total), confirming
	// with "Send" instead of adding group members.
	let attach_contact_search_field = parts.attach_contact_dialog.search_field;
	let attach_contact_listbox = parts.attach_contact_dialog.contacts_listbox;
	let attach_contact_selection_announcer = parts.attach_contact_dialog.selection_announcer;
	let state_attach_contact_search = state.clone();
	let attach_contact_search_timer = parts.attach_contact_timer.clone();
	let attach_contact_timer_tick = attach_contact_search_timer.clone();
	attach_contact_timer_tick.on_tick(move |_| {
		let query = attach_contact_search_field.get_value().to_lowercase().trim().to_string();
		let mut s = state_attach_contact_search.borrow_mut();
		let acs = &mut s.attach_contact_state;
		let count = attach_contact_listbox.get_count();
		for idx in 0..count {
			if idx < acs.current_jids.len() as u32 {
				let jid = acs.current_jids[idx as usize].clone();
				if attach_contact_listbox.is_checked(idx) {
					acs.checked_jids.insert(jid);
				} else {
					acs.checked_jids.remove(&jid);
				}
			}
		}
		let mut filtered_labels = Vec::new();
		let mut filtered_jids = Vec::new();
		for (lbl, jid) in acs.all_contact_labels.iter().zip(acs.all_contact_jids.iter()) {
			if query.is_empty() || lbl.to_lowercase().contains(&query) {
				filtered_labels.push(lbl.clone());
				filtered_jids.push(jid.clone());
			}
		}
		attach_contact_listbox.clear();
		for lbl in &filtered_labels {
			attach_contact_listbox.append(lbl);
		}
		acs.current_jids = filtered_jids;
		for (idx, jid) in acs.current_jids.iter().enumerate() {
			if acs.checked_jids.contains(jid) {
				attach_contact_listbox.check(idx as u32, true);
			}
		}
		if !acs.current_jids.is_empty() {
			attach_contact_listbox.set_selection(0, true);
		}
		if !acs.checked_jids.is_empty() {
			let count = acs.checked_jids.len();
			crate::announce::announce(
				attach_contact_selection_announcer,
				&format!(
					"{} shown. {} contact{} selected.",
					acs.current_jids.len(),
					count,
					if count == 1 { "" } else { "s" }
				),
			);
		}
	});
	attach_contact_search_field.on_text_changed(move |_| {
		attach_contact_search_timer.stop();
		attach_contact_search_timer.start(250, true);
	});

	let attach_contact_listbox_toggle = parts.attach_contact_dialog.contacts_listbox;
	let attach_contact_selection_announcer_toggle = parts.attach_contact_dialog.selection_announcer;
	let state_attach_contact_toggle = state.clone();
	attach_contact_listbox_toggle.on_toggled(move |event| {
		if let Some(idx) = event.get_selection() {
			let mut s = state_attach_contact_toggle.borrow_mut();
			let acs = &mut s.attach_contact_state;
			if idx < acs.current_jids.len() as u32 {
				let jid = acs.current_jids[idx as usize].clone();
				let checked = attach_contact_listbox_toggle.is_checked(idx);
				if checked {
					acs.checked_jids.insert(jid);
				} else {
					acs.checked_jids.remove(&jid);
				}
				let label = attach_contact_listbox_toggle.get_string(idx as usize).unwrap_or_default();
				let state_str = if checked { "selected" } else { "not selected" };
				let count = acs.checked_jids.len();
				crate::announce::announce(
					attach_contact_selection_announcer_toggle,
					&format!(
						"{}: {}. {} contact{} selected.",
						label,
						state_str,
						count,
						if count == 1 { "" } else { "s" }
					),
				);
			}
		}
	});

	let tx_attach_contact_confirm = ui_tx.clone();
	let attach_contact_listbox_ok = parts.attach_contact_dialog.contacts_listbox;
	let attach_contact_dialog_ok = parts.attach_contact_dialog.dialog;
	let state_attach_contact_ok = state.clone();
	parts.attach_contact_dialog.send_btn.on_click(move |_| {
		let mut s = state_attach_contact_ok.borrow_mut();
		let acs = &mut s.attach_contact_state;
		let count = attach_contact_listbox_ok.get_count();
		for idx in 0..count {
			if idx < acs.current_jids.len() as u32 {
				let jid = acs.current_jids[idx as usize].clone();
				if attach_contact_listbox_ok.is_checked(idx) {
					acs.checked_jids.insert(jid);
				} else {
					acs.checked_jids.remove(&jid);
				}
			}
		}
		let contact_jids: Vec<String> = acs.checked_jids.iter().cloned().collect();
		drop(s);
		if contact_jids.is_empty() {
			unsafe {
				let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
					windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_STYLE(0x00000030),
				);
			}
			MessageDialog::builder(&attach_contact_dialog_ok, "At least one contact must be selected.", "Error")
				.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
				.build()
				.show_modal();
			attach_contact_listbox_ok.set_focus();
			return;
		}
		attach_contact_dialog_ok.end_modal(wxdragon::id::ID_OK);
		let _ = tx_attach_contact_confirm.send(UiCommand::SendContacts(contact_jids));
	});

	let attach_contact_cancel = parts.attach_contact_dialog.dialog;
	parts.attach_contact_dialog.cancel_btn.on_click(move |_| {
		attach_contact_cancel.end_modal(0);
	});

	let tx_nav = ui_tx.clone();
	let state_nav = state.clone();
	parts.nav_list.on_selection_changed(move |event| {
		if let Some(idx) = event.get_selection() {
			let tab = state_nav.borrow().settings.tab_order().get(idx as usize).copied().unwrap_or(idx as usize);
			let _ = tx_nav.send(UiCommand::NavSelected(tab));
		}
	});

	// Ctrl+Shift+Left/Right moves the highlighted tab up or down the list, like
	// moving a timeline in Fedra. Swapping the two rows' text in place (rather
	// than clearing and refilling) keeps the list from announcing itself anew.
	let state_nav_move = state.clone();
	let nav_list_move = parts.nav_list;
	let nav_move_announcer = parts.conversations_parts.messages_announcer;
	parts.nav_list.on_key_down(move |event| {
		let WindowEventData::Keyboard(ref key) = event else {
			event.skip(true);
			return;
		};
		let code = key.get_key_code();
		let delta: isize = if crate::ui::keys::is_left(code) {
			-1
		} else if crate::ui::keys::is_right(code) {
			1
		} else {
			event.skip(true);
			return;
		};
		if !(key.control_down() && key.shift_down()) || key.alt_down() {
			event.skip(true);
			return;
		}
		event.skip(false);
		let Some(pos) = nav_list_move.get_selection().map(|p| p as usize) else { return };
		let mut order = state_nav_move.borrow().settings.tab_order();
		let Some(new_pos) = crate::settings::move_tab(&mut order, pos, delta) else {
			crate::announce::announce(
				nav_move_announcer,
				if delta < 0 { "Cannot move left" } else { "Cannot move right" },
			);
			return;
		};
		let names: Vec<&str> = order.iter().map(|&t| crate::settings::TAB_NAMES[t]).collect();
		let unread_chats = crate::message_display::unread_chats_on_chats_tab(&state_nav_move.borrow());
		for p in [pos, new_pos] {
			nav_list_move.set_string(p as u32, &crate::settings::nav_tab_label(order[p], unread_chats));
		}
		nav_list_move.set_selection(new_pos as u32, true);
		{
			let mut s = state_nav_move.borrow_mut();
			s.settings.tab_order = order;
			s.settings.save();
		}
		let msg = match (new_pos.checked_sub(1), names.get(new_pos + 1)) {
			(Some(before), Some(after)) => format!("Moved between {} and {}", names[before], after),
			(None, Some(after)) => format!("Moved before {after}"),
			(Some(before), None) => format!("Moved after {}", names[before]),
			(None, None) => format!("Moved {}", names[new_pos]),
		};
		crate::announce::announce(nav_move_announcer, &msg);
	});

	let tx_send_field = ui_tx.clone();
	let conv_msg_field_esc = parts.conversations_parts.message_field;
	let cancel_edit_btn_field = parts.conversations_parts.cancel_edit_btn;
	let state_field = state.clone();
	let conv_parts_field = parts.conversations_parts;
	conv_msg_field_esc.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key_event) = event {
			let code = key_event.get_key_code();
			// Open @mention suggestions get first say over arrows, Enter, Tab
			// and Escape.
			if crate::ui_commands::mentions::handle_key(
				&state_field,
				&conv_parts_field,
				code,
				key_event.shift_down(),
				key_event.control_down(),
			) {
				event.skip(false);
				return;
			}
			let is_editing = state_field.borrow().editing_message_id.is_some();
			if keys::is_enter(code) {
				// Enter key
				if key_event.shift_down() {
					event.skip(true); // Shift+Enter -> insert newline in multiline text box
				} else {
					let text = conv_msg_field_esc.get_value();
					if is_editing {
						let message_id = state_field.borrow().editing_message_id.clone().unwrap_or_default();
						let _ = tx_send_field.send(UiCommand::EditMessage { message_id, new_text: text });
					} else if !text.is_empty() {
						let _ = tx_send_field.send(UiCommand::SendMessage(text));
					}
					event.skip(false); // Suppress default Enter newline insertion
				}
			} else if keys::is_escape(code) {
				// Escape key: cancel an in-progress edit instead of closing
				// the conversation, so Escape always backs out of whatever
				// the composer is currently doing.
				if is_editing {
					let _ = tx_send_field.send(UiCommand::CancelEdit);
				} else {
					let _ = tx_send_field.send(UiCommand::CloseConversation);
				}
				event.skip(false);
			} else if keys::is_tab(code) && is_editing && !key_event.shift_down() {
				// Forward Tab while editing jumps straight to "Cancel edit" -
				// native traversal would otherwise land on "Add attachment",
				// which edit mode hides rather than removes from the tab
				// order (see the same note on cancel_edit_btn's own handler).
				// Shift+Tab is deliberately NOT overridden here: doing so
				// bounced focus straight back to "Cancel edit" too, and since
				// that button's own Shift+Tab handler sends focus right back
				// to this field, the two together trapped keyboard users in
				// an inescapable two-control loop with no way to tab out of
				// the composer at all.
				cancel_edit_btn_field.set_focus();
				event.skip(false);
			} else if keys::is_letter(code, 'V') && key_event.control_down() {
				// Ctrl+V
				let mut has_files = false;
				if let Some(_locker) = Clipboard::get().locker() {
					let fdo = FileDataObject::new();
					if Clipboard::get().get_data(&fdo) && fdo.get_file_count() > 0 {
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

	// Suggestions stay put while focus is away (a click on one moves focus to
	// the list first) and are worked out afresh from the caret on return.
	let state_mention_focus = state.clone();
	let conv_parts_focus = parts.conversations_parts;
	parts.conversations_parts.message_field.on_set_focus(move |event| {
		crate::ui_commands::mentions::refresh(&state_mention_focus, &conv_parts_focus);
		event.skip(true);
	});
	// A click on a suggestion picks it, the way Enter would.
	let state_mention_click = state.clone();
	let conv_parts_click = parts.conversations_parts;
	parts.conversations_parts.mention_list.on_selection_changed(move |event| {
		if let Some(idx) = event.get_selection() {
			crate::ui_commands::mentions::pick(&state_mention_click, &conv_parts_click, idx as usize);
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
	let composer_hook = parts.conversations_parts.message_field;
	parts.frame.bind_internal(wxdragon::event::EventType::CHAR_HOOK, move |event| {
		let data = WindowEventData::new(event);
		if let WindowEventData::Keyboard(ref key_event) = data
			&& keys::is_escape(key_event.get_key_code())
		{
			// Escape closes open @mention suggestions first; the composer's
			// own key handler does that.
			if state_hook.borrow().mention_popup.is_some() && composer_hook.has_focus() {
				data.skip(true);
				return;
			}
			if state_hook.borrow().chats_unlocked {
				let _ = tx_escape_hook.send(UiCommand::CloseLockedChats);
				data.skip(false);
				return;
			}
			if conv_panel_hook.is_shown() && !search_panel_hook.is_shown() && !state_hook.borrow().selection_mode {
				// This hook runs ahead of whichever control has focus, so
				// it's the one place Escape reliably reaches regardless
				// of where the user's focus is - including mid-edit,
				// which is why the composer's own on_key_down Escape
				// handling for editing never actually ran before: this
				// unconditional CloseConversation always won the race.
				if state_hook.borrow().editing_message_id.is_some() {
					let _ = tx_escape_hook.send(UiCommand::CancelEdit);
				} else {
					let _ = tx_escape_hook.send(UiCommand::CloseConversation);
				}
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

	let state_conv_search = state.clone();
	let tx_search_enter = ui_tx.clone();
	conv_search.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key_event) = event {
			let code = key_event.get_key_code();
			if keys::is_enter(code) && state_conv_search.borrow().active_tab == 6 {
				let _ = tx_search_enter.send(UiCommand::PlaceCall);
				event.skip(false);
				return;
			}
		}
		event.skip(true);
	});

	let tx_open = ui_tx.clone();
	parts.conversations_parts.conversations_list.on_item_double_clicked(move |event| {
		if let Some(idx) = event.get_selection() {
			let _ = tx_open.send(UiCommand::OpenConversation(idx as usize));
		}
	});

	let convs_list_ctx = parts.conversations_parts.conversations_list;
	let state_chat_ctx = state.clone();
	let ui_tx_chat_ctx = ui_tx.clone();
	let parent_frame_chat_ctx = parts.frame;

	// The chat list opens the selected chat from BOTH key-down and key-up, which
	// needs a shared "this Enter release is already spoken for" flag to stay
	// correct. Two separate releases would otherwise each open a chat:
	//
	//  - Plain Enter on the list opens on key-down, and the key-up that follows
	//    would open it a second time. `open_chat_for_jid` has no already-open
	//    guard, so that is two history round trips and two list rebuilds; on the
	//    Status tab it is two `open_media` sends, i.e. two player windows.
	//  - Activating a context-menu item with Enter closes the menu on the key
	//    *down*, leaving the key-up to land on the list underneath, which read it
	//    as "open this chat" - so picking an archived-alert level also opened the
	//    chat.
	//
	// Key-up honours the flag once and clears it. Key-down sets it when it has
	// already opened the chat itself, and clears it for any other key, so a menu
	// dismissed with the mouse (no key-up follows at all) can't leave it armed to
	// swallow a later numpad Enter, which only ever opens from key-up.
	// (`enter_release_handled` is defined above the tray icon handlers, which
	// arm it too.)

	// Menu-bar items (Reconnect Now, Mute Chat, ...) close on Enter's key-down
	// just like the context menu, so their release hits the list too. Arm the
	// same flag - but only for real menu-bar IDs, and only while Enter is
	// actually held: plain Enter on the list also reaches the frame as a menu
	// event (Windows turns it into an IDOK command), and arming on that
	// swallowed the list's own Enter so chats stopped opening; F5 or a mouse
	// click has no Enter release to swallow.
	let state_menu_open = state.clone();
	let frame_menu_open = parts.frame;
	// Popup menus open with the state sometimes still borrowed, so this
	// only tries.
	parts.frame.on_menu_opened(move |_| {
		if let Ok(s) = state_menu_open.try_borrow() {
			refresh_menu_bar(&s, &frame_menu_open);
		}
	});

	let guard_menu_bar = enter_release_handled.clone();
	parts.frame.on_menu_selected(move |evt| {
		if (MENU_ID_SETTINGS..=MENU_ID_JOIN_CALL_LINK).contains(&evt.get_id()) && enter_key_held() {
			guard_menu_bar.set(true);
		}
	});

	let guard_ctx = enter_release_handled.clone();
	let tx_status_menu = ui_tx.clone();
	// The Applications key reaches the list twice: as a key-down, and as a
	// context-menu event once it's released. A menu shown straight away
	// swallows the release, but the Status and Starred menus are shown from
	// the command queue, so the release would open a second one.
	let last_queued_menu =
		Rc::new(std::cell::Cell::new(std::time::Instant::now() - std::time::Duration::from_secs(10)));
	let open_chat_context_menu = Rc::new(move || {
		if let Some(idx) = convs_list_ctx.get_selection() {
			// Only chat rows get the chat menu: a status update or a starred
			// message is not a chat.
			let queued = match state_chat_ctx.borrow().active_tab {
				1 => Some(UiCommand::ShowStatusMenu(idx as usize)),
				2 => Some(UiCommand::ShowStarredMenu(idx as usize)),
				6 => Some(UiCommand::ShowCallsMenu(idx as usize)),
				_ => None,
			};
			if let Some(cmd) = queued {
				if last_queued_menu.get().elapsed() > std::time::Duration::from_millis(700) {
					last_queued_menu.set(std::time::Instant::now());
					let _ = tx_status_menu.send(cmd);
				}
				return;
			}
			let pin_id = 4001;
			let archive_id = 4002;
			let lock_id = 4012;
			let mute_id = 4003;
			let read_id = 4004;
			let delete_id = 4005;
			let clear_id = 4013;
			let export_id = 4006;
			let export_text_id = 4007;
			let leave_group_id = 4008;
			let alert_silent_id = 4009;
			let alert_sound_id = 4010;
			let alert_full_id = 4011;
			let report_id = 4014;
			let block_id = 4015;
			let favorite_id = 4016;
			let lists_id = 4017;

			let s = state_chat_ctx.borrow();
			let jid_opt = s.filtered_chats.get(idx as usize).cloned();
			if let Some(jid) = jid_opt {
				let is_pinned = s.pinned_jids.contains(&jid);
				let is_archived = s.archived_jids.contains(&jid);
				let is_locked = s.locked_jids.contains(&jid);
				let is_muted = s.muted_jids.contains(&jid);
				let is_blocked = s.blocked_jids.contains(&jid);
				let is_favorite = s.favorite_jids.contains(&jid);
				let unread_count = s.unread_counts.get(&jid).copied().unwrap_or(0);
				let is_unread = unread_count > 0;
				let is_group = jid.ends_with("@g.us");
				let chat_name = s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
				let archived_level = s.settings.archived_alert(&jid);
				drop(s);

				let mut menu_builder = Menu::builder();

				let pin_label = if is_pinned { "Un&pin chat" } else { "&Pin chat" };
				let archive_label = if is_archived { "&Unarchive chat" } else { "&Archive chat" };
				let lock_label = if is_locked { "Un&lock chat" } else { "&Lock chat" };
				let mute_label = if is_muted { "&Unmute notifications" } else { "&Mute notifications" };
				let read_label = if is_unread { "Mark as &read" } else { "Mark as &unread" };

				menu_builder = menu_builder
					.append_item(pin_id, pin_label, "Pin or unpin chat")
					.append_item(archive_id, archive_label, "Archive or unarchive chat")
					.append_item(lock_id, lock_label, "Lock or unlock chat")
					.append_item(mute_id, mute_label, "Mute or unmute notifications")
					.append_item(read_id, read_label, "Mark chat as read or unread");
				if !jid.ends_with("@broadcast") {
					let favorite_label = if is_favorite { "Remove from fa&vorites" } else { "Add to fa&vorites" };
					menu_builder = menu_builder
						.append_item(favorite_id, favorite_label, "Show or stop showing this chat under Favorites")
						.append_item(lists_id, "Add to l&ist...", "Choose which of your lists this chat is in");
				}
				menu_builder = menu_builder
					.append_item(export_id, "&Export chat...", "Export this chat's transcript and media")
					.append_item(
						export_text_id,
						"Export &text only (Markdown)...",
						"Export just the message text as a Markdown file, no media",
					);
				if is_group {
					menu_builder = menu_builder.append_item(leave_group_id, "Leave &group...", "Leave this group");
				}
				menu_builder = menu_builder.append_item(
					clear_id,
					"Clea&r chat...",
					"Delete this chat's messages, keeping the chat itself",
				);
				if !jid.ends_with("@newsletter") && !jid.ends_with("@broadcast") {
					let what =
						if is_group { "Report this group to WhatsApp" } else { "Report this contact to WhatsApp" };
					menu_builder = menu_builder.append_item(report_id, "Re&port...", what);
				}
				if jid.ends_with("@s.whatsapp.net") || jid.ends_with("@lid") {
					menu_builder = if is_blocked {
						menu_builder.append_item(
							block_id,
							"Un&block contact",
							"Let this contact message and call you again",
						)
					} else {
						menu_builder.append_item(
							block_id,
							"&Block contact...",
							"Stop this contact messaging and calling you",
						)
					};
				}
				menu_builder = menu_builder.append_item(delete_id, "&Delete chat...", "Delete chat history");

				let mut popup_menu = menu_builder.build();

				// Archived chats are silent by default; this submenu is where one
				// gets dialled back up. It has to be appended after `build()`
				// because only a finished `Menu` can take a submenu, which also
				// means it lands last - no bad thing, since it keeps "Delete
				// chat..." away from the bottom edge of the menu.
				//
				// A submenu item's event is offered to the submenu's own handler
				// and then to the invoking window, never to the parent menu's, so
				// this needs its own `on_selected` rather than riding on the one
				// below. The closure is boxed into the underlying wxMenu, so it
				// outlives `append_submenu` taking ownership of the wrapper.
				if is_archived {
					let alert_menu = Menu::builder()
						.append_radio_item(alert_silent_id, "&Silent", "No sound and no notification for this chat")
						.append_radio_item(
							alert_sound_id,
							"Sound &only",
							"Play the message sound but show no notification",
						)
						.append_radio_item(alert_full_id, "&Notify normally", "Alert exactly like an unarchived chat")
						.build();
					alert_menu.check_item(
						match archived_level {
							whatsnap::ArchivedAlert::Silent => alert_silent_id,
							whatsnap::ArchivedAlert::SoundOnly => alert_sound_id,
							whatsnap::ArchivedAlert::Full => alert_full_id,
						},
						true,
					);

					let ui_tx_alert = ui_tx_chat_ctx.clone();
					let jid_alert = jid.clone();
					alert_menu.on_selected(move |menu_evt| {
						let id = menu_evt.get_id();
						let level = if id == alert_silent_id {
							whatsnap::ArchivedAlert::Silent
						} else if id == alert_sound_id {
							whatsnap::ArchivedAlert::SoundOnly
						} else if id == alert_full_id {
							whatsnap::ArchivedAlert::Full
						} else {
							return;
						};
						let _ = ui_tx_alert.send(UiCommand::SetArchivedAlert { jid: jid_alert.clone(), level });
					});

					popup_menu.append_submenu(
						alert_menu,
						"Archived a&lerts",
						"How this archived chat alerts you about new messages",
					);
				}

				let ui_tx_clone = ui_tx_chat_ctx.clone();
				let jid_clone = jid.clone();
				let parent_clone = parent_frame_chat_ctx;

				popup_menu.on_selected(move |menu_evt| {
					let id = menu_evt.get_id();
					if id == pin_id {
						let _ = ui_tx_clone.send(UiCommand::TogglePinChat { jid: jid_clone.clone(), pin: !is_pinned });
					} else if id == archive_id {
						let _ = ui_tx_clone
							.send(UiCommand::ToggleArchiveChat { jid: jid_clone.clone(), archive: !is_archived });
					} else if id == lock_id {
						let _ =
							ui_tx_clone.send(UiCommand::ToggleLockChat { jid: jid_clone.clone(), lock: !is_locked });
					} else if id == report_id {
						let _ = ui_tx_clone.send(UiCommand::ReportChat { jid: jid_clone.clone() });
					} else if id == favorite_id {
						let _ =
							ui_tx_clone.send(UiCommand::SetFavorite { jid: jid_clone.clone(), favorite: !is_favorite });
					} else if id == lists_id {
						let _ = ui_tx_clone.send(UiCommand::ChooseChatLists(jid_clone.clone()));
					} else if id == block_id {
						let _ = ui_tx_clone.send(UiCommand::ToggleBlockChat { jid: jid_clone.clone() });
					} else if id == mute_id {
						let _ = ui_tx_clone.send(UiCommand::ToggleMuteChat { jid: jid_clone.clone(), mute: !is_muted });
					} else if id == read_id {
						let _ =
							ui_tx_clone.send(UiCommand::ToggleReadChat { jid: jid_clone.clone(), read: !is_unread });
					} else if id == delete_id {
						if is_group {
							let choices: Vec<&str> = vec!["Delete chat only", "Delete chat and exit group"];
							let dialog = SingleChoiceDialog::builder(
								&parent_clone,
								&format!("Delete chat with {}?", chat_name),
								"Confirm Delete",
								&choices,
							)
							.build();
							if dialog.show_modal() == wxdragon::id::ID_OK {
								match dialog.get_string_selection().as_deref() {
									Some("Delete chat only") => {
										let _ = ui_tx_clone.send(UiCommand::DeleteChat { jid: jid_clone.clone() });
									}
									Some("Delete chat and exit group") => {
										let _ = ui_tx_clone
											.send(UiCommand::DeleteChatAndLeaveGroup { jid: jid_clone.clone() });
									}
									_ => {}
								}
							}
						} else {
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
						}
					} else if id == clear_id {
						let dialog = MessageDialog::builder(
							&parent_clone,
							&format!(
								"Delete all messages in the chat with {}? The chat itself will stay in your list.",
								chat_name
							),
							"Confirm Clear Chat",
						)
						.with_style(
							MessageDialogStyle::OK | MessageDialogStyle::Cancel | MessageDialogStyle::IconQuestion,
						)
						.build();
						if dialog.show_modal() == wxdragon::id::ID_OK {
							let _ = ui_tx_clone.send(UiCommand::ClearChat { jid: jid_clone.clone() });
						}
					} else if id == leave_group_id {
						let dialog = MessageDialog::builder(
							&parent_clone,
							&format!("Are you sure you want to leave {}?", chat_name),
							"Leave Group",
						)
						.with_style(
							MessageDialogStyle::OK | MessageDialogStyle::Cancel | MessageDialogStyle::IconQuestion,
						)
						.build();
						if dialog.show_modal() == wxdragon::id::ID_OK {
							let _ = ui_tx_clone.send(UiCommand::LeaveGroup { jid: jid_clone.clone() });
						}
					} else if id == export_id {
						let default_name = format!("{}.zip", sanitize_export_filename(&chat_name));
						let dialog = FileDialog::builder(&parent_clone)
							.with_message("Export chat as")
							.with_default_file(&default_name)
							.with_wildcard("Zip archive (*.zip)|*.zip")
							.with_style(FileDialogStyle::Save | FileDialogStyle::OverwritePrompt)
							.build();
						if dialog.show_modal() == wxdragon::id::ID_OK
							&& let Some(path) = dialog.get_path()
						{
							let _ = ui_tx_clone.send(UiCommand::ExportChat { jid: jid_clone.clone(), dest_path: path });
						}
					} else if id == export_text_id {
						let default_name = format!("{}.md", sanitize_export_filename(&chat_name));
						let dialog = FileDialog::builder(&parent_clone)
							.with_message("Export chat text as")
							.with_default_file(&default_name)
							.with_wildcard("Markdown file (*.md)|*.md")
							.with_style(FileDialogStyle::Save | FileDialogStyle::OverwritePrompt)
							.build();
						if dialog.show_modal() == wxdragon::id::ID_OK
							&& let Some(path) = dialog.get_path()
						{
							let _ =
								ui_tx_clone.send(UiCommand::ExportChatText { jid: jid_clone.clone(), dest_path: path });
						}
					}
				});

				convs_list_ctx.popup_menu(&mut popup_menu, None);
				popup_menu.destroy_menu();
				guard_ctx.set(true);
			}
		}
	});

	// Enter opens the chat. The guard is the app-wide one: closing a context
	// menu or a menu-bar item with Enter leaves its release behind on this
	// list, and that release mustn't open a chat.
	let conv_list_key = parts.conversations_parts.conversations_list;
	let tx_open_enter = ui_tx.clone();
	keys::on_enter(&parts.conversations_parts.conversations_list, enter_release_handled.clone(), move || {
		if let Some(idx) = conv_list_key.get_selection() {
			let _ = tx_open_enter.send(UiCommand::OpenConversation(idx as usize));
		}
	});

	// The menu arms the Enter guard itself once it closes.
	let open_chat_menu_key = open_chat_context_menu.clone();
	parts.conversations_parts.conversations_list.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key_event) = event
			&& keys::is_context_menu_key(key_event)
		{
			open_chat_menu_key.as_ref()();
			event.skip(false);
			return;
		}
		event.skip(true);
	});

	let open_chat_menu_cb = open_chat_context_menu.clone();
	parts.conversations_parts.conversations_list.bind_internal(EventType::CONTEXT_MENU, move |_event| {
		open_chat_menu_cb.as_ref()();
	});

	let tx_play_media = ui_tx.clone();
	let msgs_list_play = parts.conversations_parts.messages_list;
	let state_play = state.clone();
	let play_voice_btn_space = parts.conversations_parts.play_voice_btn;
	let tx_close_list = ui_tx.clone();
	let parent_frame = parts.frame;
	let msgs_announcer = parts.conversations_parts.messages_announcer;
	let msg_input_field = parts.conversations_parts.message_field;
	let search_panel_key = parts.conversations_parts.search_panel;
	let remove_quote_btn_play = parts.conversations_parts.remove_quote_btn;
	let conv_panel_play = parts.conversations_parts.conversation_panel;
	let _conv_panel_key = parts.conversations_parts.conversation_panel;
	let last_left_arrow =
		std::rc::Rc::new(std::cell::Cell::new(std::time::Instant::now() - std::time::Duration::from_secs(10)));
	let msgs_list_ctx = parts.conversations_parts.messages_list;
	let state_ctx = state.clone();
	let parent_frame_ctx = parts.frame;
	let msgs_announcer_ctx = parts.conversations_parts.messages_announcer;
	let msg_input_field_ctx = parts.conversations_parts.message_field;
	let tx_play_media_ctx = ui_tx.clone();
	let remove_quote_btn_ctx = parts.conversations_parts.remove_quote_btn;
	let conv_panel_ctx = parts.conversations_parts.conversation_panel;

	let state_copy = state.clone();
	let tx_copy = ui_tx.clone();
	let msgs_announcer_copy = parts.conversations_parts.messages_announcer;
	let msgs_list_copy = parts.conversations_parts.messages_list;

	let perform_copy_selected = std::rc::Rc::new(move || {
		if let Some(idx) = msgs_list_copy.get_selection() {
			let s = state_copy.borrow();
			if let Some(display_item) = s.display_items.get(idx as usize).cloned() {
				drop(s);
				match display_item {
					whatsnap::DisplayItem::Single(msg) => {
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
									crate::announce::announce(
										msgs_announcer_copy,
										&format!("Copied {} to clipboard.", msg_kind),
									);
								} else {
									crate::announce::announce(msgs_announcer_copy, "Failed to copy file to clipboard.");
								}
							} else {
								let _ =
									tx_copy.send(UiCommand::CopyAlbumToClipboard { message_ids: vec![msg.id.clone()] });
							}
						} else {
							if let Some(_locker) = Clipboard::get().locker()
								&& Clipboard::get().set_text(&msg.text)
							{
								crate::announce::announce(msgs_announcer_copy, "Copied message text to clipboard.");
							}
						}
					}
					whatsnap::DisplayItem::Album { messages, .. } => {
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
			let report_id = 3016;
			let report_admins_id = 3017;

			let s = state_ctx.borrow();
			let display_item = s.display_items.get(idx as usize).cloned();
			let is_group_chat = s.selected_chat.as_ref().map(|j| j.ends_with("@g.us")).unwrap_or(false);
			let is_channel_chat = s.selected_chat.as_ref().is_some_and(|j| j.ends_with("@newsletter"));
			drop(s);

			// A message someone else sent, which can be reported to WhatsApp
			// (or, in a group, to its admins).
			let reportable_id = display_item
				.as_ref()
				.and_then(|item| match item {
					whatsnap::DisplayItem::Single(m) => {
						(!m.from_me && !m.is_date_divider && !m.is_system() && !m.is_deleted && m.media_type != "call")
							.then(|| m.id.clone())
					}
					whatsnap::DisplayItem::Album { messages, .. } => {
						messages.first().filter(|m| !m.from_me).map(|m| m.id.clone())
					}
				})
				.filter(|_| !is_channel_chat);

			let is_starred = display_item
				.as_ref()
				.map(|item| match item {
					whatsnap::DisplayItem::Single(m) => m.starred && !m.is_date_divider,
					whatsnap::DisplayItem::Album { .. } => false,
				})
				.unwrap_or(false);

			let is_from_me = display_item
				.as_ref()
				.map(|item| match item {
					whatsnap::DisplayItem::Single(m) => m.from_me && !m.is_date_divider,
					whatsnap::DisplayItem::Album { messages, .. } => {
						messages.first().map(|m| m.from_me).unwrap_or(false)
					}
				})
				.unwrap_or(false);
			let is_photo_album = display_item
				.as_ref()
				.map(|item| match item {
					whatsnap::DisplayItem::Album { messages, .. } => messages.iter().all(|m| m.media_type == "image"),
					_ => false,
				})
				.unwrap_or(false);
			let is_voice_msg = display_item
				.as_ref()
				.map(|item| match item {
					whatsnap::DisplayItem::Single(m) => {
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
					whatsnap::DisplayItem::Single(m) => {
						m.from_me && !m.is_date_divider && (m.status == "pending" || m.status == "failed")
					}
					whatsnap::DisplayItem::Album { messages, .. } => messages
						.first()
						.map(|m| m.from_me && (m.status == "pending" || m.status == "failed"))
						.unwrap_or(false),
				})
				.unwrap_or(false);

			let is_unreadable = display_item
				.as_ref()
				.map(|item| match item {
					whatsnap::DisplayItem::Single(m) => m.is_unreadable(),
					_ => false,
				})
				.unwrap_or(false);

			let phone_match = display_item.as_ref().and_then(|item| match item {
				whatsnap::DisplayItem::Single(m) => m.first_phone_number(),
				whatsnap::DisplayItem::Album { .. } => None,
			});

			// Only meaningful in a group: a 1:1 chat has no other "sender" to
			// jump to, and messages from you have no other party to reach.
			let sender_jid = if is_group_chat {
				display_item.as_ref().and_then(|item| match item {
					whatsnap::DisplayItem::Single(m)
						if !m.is_date_divider && !m.from_me && !m.sender_jid.is_empty() =>
					{
						Some(m.sender_jid.clone())
					}
					whatsnap::DisplayItem::Album { messages, .. } => messages
						.first()
						.filter(|m| !m.from_me && !m.sender_jid.is_empty())
						.map(|m| m.sender_jid.clone()),
					_ => None,
				})
			} else {
				None
			};
			let sender_name = sender_jid.as_ref().map(|jid| {
				let msg_name = display_item
					.as_ref()
					.map(|item| match item {
						whatsnap::DisplayItem::Single(m) => m.sender_name.clone(),
						whatsnap::DisplayItem::Album { sender_name, .. } => sender_name.clone(),
					})
					.filter(|n| !n.is_empty() && n != jid);

				let s = state_ctx.borrow();
				msg_name.or_else(|| s.contacts.get(jid).cloned()).unwrap_or_else(|| {
					let raw_phone = jid.split('@').next().unwrap_or(jid);
					if !raw_phone.is_empty() && raw_phone.chars().all(|c| c.is_ascii_digit()) {
						if let Some(formatted) = whatsnap::phone::format_international(raw_phone) {
							return formatted;
						}
						return format!("+{}", raw_phone);
					}
					jid.clone()
				})
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
				matches!(display_item, Some(whatsnap::DisplayItem::Single(ref m)) if !m.is_date_divider);
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
			let is_poll_msg =
				matches!(display_item, Some(whatsnap::DisplayItem::Single(ref m)) if m.is_poll() && !m.is_deleted);
			if is_from_me || is_poll_msg {
				menu_builder =
					menu_builder.append_item(info_id, "Message &Info", "View delivery info, or a poll's votes");
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
			if reportable_id.is_some() {
				menu_builder = menu_builder.append_item(report_id, "Re&port...", "Report this message to WhatsApp");
				if is_group_chat {
					menu_builder = menu_builder.append_item(
						report_admins_id,
						"Report to group &admins",
						"Send this message to the group's admins to review",
					);
				}
			}
			let mut popup_menu = menu_builder.build();
			let extras_msg = crate::ui_commands::message_extras::append_message_actions(
				&popup_menu,
				&state_ctx,
				display_item.as_ref(),
			);

			// A channel post has its own actions: pinning it and seeing who
			// reacted or voted, for the channel's owner and admins.
			let channel_post = match &display_item {
				Some(whatsnap::DisplayItem::Single(m)) if !m.is_date_divider && !m.is_deleted => {
					let s = state_ctx.borrow();
					s.selected_chat
						.clone()
						.filter(|jid| crate::ui_commands::links::can_post_in_channel(&s, jid))
						.map(|jid| (jid, m.clone()))
				}
				_ => None,
			};
			if let Some((channel_jid, post)) = channel_post {
				let submenu = crate::ui_commands::channel_admin::channel_post_menu(&state_ctx, &channel_jid, &post);
				popup_menu.append_submenu(submenu, "C&hannel post", "Pin this post, or see who reacted or voted");
			}

			// Same reasoning as the archived-alerts submenu on the chat-list
			// context menu: a submenu can only be appended to an already-built
			// `Menu`, and its items are only ever offered to its own
			// `on_selected`, never the parent's - so this needs its own.
			if let (Some(sender_jid), Some(sender_name)) = (sender_jid.clone(), sender_name.clone()) {
				let message_sender_id = 3101;
				let call_sender_id = 3102;

				let sender_menu = Menu::builder()
					.append_item(
						message_sender_id,
						&format!("&Message {}", sender_name),
						"Open a one-on-one chat with this message's sender",
					)
					.append_item(
						call_sender_id,
						&format!("&Call {}", sender_name),
						"Start a call with this message's sender",
					)
					.build();

				let tx_sender_menu = tx_play_media_ctx.clone();
				let sender_jid_menu = sender_jid.clone();
				sender_menu.on_selected(move |menu_evt| {
					let id = menu_evt.get_id();
					if id == message_sender_id {
						let _ = tx_sender_menu.send(UiCommand::OpenChatByJid(sender_jid_menu.clone()));
					} else if id == call_sender_id {
						let _ = tx_sender_menu.send(UiCommand::OpenChatByJid(sender_jid_menu.clone()));
						let _ = tx_sender_menu.send(UiCommand::PlaceCall);
					}
				});

				popup_menu.append_submenu(
					sender_menu,
					&format!("&User actions for {}", sender_name),
					"Actions for this message's sender",
				);
			}

			let state_clone = state_ctx.clone();
			let parent_frame_clone = parent_frame_ctx;
			let msgs_announcer_clone = msgs_announcer_ctx;
			let msg_input_field_clone = msg_input_field_ctx;
			let tx_play_media_clone = tx_play_media_ctx.clone();
			let phone_match_clone = phone_match.clone();
			let copy_for_menu = copy_for_ctx.clone();

			popup_menu.on_selected(move |menu_evt| {
				let id = menu_evt.get_id();
				if let Some(msg) = extras_msg.as_ref()
					&& crate::ui_commands::message_extras::handle_message_action(
						id,
						&state_clone,
						&parent_frame_clone,
						msg,
					) {
					return;
				}
				if id == reply_id {
					let mut s = state_clone.borrow_mut();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whatsnap::DisplayItem::Single(m) => Some(m.clone()),
						whatsnap::DisplayItem::Album { messages, .. } => messages.first().cloned(),
					});
					if let Some(msg) = msg_opt
						&& !msg.is_date_divider
						&& !msg.is_deleted
					{
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
						crate::announce::announce(msgs_announcer_clone, &status_msg);
						msg_input_field_clone.set_focus();
						remove_quote_btn_ctx.show(true);
						conv_panel_ctx.layout();
						unsafe {
							let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
								windows::Win32::UI::WindowsAndMessaging::MB_OK,
							);
						}
					}
				} else if id == forward_id {
					let s = state_clone.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whatsnap::DisplayItem::Single(m) => Some(m.clone()),
						whatsnap::DisplayItem::Album { messages, .. } => messages.first().cloned(),
					});
					if let Some(msg) = msg_opt
						&& !msg.is_date_divider
						&& !msg.is_deleted
					{
						let mut labels = Vec::new();
						let mut jids = Vec::new();
						for jid in &s.chats {
							if !jid.ends_with("@broadcast") && crate::ui_commands::links::can_forward_to(&s, jid) {
								let name = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.clone());
								labels.push(name);
								jids.push(jid.clone());
							}
						}
						for (jid, name) in &s.contacts {
							if !jid.ends_with("@broadcast")
								&& !jids.contains(jid)
								&& crate::ui_commands::links::can_forward_to(&s, jid)
							{
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

							let contacts_listbox = ListBox::builder(&dlg_panel).with_choices(labels.clone()).build();
							dlg_sizer.add(
								&contacts_listbox,
								1,
								SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right,
								8,
							);

							let btn_panel = Panel::builder(&dlg_panel).build();
							let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
							let ok_btn =
								Button::builder(&btn_panel).with_id(wxdragon::id::ID_OK).with_label("Forward").build();
							let cancel_btn = Button::builder(&btn_panel)
								.with_id(wxdragon::id::ID_CANCEL)
								.with_label("Cancel")
								.build();
							btn_sizer.add(&ok_btn, 0, SizerFlag::Right, 5);
							btn_sizer.add(&cancel_btn, 0, SizerFlag::empty(), 0);
							btn_panel.set_sizer(btn_sizer, true);

							dlg_sizer.add(&btn_panel, 0, SizerFlag::AlignRight | SizerFlag::All, 8);
							dlg_panel.set_sizer(dlg_sizer, true);

							let dlg_esc_search = dlg;
							search_ctrl.on_key_down(keys::escape_cancels(dlg_esc_search));

							let dlg_esc_list = dlg;
							contacts_listbox.on_key_down(keys::escape_cancels(dlg_esc_list));

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

							let dlg_cancel = dlg;
							cancel_btn.on_click(move |_| {
								dlg_cancel.end_modal(0);
							});

							let dlg_ok = dlg;
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
							// The base Dialog has no Drop, so a dialog built per use must be
							// destroyed or it (and every closure bound to it) lives on hidden.
							dlg.destroy();
						}
					}
				} else if id == edit_id {
					let s = state_clone.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whatsnap::DisplayItem::Single(m) => Some(m.clone()),
						whatsnap::DisplayItem::Album { messages, .. } => messages.first().cloned(),
					});
					drop(s);
					if let Some(msg) = msg_opt
						&& !msg.is_date_divider
					{
						let now = std::time::SystemTime::now()
							.duration_since(std::time::UNIX_EPOCH)
							.unwrap_or_default()
							.as_secs() as i64;
						if !msg.is_editable(now) {
							unsafe {
								let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
									windows::Win32::UI::WindowsAndMessaging::MB_ICONHAND,
								);
							}
							let still_eligible_otherwise = msg.from_me && !msg.is_deleted && !msg.is_voice();
							let reason = if still_eligible_otherwise {
								"This message can no longer be edited - the 20 minute edit window has passed."
							} else {
								"Message can't be edited."
							};
							crate::announce::announce(msgs_announcer_clone, reason);
						} else {
							let _ = tx_play_media_clone.send(UiCommand::BeginEditMessage {
								message_id: msg.id.clone(),
								text: msg.text.clone(),
							});
						}
					}
				} else if id == delete_id {
					let s = state_clone.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whatsnap::DisplayItem::Single(m) => Some(m.clone()),
						whatsnap::DisplayItem::Album { messages, .. } => messages.first().cloned(),
					});
					drop(s);
					if let Some(msg) = msg_opt
						&& !msg.is_date_divider
					{
						let is_group_admin = state_clone
							.borrow()
							.selected_chat
							.as_ref()
							.map(|jid| {
								let s = state_clone.borrow();
								// A channel's admins can delete any of its posts.
								s.admin_group_jids.contains(jid)
									|| crate::ui_commands::links::can_post_in_channel(&s, jid)
							})
							.unwrap_or(false);
						let now = std::time::SystemTime::now()
							.duration_since(std::time::UNIX_EPOCH)
							.unwrap_or_default()
							.as_secs() as i64;
						let can_delete_for_everyone =
							(msg.from_me && msg.is_delete_for_everyone_eligible(now)) || is_group_admin;
						if let Some(for_everyone) =
							confirm_delete_choice(&parent_frame_clone, 1, can_delete_for_everyone)
						{
							let sender_jid = if msg.from_me { "me".to_string() } else { msg.sender_jid.clone() };
							let _ = tx_play_media_clone.send(UiCommand::DeleteMessage {
								message_id: msg.id.clone(),
								sender_jid,
								for_everyone,
							});
						}
					}
				} else if id == star_id {
					let s = state_clone.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whatsnap::DisplayItem::Single(m) => Some(m.clone()),
						whatsnap::DisplayItem::Album { .. } => None,
					});
					drop(s);
					if let Some(msg) = msg_opt
						&& !msg.is_date_divider
					{
						let sender_jid = if msg.from_me { "me".to_string() } else { msg.sender_jid.clone() };
						let _ = tx_play_media_clone.send(UiCommand::StarMessage {
							message_id: msg.id.clone(),
							sender_jid,
							from_me: msg.from_me,
							starred: !msg.starred,
						});
					}
				} else if id == report_id || id == report_admins_id {
					if let Some(message_id) = reportable_id.clone() {
						let cmd = if id == report_id {
							UiCommand::ReportMessage { message_id }
						} else {
							UiCommand::ReportToAdmins { message_id }
						};
						let _ = tx_play_media_clone.send(cmd);
					}
				} else if id == info_id {
					let s = state_clone.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whatsnap::DisplayItem::Single(m) => Some(m.clone()),
						whatsnap::DisplayItem::Album { messages, .. } => messages.first().cloned(),
					});
					if let Some(msg) = msg_opt
						&& (msg.from_me || msg.is_poll())
						&& !msg.is_date_divider
					{
						let _ = tx_play_media_clone.send(UiCommand::GetMessageInfo { message_id: msg.id.clone() });
					}
				} else if id == copy_id {
					copy_for_menu.as_ref()();
				} else if id == retry_id {
					let s = state_clone.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whatsnap::DisplayItem::Single(m) => Some(m.clone()),
						whatsnap::DisplayItem::Album { messages, .. } => messages.first().cloned(),
					});
					drop(s);
					if let Some(msg) = msg_opt {
						let _ = tx_play_media_clone.send(UiCommand::RetryMessage { message_id: msg.id.clone() });
					}
				} else if id == resend_id {
					let s = state_clone.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whatsnap::DisplayItem::Single(m) => Some(m.clone()),
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
					if let Some(whatsnap::DisplayItem::Album { messages, .. }) = s.display_items.get(idx as usize) {
						let msg_ids: Vec<String> = messages.iter().map(|m| m.id.clone()).collect();
						if !msg_ids.is_empty() {
							let _ = tx_play_media_clone.send(UiCommand::CopyAlbumToClipboard { message_ids: msg_ids });
						}
					}
				} else if id == copy_number_id {
					if let Some((_, digits)) = phone_match_clone.as_ref()
						&& let Some(_locker) = Clipboard::get().locker()
						&& Clipboard::get().set_text(&format!("+{}", digits))
					{
						crate::announce::announce(msgs_announcer_clone, "Copied phone number to clipboard.");
					}
				} else if id == message_number_id {
					if let Some((_, digits)) = phone_match_clone.as_ref() {
						let _ = tx_play_media_clone.send(UiCommand::OpenChatByPhone(digits.clone()));
					}
				} else if id == react_dialog_id {
					let s = state_clone.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whatsnap::DisplayItem::Single(m) => Some(m.clone()),
						whatsnap::DisplayItem::Album { messages, .. } => messages.first().cloned(),
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

						let dlg_ref = dlg;
						let tx_react = tx_play_media_clone.clone();
						let msg_id = msg.id.clone();
						let sender_jid = if msg.from_me { "me".to_string() } else { msg.sender_jid.clone() };
						let parent_ref = parent_frame_clone;

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
									if custom_dlg.show_modal() == wxdragon::id::ID_OK
										&& let Some(val) = custom_dlg.get_value()
									{
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
						let list_ref = emoji_list;
						keys::on_enter(&emoji_list, Rc::default(), move || {
							if let Some(sel) = list_ref.get_selection() {
								on_select_key(sel as usize);
							}
						});
						emoji_list.on_key_down(keys::escape_cancels(dlg));

						emoji_list.set_focus();
						dlg.show_modal();
						// The base Dialog has no Drop, so a dialog built per use must be
						// destroyed or it (and every closure bound to it) lives on hidden.
						dlg.destroy();
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
			let keymap = state_play.borrow().keymap.clone();
			let pressed = |id| keymap.is_pressed(id, key_event);

			if keys::is_left(code) {
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
								whatsnap::DisplayItem::Single(m) => Some(m.clone()),
								whatsnap::DisplayItem::Album { messages, .. } => messages.first().cloned(),
							});
							if let Some(msg) = msg_opt {
								drop(s);
								if !msg.is_date_divider && (!msg.quoted_id.is_empty() || !msg.quoted_text.is_empty()) {
									// Jump to the quoted message — pass both id and text so handler can try both
									let _ = tx_close_list.send(UiCommand::JumpToSearchResult {
										message_id: msg.quoted_id.clone(),
										message_text: msg.quoted_text.clone(),
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
							whatsnap::DisplayItem::Single(m) => Some(m.clone()),
							whatsnap::DisplayItem::Album { messages, .. } => messages.first().cloned(),
						});
						if let Some(msg) = msg_opt
							&& (!msg.quoted_id.is_empty() || !msg.quoted_text.is_empty())
						{
							let mut target = if !msg.quoted_id.is_empty() {
								s.display_items.iter().position(|item| match item {
									whatsnap::DisplayItem::Single(m) => m.id == msg.quoted_id,
									whatsnap::DisplayItem::Album { messages, .. } => {
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
									|| msg.quoted_text == "Audio"
									|| msg.quoted_text == "[Media]";
								if !is_generic && (idx as usize) < s.display_items.len() {
									target = s.display_items[..idx as usize].iter().rposition(|item| match item {
										whatsnap::DisplayItem::Single(m) => {
											m.text == msg.quoted_text
												|| (!m.text.is_empty() && m.text.contains(&msg.quoted_text))
										}
										whatsnap::DisplayItem::Album { messages, .. } => messages.iter().any(|m| {
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
					event.skip(false);
				} else {
					last_left_arrow.set(now);
					if state_play.borrow().playing_msg_id.is_some() {
						let _ = tx_close_list.send(UiCommand::SeekAudio(-5.0));
					} else if let Some(idx) = msgs_list_play.get_selection() {
						// One press on a reply reads out the message it replies to
						// without moving the cursor; a second press within the
						// window jumps to it (above).
						let said = state_play.borrow().display_items.get(idx as usize).and_then(|item| match item {
							whatsnap::DisplayItem::Single(m) => m.reply_announcement(),
							whatsnap::DisplayItem::Album { messages, .. } => {
								messages.first().and_then(|m| m.reply_announcement())
							}
						});
						if let Some(said) = said {
							crate::announce::announce(msgs_announcer, &said);
						}
					}
					event.skip(false);
				}
			} else if keys::is_right(code) {
				if state_play.borrow().playing_msg_id.is_some() {
					let _ = tx_close_list.send(UiCommand::SeekAudio(5.0));
					event.skip(false);
				} else {
					event.skip(true);
				}
			} else if pressed(message_list::SEEK_BACK) {
				if state_play.borrow().playing_msg_id.is_some() {
					let _ = tx_close_list.send(UiCommand::SeekAudio(-5.0));
					event.skip(false);
				} else {
					event.skip(true);
				}
			} else if pressed(message_list::SEEK_FORWARD) {
				if state_play.borrow().playing_msg_id.is_some() {
					let _ = tx_close_list.send(UiCommand::SeekAudio(5.0));
					event.skip(false);
				} else {
					event.skip(true);
				}
			} else if keys::is_escape(key_event.get_key_code()) {
				// Escape
				let _ = tx_close_list.send(UiCommand::CloseConversation);
				event.skip(false);
			} else if keys::is_home(key_event.get_key_code()) {
				// WXK_HOME (313) -> Oldest message at top. set_selection() is a
				// programmatic call - unlike the real keyboard-driven navigation
				// Up/Down arrows go through, wxWidgets never fires
				// on_selection_changed for it, so the idx==0 check that triggers
				// LoadMoreMessages there never ran for Home on its own (only after
				// a follow-up Down+Up, which IS native navigation). Send it here
				// directly instead of relying on that handler for this case.
				let count = msgs_list_play.get_count();
				if count > 0 {
					msgs_list_play.set_selection(0, true);
					let _ = tx_close_list.send(UiCommand::LoadMoreMessages);
				}
				event.skip(false);
			} else if keys::is_end(key_event.get_key_code()) {
				// WXK_END (312) -> Newest message at bottom
				let count = msgs_list_play.get_count();
				if count > 0 {
					msgs_list_play.set_selection(count - 1, true);
				}
				// A window around a search hit doesn't hold the newest message;
				// this fetches it (a no-op when the list already runs to it).
				let _ = tx_close_list.send(UiCommand::JumpToLatestMessage);
				event.skip(false);
			} else if keys::is_letter(code, 'C') && key_event.control_down() {
				copy_for_key.as_ref()();
				event.skip(false);
			} else if keys::is_letter(key_event.get_key_code(), 'V') && key_event.control_down() {
				// Ctrl+V
				let mut has_files = false;
				if let Some(_locker) = Clipboard::get().locker() {
					let fdo = FileDataObject::new();
					if Clipboard::get().get_data(&fdo) && fdo.get_file_count() > 0 {
						has_files = true;
					}
				}
				if has_files {
					let _ = tx_close_list.send(UiCommand::PasteAttachment);
					event.skip(false);
				} else {
					event.skip(true);
				}
			} else if key_event.get_key_code() == Some(WXK_RETURN) && key_event.shift_down() {
				// Shift+Enter is handled on key-up below (see that handler's
				// comment) - just make sure it doesn't fall through to plain
				// Enter's voice-toggle/call-back handling if this does reach
				// on_key_down in some configuration.
				event.skip(false);
			} else if key_event.get_key_code() == Some(WXK_RETURN) {
				// Enter key: play/pause a selected voice message directly,
				// same as the Space handler below, instead of requiring a
				// Tab to the play button. Non-voice messages keep whatever
				// default Enter behavior the list already had.
				let mut handled = false;
				if let Some(idx) = msgs_list_play.get_selection() {
					let mut s = state_play.borrow_mut();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whatsnap::DisplayItem::Single(m) => Some(m.clone()),
						whatsnap::DisplayItem::Album { messages, .. } => messages.first().cloned(),
					});
					if let Some(msg) = msg_opt {
						// Enter on a call-history row calls that chat back, the same
						// shortcut the Call button gives, so the history is usable
						// without leaving the message list.
						if msg.is_call() {
							handled = true;
							let _ = tx_enter_call.send(UiCommand::PlaceCall);
						}
						let is_voice =
							msg.is_audio || msg.text.starts_with("[Voice Message]") || msg.text.starts_with("[Audio]");
						if !handled && is_voice && !msg.is_deleted {
							handled = true;
							let is_currently_playing = s.playing_msg_id.as_deref() == Some(&msg.id);
							if is_currently_playing {
								if s.is_audio_paused {
									// Resume
									if let Some(ref sink) = s.audio_sink {
										sink.play();
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
								let _ = tx_play_media.send(UiCommand::PlayVoiceMessage(msg.id.clone()));
							}
						}
					}
				}
				event.skip(!handled);
			} else if pressed(message_list::SELECTION_MODE) {
				// Ctrl+Space: enter/exit selection mode.
				let _ = tx_close_list.send(UiCommand::ToggleSelectionMode);
				event.skip(false);
			} else if keys::is_space(key_event.get_key_code()) && !key_event.control_down() {
				// Space
				if let Some(idx) = msgs_list_play.get_selection() {
					let mut s = state_play.borrow_mut();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whatsnap::DisplayItem::Single(m) => Some(m.clone()),
						whatsnap::DisplayItem::Album { messages, .. } => messages.first().cloned(),
					});
					if let Some(msg) = msg_opt {
						let is_voice =
							msg.is_audio || msg.text.starts_with("[Voice Message]") || msg.text.starts_with("[Audio]");
						if !is_voice || msg.is_deleted {
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
							let _ = tx_play_media.send(UiCommand::PlayVoiceMessage(msg.id.clone()));
						}
					}
				}
				event.skip(false);
			} else if pressed(message_list::VOTE) {
				// 'v' -> vote in the poll, or answer the event
				if let Some(idx) = msgs_list_play.get_selection() {
					let s = state_play.borrow();
					if let Some(whatsnap::DisplayItem::Single(m)) = s.display_items.get(idx as usize)
						&& (m.is_poll() || m.is_event())
					{
						let _ = tx_close_list.send(UiCommand::OpenPollVote(m.id.clone()));
					}
				}
				event.skip(false);
			} else if pressed(message_list::REPLY) {
				// 'r' / 'R' -> Reply
				if let Some(idx) = msgs_list_play.get_selection() {
					let mut s = state_play.borrow_mut();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whatsnap::DisplayItem::Single(m) => Some(m.clone()),
						whatsnap::DisplayItem::Album { messages, .. } => messages.first().cloned(),
					});
					if let Some(msg) = msg_opt
						&& !msg.is_date_divider
						&& !msg.is_deleted
					{
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
						crate::announce::announce(msgs_announcer, &status_msg);
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
				event.skip(false);
			} else if pressed(message_list::EDIT) {
				// 'e' / 'E' -> Edit
				if let Some(idx) = msgs_list_play.get_selection() {
					let s = state_play.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whatsnap::DisplayItem::Single(m) => Some(m.clone()),
						whatsnap::DisplayItem::Album { messages, .. } => messages.first().cloned(),
					});
					drop(s);
					if let Some(msg) = msg_opt
						&& !msg.is_date_divider
					{
						let now = std::time::SystemTime::now()
							.duration_since(std::time::UNIX_EPOCH)
							.unwrap_or_default()
							.as_secs() as i64;
						if !msg.is_editable(now) {
							unsafe {
								let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
									windows::Win32::UI::WindowsAndMessaging::MB_ICONHAND,
								);
							}
							let still_eligible_otherwise = msg.from_me && !msg.is_deleted && !msg.is_voice();
							let reason = if still_eligible_otherwise {
								"This message can no longer be edited - the 20 minute edit window has passed."
							} else {
								"Message can't be edited."
							};
							crate::announce::announce(msgs_announcer, reason);
						} else {
							let _ = tx_play_media.send(UiCommand::BeginEditMessage {
								message_id: msg.id.clone(),
								text: msg.text.clone(),
							});
						}
					}
				}
				event.skip(false);
			} else if pressed(message_list::STAR) {
				// 's' / 'S' -> Star/Unstar
				if let Some(idx) = msgs_list_play.get_selection() {
					let s = state_play.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whatsnap::DisplayItem::Single(m) => Some(m.clone()),
						whatsnap::DisplayItem::Album { .. } => None,
					});
					drop(s);
					if let Some(msg) = msg_opt
						&& !msg.is_date_divider
					{
						let sender_jid = if msg.from_me { "me".to_string() } else { msg.sender_jid.clone() };
						let _ = tx_play_media.send(UiCommand::StarMessage {
							message_id: msg.id.clone(),
							sender_jid,
							from_me: msg.from_me,
							starred: !msg.starred,
						});
					}
				}
				event.skip(false);
			} else if pressed(message_list::FORWARD) {
				// 'f' / 'F' -> Forward
				if let Some(idx) = msgs_list_play.get_selection() {
					let s = state_play.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whatsnap::DisplayItem::Single(m) => Some(m.clone()),
						whatsnap::DisplayItem::Album { messages, .. } => messages.first().cloned(),
					});
					if let Some(msg) = msg_opt
						&& !msg.is_date_divider
					{
						let mut labels = Vec::new();
						let mut jids = Vec::new();
						for jid in &s.chats {
							if !jid.ends_with("@broadcast") && crate::ui_commands::links::can_forward_to(&s, jid) {
								let name = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.clone());
								labels.push(name);
								jids.push(jid.clone());
							}
						}
						for (jid, name) in &s.contacts {
							if !jid.ends_with("@broadcast")
								&& !jids.contains(jid)
								&& crate::ui_commands::links::can_forward_to(&s, jid)
							{
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

							let contacts_listbox = ListBox::builder(&dlg_panel).with_choices(labels.clone()).build();
							dlg_sizer.add(
								&contacts_listbox,
								1,
								SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right,
								8,
							);

							let btn_panel = Panel::builder(&dlg_panel).build();
							let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
							let ok_btn =
								Button::builder(&btn_panel).with_id(wxdragon::id::ID_OK).with_label("Forward").build();
							let cancel_btn = Button::builder(&btn_panel)
								.with_id(wxdragon::id::ID_CANCEL)
								.with_label("Cancel")
								.build();
							btn_sizer.add(&ok_btn, 0, SizerFlag::Right, 5);
							btn_sizer.add(&cancel_btn, 0, SizerFlag::empty(), 0);
							btn_panel.set_sizer(btn_sizer, true);

							dlg_sizer.add(&btn_panel, 0, SizerFlag::AlignRight | SizerFlag::All, 8);
							dlg_panel.set_sizer(dlg_sizer, true);

							let dlg_esc_search = dlg;
							search_ctrl.on_key_down(keys::escape_cancels(dlg_esc_search));

							let dlg_esc_list = dlg;
							contacts_listbox.on_key_down(keys::escape_cancels(dlg_esc_list));

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

							let dlg_cancel = dlg;
							cancel_btn.on_click(move |_| {
								dlg_cancel.end_modal(0);
							});

							let dlg_ok = dlg;
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
							// The base Dialog has no Drop, so a dialog built per use must be
							// destroyed or it (and every closure bound to it) lives on hidden.
							dlg.destroy();
						}
					}
				}
				event.skip(false);
			} else if pressed(message_list::DELETE) {
				// Delete key -> Delete
				if let Some(idx) = msgs_list_play.get_selection() {
					let s = state_play.borrow();
					let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
						whatsnap::DisplayItem::Single(m) => Some(m.clone()),
						whatsnap::DisplayItem::Album { messages, .. } => messages.first().cloned(),
					});
					drop(s);
					if let Some(msg) = msg_opt
						&& !msg.is_date_divider
					{
						let is_group_admin = state_play
							.borrow()
							.selected_chat
							.as_ref()
							.map(|jid| {
								let s = state_play.borrow();
								// A channel's admins can delete any of its posts.
								s.admin_group_jids.contains(jid)
									|| crate::ui_commands::links::can_post_in_channel(&s, jid)
							})
							.unwrap_or(false);
						let now = std::time::SystemTime::now()
							.duration_since(std::time::UNIX_EPOCH)
							.unwrap_or_default()
							.as_secs() as i64;
						let can_delete_for_everyone =
							(msg.from_me && msg.is_delete_for_everyone_eligible(now)) || is_group_admin;
						if let Some(for_everyone) = confirm_delete_choice(&parent_frame, 1, can_delete_for_everyone) {
							let sender_jid = if msg.from_me { "me".to_string() } else { msg.sender_jid.clone() };
							let _ = tx_play_media.send(UiCommand::DeleteMessage {
								message_id: msg.id.clone(),
								sender_jid,
								for_everyone,
							});
						}
					}
				}
			} else if pressed(message_list::VIEW_TEXT) && code != Some(WXK_RETURN) {
				show_message_text(&parent_frame, &msgs_list_play, &state_play);
				event.skip(false);
			} else if keys::is_context_menu_key(key_event) {
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
	let tx_invite_enter = ui_tx.clone();
	let msgs_list_enter = parts.conversations_parts.messages_list;
	let state_enter = state.clone();
	let search_panel_enter = parts.conversations_parts.search_panel;
	let msgs_announcer_enter = parts.conversations_parts.messages_announcer;
	let parent_frame_enter = parts.frame;
	parts.conversations_parts.messages_list.on_key_up(move |event| {
		if let WindowEventData::Keyboard(ref key_event) = event {
			let code = key_event.get_key_code();
			if code == Some(WXK_RETURN) && state_enter.borrow().keymap.is_pressed(message_list::VIEW_TEXT, key_event) {
				// Enter-based chords open the message text here, on release: the OS
				// eats the Enter key-down in a list box (other chords use key-down).
				show_message_text(&parent_frame_enter, &msgs_list_enter, &state_enter);
				event.skip(false);
				return;
			}
			if keys::is_enter(code) {
				// Enter or numpad Enter
				if search_panel_enter.is_shown() {
					if let Some(idx) = msgs_list_enter.get_selection() {
						let s = state_enter.borrow();
						let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
							whatsnap::DisplayItem::Single(m) => Some(m.clone()),
							whatsnap::DisplayItem::Album { messages, .. } => messages.first().cloned(),
						});
						if let Some(msg) = msg_opt {
							drop(s);
							if !msg.is_date_divider {
								let _ = tx_jump_enter.send(UiCommand::JumpToSearchResult {
									message_id: msg.id.clone(),
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
							whatsnap::DisplayItem::Single(m) => Some(m.clone()),
							whatsnap::DisplayItem::Album { messages, .. } => messages.first().cloned(),
						});
						drop(s);
						if let Some(msg) = msg_opt {
							if msg.text.starts_with("[Group Invite]") {
								let _ = tx_invite_enter.send(UiCommand::OpenGroupInvite(msg.id.clone()));
								event.skip(false);
								return;
							}
							if msg.text.starts_with("[Channel Admin Invite]") {
								let _ = tx_invite_enter.send(UiCommand::OpenChannelAdminInvite(msg.id.clone()));
								event.skip(false);
								return;
							}
							if (msg.is_poll() || msg.is_event()) && !msg.is_deleted {
								let _ = tx_invite_enter.send(UiCommand::OpenPollVote(msg.id.clone()));
								event.skip(false);
								return;
							}
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

							// A WhatsApp link (a chat, group invite or channel) opens
							// here, the way it would in WhatsApp; anything else goes
							// to the browser.
							let open_url = |url: &str| {
								if whatsnap::parse_whatsapp_link(url).is_some() {
									let _ = tx_invite_enter.send(UiCommand::OpenWhatsAppLink(url.to_string()));
								} else {
									wxdragon::utils::launch_default_browser(
										url,
										wxdragon::utils::BrowserLaunchFlags::Default,
									);
								}
							};
							if urls.len() == 1 {
								open_url(&urls[0]);
							} else if urls.len() > 1 {
								let url_strs: Vec<&str> = urls.iter().map(|s| s.as_str()).collect();
								let dialog = wxdragon::prelude::SingleChoiceDialog::builder(
									&msgs_list_enter,
									"Select a link to open:",
									"Open Link",
									&url_strs,
								)
								.build();
								if dialog.show_modal() == wxdragon::id::ID_OK
									&& let Some(selection) = dialog.get_string_selection()
								{
									open_url(&selection);
								}
							} else if let Some((_, digits)) = msg.first_phone_number() {
								let copy_num_id = 9001;
								let msg_num_id = 9002;
								let mut phone_menu = Menu::builder()
									.append_item(copy_num_id, "&Copy Number", "Copy phone number to clipboard")
									.append_item(msg_num_id, "&Message Number", "Start a chat with this phone number")
									.build();
								let tx_phone = tx_phone_enter.clone();
								let msgs_announcer_phone = msgs_announcer_enter;
								phone_menu.on_selected(move |menu_evt| {
									let sel_id = menu_evt.get_id();
									if sel_id == copy_num_id {
										if let Some(_locker) = Clipboard::get().locker()
											&& Clipboard::get().set_text(&format!("+{}", digits))
										{
											crate::announce::announce(
												msgs_announcer_phone,
												"Copied phone number to clipboard.",
											);
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
	let msgs_list_btn = parts.conversations_parts.messages_list;
	let state_play_btn = state.clone();
	let play_voice_btn_ref = parts.conversations_parts.play_voice_btn;
	parts.conversations_parts.play_voice_btn.on_click(move |_| {
		if let Some(idx) = msgs_list_btn.get_selection() {
			let mut s = state_play_btn.borrow_mut();
			let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
				whatsnap::DisplayItem::Single(m) => Some(m.clone()),
				whatsnap::DisplayItem::Album { messages, .. } => messages.first().cloned(),
			});
			if let Some(msg) = msg_opt {
				let is_currently_playing = s.playing_msg_id.as_deref() == Some(&msg.id);
				if is_currently_playing {
					if s.is_audio_paused {
						// Resume
						if let Some(ref sink) = s.audio_sink {
							sink.play();
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
				let _ = tx_play_btn.send(UiCommand::PlayVoiceMessage(msg.id.clone()));
			}
		}
	});

	let state_sel = state.clone();
	let play_voice_btn_sel = parts.conversations_parts.play_voice_btn;
	let audio_slider_sel = parts.conversations_parts.audio_slider;
	let audio_speed_btn_sel = parts.conversations_parts.audio_speed_btn;
	let audio_progress_label_sel = parts.conversations_parts.audio_progress_label;
	let conv_parts_sel = parts.conversations_parts;
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
	let action_panel_for_timer = parts.conversations_parts.action_panel;
	action_layout_timer.on_tick(move |_| {
		action_panel_for_timer.layout();
	});
	parts.conversations_parts.messages_list.on_selection_changed(move |event| {
		if let Some(idx) = event.get_selection() {
			if idx == 0 {
				let _ = tx_sel.send(UiCommand::LoadMoreMessages);
			} else if idx + 1 == conv_parts_sel.messages_list.get_count() as i32 {
				let _ = tx_sel.send(UiCommand::LoadNewerMessages);
			}
			let s = state_sel.borrow();
			conv_parts_sel.action_panel.freeze();
			let mut need_layout = crate::message_display::update_action_buttons(&s, &conv_parts_sel);

			let msg_opt = s.display_items.get(idx as usize).and_then(|item| match item {
				whatsnap::DisplayItem::Single(m) => Some(m.clone()),
				whatsnap::DisplayItem::Album { messages, .. } => messages.first().cloned(),
			});
			if let Some(msg) = msg_opt {
				let is_voice = match s.display_items.get(idx as usize) {
					Some(whatsnap::DisplayItem::Single(m)) => {
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
						} else {
							play_voice_btn_sel.set_label("Play voice message");
						}
						if !audio_slider_sel.is_shown() {
							audio_slider_sel.show(true);
							audio_speed_btn_sel.show(true);
							audio_speed_btn_sel.set_label(&format!("{:.1}x", s.audio_speed));
							audio_progress_label_sel.show(true);
							need_layout = true;
						}
					} else {
						play_voice_btn_sel.set_label("Play voice message");
						if audio_slider_sel.is_shown() {
							audio_slider_sel.show(false);
							audio_speed_btn_sel.show(false);
							audio_progress_label_sel.show(false);
							need_layout = true;
						}
					}
				} else if audio_slider_sel.is_shown() {
					audio_slider_sel.show(false);
					audio_speed_btn_sel.show(false);
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
					audio_speed_btn_sel.show(false);
					audio_progress_label_sel.show(false);
					need_layout = true;
				}
			}

			// Show/hide action_panel based on whether any action buttons are visible
			let any_action_visible = play_voice_btn_sel.is_shown()
				|| conv_parts_sel.action_retry_btn.is_shown()
				|| conv_parts_sel.action_open_btn.is_shown()
				|| conv_parts_sel.action_save_as_btn.is_shown()
				|| conv_parts_sel.action_download_btn.is_shown()
				|| audio_slider_sel.is_shown()
				|| audio_speed_btn_sel.is_shown();
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
			let mut need_layout = crate::message_display::update_action_buttons(&s, &conv_parts_sel);
			if play_voice_btn_sel.is_shown() {
				play_voice_btn_sel.show(false);
				need_layout = true;
			}
			if audio_slider_sel.is_shown() {
				audio_slider_sel.show(false);
				audio_speed_btn_sel.show(false);
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
			if let Some(id) = s.selection_rows.get(idx as usize).map(|item| item.id().to_string()) {
				if selection_list_toggle.is_checked(idx) {
					s.selected_message_ids.insert(id);
				} else {
					s.selected_message_ids.remove(&id);
				}
				let count = s.selected_message_ids.len();
				selection_count_label_toggle.set_label(&format!("{} selected", count));
			}
		}
	});

	// `messages_select_list` (not `messages_list`) has focus while selection
	// mode is active, so the selection-mode/Escape handling on `messages_list`
	// above never fires here - bind the same exit shortcuts on it directly.
	let tx_selection_list_keys = ui_tx.clone();
	let state_selection_list_keys = state.clone();
	parts.conversations_parts.messages_select_list.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key_event) = event {
			let keymap = state_selection_list_keys.borrow().keymap.clone();
			if keymap.is_pressed(message_list::SELECTION_MODE, key_event) || keys::is_escape(key_event.get_key_code()) {
				let _ = tx_selection_list_keys.send(UiCommand::ToggleSelectionMode);
				event.skip(false);
				return;
			}
		}
		event.skip(true);
	});

	let tx_delete_selection = ui_tx.clone();
	let parent_frame_selection = parts.frame;
	let state_delete_selection = state.clone();
	parts.conversations_parts.selection_delete_btn.on_click(move |_| {
		let (count, can_delete_for_everyone) = {
			let s = state_delete_selection.borrow();
			let ids = &s.selected_message_ids;
			let msgs: Vec<&whatsnap::Msg> = s
				.selection_rows
				.iter()
				.filter(|item| ids.contains(item.id()))
				.flat_map(|item| item.messages())
				.collect();
			let now =
				std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
			let all_from_me_and_eligible =
				!msgs.is_empty() && msgs.iter().all(|m| m.from_me && m.is_delete_for_everyone_eligible(now));
			let is_group_admin = s
				.selected_chat
				.as_ref()
				.map(|jid| s.admin_group_jids.contains(jid) || crate::ui_commands::links::can_post_in_channel(&s, jid))
				.unwrap_or(false);
			(msgs.len(), all_from_me_and_eligible || is_group_admin)
		};
		if count == 0 {
			return;
		}
		if let Some(for_everyone) = confirm_delete_choice(&parent_frame_selection, count, can_delete_for_everyone) {
			let _ = tx_delete_selection.send(UiCommand::DeleteSelectedMessages { for_everyone });
		}
	});

	let tx_close_search = ui_tx.clone();
	parts.conversations_parts.search_close_btn.on_click(move |_| {
		let _ = tx_close_search.send(UiCommand::CloseSearch);
	});

	let tx_search_text = ui_tx.clone();
	let search_field_ref = parts.conversations_parts.search_conv_field;
	let msgs_list_search_focus = parts.conversations_parts.messages_list;
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
					whatsnap::DisplayItem::Single(m) => {
						if !m.is_date_divider {
							selected_idx = i as u32;
							break;
						}
					}
					whatsnap::DisplayItem::Album { .. } => {
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

	let tx_emoji = ui_tx.clone();
	parts.conversations_parts.emoji_btn.on_click(move |_| {
		let _ = tx_emoji.send(UiCommand::OpenEmojiPanel { stickers: false });
	});

	let tx_attach = ui_tx.clone();
	let attach_btn_menu = parts.conversations_parts.add_attachment_btn;
	parts.conversations_parts.add_attachment_btn.on_click(move |_| {
		// WhatsApp's attach menu, in its order.
		const ATTACH_DOCUMENT_ID: i32 = 4201;
		const ATTACH_CONTACT_ID: i32 = 4202;
		const ATTACH_MEDIA_ID: i32 = 4203;
		const ATTACH_AUDIO_ID: i32 = 4204;
		const ATTACH_POLL_ID: i32 = 4205;
		const ATTACH_EVENT_ID: i32 = 4206;
		const ATTACH_STICKER_ID: i32 = 4207;
		let mut popup_menu = Menu::builder()
			.append_item(ATTACH_DOCUMENT_ID, "&Document", "Send any file as a document, photos at full quality")
			.append_item(ATTACH_MEDIA_ID, "&Photos && videos", "Send photos and videos")
			.append_item(ATTACH_AUDIO_ID, "&Audio", "Send audio files to play in the chat")
			.append_item(ATTACH_CONTACT_ID, "&Contact", "Attach a contact from your WhatsApp contacts")
			.append_item(ATTACH_POLL_ID, "P&oll", "Create a poll or a quiz")
			.append_item(ATTACH_EVENT_ID, "&Event", "Create an event people can answer")
			.append_item(
				ATTACH_STICKER_ID,
				"&Sticker",
				"Send a favorite, recent or WhatsApp sticker, or a sticker file",
			)
			.build();
		let tx_menu = tx_attach.clone();
		popup_menu.on_selected(move |menu_evt| {
			let id = menu_evt.get_id();
			let cmd = match id {
				ATTACH_DOCUMENT_ID => Some(UiCommand::AttachFiles(whatsnap::AttachKind::Document)),
				ATTACH_MEDIA_ID => Some(UiCommand::AttachFiles(whatsnap::AttachKind::Media)),
				ATTACH_AUDIO_ID => Some(UiCommand::AttachFiles(whatsnap::AttachKind::Audio)),
				ATTACH_CONTACT_ID => Some(UiCommand::PrepareAttachContact),
				ATTACH_POLL_ID => Some(UiCommand::OpenPollDialog),
				ATTACH_EVENT_ID => Some(UiCommand::OpenEventDialog),
				ATTACH_STICKER_ID => Some(UiCommand::OpenEmojiPanel { stickers: true }),
				_ => None,
			};
			if let Some(cmd) = cmd {
				let _ = tx_menu.send(cmd);
			}
		});
		attach_btn_menu.popup_menu(&mut popup_menu, None);
		popup_menu.destroy_menu();
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
	// Enter in the caption box sends, like the Send button.
	let tx_send_attach_enter = ui_tx.clone();
	caption_field_ref.on_text_enter(move |_| {
		let caption = caption_field_ref.get_value();
		let _ = tx_send_attach_enter.send(UiCommand::SendAttachment { caption });
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

	let tx_retry = ui_tx.clone();
	parts.conversations_parts.action_retry_btn.on_click(move |_| {
		let _ = tx_retry.send(UiCommand::RetrySelectedMessage);
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

	// Tabbing away from here would normally rely on native tab-traversal
	// landing on whatever's next/previous in the panel - but the neighbors
	// on both sides ("Add attachment" before it, "Remove quote" after it)
	// are hidden rather than removed from the tab order while editing (this
	// button is only ever visible during an edit), so native traversal tries
	// to focus a hidden control and silently goes nowhere. Route both
	// directions explicitly instead of depending on that.
	let msg_field_shift_tab = parts.conversations_parts.message_field;
	let send_btn_tab = parts.conversations_parts.send_message_btn;
	parts.conversations_parts.cancel_edit_btn.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key_event) = event
			&& keys::is_tab(key_event.get_key_code())
		{
			if key_event.shift_down() {
				msg_field_shift_tab.set_focus();
			} else {
				send_btn_tab.set_focus();
			}
			event.skip(false);
			return;
		}
		event.skip(true);
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
			if keys::is_left(code) || keys::is_down(code) || keys::is_home(code) {
				let _ = tx_slider_key.send(UiCommand::SeekAudio(-5.0));
				event.skip(false);
			} else if keys::is_right(code) || keys::is_up(code) || keys::is_end(code) {
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

/// Opens the "My Contacts Except..." picker for one privacy setting, starting
/// from the list as last picked or as the server has it. The choice is kept
/// until Settings is saved.
fn choose_excluded_contacts(
	state: &std::rc::Rc<std::cell::RefCell<crate::AppState>>,
	dialog: Dialog,
	control: &crate::ui::settings_dialog::PrivacyControl,
) {
	let tell = |message: &str| {
		MessageDialog::builder(&dialog, message, "Settings")
			.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
			.build()
			.show_modal();
	};
	let start: Result<(Vec<crate::Contact>, Vec<crate::Contact>), String> = {
		let s = state.borrow();
		let loaded = s.privacy_lists.get(control.name);
		let is_except =
			s.privacy_settings.get(control.name).map(String::as_str) == Some(crate::ui::settings_dialog::EXCEPT_VALUE);
		let excluded = if let Some(jids) = s.privacy_list_edits.get(control.name) {
			let known = loaded.and_then(|l| l.as_ref().ok());
			Ok(jids
				.iter()
				.map(|jid| {
					s.all_contacts
						.iter()
						.chain(known.into_iter().flatten())
						.find(|c| c.jid == *jid)
						.cloned()
						.unwrap_or_else(|| crate::Contact {
							jid: jid.clone(),
							name: String::new(),
							phone: String::new(),
						})
				})
				.collect())
		} else {
			match loaded {
				Some(Ok(list)) => Ok(list.clone()),
				// With no list in use there's nothing to lose by starting
				// from an empty one; with one in use, an empty start would
				// drop everyone on it.
				Some(Err(_)) if !is_except => Ok(Vec::new()),
				Some(Err(_)) => Err(format!(
					"Couldn't load who's excluded from {} right now, so it can't be changed. Try again later.",
					control.label
				)),
				None => Err(format!("Still loading who's excluded from {}. Try again in a moment.", control.label)),
			}
		};
		excluded.map(|excluded| (s.all_contacts.clone(), excluded))
	};
	let (contacts, excluded) = match start {
		Ok(start) => start,
		Err(message) => {
			tell(&message);
			return;
		}
	};
	if let Some(jids) =
		crate::ui::privacy_list_picker::pick_excluded_contacts(&dialog, control.label, &contacts, &excluded)
	{
		state.borrow_mut().privacy_list_edits.insert(control.name.to_string(), jids);
	}
}
