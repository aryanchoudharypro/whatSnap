#![windows_subsystem = "windows"]

mod countries;
mod ui;
mod ui_wake;

use std::{
	cell::RefCell,
	io::{BufRead, BufReader, Write},
	rc::Rc,
	sync::{
		Arc, Mutex,
		atomic::{AtomicBool, Ordering},
		mpsc,
	},
	thread,
	time::Duration,
};

use interprocess::{
	TryClone,
	local_socket::{GenericNamespaced, ToNsName, prelude::*},
};
use wxdragon::prelude::*;

use crate::{
	ui::window::{WindowParts, bind_input_handlers, build_main_window},
	ui_wake::{ID_UI_WAKE, UiCommandSender, UiWaker},
};

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
	ToggleSearchPanel,
	CloseSearch,
	JumpToSearchResult { message_id: String, message_text: String },
	StartRecording,
	TogglePauseRecording,
	StopRecording,
	DiscardRecording,
	ForceSync,
	SendVoiceMessage,
	DiscardVoiceMessage,
	AddAttachment,
	PasteAttachment,
	SendAttachment { caption: String },
	DiscardAttachment,
	DownloadMedia,
	OpenMedia,
	SaveMediaAs,
	CancelEdit,
	RemoveQuote,
	EditMessage { message_id: String, new_text: String },
	DeleteMessage { message_id: String, sender_jid: String, for_everyone: bool },
	GetMessageInfo { message_id: String },
	CopyAlbumToClipboard { message_ids: Vec<String> },
	SendTyping { jid: String, state: String, media: String },
	SubscribePresence(String),
	MarkRead { jid: String, message_ids: Vec<String> },
	LoadMoreMessages,
	PlayVoiceMessage(String),
	SeekAudio(f32),         // relative delta in seconds
	SeekAudioAbsolute(i32), // slider value 0..1000
	CloseConversation,
	ReactToMessage { message_id: String, sender_jid: String, emoji: String },
	ToggleAudioSpeed,
	ShowConversationData,
	ForwardMessage { message_id: String, to_jids: Vec<String> },
	TogglePinChat { jid: String, pin: bool },
	ToggleArchiveChat { jid: String, archive: bool },
	ToggleMuteChat { jid: String, mute: bool },
	ToggleReadChat { jid: String, read: bool },
	DeleteChat { jid: String },
}

use std::fs::File;

use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player};
use rodio_wsola::WsolaSourceExt;
use whantv::ChatMessage;

fn play_notification_sound(sound_file: &str, sounds_enabled: bool) {
	if !sounds_enabled {
		return;
	}
	let wav_file = sound_file.replace(".ogg", ".wav");
	let path = sound_file.to_string();
	std::thread::spawn(move || {
		#[cfg(target_os = "windows")]
		unsafe {
			let _ =
				windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
		}

		let find_path = |target_name: &str| -> Option<std::path::PathBuf> {
			if let Ok(exe_path) = std::env::current_exe() {
				if let Some(parent) = exe_path.parent() {
					let p = parent.join(target_name);
					if p.exists() {
						return Some(p);
					}
				}
			}
			let p = std::path::PathBuf::from(target_name);
			if p.exists() { Some(p) } else { None }
		};

		let resolved_path =
			find_path(&wav_file).or_else(|| find_path(&path)).unwrap_or_else(|| std::path::PathBuf::from(&wav_file));

		if !resolved_path.exists() {
			eprintln!("[LOG SOUND] Sound file does not exist at {:?}", resolved_path);
			return;
		}

		if let Ok(stream_handle) = DeviceSinkBuilder::open_default_sink() {
			let sink = Player::connect_new(&stream_handle.mixer());
			if let Ok(file) = File::open(&resolved_path) {
				if let Ok(decoder) = Decoder::new(BufReader::new(file)) {
					sink.append(decoder);
					sink.sleep_until_end();
				} else {
					eprintln!("[LOG SOUND] Failed to decode audio {:?}", resolved_path);
				}
			}
		}
	});
}

#[cfg(target_os = "windows")]
unsafe extern "system" {
	fn mciSendStringW(
		lpstrCommand: *const u16,
		lpstrReturnString: *mut u16,
		uReturnLength: u32,
		hwndCallback: isize,
	) -> u32;
}

fn execute_mci_command(cmd: &str) -> u32 {
	#[cfg(target_os = "windows")]
	{
		let wide: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
		unsafe { mciSendStringW(wide.as_ptr(), std::ptr::null_mut(), 0, 0) }
	}
	#[cfg(not(target_os = "windows"))]
	{
		0
	}
}

/// Opens a fresh MCI waveaudio recording device and starts capturing into it.
/// Used both for the initial recording and for each resume after a pause:
/// MCI's own "pause"/"resume" pair isn't reliably continued into the same
/// buffer by every audio driver, so instead of trusting "resume" we always
/// start a brand new, self-contained segment and splice segments together
/// on send (see `concat_wav_segments`).
fn mci_start_new_segment() -> bool {
	execute_mci_command("close voice_rec");
	let res1 = execute_mci_command("open new type waveaudio alias voice_rec");
	let _ = execute_mci_command(
		"set voice_rec channels 2 samplespersec 48000 bitspersample 16 alignment 4 bytespersec 192000 format tag pcm",
	);
	let res2 = execute_mci_command("record voice_rec");
	res1 == 0 && res2 == 0
}

/// Stops the active MCI recording, saves it to `path`, and closes the
/// device, leaving `path` a complete, independently-playable WAV file.
fn mci_finish_segment(path: &str) {
	execute_mci_command("stop voice_rec");
	execute_mci_command(&format!("save voice_rec \"{}\"", path));
	execute_mci_command("close voice_rec");
}

/// Concatenates the PCM audio from one or more same-format WAV segments
/// (produced across a recording's pause/resume cycles) into a single WAV
/// file at `out_path`.
fn concat_wav_segments(segments: &[String], out_path: &str) -> anyhow::Result<()> {
	use hound::{WavReader, WavWriter};
	let mut spec = None;
	let mut writer: Option<WavWriter<std::io::BufWriter<std::fs::File>>> = None;
	for seg in segments {
		let mut reader = WavReader::open(seg)?;
		let seg_spec = reader.spec();
		if writer.is_none() {
			spec = Some(seg_spec);
			writer = Some(WavWriter::create(out_path, seg_spec)?);
		} else if spec != Some(seg_spec) {
			eprintln!("[LOG UI] Voice recording segment {} has a mismatched format, skipping it", seg);
			continue;
		}
		if let Some(ref mut w) = writer {
			for sample in reader.samples::<i16>() {
				w.write_sample(sample?)?;
			}
		}
	}
	match writer {
		Some(w) => {
			w.finalize()?;
			Ok(())
		}
		None => Err(anyhow::anyhow!("no valid voice recording segments to concatenate")),
	}
}

