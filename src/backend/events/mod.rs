mod account;
mod calls;
mod chats;
mod conversation;
mod messages;
mod people;
mod session;
mod status;

use std::{borrow::Cow, cell::RefCell, rc::Rc};

use chats::*;
use conversation::*;
use rodio::{Player, Source};
use rodio_wsola::WsolaSourceExt;
use serde::Deserialize;
use whatsnap::ChatMessage;
use wxdragon::prelude::*;

use crate::{
	AppState, CallLogEntry, CallTarget, CallVideo, Contact, NotificationSound, StarredEntry, StatusGroup, StatusItem,
	UiCommand,
	audio::{
		ensure_audio_stream, play_call_fail_sound, play_notification_sound, start_call_ringback,
		stop_call_ringing_sound,
	},
	calls::helpers::{
		announce_call, call_participant_changes, get_status_rank, group_call_status_text, incoming_call_description,
		parse_call_participants, peer_video_announcement, show_call_device_combos,
	},
	platform::tray::{install_tray_icon, refresh_tray_tooltip},
	ui::{
		chat_ui::show_toast_notification,
		contact_picker::{
			apply_contact_picker_filter, display_name_for_jid, refresh_selection_list, sync_add_members_contact_labels,
			sync_attach_contact_labels, sync_new_group_contact_labels,
		},
		message_display::{
			InfoMediaLists, apply_chat_filter, fill_info_media, refresh_voice_action_button, update_action_buttons,
			update_loaded_message, update_loaded_messages,
		},
		window::WindowParts,
	},
};

pub(crate) fn format_conv_data_label(
	chat_jid: &str,
	chat_name: &str,
	sender_name: &str,
	is_typing: bool,
	is_recording: bool,
	presence_label: Option<&str>,
) -> String {
	if is_recording || is_typing {
		let is_group = chat_jid.ends_with("@g.us");
		if is_group {
			let status = if is_recording {
				format!("{} is recording audio...", sender_name)
			} else {
				format!("{} is typing...", sender_name)
			};
			format!("{}, {}", chat_name, status)
		} else {
			let status = if is_recording { "recording audio..." } else { "typing..." };
			format!("{} ({})", chat_name, status)
		}
	} else {
		presence_label.filter(|l| !l.is_empty()).map(|l| l.to_string()).unwrap_or_else(|| chat_name.to_string())
	}
}

/// A message from a "starred_messages" or "search_results" event, as the
/// Starred tab and the chat list's search results show it: which chat, who
/// sent it, what it says and when.
pub(crate) fn message_hit(s: &AppState, item: &serde_json::Value) -> Option<crate::MessageHit> {
	fn text_of<'a>(item: &'a serde_json::Value, key: &str) -> &'a str {
		item.get(key).and_then(|v| v.as_str()).unwrap_or("")
	}
	let (jid, message_id) = (text_of(item, "jid"), text_of(item, "id"));
	if jid.is_empty() || message_id.is_empty() {
		return None;
	}
	let from_me = item.get("fromMe").and_then(|f| f.as_bool()).unwrap_or(false);
	let (sender_name, text, quoted_text, media_type) =
		(text_of(item, "senderName"), text_of(item, "text"), text_of(item, "quotedText"), text_of(item, "mediaType"));
	let timestamp = item.get("timestamp").and_then(|t| t.as_i64()).unwrap_or(0);

	let chat_name = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.to_string());
	let sender = if from_me {
		"You".to_string()
	} else if !sender_name.is_empty() {
		sender_name.to_string()
	} else {
		chat_name.clone()
	};
	let body = if !text.is_empty() {
		whatsnap::friendly_preview_text(text)
	} else if !quoted_text.is_empty() {
		whatsnap::friendly_preview_text(quoted_text)
	} else if !media_type.is_empty() {
		match media_type {
			"image" => "Photo",
			"video" => "Video",
			"ptt" => "Voice message",
			"audio" => "Audio",
			"document" => "Document",
			"sticker" => "Sticker",
			"contact" => "Contact",
			"location" | "live_location" => "Location",
			"poll" | "quiz" => "Poll",
			_ => "Media",
		}
		.to_string()
	} else {
		"Message".to_string()
	};
	let date = if timestamp > 0 {
		whatsnap::LocalTime::from_unix(timestamp)
			.map(|local_dt| whatsnap::day_name(local_dt, "%b %e, %Y"))
			.unwrap_or_default()
	} else {
		String::new()
	};
	Some(crate::MessageHit { jid: jid.to_string(), message_id: message_id.to_string(), chat_name, sender, body, date })
}

