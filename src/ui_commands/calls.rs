//! `handle_ui_command` arms for placing, answering and controlling WhatsApp calls
//! (`PlaceCall`, `AnswerCall`, `RejectCall`, `LeaveCall`, `MuteCall`, device switching).

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState, CallVideo,
	audio::{play_call_fail_sound, stop_call_ringing_sound},
	call_helpers::{announce_call, call_window_title, local_camera_announcement, local_screen_share_announcement},
	call_video_send::{self, VideoSource},
	ui::window::WindowParts,
};

/// Whether a chat can be called at all. Channels, broadcast lists and a
/// community itself (as opposed to its groups) have no calls in WhatsApp.
pub(crate) fn can_call(s: &AppState, jid: &str) -> bool {
	!jid.ends_with("@newsletter") && !jid.ends_with("@broadcast") && !s.community_jids.contains(jid)
}

fn place_call_impl(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, video: bool) {
	if crate::ui_commands::blocking::stop_if_blocked(state, parts) {
		return;
	}
	let jid = {
		let s = state.borrow();
		if s.active_tab == 6 {
			let search_val = parts.conversations_parts.search_field.get_value().trim().to_string();
			let digits: String = search_val.chars().filter(|c| c.is_ascii_digit()).collect();
			let is_phone = !digits.is_empty() && search_val.chars().all(|c| c.is_ascii_digit() || "+- ()".contains(c));
			if is_phone && digits.len() >= 3 {
				format!("{}@s.whatsapp.net", digits)
			} else {
				// A heading row has an empty jid and is never dialed.
				let row = |i: usize| s.filtered_chats.get(i).filter(|j| !j.is_empty()).cloned();
				let sel = parts.conversations_parts.conversations_list.get_selection().map(|i| i as usize);
				if let Some(target) = sel.and_then(&row) {
					target
				} else if !search_val.is_empty() {
					// Enter after typing a name dials the best match. With nothing
					// typed and nothing selected there is no one to call.
					match (0..s.filtered_chats.len()).find_map(&row) {
						Some(target) => target,
						None => return,
					}
				} else {
					return;
				}
			}
		} else if let Some(j) = s.selected_chat.as_ref().filter(|j| can_call(&s, j)) {
			j.clone()
		} else {
			return;
		}
	};
	let msg = serde_json::json!({
		"action": "place_call",
		"data": { "jid": jid, "video": video }
	});
	let mut s = state.borrow_mut();
	s.selected_chat = Some(jid.clone());
	let _ = s.ws_tx.send(msg.to_string());
	s.call_placing = true;
	s.call_pending_cancel = false;

	let is_group = jid.ends_with("@g.us");
	let name = s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
	// Group calls have no video option on the backend (see handlePlaceCall) -
	// always placed as voice and, if a video group call was requested, only
	// upgraded to video once connected via the same autostart_camera path a
	// mid-call toggle uses, not declared video at signaling time.
	let signals_video = video && !is_group;
	s.call_video = CallVideo {
		is_video: signals_video,
		peer_name: if is_group { None } else { Some(name.clone()) },
		peer_camera_on: Some(false),
		local_camera_on: false,
		local_screen_on: false,
		camera_before_share: false,
		autostart_camera: video,
		..Default::default()
	};
	parts.call_parts.frame.set_title(call_window_title(is_group, video));
	if is_group {
		parts.call_parts.status_label.set_label(&format!("Starting group call in {}", name));
	} else {
		parts.call_parts.status_label.set_label(&format!("In call with {}", name));
	}
	parts.call_parts.accept_btn.show(false);
	parts.call_parts.reject_btn.show(false);
	parts.call_parts.mute_btn.show(true);
	parts.call_parts.camera_btn.set_label("Turn camera on");
	// Shown by call_connected - it needs the video bridge port from
	// call_connected before the button can do anything, same reasoning as
	// add_participant_btn below.
	parts.call_parts.camera_btn.show(false);
	parts.call_parts.screen_btn.set_label("Share screen");
	parts.call_parts.screen_btn.show(false);
	// Shown by call_connected - WhatsApp only accepts an invite into an
	// established call, so offering it while ringing just fails.
	parts.call_parts.add_participant_btn.show(false);
	crate::ui_commands::call_extras::hide_call_buttons(parts);
	parts.call_parts.leave_btn.show(true);
	parts.call_parts.frame.show(true);
	drop(s);

	let placing_msg = match (is_group, video) {
		(true, _) => "Placing group call...",
		(false, true) => "Placing video call...",
		(false, false) => "Placing call...",
	};
	announce_call(parts, placing_msg);
}

