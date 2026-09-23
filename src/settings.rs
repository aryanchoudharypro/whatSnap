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
	/// entry uses [`whantv::ArchivedAlert::default`] (silent), so archiving
	/// quiets a chat with no bookkeeping at all. Entries deliberately survive
	/// unarchiving: re-archiving a chat shouldn't silently discard the choice
	/// made for it last time.
	pub archived_alert_levels: std::collections::HashMap<String, whantv::ArchivedAlert>,
	/// Whether the backend logs every WhatsApp protocol frame and SQL query
	/// (DEBUG level) instead of just warnings/errors. Off by default: it's
	/// continuous, high-volume logging only useful while actively
	/// troubleshooting a connection issue. Read once at startup when the
	/// backend process is launched - toggling it takes effect the next time
	/// whaNTV starts, not on the running backend.
	pub verbose_backend_logging: bool,
}

impl Default for Settings {
	fn default() -> Self {
		Self {
			sounds_enabled: true,
			notifications_enabled: true,
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
