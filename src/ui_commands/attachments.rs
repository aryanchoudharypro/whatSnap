//! `handle_ui_command` arms for outgoing/incoming media attachments (add/paste/discard/
//! send, download/open/save, album clipboard copy).

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState,
	clipboard::{self, copy_files_to_clipboard},
	contact_picker::refresh_attach_contact_list,
	message_display::update_action_buttons,
	ui::window::WindowParts,
};

/// An attach-menu option: pick files the way that option does.
pub(crate) fn attach_files(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, kind: whantv::AttachKind) {
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

/// The file picker for the current batch's attach option (also "Add more
/// files" in the attachment dialog, which stays with that option).
pub(crate) fn add_attachment(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	if crate::ui_commands::blocking::stop_if_cant_send(state, parts) {
		return;
	}
	let kind = state.borrow().pending_attachment_kind;
	let message = match kind {
		whantv::AttachKind::Media => "Choose photos or videos to send",
		whantv::AttachKind::Audio => "Choose audio files to send",
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

/// How many files one batch can send, as in WhatsApp Web's attach picker
/// (its media_picker_select_limit).
const MAX_BATCH_FILES: usize = 30;

/// Adds files to the pending batch, up to MAX_BATCH_FILES; returns how many
/// didn't fit.
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
		crate::announce::announce(
			parts.conversations_parts.messages_announcer,
			&format!(
				"You can send up to {MAX_BATCH_FILES} files at once; {left_out} file{} weren't added.",
				if left_out == 1 { "" } else { "s" }
			),
		);
	}
}

/// Fills in and shows the attachment dialog for the pending batch. A new batch
/// starts with `caption` (or an empty one); adding files to a batch that's
/// already open keeps whatever caption has been typed.
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
		whantv::AttachKind::Document => "Documents",
		whantv::AttachKind::Media => "Photos and videos",
		whantv::AttachKind::Audio => "Audio",
		whantv::AttachKind::Pasted => "Files",
	};
	let d = &parts.attachment_dialog;
	d.attachment_info_label.set_label(&format!("{}: {}", heading, file_names.join(", ")));
	let captioned = kind.has_caption();
	if !d.dialog.is_shown() {
		d.caption_field.set_value(caption.as_deref().unwrap_or(""));
		d.caption_field.set_insertion_point_end();
		d.view_once_check.set_value(false);
	}
	// View once is for photos and videos, and not in a channel.
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
	crate::announce::announce(
		parts.conversations_parts.messages_announcer,
		&format!("{}: {}", announcement, file_names.len()),
	);
	if captioned {
		d.caption_field.set_focus();
	} else {
		d.send_btn.set_focus();
	}
	if !d.dialog.is_shown() {
		d.dialog.show_modal();
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
	d.dialog.show_modal();
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
	crate::announce::announce(parts.conversations_parts.messages_announcer, sending);
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
	parts.attach_contact_dialog.dialog.show_modal();
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
		crate::announce::announce(
			parts.conversations_parts.messages_announcer,
			&format!("Sending {} contact{}...", count, if count == 1 { "" } else { "s" }),
		);
	}
}

