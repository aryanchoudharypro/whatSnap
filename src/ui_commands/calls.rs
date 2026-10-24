//! `handle_ui_command` arms for placing, answering and controlling WhatsApp calls
//! (`PlaceCall`, `AnswerCall`, `RejectCall`, `LeaveCall`, `MuteCall`, device switching).

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState, CallVideo,
	audio::{play_call_fail_sound, stop_call_ringing_sound},
	call_helpers::{
		SYSTEM_AUDIO_ANNOUNCEMENT, announce_call, call_window_title, local_camera_announcement,
		local_screen_share_announcement,
	},
	call_video_send::{self, VideoSource},
	ui::window::WindowParts,
};

/// Whether a chat can be called at all. Channels, broadcast lists and a
/// community itself (as opposed to its groups) have no calls in WhatsApp.
pub(crate) fn can_call(s: &AppState, jid: &str) -> bool {
	!jid.ends_with("@newsletter") && !jid.ends_with("@broadcast") && !s.community_jids.contains(jid)
}

/// A group call still going on that we're invited to but not in: one we
/// missed, declined or left (the backend's `call_joinable` list).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct JoinableCall {
	pub call_id: String,
	pub group_jid: String,
	/// Who started the call, for one with no group to name it by.
	pub creator_name: String,
	pub video: bool,
	pub connected: u64,
}

/// A Calls tab row stands for a call going on rather than a chat when its
/// jid starts with this, followed by the call ID.
pub(crate) const JOIN_ROW_PREFIX: &str = "join:";

/// The Calls tab's line for a group call going on, read before the
/// instruction to press Enter.
pub(crate) fn joinable_call_label(s: &AppState, call: &JoinableCall) -> String {
	let kind = if call.video { "Group video call" } else { "Group voice call" };
	let place = match s.contacts.get(&call.group_jid) {
		Some(name) if !call.group_jid.is_empty() => format!(" in {}", name),
		_ if !call.creator_name.is_empty() => format!(" started by {}", call.creator_name),
		_ => String::new(),
	};
	let people = match call.connected {
		0 => String::new(),
		1 => ", 1 person in it".to_string(),
		n => format!(", {} people in it", n),
	};
	format!("{}{}{}{}", kind, place, people, ONGOING_CALL_SUFFIX)
}

/// Added to a call-history row while its call is still going on.
pub(crate) const ONGOING_CALL_SUFFIX: &str = ". Going on now, press Enter to join";

pub(crate) fn joinable_call_for_chat<'a>(s: &'a AppState, jid: &str) -> Option<&'a JoinableCall> {
	s.joinable_calls.iter().find(|c| !c.group_jid.is_empty() && c.group_jid == jid)
}

/// Shows a chat's call buttons: "Join call" in place of Call and Video call
/// while one of its group calls is going on, like WhatsApp.
pub(crate) fn apply_call_buttons(s: &AppState, parts: &WindowParts, jid: &str) {
	let can_call = can_call(s, jid);
	let joinable = can_call && joinable_call_for_chat(s, jid).is_some();
	let c = &parts.conversations_parts;
	c.place_call_btn.show(can_call && !joinable);
	c.place_video_call_btn.show(can_call && !joinable);
	c.join_call_btn.show(joinable);
}

/// Marks the open chat's call-history rows whose call is still going on.
pub(crate) fn mark_joinable_rows(s: &AppState, parts: &WindowParts) {
	let Some(jid) = s.selected_chat.as_deref() else { return };
	let is_group = jid.ends_with("@g.us");
	let list = &parts.conversations_parts.messages_list;
	for (i, item) in s.display_items.iter().enumerate() {
		let whatsnap::DisplayItem::Single(msg) = item else { continue };
		let Some(call_id) = msg.id.strip_prefix("call:").filter(|_| msg.is_call()) else { continue };
		let mut text = item.format_display(is_group);
		if s.joinable_calls.iter().any(|c| c.call_id == call_id && c.group_jid == jid) {
			text.push_str(ONGOING_CALL_SUFFIX);
		}
		if list.get_string(i as u32).as_deref() != Some(text.as_str()) {
			list.set_string(i as u32, &text);
		}
	}
}

/// The joinable call a call-history row is for, if its call is still going on.
pub(crate) fn joinable_call_for_row(s: &AppState, msg: &whatsnap::Msg) -> Option<String> {
	let call_id = msg.id.strip_prefix("call:")?;
	s.joinable_calls.iter().any(|c| c.call_id == call_id).then(|| call_id.to_string())
}

