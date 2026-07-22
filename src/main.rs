
mod ui;
mod ui_wake;
mod countries;

use std::{
	cell::RefCell,
	rc::Rc,
	sync::{Arc, mpsc, atomic::{AtomicBool, Ordering}},
	thread,
	time::Duration,
};

use tungstenite::{connect, Message, stream::MaybeTlsStream};
use wxdragon::prelude::*;

use crate::ui::window::{bind_input_handlers, build_main_window, WindowParts};
use crate::ui_wake::{UiCommandSender, UiWaker, ID_UI_WAKE};

#[derive(Clone)]
pub enum UiCommand {
	RequestPairing(String),
	SendMessage(String),
	ConversationSelectionChanged(usize),
	ProcessWebSocketMessage(String),
	ExitApp,
	NewContact { name: String, phone: String },
	PrepareNewGroup,
	CreateGroup { name: String, participants: Vec<String> },
	RequestQr,
	CancelQr,
	CancelPairing,
	OpenSettings,
	SaveSettings,
	NavSelected(usize),
	FilterConversations(String),
	OpenConversation(usize),
	SearchInConversation(String),
	SearchNextResult,
	SearchPrevResult,
	ToggleSearchPanel,
	StartRecording,
	TogglePauseRecording,
	SendVoiceMessage,
	DiscardVoiceMessage,
	AddAttachment,
	SendAttachment { caption: String },
	DownloadMedia,
	OpenMedia,
	SaveMediaAs,
	CancelEdit,
	RemoveQuote,
	SendTyping { jid: String, state: String, media: String },
	SubscribePresence(String),
	MarkRead { jid: String, message_ids: Vec<String> },
	LoadMoreMessages,
}

use winzapp_rust::ChatMessage;

struct AppState {
	ws_tx: mpsc::Sender<String>,
	chats: Vec<String>,
	filtered_chats: Vec<String>,
	archived_jids: std::collections::HashSet<String>,
	display_names: std::collections::HashMap<String, String>,
	contacts: std::collections::HashMap<String, String>,
	group_state: crate::ui::new_group::NewGroupState,
	selected_chat: Option<String>,
	messages: Vec<ChatMessage>,
	active_tab: usize,
	search_query: String,
	config: crate::ui::settings::AppConfig,
}

fn apply_chat_filter(s: &mut AppState, parts: &crate::ui::window::WindowParts) {
	s.filtered_chats.clear();
	parts.conversations_parts.conversations_list.clear();
	let lower_query = s.search_query.to_lowercase();
	
	let mut selected_idx_to_restore = None;
	
	for jid in &s.chats {
		let is_archived = s.archived_jids.contains(jid);
		if (s.active_tab == 0 && is_archived) || (s.active_tab == 1 && !is_archived) {
			continue;
		}
		
		let name = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.clone());
		if lower_query.is_empty() || name.to_lowercase().contains(&lower_query) {
			s.filtered_chats.push(jid.clone());
			let display_name = s.display_names.get(jid).cloned().unwrap_or_else(|| name);
			parts.conversations_parts.conversations_list.append(&display_name);
			
			if Some(jid.clone()) == s.selected_chat {
				selected_idx_to_restore = Some(s.filtered_chats.len() - 1);
			}
		}
	}
	
	if let Some(idx) = selected_idx_to_restore {
		parts.conversations_parts.conversations_list.set_selection(idx as u32, false);
	}
}

use std::process::{Command, Stdio};
use std::os::windows::process::CommandExt;

