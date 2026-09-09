//! User-configurable app preferences, persisted as JSON next to the session
//! database (see [`crate::paths::app_data_dir`]).

use std::path::PathBuf;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
	pub sounds_enabled: bool,
	pub notifications_enabled: bool,
	pub announce_typing: bool,
	pub announce_recording: bool,
	pub show_tray_icon: bool,
	/// Friendly name of the preferred microphone for calls, or `None` for the
	/// system default communications device. Also used for voice-message
	/// recording (see [`crate::voice_recorder`]), which honors this device.
	pub input_device_name: Option<String>,
	/// Friendly name of the preferred speaker/output device for calls and for
	/// voice-message/notification playback, or `None` for the system default.
	pub output_device_name: Option<String>,
	/// Whether recording a voice message applies Windows' communications
	/// audio-effects pipeline (echo cancellation/noise-suppression/AGC) - the
	/// same processing calls always use. Off by default: a voice message has
	/// no speaker output to echo-cancel against, and some prefer the raw mic
	/// signal over what the effects pipeline does to it.
	pub voice_message_noise_suppression: bool,
	/// Per-chat alert level for archived chats, keyed by JID. A chat with no
	/// entry uses [`whantv::ArchivedAlert::default`] (silent), so archiving
	/// quiets a chat with no bookkeeping at all. Entries deliberately survive
	/// unarchiving: re-archiving a chat shouldn't silently discard the choice
	/// made for it last time.
	pub archived_alert_levels: std::collections::HashMap<String, whantv::ArchivedAlert>,
}

impl Default for Settings {
	fn default() -> Self {
		Self {
			sounds_enabled: true,
			notifications_enabled: true,
			announce_typing: true,
			announce_recording: true,
			show_tray_icon: true,
			input_device_name: None,
			output_device_name: None,
			voice_message_noise_suppression: false,
			archived_alert_levels: std::collections::HashMap::new(),
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
	/// never consult this - see [`whantv::incoming_alert`].
	pub fn archived_alert(&self, jid: &str) -> whantv::ArchivedAlert {
		self.archived_alert_levels.get(jid).copied().unwrap_or_default()
	}

	/// Records `level` for `jid`, dropping the entry when it's the default so
	/// settings.json doesn't grow a line for every archived chat left alone.
	pub fn set_archived_alert(&mut self, jid: &str, level: whantv::ArchivedAlert) {
		if level == whantv::ArchivedAlert::default() {
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
}
