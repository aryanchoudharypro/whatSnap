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
				.unwrap_or_else(|| crate::ui::contact_picker::display_name_for_jid(&jid));
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

pub(crate) fn is_ringing_state(state: &str) -> bool {
	matches!(state, "outgoing" | "receipt" | "ringing")
}

pub(crate) fn group_call_status_text(participants: &[CallParticipant]) -> String {
	let connected: Vec<&str> =
		participants.iter().filter(|p| p.state == "connected").map(|p| p.name.as_str()).collect();
	let ringing: Vec<&str> =
		participants.iter().filter(|p| is_ringing_state(&p.state)).map(|p| p.name.as_str()).collect();
	match (connected.is_empty(), ringing.is_empty()) {
		(true, true) => "In group call. No one else is here yet.".to_string(),
		(true, false) => format!("Calling {}.", join_names(&ringing)),
		(false, true) => format!("In group call with {}.", join_names(&connected)),
		(false, false) => {
			format!("In group call with {}. Ringing {}.", join_names(&connected), join_names(&ringing))
		}
	}
}

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
		} else if p.state == "invited" && before.is_some_and(is_ringing_state) {
			changes.push(format!("{} didn't answer.", p.name));
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
pub(crate) fn announce_call(parts: &WindowParts, message: &str, priority: live_region::Priority) {
	// Calls interrupt whatever the screen reader is saying; everything else
	// waits its turn (see platform/announce.rs).
	let label = if parts.call_parts.frame.is_shown() {
		parts.call_parts.announcer
	} else {
		parts.conversations_parts.messages_announcer
	};
	live_region::announce_with_priority(label, message, priority);
}

pub(crate) fn format_call_duration(seconds: u64) -> String {
	if seconds >= 3600 {
		format!("{}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)
	} else {
		format!("{}:{:02}", seconds / 60, seconds % 60)
	}
}

pub(crate) fn call_running_info_text(
	is_incoming: bool,
	is_video: bool,
	peer_name: &str,
	duration_secs: u64,
	is_reconnecting: bool,
) -> String {
	let direction = if is_incoming { "Incoming" } else { "Outgoing" };
	let kind = if is_video { " video call " } else { " call " };
	let prep = if is_incoming { "from" } else { "to" };
	let clock = format_call_duration(duration_secs);
	let line2 = if is_reconnecting { format!("Reconnecting... ({clock})") } else { clock };
	format!("{direction}{kind}{prep} {peer_name}\r\n{line2}")
}

pub(crate) fn call_incoming_info_text(
	peer_name: &str,
	phone_digits: Option<&str>,
	group: Option<&str>,
	is_video: bool,
) -> String {
	let kind = if is_video { "video call" } else { "call" };
	let line1 = if group.is_some() {
		format!("Incoming group {kind} from {peer_name}")
	} else {
		format!("Incoming {kind} from {peer_name}")
	};
	let line2 = if let Some(digits) = phone_digits {
		whatsnap::phone::format_international(digits).unwrap_or_else(|| format!("+{digits}"))
	} else if let Some(grp) = group {
		format!("in {grp}")
	} else {
		"Ringing...".to_string()
	};
	format!("{line1}\r\n{line2}")
}

/// The call window's details for a call we're placing, laid out like
/// WhatsApp's call screen: who (name, then their number when the name isn't
/// just the number), then where the call is - "Calling..." until their phone
/// gets it, "Ringing...", then "Connecting..." once they answer.
pub(crate) fn call_outgoing_info_text(
	peer_name: &str,
	phone_digits: Option<&str>,
	is_video: bool,
	is_group: bool,
	status: &str,
) -> String {
	let kind = if is_video { "video call" } else { "call" };
	let mut text = if is_group {
		format!("Outgoing group {kind} in {peer_name}")
	} else {
		format!("Outgoing {kind} to {peer_name}")
	};
	let name_is_number = |digits: &str| peer_name.chars().filter(char::is_ascii_digit).eq(digits.chars());
	if let Some(digits) = phone_digits.filter(|d| !d.is_empty() && !name_is_number(d)) {
		text.push_str("\r\n");
		text.push_str(&whatsnap::phone::format_international(digits).unwrap_or_else(|| format!("+{digits}")));
	}
	text.push_str("\r\n");
	text.push_str(status);
	text
}

pub(crate) fn call_ended_info_text(peer_name: &str, duration_secs: u64, reason: &str) -> String {
	if duration_secs > 0 {
		let clock = format_call_duration(duration_secs);
		let status = if reason == "connection_lost" { "Call ended. Connection lost" } else { "Call ended" };
		format!("{status} with {peer_name}\r\nDuration: {clock}")
	} else {
		let status = match reason {
			"busy" => "Busy, on another call",
			"declined" | "rejected" => "Call declined",
			"connection_lost" => "Call ended. Connection lost",
			"cancelled" => "Call cancelled",
			"timeout" | "no_answer" => "No answer",
			_ => "Call ended",
		};
		format!("{status} with {peer_name}")
	}
}

pub(crate) fn update_call_avatar(parts: &WindowParts, jid: &str) {
	let avatar_bm = if !jid.is_empty() {
		crate::ui::avatars::path_of(jid)
			.as_deref()
			.and_then(|path| crate::ui::owner_draw::round_picture(path, 112))
			.unwrap_or_else(|| crate::ui::call_window::empty_avatar_bitmap(112))
	} else {
		crate::ui::call_window::empty_avatar_bitmap(112)
	};
	parts.call_parts.avatar_bitmap.set_bitmap(&avatar_bm);
	parts.call_parts.avatar_bitmap.show(true);
}

/// What WhatsApp's call screen says about a call we're placing that hasn't
/// connected yet. `progress` is the backend's `call_progress` state.
pub(crate) fn outgoing_call_status(progress: &str) -> Option<&'static str> {
	match progress {
		"calling" => Some("Calling..."),
		"ringing" => Some("Ringing..."),
		"connecting" => Some("Connecting..."),
		_ => None,
	}
}

pub(crate) fn resolve_call_peer_name_and_phone(
	contacts: &std::collections::HashMap<String, String>,
	all_contacts: &[crate::Contact],
	caller: &str,
	chat_jid: Option<&str>,
	caller_name: Option<&str>,
	caller_phone: Option<&str>,
) -> (String, Option<String>) {
	// Normalize caller JID to remove any AD suffix (e.g. "123:0@s.whatsapp.net" -> "123@s.whatsapp.net")
	let clean_caller = if let Some((user, rest)) = caller.split_once('@') {
		let clean_user = user.split_once(':').map(|(u, _)| u).unwrap_or(user);
		format!("{}@{}", clean_user, rest)
	} else {
		caller.to_string()
	};

	// Determine phone digits
	let phone_digits = caller_phone
		.filter(|p| !p.is_empty())
		.map(|p| p.to_string())
		.or_else(|| {
			chat_jid.and_then(|j| {
				let (user, server) = j.split_once('@')?;
				let clean_user = user.split_once(':').map(|(u, _)| u).unwrap_or(user);
				(server == "s.whatsapp.net" && clean_user.chars().all(|c| c.is_ascii_digit()))
					.then(|| clean_user.to_string())
			})
		})
		.or_else(|| {
			let (user, server) = clean_caller.split_once('@')?;
			(server == "s.whatsapp.net" && user.chars().all(|c| c.is_ascii_digit())).then(|| user.to_string())
		})
		.or_else(|| {
			all_contacts.iter().find(|c| c.jid == clean_caller || chat_jid == Some(&c.jid)).map(|c| c.phone.clone())
		});

	// Find contact name from contacts map or all_contacts
	let contact_name = chat_jid
		.and_then(|j| contacts.get(j))
		.or_else(|| contacts.get(&clean_caller))
		.or_else(|| contacts.get(caller))
		.cloned()
		.or_else(|| {
			if let Some(digits) = &phone_digits {
				let jid_with_server = format!("{}@s.whatsapp.net", digits);
				contacts.get(&jid_with_server).cloned().or_else(|| {
					all_contacts.iter().find(|c| c.phone == *digits && !c.name.is_empty()).map(|c| c.name.clone())
				})
			} else {
				None
			}
		});

	let name = if let Some(saved) = contact_name {
		saved
	} else if let Some(n) = caller_name.filter(|n| !n.is_empty() && *n != caller && *n != clean_caller) {
		n.to_string()
	} else if let Some(digits) = &phone_digits {
		crate::ui::contact_picker::format_phone_digits(digits)
	} else {
		crate::ui::contact_picker::display_name_for_jid(&clean_caller)
	};

	(name, phone_digits)
}

#[cfg(test)]
mod caller_info_tests {
	use super::*;

	#[test]
	fn formats_call_duration_clock() {
		assert_eq!(format_call_duration(0), "0:00");
		assert_eq!(format_call_duration(42), "0:42");
		assert_eq!(format_call_duration(65), "1:05");
		assert_eq!(format_call_duration(125), "2:05");
		assert_eq!(format_call_duration(3665), "1:01:05");
	}

	#[test]
	fn running_call_shows_direction_peer_and_formatted_duration() {
		assert_eq!(call_running_info_text(true, false, "Asha", 0, false), "Incoming call from Asha\r\n0:00");
		assert_eq!(call_running_info_text(true, true, "Asha", 42, false), "Incoming video call from Asha\r\n0:42");
		assert_eq!(call_running_info_text(false, false, "Asha", 125, false), "Outgoing call to Asha\r\n2:05");
		assert_eq!(
			call_running_info_text(false, true, "Asha", 65, true),
			"Outgoing video call to Asha\r\nReconnecting... (1:05)"
		);
	}

	#[test]
	fn incoming_info_text_formats_multiline() {
		assert_eq!(
			call_incoming_info_text("Asha", Some("919810012345"), None, false),
			"Incoming call from Asha\r\n+91 98100 12345"
		);
		assert_eq!(
			call_incoming_info_text("Asha", None, Some("Family"), false),
			"Incoming group call from Asha\r\nin Family"
		);
		assert_eq!(call_incoming_info_text("Asha", None, None, true), "Incoming video call from Asha\r\nRinging...");
	}

	#[test]
	fn outgoing_info_text_formats_multiline() {
		assert_eq!(
			call_outgoing_info_text("Asha", None, false, false, "Calling..."),
			"Outgoing call to Asha\r\nCalling..."
		);
		assert_eq!(
			call_outgoing_info_text("Family", None, false, true, "Calling..."),
			"Outgoing group call in Family\r\nCalling..."
		);
		assert_eq!(
			call_outgoing_info_text("Asha", Some("919810012345"), true, false, "Ringing..."),
			"Outgoing video call to Asha\r\n+91 98100 12345\r\nRinging..."
		);
		// An unsaved number is already the name, so it isn't repeated.
		assert_eq!(
			call_outgoing_info_text("+1 415-555-2671", Some("14155552671"), false, false, "Connecting..."),
			"Outgoing call to +1 415-555-2671\r\nConnecting..."
		);
		assert_eq!(outgoing_call_status("ringing"), Some("Ringing..."));
		assert_eq!(outgoing_call_status("bogus"), None);
	}

	#[test]
	fn resolves_peer_name_and_phone_from_contacts() {
		let mut contacts = std::collections::HashMap::new();
		contacts.insert("919810012345@s.whatsapp.net".to_string(), "Asha".to_string());
		let all_contacts = vec![];

		// Case 1: Caller JID has AD suffix (:0), backend didn't resolve name
		let (name, phone) = resolve_call_peer_name_and_phone(
			&contacts,
			&all_contacts,
			"919810012345:0@s.whatsapp.net",
			Some("919810012345@s.whatsapp.net"),
			Some("+919810012345"),
			None,
		);
		assert_eq!(name, "Asha");
		assert_eq!(phone.as_deref(), Some("919810012345"));

		// Case 2: Caller is LID, chat_jid has PN
		let (name, phone) = resolve_call_peer_name_and_phone(
			&contacts,
			&all_contacts,
			"123456789@lid",
			Some("919810012345@s.whatsapp.net"),
			None,
			None,
		);
		assert_eq!(name, "Asha");
		assert_eq!(phone.as_deref(), Some("919810012345"));

		// Case 3: Unknown number not in contacts
		let (name, phone) = resolve_call_peer_name_and_phone(
			&contacts,
			&all_contacts,
			"14155552671:1@s.whatsapp.net",
			Some("14155552671@s.whatsapp.net"),
			None,
			None,
		);
		assert_eq!(name, "+1 415-555-2671");
		assert_eq!(phone.as_deref(), Some("14155552671"));
	}

	#[test]
	fn formats_call_ended_info_text() {
		assert_eq!(call_ended_info_text("Asha", 65, "busy"), "Call ended with Asha\r\nDuration: 1:05");
		assert_eq!(call_ended_info_text("Asha", 0, "busy"), "Busy, on another call with Asha");
		assert_eq!(call_ended_info_text("Asha", 0, "declined"), "Call declined with Asha");
		assert_eq!(
			call_ended_info_text("Asha", 120, "connection_lost"),
			"Call ended. Connection lost with Asha\r\nDuration: 2:00"
		);
	}
}

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

/// Returns the peer's display name for title bars.
/// Numbers and JIDs are NEVER shown in title bars.
/// If saved: returns the saved name.
/// If unsaved with a push name: returns "PushName (Unknown contact)".
/// If unsaved without a name: returns "Unknown contact".
pub(crate) fn clean_call_title_peer_name(raw_name: &str, is_saved: bool) -> String {
	let trimmed = raw_name.trim();
	let is_number = trimmed.is_empty()
		|| trimmed.starts_with('+')
		|| trimmed
			.chars()
			.all(|c| c.is_ascii_digit() || c.is_whitespace() || c == '-' || c == '+' || c == '(' || c == ')')
		|| trimmed.contains('@');

	if is_saved && !is_number {
		trimmed.to_string()
	} else if !is_number {
		format!("{} (Unknown contact)", trimmed)
	} else {
		"Unknown contact".to_string()
	}
}

/// Formats the title bar for call windows according to WhatsApp standards.
/// NEVER includes raw phone digits or duration (caller_info already shows duration).
pub(crate) fn format_call_window_title(
	is_incoming: bool,
	is_group: bool,
	is_video: bool,
	peer_name: &str,
	is_saved: bool,
	_duration_clock: Option<&str>,
) -> String {
	if is_group {
		let kind = if is_video { "Group video call" } else { "Group voice call" };
		format!("{kind} in {peer_name}")
	} else {
		let title_name = clean_call_title_peer_name(peer_name, is_saved);
		if is_incoming {
			let kind = if is_video { "Incoming video call" } else { "Incoming voice call" };
			format!("{kind} from {title_name}")
		} else {
			let kind = if is_video { "Video call" } else { "Voice call" };
			format!("{kind} with {title_name}")
		}
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
			Some(format!("{} wants to switch to video. Turn on your camera (Ctrl+Shift+V) to switch to video.", who))
		}
		_ => None,
	}
}