pub(crate) fn place_call(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	place_call_impl(state, parts, false);
}

pub(crate) fn place_video_call(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	place_call_impl(state, parts, true);
}

pub(crate) fn answer_call(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let mut s = state.borrow_mut();
	if let Some(call_id) = &s.active_call_id {
		let msg = serde_json::json!({
			"action": "answer_call",
			"data": { "call_id": call_id }
		});
		let _ = s.ws_tx.send(msg.to_string());
		stop_call_ringing_sound(&mut s);
		s.call_answer_deadline = Some(std::time::Instant::now());
		parts.call_parts.status_label.set_label("Answering...");
	}
}

/// Alt+Shift+Y / Alt+Shift+R. Acts only while the incoming call is still
/// ringing: the keys can fire a moment after the call was answered or ended.
pub(crate) fn call_hotkey(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	hotkey: crate::call_hotkeys::CallHotkey,
) {
	let ringing = {
		let s = state.borrow();
		s.active_call_id.is_some() && s.call_answer_deadline.is_none()
	} && parts.call_parts.accept_btn.is_shown();
	if !ringing {
		return;
	}
	match hotkey {
		crate::call_hotkeys::CallHotkey::Answer => answer_call(state, parts),
		crate::call_hotkeys::CallHotkey::Reject => reject_call(state, parts),
	}
}

pub(crate) fn reject_call(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let mut s = state.borrow_mut();
	crate::websocket_events::call_log(parts, &s, "reject pressed", "");
	if let Some(call_id) = &s.active_call_id {
		let msg = serde_json::json!({
			"action": "reject_call",
			"data": { "call_id": call_id }
		});
		let _ = s.ws_tx.send(msg.to_string());
		parts.call_parts.frame.show(false);
		s.active_call_id = None;
		s.active_call_video_send = None;
		s.call_video_port = None;
		s.call_participants.clear();
		s.call_video = CallVideo::default();
		s.call_answer_deadline = None;
		s.incoming_call_caller = None;
		stop_call_ringing_sound(&mut s);
		play_call_fail_sound(&mut s);
	}
}

pub(crate) fn leave_call(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let mut s = state.borrow_mut();
	crate::websocket_events::call_log(parts, &s, "leave pressed", "");
	if let Some(call_id) = &s.active_call_id {
		let msg = serde_json::json!({
			"action": "leave_call",
			"data": { "call_id": call_id }
		});
		let _ = s.ws_tx.send(msg.to_string());
		parts.call_parts.frame.show(false);
		s.active_call_id = None;
		s.active_call_audio = None;
		s.active_call_video_send = None;
		s.call_video_port = None;
		s.call_participants.clear();
		s.call_video = CallVideo::default();
		s.call_answer_deadline = None;
		stop_call_ringing_sound(&mut s);
		play_call_fail_sound(&mut s);
	} else if s.call_placing {
		// We're in the window between sending place_call and hearing back
		// call_placed/call_failed with a real call_id - there's nothing to
		// send leave_call for yet. Hide the UI now for immediate feedback,
		// and mark the cancel as pending so the call_placed handler hangs
		// up as soon as the call_id actually arrives instead of leaving it
		// ringing on the peer's end.
		s.call_pending_cancel = true;
		parts.call_parts.frame.show(false);
		stop_call_ringing_sound(&mut s);
	}
}

