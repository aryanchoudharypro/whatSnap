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
}

impl Default for Settings {
	fn default() -> Self {
		Self {
			sounds_enabled: true,
			notifications_enabled: true,
			announce_typing: true,
			announce_recording: true,
			show_tray_icon: true,
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

	pub fn save(&self) {
		if let Ok(json) = serde_json::to_string_pretty(self) {
			let _ = std::fs::write(settings_path(), json);
		}
	}
}
