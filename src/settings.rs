//! User-configurable app preferences, persisted as JSON next to the session
//! database (see [`crate::paths::app_data_dir`]).

use std::path::PathBuf;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
	pub sounds_enabled: bool,
	pub notifications_enabled: bool,
	/// Whether the background message sound stays quiet while Windows is
	/// holding its own notifications back (Do Not Disturb / Focus Assist, a
	/// full-screen game, presentation mode). See [`crate::do_not_disturb`].
	pub follow_windows_do_not_disturb: bool,
	pub announce_typing: bool,
	pub announce_recording: bool,
	pub show_tray_icon: bool,
	/// Whether the "Message Meta AI" button appears in the New Conversation
	/// dialog. Turning this off only removes that entry point - an existing
	/// Meta AI chat already in the chat list is left alone, same as other
	/// settings here don't retroactively touch chat data.
	pub meta_ai_enabled: bool,
	/// Friendly name of the preferred microphone for calls, or `None` for the
	/// system default communications device. Also used for voice-message
	/// recording (see [`crate::voice_recorder`]), which honors this device.
	pub input_device_name: Option<String>,
	/// Friendly name of the preferred speaker/output device for calls and for
	/// voice-message/notification playback, or `None` for the system default.
	pub output_device_name: Option<String>,
	/// Friendly name of the preferred camera for video calls, or `None` for
	/// "System Default" (the first device Media Foundation enumerates - see
	/// `video_devices.rs`).
	pub camera_device_name: Option<String>,
	/// Whether recorded voice messages go through `voice_dsp::VoiceCleanup`
	/// (high-pass, noise suppression, automatic gain, limiter) - the processing
	/// a browser gives WhatsApp Web's voice notes. Off keeps the raw mic signal.
	/// (The field name predates it; kept so saved settings carry over.)
	pub voice_message_noise_suppression: bool,
	/// Per-chat alert level for archived chats, keyed by JID. A chat with no
	/// entry uses [`whatsnap::ArchivedAlert::default`] (silent), so archiving
	/// quiets a chat with no bookkeeping at all. Entries deliberately survive
	/// unarchiving: re-archiving a chat shouldn't silently discard the choice
	/// made for it last time.
	pub archived_alert_levels: std::collections::HashMap<String, whatsnap::ArchivedAlert>,
	/// Whether the backend logs every WhatsApp protocol frame and SQL query
	/// (DEBUG level) instead of just warnings/errors. Off by default: it's
	/// continuous, high-volume logging only useful while actively
	/// troubleshooting a connection issue. Read once at startup when the
	/// backend process is launched - toggling it takes effect the next time
	/// whatSnap starts, not on the running backend.
	pub verbose_backend_logging: bool,
	/// Whether the main window opens maximized the first time it is shown,
	/// like Paperback's "Start maximized" option.
	pub start_maximized: bool,
	/// Menu shortcuts the user changed, by action id; see [`crate::keymap`].
	/// Anything not listed keeps its default.
	pub shortcuts: std::collections::BTreeMap<String, String>,
	/// Order of the main navigation tabs, as tab ids (see [`TAB_NAMES`]).
	/// Read it through [`Settings::tab_order`], which repairs a saved list that
	/// is missing a tab or repeats one.
	pub tab_order: Vec<usize>,
	/// Last-used voice-message playback speed (1.0, 1.5, or 2.0). Persisted so
	/// the chosen speed sticks across app restarts.
	pub audio_speed: f32,
	/// Experimental: a screen share goes out as its own stream next to the
	/// camera (WhatsApp's dual-stream share) instead of taking the camera's
	/// place. Applies to calls that connect after it's turned on.
	pub dual_stream_screen_share: bool,
}

/// Tab names by tab id. The id is what the rest of the app keys on
/// (`active_tab`, `nav_selected`, the Alt+N shortcuts); only the position in
/// the list changes when tabs are reordered.
pub const TAB_NAMES: [&str; 7] =
	["Chats", "Status", "Starred Messages", "Archived Chats", "Communities", "Channels", "Calls"];

/// Tabs whose navigation entry carries how many of their chats are unread.
pub const NAV_UNREAD_TABS: [usize; 2] = [0, 3];

/// The text a tab shows in the navigation list. Chats (tab id 0) and Archived
/// Chats (3) carry how many of their chats are unread, so a screen reader
/// reading the list or the Alt+N shortcut hears "Chats: 5 unread"; every other
/// tab is just its name.
pub fn nav_tab_label(tab: usize, unread_chats: usize) -> String {
	if unread_chats > 0 && NAV_UNREAD_TABS.contains(&tab) {
		format!("{}: {} unread", TAB_NAMES[tab], unread_chats)
	} else {
		TAB_NAMES[tab].to_string()
	}
}

/// Chats and Calls first, then the rest in their original order.
pub const DEFAULT_TAB_ORDER: [usize; 7] = [0, 6, 1, 2, 3, 4, 5];

/// Moves the tab at `pos` one place by `delta` (-1 or 1). Returns its new
/// position, or `None` at either end.
pub fn move_tab(order: &mut [usize], pos: usize, delta: isize) -> Option<usize> {
	let target = pos.checked_add_signed(delta).filter(|&t| t < order.len() && pos < order.len())?;
	order.swap(pos, target);
	Some(target)
}

