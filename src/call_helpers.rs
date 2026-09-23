//! Voice/video call helpers: message send-status ranking, the group-call
//! roster (parsing, status text, join/leave/decline announcements), and the
//! call window's mic/speaker device pickers.

use wxdragon::prelude::*;

use crate::{AppState, CallParticipant, ui::window::WindowParts};

pub(crate) fn get_status_rank(s: &str) -> i32 {
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

pub(crate) fn parse_call_participants(data: &serde_json::Value) -> Vec<CallParticipant> {
	let Some(list) = data.get("participants").and_then(|p| p.as_array()) else {
		return Vec::new();
	};
	list.iter()
		.filter_map(|p| {
			let jid = p.get("jid").and_then(|v| v.as_str())?.to_string();
			let name = p
				.get("name")
				.and_then(|v| v.as_str())
				.filter(|n| !n.is_empty())
				.map(|n| n.to_string())
				.unwrap_or_else(|| crate::contact_picker::display_name_for_jid(&jid));
			let state = p.get("state").and_then(|v| v.as_str()).unwrap_or("").to_string();
			Some(CallParticipant { jid, name, state })
		})
		.collect()
}

pub(crate) fn join_names(names: &[&str]) -> String {
	match names {
		[] => String::new(),
		[one] => one.to_string(),
		[rest @ .., last] => format!("{} and {}", rest.join(", "), last),
	}
}

/// Status line for the call window during a group call, e.g.
/// "In group call with Asha and Ben. Ringing Chris."
pub(crate) fn group_call_status_text(participants: &[CallParticipant]) -> String {
	let connected: Vec<&str> =
		participants.iter().filter(|p| p.state == "connected").map(|p| p.name.as_str()).collect();
	let ringing: Vec<&str> = participants
		.iter()
		.filter(|p| p.state != "connected" && p.state != "rejected")
		.map(|p| p.name.as_str())
		.collect();
	match (connected.is_empty(), ringing.is_empty()) {
		(true, true) => "In group call. No one else is here yet.".to_string(),
		(true, false) => format!("Calling {}.", join_names(&ringing)),
		(false, true) => format!("In group call with {}.", join_names(&connected)),
		(false, false) => {
			format!("In group call with {}. Ringing {}.", join_names(&connected), join_names(&ringing))
		}
	}
}

/// What changed between two group-call rosters, as screen-reader
/// announcements ("Asha joined the call.", "Ben left the call.", ...).
pub(crate) fn call_participant_changes(old: &[CallParticipant], new: &[CallParticipant]) -> Vec<String> {
	let mut changes = Vec::new();
	for p in new {
		let before = old.iter().find(|o| o.jid == p.jid).map(|o| o.state.as_str());
		if p.state == "connected" && before != Some("connected") {
			changes.push(format!("{} joined the call.", p.name));
		} else if p.state != "connected" && before == Some("connected") {
			changes.push(format!("{} left the call.", p.name));
		} else if p.state == "rejected" && before != Some("rejected") {
			changes.push(format!("{} declined the call.", p.name));
		}
	}
	for o in old {
		if o.state == "connected" && !new.iter().any(|p| p.jid == o.jid) {
			changes.push(format!("{} left the call.", o.name));
		}
	}
	changes
}

/// Re-enumerates devices (same reasoning as Settings' combos: WASAPI device
/// lists can change while the app is running) into the call window's
/// "video call from Asha", "group voice call from Asha in Family" - the
/// Announce something about the call to whichever window can actually be heard.
///
/// The call frame is a separate top-level window, and a screen reader only
/// speaks a live-region change belonging to the FOREGROUND window. While the
/// call window is up it owns focus, so routing call announcements through the
/// main window's message announcer reaches nobody - which is why camera
/// toggles and "Call connected." were silent. Fall back to the main announcer
/// once the call window is gone, so late teardown messages still land.
pub(crate) fn announce_call(parts: &WindowParts, message: &str) {
	// Calls interrupt whatever the screen reader is saying; everything else
	// waits its turn (see announce.rs).
	let label = if parts.call_parts.frame.is_shown() {
		parts.call_parts.announcer.clone()
	} else {
		parts.conversations_parts.messages_announcer.clone()
	};
	live_region::announce_with_priority(label, message, live_region::Priority::High);
}

/// incoming-call label, announcement and toast all use this.
pub(crate) fn incoming_call_description(
	is_group: bool,
	is_video: bool,
	caller: &str,
	group_name: Option<&str>,
) -> String {
	let kind = if is_video { "video call" } else { "voice call" };
	match (is_group, group_name) {
		(true, Some(group)) => format!("group {} from {} in {}", kind, caller, group),
		(true, None) => format!("group {} from {}", kind, caller),
		(false, _) => format!("{} from {}", kind, caller),
	}
}

/// Title of the call window, which NVDA reads when it takes focus.
pub(crate) fn call_window_title(is_group: bool, is_video: bool) -> &'static str {
	match (is_group, is_video) {
		(false, false) => "Voice call",
		(false, true) => "Video call",
		(true, false) => "Group voice call",
		(true, true) => "Group video call",
	}
}