pub(crate) fn paste_attachment(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	if crate::ui_commands::blocking::stop_if_cant_send(state, parts) {
		return;
	}
	if let Some(_locker) = wxdragon::prelude::Clipboard::get().locker() {
		let fdo = wxdragon::prelude::FileDataObject::new();
		if wxdragon::prelude::Clipboard::get().get_data(&fdo) {
			let paths = fdo.get_files();
			if !paths.is_empty() {
				let mut s = state.borrow_mut();
				if s.pending_attachments.is_empty() {
					s.pending_attachment_kind = whantv::AttachKind::Pasted;
				}
				let left_out = add_to_batch(&mut s, paths);
				let kind = s.pending_attachment_kind;
				drop(s);
				announce_left_out(parts, left_out);
				// Text already typed in the message box becomes the caption,
				// as in WhatsApp, rather than being left behind in the box.
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
		}
	}
}

pub(crate) fn discard_attachment(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	{
		let mut s = state.borrow_mut();
		s.pending_attachments.clear();
		s.pending_attachment_kind = whantv::AttachKind::default();
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
		// The backend persists a "uploading" row and broadcasts it
		// immediately, before the upload even starts, so the local
		// echo comes back through the normal "message" event handler
		// with a real id from the start instead of a client-side
		// placeholder here.
		let caption = if kind.has_caption() { caption } else { String::new() };
		let view_once =
			parts.attachment_dialog.view_once_check.is_shown() && parts.attachment_dialog.view_once_check.get_value();
		for (idx, path) in paths.iter().enumerate() {
			let file_type = kind.send_type(path);
			let filename = std::path::Path::new(path).file_name().unwrap_or_default().to_string_lossy().to_string();

			// Only the first attachment in a multi-file batch carries the
			// reply-to context, so a reply doesn't get quoted once per file.
			let mut payload = serde_json::json!({
				"action": "send_attachment",
				"data": {
					"jid": jid,
					"type": file_type,
					"path": path,
					"filename": filename,
					"caption": caption,
					"reply_to_id": if idx == 0 { reply_id.as_str() } else { "" },
					"reply_to_sender": if idx == 0 { reply_sender.as_str() } else { "" },
					"view_once": view_once && matches!(file_type, "image" | "video")
				}
			});
			let ws_tx = state.borrow().ws_tx.clone();
			if file_type == "audio" {
				// An audio message carries its length, which means reading
				// the file - off the UI thread, it can be a long track.
				let path = path.clone();
				std::thread::spawn(move || {
					payload["data"]["duration"] = serde_json::json!(crate::audio::audio_file_seconds(&path));
					let _ = ws_tx.send(payload.to_string());
				});
			} else if file_type == "image" && path.to_lowercase().ends_with(".gif") {
				// A GIF goes as a looping video, the way WhatsApp sends one; if
				// it can't be converted it goes as a photo, as before.
				let path = path.clone();
				std::thread::spawn(move || {
					let mp4 = std::env::temp_dir().join(format!("whantv_gif_{}.mp4", crate::util::random_hex()));
					let mp4 = mp4.to_string_lossy().to_string();
					match crate::gif_video::gif_to_mp4(&path, &mp4) {
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
				// A video message carries a preview of its first frame; the
				// backend shrinks it to WhatsApp's preview size.
				let path = path.clone();
				std::thread::spawn(move || {
					if let Some(png) = crate::video_thumbnail::first_frame_png(&path) {
						payload["data"]["thumbnail"] = serde_json::json!(crate::util::base64(&png));
					}
					let _ = ws_tx.send(payload.to_string());
				});
			} else {
				let _ = ws_tx.send(payload.to_string());
			}
		}
		parts.attachment_dialog.attachment_info_label.set_label("");
		parts.attachment_dialog.caption_field.set_value("");
		parts.attachment_dialog.dialog.end_modal(wxdragon::id::ID_OK);
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

	crate::websocket_events::ui_timing_log(&format!(
		"Download pressed: {} to start, {} to cancel, chat {}",
		to_start.len(),
		to_cancel.len(),
		if jid_opt.is_some() { "open" } else { "none" }
	));
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
			crate::announce::announce(
				parts.conversations_parts.messages_announcer,
				&format!("Downloading {}", to_start[0].1),
			);
		} else {
			crate::announce::announce(
				parts.conversations_parts.messages_announcer,
				&format!("Downloading album of {} files...", count),
			);
		}
	}
}

pub(crate) fn open_media(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let s = state.borrow();
	if let (Some(jid), Some(idx)) = (s.selected_chat.clone(), parts.conversations_parts.messages_list.get_selection())
		&& let Some(item) = s.display_items.get(idx as usize)
		&& let Some(msg) = item.messages().iter().find(|m| m.is_media_downloaded())
	{
		let payload = serde_json::json!({
			"action": "open_media",
			"data": { "jid": jid, "message_id": msg.id }
		});
		let _ = s.ws_tx.send(payload.to_string());
	}
}

pub(crate) fn save_media_as(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let item_opt = {
		let s = state.borrow();
		let sel = parts.conversations_parts.messages_list.get_selection();
		sel.and_then(|idx| s.display_items.get(idx as usize).cloned())
	};

	if let Some(item) = item_opt {
		match item {
			whantv::DisplayItem::Single(msg) => {
				// Grab an owned copy and drop the borrow before opening the
				// modal dialog below — show_modal() pumps the message loop,
				// and any handler that needs state.borrow_mut() while we're
				// still holding this Ref would panic with a BorrowMutError.
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
							let _ = std::fs::copy(&msg.media_path, &path);
							crate::announce::announce(
								parts.conversations_parts.messages_announcer,
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
			whantv::DisplayItem::Album { messages, .. } => {
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
					crate::announce::announce(
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
		crate::announce::announce(parts.conversations_parts.messages_announcer, &format!("Downloading {}", filename));
	}
}

pub(crate) fn open_media_message(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>, message_id: String) {
	let s = state.borrow();
	if let Some(jid) = s.selected_chat.clone()
		&& s.messages.iter().any(|m| m.id == message_id && m.is_media_downloaded())
	{
		let payload = serde_json::json!({
			"action": "open_media",
			"data": { "jid": jid, "message_id": message_id }
		});
		let _ = s.ws_tx.send(payload.to_string());
	}
}

pub(crate) fn save_media_message_as(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, message_id: String) {
	// Grab an owned copy and drop the borrow before opening the modal
	// save dialog below - show_modal() pumps the message loop, and any
	// handler that needs state.borrow_mut() while we're still holding
	// this Ref would panic with a BorrowMutError.
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
				let _ = std::fs::copy(&msg.media_path, &path);
				crate::announce::announce(parts.conversations_parts.messages_announcer, "File saved successfully.");
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
			crate::announce::announce(parts.conversations_parts.messages_announcer, &ann_text);
		} else {
			crate::announce::announce(
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
		crate::announce::announce(
			parts.conversations_parts.messages_announcer,
			"Downloading file before copying to clipboard...",
		);
	}
}