impl Default for Settings {
	fn default() -> Self {
		Self {
			sounds_enabled: true,
			notifications_enabled: true,
			follow_windows_do_not_disturb: true,
			announce_typing: true,
			announce_recording: true,
			show_tray_icon: true,
			meta_ai_enabled: true,
			input_device_name: None,
			output_device_name: None,
			camera_device_name: None,
			voice_message_noise_suppression: false,
			archived_alert_levels: std::collections::HashMap::new(),
			verbose_backend_logging: false,
			start_maximized: false,
			shortcuts: std::collections::BTreeMap::new(),
			tab_order: DEFAULT_TAB_ORDER.to_vec(),
			audio_speed: 1.0,
			dual_stream_screen_share: false,
		}
	}
}

fn settings_path() -> PathBuf {
	crate::paths::app_data_dir().join("settings.json")
}

impl Settings {
	pub fn load() -> Self {
		std::fs::read_to_string(settings_path())
			.ok()
			.and_then(|raw| serde_json::from_str(&raw).ok())
			.unwrap_or_default()
	}

	/// Alert level for `jid` as an archived chat. Chats that aren't archived
	/// never consult this - see [`whatsnap::incoming_alert`].
	pub fn archived_alert(&self, jid: &str) -> whatsnap::ArchivedAlert {
		self.archived_alert_levels.get(jid).copied().unwrap_or_default()
	}

	/// Records `level` for `jid`, dropping the entry when it's the default so
	/// settings.json doesn't grow a line for every archived chat left alone.
	pub fn set_archived_alert(&mut self, jid: &str, level: whatsnap::ArchivedAlert) {
		if level == whatsnap::ArchivedAlert::default() {
			self.archived_alert_levels.remove(jid);
		} else {
			self.archived_alert_levels.insert(jid.to_string(), level);
		}
	}

	pub fn save(&self) {
		if let Ok(json) = serde_json::to_string_pretty(self) {
			let _ = std::fs::write(settings_path(), json);
		}
	}

	/// The saved tab order, or the default when it doesn't list each tab
	/// exactly once.
	pub fn tab_order(&self) -> Vec<usize> {
		let mut seen = [false; TAB_NAMES.len()];
		let valid = self.tab_order.len() == TAB_NAMES.len()
			&& self.tab_order.iter().all(|&t| t < TAB_NAMES.len() && !std::mem::replace(&mut seen[t], true));
		if valid { self.tab_order.clone() } else { DEFAULT_TAB_ORDER.to_vec() }
	}

	/// Voice message playback speed (1.0, 1.5, or 2.0), falling back to 1.0
	/// if a saved value is missing, unrecognized, or non-finite.
	pub fn audio_speed(&self) -> f32 {
		if (self.audio_speed - 1.5).abs() < 0.01 {
			1.5
		} else if (self.audio_speed - 2.0).abs() < 0.01 {
			2.0
		} else {
			1.0
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn audio_speed_defaults_to_1() {
		let s = Settings::default();
		assert_eq!(s.audio_speed(), 1.0);
	}

	#[test]
	fn audio_speed_validates_known_speeds() {
		let s1 = Settings { audio_speed: 1.5, ..Settings::default() };
		assert_eq!(s1.audio_speed(), 1.5);
		let s2 = Settings { audio_speed: 2.0, ..Settings::default() };
		assert_eq!(s2.audio_speed(), 2.0);
		let s_bad = Settings { audio_speed: 0.0, ..Settings::default() };
		assert_eq!(s_bad.audio_speed(), 1.0);
		let s_weird = Settings { audio_speed: 3.5, ..Settings::default() };
		assert_eq!(s_weird.audio_speed(), 1.0);
	}

	#[test]
	fn only_chats_shows_an_unread_count() {
		assert_eq!(nav_tab_label(0, 0), "Chats");
		assert_eq!(nav_tab_label(0, 1), "Chats: 1 unread");
		assert_eq!(nav_tab_label(0, 5), "Chats: 5 unread");
		assert_eq!(nav_tab_label(1, 5), "Status");
		assert_eq!(nav_tab_label(6, 5), "Calls");
		assert_eq!(nav_tab_label(3, 0), "Archived Chats");
		assert_eq!(nav_tab_label(3, 2), "Archived Chats: 2 unread");
	}

	#[test]
	fn default_puts_calls_after_chats() {
		let s = Settings::default();
		assert_eq!(s.tab_order(), vec![0, 6, 1, 2, 3, 4, 5]);
	}

	#[test]
	fn bad_saved_orders_fall_back_to_default() {
		for bad in [vec![], vec![0, 1, 2], vec![0, 0, 1, 2, 3, 4, 5], vec![0, 1, 2, 3, 4, 5, 9]] {
			let s = Settings { tab_order: bad, ..Settings::default() };
			assert_eq!(s.tab_order(), DEFAULT_TAB_ORDER.to_vec());
		}
	}

	#[test]
	fn a_reordered_list_is_kept() {
		let s = Settings { tab_order: vec![6, 5, 4, 3, 2, 1, 0], ..Settings::default() };
		assert_eq!(s.tab_order(), vec![6, 5, 4, 3, 2, 1, 0]);
	}

	#[test]
	fn moving_swaps_with_the_neighbour_and_stops_at_the_ends() {
		let mut order = vec![0, 6, 1];
		assert_eq!(move_tab(&mut order, 1, 1), Some(2));
		assert_eq!(order, vec![0, 1, 6]);
		assert_eq!(move_tab(&mut order, 2, 1), None);
		assert_eq!(move_tab(&mut order, 0, -1), None);
		assert_eq!(move_tab(&mut order, 2, -1), Some(1));
		assert_eq!(order, vec![0, 6, 1]);
	}
}