pub(crate) fn mute_call(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let s = state.borrow();
	if let Some(audio) = &s.active_call_audio {
		let current = audio.is_muted.load(std::sync::atomic::Ordering::Relaxed);
		audio.is_muted.store(!current, std::sync::atomic::Ordering::Relaxed);
		let status = if !current { "Microphone muted." } else { "Microphone unmuted." };
		parts.call_parts.mute_btn.set_label(if !current { "Unmute microphone" } else { "Mute microphone" });
		announce_call(parts, status);
		// The local silence above is what actually mutes; this tells the other
		// side, whose call screen then shows us muted.
		if let Some(call_id) = &s.active_call_id {
			let payload = serde_json::json!({
				"action": "mute_call",
				"data": { "call_id": call_id, "muted": !current }
			});
			let _ = s.ws_tx.send(payload.to_string());
		}
	}
}

pub(crate) fn set_call_input_device(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, name: Option<String>) {
	let mut s = state.borrow_mut();
	if let Some(audio) = &s.active_call_audio {
		audio.set_input_device(name.clone());
	}
	// Remembered as the regular setting too, so the next call starts
	// on the device just picked mid-call instead of reverting.
	s.settings.input_device_name = name;
	s.settings.save();
	announce_call(parts, "Microphone changed.");
}

pub(crate) fn set_call_output_device(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, name: Option<String>) {
	let mut s = state.borrow_mut();
	if let Some(audio) = &s.active_call_audio {
		audio.set_output_device(name.clone());
	}
	if name != s.settings.output_device_name {
		s.settings.output_device_name = name;
		// Same cache-invalidation as the Settings dialog's Save
		// handler: drop the cached voice-message/notification
		// playback stream so it reopens on the newly selected
		// device next time something plays.
		s.audio_stream = None;
	}
	s.settings.save();
	announce_call(parts, "Speaker changed.");
}

pub(crate) fn set_call_camera_device(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, name: Option<String>) {
	let mut s = state.borrow_mut();
	if let Some(video_send) = &s.active_call_video_send {
		video_send.set_device(name.clone());
	}
	s.settings.camera_device_name = name;
	s.settings.save();
	announce_call(parts, "Camera changed.");
}

/// Turns the local camera on or off - the button, Ctrl+Shift+V, and the
/// auto-start after `PlaceVideoCall` connects (see `call_video.autostart_camera`).
pub(crate) fn toggle_camera(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let mut s = state.borrow_mut();
	let Some(call_id) = s.active_call_id.clone() else { return };

	if s.call_video.local_camera_on {
		s.active_call_video_send = None;
		s.call_video.local_camera_on = false;
		send_call_toggle(&s, "set_video", &call_id, false);
		parts.call_parts.camera_btn.set_label("Turn camera on");
		drop(s);
		announce_call(parts, local_camera_announcement(false));
		return;
	}

	let Some(port) = s.call_video_port else {
		drop(s);
		announce_call(parts, "The camera isn't ready yet - wait for the call to connect.");
		return;
	};
	// The call's one video stream goes back to the camera: the share ends,
	// and video is already on, so there's nothing to turn on.
	let was_sharing = s.call_video.local_screen_on;
	start_camera(&mut s, port);
	if was_sharing {
		s.call_video.local_screen_on = false;
		s.call_video.camera_before_share = false;
		send_call_toggle(&s, "set_screen_share", &call_id, false);
		parts.call_parts.screen_btn.set_label("Share screen");
	} else {
		send_call_toggle(&s, "set_video", &call_id, true);
	}
	parts.call_parts.camera_btn.set_label("Turn camera off");
	drop(s);
	announce_call(
		parts,
		if was_sharing { local_screen_share_announcement(false, true) } else { local_camera_announcement(true) },
	);
}