/// The backend's `call_joinable` event: the full list of joinable calls.
pub(crate) fn on_joinable_calls(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let calls: Vec<JoinableCall> = data
		.get("calls")
		.and_then(|c| c.as_array())
		.map(|list| {
			list.iter()
				.map(|c| JoinableCall {
					call_id: c.get("call_id").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
					group_jid: c.get("group_jid").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
					creator_name: c.get("creator_name").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
					video: c.get("video").and_then(|v| v.as_bool()).unwrap_or(false),
					connected: c.get("connected").and_then(|v| v.as_u64()).unwrap_or(0),
				})
				.filter(|c| !c.call_id.is_empty())
				.collect()
		})
		.unwrap_or_default();
	let mut s = state.borrow_mut();
	if s.joinable_calls == calls {
		return;
	}
	s.joinable_calls = calls;
	if s.active_tab == 6 {
		crate::message_display::apply_chat_filter(&mut s, parts);
	}
	if let Some(jid) = s.selected_chat.clone() {
		apply_call_buttons(&s, parts, &jid);
		parts.conversations_parts.top_btns_panel.layout();
		mark_joinable_rows(&s, parts);
	}
}

/// "Join call": joins the open group's call that's going on, the way
/// joining a call link does - no ringing, straight in.
pub(crate) fn join_group_call(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, call_id: Option<String>) {
	let mut s = state.borrow_mut();
	let call = match call_id {
		Some(id) => s.joinable_calls.iter().find(|c| c.call_id == id).cloned(),
		None => s.selected_chat.as_deref().and_then(|jid| joinable_call_for_chat(&s, jid)).cloned(),
	};
	let Some(call) = call else {
		drop(s);
		announce_call(parts, "That call has ended.");
		return;
	};
	if s.active_call_id.is_some() || s.call_placing {
		drop(s);
		announce_call(parts, "You're already in a call.");
		return;
	}
	let msg = serde_json::json!({
		"action": "join_group_call",
		"data": { "call_id": call.call_id, "video": false }
	});
	let _ = s.ws_tx.send(msg.to_string());
	if !call.group_jid.is_empty() {
		s.selected_chat = Some(call.group_jid.clone());
	}
	s.call_placing = true;
	s.call_pending_cancel = false;
	s.call_video = CallVideo { video_call: call.video, ..Default::default() };
	let c = &parts.call_parts;
	c.frame.set_title(call_window_title(true, call.video));
	c.status_label.set_label("Joining call...");
	c.accept_btn.show(false);
	c.reject_btn.show(false);
	c.mute_btn.show(true);
	c.camera_btn.set_label("Turn camera on");
	c.camera_btn.show(false);
	c.add_participant_btn.show(false);
	crate::ui_commands::call_extras::hide_call_buttons(parts);
	c.leave_btn.show(true);
	c.frame.show(true);
	drop(s);
	announce_call(parts, "Joining call...");
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
	// A row for a call going on, or a group whose call is going on, joins
	// that call rather than starting a new one, as the chat's Join button does.
	let join = match jid.strip_prefix(JOIN_ROW_PREFIX) {
		Some(call_id) => Some(call_id.to_string()),
		None => joinable_call_for_chat(&state.borrow(), &jid).map(|c| c.call_id.clone()),
	};
	if join.is_some() {
		join_group_call(state, parts, join);
		return;
	}
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
		video_call: video,
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
	hide_screen_share_controls(parts);
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
	// and video is already on, so there's nothing to turn on. A dual-stream
	// share has its own stream and goes on.
	let was_sharing = s.call_video.local_screen_on && s.call_video.screen_send.is_none();
	start_camera(&mut s, port);
	if was_sharing {
		s.call_video.local_screen_on = false;
		s.call_video.camera_before_share = false;
		set_system_audio(&mut s, false);
		send_call_toggle(&s, "set_screen_share", &call_id, false);
		parts.call_parts.screen_btn.set_label("Share screen");
	} else {
		send_call_toggle(&s, "set_video", &call_id, true);
	}
	parts.call_parts.camera_btn.set_label("Turn camera off");
	// A voice call with a camera on is a video call now, so its screen can
	// be shared too.
	if !s.call_video.video_call {
		s.call_video.video_call = true;
		show_screen_share_controls(&s, parts);
	}
	drop(s);
	let message = if was_sharing {
		local_screen_share_announcement(false, None, true)
	} else {
		local_camera_announcement(true).to_string()
	};
	announce_call(parts, &message);
}