pub(crate) fn call_log(parts: &WindowParts, state: &AppState, event: &str, call_id: &str) {
	ui_timing_log(&format!(
		"Call: {} {} (active call {:?}, placing {}, window shown {})",
		event,
		call_id,
		state.active_call_id,
		state.call_placing,
		parts.call_parts.frame.is_shown()
	));
}

fn refresh_open_contact_info(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let s = state.borrow();
	let Some(jid) = s.info_chat.as_deref() else { return };
	if parts.conversation_data_dialog.dialog.is_shown() {
		let payload = serde_json::json!({ "action": "get_user_info", "data": { "jid": jid } });
		let _ = s.ws_tx.send(payload.to_string());
	}
}

pub(crate) fn qr_rgba(qr: &qrcode::QrCode) -> (u32, u32, Vec<u8>) {
	const MODULE: usize = 5;
	const QUIET: usize = 4;
	let modules = qr.width();
	let colors = qr.to_colors();
	let side = (modules + 2 * QUIET) * MODULE;
	let mut rgba = vec![255u8; side * side * 4];
	for (i, color) in colors.iter().enumerate() {
		if *color != qrcode::Color::Dark {
			continue;
		}
		let (mx, my) = (i % modules + QUIET, i / modules + QUIET);
		for y in my * MODULE..(my + 1) * MODULE {
			for x in mx * MODULE..(mx + 1) * MODULE {
				rgba[(y * side + x) * 4..][..3].fill(0);
			}
		}
	}
	(side as u32, side as u32, rgba)
}

pub(crate) fn ui_timing_log(msg: &str) {
	use std::io::Write;
	let now = whatsnap::LocalTime::now().format("%Y/%m/%d %H:%M:%S");
	if let Ok(mut f) =
		std::fs::OpenOptions::new().create(true).append(true).open(crate::platform::paths::log_dir().join("ui.log"))
	{
		let _ = writeln!(f, "{now} {msg}");
	}
}

pub(crate) struct SlowUiLog<'a> {
	what: &'a str,
	started: std::time::Instant,
}

impl<'a> SlowUiLog<'a> {
	pub(crate) fn new(what: &'a str) -> Self {
		Self { what, started: std::time::Instant::now() }
	}
}

impl Drop for SlowUiLog<'_> {
	fn drop(&mut self) {
		let took = self.started.elapsed();
		if took >= std::time::Duration::from_millis(500) {
			ui_timing_log(&format!("Slow: {} held the UI for {} ms", self.what, took.as_millis()));
		}
	}
}

pub(crate) fn process_websocket_message(raw: String, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let path = fast_path(&raw);
	if path == FastPath::Chats {
		match serde_json::from_str::<ChatsEnvelope>(&raw) {
			Ok(ChatsEnvelope { data: Some(chats) }) => apply_chats(chats, state, parts),
			Ok(ChatsEnvelope { data: None }) => crate::ui_commands::links::open_pending_whatsapp_link(state),
			Err(e) => eprintln!("[LOG UI] Failed to parse a chats payload: {}", e),
		}
		return;
	}
	if path == FastPath::ConversationMessages {
		let parse_started = std::time::Instant::now();
		let parsed = parse_conversation_messages(&raw);
		ui_timing_log(&format!(
			"Parsed a {} KB conversation_messages payload in {} ms",
			raw.len() / 1024,
			parse_started.elapsed().as_millis()
		));
		drop(raw);
		match parsed {
			Some(page) => {
				if state.borrow().selected_chat.as_deref() == Some(page.jid.as_str()) {
					if page.load_newer {
						apply_newer_messages(page, state, parts);
						return;
					}
					if let Some(id) = page.target_id.clone() {
						state.borrow_mut().search_target_id = Some(id);
					}
					if !page.target_found && !state.borrow().display_items.is_empty() {
						// A quoted message we don't have: stay put, as before.
						let mut s = state.borrow_mut();
						s.search_target_id = None;
						s.search_target_text = None;
						unsafe {
							let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
								windows::Win32::UI::WindowsAndMessaging::MB_ICONASTERISK,
							);
						}
						return;
					}
					if !page.load_more {
						let mut s = state.borrow_mut();
						s.newer_loading = false;
						s.messages_at_latest = page.is_latest;
					}
					apply_conversation_messages(page, state, parts);
				}
			}
			None => eprintln!("[LOG UI] Failed to parse a conversation_messages payload"),
		}
		return;
	}

	if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw)
		&& let Some(t) = val.get("type").and_then(|v| v.as_str())
	{
		let _slow = SlowUiLog::new(t);
		let _ = session::handle(t, &val, state, parts)
			|| calls::handle(t, &val, state, parts)
			|| messages::handle(t, &val, state, parts)
			|| people::handle(t, &val, state, parts)
			|| status::handle(t, &val, state, parts)
			|| account::handle(t, &val, state, parts);
	}
}

