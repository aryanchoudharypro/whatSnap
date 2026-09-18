//! `handle_ui_command` arms for outgoing/incoming media attachments (add/paste/discard/
//! send, download/open/save, album clipboard copy).

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState,
	clipboard::{self, copy_files_to_clipboard},
	message_display::update_action_buttons,
	ui::window::WindowParts,
};

pub(crate) fn add_attachment(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
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

pub(crate) fn paste_attachment(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
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
					.map(|p| std::path::Path::new(p).file_name().unwrap_or_default().to_string_lossy().to_string())
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

pub(crate) fn discard_attachment(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	{
		let mut s = state.borrow_mut();
		s.pending_attachments.clear();
	}
	parts.attachment_dialog.attachment_info_label.set_label("");
	parts.attachment_dialog.caption_field.set_value("");
	parts.attachment_dialog.dialog.end_modal(wxdragon::id::ID_CANCEL);
}

pub(crate) fn send_attachment(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, caption: String) {
	let jid = state.borrow().selected_chat.clone();
	let (paths, reply_id, reply_sender) = {
		let mut s = state.borrow_mut();
		let paths = std::mem::take(&mut s.pending_attachments);
		let reply_id = s.pending_reply_id.take().unwrap_or_default();
		let reply_sender = s.pending_reply_sender.take().unwrap_or_default();
		s.pending_reply_preview = None;
		(paths, reply_id, reply_sender)
	};
	parts.conversations_parts.remove_quote_btn.show(false);
	if let Some(ref jid) = jid {
		// The backend persists a "uploading" row and broadcasts it
		// immediately, before the upload even starts, so the local
		// echo comes back through the normal "message" event handler
		// with a real id from the start instead of a client-side
		// placeholder here.
		for (idx, path) in paths.iter().enumerate() {
			let ext = std::path::Path::new(path).extension().unwrap_or_default().to_string_lossy().to_lowercase();
			let file_type = match ext.as_str() {
				"jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" => "image",
				"mp4" | "avi" | "mov" | "mkv" | "webm" | "3gp" => "video",
				_ => "document",
			};
			let filename = std::path::Path::new(path).file_name().unwrap_or_default().to_string_lossy().to_string();

			// Only the first attachment in a multi-file batch carries the
			// reply-to context, so a reply doesn't get quoted once per file.
			let payload = serde_json::json!({
				"action": "send_attachment",
				"data": {
					"jid": jid,
					"type": file_type,
					"path": path,
					"filename": filename,
					"caption": caption,
					"reply_to_id": if idx == 0 { reply_id.as_str() } else { "" },
					"reply_to_sender": if idx == 0 { reply_sender.as_str() } else { "" }
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
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

pub(crate) fn open_media(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let s = state.borrow();
	if let (Some(jid), Some(idx)) = (s.selected_chat.clone(), parts.conversations_parts.messages_list.get_selection()) {
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
		live_region::announce(
			parts.conversations_parts.messages_announcer.clone(),
			&format!("Downloading {}", filename),
		);
	}
}

pub(crate) fn open_media_message(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>, message_id: String) {
	let s = state.borrow();
	if let Some(jid) = s.selected_chat.clone() {
		if s.messages.iter().any(|m| m.id == message_id && m.is_media_downloaded()) {
			let payload = serde_json::json!({
				"action": "open_media",
				"data": { "jid": jid, "message_id": message_id }
			});
			let _ = s.ws_tx.send(payload.to_string());
		}
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
			if let Some(msg) = s.messages.iter().find(|m| m.id == *id) {
				if msg.is_media_downloaded()
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