fn main() {
	std::panic::set_hook(Box::new(|info| {
		let _ = std::fs::write("panic.log", format!("{}", info));
	}));

	let exe_dir = std::env::current_exe()
		.ok()
		.and_then(|p| p.parent().map(|d| d.to_path_buf()))
		.unwrap_or_else(|| std::env::current_dir().unwrap());

	let candidate_paths = [
		exe_dir.join("backend").join("backend.exe"),
		exe_dir.join("backend.exe"),
		exe_dir.join("backend/backend.exe"),
		std::path::PathBuf::from("backend").join("backend.exe"),
		std::path::PathBuf::from("backend/backend.exe"),
		std::path::PathBuf::from("backend").join("winzapp_backend.exe"),
		std::path::PathBuf::from("backend/winzapp_backend.exe"),
	];

	let mut backend_exe = None;
	for candidate in &candidate_paths {
		if candidate.exists() {
			backend_exe = Some(candidate.clone());
			break;
		}
	}

	let backend_dir = backend_exe
		.as_ref()
		.and_then(|p| p.parent().map(|d| d.to_path_buf()))
		.unwrap_or_else(|| std::path::PathBuf::from("backend"));

	let db_path = backend_dir.join("session.db");
	let abs_db_path = if db_path.is_absolute() {
		db_path.clone()
	} else if let Ok(cwd) = std::env::current_dir() {
		cwd.join(&db_path)
	} else {
		db_path.clone()
	};
	let db_path_str = abs_db_path.to_str().unwrap_or("session.db").to_string();

	let _ = Command::new("taskkill")
		.args(&["/F", "/IM", "backend.exe"])
		.creation_flags(0x08000000)
		.output();

	let mut bridge_child = if let Some(ref backend_path) = backend_exe {
		Command::new(backend_path)
			.arg("-db")
			.arg(&db_path_str)
			.current_dir(&backend_dir)
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.spawn()
			.ok()
	} else {
		None
	};

	let res = wxdragon::main(|_| {
		let parts = build_main_window();
		let frame = parts.frame;
		let config = crate::ui::settings::AppConfig::load();
		parts.settings_dialog.populate(&config);

		let (ui_tx_raw, ui_rx) = mpsc::channel();
		let ui_alive = Arc::new(AtomicBool::new(true));
		let ui_waker = UiWaker::new(frame, ui_alive.clone());
		let ui_tx = UiCommandSender::new(ui_tx_raw, ui_waker.clone());

		let (ws_out_tx, ws_out_rx) = mpsc::channel::<String>();
		let ui_tx_clone = ui_tx.clone();
		let ui_alive_clone = ui_alive.clone();

		thread::spawn(move || {
			loop {
				if !ui_alive_clone.load(Ordering::SeqCst) {
					break;
				}

				if let Ok((mut socket, _)) = connect("ws://localhost:9000/ws") {
					if let MaybeTlsStream::Plain(s) = socket.get_ref() {
						let _ = s.set_nonblocking(true);
					}
					loop {
						if !ui_alive_clone.load(Ordering::SeqCst) {
							break;
						}

						match socket.read() {
							Ok(Message::Text(text)) => {
								let _ = ui_tx_clone.send(UiCommand::ProcessWebSocketMessage(text.to_string()));
							}
							Ok(_) => {} // ignore other messages
							Err(tungstenite::Error::Io(ref err)) if err.kind() == std::io::ErrorKind::WouldBlock => {} // this is fine
							Err(_) => {
								// Socket is closed or errored, break to reconnect
								break;
							}
						}

						if let Ok(out_msg) = ws_out_rx.try_recv() {
							if let Err(e) = socket.send(Message::Text(out_msg.into())) {
								if let tungstenite::Error::Io(ref err) = e {
									if err.kind() != std::io::ErrorKind::WouldBlock {
										break;
									}
								} else {
									break;
								}
							}
						}

						thread::sleep(Duration::from_millis(50));
					}
				}

				thread::sleep(Duration::from_secs(2));
			}
		});

		let state = Rc::new(RefCell::new(AppState {
			ws_tx: ws_out_tx,
			chats: Vec::new(),
			filtered_chats: Vec::new(),
			archived_jids: std::collections::HashSet::new(),
			display_names: std::collections::HashMap::new(),
			contacts: std::collections::HashMap::new(),
			group_state: crate::ui::new_group::NewGroupState {
				all_contact_labels: Vec::new(),
				all_contact_jids: Vec::new(),
				checked_jids: std::collections::HashSet::new(),
				current_jids: Vec::new(),
			},
			selected_chat: None,
			messages: Vec::new(),
			active_tab: 0,
			search_query: String::new(),
			config,
		}));

		let state_clone = state.clone();
		let parts_clone = Rc::new(parts);
		let parts_ui = parts_clone.clone();

		frame.on_menu_selected(move |event| {
			if event.get_id() == ID_UI_WAKE {
				ui_waker.reset();
				while let Ok(cmd) = ui_rx.try_recv() {
					handle_ui_command(cmd, &state_clone, &parts_ui);
				}
			}
		});

		bind_input_handlers(&parts_clone, ui_tx, &state);

		frame.show(true);
	});
	if let Err(e) = res {
		let _ = std::fs::write("main_error.log", format!("{:?}", e));
	}

	if let Some(ref mut child) = bridge_child {
		let _ = child.kill();
		let _ = child.wait();
	}
}

