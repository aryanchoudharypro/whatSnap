//! The attach menu's "GIF": search Giphy, as WA Web's GIF panel does, with
//! the user's own API key, and send the one picked as a looping video. What
//! the backend answers: `gifs`, `gif_ready` and `gifs_failed`.

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{AppState, ui::window::WindowParts};

fn announce(parts: &WindowParts, message: &str) {
	crate::announce::announce(parts.conversations_parts.messages_announcer.clone(), message);
}

fn text(data: &serde_json::Value, key: &str) -> String {
	data.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

/// Attach menu > "GIF": the key first if there's none yet, then what to
/// search for. An empty search shows what's trending.
pub(crate) fn open_gif_search(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	if crate::ui_commands::blocking::stop_if_cant_send(state, parts) || state.borrow().selected_chat.is_none() {
		return;
	}
	if state.borrow().settings.giphy_api_key.trim().is_empty() && !ask_key(state, parts) {
		return;
	}
	let dialog =
		TextEntryDialog::builder(&parts.frame, "Search GIFs (leave empty for trending):", "GIFs, Powered by GIPHY")
			.build();
	if dialog.show_modal() != wxdragon::id::ID_OK {
		return;
	}
	search(state, parts, &dialog.get_value().unwrap_or_default());
}

/// Asks for a Giphy API key and keeps it in the settings.
fn ask_key(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) -> bool {
	let dialog = TextEntryDialog::builder(
		&parts.frame,
		"GIF search uses Giphy, which needs an API key. You can get one free at developers.giphy.com.\n\nGiphy API key:",
		"GIF Search",
	)
	.build();
	if dialog.show_modal() != wxdragon::id::ID_OK {
		return false;
	}
	let key = dialog.get_value().unwrap_or_default().trim().to_string();
	if key.is_empty() {
		return false;
	}
	let mut s = state.borrow_mut();
	s.settings.giphy_api_key = key;
	s.settings.save();
	true
}

fn search(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, query: &str) {
	let s = state.borrow();
	let payload = serde_json::json!({ "action": "search_gifs", "data": { "key": s.settings.giphy_api_key, "query": query.trim() } });
	let _ = s.ws_tx.send(payload.to_string());
	drop(s);
	announce(parts, "Searching GIFs...");
}

pub(crate) fn on_event(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, t: &str, data: &serde_json::Value) {
	match t {
		"gifs" => pick(state, parts, data),
		"gif_ready" => send_gif(state, parts, text(data, "jid"), text(data, "path")),
		"gifs_failed" => {
			let message = text(data, "message");
			// A key Giphy refuses is asked for again next time.
			if message.contains("API key") {
				let mut s = state.borrow_mut();
				s.settings.giphy_api_key.clear();
				s.settings.save();
			}
			announce(parts, &message);
		}
		_ => {}
	}
}

/// The results: pick one to send, or search again.
fn pick(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let query = text(data, "query");
	let results = data.get("results").and_then(|r| r.as_array()).cloned().unwrap_or_default();
	let mut labels = vec!["Search again...".to_string()];
	for (i, r) in results.iter().enumerate() {
		let title = text(r, "title");
		labels.push(if title.is_empty() { format!("GIF {}", i + 1) } else { title });
	}
	let heading = match (results.is_empty(), query.is_empty()) {
		(true, _) => format!("No GIFs found for {}.", query),
		(false, true) => "Trending GIFs:".to_string(),
		(false, false) => format!("GIFs for {}:", query),
	};
	let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
	let dialog = SingleChoiceDialog::builder(&parts.frame, &heading, "GIFs, Powered by GIPHY", &refs).build();
	if dialog.show_modal() != wxdragon::id::ID_OK {
		return;
	}
	let Ok(picked) = usize::try_from(dialog.get_selection()) else { return };
	if picked == 0 {
		open_gif_search(state, parts);
		return;
	}
	let Some(gif) = results.get(picked - 1) else { return };
	let Some(jid) = state.borrow().selected_chat.clone() else { return };
	let payload = serde_json::json!({ "action": "fetch_gif", "data": {
		"url": text(gif, "mp4"), "id": text(gif, "id"), "title": text(gif, "title"), "jid": jid,
	}});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	announce(parts, "Sending GIF...");
}

/// Sends the downloaded GIF as the attach dialog sends a GIF file: a video
/// with gif playback and its first frame as the preview.
/// The reply waiting in the composer goes with it if that chat is still open.
fn send_gif(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String, path: String) {
	if jid.is_empty() || path.is_empty() {
		return;
	}
	let (reply_id, reply_sender, ws_tx) = {
		let mut s = state.borrow_mut();
		let (reply_id, reply_sender) = if s.selected_chat.as_deref() == Some(jid.as_str()) {
			s.pending_reply_preview = None;
			parts.conversations_parts.remove_quote_btn.show(false);
			(s.pending_reply_id.take().unwrap_or_default(), s.pending_reply_sender.take().unwrap_or_default())
		} else {
			Default::default()
		};
		(reply_id, reply_sender, s.ws_tx.clone())
	};
	let filename = std::path::Path::new(&path).file_name().unwrap_or_default().to_string_lossy().to_string();
	let mut payload = serde_json::json!({ "action": "send_attachment", "data": {
		"jid": jid, "type": "video", "path": path, "filename": filename, "caption": "",
		"reply_to_id": reply_id, "reply_to_sender": reply_sender, "gif_playback": true,
	}});
	std::thread::spawn(move || {
		if let Some(png) = crate::video_thumbnail::first_frame_png(&path) {
			use base64::Engine;
			payload["data"]["thumbnail"] = serde_json::json!(base64::engine::general_purpose::STANDARD.encode(png));
		}
		let _ = ws_tx.send(payload.to_string());
	});
}
