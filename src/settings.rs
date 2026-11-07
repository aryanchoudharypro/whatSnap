use std::path::PathBuf;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
	pub sounds_enabled: bool,
	pub notifications_enabled: bool,
	/// Whether the background message sound stays quiet while Windows is
	/// holding its own notifications back (Do Not Disturb / Focus Assist, a
	/// full-screen game, presentation mode). See [`crate::platform::do_not_disturb`].
	pub follow_windows_do_not_disturb: bool,
	pub announce_typing: bool,
	pub announce_recording: bool,
	pub show_tray_icon: bool,
	/// Whether the "Message Meta AI" button appears in the New Conversation
	/// dialog. Turning this off only removes that entry point - an existing
	/// Meta AI chat already in the chat list is left alone, same as other
	/// settings here don't retroactively touch chat data.
	pub meta_ai_enabled: bool,
	pub input_device_name: Option<String>,
	/// The output device for alert sounds and voice messages.
	pub output_device_name: Option<String>,
	/// The output device for calls: ringing, ringback and the other call
	/// tones, and the call itself. Separate from `output_device_name` so
	/// alerts can play on one device while calls use another.
	pub call_output_device_name: Option<String>,
	pub camera_device_name: Option<String>,
	pub voice_message_noise_suppression: bool,
	pub archived_alert_levels: std::collections::HashMap<String, whatsnap::ArchivedAlert>,
	pub verbose_backend_logging: bool,
	pub start_maximized: bool,
	pub check_for_updates_on_startup: bool,
	pub update_channel: String,
	pub chat_list_width: i32,
	pub shortcuts: std::collections::BTreeMap<String, String>,
	pub tab_order: Vec<usize>,
	pub audio_speed: f32,
	pub dual_stream_screen_share: bool,
	pub status_auto_advance: bool,
	pub enter_is_send: bool,
	/// Typing a letter while the message list has focus moves to the message
	/// box with that letter, and the one-letter message shortcuts (R reply,
	/// F forward...) then need Alt.
	pub type_to_compose: bool,
	/// Leaving the message list or sending doesn't jump it to the newest
	/// message; it stays where the user was.
	pub keep_list_position: bool,
	/// New messages in the open chat are read out. Chats can override it,
	/// see [`whatsnap::ChatNotify::announce`].
	pub announce_incoming: bool,
	/// Our own sent messages are read out too, when the chat reads new
	/// messages aloud. Off by default.
	pub announce_outgoing: bool,
	pub replace_text_with_emoji: bool,
	pub spell_check: bool,
	/// Say "Misspelled: word" when a word is finished wrongly. NVDA can't
	/// report it in this box itself, so it's ours; off by default.
	pub spell_check_announce: bool,
	pub spell_language: String,
	pub notification_previews: bool,
	pub reaction_notifications: bool,
	pub theme: crate::ui::theme::ThemeChoice,
	pub group_notifications: bool,
	pub group_sounds: bool,
	/// Which sound new messages play, as a [`whatsnap::TONE_DEFAULT`] style
	/// number. Whether they play one at all is `sounds_enabled`; WhatsApp
	/// folds both into one synced number, None meaning off.
	pub message_tone: u8,
	pub group_tone: u8,
	/// The sound files a [`whatsnap::TONE_CUSTOM`] message or group tone plays.
	pub message_tone_file: String,
	pub group_tone_file: String,
	pub outgoing_sounds: bool,
	pub call_notifications: bool,
	pub call_ringtone: bool,
	/// What calls and group calls ring with, see [`whatsnap::RINGTONE_WHATSAPP`];
	/// empty is WhatsApp's own ringtone. Whether they ring at all is
	/// `call_ringtone`. Only this computer has these.
	pub call_ringtone_sound: String,
	pub group_call_ringtone_sound: String,
	pub chat_notifications: std::collections::HashMap<String, whatsnap::ChatNotify>,
	/// Unlock locked chats with Windows Hello (fingerprint, face or PIN)
	/// instead of asking for the secret code each time. Only this computer
	/// has it.
	pub chat_lock_hello: bool,
	/// The one-time "unlock your chats faster" offer has been made, so it is
	/// not shown again whatever the answer was.
	pub chat_lock_hello_offered: bool,
	/// Lock the whole app (on start and with the Lock App shortcut) until
	/// the user unlocks it. Only this computer has it.
	/// The three app lock fields are not in `settings.json`: they live in a
	/// DPAPI blob, see [`crate::platform::lock_store`].
	#[serde(skip)]
	pub app_lock: bool,
	/// Unlock the app with Windows Hello; otherwise with the password.
	#[serde(skip)]
	pub app_lock_hello: bool,
	/// The password's salted hash, see [`whatsnap::passhash`]; never the
	/// password itself. Empty until one is set.
	#[serde(skip)]
	pub app_lock_password: String,
}

