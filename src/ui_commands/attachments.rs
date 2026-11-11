use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState,
	platform::clipboard::{self, copy_files_to_clipboard},
	ui::{contact_picker::refresh_attach_contact_list, message_display::update_action_buttons, window::WindowParts},
};

pub(crate) fn attach_files(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, kind: whatsnap::AttachKind) {
	if crate::ui_commands::blocking::stop_if_cant_send(state, parts) {
		return;
	}
	{
		let mut s = state.borrow_mut();
		if s.pending_attachments.is_empty() {
			s.pending_attachment_kind = kind;
		}
	}
	add_attachment(state, parts);
}

pub(crate) fn add_attachment(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	if crate::ui_commands::blocking::stop_if_cant_send(state, parts) {
		return;
	}
	let kind = state.borrow().pending_attachment_kind;
	let message = match kind {
		whatsnap::AttachKind::Media => "Choose photos or videos to send",
		whatsnap::AttachKind::Audio => "Choose audio files to send",
		_ => "Choose files to send as documents",
	};
	let file_dialog = FileDialog::builder(&parts.frame)
		.with_message(message)
		.with_wildcard(&kind.file_filter())
		.with_style(FileDialogStyle::Open | FileDialogStyle::FileMustExist | FileDialogStyle::Multiple)
		.build();
	if file_dialog.show_modal() == wxdragon::id::ID_OK {
		let paths = file_dialog.get_paths();
		if !paths.is_empty() {
			let left_out = add_to_batch(&mut state.borrow_mut(), paths);
			announce_left_out(parts, left_out);
			show_attachment_dialog(state, parts, "Files attached", None);
		}
	}
}

pub(crate) const MAX_BATCH_FILES: usize = 30;

fn add_to_batch(s: &mut AppState, paths: Vec<String>) -> usize {
	let mut left_out = 0;
	for path in paths.into_iter().filter(|p| !p.is_empty()) {
		if s.pending_attachments.len() < MAX_BATCH_FILES {
			s.pending_attachments.push(path);
		} else {
			left_out += 1;
		}
	}
	left_out
}

fn announce_left_out(parts: &Rc<WindowParts>, left_out: usize) {
	if left_out > 0 {
		crate::platform::announce::announce(
			parts.conversations_parts.messages_announcer,
			&format!(
				"You can send up to {MAX_BATCH_FILES} files at once; {left_out} file{} weren't added.",
				if left_out == 1 { "" } else { "s" }
			),
		);
	}
}

fn show_attachment_dialog(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	announcement: &str,
	caption: Option<String>,
) {
	let (kind, file_names) = {
		let s = state.borrow();
		let names: Vec<String> = s
			.pending_attachments
			.iter()
			.map(|p| std::path::Path::new(p).file_name().unwrap_or_default().to_string_lossy().to_string())
			.collect();
		(s.pending_attachment_kind, names)
	};
	let heading = match kind {
		whatsnap::AttachKind::Document => "Documents",
		whatsnap::AttachKind::Media => "Photos and videos",
		whatsnap::AttachKind::Audio => "Audio",
		whatsnap::AttachKind::Pasted => "Files",
	};
	let d = &parts.attachment_dialog;
	d.attachment_info_label.set_label(&format!("{}: {}", heading, file_names.join(", ")));
	let captioned = kind.has_caption();
	if !d.dialog.is_shown() {
		d.caption_field.set_value(caption.as_deref().unwrap_or(""));
		d.caption_field.set_insertion_point_end();
		d.view_once_check.set_value(false);
	}
	let view_once = {
		let s = state.borrow();
		!s.selected_chat.as_deref().is_some_and(|j| j.ends_with("@newsletter"))
			&& s.pending_attachments.iter().any(|p| matches!(kind.send_type(p), "image" | "video"))
	};
	if d.view_once_check.is_shown() != view_once {
		d.view_once_check.show(view_once);
		d.dialog.layout();
	}
	if d.caption_field.is_shown() != captioned {
		d.caption_label.show(captioned);
		d.caption_field.show(captioned);
		d.dialog.layout();
	}
	crate::platform::announce::announce(
		parts.conversations_parts.messages_announcer,
		&format!("{}: {}", announcement, file_names.len()),
	);
	if captioned {
		d.caption_field.set_focus();
	} else {
		d.send_btn.set_focus();
	}
	if !d.dialog.is_shown() {
		let res = crate::ui::show_modal_again(&d.dialog);
		if res != wxdragon::id::ID_OK {
			state.borrow_mut().pending_attachments.clear();
			d.attachment_info_label.set_label("");
			d.caption_field.set_value("");
		}
	}
}