pub(crate) fn local_camera_announcement(on: bool) -> &'static str {
	if on { "Camera on." } else { "Camera off." }
}

pub(crate) fn peer_screen_share_announcement(name: &str, active: bool) -> String {
	let who = if name.is_empty() { "Someone" } else { name };
	if active {
		format!("{} started sharing their screen.", who)
	} else {
		format!("{} stopped sharing their screen.", who)
	}
}

pub(crate) fn local_screen_share_announcement(sharing: bool, window: Option<&str>, camera_changed: bool) -> String {
	let start = match window {
		Some(title) => format!("You're sharing a window: {title}."),
		None => "You're sharing your screen.".to_string(),
	};
	match (sharing, camera_changed) {
		(true, false) => start,
		(true, true) => format!("{start} Camera off."),
		(false, false) => "Stopped sharing your screen.".to_string(),
		(false, true) => "Stopped sharing your screen. Camera on.".to_string(),
	}
}

/// What's said when a screen share starts carrying the computer's sound.
/// It says the screen reader is heard too, since whoever shares with one on
/// would otherwise only find out from the others.
pub(crate) const SYSTEM_AUDIO_ANNOUNCEMENT: &str =
	"Your computer's sound is shared too, screen reader speech included. Muting your microphone mutes it as well.";

