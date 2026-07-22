use serde::{Deserialize, Serialize};
use wxdragon::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
	pub port: u16,
	pub server: String,
	pub ws_server: String,
	pub api_key: String,
	pub sounds_enabled: bool,
	pub notifications_enabled: bool,
	pub autostart: bool,
	pub tray_icon: bool,
	pub updates: bool,
	pub hotkey: String,
	pub messages_page_size: u32,
	pub announce_typing: bool,
	pub announce_recording: bool,
	pub custom_api: bool,
	pub audio_speed: String,
}

impl Default for AppConfig {
	fn default() -> Self {
		Self {
			port: 9000,
			server: "http://localhost:9000".to_string(),
			ws_server: "ws://localhost:9000/ws".to_string(),
			api_key: String::new(),
			sounds_enabled: true,
			notifications_enabled: true,
			autostart: false,
			tray_icon: true,
			updates: true,
			hotkey: "Ctrl+Alt+W".to_string(),
			messages_page_size: 50,
			announce_typing: true,
			announce_recording: true,
			custom_api: false,
			audio_speed: "1.0x".to_string(),
		}
	}
}

impl AppConfig {
	pub fn path() -> std::path::PathBuf {
		if let Ok(exe_path) = std::env::current_exe() {
			if let Some(parent) = exe_path.parent() {
				let cfg_in_exe = parent.join("config.json");
				if cfg_in_exe.exists() {
					return cfg_in_exe;
				}
			}
		}
		std::path::PathBuf::from("config.json")
	}

	pub fn load() -> Self {
		let path = Self::path();
		if path.exists() {
			if let Ok(content) = std::fs::read_to_string(&path) {
				if let Ok(cfg) = serde_json::from_str::<AppConfig>(&content) {
					return cfg;
				}
			}
		}
		let cfg = Self::default();
		let _ = cfg.save();
		cfg
	}

	pub fn save(&self) -> Result<(), std::io::Error> {
		let path = Self::path();
		let json = serde_json::to_string_pretty(self)?;
		std::fs::write(path, json)
	}
}

pub struct SettingsDialogParts {
	pub dialog: Dialog,
	pub notebook: Notebook,

	pub general_page: Panel,
	pub sounds_check: CheckBox,
	pub notifications_check: CheckBox,
	pub autostart_check: CheckBox,
	pub tray_icon_check: CheckBox,
	pub updates_check: CheckBox,
	pub hotkey_field: TextCtrl,

	pub ui_page: Panel,
	pub messages_page_size_field: TextCtrl,
	pub focus_message_field_rb: RadioButton,
	pub focus_unread_or_last_rb: RadioButton,
	pub voice_focus_send_rb: RadioButton,
	pub voice_focus_discard_rb: RadioButton,
	pub msg_list_mode_classic_rb: RadioButton,
	pub msg_list_mode_listbox_rb: RadioButton,
	pub self_ref_eu_rb: RadioButton,
	pub self_ref_voce_rb: RadioButton,
	pub self_ref_other_rb: RadioButton,
	pub self_ref_custom_field: TextCtrl,

	pub speech_page: Panel,
	pub announce_typing_check: CheckBox,
	pub announce_recording_check: CheckBox,

	pub conn_page: Panel,
	pub custom_api_check: CheckBox,
	pub server_field: TextCtrl,
	pub ws_server_field: TextCtrl,
	pub port_field: TextCtrl,
	pub api_key_field: TextCtrl,

	pub audio_page: Panel,
	pub audio_speed_combo: ComboBox,

	pub ok_btn: Button,
	pub cancel_btn: Button,
	pub apply_btn: Button,
}

impl SettingsDialogParts {
	pub fn populate(&self, config: &AppConfig) {
		self.port_field.set_value(&config.port.to_string());
		self.server_field.set_value(&config.server);
		self.ws_server_field.set_value(&config.ws_server);
		self.api_key_field.set_value(&config.api_key);
		self.sounds_check.set_value(config.sounds_enabled);
		self.notifications_check.set_value(config.notifications_enabled);
		self.autostart_check.set_value(config.autostart);
		self.tray_icon_check.set_value(config.tray_icon);
		self.updates_check.set_value(config.updates);
		self.hotkey_field.set_value(&config.hotkey);
		self.messages_page_size_field.set_value(&config.messages_page_size.to_string());
		self.announce_typing_check.set_value(config.announce_typing);
		self.announce_recording_check.set_value(config.announce_recording);
		self.custom_api_check.set_value(config.custom_api);
	}

