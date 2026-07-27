#![windows_subsystem = "windows"]

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
	NavSelected(usize),
	FilterConversations(String),
	OpenConversation(usize),
	SearchInConversation(String),
	SearchNextResult,
	SearchPrevResult,
	ToggleSearchPanel,
	StartRecording,
	TogglePauseRecording,
	StopRecording,
	DiscardRecording,
	ForceSync,
	SendVoiceMessage,
	DiscardVoiceMessage,
	AddAttachment,
	SendAttachment { caption: String },
	DownloadMedia,
	OpenMedia,
	SaveMediaAs,
	CancelEdit,
	RemoveQuote,
	EditMessage { message_id: String, new_text: String },
	DeleteMessage { message_id: String, sender_jid: String, for_everyone: bool },
	SendTyping { jid: String, state: String, media: String },
	SubscribePresence(String),
	MarkRead { jid: String, message_ids: Vec<String> },
	LoadMoreMessages,
	PlayVoiceMessage(String),
	CloseConversation,
}

use whantv::ChatMessage;
use rodio::{OutputStream, Sink, Decoder};
use std::fs::File;
use std::io::BufReader;

struct AppState {
	ws_tx: mpsc::Sender<String>,
	audio_stream: Option<(OutputStream, rodio::OutputStreamHandle)>,
	audio_sink: Option<Sink>,
	playing_msg_id: Option<String>,
	audio_process: Option<std::process::Child>,
	audio_stdin: Option<std::process::ChildStdin>,
	is_audio_paused: bool,
	chats: Vec<String>,
	filtered_chats: Vec<String>,
	archived_jids: std::collections::HashSet<String>,
	display_names: std::collections::HashMap<String, String>,
	contacts: std::collections::HashMap<String, String>,
	group_state: crate::ui::new_group::NewGroupState,
	selected_chat: Option<String>,
	messages: Vec<ChatMessage>,
	loading_messages: bool,
	no_more_messages: bool,
	history_loading: bool,
	completed_history_chats: std::collections::HashSet<String>,
	prev_msg_count: usize,
	active_tab: usize,
	search_query: String,
	config: crate::ui::settings::AppConfig,
	pending_attachments: Vec<String>,
	pending_reply_id: Option<String>,
	pending_reply_sender: Option<String>,
	pending_reply_preview: Option<String>,
	pending_receipts: std::collections::HashMap<String, String>,
}