pub(crate) fn show_call_device_combos(s: &AppState, parts: &WindowParts) {
	use crate::ui::dialogs::settings_dialog::set_device_combo;
	set_device_combo(
		&parts.call_parts.input_device_combo,
		&crate::audio::devices::input_device_names(),
		s.settings.input_device_name.as_deref(),
	);
	set_device_combo(
		&parts.call_parts.output_device_combo,
		&crate::audio::devices::output_device_names(),
		s.settings.call_output_device_name.as_deref(),
	);
	let is_video = s.call_video.video_call || s.call_video.is_video || s.call_video.local_camera_on;
	parts.call_parts.camera_device_group.show(is_video);
	if is_video {
		set_device_combo(
			&parts.call_parts.camera_device_combo,
			&crate::calls::video_devices::camera_names(),
			s.settings.camera_device_name.as_deref(),
		);
	}
	parts.call_parts.frame.layout();
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
	fn unanswered_invite_stops_ringing() {
		let ringing = [p("1@lid", "Asha", "connected"), p("2@lid", "Ben", "receipt"), p("3@lid", "Chris", "outgoing")];
		assert_eq!(group_call_status_text(&ringing), "In group call with Asha. Ringing Ben and Chris.");
		let timed_out = [p("1@lid", "Asha", "connected"), p("2@lid", "Ben", "invited"), p("3@lid", "Chris", "invited")];
		assert_eq!(group_call_status_text(&timed_out), "In group call with Asha.");
		assert_eq!(call_participant_changes(&ringing, &timed_out), vec!["Ben didn't answer.", "Chris didn't answer."]);
		assert!(call_participant_changes(&timed_out, &timed_out).is_empty());
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
			Some("Asha wants to switch to video. Turn on your camera (Ctrl+Shift+V) to switch to video.")
		);
		assert_eq!(peer_video_announcement("bogus", None, Some("Asha")), None);
	}

	#[test]
	fn screen_share_announcements_name_the_sharer_and_the_camera() {
		assert_eq!(peer_screen_share_announcement("Asha", true), "Asha started sharing their screen.");
		assert_eq!(peer_screen_share_announcement("", false), "Someone stopped sharing their screen.");
		assert_eq!(local_screen_share_announcement(true, None, true), "You're sharing your screen. Camera off.");
		assert_eq!(local_screen_share_announcement(true, Some("Notes"), false), "You're sharing a window: Notes.");
		assert_eq!(local_screen_share_announcement(false, None, false), "Stopped sharing your screen.");
	}

	#[test]
	fn local_camera_announces_on_and_off() {
		assert_eq!(local_camera_announcement(true), "Camera on.");
		assert_eq!(local_camera_announcement(false), "Camera off.");
	}

	#[test]
	fn titles_format_cleanly_without_numbers() {
		assert_eq!(
			format_call_window_title(false, false, false, "Shubham Sharma", true, None),
			"Voice call with Shubham Sharma"
		);
		assert_eq!(
			format_call_window_title(false, false, false, "Shubham Sharma", true, Some("01:25")),
			"Voice call with Shubham Sharma"
		);
		assert_eq!(
			format_call_window_title(false, false, true, "Aarav", false, None),
			"Video call with Aarav (Unknown contact)"
		);
		assert_eq!(
			format_call_window_title(false, false, false, "+91 98765 43210", false, None),
			"Voice call with Unknown contact"
		);
		assert_eq!(
			format_call_window_title(true, false, false, "Priya", false, None),
			"Incoming voice call from Priya (Unknown contact)"
		);
		assert_eq!(
			format_call_window_title(true, false, true, "+1234567890", false, None),
			"Incoming video call from Unknown contact"
		);
		assert_eq!(
			format_call_window_title(false, true, false, "Dev Group", true, Some("04:12")),
			"Group voice call in Dev Group"
		);
	}
}