pub const TAB_NAMES: [&str; 7] =
	["Chats", "Status", "Starred Messages", "Archived Chats", "Communities", "Channels", "Calls"];

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

pub const DEFAULT_TAB_ORDER: [usize; 7] = [0, 6, 1, 2, 3, 4, 5];

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
			call_output_device_name: None,
			camera_device_name: None,
			voice_message_noise_suppression: false,
			archived_alert_levels: std::collections::HashMap::new(),
			verbose_backend_logging: false,
			start_maximized: false,
			check_for_updates_on_startup: true,
			update_channel: "stable".to_string(),
			chat_list_width: 0,
			shortcuts: std::collections::BTreeMap::new(),
			tab_order: DEFAULT_TAB_ORDER.to_vec(),
			audio_speed: 1.0,
			dual_stream_screen_share: false,
			status_auto_advance: true,
			enter_is_send: true,
			type_to_compose: false,
			keep_list_position: false,
			announce_incoming: true,
			announce_outgoing: false,
			replace_text_with_emoji: true,
			spell_check: true,
			spell_check_announce: false,
			spell_language: String::new(),
			notification_previews: true,
			reaction_notifications: true,
			theme: Default::default(),
			group_notifications: true,
			group_sounds: true,
			message_tone: whatsnap::TONE_DEFAULT,
			group_tone: whatsnap::TONE_DEFAULT,
			message_tone_file: String::new(),
			group_tone_file: String::new(),
			outgoing_sounds: true,
			call_notifications: true,
			call_ringtone: true,
			call_ringtone_sound: String::new(),
			group_call_ringtone_sound: String::new(),
			chat_notifications: std::collections::HashMap::new(),
			chat_lock_hello: false,
			chat_lock_hello_offered: false,
			app_lock: false,
			app_lock_hello: false,
			app_lock_password: String::new(),
		}
	}
}

fn synced_tone(on: bool, tone: u8) -> serde_json::Value {
	match tone {
		_ if !on => whatsnap::TONE_NONE.into(),
		whatsnap::TONE_CUSTOM => serde_json::Value::Null,
		tone => tone.into(),
	}
}

fn settings_path() -> PathBuf {
	crate::platform::paths::app_data_dir().join("settings.json")
}