pub(crate) fn open_poll_dialog(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	if crate::ui_commands::blocking::stop_if_cant_send(state, parts) {
		return;
	}
	if state.borrow().selected_chat.is_none() {
		return;
	}
	let d = &parts.poll_dialog;
	d.question_field.set_value("");
	d.options_field.set_value("");
	d.multiple_check.set_value(true);
	d.multiple_check.enable(true);
	d.quiz_check.set_value(false);
	d.question_field.set_focus();
	crate::ui::show_modal_again(&d.dialog);
}

pub(crate) fn send_poll(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	question: String,
	options: Vec<String>,
	multiple_answers: bool,
	correct_answer: Option<String>,
) {
	let Some(jid) = state.borrow().selected_chat.clone() else {
		return;
	};
	let (reply_id, reply_sender) = {
		let mut s = state.borrow_mut();
		let reply_id = s.pending_reply_id.take().unwrap_or_default();
		let reply_sender = s.pending_reply_sender.take().unwrap_or_default();
		s.pending_reply_preview = None;
		(reply_id, reply_sender)
	};
	parts.conversations_parts.remove_quote_btn.show(false);
	let payload = serde_json::json!({
		"action": "send_poll",
		"data": {
			"jid": jid,
			"question": question,
			"options": options,
			"multiple_answers": multiple_answers,
			"correct_answer": correct_answer,
			"reply_to_id": reply_id,
			"reply_to_sender": reply_sender
		}
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	let sending = if correct_answer.is_some() { "Sending quiz..." } else { "Sending poll..." };
	crate::platform::announce::announce(parts.conversations_parts.messages_announcer, sending);
}

pub(crate) fn prepare_attach_contact(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	if crate::ui_commands::blocking::stop_if_cant_send(state, parts) {
		return;
	}
	if state.borrow().selected_chat.is_none() {
		return;
	}
	parts.attach_contact_dialog.search_field.set_value("");
	parts.attach_contact_dialog.selection_announcer.set_label("");
	state.borrow_mut().attach_contact_state.checked_jids.clear();
	refresh_attach_contact_list("", state, parts);
	let payload = serde_json::json!({ "action": "get_contacts" });
	let _ = state.borrow().ws_tx.send(payload.to_string());
	crate::ui::show_modal_again(&parts.attach_contact_dialog.dialog);
}

pub(crate) fn send_contacts(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, contact_jids: Vec<String>) {
	let jid = state.borrow().selected_chat.clone();
	let (reply_id, reply_sender) = {
		let mut s = state.borrow_mut();
		let reply_id = s.pending_reply_id.take().unwrap_or_default();
		let reply_sender = s.pending_reply_sender.take().unwrap_or_default();
		s.pending_reply_preview = None;
		(reply_id, reply_sender)
	};
	parts.conversations_parts.remove_quote_btn.show(false);
	if let Some(jid) = jid {
		let payload = serde_json::json!({
			"action": "send_contacts",
			"data": {
				"jid": jid,
				"contact_jids": contact_jids,
				"reply_to_id": reply_id,
				"reply_to_sender": reply_sender
			}
		});
		let _ = state.borrow().ws_tx.send(payload.to_string());
		let count = contact_jids.len();
		crate::platform::announce::announce(
			parts.conversations_parts.messages_announcer,
			&format!("Sending {} contact{}...", count, if count == 1 { "" } else { "s" }),
		);
	}
}

pub(crate) fn paste_attachment(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	if crate::ui_commands::blocking::stop_if_cant_send(state, parts) {
		return;
	}
	let paths = {
		let clipboard = wxdragon::prelude::Clipboard::get();
		let Some(_locker) = clipboard.locker() else { return };
		let fdo = wxdragon::prelude::FileDataObject::new();
		if !clipboard.get_data(&fdo) {
			return;
		}
		fdo.get_files()
	};
	if paths.is_empty() {
		return;
	}
	let starting_batch = state.borrow().pending_attachments.is_empty();
	let mut chosen = whatsnap::AttachKind::Pasted;
	if starting_batch {
		let choices = whatsnap::AttachKind::paste_choices(&paths);
		if !choices.is_empty() {
			let labels: Vec<&str> = choices.iter().map(|(label, _)| *label).collect();
			let dialog = SingleChoiceDialog::builder(&parts.frame, "Send as:", "Send Pasted Files", &labels).build();
			if dialog.show_modal() != wxdragon::id::ID_OK {
				return;
			}
			chosen = choices.get(dialog.get_selection() as usize).map_or(chosen, |(_, kind)| *kind);
		}
	}
	let mut s = state.borrow_mut();
	if s.pending_attachments.is_empty() {
		s.pending_attachment_kind = chosen;
	}
	let left_out = add_to_batch(&mut s, paths);
	let kind = s.pending_attachment_kind;
	drop(s);
	announce_left_out(parts, left_out);
	let mut caption = None;
	if !parts.attachment_dialog.dialog.is_shown() && kind.has_caption() {
		let field = &parts.conversations_parts.message_field;
		let typed = field.get_value();
		if !typed.trim().is_empty() {
			field.set_value("");
			caption = Some(typed);
		}
	}
	show_attachment_dialog(state, parts, "Pasted files", caption);
}

pub(crate) fn discard_attachment(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	{
		let mut s = state.borrow_mut();
		s.pending_attachments.clear();
		s.pending_attachment_kind = whatsnap::AttachKind::default();
	}
	parts.attachment_dialog.attachment_info_label.set_label("");
	parts.attachment_dialog.caption_field.set_value("");
	parts.attachment_dialog.dialog.end_modal(wxdragon::id::ID_CANCEL);
}

pub(crate) fn send_attachment(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, caption: String) {
	let jid = state.borrow().selected_chat.clone();
	let (paths, kind, reply_id, reply_sender) = {
		let mut s = state.borrow_mut();
		let paths = std::mem::take(&mut s.pending_attachments);
		let kind = std::mem::take(&mut s.pending_attachment_kind);
		let reply_id = s.pending_reply_id.take().unwrap_or_default();
		let reply_sender = s.pending_reply_sender.take().unwrap_or_default();
		s.pending_reply_preview = None;
		(paths, kind, reply_id, reply_sender)
	};
	parts.conversations_parts.remove_quote_btn.show(false);
	if let Some(ref jid) = jid {
		let caption = if kind.has_caption() { caption } else { String::new() };
		let view_once =
			parts.attachment_dialog.view_once_check.is_shown() && parts.attachment_dialog.view_once_check.get_value();
		let ws_tx = state.borrow().ws_tx.clone();
		send_files(&ws_tx, jid, &paths, kind, &caption, &reply_id, &reply_sender, view_once);
		parts.attachment_dialog.attachment_info_label.set_label("");
		parts.attachment_dialog.caption_field.set_value("");
		parts.attachment_dialog.dialog.end_modal(wxdragon::id::ID_OK);
	}
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn send_files(
	ws_tx: &std::sync::mpsc::Sender<String>,
	jid: &str,
	paths: &[String],
	kind: whatsnap::AttachKind,
	caption: &str,
	reply_id: &str,
	reply_sender: &str,
	view_once: bool,
) {
	for (idx, path) in paths.iter().enumerate() {
		let file_type = kind.send_type(path);
		let filename = std::path::Path::new(path).file_name().unwrap_or_default().to_string_lossy().to_string();

		let mut payload = serde_json::json!({
			"action": "send_attachment",
			"data": {
				"jid": jid,
				"type": file_type,
				"path": path,
				"filename": filename,
				"caption": if idx == 0 { caption } else { "" },
				"reply_to_id": if idx == 0 { reply_id } else { "" },
				"reply_to_sender": if idx == 0 { reply_sender } else { "" },
				"view_once": view_once && matches!(file_type, "image" | "video")
			}
		});
		let ws_tx = ws_tx.clone();
		if file_type == "audio" {
			let path = path.clone();
			std::thread::spawn(move || {
				payload["data"]["duration"] = serde_json::json!(crate::audio::audio_file_seconds(&path));
				payload["data"]["waveform"] = serde_json::json!(crate::audio::audio_file_waveform(&path));
				let _ = ws_tx.send(payload.to_string());
			});
		} else if file_type == "image" && path.to_lowercase().ends_with(".gif") {
			let path = path.clone();
			std::thread::spawn(move || {
				let mp4 = std::env::temp_dir().join(format!("whantv_gif_{}.mp4", crate::util::random_hex()));
				let mp4 = mp4.to_string_lossy().to_string();
				match crate::media::gif_video::gif_to_mp4(&path, &mp4) {
					Ok(first_png) => {
						payload["data"]["type"] = serde_json::json!("video");
						payload["data"]["path"] = serde_json::json!(mp4);
						payload["data"]["gif_playback"] = serde_json::json!(true);
						payload["data"]["thumbnail"] = serde_json::json!(crate::util::base64(&first_png));
					}
					Err(e) => eprintln!("[LOG UI] Couldn't turn {path} into a video, sending it as a photo: {e}"),
				}
				let _ = ws_tx.send(payload.to_string());
			});
		} else if file_type == "video" {
			let path = path.clone();
			std::thread::spawn(move || {
				if let Some(png) = crate::media::video_thumbnail::first_frame_png(&path) {
					payload["data"]["thumbnail"] = serde_json::json!(crate::util::base64(&png));
				}
				let _ = ws_tx.send(payload.to_string());
			});
		} else {
			let _ = ws_tx.send(payload.to_string());
		}
	}
}

pub(crate) fn download_media(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let (to_start, to_cancel, jid_opt) = {
		let s = state.borrow();
		let sel = parts.conversations_parts.messages_list.get_selection();
		if let Some(idx) = sel {
			if let Some(item) = s.display_items.get(idx as usize) {
				let pending: Vec<(String, String)> = item
					.messages()
					.iter()
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

	crate::backend::events::ui_timing_log(&format!(
		"Download pressed: {} to start, {} to cancel, chat {}",
		to_start.len(),
		to_cancel.len(),
		if jid_opt.is_some() { "open" } else { "none" }
	));
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
			crate::platform::announce::announce(
				parts.conversations_parts.messages_announcer,
				&format!("Downloading {}", to_start[0].1),
			);
		} else {
			crate::platform::announce::announce(
				parts.conversations_parts.messages_announcer,
				&format!("Downloading album of {} files...", count),
			);
		}
	}
}

pub(crate) fn open_media(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	if open_album_items(state, parts) {
		return;
	}
	let s = state.borrow();
	if let (Some(jid), Some(idx)) = (s.selected_chat.clone(), parts.conversations_parts.messages_list.get_selection())
		&& let Some(item) = s.display_items.get(idx as usize)
		&& let Some(msg) = item.messages().iter().find(|m| m.is_media_downloaded())
	{
		if plays_in_app(msg) {
			let msg = msg.clone();
			drop(s);
			play_video(state, parts, &jid, &msg);
			return;
		}
		let payload = serde_json::json!({
			"action": "open_media",
			"data": { "jid": jid, "message_id": msg.id }
		});
		let _ = s.ws_tx.send(payload.to_string());
	}
}

/// Whether `msg` opens in the app's own video player. View-once videos don't:
/// the backend has to see them opened, since that's what uses them up.
fn plays_in_app(msg: &whatsnap::ChatMessage) -> bool {
	msg.media_type == "video" && !msg.text.starts_with("[View once]")
}

fn play_video(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: &str, msg: &whatsnap::ChatMessage) {
	let title = if msg.from_me {
		"Video you sent".to_string()
	} else if !msg.sender_name.is_empty() {
		format!("Video from {}", msg.sender_name)
	} else {
		"Video".to_string()
	};
	let choice = crate::ui::video_player::show(&parts.frame, &msg.media_path, &title, &msg.display_filename());
	crate::platform::tray::enter_release_guard().set(true);
	if let crate::ui::video_player::VideoChoice::OpenElsewhere = choice {
		let payload = serde_json::json!({
			"action": "open_media",
			"data": { "jid": jid, "message_id": msg.id }
		});
		let _ = state.borrow().ws_tx.send(payload.to_string());
	}
}

/// Says where a Save as copy went, or that it failed. A failed copy used to be
/// announced as saved, so the user went looking for a file that never existed.
fn announce_saved(parts: &Rc<WindowParts>, result: std::io::Result<u64>, path: &str) {
	let text = match result {
		Ok(_) => format!("File saved to {path}"),
		Err(err) => format!("Couldn't save the file: {err}"),
	};
	crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &text);
}

pub(crate) fn save_media_as(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let item_opt = {
		let s = state.borrow();
		let sel = parts.conversations_parts.messages_list.get_selection();
		sel.and_then(|idx| s.display_items.get(idx as usize).cloned())
	};

	if let Some(item) = item_opt {
		match item {
			whatsnap::DisplayItem::Single(msg) => {
				let jid_opt = state.borrow().selected_chat.clone();
				if let Some(jid) = jid_opt {
					let filename = msg.display_filename();
					let dialog = FileDialog::builder(&parts.frame)
						.with_message("Save file as")
						.with_default_file(&filename)
						.with_style(FileDialogStyle::Save | FileDialogStyle::OverwritePrompt)
						.build();
					if dialog.show_modal() == wxdragon::id::ID_OK
						&& let Some(path) = dialog.get_path()
					{
						if msg.is_media_downloaded() {
							announce_saved(parts, std::fs::copy(&msg.media_path, &path), &path);
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
			whatsnap::DisplayItem::Album { messages, .. } => {
				let photo_count = messages
					.iter()
					.filter(|m| {
						m.media_type == "image" || m.text.starts_with("[Photo]") || m.text.starts_with("[Image]")
					})
					.count();
				let video_count =
					messages.iter().filter(|m| m.media_type == "video" || m.text.starts_with("[Video]")).count();
				let (dlg_title, noun) = if photo_count > 0 && video_count == 0 {
					("Select folder to save all photos into", "photos")
				} else if video_count > 0 && photo_count == 0 {
					("Select folder to save all videos into", "videos")
				} else {
					("Select folder to save all media into", "media files")
				};
				let dir_dialog = wxdragon::DirDialog::builder(&parts.frame, dlg_title, "").build();
				if dir_dialog.show_modal() == wxdragon::id::ID_OK
					&& let Some(picked_path) = dir_dialog.get_path()
				{
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
					crate::platform::announce::announce(
						parts.conversations_parts.messages_announcer,
						&format!("Saved {} {} to folder.", saved_count, noun),
					);
				}
			}
		}
	}
}

pub(crate) fn download_media_message(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, message_id: String) {
	let (jid_opt, needs_download, filename) = {
		let s = state.borrow();
		let msg = s.messages.iter().find(|m| m.id == message_id);
		(
			s.selected_chat.clone(),
			msg.map(|m| !m.is_media_downloaded()).unwrap_or(false),
			msg.map(|m| m.display_filename()).unwrap_or_default(),
		)
	};
	if let (Some(jid), true) = (jid_opt, needs_download) {
		state.borrow_mut().downloads_in_progress.insert(message_id.clone(), 0);
		let payload = serde_json::json!({
			"action": "download_media",
			"data": { "jid": jid, "message_id": message_id }
		});
		let _ = state.borrow().ws_tx.send(payload.to_string());
		crate::platform::announce::announce(
			parts.conversations_parts.messages_announcer,
			&format!("Downloading {}", filename),
		);
	}
}

pub(crate) fn open_media_message(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, message_id: String) {
	let s = state.borrow();
	if let Some(jid) = s.selected_chat.clone()
		&& let Some(msg) = s.messages.iter().find(|m| m.id == message_id && m.is_media_downloaded())
	{
		if plays_in_app(msg) {
			let msg = msg.clone();
			drop(s);
			play_video(state, parts, &jid, &msg);
			return;
		}
		let payload = serde_json::json!({
			"action": "open_media",
			"data": { "jid": jid, "message_id": message_id }
		});
		let _ = s.ws_tx.send(payload.to_string());
	}
}

pub(crate) fn save_media_message_as(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, message_id: String) {
	let (jid_opt, msg_opt) = {
		let s = state.borrow();
		(s.selected_chat.clone(), s.messages.iter().find(|m| m.id == message_id).cloned())
	};
	if let (Some(jid), Some(msg)) = (jid_opt, msg_opt) {
		let filename = msg.display_filename();
		let dialog = FileDialog::builder(&parts.frame)
			.with_message("Save file as")
			.with_default_file(&filename)
			.with_style(FileDialogStyle::Save | FileDialogStyle::OverwritePrompt)
			.build();
		if dialog.show_modal() == wxdragon::id::ID_OK
			&& let Some(path) = dialog.get_path()
		{
			if msg.is_media_downloaded() {
				announce_saved(parts, std::fs::copy(&msg.media_path, &path), &path);
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

pub(crate) fn copy_album_to_clipboard(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	message_ids: Vec<String>,
) {
	let (paths, is_single_voice): (Vec<String>, bool) = {
		let s = state.borrow();
		let mut p = Vec::new();
		let mut all_local = true;
		let mut voice_count = 0;
		let temp_dir = std::env::temp_dir().join("whantv_clipboard");
		let _ = std::fs::create_dir_all(&temp_dir);
		let mut used_names: std::collections::HashMap<String, u32> = std::collections::HashMap::new();

		for id in &message_ids {
			if let Some(msg) = s.messages.iter().find(|m| m.id == *id)
				&& msg.is_media_downloaded()
				&& !msg.media_path.is_empty()
				&& std::path::Path::new(&msg.media_path).exists()
			{
				let display_name = clipboard::unique_clipboard_filename(&msg.display_filename(), &mut used_names);
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
			} else {
				all_local = false;
			}
		}
		// Files still to download: the backend fetches them all and copies
		// the whole set, rather than copying only those already here.
		if !all_local {
			p.clear();
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
			crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &ann_text);
		} else {
			crate::platform::announce::announce(
				parts.conversations_parts.messages_announcer,
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
		crate::platform::announce::announce(
			parts.conversations_parts.messages_announcer,
			"Downloading file before copying to clipboard...",
		);
	}
}

/// Opens the item list of the selected album row. False when the row isn't an album.
pub(crate) fn open_album_items(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) -> bool {
	let messages = {
		let s = state.borrow();
		let Some(idx) = parts.conversations_parts.messages_list.get_selection() else { return false };
		match s.display_items.get(idx as usize) {
			Some(whatsnap::DisplayItem::Album { messages, .. }) => {
				messages.iter().filter(|m| !m.is_deleted).cloned().collect::<Vec<_>>()
			}
			_ => return false,
		}
	};
	let total = messages.len();
	let labels: Vec<String> = messages
		.iter()
		.enumerate()
		.map(|(i, m)| {
			let kind = if m.media_type == "video" || m.text.starts_with("[Video]") { "Video" } else { "Photo" };
			let state_note = if m.is_media_downloaded() { "" } else { ", not downloaded" };
			format!("{} {} of {}{}", kind, i + 1, total, state_note)
		})
		.collect();
	let ids: Vec<String> = messages.iter().map(|m| m.id.clone()).collect();
	let downloaded: Vec<bool> = messages.iter().map(|m| m.is_media_downloaded()).collect();
	let (state_open, parts_open, ids_open) = (state.clone(), parts.clone(), ids.clone());
	let (state_save, parts_save) = (state.clone(), parts.clone());
	crate::ui::dialogs::album_items::show_album_items(
		&parts.frame,
		"Album",
		&labels,
		move |i| {
			if let Some(id) = ids_open.get(i) {
				if downloaded[i] {
					open_media_message(&state_open, &parts_open, id.clone());
				} else {
					download_media_message(&state_open, &parts_open, id.clone());
				}
			}
		},
		move |i| {
			if let Some(id) = ids.get(i) {
				save_media_message_as(&state_save, &parts_save, id.clone());
			}
		},
	);
	crate::platform::tray::enter_release_guard().set(true);
	true
}
