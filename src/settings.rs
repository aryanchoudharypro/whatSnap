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
	/// How wide the user dragged the chat list column, in pixels; 0 lets it
	/// follow the window's width.
	pub chat_list_width: i32,
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
	/// The status viewer moves on to the next update by itself (a photo or
	/// text after a few seconds, a video when it ends), as WhatsApp's does.
	/// Off, it waits for Next.
	pub status_auto_advance: bool,
	/// WhatsApp's "Enter is send": Enter sends and Shift+Enter starts a new
	/// line. Off, Enter starts a new line and the Send button sends.
	pub enter_is_send: bool,
	/// WhatsApp's "Replace text with emoji": typing :-) or (y) and the like
	/// turns it into the emoji (see `ui::emoji_replace`).
	pub replace_text_with_emoji: bool,
	/// Spell checking in the message box (see `ui::spell_check`).
	pub spell_check: bool,
	/// Say "Misspelled: word" when a word is finished wrongly. NVDA can't
	/// report it in this box itself, so it's ours; off by default.
	pub spell_check_announce: bool,
	/// The language to check, as a tag like "en-GB"; empty for Windows' own.
	pub spell_language: String,
	/// WhatsApp's "Show previews": a message notification shows the
	/// message's text. Off, it only says a message came.
	pub notification_previews: bool,
	/// WhatsApp's "Show reaction notifications": a reaction to one of our
	/// messages in a chat that isn't open notifies.
	pub reaction_notifications: bool,
	/// WhatsApp's Theme: System default, Light or Dark.
	pub theme: crate::ui::theme::ThemeChoice,
	/// WhatsApp's separate group settings (its granular notifications): a
	/// group's new messages show a notification, play the sound.
	pub group_notifications: bool,
	pub group_sounds: bool,
	/// WhatsApp's "Play sounds for outgoing messages".
	pub outgoing_sounds: bool,
	/// WhatsApp's "Call notifications" and "Play sounds for incoming calls".
	pub call_notifications: bool,
	pub call_ringtone: bool,
	/// Each chat's notification choices on this computer, by chat; chats
	/// left as they come have no entry.
	pub chat_notifications: std::collections::HashMap<String, whatsnap::ChatNotify>,
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
			chat_list_width: 0,
			shortcuts: std::collections::BTreeMap::new(),
			tab_order: DEFAULT_TAB_ORDER.to_vec(),
			audio_speed: 1.0,
			dual_stream_screen_share: false,
			status_auto_advance: true,
			enter_is_send: true,
			replace_text_with_emoji: true,
			spell_check: true,
			spell_check_announce: false,
			spell_language: String::new(),
			notification_previews: true,
			reaction_notifications: true,
			theme: Default::default(),
			group_notifications: true,
			group_sounds: true,
			outgoing_sounds: true,
			call_notifications: true,
			call_ringtone: true,
			chat_notifications: std::collections::HashMap::new(),
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

	/// `jid`'s notification choices on this computer.
	pub fn chat_notify(&self, jid: &str) -> whatsnap::ChatNotify {
		self.chat_notifications.get(jid).copied().unwrap_or_default()
	}

	/// Records `notify` for `jid`, dropping the entry when it's the default.
	pub fn set_chat_notify(&mut self, jid: &str, notify: whatsnap::ChatNotify) {
		if notify == whatsnap::ChatNotify::default() {
			self.chat_notifications.remove(jid);
		} else {
			self.chat_notifications.insert(jid.to_string(), notify);
		}
	}

	/// What a new message in `jid` may do, by every rule there is: WhatsApp's
	/// mute, archiving, the chat's choice on this computer, then the app's
	/// own switches (a chat's own choice stands over the group ones).
	pub fn message_alert(&self, jid: &str, muted: bool, archived: bool) -> whatsnap::IncomingAlert {
		let base = whatsnap::incoming_alert(muted, archived, self.archived_alert(jid));
		let chat = self.chat_notify(jid).alert;
		let alert = whatsnap::with_chat_alert(base, muted, chat);
		let group = jid.ends_with("@g.us") && chat == whatsnap::ChatAlert::Default;
		whatsnap::IncomingAlert {
			sound: alert.sound && self.sounds_enabled && (!group || self.group_sounds),
			toast: alert.toast && self.notifications_enabled && (!group || self.group_notifications),
		}
	}

	/// `muted` with the chats turned off on this computer: those the tray's
	/// unread total leaves out.
	pub fn quiet_chats(&self, muted: &std::collections::HashSet<String>) -> std::collections::HashSet<String> {
		let off = self.chat_notifications.iter().filter(|(_, n)| n.alert == whatsnap::ChatAlert::Off);
		muted.iter().cloned().chain(off.map(|(jid, _)| jid.clone())).collect()
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

	/// The settings WhatsApp Web and its desktop apps share between
	/// themselves (settings sync), by WhatsApp's name for each
	/// (WAWebSettingsSyncConst) and in its form: a sound switch is a tone,
	/// None (0) or Default (1), and the theme is 0 system, 1 light, 2 dark.
	pub fn synced(&self) -> Vec<(&'static str, serde_json::Value)> {
		use crate::ui::theme::ThemeChoice;
		let theme = match self.theme {
			ThemeChoice::System => 0,
			ThemeChoice::Light => 1,
			ThemeChoice::Dark => 2,
		};
		vec![
			("isMessagesNotificationEnabled", self.notifications_enabled.into()),
			("defaultNotificationToneId", i32::from(self.sounds_enabled).into()),
			("isGroupMessageNotificationEnabled", self.group_notifications.into()),
			("groupDefaultNotificationToneId", i32::from(self.group_sounds).into()),
			("isTextPreviewForNotificationEnabled", self.notification_previews.into()),
			("isReactionsNotificationEnabled", self.reaction_notifications.into()),
			("isCallsNotificationEnabled", self.call_notifications.into()),
			("shouldPlaySoundForCallNotification", self.call_ringtone.into()),
			("isEnterToSendEnabled", self.enter_is_send.into()),
			("replaceTextWithEmoji", self.replace_text_with_emoji.into()),
			("isSpellCheckEnabled", self.spell_check.into()),
			("appTheme", theme.into()),
		]
	}

	/// What changed in [`Settings::synced`] since `before` was taken.
	pub fn synced_changes(
		&self,
		before: &[(&'static str, serde_json::Value)],
	) -> serde_json::Map<String, serde_json::Value> {
		self.synced()
			.into_iter()
			.zip(before)
			.filter(|((_, now), (_, was))| now != was)
			.map(|((name, now), _)| (name.to_string(), now))
			.collect()
	}

	/// Takes a setting another WhatsApp desktop device changed; true when
	/// it changed something here. Any tone but None means sounds are on, as
	/// WhatsApp only plays none for None.
	pub fn apply_synced(&mut self, name: &str, value: &serde_json::Value) -> bool {
		use crate::ui::theme::ThemeChoice;
		let on = value.as_bool().or_else(|| value.as_i64().map(|tone| tone != 0));
		let Some(on) = on else { return false };
		let field = match name {
			"isMessagesNotificationEnabled" => &mut self.notifications_enabled,
			"defaultNotificationToneId" => &mut self.sounds_enabled,
			"isGroupMessageNotificationEnabled" => &mut self.group_notifications,
			"groupDefaultNotificationToneId" => &mut self.group_sounds,
			"isTextPreviewForNotificationEnabled" => &mut self.notification_previews,
			"isReactionsNotificationEnabled" => &mut self.reaction_notifications,
			"isCallsNotificationEnabled" => &mut self.call_notifications,
			"shouldPlaySoundForCallNotification" => &mut self.call_ringtone,
			"isEnterToSendEnabled" => &mut self.enter_is_send,
			"replaceTextWithEmoji" => &mut self.replace_text_with_emoji,
			"isSpellCheckEnabled" => &mut self.spell_check,
			"appTheme" => {
				let theme = match value.as_i64() {
					Some(0) => ThemeChoice::System,
					Some(1) => ThemeChoice::Light,
					Some(2) => ThemeChoice::Dark,
					_ => return false,
				};
				return std::mem::replace(&mut self.theme, theme) != theme;
			}
			_ => return false,
		};
		std::mem::replace(field, on) != on
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
	fn synced_settings_round_trip_in_whatsapps_form() {
		let mut s = Settings::default();
		let before = s.synced();
		s.sounds_enabled = false;
		s.theme = crate::ui::theme::ThemeChoice::Dark;
		let changes = s.synced_changes(&before);
		assert_eq!(changes.len(), 2);
		assert_eq!(changes["defaultNotificationToneId"], serde_json::json!(0));
		assert_eq!(changes["appTheme"], serde_json::json!(2));

		let mut other = Settings::default();
		for (name, value) in &changes {
			assert!(other.apply_synced(name, value));
		}
		assert!(!other.sounds_enabled);
		assert_eq!(other.theme, crate::ui::theme::ThemeChoice::Dark);
		// A chosen alert tone still means sounds on; the same value changes nothing.
		assert!(other.apply_synced("defaultNotificationToneId", &serde_json::json!(5)));
		assert!(!other.apply_synced("defaultNotificationToneId", &serde_json::json!(1)));
		assert!(!other.apply_synced("appTheme", &serde_json::json!(9)));
		assert!(!other.apply_synced("wallpaperId", &serde_json::json!(true)));
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
