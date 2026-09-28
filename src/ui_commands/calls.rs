//! `handle_ui_command` arms for placing, answering and controlling WhatsApp calls
//! (`PlaceCall`, `AnswerCall`, `RejectCall`, `LeaveCall`, `MuteCall`, device switching).

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState, CallVideo,
	audio::{play_call_fail_sound, stop_call_ringing_sound},
	call_helpers::{announce_call, call_window_title, local_camera_announcement},
	call_video_send,
	ui::window::WindowParts,
};

/// Whether a chat can be called at all. Channels, broadcast lists and a
/// community itself (as opposed to its groups) have no calls in WhatsApp.
pub(crate) fn can_call(s: &AppState, jid: &str) -> bool {
	!jid.ends_with("@newsletter") && !jid.ends_with("@broadcast") && !s.community_jids.contains(jid)
}

fn place_call_impl(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, video: bool) {
	let jid = {
		let s = state.borrow();
		if let Some(j) = s.selected_chat.as_ref().filter(|j| can_call(&s, j)) {
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
		autostart_camera: video,
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
	// Shown by call_connected - WhatsApp only accepts an invite into an
	// established call, so offering it while ringing just fails.
	parts.call_parts.add_participant_btn.show(false);
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
		let msg = serde_json::json!({
			"action": "set_video",
			"data": { "call_id": call_id, "enabled": false }
		});
		let _ = s.ws_tx.send(msg.to_string());
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
	let device = s.settings.camera_device_name.clone();
	let ui_tx = s.ui_tx.clone();
	let send_state = call_video_send::start_video_send(port, device, ui_tx);
	s.active_call_video_send = Some(send_state);
	s.call_video.local_camera_on = true;
	let msg = serde_json::json!({
		"action": "set_video",
		"data": { "call_id": call_id, "enabled": true }
	});
	let _ = s.ws_tx.send(msg.to_string());
	parts.call_parts.camera_btn.set_label("Turn camera off");
	drop(s);
	announce_call(parts, local_camera_announcement(true));
}

/// The capture/encode thread couldn't open a camera at all - see
/// `call_video_send::start_video_send`'s doc comment for why this only
/// fires once, for the very first open attempt.
pub(crate) fn camera_failed(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, reason: String) {
	let mut s = state.borrow_mut();
	s.active_call_video_send = None;
	s.call_video.local_camera_on = false;
	if let Some(call_id) = s.active_call_id.clone() {
		let msg = serde_json::json!({
			"action": "set_video",
			"data": { "call_id": call_id, "enabled": false }
		});
		let _ = s.ws_tx.send(msg.to_string());
	}
	parts.call_parts.camera_btn.set_label("Turn camera on");
	drop(s);
	announce_call(parts, &format!("Couldn't start the camera: {reason}"));
}