/// Tells the backend the call settings it acts on itself.
pub(crate) fn send_call_options(s: &AppState) {
	let msg = serde_json::json!({
		"action": "set_call_options",
		"data": { "dual_stream_screen_share": s.settings.dual_stream_screen_share }
	});
	let _ = s.ws_tx.send(msg.to_string());
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
	// Ctrl+Shift+S still reaches here on a voice call, whose button is hidden.
	if !s.call_video.video_call && !s.call_video.local_screen_on {
		drop(s);
		announce_call(parts, "Screen sharing is only on video calls. Turn your camera on to make this a video call.");
		return;
	}
	let Some(port) = s.call_video_port else {
		drop(s);
		announce_call(parts, "Screen sharing isn't ready yet - wait for the call to connect.");
		return;
	};

	if s.call_video.local_screen_on && s.call_video.screen_send.take().is_some() {
		// A dual-stream share: the camera never moved.
		s.call_video.local_screen_on = false;
		set_system_audio(&mut s, false);
		send_call_toggle(&s, "set_screen_share", &call_id, false);
		parts.call_parts.screen_btn.set_label("Share screen");
		drop(s);
		announce_call(parts, &local_screen_share_announcement(false, None, false));
		return;
	}
	if s.call_video.local_screen_on {
		let camera_returns = s.call_video.camera_before_share;
		s.call_video.local_screen_on = false;
		s.call_video.camera_before_share = false;
		set_system_audio(&mut s, false);
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
		announce_call(parts, &local_screen_share_announcement(false, None, camera_returns));
		return;
	}
	drop(s);

	// Asked every time, like WhatsApp's own share picker. The call can end,
	// or a share start some other way, while the question is up.
	let Some(choice) = crate::ui::screen_picker::pick_share_target(&parts.call_parts.frame) else { return };
	let Some(with_sound) = ask_share_sound(parts) else { return };
	let mut s = state.borrow_mut();
	if s.active_call_id.as_deref() != Some(call_id.as_str()) || s.call_video.local_screen_on {
		return;
	}
	let Some(port) = s.call_video_port else { return };

	let camera_was_on = s.call_video.local_camera_on;
	let ui_tx = s.ui_tx.clone();
	let source = VideoSource::Screen(choice.target);
	// With the camera on and the call offering the screen its own stream,
	// the screen goes there and the camera stays as it is.
	if let Some(screen_port) = s.call_video.screen_port.filter(|_| camera_was_on) {
		s.call_video.screen_send = Some(call_video_send::start_video_send(screen_port, source, None, ui_tx));
		s.call_video.local_screen_on = true;
		let msg = serde_json::json!({
			"action": "set_screen_share",
			"data": { "call_id": call_id, "enabled": true, "dual": true }
		});
		let _ = s.ws_tx.send(msg.to_string());
		parts.call_parts.screen_btn.set_label("Stop sharing screen");
		if with_sound {
			set_system_audio(&mut s, true);
		}
		drop(s);
		let shared = local_screen_share_announcement(true, choice.window_title.as_deref(), false);
		if with_sound {
			announce_call(parts, &format!("{shared} {}", SYSTEM_AUDIO_ANNOUNCEMENT));
		} else {
			announce_call(parts, &shared);
		}
		return;
	}
	s.active_call_video_send = Some(call_video_send::start_video_send(port, source, None, ui_tx));
	s.call_video.local_screen_on = true;
	s.call_video.local_camera_on = false;
	s.call_video.camera_before_share = camera_was_on;
	if !camera_was_on {
		send_call_toggle(&s, "set_video", &call_id, true);
	}
	send_call_toggle(&s, "set_screen_share", &call_id, true);
	parts.call_parts.screen_btn.set_label("Stop sharing screen");
	parts.call_parts.camera_btn.set_label("Turn camera on");
	if with_sound {
		set_system_audio(&mut s, true);
	}
	drop(s);
	let shared = local_screen_share_announcement(true, choice.window_title.as_deref(), camera_was_on);
	if with_sound {
		announce_call(parts, &format!("{shared} {}", SYSTEM_AUDIO_ANNOUNCEMENT));
	} else {
		announce_call(parts, &shared);
	}
}