fn apply_chat_filter(s: &mut AppState, parts: &crate::ui::window::WindowParts) {
	s.filtered_chats.clear();
	parts.conversations_parts.conversations_list.freeze();
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
	parts.conversations_parts.conversations_list.thaw();
	
	if let Some(idx) = selected_idx_to_restore {
		let count = parts.conversations_parts.conversations_list.get_count();
		if count > 0 && (idx as u32) < count {
			parts.conversations_parts.conversations_list.set_selection(idx as u32, true);
		}
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
		std::path::PathBuf::from("backend").join("whantv_backend.exe"),
		std::path::PathBuf::from("backend/whantv_backend.exe"),
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
			audio_stream: None,
			audio_sink: None,
			playing_msg_id: None,
			audio_process: None,
			audio_stdin: None,
			is_audio_paused: false,
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
			loading_messages: false,
			no_more_messages: false,
			history_loading: false,
			completed_history_chats: std::collections::HashSet::new(),
			prev_msg_count: 0,
			active_tab: 0,
			search_query: String::new(),
			config,
			pending_attachments: Vec::new(),
			pending_reply_id: None,
			pending_reply_sender: None,
			pending_reply_preview: None,
			pending_receipts: std::collections::HashMap::new(),
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
		if let Ok(mut stream) = std::net::TcpStream::connect("127.0.0.1:9000") {
			use std::io::Write;
			let _ = stream.write_all(b"POST /shutdown HTTP/1.1\r\nHost: 127.0.0.1:9000\r\nContent-Length: 0\r\n\r\n");
			std::thread::sleep(std::time::Duration::from_millis(600));
		}
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
			let mut s = state.borrow_mut();
			if let Some(ref jid) = s.selected_chat.clone() {
				let reply_id = s.pending_reply_id.take().unwrap_or_default();
				let reply_sender = s.pending_reply_sender.take().unwrap_or_default();
				s.pending_reply_preview = None;

				let payload = serde_json::json!({
					"action": "send",
					"data": {
						"to": jid,
						"text": text,
						"reply_to_id": reply_id,
						"reply_to_sender": reply_sender
					}
				});
				let _ = s.ws_tx.send(payload.to_string());
				parts.conversations_parts.message_field.set_value("");
			}
		}
		UiCommand::ConversationSelectionChanged(idx) => {
			let jid = state.borrow().filtered_chats.get(idx).cloned();
			println!("DEBUG: ConversationSelectionChanged idx={}, jid={:?}", idx, jid);
			let mut s = state.borrow_mut();
			s.selected_chat = jid;
			s.messages.clear();
// 			parts.conversations_parts.messages_list.update_entries(&[], None);
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
								let mut s = state.borrow_mut();
								let is_group = s.selected_chat.as_ref().map(|j| j.ends_with("@g.us")).unwrap_or(false);
								let mut temp_idx = None;
								for (idx, m) in s.messages.iter().enumerate() {
									if m.id.starts_with("temp_upload_") {
										temp_idx = Some(idx);
										break;
									}
								}
								if let Some(idx) = temp_idx {
									s.messages[idx].text = format!("[Upload failed: {}]", msg);
									parts.conversations_parts.messages_list.set_string(idx as u32, &s.messages[idx].format_display(is_group));
									let formatted = s.messages[idx].format_display(is_group);
									live_region::announce(parts.conversations_parts.messages_announcer.clone(), &formatted);
								}
								MessageDialog::builder(&parts.frame, msg, "Error")
									.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
									.build()
									.show_modal();
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
									let is_from_me = data.get("from_me").and_then(|f| f.as_bool()).unwrap_or_else(|| from == "me" || from == "You");
									let is_group = jid.ends_with("@g.us");
									let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
									let msg_id = data.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();

									let mut temp_idx = None;
									if is_from_me {
										for (idx, msg) in s.messages.iter().enumerate() {
											if msg.id.starts_with("temp_upload_") {
												temp_idx = Some(idx);
												break;
											}
										}
									}

									if let Some(idx) = temp_idx {
										s.messages[idx].id = msg_id;
										s.messages[idx].text = text.to_string();
										parts.conversations_parts.messages_list.set_string(idx as u32, &s.messages[idx].format_display(is_group));
										let formatted = s.messages[idx].format_display(is_group);
										live_region::announce(parts.conversations_parts.messages_announcer.clone(), &formatted);
									} else {
										let sender_name = if is_from_me {
											"You".to_string()
										} else {
											let mut clean_jid = from.to_string();
											if let Some(idx) = from.find(':') {
												clean_jid = format!("{}@s.whatsapp.net", &from[..idx]);
											}
											s.contacts.get(&clean_jid).or_else(|| s.contacts.get(from)).cloned().unwrap_or_else(|| clean_jid)
										};
										
										let mut insert_date = false;
										let mut date_str = String::new();
										if let Some(datetime) = chrono::DateTime::from_timestamp(ts, 0) {
											let local_datetime: chrono::DateTime<chrono::Local> = datetime.into();
											date_str = local_datetime.format("%A, %B %e, %Y").to_string();
											if let Some(last_msg) = s.messages.last() {
												if !last_msg.is_date_divider {
													if let Some(last_dt) = chrono::DateTime::from_timestamp(last_msg.timestamp, 0) {
														let last_local: chrono::DateTime<chrono::Local> = last_dt.into();
														let last_date_str = last_local.format("%A, %B %e, %Y").to_string();
														if date_str != last_date_str {
															insert_date = true;
														}
													}
												}
											} else {
												insert_date = true;
											}
										}
										
										let quoted_id = data.get("quoted_id").or_else(|| data.get("quotedId")).and_then(|q| q.as_str()).unwrap_or("").to_string();
										let quoted_text = data.get("quoted_text").or_else(|| data.get("quotedText")).and_then(|q| q.as_str()).unwrap_or("").to_string();
										let quoted_sender = data.get("quoted_sender").or_else(|| data.get("quotedSender")).and_then(|q| q.as_str()).unwrap_or("").to_string();

										if insert_date && !date_str.is_empty() {
											let divider = ChatMessage {
												id: String::new(),
												text: date_str,
												timestamp: ts,
												from_me: false,
												sender_jid: String::new(),
												sender_name: String::new(),
												is_audio: false,
												is_date_divider: true,
												is_deleted: false,
												status: String::new(),
												reactions: Vec::new(),
												quoted_id: String::new(),
												quoted_text: String::new(),
												quoted_sender: String::new(),
											};
											s.messages.push(divider.clone());
											parts.conversations_parts.messages_list.append(&divider.format_display(is_group));
										}
										
										let chat_msg = ChatMessage {
											id: msg_id.clone(),
											text: text.to_string(),
											timestamp: ts,
											from_me: is_from_me,
											sender_jid: from.to_string(),
											sender_name,
											is_audio: text.starts_with("[Voice Message]") || text.starts_with("[Audio]"),
											is_date_divider: false,
											is_deleted: false,
											status: s.pending_receipts.remove(&msg_id).unwrap_or_else(|| "sent".to_string()),
											reactions: Vec::new(),
											quoted_id,
											quoted_text,
											quoted_sender,
										};
										s.messages.push(chat_msg.clone());

										let current_selection = parts.conversations_parts.messages_list.get_selection();
										let was_at_bottom = current_selection.map(|idx| idx as usize == s.messages.len() - (if insert_date { 3 } else { 2 })).unwrap_or(true);
										
										parts.conversations_parts.messages_list.append(&chat_msg.format_display(is_group));
										
										let count = parts.conversations_parts.messages_list.get_count();
										if count > 0 {
											if was_at_bottom || is_from_me {
												parts.conversations_parts.messages_list.set_selection((count - 1) as u32, true);
											}
										}
										let formatted = chat_msg.format_display(is_group);
										live_region::announce(parts.conversations_parts.messages_announcer.clone(), &formatted);

										if !is_from_me {
											let mark_jid = jid.to_string();
											let mark_msg_id = msg_id.clone();
											let tx = s.ws_tx.clone();
											let payload = serde_json::json!({
												"action": "mark_read",
												"data": { "jid": mark_jid, "message_ids": vec![mark_msg_id] }
											});
											let _ = tx.send(payload.to_string());
										}
									}
								}
							}
						}
						"message_deleted" => {
							if let Some(data) = val.get("data") {
								let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
								let message_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("");
								
								let mut s = state.borrow_mut();
								if Some(jid.to_string()) == s.selected_chat {
									let is_group = jid.ends_with("@g.us");
									let mut changed = false;
									for (i, msg) in s.messages.iter_mut().enumerate() {
										if msg.id == message_id {
											msg.is_deleted = true;
											msg.text = "".to_string(); // Clear text just in case
											parts.conversations_parts.messages_list.set_string(i as u32, &msg.format_display(is_group));
											changed = true;
											break;
										}
									}
									if changed {
										live_region::announce(parts.qrcode_announcer.clone(), "Message deleted.");
									}
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
										MessageDialog::builder(&parts.new_group_dialog.dialog, err_msg, "Error").with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError).build().show_modal();
									}
								}
							}
						}
						"conversation_messages" => {
							if let Some(data) = val.get("data") {
								if let Some(messages) = data.get("messages").and_then(|m| m.as_array()) {
									let mut parsed_msgs = Vec::new();
									for msg in messages {
										match serde_json::from_value::<ChatMessage>(msg.clone()) {
											Ok(chat_msg) => parsed_msgs.push(chat_msg),
											Err(e) => {
												let err_msg = ChatMessage {
													id: String::new(),
													text: format!("Parse error: {} for json: {}", e, msg),
													timestamp: 0,
													from_me: false,
													sender_jid: String::new(),
													sender_name: String::new(),
													is_audio: false,
													is_date_divider: false,
													is_deleted: false,
													status: String::new(),
													reactions: Vec::new(),
													quoted_id: String::new(),
													quoted_text: String::new(),
													quoted_sender: String::new(),
												};
												parsed_msgs.push(err_msg);
											}
										}
									}
									let mut seen_ids = std::collections::HashSet::new();
									let mut deduplicated = Vec::new();
									for msg in parsed_msgs {
										if !msg.id.is_empty() {
											if seen_ids.insert(msg.id.clone()) {
												deduplicated.push(msg);
											}
										} else {
											deduplicated.push(msg);
										}
									}
									
									let mut with_dates = Vec::new();
									let mut last_date = String::new();
									for msg in deduplicated {
										if let Some(datetime) = chrono::DateTime::from_timestamp(msg.timestamp, 0) {
											let local_datetime: chrono::DateTime<chrono::Local> = datetime.into();
											let date_str = local_datetime.format("%A, %B %e, %Y").to_string();
											if date_str != last_date {
												last_date = date_str.clone();
												with_dates.push(ChatMessage {
													id: String::new(),
													text: date_str,
													timestamp: msg.timestamp,
													from_me: false,
													sender_jid: String::new(),
													sender_name: String::new(),
													is_audio: false,
													is_date_divider: true,
													is_deleted: false,
													status: String::new(),
													reactions: Vec::new(),
													quoted_id: String::new(),
													quoted_text: String::new(),
													quoted_sender: String::new(),
												});
											}
										}
										with_dates.push(msg);
									}
									
									let mut s = state.borrow_mut();
									s.loading_messages = false;
									let was_history_loading = s.history_loading;
									s.history_loading = false;
									let old_count = s.prev_msg_count;
									let is_group = s.selected_chat.as_ref().map(|j| j.ends_with("@g.us")).unwrap_or(false);
									s.messages = with_dates;
									let new_len = s.messages.len();
									s.prev_msg_count = new_len;
									
									parts.conversations_parts.messages_list.freeze();
									parts.conversations_parts.messages_list.clear();
									for msg in &s.messages {
										parts.conversations_parts.messages_list.append(&msg.format_display(is_group));
									}
									parts.conversations_parts.messages_list.thaw();
									
									let count = parts.conversations_parts.messages_list.get_count();
									if count > 0 {
										if was_history_loading && new_len > old_count && old_count > 0 {
											let keep_idx = (new_len - old_count) as u32;
											if keep_idx < count {
												parts.conversations_parts.messages_list.set_selection(keep_idx, true);
											} else {
												parts.conversations_parts.messages_list.set_selection(0, true);
											}
										} else if old_count == 0 {
											parts.conversations_parts.messages_list.set_selection((count - 1) as u32, true);
											parts.conversations_parts.messages_list.set_focus();
										}
									}
									
									if was_history_loading && new_len == old_count {
										if let Some(ref jid) = s.selected_chat.clone() {
											eprintln!("[LOG UI] No new history messages received for {}. Marking history complete.", jid);
											s.completed_history_chats.insert(jid.clone());
										}
									}
								}
							}
						}
						"no_more_history" => {
							let mut s = state.borrow_mut();
							s.history_loading = false;
							if let Some(jid) = val.get("data").and_then(|d| d.get("jid")).and_then(|j| j.as_str()) {
								if !jid.is_empty() {
									eprintln!("[LOG UI] End of history reached on phone for chat: {}", jid);
									s.completed_history_chats.insert(jid.to_string());
								}
							}
						}
						"play_audio_failed" => {
							eprintln!("[LOG UI] Voice message audio failed to load or download");
							live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Could not load voice message");
						}
						"history_updated" => {
							eprintln!("[LOG UI] History sync completed on phone! Refreshing messages for active chat...");
							let s = state.borrow();
							if let Some(ref jid) = s.selected_chat.clone() {
								let limit = 9999999;
								let payload = serde_json::json!({
									"action": "get_messages",
									"data": { "jid": jid, "limit": limit, "offset": 0 }
								});
								let _ = s.ws_tx.send(payload.to_string());
							}
						}
						"search_results" => {
							if let Some(data) = val.get("data") {
								if let Some(results) = data.get("results").and_then(|r| r.as_array()) {
									parts.conversations_parts.messages_list.clear();
									for msg in results {
										let from = msg.get("from").and_then(|f| f.as_str()).unwrap_or("");
										let text = msg.get("text").and_then(|t| t.as_str()).unwrap_or("");
										let formatted = format!("{}: {}", from, text);
										parts.conversations_parts.messages_list.append(&formatted);
									}
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
// 										parts.conversations_parts.messages_list.announce(&announcement);
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
// 									parts.conversations_parts.messages_list.announce(&format!("{} is now {}", name, status_str));
								}
							}
						}
						"message_status" | "receipt" => {
							if let Some(data) = val.get("data") {
								let chat_jid = data.get("jid").or_else(|| data.get("chat_jid")).and_then(|j| j.as_str()).unwrap_or("");
								let raw_status = data.get("status").or_else(|| data.get("type")).and_then(|s| s.as_str()).unwrap_or("");
								
								let mut msg_ids = Vec::new();
								if let Some(ids) = data.get("message_ids").and_then(|i| i.as_array()) {
									for id_val in ids {
										if let Some(id_str) = id_val.as_str() {
											msg_ids.push(id_str.to_string());
										}
									}
								} else if let Some(id_str) = data.get("message_id").and_then(|i| i.as_str()) {
									msg_ids.push(id_str.to_string());
								}

								let clean_incoming_jid = chat_jid.split('@').next().unwrap_or(chat_jid);

								let mut s = state.borrow_mut();
								let is_match = if let Some(ref sel) = s.selected_chat {
									let clean_sel = sel.split('@').next().unwrap_or(sel);
									sel == chat_jid || clean_incoming_jid == clean_sel
								} else {
									false
								};

								if is_match {
									let is_group = chat_jid.ends_with("@g.us");
									let norm_status = match raw_status {
										"read-self" => "read",
										"" => "delivered",
										other => other,
									};

									for target_id in msg_ids {
										let mut found = false;
										for (idx, msg) in s.messages.iter_mut().enumerate() {
											if msg.id == target_id {
												msg.status = norm_status.to_string();
												let updated_display = msg.format_display(is_group);
												parts.conversations_parts.messages_list.set_string(idx as u32, &updated_display);
												live_region::announce(parts.conversations_parts.messages_announcer.clone(), &updated_display);
												found = true;
												break;
											}
										}
										if !found {
											s.pending_receipts.insert(target_id.clone(), norm_status.to_string());
										}
									}
								}
							}
						}
						"message_edited" => {
							if let Some(data) = val.get("data") {
								let chat_jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
								let target_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("");
								let new_text = data.get("text").and_then(|t| t.as_str()).unwrap_or("");

								let clean_incoming_jid = chat_jid.split('@').next().unwrap_or(chat_jid);

								let mut s = state.borrow_mut();
								let is_match = if let Some(ref sel) = s.selected_chat {
									let clean_sel = sel.split('@').next().unwrap_or(sel);
									sel == chat_jid || clean_incoming_jid == clean_sel
								} else {
									false
								};

								if is_match {
									let is_group = chat_jid.ends_with("@g.us");
									for (idx, msg) in s.messages.iter_mut().enumerate() {
										if msg.id == target_id {
											msg.text = new_text.to_string();
											let updated_display = msg.format_display(is_group);
											parts.conversations_parts.messages_list.set_string(idx as u32, &updated_display);
											live_region::announce(parts.conversations_parts.messages_announcer.clone(), &format!("Message edited: {}", updated_display));
											break;
										}
									}
								}
							}
						}
						"message_reaction" => {
							if let Some(data) = val.get("data") {
								let chat_jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
								let target_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("");
								let reactor = data.get("reactor").and_then(|r| r.as_str()).unwrap_or("");
								let emoji = data.get("emoji").and_then(|e| e.as_str()).unwrap_or("");
								let reactor_name_val = data.get("reactor_name").and_then(|n| n.as_str()).unwrap_or("");

								let clean_incoming_jid = chat_jid.split('@').next().unwrap_or(chat_jid);

								let mut s = state.borrow_mut();
								let is_match = if let Some(ref sel) = s.selected_chat {
									let clean_sel = sel.split('@').next().unwrap_or(sel);
									sel == chat_jid || clean_incoming_jid == clean_sel
								} else {
									false
								};

								if is_match {
									let is_group = chat_jid.ends_with("@g.us");

									let is_own_reaction = reactor == "me" || reactor_name_val == "You" || reactor_name_val == "me";

									let resolved_name = if is_own_reaction {
										"You".to_string()
									} else if !reactor_name_val.is_empty() && !reactor_name_val.contains('@') {
										reactor_name_val.to_string()
									} else {
										s.contacts.get(clean_incoming_jid)
											.or_else(|| s.contacts.get(chat_jid))
											.or_else(|| s.contacts.get(reactor))
											.cloned()
											.unwrap_or_else(|| "Contact".to_string())
									};

									for (idx, msg) in s.messages.iter_mut().enumerate() {
										if msg.id == target_id {
											if emoji.is_empty() {
												msg.reactions.retain(|r| {
													if is_own_reaction {
														r.reactor_name != "You" && r.reactor_jid != "me" && r.reactor_name != "me"
													} else {
														r.reactor_jid != reactor && r.reactor_name != resolved_name
													}
												});
											} else {
												let mut found = false;
												for r in msg.reactions.iter_mut() {
													let r_is_own = r.reactor_name == "You" || r.reactor_jid == "me";
													if (is_own_reaction && r_is_own) || (!is_own_reaction && (r.reactor_jid == reactor || r.reactor_name == resolved_name)) {
														r.emoji = emoji.to_string();
														r.reactor_name = resolved_name.clone();
														found = true;
														break;
													}
												}
												if !found {
													msg.reactions.push(whantv::ChatReaction {
														reactor_jid: if is_own_reaction { "me".to_string() } else { reactor.to_string() },
														reactor_name: resolved_name.clone(),
														emoji: emoji.to_string(),
														timestamp: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64,
													});
												}
											}

											let updated_display = msg.format_display(is_group);
											parts.conversations_parts.messages_list.set_string(idx as u32, &updated_display);
											
											let announce_msg = if emoji.is_empty() {
												format!("Reaction removed by {}", resolved_name)
											} else {
												format!("{} reacted with {}", resolved_name, emoji)
											};
											live_region::announce(parts.conversations_parts.messages_announcer.clone(), &announce_msg);
											break;
										}
									}
								}
							}
						}
						"media_downloaded" => {
							if let Some(data) = val.get("data") {
								let path = data.get("path").and_then(|p| p.as_str()).unwrap_or("unknown path");
// 								parts.conversations_parts.messages_list.announce(&format!("Media downloaded to {}", path));
							}
						}
						"play_audio" => {
							if let Some(data) = val.get("data") {
								if let Some(path) = data.get("path").and_then(|p| p.as_str()) {
									let msg_id = data.get("id").and_then(|i| i.as_str()).map(|s| s.to_string());
									eprintln!("[LOG UI] Playing voice note: {} from path: {}", msg_id.as_deref().unwrap_or(""), path);
									let mut s = state.borrow_mut();
									s.playing_msg_id = msg_id;
									s.is_audio_paused = false;
									if let Some(ref sink) = s.audio_sink {
										sink.stop();
									}
									s.audio_sink = None;
									
									if s.audio_stream.is_none() {
										if let Ok((stream, stream_handle)) = OutputStream::try_default() {
											s.audio_stream = Some((stream, stream_handle));
										}
									}
									let mut played = false;
									if let Some((_, ref stream_handle)) = s.audio_stream {
										if let Ok(sink) = Sink::try_new(stream_handle) {
											if let Ok(file) = File::open(path) {
												if let Ok(decoder) = Decoder::new(BufReader::new(file)) {
													sink.append(decoder);
													sink.play();
													s.audio_sink = Some(sink);
													played = true;
													parts.conversations_parts.play_voice_btn.set_label("Pause voice message");
												}
											}
										}
									}
									if !played {
										use std::io::Write;
										let path_escaped = path.replace("'", "''");
										if s.audio_stdin.is_none() {
											let ps_script = r#"
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$null = [Windows.Media.Playback.MediaPlayer, Windows.Media, ContentType=WindowsRuntime]
$p = New-Object Windows.Media.Playback.MediaPlayer
while ($true) {
    $line = [System.Console]::ReadLine()
    if ($null -eq $line -or $line -eq "EXIT") { break }
    if ($line.StartsWith("PLAY ")) {
        $path = $line.Substring(5)
        $p.Source = [Windows.Media.Core.MediaSource]::CreateFromUri([Uri]$path)
        $p.Play()
    } elseif ($line -eq "PAUSE") {
        $p.Pause()
    } elseif ($line -eq "RESUME") {
        $p.Play()
    }
}
"#;
											if let Ok(mut child) = std::process::Command::new("powershell")
												.args(&["-NoProfile", "-WindowStyle", "Hidden", "-Command", ps_script])
												.stdin(std::process::Stdio::piped())
												.spawn() {
												s.audio_stdin = child.stdin.take();
												s.audio_process = Some(child);
											}
										}
										if let Some(ref mut stdin) = s.audio_stdin {
											let _ = writeln!(stdin, "PLAY {}", path_escaped);
											let _ = stdin.flush();
											parts.conversations_parts.play_voice_btn.set_label("Pause voice message");
										}
									}
								}
							}
						}
						"attachment_uploading" => {
							if let Some(data) = val.get("data") {
								let filename = data.get("filename").and_then(|f| f.as_str()).unwrap_or("file");
								let percent = data.get("percent").and_then(|p| p.as_i64());
								let msg = if let Some(p) = percent {
									format!("Uploading {} ({}%)...", filename, p)
								} else {
									format!("Uploading {}...", filename)
								};
								parts.conversations_parts.attachment_info_label.set_label(&msg);

								let mut s = state.borrow_mut();
								let is_group = s.selected_chat.as_ref().map(|j| j.ends_with("@g.us")).unwrap_or(false);
								let mut temp_idx = None;
								for (idx, m) in s.messages.iter().enumerate() {
									if m.id.starts_with("temp_upload_") {
										temp_idx = Some(idx);
										break;
									}
								}
								if let Some(idx) = temp_idx {
									let existing_text = &s.messages[idx].text;
									let caption_suffix = if let Some(pos) = existing_text.find("] ") {
										existing_text[pos + 2..].to_string()
									} else {
										String::new()
									};
									s.messages[idx].text = if caption_suffix.is_empty() {
										format!("[{}]", msg)
									} else {
										format!("[{}] {}", msg, caption_suffix)
									};
									parts.conversations_parts.messages_list.set_string(idx as u32, &s.messages[idx].format_display(is_group));
								}
								live_region::announce(parts.conversations_parts.messages_announcer.clone(), &msg);
							}
						}
						"attachment_sent" => {
							if let Some(data) = val.get("data") {
								let filename = data.get("filename").and_then(|f| f.as_str()).unwrap_or("file");
								let real_id = data.get("id").and_then(|i| i.as_str()).unwrap_or("");

								parts.conversations_parts.attachment_info_label.set_label("");

								let mut s = state.borrow_mut();
								let is_group = s.selected_chat.as_ref().map(|j| j.ends_with("@g.us")).unwrap_or(false);

								let mut target_idx = None;
								for (idx, m) in s.messages.iter().enumerate() {
									if m.id.starts_with("temp_upload_") || m.status == "uploading" || m.status == "pending" {
										target_idx = Some(idx);
										break;
									}
								}

								if let Some(idx) = target_idx {
									if !real_id.is_empty() {
										s.messages[idx].id = real_id.to_string();
									}
									s.messages[idx].status = "sent".to_string();

									let text = &s.messages[idx].text;
									if let Some(pos) = text.find("] ") {
										s.messages[idx].text = text[pos + 2..].to_string();
									}

									let updated_display = s.messages[idx].format_display(is_group);
									parts.conversations_parts.messages_list.set_string(idx as u32, &updated_display);
									live_region::announce(parts.conversations_parts.messages_announcer.clone(), &updated_display);
								} else {
									let msg = format!("{} sent successfully", filename);
									live_region::announce(parts.conversations_parts.messages_announcer.clone(), &msg);
								}
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
		UiCommand::NavSelected(idx) => {
			match idx {
				0 => { // Conversations
					parts.conversations_parts.panel.show(true);
					state.borrow_mut().active_tab = 0;
					apply_chat_filter(&mut state.borrow_mut(), parts);
				}
				1 => { // Archived Chats
					parts.conversations_parts.panel.show(true);
					state.borrow_mut().active_tab = 1;
					apply_chat_filter(&mut state.borrow_mut(), parts);
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
				s.loading_messages = false;
				s.no_more_messages = false;
				s.history_loading = false;
				s.prev_msg_count = 0;
				s.pending_attachments.clear();
				parts.conversations_parts.conversation_panel.show(true);
				parts.conversations_parts.play_voice_btn.show(false);
				parts.conversations_parts.attachment_panel.show(false);
				parts.conversations_parts.attachment_info_label.set_label("");
				parts.conversations_parts.messages_list.clear();
				parts.conversations_parts.message_field.set_value("");
				parts.conversations_parts.send_message_btn.show(true);
				parts.conversations_parts.panel.layout();
				let limit = 9999999;
				let payload = serde_json::json!({
					"action": "get_messages",
					"data": { "jid": jid, "limit": limit, "offset": 0 }
				});
				let _ = s.ws_tx.send(payload.to_string());
				drop(s);
				handle_ui_command(UiCommand::SubscribePresence(jid.clone()), state, parts);
				handle_ui_command(UiCommand::MarkRead { jid: jid.clone(), message_ids: Vec::new() }, state, parts);
				parts.conversations_parts.messages_list.set_focus();
			}
		}
		UiCommand::LoadMoreMessages => {
			let mut s = state.borrow_mut();
			if s.history_loading {
				eprintln!("[LOG UI] Request already in flight for chat, skipping duplicate trigger");
				return;
			}
			if let Some(ref jid) = s.selected_chat.clone() {
				if s.completed_history_chats.contains(jid) {
					eprintln!("[LOG UI] All available history already loaded for chat {}, skipping request", jid);
					return;
				}
				s.history_loading = true;
				eprintln!("[LOG UI] Reached top of chat! Requesting older history from phone for: {}", jid);
				let oldest_msg_id = s.messages.first().map(|m| m.id.clone()).unwrap_or_default();
				let payload = serde_json::json!({
					"action": "request_history_sync",
					"data": { "jid": jid, "message_id": oldest_msg_id, "count": 50 }
				});
				let _ = s.ws_tx.send(payload.to_string());
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
		UiCommand::StopRecording => {
			let _ = state.borrow().ws_tx.send(serde_json::json!({ "action": "stop_recording" }).to_string());
		}
		UiCommand::TogglePauseRecording => {
			let payload = serde_json::json!({ "action": "toggle_pause_recording" });
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::ForceSync => {
			let _ = state.borrow().ws_tx.send(serde_json::json!({ "action": "force_sync" }).to_string());
		}
		UiCommand::SendVoiceMessage => {
			let payload = serde_json::json!({ "action": "send_voice_message" });
			let _ = state.borrow().ws_tx.send(payload.to_string());
			parts.conversations_parts.voice_panel.show(false);
			parts.conversations_parts.record_voice_btn.show(true);
			parts.conversations_parts.conversation_panel.layout();
		}
		UiCommand::DiscardRecording => {
			let _ = state.borrow().ws_tx.send(serde_json::json!({ "action": "discard_recording" }).to_string());
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
						let mut s = state.borrow_mut();
						s.pending_attachments.push(path.clone());
						let file_names: Vec<String> = s.pending_attachments.iter().map(|p| {
							std::path::Path::new(p).file_name().unwrap_or_default().to_string_lossy().to_string()
						}).collect();
						let label_text = format!("Files: {}", file_names.join(", "));
						drop(s);
						parts.conversations_parts.attachment_info_label.set_label(&label_text);
						parts.conversations_parts.attachment_panel.show(true);
						parts.conversations_parts.caption_field.set_value("");
						parts.conversations_parts.caption_field.set_focus();
						parts.conversations_parts.conversation_panel.layout();
						live_region::announce(parts.conversations_parts.messages_announcer.clone(), &format!("File attached: {}", file_names.last().unwrap_or(&String::new())));
					}
				}
			}
		}
		UiCommand::SendAttachment { caption } => {
			let jid = state.borrow().selected_chat.clone();
			let paths = {
				let mut s = state.borrow_mut();
				std::mem::take(&mut s.pending_attachments)
			};
			if let Some(ref jid) = jid {
				let is_group = jid.ends_with("@g.us");
				let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;

				for (idx, path) in paths.iter().enumerate() {
					let ext = std::path::Path::new(path)
						.extension()
						.unwrap_or_default()
						.to_string_lossy()
						.to_lowercase();
					let file_type = match ext.as_str() {
						"jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" => "image",
						"mp4" | "avi" | "mov" | "mkv" | "webm" | "3gp" => "video",
						_ => "document",
					};
					let filename = std::path::Path::new(path)
						.file_name()
						.unwrap_or_default()
						.to_string_lossy()
						.to_string();

					let display_text = if caption.trim().is_empty() {
						format!("[Uploading {}...]", filename)
					} else {
						format!("[Uploading {}...] {}", filename, caption)
					};

					let temp_id = format!("temp_upload_{}_{}", ts, idx);
					let pending_msg = ChatMessage {
						id: temp_id,
						text: display_text,
						timestamp: ts,
						from_me: true,
						sender_jid: "me".to_string(),
						sender_name: "You".to_string(),
						is_audio: false,
						is_date_divider: false,
						is_deleted: false,
						status: "uploading".to_string(),
						reactions: Vec::new(),
						quoted_id: String::new(),
						quoted_text: String::new(),
						quoted_sender: String::new(),
					};

					{
						let mut s = state.borrow_mut();
						s.messages.push(pending_msg.clone());
					}
					parts.conversations_parts.messages_list.append(&pending_msg.format_display(is_group));

					let count = parts.conversations_parts.messages_list.get_count();
					if count > 0 {
						parts.conversations_parts.messages_list.set_selection((count - 1) as u32, true);
						parts.conversations_parts.messages_list.set_focus();
					}
					live_region::announce(parts.conversations_parts.messages_announcer.clone(), &pending_msg.format_display(is_group));

					let payload = serde_json::json!({
						"action": "send_attachment",
						"data": {
							"jid": jid,
							"type": file_type,
							"path": path,
							"filename": filename,
							"caption": caption
						}
					});
					let _ = state.borrow().ws_tx.send(payload.to_string());
				}
				parts.conversations_parts.attachment_panel.show(false);
				parts.conversations_parts.attachment_info_label.set_label("");
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
		UiCommand::PlayVoiceMessage(msg_id) => {
			let payload = serde_json::json!({
				"action": "play_voice_message",
				"data": { "id": msg_id }
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::CloseConversation => {
			let mut s = state.borrow_mut();
			s.selected_chat = None;
			parts.conversations_parts.conversation_panel.show(false);
			parts.conversations_parts.panel.layout();
			parts.conversations_parts.conversations_list.set_focus();
		}
		UiCommand::EditMessage { message_id, new_text } => {
			let jid = state.borrow().selected_chat.clone();
			if let Some(jid) = jid {
				let payload = serde_json::json!({
					"action": "edit_message",
					"data": {
						"jid": jid,
						"message_id": message_id,
						"new_text": new_text
					}
				});
				let _ = state.borrow().ws_tx.send(payload.to_string());
			}
		}
		UiCommand::DeleteMessage { message_id, sender_jid, for_everyone } => {
			let jid = state.borrow().selected_chat.clone();
			if let Some(jid) = jid {
				let payload = serde_json::json!({
					"action": "delete_message",
					"data": {
						"jid": jid,
						"message_id": message_id,
						"sender_jid": sender_jid,
						"for_everyone": for_everyone
					}
				});
				let _ = state.borrow().ws_tx.send(payload.to_string());
			}
		}
	}
}