/// What to announce for a `call_video_state` event. `peer` is the other
/// person's name on a 1:1 call and None on a group call (the stanza doesn't
/// say who sent it). `was_on` is their last known camera state, so a
/// repeated state isn't announced twice.
///
/// "upgrade_requested" means the PEER wants to add video so THEY can send
/// us their camera - unrelated to whether we're sending ours. Receiving and
/// rendering incoming video is a separate, later piece of work (see
/// [[video_calls_work_status]]), so this app never calls `AcceptVideo()` and
/// just says so.
pub(crate) fn peer_video_announcement(event: &str, was_on: Option<bool>, peer: Option<&str>) -> Option<String> {
	let who = peer.unwrap_or("Someone");
	match event {
		"on" if was_on != Some(true) => Some(format!("{} turned their camera on.", who)),
		"off" if was_on != Some(false) => Some(format!("{} turned their camera off.", who)),
		"upgrade_requested" => {
			Some(format!("{} wants to switch to video, but this app can't show incoming video yet.", who))
		}
		_ => None,
	}
}

/// What to announce when OUR OWN camera changes state, for the toggle button
/// (Ctrl+Shift+V) and the menu item.
pub(crate) fn local_camera_announcement(on: bool) -> &'static str {
	if on { "Camera on." } else { "Camera off." }
}

/// mic/speaker pickers and shows them, once a call has actually connected.
pub(crate) fn show_call_device_combos(s: &AppState, parts: &WindowParts) {
	use crate::ui::settings_dialog::set_device_combo;
	set_device_combo(
		&parts.call_parts.input_device_combo,
		&crate::audio_devices::input_device_names(),
		s.settings.input_device_name.as_deref(),
	);
	set_device_combo(
		&parts.call_parts.output_device_combo,
		&crate::audio_devices::output_device_names(),
		s.settings.output_device_name.as_deref(),
	);
	set_device_combo(
		&parts.call_parts.camera_device_combo,
		&crate::video_devices::camera_names(),
		s.settings.camera_device_name.as_deref(),
	);
	parts.call_parts.devices_panel.show(true);
}

#[cfg(test)]
mod call_participant_tests {
	use super::*;

	fn p(jid: &str, name: &str, state: &str) -> CallParticipant {
		CallParticipant { jid: jid.into(), name: name.into(), state: state.into() }
	}

	#[test]
	fn status_text_lists_connected_and_ringing() {
		let roster = [p("1@lid", "Asha", "connected"), p("2@lid", "Ben", "connected"), p("3@lid", "Chris", "ringing")];
		assert_eq!(group_call_status_text(&roster), "In group call with Asha and Ben. Ringing Chris.");
		let roster = [p("1@lid", "Asha", "outgoing"), p("2@lid", "Ben", "outgoing"), p("3@lid", "Chris", "outgoing")];
		assert_eq!(group_call_status_text(&roster), "Calling Asha, Ben and Chris.");
	}

	#[test]
	fn changes_announce_join_leave_and_decline() {
		let old = [
			p("1@lid", "Asha", "outgoing"),
			p("2@lid", "Ben", "ringing"),
			p("3@lid", "Chris", "connected"),
			p("4@lid", "Dev", "connected"),
		];
		let new = [p("1@lid", "Asha", "connected"), p("2@lid", "Ben", "rejected"), p("4@lid", "Dev", "rejected")];
		assert_eq!(
			call_participant_changes(&old, &new),
			vec!["Asha joined the call.", "Ben declined the call.", "Dev left the call.", "Chris left the call."]
		);
		assert!(call_participant_changes(&new, &new).is_empty());
	}

	#[test]
	fn incoming_description_names_call_kind() {
		assert_eq!(incoming_call_description(false, false, "Asha", None), "voice call from Asha");
		assert_eq!(incoming_call_description(false, true, "Asha", None), "video call from Asha");
		assert_eq!(
			incoming_call_description(true, true, "Asha", Some("Family")),
			"group video call from Asha in Family"
		);
		assert_eq!(incoming_call_description(true, false, "Asha", None), "group voice call from Asha");
		assert_eq!(call_window_title(false, true), "Video call");
		assert_eq!(call_window_title(true, false), "Group voice call");
	}

	#[test]
	fn peer_video_announces_changes_only() {
		assert_eq!(
			peer_video_announcement("on", Some(false), Some("Asha")).as_deref(),
			Some("Asha turned their camera on.")
		);
		assert_eq!(peer_video_announcement("on", Some(true), Some("Asha")), None);
		assert_eq!(peer_video_announcement("off", None, None).as_deref(), Some("Someone turned their camera off."));
		assert_eq!(peer_video_announcement("off", Some(false), Some("Asha")), None);
		assert_eq!(
			peer_video_announcement("upgrade_requested", Some(false), Some("Asha")).as_deref(),
			Some("Asha wants to switch to video, but this app can't show incoming video yet.")
		);
		assert_eq!(peer_video_announcement("bogus", None, Some("Asha")), None);
	}

	#[test]
	fn local_camera_announces_on_and_off() {
		assert_eq!(local_camera_announcement(true), "Camera on.");
		assert_eq!(local_camera_announcement(false), "Camera off.");
	}
}