fn handle_ui_command(cmd: UiCommand, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	match cmd {
		UiCommand::PrepareNewGroup => {
			parts.new_group_dialog.ok_btn.enable(true);
			parts.new_group_dialog.name_field.set_value("");
			parts.new_group_dialog.search_field.set_value("");
			parts.new_group_dialog.extra_number_field.set_value("");
			let (all_labels, all_jids) = {
				let s = state.borrow();
				let mut labels = Vec::new();
				let mut jids = Vec::new();
				for jid in &s.chats {
					if !jid.ends_with("@g.us") && !jid.ends_with("@broadcast") {
						let name = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.clone());
						labels.push(name);
						jids.push(jid.clone());
					}
				}
				(labels, jids)
			};
			{
				let mut s_mut = state.borrow_mut();
				let gs = &mut s_mut.group_state;
				gs.all_contact_labels = all_labels.clone();
				gs.all_contact_jids = all_jids.clone();
				gs.checked_jids.clear();
				gs.current_jids = all_jids;
			}
			parts.new_group_dialog.contacts_listbox.clear();
			for label in all_labels {
				parts.new_group_dialog.contacts_listbox.append(&label);
			}
			parts.new_group_dialog.dialog.show_modal();
		}
		UiCommand::CreateGroup { name, participants } => {
			let payload = serde_json::json!({
				"action": "create_group",
				"data": {
					"name": name,
					"participants": participants
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::NewContact { name, phone } => {
			let payload = serde_json::json!({
				"action": "new_contact",
				"data": {
					"phone": phone,
					"name": name
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::RequestQr => {
			let payload = serde_json::json!({
				"action": "get_qr"
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::CancelQr => {
			let payload = serde_json::json!({
				"action": "cancel_qr"
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::CancelPairing => {
			let payload = serde_json::json!({
				"action": "cancel_pairing"
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::RequestPairing(phone) => {
			let payload = serde_json::json!({
				"action": "pair",
				"data": {
					"phone": phone
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::SendMessage(text) => {
			if let Some(ref jid) = state.borrow().selected_chat {
				let payload = serde_json::json!({
					"action": "send",
					"data": {
						"to": jid,
						"text": text
					}
				});
				let _ = state.borrow().ws_tx.send(payload.to_string());
				parts.conversations_parts.message_field.set_value("");
			}
		}
		UiCommand::ConversationSelectionChanged(idx) => {
			let jid = state.borrow().chats.get(idx).cloned();
			println!("DEBUG: ConversationSelectionChanged idx={}, jid={:?}", idx, jid);
			let mut s = state.borrow_mut();
			s.selected_chat = jid;
			s.messages.clear();
			parts.conversations_parts.messages_list.update_entries(&[], None);
		}
		UiCommand::ProcessWebSocketMessage(raw) => {
			if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
				if let Some(t) = val.get("type").and_then(|v| v.as_str()) {
					match t {
						"connected" => {
							parts.pairing_dialog.show(false);
							parts.qrcode_dialog.show(false);
							parts.login_panel.show(false);
							parts.conversations_parts.panel.show(true);
							parts.content_panel.layout();
							let payload = serde_json::json!({
								"action": "get_chats"
							});
							let _ = state.borrow().ws_tx.send(payload.to_string());
						}
						"pairing_code" => {
							if let Some(code) = val.get("data").and_then(|d| d.get("code")).and_then(|c| c.as_str()) {
								parts.pairing_code_display.set_value(code);
								println!("{{'data': {{'qrcode': {{'pairingCode': '{}'}}}}}}", code);
								parts.pairing_dialog.show(true);
								parts.pairing_code_display.select_all();
								parts.pairing_code_display.set_focus();
								unsafe {
									let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(windows::Win32::UI::WindowsAndMessaging::MB_OK);
								}
								live_region::announce(parts.pairing_announcer.clone(), &format!("Pairing code updated: {}", code));
							}
						}
						"qr" => {
							if let Some(code) = val.get("data").and_then(|d| d.get("code")).and_then(|c| c.as_str()) {
								if let Ok(qr) = qrcode::QrCode::new(code) {
									let image = qr.render::<image::Rgba<u8>>()
										.quiet_zone(true)
										.module_dimensions(5, 5)
										.build();
									let width = image.width();
									let height = image.height();
									let data = image.into_raw();
									let mut png_bytes = Vec::new();
									let encoder = image::codecs::png::PngEncoder::new(&mut png_bytes);
									use image::ImageEncoder;
									if encoder.write_image(&data, width, height, image::ExtendedColorType::Rgba8).is_ok() {
										use base64::{Engine as _, engine::general_purpose};
										let b64 = general_purpose::STANDARD.encode(&png_bytes);
										println!("{{'data': {{'qrcode': {{'base64': 'data:image/png;base64,{}'}}}}}}", b64);
									}
									if let Some(bitmap) = wxdragon::bitmap::Bitmap::from_rgba(&data, width, height) {
										parts.qrcode_display.set_min_size(Size::new(width as i32, height as i32));
										parts.qrcode_display.set_bitmap(&bitmap);
										parts.qrcode_dialog.fit();
										parts.qrcode_dialog.layout();
									}
								}
								parts.qrcode_dialog.show(true);
								unsafe {
									let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(windows::Win32::UI::WindowsAndMessaging::MB_OK);
								}
								live_region::announce(parts.qrcode_announcer.clone(), "QR code updated");
							}
						}
						"error" => {
							if let Some(msg) = val.get("data").and_then(|d| d.get("message")).and_then(|m| m.as_str()) {
								parts.pairing_code_display.set_value(&format!("Error: {}", msg));
								parts.pairing_dialog.show(true);
							}
						}
						"chats" => {
							if let Some(data) = val.get("data") {
								if let Ok(chats_array) = serde_json::from_value::<Vec<serde_json::Value>>(data.clone()) {
									let mut chat_list = Vec::new();
									let mut archived_jids = std::collections::HashSet::new();
									let mut display_names = std::collections::HashMap::new();
									let mut contacts_map = std::collections::HashMap::new();
									for chat in chats_array {
										if let Some(jid) = chat.get("JID").and_then(|j| j.as_str()) {
											let name = chat.get("FullName").and_then(|n| n.as_str()).unwrap_or(jid).to_string();
											chat_list.push(jid.to_string());
											
											let unread = chat.get("UnreadCount").and_then(|u| u.as_u64()).unwrap_or(0);
											let pinned = chat.get("Pinned").and_then(|p| p.as_bool()).unwrap_or(false);
											let muted = chat.get("Muted").and_then(|m| m.as_bool()).unwrap_or(false);
											let archived = chat.get("Archived").and_then(|a| a.as_bool()).unwrap_or(false);
											let preview = chat.get("PreviewSnippet").and_then(|p| p.as_str()).unwrap_or("");
											
											let mut prefix = String::new();
											if archived { 
												prefix.push_str("[Archived] "); 
												archived_jids.insert(jid.to_string());
											}
											if pinned { prefix.push_str("[Pinned] "); }
											if muted { prefix.push_str("[Muted] "); }
											
											let unread_str = if unread > 0 { format!(" ({})", unread) } else { String::new() };
											let preview_str = if !preview.is_empty() { format!(" - {}", preview) } else { String::new() };
											
											let display_name = format!("{}{}{}{}", prefix, name, unread_str, preview_str);
											display_names.insert(jid.to_string(), display_name);
											contacts_map.insert(jid.to_string(), name);
										}
									}
									let mut s = state.borrow_mut();
									s.chats = chat_list;
									s.contacts = contacts_map;
									s.archived_jids = archived_jids;
									s.display_names = display_names;
									apply_chat_filter(&mut s, parts);
								}
							}
						}
						"message" => {
							if let Some(data) = val.get("data") {
								let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
								let from = data.get("from").and_then(|f| f.as_str()).unwrap_or("");
								let text = data.get("text").and_then(|t| t.as_str()).unwrap_or("");
								
								let mut s = state.borrow_mut();
								if Some(jid.to_string()) == s.selected_chat {
									let is_group = jid.ends_with("@g.us");
									let is_from_me = from == "me" || from == "You";
									let chat_msg = ChatMessage {
										id: String::new(),
										text: text.to_string(),
										timestamp: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64,
										from_me: is_from_me,
										sender_jid: from.to_string(),
										sender_name: if is_from_me { "You".to_string() } else { from.to_string() },
									};
									let formatted = chat_msg.format_display(is_group);
									s.messages.push(chat_msg);
									let entries: Vec<(accesskit::NodeId, String)> = s.messages.iter().enumerate().map(|(i, msg)| {
										(accesskit::NodeId((i + 1000) as u64), msg.format_display(is_group))
									}).collect();
									parts.conversations_parts.messages_list.update_entries(&entries, None);
									parts.conversations_parts.messages_list.announce(&formatted);
								}
							}
						}
						"new_contact_status" => {
							if let Some(data) = val.get("data") {
								if let Some(status) = data.get("status").and_then(|s| s.as_str()) {
									if status == "success" {
										parts.new_contact_dialog.dialog.end_modal(wxdragon::id::ID_OK);
										parts.new_conv_dialog.dialog.end_modal(wxdragon::id::ID_OK);
										let payload = serde_json::json!({
											"action": "get_chats"
										});
										let _ = state.borrow().ws_tx.send(payload.to_string());
									} else if status == "error" {
										let err_msg = data.get("error").and_then(|e| e.as_str()).unwrap_or("Unknown error");
										unsafe {
											let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(windows::Win32::UI::WindowsAndMessaging::MB_OK);
										}
										MessageDialog::builder(&parts.new_contact_dialog.dialog, err_msg, "Error").with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError).build().show_modal();
									}
								}
							}
						}
						"create_group_status" | "new_group_status" => {
							if let Some(data) = val.get("data") {
								if let Some(status) = data.get("status").and_then(|s| s.as_str()) {
									if status == "success" {
										parts.new_group_dialog.dialog.end_modal(wxdragon::id::ID_OK);
										parts.new_conv_dialog.dialog.end_modal(wxdragon::id::ID_OK);
										let payload = serde_json::json!({
											"action": "get_chats"
										});
										let _ = state.borrow().ws_tx.send(payload.to_string());
									} else if status == "error" {
										parts.new_group_dialog.ok_btn.enable(true);
										let err_msg = data.get("error").and_then(|e| e.as_str()).unwrap_or("Unknown error");
										unsafe {
											let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_STYLE(0x00000030));
										}
										MessageDialog::builder(&parts.new_group_dialog.dialog, err_msg, "Erro").with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError).build().show_modal();
									}
								}
							}
						}
						"conversation_messages" => {
							if let Some(data) = val.get("data") {
								let before_ts = data.get("before_timestamp").and_then(|b| b.as_i64()).unwrap_or(0);
								if let Some(messages) = data.get("messages").and_then(|m| m.as_array()) {
									let mut parsed_msgs = Vec::new();
									for msg in messages {
										if let Ok(chat_msg) = serde_json::from_value::<ChatMessage>(msg.clone()) {
											parsed_msgs.push(chat_msg);
										}
									}
									let mut s = state.borrow_mut();
									let is_group = s.selected_chat.as_ref().map(|j| j.ends_with("@g.us")).unwrap_or(false);
									if before_ts > 0 {
										parsed_msgs.extend(s.messages.drain(..));
										s.messages = parsed_msgs;
									} else {
										s.messages = parsed_msgs;
									}
									let entries: Vec<(accesskit::NodeId, String)> = s.messages.iter().enumerate().map(|(i, msg)| {
										(accesskit::NodeId((i + 1000) as u64), msg.format_display(is_group))
									}).collect();
									parts.conversations_parts.messages_list.update_entries(&entries, None);
								}
							}
						}
						"search_results" => {
							if let Some(data) = val.get("data") {
								if let Some(results) = data.get("results").and_then(|r| r.as_array()) {
									let mut entries = Vec::new();
									for (i, msg) in results.iter().enumerate() {
										let from = msg.get("from").and_then(|f| f.as_str()).unwrap_or("");
										let text = msg.get("text").and_then(|t| t.as_str()).unwrap_or("");
										let formatted = format!("{}: {}", from, text);
										entries.push((accesskit::NodeId((i + 2000) as u64), formatted));
									}
									parts.conversations_parts.messages_list.update_entries(&entries, None);
								}
							}
						}
						"typing" => {
							if let Some(data) = val.get("data") {
								let chat_jid = data.get("chat_jid").or_else(|| data.get("jid")).and_then(|j| j.as_str()).unwrap_or("");
								let mut sender_jid = data.get("sender_jid").or_else(|| data.get("jid")).and_then(|j| j.as_str()).unwrap_or("");
								if sender_jid.is_empty() {
									sender_jid = chat_jid;
								}
								if chat_jid.is_empty() || sender_jid.is_empty() {
									return;
								}
								let is_typing = data.get("typing").and_then(|t| t.as_bool()).unwrap_or(true);
								let is_recording = data.get("recording").and_then(|r| r.as_bool()).unwrap_or(false);

								let s = state.borrow();
								if Some(chat_jid.to_string()) == s.selected_chat {
									let sender_name = s.contacts.get(sender_jid).cloned().unwrap_or_else(|| sender_jid.to_string());
									if !sender_name.is_empty() {
										let announcement = if is_recording {
											format!("{} is recording audio...", sender_name)
										} else if is_typing {
											format!("{} is typing...", sender_name)
										} else {
											format!("{} stopped typing", sender_name)
										};
										parts.conversations_parts.messages_list.announce(&announcement);
									}
								}
							}
						}
						"presence" => {
							if let Some(data) = val.get("data") {
								let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
								if jid.is_empty() {
									return;
								}
								let clean_jid = if let Some((user, server)) = jid.split_once('@') {
									let base_user = user.split('.').next().unwrap_or(user).split(':').next().unwrap_or(user);
									format!("{}@{}", base_user, server)
								} else {
									jid.to_string()
								};
								let unavailable = data.get("unavailable").and_then(|u| u.as_bool()).unwrap_or(false);
								let s = state.borrow();
								if Some(clean_jid.clone()) == s.selected_chat || Some(jid.to_string()) == s.selected_chat {
									let name = s.contacts.get(&clean_jid).or_else(|| s.contacts.get(jid)).cloned().unwrap_or_else(|| clean_jid.clone());
									let status_str = if unavailable { "offline" } else { "online" };
									parts.conversations_parts.messages_list.announce(&format!("{} is now {}", name, status_str));
								}
							}
						}
						"receipt" => {
							if let Some(data) = val.get("data") {
								let chat_jid = data.get("chat_jid").and_then(|j| j.as_str()).unwrap_or("");
								let receipt_type = data.get("type").and_then(|t| t.as_str()).unwrap_or("");
								let s = state.borrow();
								if Some(chat_jid.to_string()) == s.selected_chat {
									if receipt_type == "read" || receipt_type == "read-self" {
										parts.conversations_parts.messages_list.announce("Messages read");
									}
								}
							}
						}
						"media_downloaded" => {
							if let Some(data) = val.get("data") {
								let path = data.get("path").and_then(|p| p.as_str()).unwrap_or("unknown path");
								parts.conversations_parts.messages_list.announce(&format!("Media downloaded to {}", path));
							}
						}
						_ => {}
					}
				}
			}
		}
		UiCommand::ExitApp => {
			parts.frame.close(true);
		}
		UiCommand::OpenSettings => {
			let cfg = state.borrow().config.clone();
			parts.settings_dialog.populate(&cfg);
			parts.settings_dialog.dialog.show_modal();
		}
		UiCommand::SaveSettings => {
			let new_config = parts.settings_dialog.read_config();
			let _ = new_config.save();
			state.borrow_mut().config = new_config.clone();
			let payload = serde_json::json!({
				"action": "update_settings",
				"data": new_config
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::NavSelected(idx) => {
			match idx {
				0 => { // Conversations
					parts.conversations_parts.panel.show(true);
					parts.settings_dialog.dialog.show(false);
					state.borrow_mut().active_tab = 0;
					apply_chat_filter(&mut state.borrow_mut(), parts);
				}
				1 => { // Archived Chats
					parts.conversations_parts.panel.show(true);
					parts.settings_dialog.dialog.show(false);
					state.borrow_mut().active_tab = 1;
					apply_chat_filter(&mut state.borrow_mut(), parts);
				}
				2 => { // Settings
					parts.settings_dialog.dialog.show_modal();
				}
				_ => {}
			}
			parts.content_panel.layout();
		}
		UiCommand::FilterConversations(query) => {
			let mut s = state.borrow_mut();
			s.search_query = query;
			apply_chat_filter(&mut s, parts);
		}
		UiCommand::OpenConversation(idx) => {
			let jid = state.borrow().filtered_chats.get(idx).cloned();
			if let Some(ref jid) = jid {
				let mut s = state.borrow_mut();
				s.selected_chat = Some(jid.clone());
				s.messages.clear();
				parts.conversations_parts.conversation_panel.show(true);
				parts.conversations_parts.messages_list.update_entries(&[], None);
				parts.conversations_parts.message_field.set_value("");
				parts.conversations_parts.send_message_btn.show(true);
				parts.conversations_parts.panel.layout();
				let payload = serde_json::json!({
					"action": "get_messages",
					"data": { "jid": jid, "limit": 50 }
				});
				let _ = s.ws_tx.send(payload.to_string());
				drop(s);
				handle_ui_command(UiCommand::SubscribePresence(jid.clone()), state, parts);
				handle_ui_command(UiCommand::MarkRead { jid: jid.clone(), message_ids: Vec::new() }, state, parts);
				parts.conversations_parts.message_field.set_focus();
			}
		}
		UiCommand::LoadMoreMessages => {
			let s = state.borrow();
			if s.messages.is_empty() {
				return;
			}
			if let Some(ref jid) = s.selected_chat {
				if let Some(oldest_msg) = s.messages.first() {
					let payload = serde_json::json!({
						"action": "get_messages",
						"data": {
							"jid": jid,
							"limit": 50,
							"before_timestamp": oldest_msg.timestamp,
							"before_id": oldest_msg.id
						}
					});
					let _ = s.ws_tx.send(payload.to_string());
				}
			}
		}
		UiCommand::SearchInConversation(query) => {
			if let Some(ref jid) = state.borrow().selected_chat {
				let payload = serde_json::json!({
					"action": "search_messages",
					"data": { "jid": jid, "query": query }
				});
				let _ = state.borrow().ws_tx.send(payload.to_string());
			}
		}
		UiCommand::SearchNextResult => {
			if let Some(ref jid) = state.borrow().selected_chat {
				let payload = serde_json::json!({
					"action": "search_next",
					"data": { "jid": jid }
				});
				let _ = state.borrow().ws_tx.send(payload.to_string());
			}
		}
		UiCommand::SearchPrevResult => {
			if let Some(ref jid) = state.borrow().selected_chat {
				let payload = serde_json::json!({
					"action": "search_prev",
					"data": { "jid": jid }
				});
				let _ = state.borrow().ws_tx.send(payload.to_string());
			}
		}
		UiCommand::ToggleSearchPanel => {
			let visible = parts.conversations_parts.search_panel.is_shown();
			parts.conversations_parts.search_panel.show(!visible);
			if !visible {
				parts.conversations_parts.search_conv_field.set_value("");
				parts.conversations_parts.search_conv_field.set_focus();
			}
			parts.conversations_parts.conversation_panel.layout();
		}
		UiCommand::StartRecording => {
			if let Some(ref jid) = state.borrow().selected_chat {
				let payload = serde_json::json!({
					"action": "start_recording",
					"data": { "jid": jid }
				});
				let _ = state.borrow().ws_tx.send(payload.to_string());
				parts.conversations_parts.voice_panel.show(true);
				parts.conversations_parts.record_voice_btn.show(false);
				parts.conversations_parts.conversation_panel.layout();
			}
		}
		UiCommand::TogglePauseRecording => {
			let payload = serde_json::json!({ "action": "toggle_pause_recording" });
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::SendVoiceMessage => {
			let payload = serde_json::json!({ "action": "send_voice_message" });
			let _ = state.borrow().ws_tx.send(payload.to_string());
			parts.conversations_parts.voice_panel.show(false);
			parts.conversations_parts.record_voice_btn.show(true);
			parts.conversations_parts.conversation_panel.layout();
		}
		UiCommand::DiscardVoiceMessage => {
			let payload = serde_json::json!({ "action": "discard_voice_message" });
			let _ = state.borrow().ws_tx.send(payload.to_string());
			parts.conversations_parts.voice_panel.show(false);
			parts.conversations_parts.record_voice_btn.show(true);
			parts.conversations_parts.conversation_panel.layout();
		}
		UiCommand::AddAttachment => {
			let dialog = FileDialog::builder(&parts.frame)
				.with_message("Choose file(s) to attach")
				.with_style(FileDialogStyle::Open | FileDialogStyle::FileMustExist)
				.build();
			if dialog.show_modal() == wxdragon::id::ID_OK {
				if let Some(path) = dialog.get_path() {
					if !path.is_empty() {
						parts.conversations_parts.attachment_panel.show(true);
						parts.conversations_parts.caption_field.set_value("");
						parts.conversations_parts.caption_field.set_focus();
						parts.conversations_parts.conversation_panel.layout();
					}
				}
			}
		}
		UiCommand::SendAttachment { caption } => {
			if let Some(ref jid) = state.borrow().selected_chat {
				let payload = serde_json::json!({
					"action": "send_attachment",
					"data": { "jid": jid, "caption": caption }
				});
				let _ = state.borrow().ws_tx.send(payload.to_string());
				parts.conversations_parts.attachment_panel.show(false);
				parts.conversations_parts.conversation_panel.layout();
			}
		}
		UiCommand::DownloadMedia => {
			let payload = serde_json::json!({ "action": "download_media" });
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::OpenMedia => {
			let payload = serde_json::json!({ "action": "open_media" });
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::SaveMediaAs => {
			let dialog = FileDialog::builder(&parts.frame)
				.with_message("Save file as")
				.with_style(FileDialogStyle::Save | FileDialogStyle::OverwritePrompt)
				.build();
			if dialog.show_modal() == wxdragon::id::ID_OK {
				if let Some(path) = dialog.get_path() {
					let payload = serde_json::json!({
						"action": "save_media_as",
						"data": { "path": path }
					});
					let _ = state.borrow().ws_tx.send(payload.to_string());
				}
			}
		}
		UiCommand::CancelEdit => {
			parts.conversations_parts.message_field.set_value("");
			parts.conversations_parts.cancel_edit_btn.show(false);
			parts.conversations_parts.conversation_panel.layout();
			parts.conversations_parts.message_field.set_focus();
		}
		UiCommand::RemoveQuote => {
			parts.conversations_parts.remove_quote_btn.show(false);
			parts.conversations_parts.conversation_panel.layout();
		}
		UiCommand::SendTyping { jid, state: typing_state, media } => {
			let target_jid = if jid.is_empty() {
				state.borrow().selected_chat.clone()
			} else {
				Some(jid)
			};
			if let Some(target) = target_jid {
				let payload = serde_json::json!({
					"action": "send_typing",
					"data": {
						"jid": target,
						"state": typing_state,
						"media": media
					}
				});
				let _ = state.borrow().ws_tx.send(payload.to_string());
			}
		}
		UiCommand::SubscribePresence(jid) => {
			let payload = serde_json::json!({
				"action": "subscribe_presence",
				"data": {
					"jid": jid
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::MarkRead { jid, message_ids } => {
			let payload = serde_json::json!({
				"action": "mark_read",
				"data": {
					"jid": jid,
					"message_ids": message_ids
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
	}
}

