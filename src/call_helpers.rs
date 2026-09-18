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
}