struct AppState {
	ws_tx: mpsc::Sender<String>,
	audio_stream: Option<MixerDeviceSink>,
	audio_sink: Option<Player>,
	playing_msg_id: Option<String>,
	audio_process: Option<std::process::Child>,
	audio_stdin: Option<std::process::ChildStdin>,
	is_audio_paused: bool,
	audio_path: Option<String>,
	audio_total_secs: f32,
	audio_elapsed_secs: f32,
	audio_speed: f32,
	audio_start_instant: Option<std::time::Instant>,
	is_voice_recording: bool,
	is_voice_paused: bool,
	voice_recording_path: Option<String>,
	/// Completed segments from earlier pause/resume cycles in the current
	/// recording, in order. MCI's "pause"/"resume" for a waveaudio device is
	/// not reliably continued by every driver, so each pause closes off a
	/// self-contained WAV segment instead of trusting the device to keep
	/// appending to one file; segments are spliced together on send.
	voice_recording_segments: Vec<String>,
	voice_recording_start: Option<std::time::Instant>,
	voice_paused_duration: std::time::Duration,
	voice_pause_instant: Option<std::time::Instant>,
	is_pairing_active: bool,
	chats: Vec<String>,
	filtered_chats: Vec<String>,
	archived_jids: std::collections::HashSet<String>,
	pinned_jids: std::collections::HashSet<String>,
	muted_jids: std::collections::HashSet<String>,
	unread_counts: std::collections::HashMap<String, u64>,
	display_names: std::collections::HashMap<String, String>,
	contacts: std::collections::HashMap<String, String>,
	presence_labels: std::collections::HashMap<String, String>,
	group_state: crate::ui::new_group::NewGroupState,
	selected_chat: Option<String>,
	initial_presence_received: bool,
	messages: Vec<ChatMessage>,
	loading_messages: bool,
	no_more_messages: bool,
	history_loading: bool,
	completed_history_chats: std::collections::HashSet<String>,
	prev_msg_count: usize,
	active_tab: usize,
	search_query: String,
	pending_attachments: Vec<String>,
	pending_reply_id: Option<String>,
	pending_reply_sender: Option<String>,
	pending_reply_preview: Option<String>,
	pending_receipts: std::collections::HashMap<String, String>,
	downloads_in_progress: std::collections::HashMap<String, i64>,
	pending_copy_announcements: std::collections::HashSet<String>,
	search_target_id: Option<String>,
	search_target_text: Option<String>,
	played_sent_sounds: std::collections::HashSet<String>,
	display_items: Vec<whantv::DisplayItem>,
	typing_timestamps: std::collections::HashMap<String, std::time::Instant>,
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

pub(crate) fn update_action_buttons(s: &AppState, parts: &crate::ui::conversations::ConversationsPanelParts) -> bool {
	parts.action_panel.freeze();
	let result = update_action_buttons_inner(s, parts);
	parts.action_panel.thaw();
	result
}

fn update_action_buttons_inner(s: &AppState, parts: &crate::ui::conversations::ConversationsPanelParts) -> bool {
	let sel = parts.messages_list.get_selection();
	let mut need_layout = false;

	if let Some(idx) = sel {
		if let Some(item) = s.display_items.get(idx as usize) {
			match item {
				whantv::DisplayItem::Single(msg) => {
					let is_deleted = msg.is_deleted || msg.text.contains("deleted");
					let is_voice =
						msg.is_audio || msg.text.starts_with("[Voice Message]") || msg.text.starts_with("[Audio]");
					let is_media = !is_deleted
						&& !is_voice && (!msg.media_type.is_empty()
						|| msg.is_document()
						|| msg.text.starts_with("[Photo]")
						|| msg.text.starts_with("[Image]")
						|| msg.text.starts_with("[Video]")
						|| msg.text.starts_with("[Document]")
						|| msg.text.starts_with("[Media]"));

					if is_media {
						if msg.is_media_downloaded() {
							if parts.action_download_btn.is_shown() {
								parts.action_download_btn.show(false);
								need_layout = true;
							}
							if !parts.action_open_btn.is_shown() {
								parts.action_open_btn.show(true);
								need_layout = true;
							}
							parts.action_open_btn.set_label("Open");
							if !parts.action_save_as_btn.is_shown() {
								parts.action_save_as_btn.show(true);
								need_layout = true;
							}
							parts.action_save_as_btn.set_label("Save as");
						} else {
							if parts.action_open_btn.is_shown() {
								parts.action_open_btn.show(false);
								need_layout = true;
							}
							if parts.action_save_as_btn.is_shown() {
								parts.action_save_as_btn.show(false);
								need_layout = true;
							}
							if !parts.action_download_btn.is_shown() {
								parts.action_download_btn.show(true);
								need_layout = true;
							}
							let label = if let Some(pct) = s.downloads_in_progress.get(&msg.id) {
								format!("Cancel download ({}%)", pct)
							} else {
								"Download".to_string()
							};
							parts.action_download_btn.set_label(&label);
						}
						return need_layout;
					}
				}
				whantv::DisplayItem::Album { messages, .. } => {
					let valid_messages: Vec<_> =
						messages.iter().filter(|m| !m.is_deleted && !m.text.contains("deleted")).collect();
					if !valid_messages.is_empty() {
						let all_dl = valid_messages.iter().all(|m| m.is_media_downloaded());
						if all_dl {
							if parts.action_download_btn.is_shown() {
								parts.action_download_btn.show(false);
								need_layout = true;
							}
							if !parts.action_open_btn.is_shown() {
								parts.action_open_btn.show(true);
								need_layout = true;
							}
							parts.action_open_btn.set_label("Open album");

							let photo_count = valid_messages
								.iter()
								.filter(|m| {
									m.media_type == "image"
										|| m.text.starts_with("[Photo]")
										|| m.text.starts_with("[Image]")
								})
								.count();
							let video_count = valid_messages
								.iter()
								.filter(|m| m.media_type == "video" || m.text.starts_with("[Video]"))
								.count();
							let save_label = if photo_count > 0 && video_count == 0 {
								"Save all photos"
							} else if video_count > 0 && photo_count == 0 {
								"Save all videos"
							} else {
								"Save all media"
							};
							if !parts.action_save_as_btn.is_shown() {
								parts.action_save_as_btn.show(true);
								need_layout = true;
							}
							parts.action_save_as_btn.set_label(save_label);
						} else {
							if parts.action_open_btn.is_shown() {
								parts.action_open_btn.show(false);
								need_layout = true;
							}
							if parts.action_save_as_btn.is_shown() {
								parts.action_save_as_btn.show(false);
								need_layout = true;
							}
							if !parts.action_download_btn.is_shown() {
								parts.action_download_btn.show(true);
								need_layout = true;
							}

							let uncollected = valid_messages.iter().filter(|m| !m.is_media_downloaded()).count();
							let is_dl_in_progress =
								valid_messages.iter().any(|m| s.downloads_in_progress.contains_key(&m.id));
							let dl_label = if is_dl_in_progress {
								format!("Downloading album ({} items remaining)", uncollected)
							} else {
								format!("Download album ({} items remaining)", uncollected)
							};
							parts.action_download_btn.set_label(&dl_label);
						}
					}
					return need_layout;
				}
			}
		}
	}

	if parts.action_download_btn.is_shown() {
		parts.action_download_btn.show(false);
		need_layout = true;
	}
	if parts.action_open_btn.is_shown() {
		parts.action_open_btn.show(false);
		need_layout = true;
	}
	if parts.action_save_as_btn.is_shown() {
		parts.action_save_as_btn.show(false);
		need_layout = true;
	}
	need_layout
}

#[cfg(target_os = "windows")]
mod win_cb {
	#[link(name = "user32")]
	unsafe extern "system" {
		pub fn OpenClipboard(hwnd: *mut std::ffi::c_void) -> i32;
		pub fn CloseClipboard() -> i32;
		pub fn EmptyClipboard() -> i32;
		pub fn SetClipboardData(uFormat: u32, hMem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
	}

	#[link(name = "kernel32")]
	unsafe extern "system" {
		pub fn GlobalAlloc(uFlags: u32, dwBytes: usize) -> *mut std::ffi::c_void;
		pub fn GlobalLock(hMem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
		pub fn GlobalUnlock(hMem: *mut std::ffi::c_void) -> i32;
	}
}

#[cfg(target_os = "windows")]
pub fn copy_files_to_clipboard(paths: &[String]) -> bool {
	let valid_paths: Vec<String> =
		paths.iter().filter(|p| !p.is_empty() && std::path::Path::new(p).exists()).cloned().collect();

	if valid_paths.is_empty() {
		return false;
	}

	let mut buffer: Vec<u16> = Vec::new();
	for p in &valid_paths {
		if let Ok(abs_path) = std::fs::canonicalize(p) {
			let clean = abs_path.to_string_lossy().trim_start_matches(r"\\?\").to_string();
			buffer.extend(clean.encode_utf16());
			buffer.push(0);
		} else {
			buffer.extend(p.encode_utf16());
			buffer.push(0);
		}
	}
	buffer.push(0);

	let dropfiles_size = 20;
	let total_bytes = dropfiles_size + buffer.len() * 2;

	unsafe {
		let h_global = win_cb::GlobalAlloc(0x0042, total_bytes);
		if h_global.is_null() {
			return false;
		}
		let ptr = win_cb::GlobalLock(h_global) as *mut u8;
		if ptr.is_null() {
			return false;
		}

		let p_files_ptr = ptr as *mut u32;
		*p_files_ptr = 20;
		let f_wide_ptr = ptr.add(16) as *mut u32;
		*f_wide_ptr = 1;

		let dst_str_ptr = ptr.add(dropfiles_size) as *mut u16;
		std::ptr::copy_nonoverlapping(buffer.as_ptr(), dst_str_ptr, buffer.len());

		let _ = win_cb::GlobalUnlock(h_global);

		if win_cb::OpenClipboard(std::ptr::null_mut()) != 0 {
			let _ = win_cb::EmptyClipboard();
			let res = win_cb::SetClipboardData(15, h_global);
			let _ = win_cb::CloseClipboard();
			return !res.is_null();
		}
	}
	false
}

#[cfg(not(target_os = "windows"))]
pub fn copy_files_to_clipboard(_paths: &[String]) -> bool {
	false
}

use std::{
	os::windows::process::CommandExt,
	process::{Command, Stdio},
};

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

	let _ = Command::new("taskkill").args(&["/F", "/IM", "backend.exe"]).creation_flags(0x08000000).output();

	let pipe_id = format!("whantv-backend-{}", std::process::id());
	let go_pipe_name = format!(r"\\.\pipe\{}", pipe_id);

	let mut bridge_child = if let Some(ref backend_path) = backend_exe {
		Command::new(backend_path)
			.arg("-db")
			.arg(&db_path_str)
			.arg("-pipe")
			.arg(&go_pipe_name)
			.current_dir(&backend_dir)
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.creation_flags(0x08000000)
			.spawn()
			.ok()
	} else {
		None
	};

	let res = wxdragon::main(|_| {
		let parts = build_main_window();
		let frame = parts.frame;

		let (ui_tx_raw, ui_rx) = mpsc::channel();
		let ui_alive = Arc::new(AtomicBool::new(true));
		let ui_waker = UiWaker::new(frame, ui_alive.clone());
		let ui_tx = UiCommandSender::new(ui_tx_raw, ui_waker.clone());

		let (ws_out_tx, ws_out_rx) = mpsc::channel::<String>();
		let ui_tx_clone = ui_tx.clone();
		let ui_alive_clone = ui_alive.clone();

		let ws_out_rx = Arc::new(Mutex::new(ws_out_rx));

		thread::spawn(move || {
			loop {
				if !ui_alive_clone.load(Ordering::SeqCst) {
					break;
				}

				let stream_name = pipe_id.as_str().to_ns_name::<GenericNamespaced>().unwrap();
				if let Ok(socket) = LocalSocketStream::connect(stream_name) {
					let mut socket_writer = socket.try_clone().expect("Failed to clone pipe");
					let mut buf_reader = BufReader::new(socket);
					let mut line = String::new();

					let ui_alive_write = ui_alive_clone.clone();
					let rx_clone = ws_out_rx.clone();

					// Write Thread
					thread::spawn(move || {
						while ui_alive_write.load(Ordering::SeqCst) {
							let msg_opt = {
								let rx = rx_clone.lock().unwrap();
								rx.recv_timeout(Duration::from_millis(500)).ok()
							};

							if let Some(out_msg) = msg_opt {
								let msg = format!("{}\n", out_msg);
								if socket_writer.write_all(msg.as_bytes()).is_err() {
									break;
								}
							}
						}
					});

					// Read Loop (Current thread)
					loop {
						if !ui_alive_clone.load(Ordering::SeqCst) {
							break;
						}

						match buf_reader.read_line(&mut line) {
							Ok(0) => break, // EOF
							Ok(_) => {
								let _ = ui_tx_clone.send(UiCommand::ProcessWebSocketMessage(line.trim().to_string()));
								line.clear();
							}
							Err(_) => break, // Any read error breaks and triggers reconnect
						}
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
			audio_path: None,
			audio_total_secs: 0.0,
			audio_elapsed_secs: 0.0,
			audio_speed: 1.0,
			audio_start_instant: None,
			chats: Vec::new(),
			filtered_chats: Vec::new(),
			archived_jids: std::collections::HashSet::new(),
			pinned_jids: std::collections::HashSet::new(),
			muted_jids: std::collections::HashSet::new(),
			unread_counts: std::collections::HashMap::new(),
			display_names: std::collections::HashMap::new(),
			contacts: std::collections::HashMap::new(),
			presence_labels: std::collections::HashMap::new(),
			typing_timestamps: std::collections::HashMap::new(),
			group_state: crate::ui::new_group::NewGroupState {
				all_contact_labels: Vec::new(),
				all_contact_jids: Vec::new(),
				checked_jids: std::collections::HashSet::new(),
				current_jids: Vec::new(),
			},
			selected_chat: None,
			initial_presence_received: false,
			messages: Vec::new(),
			loading_messages: false,
			no_more_messages: false,
			history_loading: false,
			completed_history_chats: std::collections::HashSet::new(),
			prev_msg_count: 0,
			active_tab: 0,
			search_query: String::new(),
			pending_attachments: Vec::new(),
			pending_reply_id: None,
			pending_reply_sender: None,
			pending_reply_preview: None,
			pending_receipts: std::collections::HashMap::new(),
			downloads_in_progress: std::collections::HashMap::new(),
			pending_copy_announcements: std::collections::HashSet::new(),
			search_target_id: None,
			search_target_text: None,
			played_sent_sounds: std::collections::HashSet::new(),
			is_voice_recording: false,
			is_voice_paused: false,
			voice_recording_path: None,
			voice_recording_segments: Vec::new(),
			voice_recording_start: None,
			voice_paused_duration: std::time::Duration::ZERO,
			voice_pause_instant: None,
			is_pairing_active: false,
			display_items: Vec::new(),
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

		// Setup audio progress timer
		let audio_timer = Timer::new(&frame);
		let state_timer = state.clone();
		let parts_timer = parts_clone.clone();
		audio_timer.on_tick(move |_| {
			let mut s = match state_timer.try_borrow_mut() {
				Ok(s) => s,
				Err(_) => return, // State already borrowed, skip this update
			};
			if s.playing_msg_id.is_none() {
				return;
			}
			// Check if sink finished naturally
			let finished = s.audio_sink.as_ref().map(|sk| sk.empty()).unwrap_or(false);
			if finished && !s.is_audio_paused {
				s.playing_msg_id = None;
				s.audio_start_instant = None;
				s.audio_elapsed_secs = 0.0;
				parts_timer.conversations_parts.play_voice_btn.set_label("Play voice message");
				parts_timer.conversations_parts.audio_slider.set_value(0);
				parts_timer.conversations_parts.audio_slider.show(false);
				parts_timer.conversations_parts.audio_speed_btn.show(false);
				parts_timer.conversations_parts.audio_progress_label.show(false);
				return;
			}
			if !s.is_audio_paused {
				if let Some(inst) = s.audio_start_instant {
					let elapsed = s.audio_elapsed_secs + inst.elapsed().as_secs_f32() * s.audio_speed;
					let total = s.audio_total_secs;
					let max_sec = (total as i32).max(1);
					let pos = (elapsed as i32).max(0).min(max_sec);
					if parts_timer.conversations_parts.audio_slider.is_shown() {
						parts_timer.conversations_parts.audio_slider.set_range(0, max_sec);
						parts_timer.conversations_parts.audio_slider.set_value(pos);
					}
				}
			}
		});
		audio_timer.start(500, false);
		Box::leak(Box::new(audio_timer));

		let typing_timer = Timer::new(&frame);
		let state_typing_timer = state.clone();
		let parts_typing_timer = parts_clone.clone();
		typing_timer.on_tick(move |_| {
			let mut s = match state_typing_timer.try_borrow_mut() {
				Ok(s) => s,
				Err(_) => return,
			};
			let now = std::time::Instant::now();
			let timeout = std::time::Duration::from_secs(30);
			let expired: Vec<String> = s
				.typing_timestamps
				.iter()
				.filter(|(_, ts)| now.duration_since(**ts) >= timeout)
				.map(|(jid, _)| jid.clone())
				.collect();
			if expired.is_empty() {
				return;
			}
			let selected = s.selected_chat.clone();
			for jid in &expired {
				s.typing_timestamps.remove(jid);
			}
			if let Some(ref sel_jid) = selected {
				if expired.iter().any(|j| j == sel_jid) {
					let norm_jid = if let Some((user, server)) = sel_jid.split_once('@') {
						let base = user.split('.').next().unwrap_or(user).split(':').next().unwrap_or(user);
						format!("{}@{}", base, server)
					} else {
						sel_jid.clone()
					};
					let label = s
						.presence_labels
						.get(sel_jid.as_str())
						.or_else(|| s.presence_labels.get(&norm_jid))
						.cloned()
						.unwrap_or_else(|| {
							s.contacts.get(sel_jid).cloned().unwrap_or_else(|| "Conversation data".to_string())
						});
					parts_typing_timer.conversations_parts.conv_data_btn.set_label(&label);
				}
			}
		});
		typing_timer.start(5000, false);
		Box::leak(Box::new(typing_timer));

		parts_clone.login_panel.set_focus();
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

/// Perform a soft seek by stopping the current sink and re-opening the file
/// from `target_secs`. Returns the new Sink if successful.
fn do_seek(stream_handle: &MixerDeviceSink, path: &str, target_secs: f32, speed: f32) -> Option<Player> {
	use rodio::Source;
	let sink = Player::connect_new(&stream_handle.mixer());
	if let Ok(file) = std::fs::File::open(path) {
		if let Ok(decoder) = rodio::Decoder::new(std::io::BufReader::new(file)) {
			let source = decoder.skip_duration(std::time::Duration::from_secs_f32(target_secs.max(0.0))).wsola(speed);
			sink.append(source);
			sink.play();
			return Some(sink);
		} else if let Some(opus_buf) = decode_opus_file(path) {
			let source = opus_buf.skip_duration(std::time::Duration::from_secs_f32(target_secs.max(0.0))).wsola(speed);
			sink.append(source);
			sink.play();
			return Some(sink);
		}
	}
	None
}

fn get_status_rank(s: &str) -> i32 {
	match s {
		"pending" => 0,
		"uploading" => 1,
		"sent" => 2,
		"delivered" => 3,
		"read" => 4,
		"played" => 5,
		_ => 0,
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
			state.borrow_mut().is_pairing_active = false;
			let payload = serde_json::json!({
				"action": "get_qr"
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::CancelQr => {
			state.borrow_mut().is_pairing_active = false;
			let payload = serde_json::json!({
				"action": "cancel_qr"
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::CancelPairing => {
			state.borrow_mut().is_pairing_active = false;
			let payload = serde_json::json!({
				"action": "cancel_pairing"
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::RequestPairing(phone) => {
			state.borrow_mut().is_pairing_active = true;
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
				parts.conversations_parts.send_message_btn.show(false);
				parts.conversations_parts.remove_quote_btn.show(false);
				parts.conversations_parts.conversation_panel.layout();
			}
		}
		UiCommand::ConversationSelectionChanged(_idx) => {}
		UiCommand::ProcessWebSocketMessage(raw) => {
			if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
				if let Some(t) = val.get("type").and_then(|v| v.as_str()) {
					match t {
						"connected" => {
							parts.frame.set_status_text("Status: Connected", 0);
							parts.pairing_dialog.show(false);
							parts.qrcode_dialog.show(false);
							parts.login_panel.show(false);
							parts.nav_panel.show(true);
							parts.content_panel.show(true);
							parts.main_panel.layout();
							parts.frame.show(true);
							parts.conversations_parts.search_field.set_focus();
							let payload = serde_json::json!({
								"action": "get_chats"
							});
							let _ = state.borrow().ws_tx.send(payload.to_string());
						}
						"needs_login" => {
							parts.login_panel.show(true);
							parts.login_panel.set_focus();
						}
						"pairing_code" => {
							if let Some(code) = val.get("data").and_then(|d| d.get("code")).and_then(|c| c.as_str()) {
								parts.pairing_code_display.set_value(code);
								println!("{{'data': {{'qrcode': {{'pairingCode': '{}'}}}}}}", code);
								parts.pairing_dialog.show(true);
								parts.qrcode_dialog.show(false);
								parts.pairing_code_display.select_all();
								parts.pairing_code_display.set_focus();
								unsafe {
									let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
										windows::Win32::UI::WindowsAndMessaging::MB_OK,
									);
								}
								live_region::announce(
									parts.pairing_announcer.clone(),
									&format!("Pairing code updated: {}", code),
								);
							}
						}
						"qr" => {
							if state.borrow().is_pairing_active {
								return;
							}
							if let Some(code) = val.get("data").and_then(|d| d.get("code")).and_then(|c| c.as_str()) {
								if let Ok(qr) = qrcode::QrCode::new(code) {
									let image =
										qr.render::<image::Rgba<u8>>().quiet_zone(true).module_dimensions(5, 5).build();
									let width = image.width();
									let height = image.height();
									let data = image.into_raw();
									let mut png_bytes = Vec::new();
									let encoder = image::codecs::png::PngEncoder::new(&mut png_bytes);
									use image::ImageEncoder;
									if encoder
										.write_image(&data, width, height, image::ExtendedColorType::Rgba8)
										.is_ok()
									{
										use base64::{Engine as _, engine::general_purpose};
										let b64 = general_purpose::STANDARD.encode(&png_bytes);
										println!(
											"{{'data': {{'qrcode': {{'base64': 'data:image/png;base64,{}'}}}}}}",
											b64
										);
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
									let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
										windows::Win32::UI::WindowsAndMessaging::MB_OK,
									);
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
									parts
										.conversations_parts
										.messages_list
										.set_string(idx as u32, &s.messages[idx].format_display(is_group));
									let formatted = s.messages[idx].format_display(is_group);
									live_region::announce(
										parts.conversations_parts.messages_announcer.clone(),
										&formatted,
									);
								}
								drop(s);
								parts.conversations_parts.play_voice_btn.set_label("Play voice message");
								MessageDialog::builder(&parts.frame, msg, "Error")
									.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
									.build()
									.show_modal();
							}
						}
						"disconnected" | "socket_disconnected" => {
							parts.frame.set_status_text("Status: Disconnected", 0);
						}
						"history_sync" => {
							parts.frame.set_status_text("Status: Connected (History Syncing...)", 0);
							if let Some(data) = val.get("data") {
								if let Some(jids) = data.get("jids").and_then(|j| j.as_array()) {
									let s = state.borrow();
									if s.history_loading {
										if let Some(selected_jid) = s.selected_chat.clone() {
											if jids.iter().any(|j| j.as_str() == Some(&selected_jid)) {
												let payload = serde_json::json!({
													"action": "get_messages",
													"data": { "jid": selected_jid, "limit": 9999999, "offset": 0 }
												});
												let _ = s.ws_tx.send(payload.to_string());
											}
										}
									}
								}
							}
						}
						"chats" => {
							if let Some(data) = val.get("data") {
								if let Ok(chats_array) = serde_json::from_value::<Vec<serde_json::Value>>(data.clone())
								{
									let mut chat_list = Vec::new();
									let mut archived_jids = std::collections::HashSet::new();
									let mut pinned_jids = std::collections::HashSet::new();
									let mut muted_jids = std::collections::HashSet::new();
									let mut unread_counts = std::collections::HashMap::new();
									let mut display_names = std::collections::HashMap::new();
									let mut contacts_map = std::collections::HashMap::new();
									for chat in chats_array {
										if let Some(jid) = chat.get("JID").and_then(|j| j.as_str()) {
											let name = chat
												.get("FullName")
												.and_then(|n| n.as_str())
												.unwrap_or(jid)
												.to_string();
											chat_list.push(jid.to_string());

											let unread = chat.get("UnreadCount").and_then(|u| u.as_u64()).unwrap_or(0);
											let pinned = chat.get("Pinned").and_then(|p| p.as_bool()).unwrap_or(false);
											let muted = chat.get("Muted").and_then(|m| m.as_bool()).unwrap_or(false);
											let archived =
												chat.get("Archived").and_then(|a| a.as_bool()).unwrap_or(false);
											let preview =
												chat.get("PreviewSnippet").and_then(|p| p.as_str()).unwrap_or("");

											let ts = chat.get("Timestamp").and_then(|t| t.as_i64()).unwrap_or(0);
											let preview_sender =
												chat.get("PreviewSender").and_then(|p| p.as_str()).unwrap_or("");
											let preview_from_me =
												chat.get("PreviewFromMe").and_then(|p| p.as_bool()).unwrap_or(false);

											let time_str = if ts > 0 {
												if let Some(datetime) = chrono::DateTime::from_timestamp(ts, 0) {
													let local_dt: chrono::DateTime<chrono::Local> = datetime.into();
													let now_dt = chrono::Local::now();
													let msg_date = local_dt.date_naive();
													let today = now_dt.date_naive();
													let yesterday = today.pred_opt().unwrap_or(today);
													let num_days_ago = (today - msg_date).num_days();
													if msg_date == today {
														local_dt.format("%I:%M %p").to_string()
													} else if msg_date == yesterday {
														"Yesterday".to_string()
													} else if num_days_ago > 1 && num_days_ago < 7 {
														local_dt.format("%A").to_string()
													} else {
														local_dt.format("%d/%m/%Y").to_string()
													}
												} else {
													String::new()
												}
											} else {
												String::new()
											};

											let mut tags = Vec::new();
											if archived {
												tags.push("Archived");
												archived_jids.insert(jid.to_string());
											}
											if pinned {
												tags.push("Pinned");
												pinned_jids.insert(jid.to_string());
											}
											if muted {
												tags.push("Muted");
												muted_jids.insert(jid.to_string());
											}
											unread_counts.insert(jid.to_string(), unread);

											let name_line = if tags.is_empty() {
												name.clone()
											} else {
												format!("{} [{}]", name, tags.join("] ["))
											};

											let unread_line = if unread == 1 {
												"1 unread message".to_string()
											} else if unread > 1 {
												format!("{} unread messages", unread)
											} else {
												String::new()
											};

											let preview_line = if !preview.is_empty() {
												if preview_from_me {
													format!("You: {}", preview)
												} else if !preview_sender.is_empty() && jid.ends_with("@g.us") {
													format!("{}: {}", preview_sender, preview)
												} else {
													preview.to_string()
												}
											} else {
												String::new()
											};

											let mut parts_vec: Vec<&str> = Vec::new();
											parts_vec.push(&name_line);
											if !unread_line.is_empty() {
												parts_vec.push(&unread_line);
											}
											if !time_str.is_empty() {
												parts_vec.push(&time_str);
											}
											if !preview_line.is_empty() {
												parts_vec.push(&preview_line);
											}
											let display_name = parts_vec.join(" | ");
											display_names.insert(jid.to_string(), display_name);
											contacts_map.insert(jid.to_string(), name);
										}
									}
									let mut s = state.borrow_mut();
									s.chats = chat_list;
									s.contacts = contacts_map;
									s.archived_jids = archived_jids;
									s.pinned_jids = pinned_jids;
									s.muted_jids = muted_jids;
									s.unread_counts = unread_counts;
									s.display_names = display_names;

									// Compute new filtered list
									let lower_query = s.search_query.to_lowercase();
									let mut new_filtered = Vec::new();
									let mut new_display = Vec::new();
									for jid in &s.chats {
										let is_archived = s.archived_jids.contains(jid);
										if (s.active_tab == 0 && is_archived) || (s.active_tab == 1 && !is_archived) {
											continue;
										}
										let name = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.clone());
										if lower_query.is_empty() || name.to_lowercase().contains(&lower_query) {
											let dn = s.display_names.get(jid).cloned().unwrap_or_else(|| name);
											new_filtered.push(jid.clone());
											new_display.push(dn);
										}
									}

									let structure_changed = new_filtered.len() != s.filtered_chats.len()
										|| new_filtered.iter().zip(s.filtered_chats.iter()).any(|(a, b)| a != b);

									if structure_changed {
										s.filtered_chats = new_filtered;
										drop(s);
										apply_chat_filter(&mut state.borrow_mut(), parts);
									} else {
										// In-place update: just refresh display strings
										parts.conversations_parts.conversations_list.freeze();
										for (i, dn) in new_display.iter().enumerate() {
											parts.conversations_parts.conversations_list.set_string(i as u32, dn);
										}
										parts.conversations_parts.conversations_list.thaw();
										s.filtered_chats = new_filtered;
									}
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
									let is_from_me = data
										.get("from_me")
										.and_then(|f| f.as_bool())
										.unwrap_or_else(|| from == "me" || from == "You");
									let is_group = jid.ends_with("@g.us");
									let ts = data
										.get("timestamp")
										.and_then(|t| t.as_i64().or_else(|| t.as_f64().map(|f| f as i64)))
										.unwrap_or_else(|| {
											std::time::SystemTime::now()
												.duration_since(std::time::UNIX_EPOCH)
												.unwrap_or_default()
												.as_secs() as i64
										});
									let msg_id = data.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();

									let mut temp_msg_idx = None;
									if is_from_me {
										for (idx, msg) in s.messages.iter().enumerate() {
											if msg.id.starts_with("temp_upload_") {
												temp_msg_idx = Some(idx);
												break;
											}
										}
									}

									if let Some(idx) = temp_msg_idx {
										s.messages[idx].id = msg_id.clone();
										s.messages[idx].text = text.to_string();
										s.messages[idx].media_type =
											data.get("mediaType").and_then(|m| m.as_str()).unwrap_or("").to_string();
										s.messages[idx].media_filename = data
											.get("mediaFilename")
											.and_then(|m| m.as_str())
											.unwrap_or("")
											.to_string();

										let media_type = s.messages[idx].media_type.clone();
										let media_filename = s.messages[idx].media_filename.clone();

										// Update display_items and UI
										let mut target_di = None;
										let mut updated_display = String::new();
										for (di, item) in s.display_items.iter_mut().enumerate() {
											let mut matched = false;
											match item {
												whantv::DisplayItem::Single(m) => {
													if m.id.starts_with("temp_upload_") {
														m.id = msg_id.clone();
														m.text = text.to_string();
														m.media_type = media_type.clone();
														m.media_filename = media_filename.clone();
														matched = true;
													}
												}
												whantv::DisplayItem::Album { messages, .. } => {
													for m in messages.iter_mut() {
														if m.id.starts_with("temp_upload_") {
															m.id = msg_id.clone();
															m.text = text.to_string();
															m.media_type = media_type.clone();
															m.media_filename = media_filename.clone();
															matched = true;
															break;
														}
													}
												}
											}
											if matched {
												target_di = Some(di);
												updated_display = item.format_display(is_group);
												break;
											}
										}

										if let Some(di) = target_di {
											parts
												.conversations_parts
												.messages_list
												.set_string(di as u32, &updated_display);
											live_region::announce(
												parts.conversations_parts.messages_announcer.clone(),
												&updated_display,
											);
										}
									} else {
										let sender_name = if is_from_me {
											"You".to_string()
										} else {
											let mut clean_jid = from.to_string();
											if let Some(idx) = from.find(':') {
												clean_jid = format!("{}@s.whatsapp.net", &from[..idx]);
											}
											s.contacts
												.get(&clean_jid)
												.or_else(|| s.contacts.get(from))
												.cloned()
												.unwrap_or_else(|| clean_jid)
										};

										let mut insert_date = false;
										let mut date_str = String::new();
										if let Some(datetime) = chrono::DateTime::from_timestamp(ts, 0) {
											let local_datetime: chrono::DateTime<chrono::Local> = datetime.into();
											date_str = local_datetime.format("%A, %B %e, %Y").to_string();
											if let Some(last_msg) = s.messages.last() {
												if !last_msg.is_date_divider {
													if let Some(last_dt) =
														chrono::DateTime::from_timestamp(last_msg.timestamp, 0)
													{
														let last_local: chrono::DateTime<chrono::Local> =
															last_dt.into();
														let last_date_str =
															last_local.format("%A, %B %e, %Y").to_string();
														if date_str != last_date_str {
															insert_date = true;
														}
													}
												}
											} else {
												insert_date = true;
											}
										}

										let quoted_id = data
											.get("quoted_id")
											.or_else(|| data.get("quotedId"))
											.and_then(|q| q.as_str())
											.unwrap_or("")
											.to_string();
										let quoted_text = data
											.get("quoted_text")
											.or_else(|| data.get("quotedText"))
											.and_then(|q| q.as_str())
											.unwrap_or("")
											.to_string();
										let quoted_sender = data
											.get("quoted_sender")
											.or_else(|| data.get("quotedSender"))
											.and_then(|q| q.as_str())
											.unwrap_or("")
											.to_string();

										if insert_date && !date_str.is_empty() {
											let divider = ChatMessage {
												id: String::new(),
												text: date_str,
												timestamp: ts,
												is_date_divider: true,
												..Default::default()
											};
											s.messages.push(divider.clone());
											parts
												.conversations_parts
												.messages_list
												.append(&divider.format_display(is_group));
										}

										let chat_msg = ChatMessage {
											id: msg_id.clone(),
											text: text.to_string(),
											timestamp: ts,
											from_me: is_from_me,
											sender_jid: from.to_string(),
											sender_name,
											is_audio: text.starts_with("[Voice Message]")
												|| text.starts_with("[Audio]"),
											status: s
												.pending_receipts
												.remove(&msg_id)
												.unwrap_or_else(|| "sent".to_string()),
											sent_at: if is_from_me { ts } else { 0 },
											quoted_id,
											quoted_text,
											quoted_sender,
											media_type: data
												.get("mediaType")
												.and_then(|m| m.as_str())
												.unwrap_or("")
												.to_string(),
											media_filename: data
												.get("mediaFilename")
												.and_then(|m| m.as_str())
												.unwrap_or("")
												.to_string(),
											..Default::default()
										};
										s.messages.push(chat_msg.clone());

										// Incremental display_items update instead of full O(n) regrouping
										let is_deleted_msg = chat_msg.is_deleted || chat_msg.text.contains("deleted");
										let is_media_msg = !is_deleted_msg
											&& (chat_msg.media_type == "image"
												|| chat_msg.media_type == "video" || chat_msg
												.text
												.starts_with("[Photo]") || chat_msg.text.starts_with("[Image]")
												|| chat_msg.text.starts_with("[Video]"));

										let mut needs_full_rebuild = false;
										if is_media_msg {
											let should_merge = if let Some(last_item) = s.display_items.last() {
												match last_item {
													whantv::DisplayItem::Single(prev_msg) => {
														let prev_is_media =
															!prev_msg.is_deleted
																&& !prev_msg.text.contains("deleted") && (prev_msg
																.media_type == "image"
																|| prev_msg.media_type == "video"
																|| prev_msg.text.starts_with("[Photo]")
																|| prev_msg.text.starts_with("[Image]")
																|| prev_msg.text.starts_with("[Video]"));
														prev_is_media
															&& prev_msg.from_me == chat_msg.from_me && prev_msg.sender_jid
															== chat_msg.sender_jid && (chat_msg.timestamp
															- prev_msg.timestamp)
															.abs() <= 10
													}
													whantv::DisplayItem::Album {
														from_me,
														sender_jid,
														messages,
														..
													} => {
														*from_me == chat_msg.from_me
															&& sender_jid == &chat_msg.sender_jid && messages
															.last()
															.map_or(false, |m| {
																(chat_msg.timestamp - m.timestamp).abs() <= 10
															})
													}
												}
											} else {
												false
											};

											if should_merge {
												needs_full_rebuild = true;
											} else {
												s.display_items.push(whantv::DisplayItem::Single(chat_msg.clone()));
												parts
													.conversations_parts
													.messages_list
													.append(&chat_msg.format_display(is_group));
											}
										} else {
											s.display_items.push(whantv::DisplayItem::Single(chat_msg.clone()));
											parts
												.conversations_parts
												.messages_list
												.append(&chat_msg.format_display(is_group));
										}

										if needs_full_rebuild {
											s.display_items = whantv::group_messages_into_display_items(&s.messages);
											parts.conversations_parts.messages_list.freeze();
											parts.conversations_parts.messages_list.clear();
											for item in &s.display_items {
												parts
													.conversations_parts
													.messages_list
													.append(&item.format_display(is_group));
											}
											parts.conversations_parts.messages_list.thaw();
										}

										let count = parts.conversations_parts.messages_list.get_count();
										if count > 0 && is_from_me {
											parts
												.conversations_parts
												.messages_list
												.set_selection((count - 1) as u32, true);
										}
										let formatted = chat_msg.format_display(is_group);
										live_region::announce(
											parts.conversations_parts.messages_announcer.clone(),
											&formatted,
										);

										if !is_from_me {
											let mark_jid = jid.to_string();
											let mark_msg_id = msg_id.clone();
											let tx = s.ws_tx.clone();
											let payload = serde_json::json!({
												"action": "mark_read",
												"data": { "jid": mark_jid, "message_ids": vec![mark_msg_id] }
											});
											let sounds_enabled = true;
											let _ = tx.send(payload.to_string());
											play_notification_sound("sounds/message_current.ogg", sounds_enabled);
										}
									}
								} else {
									let is_ignored = jid == "status@broadcast"
										|| jid.ends_with("@broadcast") || jid.ends_with("@newsletter");
									if !is_ignored {
										let is_from_me = data
											.get("from_me")
											.and_then(|f| f.as_bool())
											.unwrap_or_else(|| from == "me" || from == "You");
										if !is_from_me {
											play_notification_sound("sounds/message_background.ogg", true);
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
									for msg in s.messages.iter_mut() {
										if msg.id == message_id {
											msg.is_deleted = true;
											msg.text = "".to_string();
											changed = true;
											break;
										}
									}
									if changed {
										// Update display_items in-place instead of full O(n) regrouping
										for item in s.display_items.iter_mut() {
											match item {
												whantv::DisplayItem::Single(m) if m.id == message_id => {
													m.is_deleted = true;
													m.text = String::new();
													break;
												}
												whantv::DisplayItem::Album { messages, .. } => {
													if let Some(am) = messages.iter_mut().find(|am| am.id == message_id)
													{
														am.is_deleted = true;
														am.text = String::new();
														break;
													}
												}
												_ => {}
											}
										}
										for (di, item) in s.display_items.iter().enumerate() {
											let contains = match item {
												whantv::DisplayItem::Single(m) => m.id == message_id,
												whantv::DisplayItem::Album { messages, .. } => {
													messages.iter().any(|m| m.id == message_id)
												}
											};
											if contains {
												parts
													.conversations_parts
													.messages_list
													.set_string(di as u32, &item.format_display(is_group));

												if let Some(sel_idx) =
													parts.conversations_parts.messages_list.get_selection()
												{
													if sel_idx as u32 == di as u32 {
														let mut need_layout = crate::update_action_buttons(
															&s,
															&parts.conversations_parts,
														);
														if parts.conversations_parts.play_voice_btn.is_shown() {
															parts.conversations_parts.play_voice_btn.show(false);
															need_layout = true;
														}
														if parts.conversations_parts.audio_slider.is_shown() {
															parts.conversations_parts.audio_slider.show(false);
															parts.conversations_parts.audio_progress_label.show(false);
															parts.conversations_parts.audio_speed_btn.show(false);
															need_layout = true;
														}
														if need_layout {
															parts.conversations_parts.action_panel.layout();
															parts.conversations_parts.conversation_panel.layout();
														}
													}
												}
												break;
											}
										}
										live_region::announce(parts.qrcode_announcer.clone(), "Message deleted.");
									}
								}
							}
						}
						"new_contact_status" => {
							if let Some(data) = val.get("data") {
								if let Some(status) = data.get("status").and_then(|s| s.as_str()) {
									if status == "success" {
										let payload = serde_json::json!({
											"action": "get_chats"
										});
										let _ = state.borrow().ws_tx.send(payload.to_string());
									} else if status == "error" {
										let err_msg =
											data.get("error").and_then(|e| e.as_str()).unwrap_or("Unknown error");
										unsafe {
											let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
												windows::Win32::UI::WindowsAndMessaging::MB_OK,
											);
										}
										MessageDialog::builder(&parts.new_contact_dialog.dialog, err_msg, "Error")
											.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
											.build()
											.show_modal();
									}
								}
							}
						}
						"create_group_status" | "new_group_status" => {
							if let Some(data) = val.get("data") {
								if let Some(status) = data.get("status").and_then(|s| s.as_str()) {
									if status == "success" {
										let payload = serde_json::json!({
											"action": "get_chats"
										});
										let _ = state.borrow().ws_tx.send(payload.to_string());
									} else if status == "error" {
										parts.new_group_dialog.ok_btn.enable(true);
										let err_msg =
											data.get("error").and_then(|e| e.as_str()).unwrap_or("Unknown error");
										unsafe {
											let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
												windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_STYLE(0x00000030),
											);
										}
										MessageDialog::builder(&parts.new_group_dialog.dialog, err_msg, "Error")
											.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
											.build()
											.show_modal();
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
													text: format!("Parse error: {} for json: {}", e, msg),
													..Default::default()
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

									let mut s = state.borrow_mut();
									let chat_jid = s.selected_chat.clone().unwrap_or_default();
									let chat_name =
										s.contacts.get(&chat_jid).cloned().unwrap_or_else(|| chat_jid.clone());
									let is_group_chat = chat_jid.ends_with("@g.us");

									let mut with_dates = Vec::new();
									let mut last_date = String::new();
									for mut msg in deduplicated {
										if !is_group_chat
											&& !msg.from_me && msg.sender_name.is_empty()
											&& msg.sender_jid.is_empty()
										{
											msg.sender_name = chat_name.clone();
										}

										if let Some(datetime) = chrono::DateTime::from_timestamp(msg.timestamp, 0) {
											let local_datetime: chrono::DateTime<chrono::Local> = datetime.into();
											let date_str = local_datetime.format("%A, %B %e, %Y").to_string();
											if date_str != last_date {
												last_date = date_str.clone();
												with_dates.push(ChatMessage {
													text: date_str,
													timestamp: msg.timestamp,
													is_date_divider: true,
													..Default::default()
												});
											}
										}
										with_dates.push(msg);
									}

									s.loading_messages = false;
									let was_history_loading = s.history_loading;
									s.history_loading = false;
									let old_count = s.prev_msg_count;
									let is_group =
										s.selected_chat.as_ref().map(|j| j.ends_with("@g.us")).unwrap_or(false);
									s.messages = with_dates;
									// Apply any receipts that arrived before messages finished loading.
									// Collect first to avoid simultaneous mutable borrows of s.
									let pending_ids: Vec<(String, String)> = s
										.messages
										.iter()
										.filter_map(|msg| {
											s.pending_receipts.get(&msg.id).map(|st| (msg.id.clone(), st.clone()))
										})
										.collect();
									for (id, st) in pending_ids {
										s.pending_receipts.remove(&id);
										if let Some(msg) = s.messages.iter_mut().find(|m| m.id == id) {
											if get_status_rank(&st) >= get_status_rank(&msg.status) {
												msg.status = st;
											}
										}
									}
									s.display_items = whantv::group_messages_into_display_items(&s.messages);
									let new_len = s.display_items.len();
									s.prev_msg_count = new_len;

									parts.conversations_parts.messages_list.freeze();
									parts.conversations_parts.messages_list.clear();
									for item in &s.display_items {
										parts.conversations_parts.messages_list.append(&item.format_display(is_group));
									}
									parts.conversations_parts.messages_list.thaw();

									let count = parts.conversations_parts.messages_list.get_count();
									let target_pos = if let Some(target_id) = s.search_target_id.take() {
										s.display_items.iter().position(|item| match item {
											whantv::DisplayItem::Single(m) => m.id == target_id,
											whantv::DisplayItem::Album { messages, .. } => {
												messages.iter().any(|m| m.id == target_id)
											}
										})
									} else if let Some(target_text) = s.search_target_text.take() {
										let is_generic = target_text == "[Voice Message]"
											|| target_text == "[Audio]" || target_text == "Voice message"
											|| target_text == "Audio" || target_text == "[Media]";
										if is_generic {
											None
										} else {
											s.display_items.iter().position(|item| match item {
												whantv::DisplayItem::Single(m) => {
													m.text == target_text
														|| (!m.text.is_empty() && m.text.contains(&target_text))
												}
												whantv::DisplayItem::Album { messages, .. } => {
													messages.iter().any(|m| {
														m.text == target_text
															|| (!m.text.is_empty() && m.text.contains(&target_text))
													})
												}
											})
										}
									} else {
										None
									};
									if let Some(pos) = target_pos {
										parts.conversations_parts.messages_list.set_selection(pos as u32, true);
										parts.conversations_parts.messages_list.set_focus();
									} else if count > 0 {
										if was_history_loading && new_len > old_count && old_count > 0 {
											let keep_idx = (new_len - old_count) as u32;
											if keep_idx < count {
												parts.conversations_parts.messages_list.set_selection(keep_idx, true);
											} else {
												parts.conversations_parts.messages_list.set_selection(0, true);
											}
										} else if old_count == 0 {
											parts
												.conversations_parts
												.messages_list
												.set_selection((count - 1) as u32, true);
											parts.conversations_parts.messages_list.set_focus();
										}
									}
									parts.conversations_parts.conversation_panel.freeze();
									if update_action_buttons(&s, &parts.conversations_parts) {
										parts.conversations_parts.conversation_panel.layout();
									}
									parts.conversations_parts.conversation_panel.thaw();

									if was_history_loading && new_len == old_count {
										if let Some(ref jid) = s.selected_chat.clone() {
											eprintln!(
												"[LOG UI] No new history messages received for {}. Marking history complete.",
												jid
											);
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
							live_region::announce(
								parts.conversations_parts.messages_announcer.clone(),
								"Could not load voice message",
							);
						}
						"message_info" => {
							if let Some(data) = val.get("data") {
								let fmt_ts = |ts: i64| -> String {
									if ts == 0 {
										return String::new();
									}
									let dt = chrono::DateTime::from_timestamp(ts, 0)
										.unwrap_or_default()
										.with_timezone(&chrono::Local);
									dt.format("%l:%M %p").to_string().trim().to_lowercase()
								};
								let msg_id = data.get("id").and_then(|v| v.as_str()).unwrap_or("");
								let mut sent_s = data.get("sent_at").and_then(|v| v.as_i64()).unwrap_or(0);
								let mut delivered_s = data.get("delivered_at").and_then(|v| v.as_i64()).unwrap_or(0);
								let mut read_s = data.get("read_at").and_then(|v| v.as_i64()).unwrap_or(0);
								let mut played_s = data.get("played_at").and_then(|v| v.as_i64()).unwrap_or(0);
								let mut edited_s = data.get("edited_at").and_then(|v| v.as_i64()).unwrap_or(0);
								let media_type = data.get("media_type").and_then(|v| v.as_str()).unwrap_or("");
								let is_voice = media_type == "voice" || media_type == "audio";

								{
									let s = state.borrow();
									if let Some(msg) = s.messages.iter().find(|m| m.id == msg_id) {
										if sent_s == 0 {
											sent_s = if msg.sent_at > 0 { msg.sent_at } else { msg.timestamp };
										}
										if delivered_s == 0 && msg.delivered_at > 0 {
											delivered_s = msg.delivered_at;
										}
										if read_s == 0 && msg.read_at > 0 {
											read_s = msg.read_at;
										}
										if played_s == 0 && msg.played_at > 0 {
											played_s = msg.played_at;
										}
										if edited_s == 0 && msg.edited_at > 0 {
											edited_s = msg.edited_at;
										}
									}
								}

								let mut lines = Vec::new();
								let sent_fmt = fmt_ts(sent_s);
								if !sent_fmt.is_empty() {
									lines.push(format!("Sent at {}", sent_fmt));
								}
								let del_fmt = fmt_ts(delivered_s);
								if !del_fmt.is_empty() {
									lines.push(format!("Delivered at {}", del_fmt));
								}
								let read_fmt = fmt_ts(read_s);
								if !read_fmt.is_empty() {
									lines.push(format!("Read at {}", read_fmt));
								}
								if is_voice {
									let played_fmt = fmt_ts(played_s);
									if !played_fmt.is_empty() {
										lines.push(format!("Played at {}", played_fmt));
									}
								}
								let edited_fmt = fmt_ts(edited_s);
								if !edited_fmt.is_empty() {
									lines.push(format!("Edited at {}", edited_fmt));
								}

								if lines.is_empty() {
									lines.push("No receipt information available.".to_string());
								}
								let info_text = lines.join("\n");

								let dlg = Dialog::builder(&parts.frame, "Message Info")
									.with_style(DialogStyle::DefaultDialogStyle)
									.build();
								let sizer = BoxSizer::builder(Orientation::Vertical).build();
								let txt = TextCtrl::builder(&dlg)
									.with_value(&info_text)
									.with_style(
										TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly | TextCtrlStyle::DontWrap,
									)
									.build();
								sizer.add(&txt, 1, SizerFlag::Expand | SizerFlag::All, 10);
								let ok_btn = Button::builder(&dlg).with_label("OK").build();
								sizer.add(&ok_btn, 0, SizerFlag::AlignCentre | SizerFlag::Bottom | SizerFlag::All, 8);
								dlg.set_sizer_and_fit(sizer, true);
								let dlg_close = dlg.clone();
								ok_btn.on_click(move |_| {
									dlg_close.end_modal(wxdragon::id::ID_OK);
								});
								dlg.show_modal();
							}
						}
						"history_updated" => {
							eprintln!(
								"[LOG UI] History sync completed on phone! Refreshing messages for active chat..."
							);
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
									let mut parsed_msgs: Vec<ChatMessage> = Vec::new();
									for msg_val in results {
										if let Ok(msg) = serde_json::from_value::<ChatMessage>(msg_val.clone()) {
											parsed_msgs.push(msg);
										}
									}

									let mut with_dates = Vec::new();
									let mut last_date = String::new();
									for msg in parsed_msgs {
										if let Some(datetime) = chrono::DateTime::from_timestamp(msg.timestamp, 0) {
											let local_datetime: chrono::DateTime<chrono::Local> = datetime.into();
											let date_str = local_datetime.format("%A, %B %e, %Y").to_string();
											if date_str != last_date {
												last_date = date_str.clone();
												with_dates.push(ChatMessage {
													text: date_str,
													timestamp: msg.timestamp,
													is_date_divider: true,
													..Default::default()
												});
											}
										}
										with_dates.push(msg);
									}

									let mut s = state.borrow_mut();
									let is_group =
										s.selected_chat.as_ref().map(|j| j.ends_with("@g.us")).unwrap_or(false);
									s.messages = with_dates;
									s.display_items = whantv::group_messages_into_display_items(&s.messages);

									parts.conversations_parts.messages_list.freeze();
									parts.conversations_parts.messages_list.clear();
									for item in &s.display_items {
										parts.conversations_parts.messages_list.append(&item.format_display(is_group));
									}
									parts.conversations_parts.messages_list.thaw();
									let count = parts.conversations_parts.messages_list.get_count();
									if count > 0 {
										parts.conversations_parts.messages_list.set_selection(0, true);
									}
								}
							}
						}
						"typing" => {
							if let Some(data) = val.get("data") {
								let chat_jid = data
									.get("chat_jid")
									.or_else(|| data.get("jid"))
									.and_then(|j| j.as_str())
									.unwrap_or("");
								let mut sender_jid = data
									.get("sender_jid")
									.or_else(|| data.get("jid"))
									.and_then(|j| j.as_str())
									.unwrap_or("");
								if sender_jid.is_empty() {
									sender_jid = chat_jid;
								}
								if chat_jid.is_empty() || sender_jid.is_empty() {
									return;
								}
								let is_typing = data.get("typing").and_then(|t| t.as_bool()).unwrap_or(false);
								let is_recording = data.get("recording").and_then(|r| r.as_bool()).unwrap_or(false);

								{
									let mut s = state.borrow_mut();
									if is_typing || is_recording {
										s.typing_timestamps.insert(chat_jid.to_string(), std::time::Instant::now());
									} else {
										s.typing_timestamps.remove(chat_jid);
									}
								}

								let s = state.borrow();
								if Some(chat_jid.to_string()) == s.selected_chat {
									let sender_name =
										s.contacts.get(sender_jid).cloned().unwrap_or_else(|| sender_jid.to_string());
									if !sender_name.is_empty() {
										let announcement = if is_recording {
											Some(format!("{} is recording audio...", sender_name))
										} else if is_typing {
											Some(format!("{} is typing...", sender_name))
										} else {
											None
										};
										if let Some(ann) = announcement {
											live_region::announce(
												parts.conversations_parts.messages_announcer.clone(),
												&ann,
											);
										}
										let btn_label = if is_recording || is_typing {
											if is_recording {
												format!("{} is recording audio...", sender_name)
											} else {
												format!("{} is typing...", sender_name)
											}
										} else {
											let norm_chat_jid = if let Some((user, server)) = chat_jid.split_once('@') {
												let base = user
													.split('.')
													.next()
													.unwrap_or(user)
													.split(':')
													.next()
													.unwrap_or(user);
												format!("{}@{}", base, server)
											} else {
												chat_jid.to_string()
											};
											s.presence_labels
												.get(chat_jid)
												.or_else(|| s.presence_labels.get(&norm_chat_jid))
												.cloned()
												.unwrap_or_else(|| format!("Conversation data"))
										};
										parts.conversations_parts.conv_data_btn.set_label(&btn_label);
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
									let base_user =
										user.split('.').next().unwrap_or(user).split(':').next().unwrap_or(user);
									format!("{}@{}", base_user, server)
								} else {
									jid.to_string()
								};
								let unavailable = data.get("unavailable").and_then(|u| u.as_bool()).unwrap_or(false);
								let last_seen = data.get("last_seen").and_then(|l| l.as_i64()).unwrap_or(0);
								let mut s = state.borrow_mut();

								let name = s
									.contacts
									.get(&clean_jid)
									.or_else(|| s.contacts.get(jid))
									.cloned()
									.unwrap_or_else(|| clean_jid.clone());

								let btn_label = if unavailable {
									if last_seen > 0 {
										if let Some(datetime) = chrono::DateTime::from_timestamp(last_seen, 0) {
											let local_dt: chrono::DateTime<chrono::Local> = datetime.into();
											let now = chrono::Local::now();
											let time_str = local_dt.format("%l:%M %p").to_string().trim().to_string();
											let date_str = if local_dt.date_naive() == now.date_naive() {
												format!("today at {}", time_str)
											} else {
												local_dt.format("%b %e at %l:%M %p").to_string().replace("  ", " ")
											};
											format!("{} (last seen {})", name, date_str)
										} else {
											name.clone()
										}
									} else {
										name.clone()
									}
								} else {
									format!("{} (online)", name)
								};

								s.presence_labels.insert(clean_jid.clone(), btn_label.clone());
								s.presence_labels.insert(jid.to_string(), btn_label.clone());

								if Some(clean_jid.clone()) == s.selected_chat
									|| Some(jid.to_string()) == s.selected_chat
								{
									let mut should_announce = true;
									if !s.initial_presence_received {
										s.initial_presence_received = true;
										should_announce = false;
									}

									let announcement = if unavailable {
										if last_seen > 0 {
											if let Some(datetime) = chrono::DateTime::from_timestamp(last_seen, 0) {
												let local_dt: chrono::DateTime<chrono::Local> = datetime.into();
												let now = chrono::Local::now();
												let time_str =
													local_dt.format("%l:%M %p").to_string().trim().to_string();
												let date_str = if local_dt.date_naive() == now.date_naive() {
													format!("today at {}", time_str)
												} else {
													local_dt.format("%b %e at %l:%M %p").to_string().replace("  ", " ")
												};
												format!("{}, last seen {}", name, date_str)
											} else {
												format!("{}, offline", name)
											}
										} else {
											format!("{}, offline", name)
										}
									} else {
										format!("{} is now online", name)
									};
									if should_announce {
										live_region::announce(
											parts.conversations_parts.messages_announcer.clone(),
											&announcement,
										);
									}
									parts.conversations_parts.conv_data_btn.set_label(&btn_label);
								}
							}
						}
						"user_info" => {
							if let Some(data) = val.get("data") {
								let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
								let status = data.get("status").and_then(|s| s.as_str()).unwrap_or("");
								let push_name = data.get("pushName").and_then(|p| p.as_str()).unwrap_or("");
								let full_name = data.get("fullName").and_then(|f| f.as_str()).unwrap_or("");

								let s = state.borrow();
								if Some(jid.to_string()) == s.selected_chat {
									let local_name = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.to_string());
									let raw_phone = jid.split('@').next().unwrap_or(jid);
									let mut phone = raw_phone.to_string();
									if let Ok(parsed) = phonenumber::parse(None, format!("+{}", raw_phone)) {
										phone = parsed.format().mode(phonenumber::Mode::International).to_string();
									}

									let mut text = format!("Name: {}\nPhone: {}\n", local_name, phone);
									if !push_name.is_empty() {
										text.push_str(&format!("WhatsApp Name: {}\n", push_name));
									} else if !full_name.is_empty() {
										text.push_str(&format!("WhatsApp Name: {}\n", full_name));
									} else {
										text.push_str("WhatsApp Name: Not available\n");
									}

									if !status.is_empty() {
										text.push_str(&format!("About/Status: {}\n", status));
									} else {
										text.push_str("About/Status: Not set\n");
									}

									parts.conversation_data_dialog.info_field.set_value(&text);
									live_region::announce(
										parts.conversations_parts.messages_announcer.clone(),
										&format!("Conversation data loaded for {}", local_name),
									);
								}
							}
						}
						"message_status" | "receipt" => {
							if let Some(data) = val.get("data") {
								let chat_jid = data
									.get("jid")
									.or_else(|| data.get("chat_jid"))
									.and_then(|j| j.as_str())
									.unwrap_or("");
								let raw_status = data
									.get("status")
									.or_else(|| data.get("type"))
									.and_then(|s| s.as_str())
									.unwrap_or("");

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

								// Normalize JID: strip :device suffix so "1234:0@s.whatsapp.net"
								// compares equal to "1234@s.whatsapp.net".
								let norm_jid_str = |jid: &str| -> String {
									let no_device = jid.split(':').next().unwrap_or(jid);
									if no_device.contains('@') {
										no_device.to_string()
									} else {
										jid.split('@').next().unwrap_or(jid).to_string()
									}
								};
								let norm_incoming = norm_jid_str(chat_jid);

								let mut s = state.borrow_mut();
								let is_match = if let Some(ref sel) = s.selected_chat {
									norm_incoming == norm_jid_str(sel)
								} else {
									false
								};

								if is_match {
									let is_group = chat_jid.ends_with("@g.us");
									let norm_status = match raw_status {
										"read-self" => "read",
										"sender" => "sent",
										"inactive" => "delivered",
										"" => "delivered",
										other => other,
									};
									let receipt_ts = data.get("timestamp").and_then(|t| t.as_i64()).unwrap_or(0);

									let mut any_updated = false;
									for target_id in &msg_ids {
										let mut found = false;
										for msg in s.messages.iter_mut() {
											if msg.id == *target_id {
												if get_status_rank(norm_status) >= get_status_rank(&msg.status) {
													msg.status = norm_status.to_string();
													if receipt_ts > 0 {
														match norm_status {
															"delivered" => {
																if msg.delivered_at == 0 {
																	msg.delivered_at = receipt_ts;
																}
															}
															"read" => {
																if msg.read_at == 0 {
																	msg.read_at = receipt_ts;
																}
															}
															"played" => {
																if msg.played_at == 0 {
																	msg.played_at = receipt_ts;
																}
															}
															_ => {}
														}
													}
												}
												found = true;
												any_updated = true;
												break;
											}
										}
										if !found {
											s.pending_receipts.insert(target_id.clone(), norm_status.to_string());
										}
									}
									if any_updated {
										// Update display items in-place instead of full O(n) regrouping
										// Collect status updates first to avoid simultaneous borrows
										let status_updates: Vec<(String, String, i64, i64, i64)> = s
											.messages
											.iter()
											.filter(|msg| msg_ids.contains(&msg.id))
											.map(|msg| {
												(
													msg.id.clone(),
													msg.status.clone(),
													msg.delivered_at,
													msg.read_at,
													msg.played_at,
												)
											})
											.collect();
										for (upd_id, upd_status, upd_del, upd_read, upd_played) in &status_updates {
											for item in s.display_items.iter_mut() {
												match item {
													whantv::DisplayItem::Single(m) if m.id == *upd_id => {
														m.status = upd_status.clone();
														m.delivered_at = *upd_del;
														m.read_at = *upd_read;
														m.played_at = *upd_played;
														break;
													}
													whantv::DisplayItem::Album { messages, .. } => {
														if let Some(am) =
															messages.iter_mut().find(|am| am.id == *upd_id)
														{
															am.status = upd_status.clone();
															am.delivered_at = *upd_del;
															am.read_at = *upd_read;
															am.played_at = *upd_played;
															break;
														}
													}
													_ => {}
												}
											}
										}
										for (di, item) in s.display_items.iter().enumerate() {
											let contains = match item {
												whantv::DisplayItem::Single(m) => msg_ids.contains(&m.id),
												whantv::DisplayItem::Album { messages, .. } => {
													messages.iter().any(|m| msg_ids.contains(&m.id))
												}
											};
											if contains {
												parts
													.conversations_parts
													.messages_list
													.set_string(di as u32, &item.format_display(is_group));
											}
										}
									}
									if any_updated && (norm_status == "sent" || norm_status == "delivered") {
										let mut should_play = false;
										for target_id in &msg_ids {
											if s.played_sent_sounds.insert(target_id.clone()) {
												should_play = true;
											}
										}
										if should_play {
											play_notification_sound("sounds/message_sent.ogg", true);
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
								let edited_at = data.get("edited_at").and_then(|e| e.as_i64()).unwrap_or(0);

								let norm_jid_edit = |jid: &str| -> String {
									if let Some((user, server)) = jid.split_once('@') {
										let base_user =
											user.split('.').next().unwrap_or(user).split(':').next().unwrap_or(user);
										format!("{}@{}", base_user, server)
									} else {
										jid.to_string()
									}
								};

								let mut s = state.borrow_mut();
								let is_match = if let Some(ref sel) = s.selected_chat {
									norm_jid_edit(chat_jid) == norm_jid_edit(sel)
								} else {
									false
								};

								if is_match {
									let is_group = chat_jid.ends_with("@g.us");
									let mut changed = false;
									for msg in s.messages.iter_mut() {
										if msg.id == target_id {
											msg.text = new_text.to_string();
											msg.is_edited = true;
											msg.edited_at = edited_at;
											changed = true;
											break;
										}
									}
									if changed {
										for msg in s.messages.iter_mut() {
											if msg.quoted_id == target_id {
												msg.quoted_text = new_text.to_string();
											}
										}
										// Update display_items in-place instead of full O(n) regrouping
										for item in s.display_items.iter_mut() {
											match item {
												whantv::DisplayItem::Single(m) if m.id == target_id => {
													m.text = new_text.to_string();
													m.is_edited = true;
													m.edited_at = edited_at;
													break;
												}
												whantv::DisplayItem::Album { messages, .. } => {
													if let Some(am) = messages.iter_mut().find(|am| am.id == target_id)
													{
														am.text = new_text.to_string();
														am.is_edited = true;
														am.edited_at = edited_at;
														break;
													}
												}
												_ => {}
											}
										}
										for item in s.display_items.iter_mut() {
											match item {
												whantv::DisplayItem::Single(m) if m.quoted_id == target_id => {
													m.quoted_text = new_text.to_string();
												}
												whantv::DisplayItem::Album { messages, .. } => {
													for am in messages.iter_mut() {
														if am.quoted_id == target_id {
															am.quoted_text = new_text.to_string();
														}
													}
												}
												_ => {}
											}
										}
										for (di, item) in s.display_items.iter().enumerate() {
											let contains = match item {
												whantv::DisplayItem::Single(m) => {
													m.id == target_id || m.quoted_id == target_id
												}
												whantv::DisplayItem::Album { messages, .. } => messages
													.iter()
													.any(|m| m.id == target_id || m.quoted_id == target_id),
											};
											if contains {
												let updated_display = item.format_display(is_group);
												parts
													.conversations_parts
													.messages_list
													.set_string(di as u32, &updated_display);
												live_region::announce(
													parts.conversations_parts.messages_announcer.clone(),
													&format!("Message edited: {}", updated_display),
												);
												break;
											}
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

								let norm_jid_react = |jid: &str| -> String {
									let no_device = jid.split(':').next().unwrap_or(jid);
									if no_device.contains('@') {
										no_device.to_string()
									} else {
										jid.split('@').next().unwrap_or(jid).to_string()
									}
								};

								let mut s = state.borrow_mut();
								let is_match = if let Some(ref sel) = s.selected_chat {
									norm_jid_react(chat_jid) == norm_jid_react(sel)
								} else {
									false
								};

								if is_match {
									let is_group = chat_jid.ends_with("@g.us");

									let is_own_reaction =
										reactor == "me" || reactor_name_val == "You" || reactor_name_val == "me";

									let resolved_name = if is_own_reaction {
										"You".to_string()
									} else if !reactor_name_val.is_empty() && !reactor_name_val.contains('@') {
										reactor_name_val.to_string()
									} else {
										s.contacts
											.get(norm_jid_react(reactor).as_str())
											.or_else(|| s.contacts.get(reactor))
											.cloned()
											.unwrap_or_else(|| "Contact".to_string())
									};

									for msg in s.messages.iter_mut() {
										if msg.id == target_id {
											if emoji.is_empty() {
												msg.reactions.retain(|r| {
													if is_own_reaction {
														r.reactor_name != "You"
															&& r.reactor_jid != "me" && r.reactor_name != "me"
													} else {
														r.reactor_jid != reactor && r.reactor_name != resolved_name
													}
												});
											} else {
												let mut found = false;
												for r in msg.reactions.iter_mut() {
													let r_is_own = r.reactor_name == "You" || r.reactor_jid == "me";
													if (is_own_reaction && r_is_own)
														|| (!is_own_reaction
															&& (r.reactor_jid == reactor
																|| r.reactor_name == resolved_name))
													{
														r.emoji = emoji.to_string();
														r.reactor_name = resolved_name.clone();
														found = true;
														break;
													}
												}
												if !found {
													msg.reactions.push(whantv::ChatReaction {
														reactor_jid: if is_own_reaction {
															"me".to_string()
														} else {
															reactor.to_string()
														},
														reactor_name: resolved_name.clone(),
														emoji: emoji.to_string(),
														timestamp: std::time::SystemTime::now()
															.duration_since(std::time::UNIX_EPOCH)
															.unwrap_or_default()
															.as_secs() as i64,
													});
												}
											}
											break;
										}
									}

									// Sync reactions into display_items in-place instead of full O(n) regrouping
									let updated_reactions =
										s.messages.iter().find(|m| m.id == target_id).map(|m| m.reactions.clone());
									if let Some(new_reactions) = updated_reactions {
										for item in s.display_items.iter_mut() {
											match item {
												whantv::DisplayItem::Single(m) if m.id == target_id => {
													m.reactions = new_reactions.clone();
													break;
												}
												whantv::DisplayItem::Album { messages, .. } => {
													if let Some(am) = messages.iter_mut().find(|am| am.id == target_id)
													{
														am.reactions = new_reactions.clone();
														break;
													}
												}
												_ => {}
											}
										}
									}
									for (di, item) in s.display_items.iter().enumerate() {
										let contains = match item {
											whantv::DisplayItem::Single(m) => m.id == target_id,
											whantv::DisplayItem::Album { messages, .. } => {
												messages.iter().any(|m| m.id == target_id)
											}
										};
										if contains {
											parts
												.conversations_parts
												.messages_list
												.set_string(di as u32, &item.format_display(is_group));
											let announce_msg = if emoji.is_empty() {
												format!("Reaction removed by {}", resolved_name)
											} else {
												format!("{} reacted with {}", resolved_name, emoji)
											};
											live_region::announce(
												parts.conversations_parts.messages_announcer.clone(),
												&announce_msg,
											);
											break;
										}
									}
								}
							}
						}
						"copied_album_to_clipboard" => {
							let ann_text = if let Some(data) = val.get("data") {
								let msg_ids: Vec<String> = data
									.get("message_ids")
									.and_then(|ids| ids.as_array())
									.map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
									.unwrap_or_default();

								let mut s = state.borrow_mut();
								let mut is_voice = false;
								for id in &msg_ids {
									s.pending_copy_announcements.remove(id);
									if let Some(m) = s.messages.iter().find(|msg| msg.id == *id) {
										if m.is_audio
											|| m.media_type == "voice" || m.media_type == "audio"
											|| m.text.starts_with("[Voice Message]")
											|| m.text.starts_with("[Audio]")
										{
											is_voice = true;
										}
									}
								}
								if is_voice {
									"Copied voice message to clipboard.".to_string()
								} else if msg_ids.len() <= 1 {
									"Copied file to clipboard.".to_string()
								} else {
									format!("Copied {} files to clipboard.", msg_ids.len())
								}
							} else {
								"Copied file to clipboard.".to_string()
							};

							live_region::announce(parts.conversations_parts.messages_announcer.clone(), &ann_text);
						}
						"media_downloaded" => {
							if let Some(data) = val.get("data") {
								let msg_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("").to_string();
								let mut path = data.get("path").and_then(|p| p.as_str()).unwrap_or("").to_string();
								let filename = data.get("filename").and_then(|f| f.as_str()).unwrap_or("").to_string();
								let clean_filename = ChatMessage::sanitize_filename(&filename);

								let clean_path = ChatMessage::sanitize_filename(&path);
								if clean_path != path && std::path::Path::new(&path).exists() {
									if std::fs::rename(&path, &clean_path).is_ok() {
										path = clean_path;
									}
								}

								let mut s = state.borrow_mut();
								s.downloads_in_progress.remove(&msg_id);
								if let Some(msg) = s.messages.iter_mut().find(|m| m.id == msg_id) {
									msg.media_path = path.clone();
									if !clean_filename.is_empty() {
										msg.media_filename = clean_filename.clone();
									}
								}
								// Update display_items in-place instead of full O(n) regrouping
								for item in s.display_items.iter_mut() {
									match item {
										whantv::DisplayItem::Single(m) if m.id == msg_id => {
											m.media_path = path.clone();
											if !clean_filename.is_empty() {
												m.media_filename = clean_filename.clone();
											}
											break;
										}
										whantv::DisplayItem::Album { messages, .. } => {
											if let Some(am) = messages.iter_mut().find(|am| am.id == msg_id) {
												am.media_path = path.clone();
												if !clean_filename.is_empty() {
													am.media_filename = clean_filename.clone();
												}
												break;
											}
										}
										_ => {}
									}
								}

								let download_had_focus = parts.conversations_parts.action_download_btn.has_focus();
								parts.conversations_parts.conversation_panel.freeze();
								if update_action_buttons(&s, &parts.conversations_parts) {
									parts.conversations_parts.action_panel.layout();
								}
								parts.conversations_parts.conversation_panel.thaw();

								if download_had_focus && !parts.conversations_parts.action_download_btn.is_shown() {
									if parts.conversations_parts.action_open_btn.is_shown() {
										parts.conversations_parts.action_open_btn.set_focus();
									} else if parts.conversations_parts.action_save_as_btn.is_shown() {
										parts.conversations_parts.action_save_as_btn.set_focus();
									}
								}

								let is_copy_pending = s.pending_copy_announcements.contains(&msg_id);
								let media_type = data.get("mediaType").and_then(|m| m.as_str()).unwrap_or("");
								if media_type != "voice" && !is_copy_pending {
									let announce_name = if filename.is_empty() { "File".to_string() } else { filename };
									live_region::announce(
										parts.conversations_parts.messages_announcer.clone(),
										&format!("{} downloaded", announce_name),
									);
								}
							}
						}
						"media_download_progress" => {
							if let Some(data) = val.get("data") {
								let msg_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("").to_string();
								let percent = data.get("percent").and_then(|p| p.as_i64()).unwrap_or(0);
								let mut s = state.borrow_mut();
								let prev = s.downloads_in_progress.insert(msg_id.clone(), percent);
								parts.conversations_parts.conversation_panel.freeze();
								if update_action_buttons(&s, &parts.conversations_parts) {
									parts.conversations_parts.action_panel.layout();
								}
								parts.conversations_parts.conversation_panel.thaw();
								for milestone in [25, 50, 75] {
									if prev.unwrap_or(0) < milestone && percent >= milestone {
										live_region::announce(
											parts.conversations_parts.messages_announcer.clone(),
											&format!("Download {}%", percent),
										);
										break;
									}
								}
							}
						}
						"media_download_cancelled" | "media_download_failed" => {
							if let Some(data) = val.get("data") {
								let msg_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("").to_string();
								let mut s = state.borrow_mut();
								s.downloads_in_progress.remove(&msg_id);
								parts.conversations_parts.conversation_panel.freeze();
								if update_action_buttons(&s, &parts.conversations_parts) {
									parts.conversations_parts.action_panel.layout();
								}
								parts.conversations_parts.conversation_panel.thaw();
								let announcement = if t == "media_download_cancelled" {
									"Download cancelled".to_string()
								} else {
									let err = data.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error");
									format!("Download failed: {}", err)
								};
								live_region::announce(
									parts.conversations_parts.messages_announcer.clone(),
									&announcement,
								);
								parts.conversations_parts.play_voice_btn.set_label("Play voice message");
							}
						}
						"media_saved" => {
							if let Some(data) = val.get("data") {
								let path = data.get("path").and_then(|p| p.as_str()).unwrap_or("");
								live_region::announce(
									parts.conversations_parts.messages_announcer.clone(),
									&format!("File saved to {}", path),
								);
							}
						}
						"play_audio" => {
							if let Some(data) = val.get("data") {
								if let Some(path) = data.get("path").and_then(|p| p.as_str()) {
									// Backend sends "message_id", not "id"
									let msg_id = data
										.get("message_id")
										.or_else(|| data.get("id"))
										.and_then(|i| i.as_str())
										.map(|s| s.to_string());
									eprintln!(
										"[LOG UI] Playing voice note: {} from path: {}",
										msg_id.as_deref().unwrap_or(""),
										path
									);
									let mut s = state.borrow_mut();
									// Keep playing_msg_id from the UI-side set (which used the correct msg.id),
									// only update it if the server echoes a non-empty id
									if let Some(ref id) = msg_id {
										if !id.is_empty() {
											s.playing_msg_id = Some(id.clone());
										}
									}
									// The user may have hit "pause" while this was still loading
									// (playing_msg_id is set immediately on click, before this
									// response arrives) - honor that instead of always starting
									// playback out from under them.
									let start_paused = s.is_audio_paused;
									s.audio_elapsed_secs = 0.0;
									s.audio_start_instant = None;
									s.audio_path = Some(path.to_string());
									if let Some(ref sink) = s.audio_sink {
										sink.stop();
									}
									s.audio_sink = None;

									// Duration strategy:
									// 1. Count samples via rodio (works for WAV/FLAC/MP3 but not opus)
									// 2. Parse "M:SS" from the message text e.g. "[Voice Message] (0:23)"
									// 3. Count samples by consuming the decoder (always works, small files)
									s.audio_total_secs = 0.0;

									// Try rodio total_duration first (fast, works for some formats)
									if let Ok(probe_file) = File::open(path) {
										if let Ok(probe_dec) = Decoder::new(BufReader::new(probe_file)) {
											use rodio::Source;
											if let Some(dur) = probe_dec.total_duration() {
												s.audio_total_secs = dur.as_secs_f32();
											}
										}
									}

									// Parse duration from message text: "[Voice Message] (M:SS)" or "[Audio] (M:SS)"
									if s.audio_total_secs == 0.0 {
										let playing_id = s.playing_msg_id.clone();
										if let Some(ref pid) = playing_id {
											if let Some(msg) = s.messages.iter().find(|m| &m.id == pid) {
												let text = &msg.text;
												// Find "(M:SS)" pattern inside the tag
												if let Some(open) = text.find('(') {
													if let Some(close) = text[open..].find(')') {
														let inner = &text[open + 1..open + close];
														// parse "M:SS"
														let parts_dur: Vec<&str> = inner.split(':').collect();
														if parts_dur.len() == 2 {
															if let (Ok(m), Ok(ss)) = (
																parts_dur[0].parse::<f32>(),
																parts_dur[1].parse::<f32>(),
															) {
																s.audio_total_secs = m * 60.0 + ss;
															}
														}
													}
												}
											}
										}
									}

									eprintln!("[LOG UI] Audio total duration: {}s", s.audio_total_secs);

									if s.audio_stream.is_none() {
										if let Ok(stream_handle) = DeviceSinkBuilder::open_default_sink() {
											s.audio_stream = Some(stream_handle);
										}
									}
									let mut played = false;
									if let Some(ref stream_handle) = s.audio_stream {
										let sink = Player::connect_new(&stream_handle.mixer());
										if let Ok(file) = File::open(path) {
											if let Ok(decoder) = Decoder::new(BufReader::new(file)) {
												sink.append(decoder.wsola(s.audio_speed));
												if start_paused {
													sink.pause();
												} else {
													sink.play();
												}
												s.audio_sink = Some(sink);
												played = true;
												s.audio_start_instant =
													if start_paused { None } else { Some(std::time::Instant::now()) };
												parts.conversations_parts.play_voice_btn.set_label(if start_paused {
													"Play voice message"
												} else {
													"Pause voice message"
												});
											} else if let Some(opus_buf) = decode_opus_file(path) {
												sink.append(opus_buf.wsola(s.audio_speed));
												if start_paused {
													sink.pause();
												} else {
													sink.play();
												}
												s.audio_sink = Some(sink);
												played = true;
												s.audio_start_instant =
													if start_paused { None } else { Some(std::time::Instant::now()) };
												parts.conversations_parts.play_voice_btn.set_label(if start_paused {
													"Play voice message"
												} else {
													"Pause voice message"
												});
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
    } elseif ($line.StartsWith("SEEK ")) {
        $sec = [double]($line.Substring(5))
        $p.PlaybackSession.Position = [TimeSpan]::FromSeconds($sec)
    }
}
"#;
											if let Ok(mut child) = std::process::Command::new("powershell")
												.args(&["-NoProfile", "-WindowStyle", "Hidden", "-Command", ps_script])
												.stdin(std::process::Stdio::piped())
												.creation_flags(0x08000000) // CREATE_NO_WINDOW
												.spawn()
											{
												s.audio_stdin = child.stdin.take();
												s.audio_process = Some(child);
											}
										}
										if let Some(ref mut stdin) = s.audio_stdin {
											let _ = writeln!(stdin, "PLAY {}", path_escaped);
											if start_paused {
												let _ = writeln!(stdin, "PAUSE");
											}
											let _ = stdin.flush();
											s.audio_start_instant =
												if start_paused { None } else { Some(std::time::Instant::now()) };
											parts.conversations_parts.play_voice_btn.set_label(if start_paused {
												"Play voice message"
											} else {
												"Pause voice message"
											});
										}
									}
									s.is_audio_paused = start_paused;
									// Show slider UI and speed button
									parts.conversations_parts.audio_slider.show(true);
									parts.conversations_parts.audio_speed_btn.show(true);
									parts.conversations_parts.audio_progress_label.show(false);
									let total = s.audio_total_secs;
									let max_sec = (total as i32).max(1);
									parts.conversations_parts.audio_slider.set_range(0, max_sec);
									parts.conversations_parts.audio_slider.set_value(0);
									let fmt_dur = |v: f32| {
										if v > 0.0 {
											format!("{}:{:02}", v as u32 / 60, v as u32 % 60)
										} else {
											"?:??".to_string()
										}
									};
									parts
										.conversations_parts
										.audio_progress_label
										.set_label(&format!("0:00 / {}", fmt_dur(total)));
									parts.conversations_parts.conversation_panel.layout();
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
								parts.attachment_dialog.attachment_info_label.set_label(&msg);

								let mut s = state.borrow_mut();
								let is_group = s.selected_chat.as_ref().map(|j| j.ends_with("@g.us")).unwrap_or(false);

								// 1. Update s.messages
								let mut temp_msg_idx = None;
								for (idx, m) in s.messages.iter().enumerate() {
									if m.id.starts_with("temp_upload_") {
										temp_msg_idx = Some(idx);
										break;
									}
								}

								if let Some(idx) = temp_msg_idx {
									let existing_text = &s.messages[idx].text;
									let caption_suffix = if existing_text.starts_with("[Uploading ") {
										if let Some(pos) = existing_text.find("] ") {
											existing_text[pos + 2..].to_string()
										} else {
											String::new()
										}
									} else {
										// If it already transformed to [Document] etc., just keep it as is,
										// but actually we shouldn't really overwrite it if it's no longer Uploading,
										// but to be safe we'll preserve the whole text as suffix if it lacks ]
										if let Some(pos) = existing_text.find("] ") {
											existing_text[pos + 2..].to_string()
										} else {
											String::new()
										}
									};
									s.messages[idx].text = if caption_suffix.is_empty() {
										format!("[{}]", msg)
									} else {
										format!("[{}] {}", msg, caption_suffix)
									};
								}

								// 2. Update s.display_items and UI
								let mut target_di = None;
								let mut updated_display = String::new();
								for (di, item) in s.display_items.iter_mut().enumerate() {
									let mut matched = false;
									match item {
										whantv::DisplayItem::Single(m) => {
											if m.id.starts_with("temp_upload_") {
												let existing_text = &m.text;
												let caption_suffix = if let Some(pos) = existing_text.find("] ") {
													existing_text[pos + 2..].to_string()
												} else {
													String::new()
												};
												m.text = if caption_suffix.is_empty() {
													format!("[{}]", msg)
												} else {
													format!("[{}] {}", msg, caption_suffix)
												};
												matched = true;
											}
										}
										whantv::DisplayItem::Album { messages, .. } => {
											for m in messages.iter_mut() {
												if m.id.starts_with("temp_upload_") {
													let existing_text = &m.text;
													let caption_suffix = if let Some(pos) = existing_text.find("] ") {
														existing_text[pos + 2..].to_string()
													} else {
														String::new()
													};
													m.text = if caption_suffix.is_empty() {
														format!("[{}]", msg)
													} else {
														format!("[{}] {}", msg, caption_suffix)
													};
													matched = true;
													break;
												}
											}
										}
									}
									if matched {
										target_di = Some(di);
										updated_display = item.format_display(is_group);
										break;
									}
								}

								if let Some(di) = target_di {
									parts.conversations_parts.messages_list.set_string(di as u32, &updated_display);
								}
								live_region::announce(parts.conversations_parts.messages_announcer.clone(), &msg);
							}
						}
						"attachment_sent" => {
							if let Some(data) = val.get("data") {
								let filename = data.get("filename").and_then(|f| f.as_str()).unwrap_or("file");
								let real_id = data.get("id").and_then(|i| i.as_str()).unwrap_or("");

								parts.attachment_dialog.attachment_info_label.set_label("");

								let mut s = state.borrow_mut();
								let is_group = s.selected_chat.as_ref().map(|j| j.ends_with("@g.us")).unwrap_or(false);

								// 1. Update s.messages
								let mut target_msg_idx = None;
								for (idx, m) in s.messages.iter().enumerate() {
									if m.id.starts_with("temp_upload_")
										|| m.status == "uploading" || m.status == "pending"
									{
										target_msg_idx = Some(idx);
										break;
									}
								}

								if let Some(idx) = target_msg_idx {
									if !real_id.is_empty() {
										s.messages[idx].id = real_id.to_string();
									}
									s.messages[idx].status = "sent".to_string();

									let text = &s.messages[idx].text;
									if text.starts_with("[Uploading ") {
										if let Some(pos) = text.find("] ") {
											s.messages[idx].text = text[pos + 2..].to_string();
										} else if text.ends_with(']') {
											s.messages[idx].text = String::new();
										}
									}
								}

								// 2. Update s.display_items and UI
								let mut target_di = None;
								let mut updated_display = String::new();
								for (di, item) in s.display_items.iter_mut().enumerate() {
									let mut matched = false;
									match item {
										whantv::DisplayItem::Single(m) => {
											if m.id.starts_with("temp_upload_")
												|| m.status == "uploading" || m.status == "pending"
											{
												if !real_id.is_empty() {
													m.id = real_id.to_string();
												}
												m.status = "sent".to_string();

												let text = &m.text;
												if text.starts_with("[Uploading ") {
													if let Some(pos) = text.find("] ") {
														m.text = text[pos + 2..].to_string();
													} else if text.ends_with(']') {
														m.text = String::new();
													}
												}
												matched = true;
											}
										}
										whantv::DisplayItem::Album { messages, .. } => {
											for m in messages.iter_mut() {
												if m.id.starts_with("temp_upload_")
													|| m.status == "uploading" || m.status == "pending"
												{
													if !real_id.is_empty() {
														m.id = real_id.to_string();
													}
													m.status = "sent".to_string();

													let text = &m.text;
													if text.starts_with("[Uploading ") {
														if let Some(pos) = text.find("] ") {
															m.text = text[pos + 2..].to_string();
														} else if text.ends_with(']') {
															m.text = String::new();
														}
													}
													matched = true;
													break;
												}
											}
										}
									}
									if matched {
										target_di = Some(di);
										updated_display = item.format_display(is_group);
										break;
									}
								}

								if let Some(di) = target_di {
									parts.conversations_parts.messages_list.set_string(di as u32, &updated_display);
									live_region::announce(
										parts.conversations_parts.messages_announcer.clone(),
										&updated_display,
									);
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
				0 => {
					// Conversations
					parts.conversations_parts.panel.show(true);
					state.borrow_mut().active_tab = 0;
					apply_chat_filter(&mut state.borrow_mut(), parts);
				}
				1 => {
					// Archived Chats
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
				s.initial_presence_received = false;
				s.messages.clear();
				s.loading_messages = false;
				s.no_more_messages = false;
				s.history_loading = false;
				s.prev_msg_count = 0;
				s.pending_attachments.clear();
				parts.conversations_parts.conversation_panel.show(true);
				parts.conversations_parts.play_voice_btn.show(false);
				parts.conversations_parts.action_panel.show(false);

				let name = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.clone());
				let label = s.presence_labels.get(jid).cloned().unwrap_or_else(|| name.clone());
				parts.conversations_parts.conv_data_btn.set_label(&label);

				parts.attachment_dialog.attachment_info_label.set_label("");
				parts.conversations_parts.messages_list.clear();
				parts.conversations_parts.message_field.set_value("");
				parts.conversations_parts.send_message_btn.show(false);
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
		UiCommand::ToggleSearchPanel => {
			let visible = parts.conversations_parts.search_panel.is_shown();
			parts.conversations_parts.search_panel.show(!visible);
			if !visible {
				parts.conversations_parts.search_conv_field.set_value("");
				parts.conversations_parts.search_conv_field.set_focus();
			}
			parts.conversations_parts.conversation_panel.layout();
		}
		UiCommand::CloseSearch => {
			// Hide search panel and reload the real chat messages
			parts.conversations_parts.search_panel.show(false);
			parts.conversations_parts.search_conv_field.set_value("");
			if let Some(jid) = state.borrow().selected_chat.clone() {
				let limit = 9999999;
				let payload = serde_json::json!({
					"action": "get_messages",
					"data": { "jid": jid, "limit": limit, "offset": 0 }
				});
				let _ = state.borrow().ws_tx.send(payload.to_string());
			}
			parts.conversations_parts.conversation_panel.layout();
		}
		UiCommand::JumpToSearchResult { message_id, message_text } => {
			// Hide search, set the target so get_messages scrolls to it, reload chat
			parts.conversations_parts.search_panel.show(false);
			parts.conversations_parts.search_conv_field.set_value("");
			{
				let mut s = state.borrow_mut();
				if !message_id.is_empty() {
					s.search_target_id = Some(message_id);
				} else if !message_text.is_empty() {
					s.search_target_text = Some(message_text);
				}
				if let Some(jid) = s.selected_chat.clone() {
					let limit = 9999999;
					let payload = serde_json::json!({
						"action": "get_messages",
						"data": { "jid": jid, "limit": limit, "offset": 0 }
					});
					let _ = s.ws_tx.send(payload.to_string());
				}
			}
			parts.conversations_parts.conversation_panel.layout();
		}
		UiCommand::StartRecording => {
			let mut s = state.borrow_mut();
			if s.selected_chat.is_some() {
				let temp_dir = std::env::temp_dir();
				let ts =
					std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
				let wav_path = temp_dir.join(format!("voice_rec_{}_0.wav", ts));
				let wav_path_str = wav_path.to_string_lossy().to_string();

				if mci_start_new_segment() {
					s.is_voice_recording = true;
					s.is_voice_paused = false;
					s.voice_recording_path = Some(wav_path_str);
					s.voice_recording_segments.clear();
					s.voice_recording_start = Some(std::time::Instant::now());
					s.voice_paused_duration = std::time::Duration::ZERO;
					s.voice_pause_instant = None;

					if let Some(ref jid) = s.selected_chat {
						let payload = serde_json::json!({
							"action": "send_typing",
							"data": { "jid": jid, "state": "composing", "media": "audio" }
						});
						let _ = s.ws_tx.send(payload.to_string());
					}

					parts.conversations_parts.message_field.show(false);
					parts.conversations_parts.add_attachment_btn.show(false);
					parts.conversations_parts.send_message_btn.show(false);
					parts.conversations_parts.record_voice_btn.show(false);
					parts.conversations_parts.discard_voice_btn.show(true);
					parts.conversations_parts.pause_resume_btn.show(true);
					parts.conversations_parts.send_voice_btn.show(true);
					parts.conversations_parts.pause_resume_btn.set_label("Pause recording");
					parts.conversations_parts.conversation_panel.layout();
					parts.conversations_parts.send_voice_btn.set_focus();
					live_region::announce(
						parts.conversations_parts.messages_announcer.clone(),
						"Recording voice message...",
					);
				} else {
					eprintln!("[LOG UI] Failed to start MCI voice recording");
					live_region::announce(
						parts.conversations_parts.messages_announcer.clone(),
						"Failed to start microphone recording.",
					);
				}
			}
		}
		UiCommand::StopRecording => {
			live_region::announce(
				parts.conversations_parts.messages_announcer.clone(),
				"Voice recording stopped. Ready to send.",
			);
		}
		UiCommand::TogglePauseRecording => {
			let mut s = state.borrow_mut();
			if s.is_voice_recording {
				if s.is_voice_paused {
					// Resume by starting a brand new segment rather than
					// trusting MCI's "resume" to keep appending to the same
					// file - some audio drivers silently stop capturing
					// after a resume, truncating everything recorded after
					// the pause.
					let temp_dir = std::env::temp_dir();
					let ts = std::time::SystemTime::now()
						.duration_since(std::time::UNIX_EPOCH)
						.unwrap_or_default()
						.as_secs();
					let seg_idx = s.voice_recording_segments.len();
					let wav_path = temp_dir.join(format!("voice_rec_{}_{}.wav", ts, seg_idx));
					let wav_path_str = wav_path.to_string_lossy().to_string();

					if mci_start_new_segment() {
						s.voice_recording_path = Some(wav_path_str);
						s.is_voice_paused = false;
						if let Some(inst) = s.voice_pause_instant.take() {
							s.voice_paused_duration += inst.elapsed();
						}
						parts.conversations_parts.pause_resume_btn.set_label("Pause recording");
						live_region::announce(
							parts.conversations_parts.messages_announcer.clone(),
							"Voice recording resumed.",
						);
					} else {
						eprintln!("[LOG UI] Failed to resume voice recording");
						live_region::announce(
							parts.conversations_parts.messages_announcer.clone(),
							"Failed to resume recording.",
						);
					}
				} else {
					// Pause by closing off the current segment as a
					// complete, valid WAV file instead of relying on MCI's
					// "pause" to keep the device alive correctly.
					if let Some(path) = s.voice_recording_path.take() {
						mci_finish_segment(&path);
						s.voice_recording_segments.push(path);
					}
					s.is_voice_paused = true;
					s.voice_pause_instant = Some(std::time::Instant::now());
					parts.conversations_parts.pause_resume_btn.set_label("Resume recording");
					live_region::announce(
						parts.conversations_parts.messages_announcer.clone(),
						"Voice recording paused.",
					);
				}
			}
		}
		UiCommand::ForceSync => {
			let _ = state.borrow().ws_tx.send(serde_json::json!({ "action": "force_sync" }).to_string());
		}
		UiCommand::SendVoiceMessage => {
			let (segments, rec_start, paused_dur, pause_inst, is_paused, jid_opt, sounds_enabled, ws_tx) = {
				let mut s = state.borrow_mut();
				s.is_voice_recording = false;
				// If still actively recording (not paused), close off the
				// final segment as a complete, valid WAV file.
				if let Some(path) = s.voice_recording_path.take() {
					mci_finish_segment(&path);
					s.voice_recording_segments.push(path);
				}
				(
					std::mem::take(&mut s.voice_recording_segments),
					s.voice_recording_start.take(),
					s.voice_paused_duration,
					s.voice_pause_instant.take(),
					s.is_voice_paused,
					s.selected_chat.clone(),
					true,
					s.ws_tx.clone(),
				)
			};

			if let Some(ref jid) = jid_opt {
				let payload = serde_json::json!({
					"action": "send_typing",
					"data": { "jid": jid, "state": "paused", "media": "" }
				});
				let _ = ws_tx.send(payload.to_string());
			}

			if !segments.is_empty() {
				if let (Some(start_inst), Some(jid)) = (rec_start, jid_opt) {
					let total_elapsed = start_inst.elapsed();
					let total_paused = paused_dur
						+ if is_paused {
							pause_inst.map(|i| i.elapsed()).unwrap_or_default()
						} else {
							std::time::Duration::ZERO
						};
					let active_secs = total_elapsed.saturating_sub(total_paused).as_secs().max(1);

					thread::spawn(move || {
						// Splice pause/resume segments into one continuous
						// WAV before encoding (a single-segment recording,
						// the common case, needs no splicing).
						let is_multi_segment = segments.len() > 1;
						let wav_path = if !is_multi_segment {
							segments[0].clone()
						} else {
							let merged_path = segments[0].replace(".wav", "_merged.wav");
							let result = concat_wav_segments(&segments, &merged_path);
							// The originals are fully absorbed into
							// `merged_path` now (or, on failure, no longer
							// useful) - clean them up either way.
							for seg in &segments {
								let _ = std::fs::remove_file(seg);
							}
							match result {
								Ok(()) => merged_path,
								Err(e) => {
									eprintln!("[LOG UI] Failed to splice voice recording segments: {}", e);
									String::new()
								}
							}
						};
						if wav_path.is_empty() {
							eprintln!("[LOG UI] No usable audio to send for voice message");
							return;
						}

						let ogg_path = wav_path.replace(".wav", ".ogg");
						let (final_path, mimetype) = match convert_wav_to_opus_ogg(&wav_path, &ogg_path) {
							Ok(_) => {
								// wav_path is only an intermediate once
								// converted; if it's the sole segment itself
								// leave it (matches the single-segment path,
								// which never deletes the source recording).
								if is_multi_segment {
									let _ = std::fs::remove_file(&wav_path);
								}
								(ogg_path, "audio/ogg; codecs=opus")
							}
							Err(e) => {
								eprintln!("[LOG UI] Error converting wav to opus natively: {}", e);
								(wav_path.clone(), "audio/wav")
							}
						};

						let payload = serde_json::json!({
							"action": "send_voice_message",
							"data": {
								"jid": jid,
								"path": final_path,
								"mimetype": mimetype,
								"duration": active_secs
							}
						});
						let _ = ws_tx.send(payload.to_string());
					});

					play_notification_sound("sounds/voicemsg_send.wav", sounds_enabled);
					live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Voice message sent.");
				}
			}

			parts.conversations_parts.message_field.show(true);
			parts.conversations_parts.add_attachment_btn.show(true);
			parts.conversations_parts.record_voice_btn.show(true);
			parts
				.conversations_parts
				.send_message_btn
				.show(!parts.conversations_parts.message_field.get_value().trim().is_empty());
			parts.conversations_parts.message_field.set_focus();

			parts.conversations_parts.discard_voice_btn.show(false);
			parts.conversations_parts.pause_resume_btn.show(false);
			parts.conversations_parts.send_voice_btn.show(false);

			parts.conversations_parts.conversation_panel.layout();
		}
		UiCommand::DiscardRecording | UiCommand::DiscardVoiceMessage => {
			{
				let mut s = state.borrow_mut();
				s.is_voice_recording = false;
				s.is_voice_paused = false;
				// The in-progress segment (if any) was never saved to disk,
				// so just abandon it via the "close voice_rec" below;
				// earlier segments from prior pause cycles were saved and
				// need real cleanup.
				s.voice_recording_path = None;
				for seg in s.voice_recording_segments.drain(..) {
					let _ = std::fs::remove_file(seg);
				}
				if let Some(ref jid) = s.selected_chat {
					let payload = serde_json::json!({
						"action": "send_typing",
						"data": { "jid": jid, "state": "paused", "media": "" }
					});
					let _ = s.ws_tx.send(payload.to_string());
				}
			}
			execute_mci_command("close voice_rec");

			parts.conversations_parts.message_field.show(true);
			parts.conversations_parts.add_attachment_btn.show(true);
			parts.conversations_parts.record_voice_btn.show(true);
			parts
				.conversations_parts
				.send_message_btn
				.show(!parts.conversations_parts.message_field.get_value().trim().is_empty());
			parts.conversations_parts.message_field.set_focus();

			parts.conversations_parts.discard_voice_btn.show(false);
			parts.conversations_parts.pause_resume_btn.show(false);
			parts.conversations_parts.send_voice_btn.show(false);

			parts.conversations_parts.conversation_panel.layout();
			live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Voice recording discarded.");
		}
		UiCommand::AddAttachment => {
			let file_dialog = FileDialog::builder(&parts.frame)
				.with_message("Choose file(s) to attach")
				.with_style(FileDialogStyle::Open | FileDialogStyle::FileMustExist | FileDialogStyle::Multiple)
				.build();
			if file_dialog.show_modal() == wxdragon::id::ID_OK {
				let paths = file_dialog.get_paths();
				if !paths.is_empty() {
					let mut s = state.borrow_mut();
					for path in paths {
						if !path.is_empty() {
							s.pending_attachments.push(path.clone());
						}
					}
					let file_names: Vec<String> = s
						.pending_attachments
						.iter()
						.map(|p| std::path::Path::new(p).file_name().unwrap_or_default().to_string_lossy().to_string())
						.collect();
					let label_text = format!("Files: {}", file_names.join(", "));
					drop(s);
					parts.attachment_dialog.attachment_info_label.set_label(&label_text);
					parts.attachment_dialog.caption_field.set_value("");
					live_region::announce(
						parts.conversations_parts.messages_announcer.clone(),
						&format!("Files attached: {}", file_names.len()),
					);

					if !parts.attachment_dialog.dialog.is_shown() {
						parts.attachment_dialog.caption_field.set_focus();
						parts.attachment_dialog.dialog.show_modal();
					} else {
						parts.attachment_dialog.caption_field.set_focus();
					}
				}
			}
		}
		UiCommand::PasteAttachment => {
			if let Some(_locker) = wxdragon::prelude::Clipboard::get().locker() {
				let mut fdo = wxdragon::prelude::FileDataObject::new();
				if wxdragon::prelude::Clipboard::get().get_data(&mut fdo) {
					let paths = fdo.get_files();
					if !paths.is_empty() {
						let mut s = state.borrow_mut();
						for path in paths {
							if !path.is_empty() {
								s.pending_attachments.push(path.clone());
							}
						}
						let file_names: Vec<String> = s
							.pending_attachments
							.iter()
							.map(|p| {
								std::path::Path::new(p).file_name().unwrap_or_default().to_string_lossy().to_string()
							})
							.collect();
						let label_text = format!("Files: {}", file_names.join(", "));
						drop(s);
						parts.attachment_dialog.attachment_info_label.set_label(&label_text);
						parts.attachment_dialog.caption_field.set_value("");
						live_region::announce(
							parts.conversations_parts.messages_announcer.clone(),
							&format!("Pasted files: {}", file_names.len()),
						);

						if !parts.attachment_dialog.dialog.is_shown() {
							parts.attachment_dialog.caption_field.set_focus();
							parts.attachment_dialog.dialog.show_modal();
						} else {
							parts.attachment_dialog.caption_field.set_focus();
						}
					}
				}
			}
		}
		UiCommand::DiscardAttachment => {
			{
				let mut s = state.borrow_mut();
				s.pending_attachments.clear();
			}
			parts.attachment_dialog.attachment_info_label.set_label("");
			parts.attachment_dialog.caption_field.set_value("");
			parts.attachment_dialog.dialog.end_modal(wxdragon::id::ID_CANCEL);
		}
		UiCommand::SendAttachment { caption } => {
			let jid = state.borrow().selected_chat.clone();
			let paths = {
				let mut s = state.borrow_mut();
				std::mem::take(&mut s.pending_attachments)
			};
			if let Some(ref jid) = jid {
				let is_group = jid.ends_with("@g.us");
				let ts =
					std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
						as i64;

				for (idx, path) in paths.iter().enumerate() {
					let ext =
						std::path::Path::new(path).extension().unwrap_or_default().to_string_lossy().to_lowercase();
					let file_type = match ext.as_str() {
						"jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" => "image",
						"mp4" | "avi" | "mov" | "mkv" | "webm" | "3gp" => "video",
						_ => "document",
					};
					let filename =
						std::path::Path::new(path).file_name().unwrap_or_default().to_string_lossy().to_string();

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
						status: "uploading".to_string(),
						sent_at: ts,
						..Default::default()
					};

					{
						let mut s = state.borrow_mut();
						s.messages.push(pending_msg.clone());
						s.display_items.push(whantv::DisplayItem::Single(pending_msg.clone()));
					}
					parts.conversations_parts.messages_list.append(&pending_msg.format_display(is_group));

					let count = parts.conversations_parts.messages_list.get_count();
					if count > 0 {
						parts.conversations_parts.messages_list.set_selection((count - 1) as u32, true);
						parts.conversations_parts.messages_list.set_focus();
					}
					live_region::announce(
						parts.conversations_parts.messages_announcer.clone(),
						&pending_msg.format_display(is_group),
					);

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
				parts.attachment_dialog.attachment_info_label.set_label("");
				parts.attachment_dialog.caption_field.set_value("");
				parts.attachment_dialog.dialog.end_modal(wxdragon::id::ID_OK);
			}
		}
		UiCommand::DownloadMedia => {
			let (to_start, to_cancel, jid_opt) = {
				let s = state.borrow();
				let sel = parts.conversations_parts.messages_list.get_selection();
				if let Some(idx) = sel {
					if let Some(item) = s.display_items.get(idx as usize) {
						let pending: Vec<(String, String)> = item
							.messages()
							.into_iter()
							.filter(|m| !m.is_media_downloaded())
							.map(|m| (m.id.clone(), m.display_filename()))
							.collect();
						let (in_progress, not_started): (Vec<_>, Vec<_>) =
							pending.into_iter().partition(|(id, _)| s.downloads_in_progress.contains_key(id));
						(not_started, in_progress, s.selected_chat.clone())
					} else {
						(Vec::new(), Vec::new(), None)
					}
				} else {
					(Vec::new(), Vec::new(), None)
				}
			};

			// Already downloading: clicking again means "cancel", not "start over".
			if let (Some(jid), false) = (jid_opt.clone(), to_cancel.is_empty()) {
				for (msg_id, _) in &to_cancel {
					let payload = serde_json::json!({
						"action": "cancel_download",
						"data": { "jid": jid, "message_id": msg_id }
					});
					let _ = state.borrow().ws_tx.send(payload.to_string());
				}
			}

			if let (Some(jid), false) = (jid_opt, to_start.is_empty()) {
				let count = to_start.len();
				{
					let mut s = state.borrow_mut();
					for (msg_id, _) in &to_start {
						s.downloads_in_progress.insert(msg_id.clone(), 0);
					}
				}
				parts.conversations_parts.conversation_panel.freeze();
				if update_action_buttons(&state.borrow(), &parts.conversations_parts) {
					parts.conversations_parts.conversation_panel.layout();
				}
				parts.conversations_parts.conversation_panel.thaw();

				for (msg_id, _) in &to_start {
					let payload = serde_json::json!({
						"action": "download_media",
						"data": { "jid": jid, "message_id": msg_id }
					});
					let _ = state.borrow().ws_tx.send(payload.to_string());
				}
				if count == 1 {
					live_region::announce(
						parts.conversations_parts.messages_announcer.clone(),
						&format!("Downloading {}", to_start[0].1),
					);
				} else {
					live_region::announce(
						parts.conversations_parts.messages_announcer.clone(),
						&format!("Downloading album of {} files...", count),
					);
				}
			}
		}
		UiCommand::OpenMedia => {
			let s = state.borrow();
			if let (Some(jid), Some(idx)) =
				(s.selected_chat.clone(), parts.conversations_parts.messages_list.get_selection())
			{
				if let Some(item) = s.display_items.get(idx as usize) {
					if let Some(msg) = item.messages().iter().find(|m| m.is_media_downloaded()) {
						let payload = serde_json::json!({
							"action": "open_media",
							"data": { "jid": jid, "message_id": msg.id }
						});
						let _ = s.ws_tx.send(payload.to_string());
					}
				}
			}
		}
		UiCommand::SaveMediaAs => {
			let item_opt = {
				let s = state.borrow();
				let sel = parts.conversations_parts.messages_list.get_selection();
				sel.and_then(|idx| s.display_items.get(idx as usize).cloned())
			};

			if let Some(item) = item_opt {
				match item {
					whantv::DisplayItem::Single(msg) => {
						if let Some(ref jid) = state.borrow().selected_chat {
							let filename = msg.display_filename();
							let dialog = FileDialog::builder(&parts.frame)
								.with_message("Save file as")
								.with_default_file(&filename)
								.with_style(FileDialogStyle::Save | FileDialogStyle::OverwritePrompt)
								.build();
							if dialog.show_modal() == wxdragon::id::ID_OK {
								if let Some(path) = dialog.get_path() {
									if msg.is_media_downloaded() {
										let _ = std::fs::copy(&msg.media_path, &path);
										live_region::announce(
											parts.conversations_parts.messages_announcer.clone(),
											"File saved successfully.",
										);
									} else {
										let payload = serde_json::json!({
											"action": "save_media_as",
											"data": { "jid": jid, "message_id": msg.id, "dest_path": path }
										});
										let _ = state.borrow().ws_tx.send(payload.to_string());
									}
								}
							}
						}
					}
					whantv::DisplayItem::Album { messages, .. } => {
						let photo_count = messages
							.iter()
							.filter(|m| {
								m.media_type == "image"
									|| m.text.starts_with("[Photo]")
									|| m.text.starts_with("[Image]")
							})
							.count();
						let video_count = messages
							.iter()
							.filter(|m| m.media_type == "video" || m.text.starts_with("[Video]"))
							.count();
						let (dlg_title, noun) = if photo_count > 0 && video_count == 0 {
							("Select folder to save all photos into", "photos")
						} else if video_count > 0 && photo_count == 0 {
							("Select folder to save all videos into", "videos")
						} else {
							("Select folder to save all media into", "media files")
						};
						let dir_dialog = wxdragon::DirDialog::builder(&parts.frame, dlg_title, "").build();
						if dir_dialog.show_modal() == wxdragon::id::ID_OK {
							if let Some(picked_path) = dir_dialog.get_path() {
								let dest_dir = std::path::PathBuf::from(&picked_path);
								let mut saved_count = 0;
								for (idx, msg) in messages.iter().enumerate() {
									if msg.is_media_downloaded() {
										let src = std::path::Path::new(&msg.media_path);
										let ext = src
											.extension()
											.and_then(|e| e.to_str())
											.unwrap_or(if msg.media_type == "video" { "mp4" } else { "jpg" });
										let fn_str = if !msg.media_filename.is_empty() {
											msg.media_filename.clone()
										} else {
											let prefix = if msg.media_type == "video" { "video" } else { "photo" };
											format!("{}_{:02}.{}", prefix, idx + 1, ext)
										};
										let dest_file = dest_dir.join(&fn_str);
										if std::fs::copy(src, &dest_file).is_ok() {
											saved_count += 1;
										}
									}
								}
								live_region::announce(
									parts.conversations_parts.messages_announcer.clone(),
									&format!("Saved {} {} to folder.", saved_count, noun),
								);
							}
						}
					}
				}
			}
		}
		UiCommand::CancelEdit => {
			parts.conversations_parts.message_field.set_value("");
			parts.conversations_parts.cancel_edit_btn.show(false);
			parts.conversations_parts.conversation_panel.layout();
			parts.conversations_parts.message_field.set_focus();
		}
		UiCommand::ShowConversationData => {
			let s = state.borrow();
			if let Some(ref jid) = s.selected_chat {
				let local_name = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.clone());
				let raw_phone = jid.split('@').next().unwrap_or(jid);
				let mut phone = raw_phone.to_string();
				if let Ok(parsed) = phonenumber::parse(None, format!("+{}", raw_phone)) {
					phone = parsed.format().mode(phonenumber::Mode::International).to_string();
				}

				let initial_text = format!(
					"Name: {}\nPhone: {}\nWhatsApp Name: Loading...\nAbout/Status: Loading...",
					local_name, phone
				);
				parts.conversation_data_dialog.info_field.set_value(&initial_text);

				let payload = serde_json::json!({
					"action": "get_user_info",
					"data": { "jid": jid }
				});
				let _ = s.ws_tx.send(payload.to_string());
				drop(s);

				parts.conversation_data_dialog.info_field.set_focus();
				parts.conversation_data_dialog.dialog.show_modal();
			}
		}
		UiCommand::RemoveQuote => {
			let mut s = state.borrow_mut();
			s.pending_reply_id = None;
			s.pending_reply_sender = None;
			s.pending_reply_preview = None;
			drop(s);
			parts.conversations_parts.remove_quote_btn.show(false);
			parts.conversations_parts.conversation_panel.layout();
			live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Quoted message removed.");
			parts.conversations_parts.message_field.set_focus();
		}
		UiCommand::SendTyping { jid, state: typing_state, media } => {
			let target_jid = if jid.is_empty() { state.borrow().selected_chat.clone() } else { Some(jid) };
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
		UiCommand::ForwardMessage { message_id, to_jids } => {
			let payload = serde_json::json!({
				"action": "forward_message",
				"data": {
					"message_id": message_id,
					"to_jids": to_jids
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
			live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Message forwarded.");
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
		UiCommand::GetMessageInfo { message_id } => {
			let payload = serde_json::json!({
				"action": "get_message_info",
				"data": { "id": message_id }
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::CopyAlbumToClipboard { message_ids } => {
			let (paths, is_single_voice): (Vec<String>, bool) = {
				let s = state.borrow();
				let mut p = Vec::new();
				let mut voice_count = 0;
				let temp_dir = std::env::temp_dir().join("whantv_clipboard");
				let _ = std::fs::create_dir_all(&temp_dir);

				for id in &message_ids {
					if let Some(msg) = s.messages.iter().find(|m| m.id == *id) {
						if msg.is_media_downloaded()
							&& !msg.media_path.is_empty()
							&& std::path::Path::new(&msg.media_path).exists()
						{
							let display_name = msg.display_filename();
							let temp_path = temp_dir.join(&display_name);
							let copy_path = if std::fs::copy(&msg.media_path, &temp_path).is_ok() {
								temp_path.to_string_lossy().to_string()
							} else {
								msg.media_path.clone()
							};
							p.push(copy_path);
							if msg.is_audio
								|| msg.media_type == "voice"
								|| msg.media_type == "audio"
								|| msg.text.starts_with("[Voice Message]")
								|| msg.text.starts_with("[Audio]")
							{
								voice_count += 1;
							}
						}
					}
				}
				(p, message_ids.len() == 1 && voice_count > 0)
			};

			if !paths.is_empty() {
				if copy_files_to_clipboard(&paths) {
					let ann_text = if is_single_voice {
						"Copied voice message to clipboard.".to_string()
					} else if paths.len() == 1 {
						"Copied file to clipboard.".to_string()
					} else {
						format!("Copied {} files to clipboard.", paths.len())
					};
					live_region::announce(parts.conversations_parts.messages_announcer.clone(), &ann_text);
				} else {
					live_region::announce(
						parts.conversations_parts.messages_announcer.clone(),
						"Failed to copy file to clipboard.",
					);
				}
			} else {
				let payload = serde_json::json!({
					"action": "copy_album_to_clipboard",
					"data": { "message_ids": message_ids }
				});
				{
					let mut s = state.borrow_mut();
					for id in &message_ids {
						s.pending_copy_announcements.insert(id.clone());
					}
					let _ = s.ws_tx.send(payload.to_string());
				}
				live_region::announce(
					parts.conversations_parts.messages_announcer.clone(),
					"Downloading file before copying to clipboard...",
				);
			}
		}
		UiCommand::SeekAudio(delta) => {
			let mut s = state.borrow_mut();
			if s.playing_msg_id.is_some() {
				let elapsed = if s.is_audio_paused {
					s.audio_elapsed_secs
				} else {
					s.audio_elapsed_secs + s.audio_start_instant.map(|i| i.elapsed().as_secs_f32()).unwrap_or(0.0)
				};
				let total = s.audio_total_secs;
				let target =
					if total > 0.0 { (elapsed + delta).max(0.0).min(total) } else { (elapsed + delta).max(0.0) };
				if let Some(ref sink) = s.audio_sink {
					sink.stop();
				}
				s.audio_sink = None;
				let path = s.audio_path.clone().unwrap_or_default();
				if let Some(ref stream_handle) = s.audio_stream {
					let new_sink = do_seek(stream_handle, &path, target, s.audio_speed);
					s.audio_sink = new_sink;
				}
				if let Some(ref mut stdin) = s.audio_stdin {
					use std::io::Write;
					let _ = writeln!(stdin, "SEEK {}", target);
					let _ = stdin.flush();
				}
				s.audio_elapsed_secs = target;
				s.audio_start_instant = Some(std::time::Instant::now());
				s.is_audio_paused = false;
				parts.conversations_parts.play_voice_btn.set_label("Pause voice message");
				let max_sec = (total as i32).max(1);
				let pos = (target as i32).max(0).min(max_sec);
				parts.conversations_parts.audio_slider.set_range(0, max_sec);
				parts.conversations_parts.audio_slider.set_value(pos);
			}
		}
		UiCommand::SeekAudioAbsolute(slider_val) => {
			let mut s = state.borrow_mut();
			if s.playing_msg_id.is_some() {
				let total = s.audio_total_secs;
				let target = (slider_val as f32).max(0.0).min(total);
				if let Some(ref sink) = s.audio_sink {
					sink.stop();
				}
				s.audio_sink = None;
				let path = s.audio_path.clone().unwrap_or_default();
				if let Some(ref stream_handle) = s.audio_stream {
					let new_sink = do_seek(stream_handle, &path, target, s.audio_speed);
					s.audio_sink = new_sink;
				}
				if let Some(ref mut stdin) = s.audio_stdin {
					use std::io::Write;
					let _ = writeln!(stdin, "SEEK {}", target);
					let _ = stdin.flush();
				}
				s.audio_elapsed_secs = target;
				s.audio_start_instant = Some(std::time::Instant::now());
				s.is_audio_paused = false;
				parts.conversations_parts.play_voice_btn.set_label("Pause voice message");
				let max_sec = (total as i32).max(1);
				let pos = (target as i32).max(0).min(max_sec);
				parts.conversations_parts.audio_slider.set_range(0, max_sec);
				parts.conversations_parts.audio_slider.set_value(pos);
			}
		}
		UiCommand::ReactToMessage { message_id, sender_jid, emoji } => {
			let s = state.borrow();
			if let Some(ref jid) = s.selected_chat {
				let payload = serde_json::json!({
					"action": "react_message",
					"data": {
						"jid": jid,
						"message_id": message_id,
						"sender_jid": sender_jid,
						"emoji": emoji
					}
				});
				let _ = s.ws_tx.send(payload.to_string());
			}
		}
		UiCommand::ToggleAudioSpeed => {
			let mut s = state.borrow_mut();
			let old_speed = s.audio_speed;
			let new_speed = if s.audio_speed >= 2.0 {
				1.0
			} else if s.audio_speed >= 1.5 {
				2.0
			} else {
				1.5
			};
			s.audio_speed = new_speed;
			parts.conversations_parts.audio_speed_btn.set_label(&format!("{:.1}x", new_speed));

			// Trigger soft-seek to apply new wsola speed
			if s.playing_msg_id.is_some() && s.audio_sink.is_some() {
				let was_paused = s.is_audio_paused;
				// Use the *old* speed: the wall-clock time elapsed since the
				// last seek/start was played back at the previous speed, not
				// the one we're switching to. Using the new speed here made
				// every speed change jump the position, i.e. "plays from a
				// bit before" where it actually was.
				let elapsed = s.audio_elapsed_secs
					+ s.audio_start_instant.map(|i| i.elapsed().as_secs_f32() * old_speed).unwrap_or(0.0);
				let target = elapsed.max(0.0).min(s.audio_total_secs);
				if let Some(ref sink) = s.audio_sink {
					sink.stop();
				}
				s.audio_sink = None;
				let path = s.audio_path.clone().unwrap_or_default();
				if let Some(ref stream_handle) = s.audio_stream {
					let new_sink = do_seek(stream_handle, &path, target, new_speed);
					if was_paused {
						if let Some(ref sink) = new_sink {
							sink.pause();
						}
					}
					s.audio_sink = new_sink;
				}
				s.audio_elapsed_secs = target;
				s.audio_start_instant = if was_paused { None } else { Some(std::time::Instant::now()) };
				s.is_audio_paused = was_paused;
				parts.conversations_parts.play_voice_btn.set_label(if was_paused {
					"Play voice message"
				} else {
					"Pause voice message"
				});
			}
		}
		UiCommand::TogglePinChat { jid, pin } => {
			let chat_name = {
				let mut s = state.borrow_mut();
				if pin {
					s.pinned_jids.insert(jid.clone());
				} else {
					s.pinned_jids.remove(&jid);
				}
				s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone())
			};
			let payload = serde_json::json!({
				"action": "set_chat_pinned",
				"data": {
					"jid": jid,
					"value": pin
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
			live_region::announce(
				parts.conversations_parts.messages_announcer.clone(),
				&format!("Chat {} {}.", chat_name, if pin { "pinned" } else { "unpinned" }),
			);
		}
		UiCommand::ToggleArchiveChat { jid, archive } => {
			let chat_name = {
				let mut s = state.borrow_mut();
				if archive {
					s.archived_jids.insert(jid.clone());
				} else {
					s.archived_jids.remove(&jid);
				}
				let name = s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
				apply_chat_filter(&mut s, parts);
				name
			};
			let payload = serde_json::json!({
				"action": "set_chat_archived",
				"data": {
					"jid": jid,
					"value": archive
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
			live_region::announce(
				parts.conversations_parts.messages_announcer.clone(),
				&format!("Chat {} {}.", chat_name, if archive { "archived" } else { "unarchived" }),
			);
		}
		UiCommand::ToggleMuteChat { jid, mute } => {
			let chat_name = {
				let mut s = state.borrow_mut();
				if mute {
					s.muted_jids.insert(jid.clone());
				} else {
					s.muted_jids.remove(&jid);
				}
				s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone())
			};
			let payload = serde_json::json!({
				"action": "set_chat_muted",
				"data": {
					"jid": jid,
					"muted": mute,
					"duration_seconds": 0
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
			live_region::announce(
				parts.conversations_parts.messages_announcer.clone(),
				&format!("Notifications {} for {}.", if mute { "muted" } else { "unmuted" }, chat_name),
			);
		}
		UiCommand::ToggleReadChat { jid, read } => {
			let chat_name = {
				let mut s = state.borrow_mut();
				if read {
					s.unread_counts.insert(jid.clone(), 0);
				} else {
					s.unread_counts.insert(jid.clone(), 1);
				}
				s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone())
			};
			let payload = serde_json::json!({
				"action": "set_chat_read",
				"data": {
					"jid": jid,
					"value": read
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
			live_region::announce(
				parts.conversations_parts.messages_announcer.clone(),
				&format!("Chat {} marked as {}.", chat_name, if read { "read" } else { "unread" }),
			);
		}
		UiCommand::DeleteChat { jid } => {
			let chat_name = {
				let mut s = state.borrow_mut();
				s.chats.retain(|j| j != &jid);
				s.filtered_chats.retain(|j| j != &jid);
				s.archived_jids.remove(&jid);
				s.pinned_jids.remove(&jid);
				s.muted_jids.remove(&jid);
				s.unread_counts.remove(&jid);
				let name = s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
				if s.selected_chat.as_ref() == Some(&jid) {
					s.selected_chat = None;
					parts.conversations_parts.conversation_panel.show(false);
				}
				apply_chat_filter(&mut s, parts);
				name
			};
			let payload = serde_json::json!({
				"action": "delete_chat",
				"data": {
					"jid": jid
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
			live_region::announce(
				parts.conversations_parts.messages_announcer.clone(),
				&format!("Chat {} deleted.", chat_name),
			);
		}
	}
}

/// Decodes an Ogg Opus file into interleaved f32 PCM at 48kHz.
///
/// This walks the raw Ogg page structure by hand instead of using the `ogg`
/// crate's `PacketReader`: that reader enforces strict page-continuity
/// bookkeeping and hard-errors (aborting the rest of the file) on real-world
/// voice notes - including ones recorded with pause/resume by WhatsApp's
/// own official app - that don't perfectly match its expectations, even
/// though the same files play back with the exact intended duration in VLC,
/// ffmpeg, and the official WhatsApp client. `PacketReader`'s only lenient
/// recovery path is gated behind an internal flag that's not reachable
/// through its public API without a real prior seek, and its packet data is
/// otherwise private, so there's no way to opt into leniency from outside
/// the crate. Since the Ogg page format itself is simple, this reimplements
/// just enough of it to reconstruct packets tolerantly: on any structural
/// oddity (a page continuation that doesn't line up, a lost sync pattern)
/// it drops only the one ambiguous packet at that seam and keeps going,
/// instead of losing everything after it.
fn decode_opus_file(path: &str) -> Option<rodio::buffer::SamplesBuffer> {
	use opus::{Channels, Decoder};

	fn feed_packet(
		packet: &[u8],
		decoder: &mut Option<Decoder>,
		channels: &mut usize,
		pre_skip: &mut usize,
		skipped: &mut usize,
		samples: &mut Vec<f32>,
	) {
		if packet.len() >= 8 && &packet[0..8] == b"OpusHead" {
			// Header packet, not audio - (re)establish the decoder from it.
			// Real chaining (a new pause/resume segment spliced in as its
			// own logical stream) shows up as more of these later in the
			// file; picking up its channel count/pre-skip here handles that
			// case too, not just the file's very first stream.
			*channels = if packet.len() >= 10 { (packet[9] as usize).max(1) } else { 1 };
			*pre_skip = if packet.len() >= 12 { u16::from_le_bytes([packet[10], packet[11]]) as usize } else { 0 };
			let opus_channels = if *channels >= 2 { Channels::Stereo } else { Channels::Mono };
			*decoder = Decoder::new(48000, opus_channels).ok();
			*skipped = 0;
			return;
		}
		if packet.len() >= 8 && &packet[0..8] == b"OpusTags" {
			return;
		}
		let Some(dec) = decoder.as_mut() else { return };
		let mut pcm = vec![0i16; 5760 * (*channels).max(1)]; // Max frame size for opus is 120ms at 48kHz
		if let Ok(n) = dec.decode(packet, &mut pcm, false) {
			pcm.truncate(n * (*channels).max(1));
			let mut frame: Vec<f32> = pcm.into_iter().map(|s| (s as f32) / 32768.0).collect();
			if *skipped < *pre_skip {
				let frames_here = frame.len() / (*channels).max(1);
				let to_skip = (*pre_skip - *skipped).min(frames_here);
				frame.drain(0..to_skip * (*channels).max(1));
				*skipped += to_skip;
			}
			samples.extend(frame);
		}
	}

	let data = std::fs::read(path).ok()?;
	let len = data.len();
	let mut pos: usize = 0;

	let mut decoder: Option<Decoder> = None;
	let mut channels: usize = 1;
	let mut pre_skip: usize = 0;
	let mut skipped: usize = 0;
	let mut samples: Vec<f32> = Vec::new();
	let mut pending: Vec<u8> = Vec::new(); // partial packet carried across a page boundary

	while pos + 27 <= len {
		if &data[pos..pos + 4] != b"OggS" {
			// Lost sync (or not actually at a page boundary yet) - scan
			// forward for the next capture pattern rather than giving up.
			match data[pos + 1..].windows(4).position(|w| w == b"OggS") {
				Some(off) => {
					pos += 1 + off;
					continue;
				}
				None => break,
			}
		}
		let header = &data[pos..pos + 27];
		let flags = header[5];
		let segs = header[26] as usize;
		if pos + 27 + segs > len {
			break;
		}
		let lacing = &data[pos + 27..pos + 27 + segs];
		let body_len: usize = lacing.iter().map(|&b| b as usize).sum();
		let body_start = pos + 27 + segs;
		if body_start + body_len > len {
			break;
		}
		let body = &data[body_start..body_start + body_len];

		let starts_with_continued = flags & 0x01 != 0;
		if !starts_with_continued {
			pending.clear();
		}

		let mut offset = 0usize;
		let mut seg_run_start = 0usize;
		let mut first_packet_of_page = true;
		for (i, &seg_len) in lacing.iter().enumerate() {
			offset += seg_len as usize;
			let is_packet_end = seg_len < 255;
			let is_last_segment = i + 1 == lacing.len();
			if is_packet_end {
				let packet_bytes = &body[seg_run_start..offset];
				seg_run_start = offset;
				let is_continuation = first_packet_of_page && starts_with_continued;
				first_packet_of_page = false;
				if is_continuation {
					if pending.is_empty() {
						// No matching partial packet to complete this with
						// (e.g. we just resynced) - can't recover it, drop
						// it and move on to the next one.
						continue;
					}
					let mut full = std::mem::take(&mut pending);
					full.extend_from_slice(packet_bytes);
					feed_packet(&full, &mut decoder, &mut channels, &mut pre_skip, &mut skipped, &mut samples);
				} else {
					feed_packet(packet_bytes, &mut decoder, &mut channels, &mut pre_skip, &mut skipped, &mut samples);
				}
			} else if is_last_segment {
				// Page ends mid-packet - stash it for the next page to
				// (hopefully) complete.
				pending.extend_from_slice(&body[seg_run_start..offset]);
			}
		}

		pos = body_start + body_len;
	}

	if samples.is_empty() {
		return None;
	}

	Some(rodio::buffer::SamplesBuffer::new(
		std::num::NonZeroU16::new(channels as u16).unwrap_or(std::num::NonZeroU16::new(1).unwrap()),
		std::num::NonZeroU32::new(48000).unwrap(),
		samples,
	))
}

fn convert_wav_to_opus_ogg(wav_path: &str, ogg_path: &str) -> anyhow::Result<()> {
	use std::{fs::File, io::BufWriter};

	use hound::WavReader;
	use ogg::PacketWriter;
	use opus::{Application, Channels, Encoder};

	let mut reader = WavReader::open(wav_path)?;
	let spec = reader.spec();

	if spec.sample_rate != 48000 {
		return Err(anyhow::anyhow!("Sample rate must be 48000"));
	}
	let is_stereo = spec.channels == 2;
	let opus_channels = if is_stereo { Channels::Stereo } else { Channels::Mono };

	let mut encoder = Encoder::new(48000, opus_channels, Application::Voip)?;
	let out_file = File::create(ogg_path)?;
	let mut writer = PacketWriter::new(BufWriter::new(out_file));
	let serial = 12345;

	// OpusHead
	let mut head = Vec::new();
	head.extend_from_slice(b"OpusHead");
	head.push(1); // Version
	head.push(spec.channels as u8); // Channels
	head.extend_from_slice(&384_u16.to_le_bytes()); // Preskip
	head.extend_from_slice(&48000_u32.to_le_bytes()); // Input sample rate
	head.extend_from_slice(&0_u16.to_le_bytes()); // Gain
	head.push(0); // Channel mapping family
	writer.write_packet(head, serial, ogg::PacketWriteEndInfo::EndPage, 0)?;

	// OpusTags
	let mut tags = Vec::new();
	tags.extend_from_slice(b"OpusTags");
	tags.extend_from_slice(&8_u32.to_le_bytes()); // Vendor length
	tags.extend_from_slice(b"whaNTV  ");
	tags.extend_from_slice(&0_u32.to_le_bytes()); // User comments list length
	writer.write_packet(tags, serial, ogg::PacketWriteEndInfo::EndPage, 0)?;

	let mut granule = 0;
	let mut samples = Vec::new();
	let samples_per_frame = if is_stereo { 960 * 2 } else { 960 };

	for sample in reader.samples::<i16>() {
		samples.push(sample?);
		if samples.len() == samples_per_frame {
			let mut out = vec![0u8; 1024];
			let len = encoder.encode(&samples, &mut out)?;
			out.truncate(len);
			granule += 960;
			writer.write_packet(out, serial, ogg::PacketWriteEndInfo::NormalPacket, granule)?;
			samples.clear();
		}
	}

	if !samples.is_empty() {
		let remaining_frames = samples.len() / (if is_stereo { 2 } else { 1 });
		samples.resize(samples_per_frame, 0);
		let mut out = vec![0u8; 1024];
		let len = encoder.encode(&samples, &mut out)?;
		out.truncate(len);
		granule += remaining_frames as u64;
		writer.write_packet(out, serial, ogg::PacketWriteEndInfo::EndStream, granule)?;
	}

	Ok(())
}