pub(crate) fn message_row_key(item: &whatsnap::DisplayItem, text: &str) -> String {
	match item.id() {
		"" => format!("\u{1}{text}"),
		id => id.to_string(),
	}
}

#[cfg(test)]
mod conversation_messages_tests {
	use super::*;

	#[test]
	fn chats_take_their_own_fast_path() {
		assert_eq!(fast_path(r#"{"type":"chats","data":[]}"#), FastPath::Chats);
		assert_eq!(fast_path(r#"{"type":"chat_cleared","data":{}}"#), FastPath::Other);
	}

	#[test]
	fn a_chats_payload_parses_with_escapes_and_missing_fields() {
		let raw = r#"{"type":"chats","data":[
			{"JID":"a@s.whatsapp.net","FullName":"A \"quoted\" name","UnreadCount":3,"Pinned":true,"Timestamp":5},
			{"JID":"g@g.us","IsCommunity":true,"ChannelRole":""}
		]}"#;
		let chats = serde_json::from_str::<ChatsEnvelope>(raw).unwrap().data.unwrap();
		assert_eq!(chats.len(), 2);
		assert_eq!(chats[0].full_name.as_deref(), Some("A \"quoted\" name"));
		assert_eq!(chats[0].unread_count, 3);
		assert!(chats[0].pinned && !chats[0].muted);
		assert_eq!(chats[1].full_name, None);
		assert!(chats[1].is_community);
	}

	#[test]
	fn a_chats_payload_without_data_has_none() {
		assert!(serde_json::from_str::<ChatsEnvelope>(r#"{"type":"chats"}"#).unwrap().data.is_none());
	}

	#[test]
	fn only_conversation_messages_takes_the_typed_fast_path() {
		assert!(is_conversation_messages(r#"{"type":"conversation_messages","data":{}}"#));
		assert!(is_conversation_messages(r#"{"data":{"messages":[{"id":"a"}]},"type":"conversation_messages"}"#));
		assert!(!is_conversation_messages(r#"{"type":"message","data":{}}"#));
		assert!(!is_conversation_messages(r#"{"data":{}}"#));
		assert!(!is_conversation_messages("not json at all"));
	}

	#[test]
	fn parses_the_jid_and_every_message() {
		let raw = r#"{"type":"conversation_messages","data":{"jid":"a@s.whatsapp.net","messages":[
			{"id":"m1","text":"hi","timestamp":10,"fromMe":true,"mediaType":"image","albumId":"alb1"},
			{"id":"m2","text":"yo","timestamp":11,"sender_jid":"b@s.whatsapp.net"}
		]}}"#;
		let MessagesPage { jid, load_more, messages: msgs, .. } =
			parse_conversation_messages(raw).expect("envelope should parse");
		assert_eq!(jid, "a@s.whatsapp.net");
		assert!(!load_more);
		assert_eq!(msgs.len(), 2);
		assert_eq!(msgs[0].id, "m1");
		assert!(msgs[0].from_me);
		assert_eq!(msgs[0].media_type, "image");
		assert_eq!(msgs[0].album_id, "alb1");
		assert_eq!(msgs[1].id, "m2");
		assert_eq!(msgs[1].sender_jid, "b@s.whatsapp.net");
	}

	#[test]
	fn parses_the_load_more_flag() {
		let raw = r#"{"type":"conversation_messages","data":{"jid":"a","loadMore":true,"messages":[]}}"#;
		let page = parse_conversation_messages(raw).expect("envelope should parse");
		assert!(page.load_more);
		assert_eq!((page.older, page.newer), (None, None));
	}

	#[test]
	fn parses_the_counts_around_a_page() {
		let raw = r#"{"type":"conversation_messages","data":{"jid":"a","older":27500,"newer":3,"messages":[]}}"#;
		let page = parse_conversation_messages(raw).expect("envelope should parse");
		assert_eq!((page.older, page.newer), (Some(27500), Some(3)));
	}

	#[test]
	fn parses_the_load_newer_flag() {
		let raw =
			r#"{"type":"conversation_messages","data":{"jid":"a","loadNewer":true,"isLatest":false,"messages":[]}}"#;
		let page = parse_conversation_messages(raw).expect("envelope should parse");
		assert!(!page.load_more);
		assert!(page.load_newer);
		assert!(!page.is_latest);
	}

	#[test]
	fn a_single_bad_message_becomes_a_placeholder_instead_of_failing_the_chat() {
		let raw = r#"{"type":"conversation_messages","data":{"jid":"a@s.whatsapp.net","messages":[
			{"id":"m1","text":"before","timestamp":10},
			{"id":"m2","text":"broken"},
			{"id":"m3","text":"after","timestamp":12}
		]}}"#;
		let msgs = parse_conversation_messages(raw).expect("one bad row must not fail the envelope").messages;
		assert_eq!(msgs.len(), 3);
		assert_eq!(msgs[0].text, "before");
		assert!(msgs[1].text.starts_with("Parse error:"), "got {:?}", msgs[1].text);
		assert_eq!(msgs[1].id, "");
		assert_eq!(msgs[2].text, "after");
	}

	#[test]
	fn an_empty_or_absent_message_list_parses_to_nothing() {
		let MessagesPage { jid, messages: msgs, .. } =
			parse_conversation_messages(r#"{"type":"conversation_messages","data":{"jid":"a","messages":[]}}"#)
				.unwrap();
		assert_eq!(jid, "a");
		assert!(msgs.is_empty());

		let MessagesPage { jid, messages: msgs, .. } =
			parse_conversation_messages(r#"{"type":"conversation_messages","data":{"jid":"a"}}"#).unwrap();
		assert_eq!(jid, "a");
		assert!(msgs.is_empty());
	}

	#[test]
	fn a_malformed_envelope_is_rejected_rather_than_panicking() {
		assert!(parse_conversation_messages(r#"{"type":"conversation_messages"}"#).is_none());
		assert!(parse_conversation_messages("{").is_none());
	}

	#[test]
	fn direct_chat_typing_replaces_presence_label() {
		let label_online = format_conv_data_label(
			"919876543210@s.whatsapp.net",
			"Aman Kumar",
			"Aman Kumar",
			true,
			false,
			Some("Aman Kumar (online)"),
		);
		assert_eq!(label_online, "Aman Kumar (typing...)");

		let label_last_seen = format_conv_data_label(
			"919876543210@s.whatsapp.net",
			"Aman Kumar",
			"Aman Kumar",
			true,
			false,
			Some("Aman Kumar (last seen today at 6:19 AM)"),
		);
		assert_eq!(label_last_seen, "Aman Kumar (typing...)");
	}

	#[test]
	fn direct_chat_recording_replaces_presence_label() {
		let label = format_conv_data_label(
			"919876543210@s.whatsapp.net",
			"Aman Kumar",
			"Aman Kumar",
			false,
			true,
			Some("Aman Kumar (online)"),
		);
		assert_eq!(label, "Aman Kumar (recording audio...)");
	}

	#[test]
	fn direct_chat_idle_uses_presence_label() {
		let label_online = format_conv_data_label(
			"919876543210@s.whatsapp.net",
			"Aman Kumar",
			"Aman Kumar",
			false,
			false,
			Some("Aman Kumar (online)"),
		);
		assert_eq!(label_online, "Aman Kumar (online)");

		let label_last_seen = format_conv_data_label(
			"919876543210@s.whatsapp.net",
			"Aman Kumar",
			"Aman Kumar",
			false,
			false,
			Some("Aman Kumar (last seen today at 6:19 AM)"),
		);
		assert_eq!(label_last_seen, "Aman Kumar (last seen today at 6:19 AM)");

		let label_no_presence =
			format_conv_data_label("919876543210@s.whatsapp.net", "Aman Kumar", "Aman Kumar", false, false, None);
		assert_eq!(label_no_presence, "Aman Kumar");
	}

	#[test]
	fn group_chat_typing_appends_sender() {
		let label = format_conv_data_label("123456789@g.us", "Family Group", "Aman Kumar", true, false, None);
		assert_eq!(label, "Family Group, Aman Kumar is typing...");
	}

	#[test]
	fn group_chat_recording_appends_sender() {
		let label = format_conv_data_label("123456789@g.us", "Family Group", "Aman Kumar", false, true, None);
		assert_eq!(label, "Family Group, Aman Kumar is recording audio...");
	}

	#[test]
	fn group_chat_idle_shows_group_name() {
		let label = format_conv_data_label("123456789@g.us", "Family Group", "", false, false, None);
		assert_eq!(label, "Family Group");
	}
}