	pub fn read_config(&self) -> AppConfig {
		let port = self.port_field.get_value().trim().parse::<u16>().unwrap_or(9000);
		let messages_page_size = self.messages_page_size_field.get_value().trim().parse::<u32>().unwrap_or(50);
		AppConfig {
			port,
			server: self.server_field.get_value().trim().to_string(),
			ws_server: self.ws_server_field.get_value().trim().to_string(),
			api_key: self.api_key_field.get_value().trim().to_string(),
			sounds_enabled: self.sounds_check.get_value(),
			notifications_enabled: self.notifications_check.get_value(),
			autostart: self.autostart_check.get_value(),
			tray_icon: self.tray_icon_check.get_value(),
			updates: self.updates_check.get_value(),
			hotkey: self.hotkey_field.get_value().trim().to_string(),
			messages_page_size,
			announce_typing: self.announce_typing_check.get_value(),
			announce_recording: self.announce_recording_check.get_value(),
			custom_api: self.custom_api_check.get_value(),
			audio_speed: "1.0x".to_string(),
		}
	}
}

pub fn build_settings_dialog(parent: &impl WxWidget) -> SettingsDialogParts {
	let dialog = Dialog::builder(parent, "Settings")
		.with_size(500, 520)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();

	let notebook = Notebook::builder(&dialog).build();

	let general_page = Panel::builder(&notebook).build();
	let gen_sizer = BoxSizer::builder(Orientation::Vertical).build();

	let sounds_check = CheckBox::builder(&general_page).with_label("Notification sounds").build();
	gen_sizer.add(&sounds_check, 0, SizerFlag::All, 8);

	let notifications_check = CheckBox::builder(&general_page).with_label("Desktop notifications").build();
	gen_sizer.add(&notifications_check, 0, SizerFlag::All, 8);

	let autostart_check = CheckBox::builder(&general_page).with_label("Start with Windows").build();
	gen_sizer.add(&autostart_check, 0, SizerFlag::All, 8);

	let tray_icon_check = CheckBox::builder(&general_page).with_label("Show tray icon").build();
	gen_sizer.add(&tray_icon_check, 0, SizerFlag::All, 8);

	let updates_check = CheckBox::builder(&general_page).with_label("Check for updates").build();
	gen_sizer.add(&updates_check, 0, SizerFlag::All, 8);

	let hotkey_label = StaticText::builder(&general_page).with_label("Global hotkey").build();
	gen_sizer.add(&hotkey_label, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Right, 8);
	let hotkey_field = TextCtrl::builder(&general_page)
		.with_style(TextCtrlStyle::ProcessEnter)
		.build();
	gen_sizer.add(&hotkey_field, 0, SizerFlag::Expand | SizerFlag::All, 8);

	general_page.set_sizer(gen_sizer, true);
	notebook.add_page(&general_page, "General", true, None);

	let ui_page = Panel::builder(&notebook).build();
	let ui_sizer = BoxSizer::builder(Orientation::Vertical).build();

	let page_size_label = StaticText::builder(&ui_page).with_label("Messages per page").build();
	ui_sizer.add(&page_size_label, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Right, 8);
	let messages_page_size_field = TextCtrl::builder(&ui_page).build();
	ui_sizer.add(&messages_page_size_field, 0, SizerFlag::Expand | SizerFlag::All, 8);

	let focus_sizer = StaticBoxSizerBuilder::new_with_label(Orientation::Vertical, &ui_page, "Focus on conversation open").build();
	let focus_box = focus_sizer.get_static_box().unwrap();
	let focus_message_field_rb = RadioButton::builder(&focus_box)
		.with_label("Message field")
		.with_style(RadioButtonStyle::GroupStart)
		.build();
	focus_sizer.add(&focus_message_field_rb, 0, SizerFlag::Left | SizerFlag::Top, 5);
	let focus_unread_or_last_rb = RadioButton::builder(&focus_box)
		.with_label("Unread or last message")
		.build();
	focus_sizer.add(&focus_unread_or_last_rb, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Bottom, 5);
	ui_sizer.add_sizer(&focus_sizer, 0, SizerFlag::Expand | SizerFlag::All, 8);

	let voice_focus_sizer = StaticBoxSizerBuilder::new_with_label(Orientation::Vertical, &ui_page, "After recording voice message").build();
	let voice_focus_box = voice_focus_sizer.get_static_box().unwrap();
	let voice_focus_send_rb = RadioButton::builder(&voice_focus_box)
		.with_label("Focus Send button")
		.with_style(RadioButtonStyle::GroupStart)
		.build();
	voice_focus_sizer.add(&voice_focus_send_rb, 0, SizerFlag::Left | SizerFlag::Top, 5);
	let voice_focus_discard_rb = RadioButton::builder(&voice_focus_box)
		.with_label("Focus Discard button")
		.build();
	voice_focus_sizer.add(&voice_focus_discard_rb, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Bottom, 5);
	ui_sizer.add_sizer(&voice_focus_sizer, 0, SizerFlag::Expand | SizerFlag::All, 8);

	let msg_list_sizer = StaticBoxSizerBuilder::new_with_label(Orientation::Vertical, &ui_page, "Message list mode").build();
	let msg_list_box = msg_list_sizer.get_static_box().unwrap();
	let msg_list_mode_classic_rb = RadioButton::builder(&msg_list_box)
		.with_label("Classic")
		.with_style(RadioButtonStyle::GroupStart)
		.build();
	msg_list_sizer.add(&msg_list_mode_classic_rb, 0, SizerFlag::Left | SizerFlag::Top, 5);
	let msg_list_mode_listbox_rb = RadioButton::builder(&msg_list_box)
		.with_label("List box")
		.build();
	msg_list_sizer.add(&msg_list_mode_listbox_rb, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Bottom, 5);
	ui_sizer.add_sizer(&msg_list_sizer, 0, SizerFlag::Expand | SizerFlag::All, 8);

	let self_ref_sizer = StaticBoxSizerBuilder::new_with_label(Orientation::Vertical, &ui_page, "Self reference").build();
	let self_ref_box = self_ref_sizer.get_static_box().unwrap();
	let self_ref_eu_rb = RadioButton::builder(&self_ref_box)
		.with_label("You")
		.with_style(RadioButtonStyle::GroupStart)
		.build();
	self_ref_sizer.add(&self_ref_eu_rb, 0, SizerFlag::Left | SizerFlag::Top, 5);
	let self_ref_voce_rb = RadioButton::builder(&self_ref_box)
		.with_label("Voce")
		.build();
	self_ref_sizer.add(&self_ref_voce_rb, 0, SizerFlag::Left | SizerFlag::Top, 5);
	let self_ref_other_rb = RadioButton::builder(&self_ref_box)
		.with_label("Other")
		.build();
	self_ref_sizer.add(&self_ref_other_rb, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Bottom, 5);
	let self_ref_custom_label = StaticText::builder(&self_ref_box).with_label("Custom word").build();
	self_ref_sizer.add(&self_ref_custom_label, 0, SizerFlag::Left | SizerFlag::Right, 8);
	let self_ref_custom_field = TextCtrl::builder(&self_ref_box).build();
	self_ref_sizer.add(&self_ref_custom_field, 0, SizerFlag::Expand | SizerFlag::All, 8);
	ui_sizer.add_sizer(&self_ref_sizer, 0, SizerFlag::Expand | SizerFlag::All, 8);

	ui_page.set_sizer(ui_sizer, true);
	notebook.add_page(&ui_page, "User Interface", false, None);

	let speech_page = Panel::builder(&notebook).build();
	let speech_sizer = BoxSizer::builder(Orientation::Vertical).build();

	let announce_typing_check = CheckBox::builder(&speech_page).with_label("Announce typing").build();
	speech_sizer.add(&announce_typing_check, 0, SizerFlag::All, 8);

	let announce_recording_check = CheckBox::builder(&speech_page).with_label("Announce recording").build();
	speech_sizer.add(&announce_recording_check, 0, SizerFlag::All, 8);

	speech_page.set_sizer(speech_sizer, true);
	notebook.add_page(&speech_page, "Spoken Content", false, None);

	let conn_page = Panel::builder(&notebook).build();
	let conn_sizer = BoxSizer::builder(Orientation::Vertical).build();

	let custom_api_check = CheckBox::builder(&conn_page).with_label("Use custom API server").build();
	conn_sizer.add(&custom_api_check, 0, SizerFlag::All, 8);

	let server_label = StaticText::builder(&conn_page).with_label("Server URL").build();
	conn_sizer.add(&server_label, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Right, 8);
	let server_field = TextCtrl::builder(&conn_page).build();
	conn_sizer.add(&server_field, 0, SizerFlag::Expand | SizerFlag::All, 8);

	let ws_server_label = StaticText::builder(&conn_page).with_label("WebSocket server URL").build();
	conn_sizer.add(&ws_server_label, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Right, 8);
	let ws_server_field = TextCtrl::builder(&conn_page).build();
	conn_sizer.add(&ws_server_field, 0, SizerFlag::Expand | SizerFlag::All, 8);

	let port_label = StaticText::builder(&conn_page).with_label("Port").build();
	conn_sizer.add(&port_label, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Right, 8);
	let port_field = TextCtrl::builder(&conn_page).build();
	conn_sizer.add(&port_field, 0, SizerFlag::Expand | SizerFlag::All, 8);

	let api_key_label = StaticText::builder(&conn_page).with_label("API key").build();
	conn_sizer.add(&api_key_label, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Right, 8);
	let api_key_field = TextCtrl::builder(&conn_page).build();
	conn_sizer.add(&api_key_field, 0, SizerFlag::Expand | SizerFlag::All, 8);

	conn_page.set_sizer(conn_sizer, true);
	notebook.add_page(&conn_page, "Connection", false, None);

	let audio_page = Panel::builder(&notebook).build();
	let audio_sizer = BoxSizer::builder(Orientation::Vertical).build();

	let audio_speed_label = StaticText::builder(&audio_page).with_label("Playback speed").build();
	audio_sizer.add(&audio_speed_label, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Right, 8);
	let audio_speed_combo = ComboBox::builder(&audio_page)
		.with_style(ComboBoxStyle::ReadOnly)
		.with_choices(vec!["1.0x".to_string(), "1.5x".to_string(), "2.0x".to_string()])
		.build();
	audio_sizer.add(&audio_speed_combo, 0, SizerFlag::Expand | SizerFlag::All, 8);

	audio_page.set_sizer(audio_sizer, true);
	notebook.add_page(&audio_page, "Audio Playback", false, None);

	let main_sizer = BoxSizer::builder(Orientation::Vertical).build();
	main_sizer.add(&notebook, 1, SizerFlag::Expand | SizerFlag::All, 5);

	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let ok_btn = Button::builder(&dialog).with_id(ID_OK).with_label("OK").build();
	let cancel_btn = Button::builder(&dialog).with_id(ID_CANCEL).with_label("Cancel").build();
	let apply_btn = Button::builder(&dialog).with_label("Apply").build();
	btn_sizer.add(&ok_btn, 0, SizerFlag::Right, 8);
	btn_sizer.add(&cancel_btn, 0, SizerFlag::Right, 8);
	btn_sizer.add_stretch_spacer(1);
	btn_sizer.add(&apply_btn, 0, SizerFlag::empty(), 0);

	main_sizer.add_sizer(&btn_sizer, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom, 8);
	dialog.set_sizer(main_sizer, true);
	dialog.center();

	SettingsDialogParts {
		dialog,
		notebook,
		general_page,
		sounds_check,
		notifications_check,
		autostart_check,
		tray_icon_check,
		updates_check,
		hotkey_field,
		ui_page,
		messages_page_size_field,
		focus_message_field_rb,
		focus_unread_or_last_rb,
		voice_focus_send_rb,
		voice_focus_discard_rb,
		msg_list_mode_classic_rb,
		msg_list_mode_listbox_rb,
		self_ref_eu_rb,
		self_ref_voce_rb,
		self_ref_other_rb,
		self_ref_custom_field,
		speech_page,
		announce_typing_check,
		announce_recording_check,
		conn_page,
		custom_api_check,
		server_field,
		ws_server_field,
		port_field,
		api_key_field,
		audio_page,
		audio_speed_combo,
		ok_btn,
		cancel_btn,
		apply_btn,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_app_config_default_and_serialization() {
		let default_cfg = AppConfig::default();
		assert_eq!(default_cfg.port, 9000);
		assert_eq!(default_cfg.messages_page_size, 50);

		let serialized = serde_json::to_string(&default_cfg).unwrap();
		let deserialized: AppConfig = serde_json::from_str(&serialized).unwrap();
		assert_eq!(deserialized.port, default_cfg.port);
		assert_eq!(deserialized.ws_server, default_cfg.ws_server);
	}
}