/// Sends one of the backend's `{call_id, enabled}` call commands.
fn send_call_toggle(s: &AppState, action: &str, call_id: &str, enabled: bool) {
	let msg = serde_json::json!({
		"action": action,
		"data": { "call_id": call_id, "enabled": enabled }
	});
	let _ = s.ws_tx.send(msg.to_string());
}

/// Points the call's video stream at the camera, replacing whatever session
/// was sending before.
fn start_camera(s: &mut AppState, port: u16) {
	let device = s.settings.camera_device_name.clone();
	let ui_tx = s.ui_tx.clone();
	s.active_call_video_send = Some(call_video_send::start_video_send(port, VideoSource::Camera, device, ui_tx));
	s.call_video.local_camera_on = true;
}

/// Starts or stops sharing the screen - the button and Ctrl+Shift+S. The
/// screen takes over the call's video stream from the camera, which comes
/// back on afterwards if it was on before.
pub(crate) fn toggle_screen_share(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let mut s = state.borrow_mut();
	let Some(call_id) = s.active_call_id.clone() else { return };
	let Some(port) = s.call_video_port else {
		drop(s);
		announce_call(parts, "Screen sharing isn't ready yet - wait for the call to connect.");
		return;
	};

	if s.call_video.local_screen_on {
		let camera_returns = s.call_video.camera_before_share;
		s.call_video.local_screen_on = false;
		s.call_video.camera_before_share = false;
		send_call_toggle(&s, "set_screen_share", &call_id, false);
		if camera_returns {
			start_camera(&mut s, port);
			parts.call_parts.camera_btn.set_label("Turn camera off");
		} else {
			s.active_call_video_send = None;
			send_call_toggle(&s, "set_video", &call_id, false);
		}
		parts.call_parts.screen_btn.set_label("Share screen");
		drop(s);
		announce_call(parts, local_screen_share_announcement(false, camera_returns));
		return;
	}

	let camera_was_on = s.call_video.local_camera_on;
	let ui_tx = s.ui_tx.clone();
	s.active_call_video_send = Some(call_video_send::start_video_send(port, VideoSource::Screen, None, ui_tx));
	s.call_video.local_screen_on = true;
	s.call_video.local_camera_on = false;
	s.call_video.camera_before_share = camera_was_on;
	if !camera_was_on {
		send_call_toggle(&s, "set_video", &call_id, true);
	}
	send_call_toggle(&s, "set_screen_share", &call_id, true);
	parts.call_parts.screen_btn.set_label("Stop sharing screen");
	parts.call_parts.camera_btn.set_label("Turn camera on");
	drop(s);
	announce_call(parts, local_screen_share_announcement(true, camera_was_on));
}

/// The capture thread couldn't read the screen at all, so the share never
/// started - the screen counterpart of `camera_failed`.
pub(crate) fn screen_share_failed(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, reason: String) {
	let mut s = state.borrow_mut();
	if !s.call_video.local_screen_on {
		return;
	}
	s.active_call_video_send = None;
	s.call_video.local_screen_on = false;
	s.call_video.camera_before_share = false;
	if let Some(call_id) = s.active_call_id.clone() {
		send_call_toggle(&s, "set_screen_share", &call_id, false);
		send_call_toggle(&s, "set_video", &call_id, false);
	}
	parts.call_parts.screen_btn.set_label("Share screen");
	drop(s);
	announce_call(parts, &format!("Couldn't share the screen: {reason}"));
}

/// The screen being shared is reaching the others as a black picture, or has
/// stopped doing so. Said out loud because nothing else would tell someone
/// using a screen reader's screen curtain that the curtain is all anyone sees.
pub(crate) fn screen_share_black(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, black: bool) {
	if !state.borrow().call_video.local_screen_on {
		return;
	}
	announce_call(
		parts,
		if black {
			"Your screen is being shared as a black picture. If your screen reader's screen curtain is on, turn it off so the others can see your screen."
		} else {
			"Your screen is visible to the others now."
		},
	);
}