impl Settings {
	pub fn load() -> Self {
		let fallback = || {
			let mut settings = Self::default();
			settings.apply_lock_store(None);
			settings
		};
		let Some(raw) = std::fs::read_to_string(settings_path()).ok() else { return fallback() };
		let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&raw) else { return fallback() };
		// Calls used to share the one output device with alerts. Settings
		// saved before the split carry on using that device for calls too.
		if let Some(obj) = value.as_object_mut()
			&& !obj.contains_key("call_output_device_name")
		{
			let inherited = obj.get("output_device_name").cloned().unwrap_or(serde_json::Value::Null);
			obj.insert("call_output_device_name".to_string(), inherited);
		}
		// App lock settings used to be plain keys in this file.
		let old_lock = value.as_object_mut().and_then(|obj| {
			let on = obj.remove("app_lock");
			let hello = obj.remove("app_lock_hello");
			let hash = obj.remove("app_lock_password");
			(on.is_some() || hello.is_some() || hash.is_some()).then(|| {
				(
					on.and_then(|v| v.as_bool()).unwrap_or(false),
					hello.and_then(|v| v.as_bool()).unwrap_or(false),
					hash.and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default(),
				)
			})
		});
		let mut settings: Self = serde_json::from_value(value).unwrap_or_default();
		settings.apply_lock_store(old_lock);
		settings
	}

	/// Fills in the app lock from the protected store. Keys the old
	/// `settings.json` had are used once, when no store exists yet, and are
	/// then scrubbed from the file; otherwise they are ignored.
	fn apply_lock_store(&mut self, old: Option<(bool, bool, String)>) {
		let had_old = old.is_some();
		match old {
			Some((on, hello, hash)) if !crate::platform::lock_store::exists() => {
				(self.app_lock, self.app_lock_hello, self.app_lock_password) = (on, hello, hash);
			}
			_ => {
				let data = crate::platform::lock_store::load();
				(self.app_lock, self.app_lock_hello, self.app_lock_password) = (data.enabled, data.hello, data.hash);
			}
		}
		if had_old {
			self.save();
		}
	}

	/// Alert level for `jid` as an archived chat. Chats that aren't archived
	/// never consult this - see [`whatsnap::incoming_alert`].
	pub fn archived_alert(&self, jid: &str) -> whatsnap::ArchivedAlert {
		self.archived_alert_levels.get(jid).copied().unwrap_or_default()
	}

	pub fn set_archived_alert(&mut self, jid: &str, level: whatsnap::ArchivedAlert) {
		if level == whatsnap::ArchivedAlert::default() {
			self.archived_alert_levels.remove(jid);
		} else {
			self.archived_alert_levels.insert(jid.to_string(), level);
		}
	}

	pub fn chat_notify(&self, jid: &str) -> whatsnap::ChatNotify {
		self.chat_notifications.get(jid).cloned().unwrap_or_default()
	}

	pub fn set_chat_notify(&mut self, jid: &str, notify: whatsnap::ChatNotify) {
		if notify == whatsnap::ChatNotify::default() {
			self.chat_notifications.remove(jid);
		} else {
			self.chat_notifications.insert(jid.to_string(), notify);
		}
	}

	/// Whether a new message in the open chat `jid` is read out.
	/// Whether a message in the open chat is read out: received ones when
	/// the chat reads new messages aloud, our own only if asked for too.
	pub fn announces_message(&self, jid: &str, from_me: bool) -> bool {
		(!from_me || self.announce_outgoing) && self.announces_incoming(jid)
	}

	pub fn announces_incoming(&self, jid: &str) -> bool {
		self.chat_notify(jid).announce.unwrap_or(self.announce_incoming)
	}

	/// Turns reading out new messages in `jid` the other way round and says
	/// whether it is now on. A chat that ends up like Settings follows
	/// Settings again.
	pub fn toggle_chat_announce(&mut self, jid: &str) -> bool {
		let on = !self.announces_incoming(jid);
		let mut notify = self.chat_notify(jid);
		notify.announce = (on != self.announce_incoming).then_some(on);
		self.set_chat_notify(jid, notify);
		on
	}

	pub fn message_alert(&self, jid: &str, muted: bool, archived: bool) -> whatsnap::IncomingAlert {
		let base = whatsnap::incoming_alert(muted, archived, self.archived_alert(jid));
		let chat = self.chat_notify(jid).alert;
		let alert = whatsnap::with_chat_alert(base, muted, chat);
		let group = jid.ends_with("@g.us") && chat == whatsnap::ChatAlert::Default;
		whatsnap::IncomingAlert {
			sound: alert.sound
				&& self.sounds_enabled
				&& (!group || self.group_sounds)
				&& self.message_tone_for(jid).0 != whatsnap::TONE_NONE,
			toast: alert.toast && self.notifications_enabled && (!group || self.group_notifications),
		}
	}

	/// The tone a new message in `jid` plays: the chat's own, else the group
	/// or message tone from Settings (WAWebWindowsNotificationToneDropdown's
	/// overrideDefaultTone).
	/// With it, the sound file a [`whatsnap::TONE_CUSTOM`] tone plays.
	pub fn message_tone_for(&self, jid: &str) -> (u8, String) {
		let notify = self.chat_notify(jid);
		match notify.tone {
			whatsnap::TONE_DEFAULT if jid.ends_with("@g.us") => (self.group_tone, self.group_tone_file.clone()),
			whatsnap::TONE_DEFAULT => (self.message_tone, self.message_tone_file.clone()),
			tone => (tone, notify.tone_file),
		}
	}

	/// What a call from `jid` (the group's, for a group call) rings with:
	/// the chat's own ringtone, else the call or group call one.
	pub fn ringtone_for(&self, jid: &str, group: bool) -> String {
		let ringtone = self.chat_notify(jid).ringtone;
		if !ringtone.is_empty() {
			ringtone
		} else if group {
			self.group_call_ringtone_sound.clone()
		} else {
			self.call_ringtone_sound.clone()
		}
	}

	pub fn set_chat_tone(&mut self, jid: &str, tone: u8) -> bool {
		let mut notify = self.chat_notify(jid);
		if notify.tone == tone {
			return false;
		}
		notify.tone = tone;
		self.set_chat_notify(jid, notify);
		true
	}

	pub fn quiet_chats(&self, muted: &std::collections::HashSet<String>) -> std::collections::HashSet<String> {
		let off = self.chat_notifications.iter().filter(|(_, n)| n.alert == whatsnap::ChatAlert::Off);
		muted.iter().cloned().chain(off.map(|(jid, _)| jid.clone())).collect()
	}

	pub fn save(&self) {
		if let Ok(json) = serde_json::to_string_pretty(self) {
			let _ = std::fs::write(settings_path(), json);
		}
		crate::platform::lock_store::save(&crate::platform::lock_store::LockData {
			enabled: self.app_lock,
			hello: self.app_lock_hello,
			hash: self.app_lock_password.clone(),
		});
	}

	pub fn tab_order(&self) -> Vec<usize> {
		let mut seen = [false; TAB_NAMES.len()];
		let valid = self.tab_order.len() == TAB_NAMES.len()
			&& self.tab_order.iter().all(|&t| t < TAB_NAMES.len() && !std::mem::replace(&mut seen[t], true));
		if valid { self.tab_order.clone() } else { DEFAULT_TAB_ORDER.to_vec() }
	}

	/// Null stands for a value WhatsApp can't hold (a custom tone), which is
	/// then not sent.
	pub fn synced(&self) -> Vec<(&'static str, serde_json::Value)> {
		use crate::ui::theme::ThemeChoice;
		let theme = match self.theme {
			ThemeChoice::System => 0,
			ThemeChoice::Light => 1,
			ThemeChoice::Dark => 2,
		};
		vec![
			("isMessagesNotificationEnabled", self.notifications_enabled.into()),
			("defaultNotificationToneId", synced_tone(self.sounds_enabled, self.message_tone)),
			("isGroupMessageNotificationEnabled", self.group_notifications.into()),
			("groupDefaultNotificationToneId", synced_tone(self.group_sounds, self.group_tone)),
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

	pub fn synced_changes(
		&self,
		before: &[(&'static str, serde_json::Value)],
	) -> serde_json::Map<String, serde_json::Value> {
		self.synced()
			.into_iter()
			.zip(before)
			.filter(|((_, now), (_, was))| now != was && !now.is_null())
			.map(|((name, now), _)| (name.to_string(), now))
			.collect()
	}

	pub fn apply_synced(&mut self, name: &str, value: &serde_json::Value) -> bool {
		use crate::ui::theme::ThemeChoice;
		let tone = match name {
			"defaultNotificationToneId" => Some((&mut self.sounds_enabled, &mut self.message_tone)),
			"groupDefaultNotificationToneId" => Some((&mut self.group_sounds, &mut self.group_tone)),
			_ => None,
		};
		if let Some((on, kept)) = tone {
			let Some(tone) = whatsnap::synced_tone(value) else { return false };
			let was = (*on, *kept);
			*on = tone != whatsnap::TONE_NONE;
			if *on {
				*kept = tone;
			}
			return was != (*on, *kept);
		}
		let on = value.as_bool().or_else(|| value.as_i64().map(|tone| tone != 0));
		let Some(on) = on else { return false };
		let field = match name {
			"isMessagesNotificationEnabled" => &mut self.notifications_enabled,
			"isGroupMessageNotificationEnabled" => &mut self.group_notifications,
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
		assert!(other.apply_synced("defaultNotificationToneId", &serde_json::json!(5)));
		assert!(other.sounds_enabled);
		assert_eq!(other.message_tone, 5);
		assert!(!other.apply_synced("defaultNotificationToneId", &serde_json::json!(5)));
		assert!(other.apply_synced("defaultNotificationToneId", &serde_json::json!(0)));
		assert_eq!((other.sounds_enabled, other.message_tone), (false, 5));
		assert!(!other.apply_synced("defaultNotificationToneId", &serde_json::json!(12)));
		other.group_tone = 4;
		assert_eq!(other.synced()[3], ("groupDefaultNotificationToneId", serde_json::json!(4)));
		let before = other.synced();
		other.group_tone = whatsnap::TONE_CUSTOM;
		assert!(other.synced_changes(&before).is_empty());
		assert!(!other.apply_synced("appTheme", &serde_json::json!(9)));
		assert!(!other.apply_synced("wallpaperId", &serde_json::json!(true)));
	}

	#[test]
	fn a_chat_tone_beats_the_group_and_message_tones() {
		let mut s = Settings { message_tone: 3, group_tone: 4, ..Settings::default() };
		assert_eq!(s.message_tone_for("1@s.whatsapp.net").0, 3);
		assert_eq!(s.message_tone_for("2@g.us").0, 4);
		assert!(s.set_chat_tone("2@g.us", 9));
		assert!(!s.set_chat_tone("2@g.us", 9));
		assert_eq!(s.message_tone_for("2@g.us").0, 9);
		assert!(s.message_alert("2@g.us", false, false).sound);
		s.set_chat_tone("2@g.us", whatsnap::TONE_NONE);
		assert!(!s.message_alert("2@g.us", false, false).sound);
		assert!(s.message_alert("2@g.us", false, false).toast);
		s.set_chat_tone("2@g.us", whatsnap::TONE_DEFAULT);
		assert!(s.chat_notifications.is_empty());
	}

	#[test]
	fn a_chat_ringtone_beats_the_call_and_group_call_ringtones() {
		let mut s = Settings { group_call_ringtone_sound: "g.wav".into(), ..Settings::default() };
		assert_eq!(s.ringtone_for("1@s.whatsapp.net", false), "");
		assert_eq!(s.ringtone_for("2@g.us", true), "g.wav");
		let notify = whatsnap::ChatNotify { ringtone: whatsnap::RINGTONE_NONE.into(), ..Default::default() };
		s.set_chat_notify("2@g.us", notify);
		assert_eq!(s.ringtone_for("2@g.us", true), whatsnap::RINGTONE_NONE);
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
