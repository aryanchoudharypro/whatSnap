//! `handle_ui_command` arms for placing, answering and controlling WhatsApp calls
//! (`PlaceCall`, `AnswerCall`, `RejectCall`, `LeaveCall`, `MuteCall`, device switching).

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState,
	audio::{play_call_fail_sound, stop_call_ringing_sound},
	ui::window::WindowParts,
};

pub(crate) fn place_call(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let jid = {
		let s = state.borrow();
		if let Some(j) = s.selected_chat.as_ref() {
			j.clone()
		} else {
			return;
		}
	};
	let msg = serde_json::json!({
		"action": "place_call",
		"data": { "jid": jid }
	});
	let mut s = state.borrow_mut();
	let _ = s.ws_tx.send(msg.to_string());
	s.call_placing = true;
	s.call_pending_cancel = false;

	let is_group = jid.ends_with("@g.us");
	let name = s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
	if is_group {
		parts.call_parts.status_label.set_label(&format!("Starting group call in {}", name));
	} else {
		parts.call_parts.status_label.set_label(&format!("In call with {}", name));
	}
	parts.call_parts.accept_btn.show(false);
	parts.call_parts.reject_btn.show(false);
	parts.call_parts.mute_btn.show(true);
	// Shown by call_connected - WhatsApp only accepts an invite into an
	// established call, so offering it while ringing just fails.
	parts.call_parts.add_participant_btn.show(false);
	parts.call_parts.leave_btn.show(true);
	parts.call_parts.frame.show(true);
	drop(s);

	live_region::announce(
		parts.conversations_parts.messages_announcer.clone(),
		if is_group { "Placing group call..." } else { "Placing call..." },
	);
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
	if let Some(call_id) = &s.active_call_id {
		let msg = serde_json::json!({
			"action": "reject_call",
			"data": { "call_id": call_id }
		});
		let _ = s.ws_tx.send(msg.to_string());
		parts.call_parts.frame.show(false);
		s.active_call_id = None;
		s.call_participants.clear();
		s.call_answer_deadline = None;
		s.incoming_call_caller = None;
		stop_call_ringing_sound(&mut s);
		play_call_fail_sound(&mut s);
	}
}

pub(crate) fn leave_call(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let mut s = state.borrow_mut();
	if let Some(call_id) = &s.active_call_id {
		let msg = serde_json::json!({
			"action": "leave_call",
			"data": { "call_id": call_id }
		});
		let _ = s.ws_tx.send(msg.to_string());
		parts.call_parts.frame.show(false);
		s.active_call_id = None;
		s.active_call_audio = None;
		s.call_participants.clear();
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
		live_region::announce(parts.conversations_parts.messages_announcer.clone(), status);
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
	live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Microphone changed.");
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
	live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Speaker changed.");
}