/// The capture/encode thread couldn't open a camera at all - see
/// `call_video_send::start_video_send`'s doc comment for why this only
/// fires once, for the very first open attempt.
pub(crate) fn camera_failed(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, reason: String) {
	let mut s = state.borrow_mut();
	// A camera session that was already replaced by a screen share reports
	// nothing worth acting on.
	if s.call_video.local_screen_on {
		return;
	}
	s.active_call_video_send = None;
	s.call_video.local_camera_on = false;
	if let Some(call_id) = s.active_call_id.clone() {
		send_call_toggle(&s, "set_video", &call_id, false);
	}
	parts.call_parts.camera_btn.set_label("Turn camera on");
	drop(s);
	announce_call(parts, &format!("Couldn't start the camera: {reason}"));
}

const MENU_CALL: i32 = 4501;
const MENU_VIDEO_CALL: i32 = 4502;
const MENU_OPEN_CHAT: i32 = 4503;
const MENU_DELETE_CONTACT: i32 = 4504;

/// The Calls tab's context menu. Its rows are calls and contacts, so it offers
/// call actions rather than the chat list's (pin, archive, mark as unread...).
pub(crate) fn show_calls_menu(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, idx: usize) {
	let (jid, saved_contact) = {
		let s = state.borrow();
		let Some(jid) = s.filtered_chats.get(idx).filter(|j| !j.is_empty()).cloned() else { return };
		// Only a saved person can be deleted: not a group, and not someone known
		// only from a call or a chat.
		let saved_contact = s.call_contacts.iter().any(|c| c.jid == jid && !c.name.is_empty() && c.name != c.phone);
		(jid, saved_contact)
	};
	// The list may not have the row selected yet (the key opens the menu on
	// the focused row); the call actions dial the selection.
	parts.conversations_parts.conversations_list.set_selection(idx as u32, true);

	let mut builder = Menu::builder()
		.append_item(MENU_CALL, "&Call", "Place a voice call")
		.append_item(MENU_VIDEO_CALL, "&Video call", "Place a video call")
		.append_item(MENU_OPEN_CHAT, "Open c&hat", "Open the conversation with this contact");
	if saved_contact && !jid.ends_with("@g.us") {
		builder = builder.append_item(
			MENU_DELETE_CONTACT,
			"Delete &contact...",
			"Delete this contact from your contacts and your phone's",
		);
	}
	let mut menu = builder.build();

	let state_menu = state.clone();
	let parts_menu = parts.clone();
	menu.on_selected(move |evt| match evt.get_id() {
		MENU_CALL => place_call(&state_menu, &parts_menu),
		MENU_VIDEO_CALL => place_video_call(&state_menu, &parts_menu),
		MENU_OPEN_CHAT => crate::chat_ui::open_chat_for_jid(&jid, &state_menu, &parts_menu),
		MENU_DELETE_CONTACT => {
			let name = {
				let s = state_menu.borrow();
				s.call_contacts.iter().find(|c| c.jid == jid).map(|c| c.name.clone()).unwrap_or_else(|| jid.clone())
			};
			let confirm = MessageDialog::builder(
				&parts_menu.frame,
				&format!("Delete {} from your contacts? They'll be removed from your phone's contacts too.", name),
				"Delete Contact",
			)
			.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
			.build();
			confirm.set_yes_no_labels("Delete", "Cancel");
			if confirm.show_modal() == wxdragon::id::ID_YES {
				let payload = serde_json::json!({ "action": "delete_contact", "data": { "jid": jid } });
				let _ = state_menu.borrow().ws_tx.send(payload.to_string());
				crate::announce::announce(parts_menu.conversations_parts.messages_announcer, "Deleting contact...");
			}
		}
		_ => {}
	});
	parts.conversations_parts.conversations_list.popup_menu(&mut menu, None);
	menu.destroy_menu();
	crate::tray::enter_release_guard().set(true);
}