/// Whether a screen share about to start carries the computer's sound too:
/// `Some(true)` for yes, `Some(false)` for the screen alone, `None` if the
/// share was cancelled. The screen alone is the default button.
fn ask_share_sound(parts: &WindowParts) -> Option<bool> {
	let dialog = MessageDialog::builder(
		&parts.call_parts.frame,
		"Share your computer's sound along with your screen? The others will hear everything it plays, \
		 your screen reader included, while your microphone is on.",
		"Share screen",
	)
	.with_style(
		MessageDialogStyle::YesNo
			| MessageDialogStyle::Cancel
			| MessageDialogStyle::NoDefault
			| MessageDialogStyle::IconQuestion,
	)
	.build();
	dialog.set_yes_no_labels("Share screen and sound", "Share screen only");
	match dialog.show_modal() {
		id if id == wxdragon::id::ID_YES => Some(true),
		id if id == wxdragon::id::ID_NO => Some(false),
		_ => None,
	}
}

/// Starts or stops mixing the computer's sound into the call, when the call's
/// audio is running.
pub(crate) fn set_system_audio(s: &mut AppState, on: bool) {
	let ui_tx = s.ui_tx.clone();
	if let Some(audio) = s.active_call_audio.as_mut() {
		audio.share_system_audio(on, &ui_tx);
	}
}

/// Shows the screen share button on a video call, once it has connected; a
/// voice call has no video to share the screen over.
pub(crate) fn show_screen_share_controls(s: &AppState, parts: &WindowParts) {
	let c = &parts.call_parts;
	let shown = s.call_video.video_call;
	c.screen_btn.set_label(if s.call_video.local_screen_on { "Stop sharing screen" } else { "Share screen" });
	c.screen_btn.show(shown);
	c.frame.layout();
}

pub(crate) fn hide_screen_share_controls(parts: &WindowParts) {
	let c = &parts.call_parts;
	c.screen_btn.set_label("Share screen");
	c.screen_btn.show(false);
}

/// The computer's sound couldn't be captured; the screen share itself goes on.
pub(crate) fn system_audio_failed(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, reason: String) {
	let mut s = state.borrow_mut();
	if !s.active_call_audio.as_ref().is_some_and(|audio| audio.sharing_system_audio()) {
		return;
	}
	set_system_audio(&mut s, false);
	drop(s);
	announce_call(parts, &format!("Couldn't share your computer's sound: {reason}"));
}

/// The capture thread couldn't read the screen at all, so the share never
/// started - the screen counterpart of `camera_failed`.
pub(crate) fn screen_share_failed(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, reason: String) {
	let mut s = state.borrow_mut();
	if !s.call_video.local_screen_on {
		return;
	}
	if s.call_video.screen_send.take().is_some() {
		// A dual-stream share: only the screen's own stream stops.
		s.call_video.local_screen_on = false;
		set_system_audio(&mut s, false);
		if let Some(call_id) = s.active_call_id.clone() {
			send_call_toggle(&s, "set_screen_share", &call_id, false);
		}
		parts.call_parts.screen_btn.set_label("Share screen");
		drop(s);
		announce_call(parts, &format!("Couldn't share the screen: {reason}"));
		return;
	}
	s.active_call_video_send = None;
	s.call_video.local_screen_on = false;
	s.call_video.camera_before_share = false;
	set_system_audio(&mut s, false);
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
	if s.call_video.local_screen_on && s.call_video.screen_send.is_none() {
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

const MENU_JOIN_CALL: i32 = 4505;

/// The context menu of a Calls tab row for a group call going on.
fn show_join_row_menu(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, call_id: String) {
	let group_jid = {
		let s = state.borrow();
		let Some(call) = s.joinable_calls.iter().find(|c| c.call_id == call_id) else { return };
		call.group_jid.clone()
	};
	let mut builder = Menu::builder().append_item(MENU_JOIN_CALL, "&Join call", "Join this group call");
	if !group_jid.is_empty() {
		builder = builder.append_item(MENU_OPEN_CHAT, "Open c&hat", "Open the group this call is in");
	}
	let mut menu = builder.build();
	let state_menu = state.clone();
	let parts_menu = parts.clone();
	menu.on_selected(move |evt| match evt.get_id() {
		MENU_JOIN_CALL => join_group_call(&state_menu, &parts_menu, Some(call_id.clone())),
		MENU_OPEN_CHAT => crate::chat_ui::open_chat_for_jid(&group_jid, &state_menu, &parts_menu),
		_ => {}
	});
	parts.conversations_parts.conversations_list.popup_menu(&mut menu, None);
	menu.destroy_menu();
	crate::tray::enter_release_guard().set(true);
}

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
	if let Some(call_id) = jid.strip_prefix(JOIN_ROW_PREFIX) {
		show_join_row_menu(state, parts, call_id.to_string());
		return;
	}

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
