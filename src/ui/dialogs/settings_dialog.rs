use wxdragon::prelude::*;

use crate::{
	audio::devices::DEFAULT_LABEL,
	ui::{
		dialog_look::{self, Field},
		sound_picker::{Kind, SoundPicker},
		theme,
	},
};

pub const VISIBILITY_CHOICES: &[(&str, &str)] =
	&[("Everyone", "all"), ("My Contacts", "contacts"), ("My Contacts Except...", EXCEPT_VALUE), ("Nobody", "none")];
pub const EXCEPT_VALUE: &str = "contact_blacklist";
pub const ONLINE_CHOICES: &[(&str, &str)] = &[("Everyone", "all"), ("Same as Last Seen", "match_last_seen")];
/// "calladd" is WhatsApp's "Silence unknown callers": with "known", calls
/// from numbers that aren't contacts still come in, they just don't ring.
/// Nobody is stopped from calling, so it isn't "who can call me".
pub const CALL_CHOICES: &[(&str, &str)] = &[("Off", "all"), ("On", "known")];
pub const DEFAULT_TIMER_CHOICES: &[(&str, &str)] =
	&[("Off", "0"), ("24 Hours", "86400"), ("7 Days", "604800"), ("90 Days", "7776000")];
pub const UPLOAD_QUALITY_CHOICES: &[(&str, &str)] = &[("Standard Quality", "sd"), ("HD Quality", "hd")];
pub const DOWNLOAD_QUALITY_CHOICES: &[(&str, &str)] = &[("Standard Quality", "sd"), ("HD Quality", "hd")];
pub const UPDATE_CHANNEL_CHOICES: &[(&str, &str)] = &[("Stable releases", "stable"), ("Development builds", "dev")];
pub const THEME_CHOICES: &[(&str, &str)] = &[("System Default", "system"), ("Light", "light"), ("Dark", "dark")];
pub const ABOUT_DURATION_CHOICES: &[(&str, &str)] = &[
	("Never", "-1"),
	("30 Minutes", "1800"),
	("1 Hour", "3600"),
	("2 Hours", "7200"),
	("4 Hours", "14400"),
	("8 Hours", "28800"),
	("24 Hours", "86400"),
];

pub const APP_LOCK_METHOD_CHOICES: &[(&str, &str)] = &[("Windows Hello", "hello"), ("Password", "password")];
pub const CHAT_LOCK_HELLO_LABEL: &str =
	"Unlock locked chats with Windows Hello (fingerprint, face or PIN) instead of the secret code";

pub struct PrivacyControl {
	pub name: &'static str,
	pub label: &'static str,
	pub combo: ComboBox,
	pub choices: &'static [(&'static str, &'static str)],
	pub except_btn: Option<Button>,
}

impl PrivacyControl {
	pub fn is_except(&self) -> bool {
		self.except_btn.is_some() && combo_value(&self.combo, self.choices) == EXCEPT_VALUE
	}
}

#[allow(dead_code)]
#[derive(Clone, Copy)]
pub struct SettingsDialogParts {
	pub dialog: Dialog,
	pub notebook: Notebook,

	pub general_panel: Panel,
	pub announce_typing_check: CheckBox,
	pub announce_recording_check: CheckBox,
	pub tray_icon_check: CheckBox,
	pub autostart_check: CheckBox,
	pub start_maximized_check: CheckBox,
	pub check_updates_check: CheckBox,
	pub update_channel_combo: ComboBox,
	pub meta_ai_enabled_check: CheckBox,
	pub verbose_backend_logging_check: CheckBox,
	pub shortcuts_btn: Button,
	pub share_menu_btn: Button,

	pub notifications_panel: Panel,
	pub sounds_check: CheckBox,
	pub notifications_check: CheckBox,
	pub notification_previews_check: CheckBox,
	pub reaction_notifications_check: CheckBox,
	pub group_notifications_check: CheckBox,
	pub group_sounds_check: CheckBox,
	pub message_tone_picker: SoundPicker,
	pub group_tone_picker: SoundPicker,
	pub outgoing_sounds_check: CheckBox,
	pub call_notifications_check: CheckBox,
	pub call_ringtone_check: CheckBox,
	pub call_ringtone_picker: SoundPicker,
	pub group_call_ringtone_picker: SoundPicker,
	pub follow_dnd_check: CheckBox,
	pub announce_incoming_check: CheckBox,
	pub announce_outgoing_check: CheckBox,

	pub chats_panel: Panel,
	pub enter_is_send_check: CheckBox,
	pub type_to_compose_check: CheckBox,
	pub keep_list_position_check: CheckBox,
	pub chat_lock_hello_check: CheckBox,
	pub app_lock_check: CheckBox,
	pub app_lock_method_combo: ComboBox,
	pub app_lock_password_btn: Button,
	pub replace_emoji_check: CheckBox,
	pub emoji_help_btn: Button,
	pub spell_check_check: CheckBox,
	pub spell_announce_check: CheckBox,
	pub spell_language_combo: ComboBox,
	pub auto_download_photos_check: CheckBox,
	pub auto_download_audio_check: CheckBox,
	pub auto_download_videos_check: CheckBox,
	pub auto_download_documents_check: CheckBox,
	pub download_quality_combo: ComboBox,
	pub upload_quality_combo: ComboBox,
	pub theme_combo: ComboBox,

	pub privacy_panel: Panel,
	pub name_field: TextCtrl,
	pub about_field: TextCtrl,
	pub about_duration_combo: ComboBox,
	pub change_photo_btn: Button,
	pub remove_photo_btn: Button,
	pub default_timer_combo: ComboBox,
	pub last_seen_combo: ComboBox,
	pub last_seen_except_btn: Button,
	pub profile_photo_combo: ComboBox,
	pub profile_photo_except_btn: Button,
	pub about_privacy_combo: ComboBox,
	pub about_privacy_except_btn: Button,
	pub read_receipts_check: CheckBox,
	pub groups_combo: ComboBox,
	pub groups_except_btn: Button,
	pub calls_combo: ComboBox,
	pub online_combo: ComboBox,
	pub link_previews_off_check: CheckBox,
	pub security_notifications_check: CheckBox,

	pub audio_panel: Panel,
	pub input_device_combo: ComboBox,
	pub output_device_combo: ComboBox,
	pub call_output_device_combo: ComboBox,
	pub voice_message_noise_suppression_check: CheckBox,
	pub dual_stream_screen_share_check: CheckBox,

	pub ok_btn: Button,
	pub cancel_btn: Button,
}

fn visibility_combo(panel: &Panel, sizer: &BoxSizer, label: &str, choices: &[(&str, &str)]) -> ComboBox {
	let text = StaticText::builder(panel).with_label(label).build();
	text.set_font(&theme::Fonts::body());
	sizer.add(&text, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);

	let labels: Vec<&str> = choices.iter().map(|(l, _)| *l).collect();
	let combo = ComboBox::builder(panel).with_string_choices(&labels).with_style(ComboBoxStyle::ReadOnly).build();
	combo.set_font(&theme::Fonts::body());
	combo.set_selection(0);
	sizer.add(&combo, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);
	sizer.add_spacer(4);
	combo
}

fn spelling_language_combo(panel: &Panel, sizer: &BoxSizer) -> ComboBox {
	let languages = crate::ui::spell_check::languages();
	let mut labels: Vec<&str> = vec!["Windows default"];
	labels.extend(languages.iter().map(|(_, name)| name.as_str()));
	let text = StaticText::builder(panel).with_label("Spelling Language").build();
	text.set_font(&theme::Fonts::body());
	sizer.add(&text, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);
	let combo = ComboBox::builder(panel).with_string_choices(&labels).with_style(ComboBoxStyle::ReadOnly).build();
	combo.set_font(&theme::Fonts::body());
	combo.set_selection(0);
	sizer.add(&combo, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);
	sizer.add_spacer(4);
	combo
}

fn setting_switch(panel: &Panel, sizer: &BoxSizer, label: &str) -> CheckBox {
	let check = CheckBox::builder(panel).with_label(label).build();
	dialog_look::switch(&check, theme::Palette::CONTENT_BG());
	sizer.add(&check, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);
	check
}

fn setting_button(panel: &Panel, label: &str) -> Button {
	let btn = Button::builder(panel).with_label(label).build();
	dialog_look::pill_button(&btn, false, theme::Palette::CONTENT_BG());
	btn
}

/// The "Choose Excluded Contacts..." button under a dropdown that has
/// "My Contacts Except...". It's named for its setting, since a screen reader
/// reads buttons on their own.
fn except_button(panel: &Panel, sizer: &BoxSizer, setting: &str) -> Button {
	let btn = setting_button(panel, &format!("Choose Contacts Excluded From {}...", setting));
	btn.enable(false);
	sizer.add(&btn, 0, SizerFlag::Left | SizerFlag::Bottom, 12);
	btn
}

pub fn build_settings_dialog(parent: &impl WxWidget) -> SettingsDialogParts {
	let dialog = Dialog::builder(parent, "Settings")
		.with_size(600, 680)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);

	let main_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let notebook = Notebook::builder(&dialog).build();

	let general_panel = Panel::builder(&notebook).build();
	theme::style_content(&general_panel);
	let general_sizer = BoxSizer::builder(Orientation::Vertical).build();

	let announce_typing_check = setting_switch(&general_panel, &general_sizer, "Announce when someone is typing");

	let announce_recording_check =
		setting_switch(&general_panel, &general_sizer, "Announce when someone is recording a voice message");

	let tray_icon_check = setting_switch(
		&general_panel,
		&general_sizer,
		"Show an icon in the notification area; closing the window minimizes to it",
	);

	let autostart_check = setting_switch(&general_panel, &general_sizer, "Start whatSnap when Windows starts");

	let start_maximized_check = setting_switch(&general_panel, &general_sizer, "Start maximized");

	let check_updates_check = setting_switch(&general_panel, &general_sizer, "Check for updates when whatSnap starts");
	let update_channel_combo = visibility_combo(&general_panel, &general_sizer, "Updates", UPDATE_CHANNEL_CHOICES);

	let meta_ai_enabled_check =
		setting_switch(&general_panel, &general_sizer, "Show a button to message Meta AI in New Chat");

	let verbose_backend_logging_check = setting_switch(
		&general_panel,
		&general_sizer,
		"Verbose backend logging, for troubleshooting connection issues (takes effect next time whatSnap starts)",
	);

	let shortcuts_btn = setting_button(&general_panel, "&Keyboard Shortcuts...");
	general_sizer.add(&shortcuts_btn, 0, SizerFlag::Left | SizerFlag::Top, 16);

	let share_menu_btn = setting_button(&general_panel, "Add whatSnap to the Windows Share Menu...");
	general_sizer.add(&share_menu_btn, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Bottom, 12);

	general_panel.set_sizer(general_sizer, true);

	let notifications_panel = Panel::builder(&notebook).build();
	theme::style_content(&notifications_panel);
	let notifications_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let notify_switch = |label: &str| setting_switch(&notifications_panel, &notifications_sizer, label);

	let notifications_check = notify_switch("Message notifications: show desktop notifications for new messages");
	let notification_previews_check = notify_switch("Show previews: message text in notifications");
	let reaction_notifications_check =
		notify_switch("Reaction notifications: show notifications for reactions to your messages");
	let sounds_check = notify_switch("Incoming sounds: play a sound for new messages");
	let sound_picker = |label: &str, what: &str, kind: Kind| {
		SoundPicker::new(&notifications_panel, &notifications_sizer, label, what, kind, "Default")
	};
	let message_tone_picker = sound_picker("Message notification tone", "Message Tone", Kind::Tone);
	let outgoing_sounds_check = notify_switch("Outgoing sounds: play sounds for messages you send");
	let group_notifications_check = notify_switch("Group notifications: show notifications for new group messages");
	let group_sounds_check = notify_switch("Group sounds: play sounds for new group messages");
	let group_tone_picker = sound_picker("Group notification tone", "Group Tone", Kind::Tone);
	let call_notifications_check = notify_switch("Call notifications: show notifications for incoming calls");
	let call_ringtone_check = notify_switch("Incoming call sounds: play the ringtone for incoming calls");
	let call_ringtone_picker = sound_picker("Call ringtone", "Call Ringtone", Kind::Ringtone);
	let group_call_ringtone_picker = sound_picker("Group call ringtone", "Group Call Ringtone", Kind::Ringtone);
	let follow_dnd_check =
		notify_switch("No message sound while Windows Do Not Disturb, a full-screen game or a presentation is on");
	let announce_incoming_check =
		notify_switch("Read new messages aloud in the open chat (Alt+L turns it on or off for one chat)");
	let announce_outgoing_check = notify_switch("Also read your own sent messages aloud");
	notifications_sizer.add_spacer(12);
	notifications_panel.set_sizer(notifications_sizer, true);

	let chats_panel = Panel::builder(&notebook).build();
	theme::style_content(&chats_panel);
	let chats_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let chats_check = |label: &str| setting_switch(&chats_panel, &chats_sizer, label);
	let enter_is_send_check = chats_check("Enter is send: the Enter key sends your message");
	let type_to_compose_check =
		chats_check("Typing a letter in the message list starts a message (message shortcuts then need Alt)");
	let keep_list_position_check =
		chats_check("Keep my place in the message list instead of jumping to the newest message");
	let chat_lock_hello_check = chats_check(CHAT_LOCK_HELLO_LABEL);
	let replace_emoji_check = chats_check("Replace text with emoji as you type, like :-) or (y)");
	let emoji_help_btn = setting_button(&chats_panel, "Replace Text With Emoji: See Help...");
	chats_sizer.add(&emoji_help_btn, 0, SizerFlag::Left | SizerFlag::Top, 12);
	let spell_check_check = chats_check("Check spelling in the message box (F7 goes to the next mistake)");
	let spell_announce_check = chats_check("Say when a word you type is misspelled");
	let spell_language_combo = spelling_language_combo(&chats_panel, &chats_sizer);
	let auto_download_photos_check = chats_check("Download photos automatically");
	let auto_download_audio_check = chats_check("Download audio automatically");
	let auto_download_videos_check = chats_check("Download videos automatically");
	let auto_download_documents_check = chats_check("Download documents automatically");
	let auto_download_note = StaticText::builder(&chats_panel)
		.with_label("Voice messages are always downloaded. Other media downloads by itself only from contacts and chats you've written in.")
		.build();
	auto_download_note.set_font(&theme::Fonts::body());
	chats_sizer.add(&auto_download_note, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);
	let download_quality_combo = visibility_combo(
		&chats_panel,
		&chats_sizer,
		"Download Quality for Photos and Videos Sent in HD",
		DOWNLOAD_QUALITY_CHOICES,
	);
	let upload_quality_combo =
		visibility_combo(&chats_panel, &chats_sizer, "Media Upload Quality", UPLOAD_QUALITY_CHOICES);
	let theme_combo = visibility_combo(&chats_panel, &chats_sizer, "Theme", THEME_CHOICES);
	chats_panel.set_sizer(chats_sizer, true);

	let lock_panel = Panel::builder(&notebook).build();
	theme::style_content(&lock_panel);
	let lock_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let app_lock_check = setting_switch(
		&lock_panel,
		&lock_sizer,
		"Lock whatSnap when it starts, and whenever I press Ctrl+Shift+L (Account, Lock App)",
	);
	let app_lock_method_combo = visibility_combo(&lock_panel, &lock_sizer, "Unlock with", APP_LOCK_METHOD_CHOICES);
	let app_lock_password_btn = setting_button(&lock_panel, "Set App Lock Password...");
	lock_sizer.add(&app_lock_password_btn, 0, SizerFlag::Left | SizerFlag::Top, 12);
	lock_panel.set_sizer(lock_sizer, true);

	let privacy_panel = Panel::builder(&notebook).build();
	theme::style_content(&privacy_panel);
	let privacy_sizer = BoxSizer::builder(Orientation::Vertical).build();

	let name_label = StaticText::builder(&privacy_panel).with_label("Name").build();
	name_label.set_font(&theme::Fonts::body());
	privacy_sizer.add(&name_label, 0, SizerFlag::Left | SizerFlag::Top, 12);
	let name_field =
		TextCtrl::builder(&privacy_panel).with_style(dialog_look::field_style(TextCtrlStyle::Default)).build();
	dialog_look::add_field(&privacy_sizer, &name_field, Field::Line, 0);

	let about_label = StaticText::builder(&privacy_panel).with_label("About").build();
	about_label.set_font(&theme::Fonts::body());
	privacy_sizer.add(&about_label, 0, SizerFlag::Left | SizerFlag::Top, 16);
	let about_field =
		TextCtrl::builder(&privacy_panel).with_style(dialog_look::field_style(TextCtrlStyle::Default)).build();
	dialog_look::add_field(&privacy_sizer, &about_field, Field::Line, 0);
	let about_duration_combo =
		visibility_combo(&privacy_panel, &privacy_sizer, "Clear About After", ABOUT_DURATION_CHOICES);

	let photo_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let change_photo_btn = setting_button(&privacy_panel, "Change Profile Photo...");
	photo_sizer.add(&change_photo_btn, 0, SizerFlag::Right, 8);
	let remove_photo_btn = setting_button(&privacy_panel, "Remove Profile Photo...");
	photo_sizer.add(&remove_photo_btn, 0, SizerFlag::Right, 8);
	privacy_sizer.add_sizer(&photo_sizer, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Bottom, 12);

	let default_timer_combo =
		visibility_combo(&privacy_panel, &privacy_sizer, "Default Message Timer for New Chats", DEFAULT_TIMER_CHOICES);
	default_timer_combo.enable(false);

	let last_seen_combo =
		visibility_combo(&privacy_panel, &privacy_sizer, "Who Can See My Last Seen", VISIBILITY_CHOICES);
	let last_seen_except_btn = except_button(&privacy_panel, &privacy_sizer, "Last Seen");
	let online_combo = visibility_combo(&privacy_panel, &privacy_sizer, "Who Can See When I'm Online", ONLINE_CHOICES);
	let profile_photo_combo =
		visibility_combo(&privacy_panel, &privacy_sizer, "Who Can See My Profile Photo", VISIBILITY_CHOICES);
	let profile_photo_except_btn = except_button(&privacy_panel, &privacy_sizer, "Profile Photo");
	let about_privacy_combo =
		visibility_combo(&privacy_panel, &privacy_sizer, "Who Can See My About", VISIBILITY_CHOICES);
	let about_privacy_except_btn = except_button(&privacy_panel, &privacy_sizer, "About");
	let read_receipts_check = setting_switch(
		&privacy_panel,
		&privacy_sizer,
		"Read receipts: if turned off, you won't send or receive read receipts. They're always sent in groups.",
	);
	read_receipts_check.enable(false);
	let groups_combo = visibility_combo(&privacy_panel, &privacy_sizer, "Who Can Add Me to Groups", VISIBILITY_CHOICES);
	let groups_except_btn = except_button(&privacy_panel, &privacy_sizer, "Groups");
	let calls_combo = visibility_combo(&privacy_panel, &privacy_sizer, "Silence Unknown Callers", CALL_CHOICES);

	let link_previews_off_check = setting_switch(
		&privacy_panel,
		&privacy_sizer,
		"Turn off link previews: links you send go without a preview, on all your devices",
	);
	let security_notifications_check = setting_switch(
		&privacy_panel,
		&privacy_sizer,
		"Show security notifications: say in a chat when a contact's security code changes",
	);

	privacy_panel.set_sizer(privacy_sizer, true);
	dialog_look::paint_fields(&privacy_panel, &[(name_field, Field::Line), (about_field, Field::Line)]);

	let audio_panel = Panel::builder(&notebook).build();
	theme::style_content(&audio_panel);
	let audio_sizer = BoxSizer::builder(Orientation::Vertical).build();

	let input_label = StaticText::builder(&audio_panel).with_label("Microphone (calls)").build();
	input_label.set_font(&theme::Fonts::body());
	audio_sizer.add(&input_label, 0, SizerFlag::Left | SizerFlag::Top, 8);
	let input_device_combo = ComboBox::builder(&audio_panel).with_style(ComboBoxStyle::ReadOnly).build();
	input_device_combo.set_font(&theme::Fonts::body());
	audio_sizer.add(&input_device_combo, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 8);

	let input_note = StaticText::builder(&audio_panel).with_label("Also used for recording voice messages.").build();
	input_note.set_font(&theme::Fonts::body());
	audio_sizer.add(&input_note, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom, 8);

	let voice_message_noise_suppression_check = setting_switch(
		&audio_panel,
		&audio_sizer,
		"Clean up voice messages: reduce background noise and even out the volume",
	);

	let noise_suppression_note = StaticText::builder(&audio_panel)
		.with_label("Like WhatsApp Web: removes background noise and brings quiet speech up to a steady, loud level. Off sends the raw mic signal.")
		.build();
	noise_suppression_note.set_font(&theme::Fonts::body());
	audio_sizer.add(&noise_suppression_note, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom, 8);

	let output_label = StaticText::builder(&audio_panel).with_label("Speaker for alerts and voice messages").build();
	output_label.set_font(&theme::Fonts::body());
	audio_sizer.add(&output_label, 0, SizerFlag::Left | SizerFlag::Top, 8);
	let output_device_combo = ComboBox::builder(&audio_panel).with_style(ComboBoxStyle::ReadOnly).build();
	output_device_combo.set_font(&theme::Fonts::body());
	audio_sizer.add(&output_device_combo, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 8);
	let output_note = StaticText::builder(&audio_panel)
		.with_label("Used for notification sounds, message tones and voice message playback.")
		.build();
	output_note.set_font(&theme::Fonts::body());
	audio_sizer.add(&output_note, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom, 8);

	let call_output_label = StaticText::builder(&audio_panel).with_label("Speaker for calls").build();
	call_output_label.set_font(&theme::Fonts::body());
	audio_sizer.add(&call_output_label, 0, SizerFlag::Left | SizerFlag::Top, 8);
	let call_output_device_combo = ComboBox::builder(&audio_panel).with_style(ComboBoxStyle::ReadOnly).build();
	call_output_device_combo.set_font(&theme::Fonts::body());
	audio_sizer.add(&call_output_device_combo, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 8);

	let dual_stream_screen_share_check = setting_switch(
		&audio_panel,
		&audio_sizer,
		"Experimental: keep the camera on while sharing the screen (applies to the next call)",
	);

	audio_panel.set_sizer(audio_sizer, true);

	notebook.add_page(&general_panel, "General", true, None);
	notebook.add_page(&notifications_panel, "Notifications", false, None);
	notebook.add_page(&chats_panel, "Chats", false, None);
	notebook.add_page(&privacy_panel, "Profile & Privacy", false, None);
	notebook.add_page(&audio_panel, "Input & Output", false, None);
	notebook.add_page(&lock_panel, "App Lock", false, None);

	main_sizer.add(&notebook, 1, SizerFlag::Expand | SizerFlag::All, 5);

	let ok_btn = Button::builder(&dialog).with_id(ID_OK).with_label("Save").build();
	dialog_look::pill_button(&ok_btn, true, theme::Palette::CONTENT_BG());
	let cancel_btn = Button::builder(&dialog).with_id(ID_CANCEL).with_label("Cancel").build();
	dialog_look::pill_button(&cancel_btn, false, theme::Palette::CONTENT_BG());
	dialog_look::button_row(&main_sizer, &[&ok_btn, &cancel_btn]);
	dialog.set_sizer(main_sizer, true);
	if let Some(display) = wxdragon::display::Display::from_window(&dialog) {
		let height = (display.client_area().height - 60).clamp(520, 900);
		dialog.set_size(Size::new(600, height));
	}
	dialog.center();

	crate::ui::keys::escape_cancels_anywhere(dialog);
	emoji_help_btn.on_click(move |_| crate::ui::emoji_replace::show_help(&dialog));
	for picker in [message_tone_picker, group_tone_picker] {
		picker.on_preview(move |picker| picker.preview(device_combo_value(&output_device_combo)));
	}
	for picker in [call_ringtone_picker, group_call_ringtone_picker] {
		picker.on_preview(move |picker| picker.preview(device_combo_value(&call_output_device_combo)));
	}

	SettingsDialogParts {
		dialog,
		notebook,
		general_panel,
		notifications_panel,
		sounds_check,
		notifications_check,
		notification_previews_check,
		reaction_notifications_check,
		group_notifications_check,
		group_sounds_check,
		message_tone_picker,
		group_tone_picker,
		outgoing_sounds_check,
		call_notifications_check,
		call_ringtone_check,
		call_ringtone_picker,
		group_call_ringtone_picker,
		follow_dnd_check,
		announce_incoming_check,
		announce_outgoing_check,
		announce_typing_check,
		announce_recording_check,
		tray_icon_check,
		autostart_check,
		start_maximized_check,
		check_updates_check,
		update_channel_combo,
		meta_ai_enabled_check,
		verbose_backend_logging_check,
		shortcuts_btn,
		share_menu_btn,
		chats_panel,
		enter_is_send_check,
		type_to_compose_check,
		keep_list_position_check,
		chat_lock_hello_check,
		app_lock_check,
		app_lock_method_combo,
		app_lock_password_btn,
		replace_emoji_check,
		emoji_help_btn,
		spell_check_check,
		spell_announce_check,
		spell_language_combo,
		auto_download_photos_check,
		auto_download_audio_check,
		auto_download_videos_check,
		auto_download_documents_check,
		download_quality_combo,
		upload_quality_combo,
		theme_combo,
		privacy_panel,
		name_field,
		about_field,
		about_duration_combo,
		change_photo_btn,
		remove_photo_btn,
		default_timer_combo,
		last_seen_combo,
		last_seen_except_btn,
		profile_photo_combo,
		profile_photo_except_btn,
		about_privacy_combo,
		about_privacy_except_btn,
		read_receipts_check,
		groups_combo,
		groups_except_btn,
		calls_combo,
		online_combo,
		link_previews_off_check,
		security_notifications_check,
		audio_panel,
		input_device_combo,
		output_device_combo,
		call_output_device_combo,
		voice_message_noise_suppression_check,
		dual_stream_screen_share_check,
		ok_btn,
		cancel_btn,
	}
}

pub fn set_device_combo(combo: &ComboBox, names: &[String], selected: Option<&str>) {
	combo.clear();
	combo.append(DEFAULT_LABEL);
	for name in names {
		combo.append(name);
	}
	let idx = selected.and_then(|s| names.iter().position(|n| n == s)).map(|i| i + 1).unwrap_or(0);
	combo.set_selection(idx as u32);
}

pub fn device_combo_value(combo: &ComboBox) -> Option<String> {
	match combo.get_string_selection() {
		Some(label) if label != DEFAULT_LABEL => Some(label),
		_ => None,
	}
}

pub fn set_combo_from_value(combo: &ComboBox, choices: &[(&str, &str)], value: &str) {
	let idx = choices.iter().position(|(_, v)| *v == value).unwrap_or(0);
	combo.set_selection(idx as u32);
}

pub fn shown_value<'a>(choices: &[(&'a str, &'a str)], value: &str) -> &'a str {
	choices.iter().find(|(_, v)| *v == value).unwrap_or(&choices[0]).1
}

impl SettingsDialogParts {
	pub fn privacy_controls(&self) -> [PrivacyControl; 6] {
		let control =
			|name, label, combo, choices, except_btn| PrivacyControl { name, label, combo, choices, except_btn };
		[
			control("last", "Last Seen", self.last_seen_combo, VISIBILITY_CHOICES, Some(self.last_seen_except_btn)),
			control("online", "Online", self.online_combo, ONLINE_CHOICES, None),
			control(
				"profile",
				"Profile Photo",
				self.profile_photo_combo,
				VISIBILITY_CHOICES,
				Some(self.profile_photo_except_btn),
			),
			control(
				"status",
				"About",
				self.about_privacy_combo,
				VISIBILITY_CHOICES,
				Some(self.about_privacy_except_btn),
			),
			control("groupadd", "Groups", self.groups_combo, VISIBILITY_CHOICES, Some(self.groups_except_btn)),
			control("calladd", "Calls", self.calls_combo, CALL_CHOICES, None),
		]
	}

	pub fn show_privacy(&self, settings: Option<&std::collections::HashMap<String, String>>) {
		for control in self.privacy_controls() {
			if let Some(settings) = settings {
				let value = settings.get(control.name).map(String::as_str).unwrap_or("");
				set_combo_from_value(&control.combo, control.choices, value);
			}
			control.combo.enable(settings.is_some());
			if let Some(btn) = control.except_btn {
				btn.enable(settings.is_some() && control.is_except());
			}
		}
		if let Some(settings) = settings {
			self.read_receipts_check.set_value(settings.get("readreceipts").map(String::as_str) != Some("none"));
		}
		self.read_receipts_check.enable(settings.is_some());
	}

	pub fn read_receipts_change(&self, settings: &std::collections::HashMap<String, String>) -> Option<&'static str> {
		let on = self.read_receipts_check.is_checked();
		let was_on = settings.get("readreceipts").map(String::as_str) != Some("none");
		(on != was_on).then_some(if on { "all" } else { "none" })
	}

	fn app_pref_checks(&self) -> [(&'static str, CheckBox); 6] {
		[
			("auto_download_photos", self.auto_download_photos_check),
			("auto_download_audio", self.auto_download_audio_check),
			("auto_download_videos", self.auto_download_videos_check),
			("auto_download_documents", self.auto_download_documents_check),
			("link_previews_disabled", self.link_previews_off_check),
			("security_notifications", self.security_notifications_check),
		]
	}

	pub fn show_app_prefs(&self, prefs: Option<&serde_json::Value>) {
		let pref = |name: &str| prefs.and_then(|p| p.get(name)).and_then(|v| v.as_bool()).unwrap_or(false);
		for (name, check) in self.app_pref_checks() {
			if prefs.is_some() {
				check.set_value(pref(name));
			}
			check.enable(prefs.is_some());
		}
		if prefs.is_some() {
			let quality = if pref("hd_media") { "hd" } else { "sd" };
			set_combo_from_value(&self.upload_quality_combo, UPLOAD_QUALITY_CHOICES, quality);
			let quality = if pref("hd_auto_download") { "hd" } else { "sd" };
			set_combo_from_value(&self.download_quality_combo, DOWNLOAD_QUALITY_CHOICES, quality);
		}
		self.upload_quality_combo.enable(prefs.is_some());
		self.download_quality_combo.enable(prefs.is_some());
	}

	pub fn app_prefs_changes(&self, loaded: &serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
		let was = |name: &str| loaded.get(name).and_then(|v| v.as_bool()).unwrap_or(false);
		let hd = combo_value(&self.upload_quality_combo, UPLOAD_QUALITY_CHOICES) == "hd";
		let hd_download = combo_value(&self.download_quality_combo, DOWNLOAD_QUALITY_CHOICES) == "hd";
		self.app_pref_checks()
			.into_iter()
			.map(|(name, check)| (name, check.is_checked()))
			.chain([("hd_media", hd), ("hd_auto_download", hd_download)])
			.filter(|&(name, now)| now != was(name))
			.map(|(name, now)| (name.to_string(), serde_json::Value::Bool(now)))
			.collect()
	}

	pub fn match_theme(&self) {
		for panel in
			[&self.general_panel, &self.notifications_panel, &self.chats_panel, &self.privacy_panel, &self.audio_panel]
		{
			theme::style_content(panel);
		}
		theme::style_content(&self.notebook);
		theme::style_content(&self.dialog);
		theme::match_theme(&self.dialog);
	}

	pub fn show_chat_settings(&self, settings: &crate::settings::Settings) {
		self.notification_previews_check.set_value(settings.notification_previews);
		self.reaction_notifications_check.set_value(settings.reaction_notifications);
		self.group_notifications_check.set_value(settings.group_notifications);
		self.group_sounds_check.set_value(settings.group_sounds);
		self.message_tone_picker.set_tone(settings.message_tone, &settings.message_tone_file);
		self.group_tone_picker.set_tone(settings.group_tone, &settings.group_tone_file);
		self.call_ringtone_picker.set_ringtone(&settings.call_ringtone_sound);
		self.group_call_ringtone_picker.set_ringtone(&settings.group_call_ringtone_sound);
		self.outgoing_sounds_check.set_value(settings.outgoing_sounds);
		self.call_notifications_check.set_value(settings.call_notifications);
		self.call_ringtone_check.set_value(settings.call_ringtone);
		self.enter_is_send_check.set_value(settings.enter_is_send);
		self.type_to_compose_check.set_value(settings.type_to_compose);
		self.keep_list_position_check.set_value(settings.keep_list_position);
		// Only offered when Windows Hello is set up now; a saved "on" stays
		// on, so it isn't lost while the sign-in method is missing.
		let hello = crate::platform::windows_hello::availability();
		self.chat_lock_hello_check.enable(hello == crate::platform::windows_hello::Availability::Ready);
		self.chat_lock_hello_check.set_value(settings.chat_lock_hello);
		self.app_lock_check.set_value(settings.app_lock);
		set_combo_from_value(
			&self.app_lock_method_combo,
			APP_LOCK_METHOD_CHOICES,
			if settings.app_lock_hello { "hello" } else { "password" },
		);
		self.announce_incoming_check.set_value(settings.announce_incoming);
		self.announce_outgoing_check.set_value(settings.announce_outgoing);
		self.replace_emoji_check.set_value(settings.replace_text_with_emoji);
		self.spell_check_check.set_value(settings.spell_check);
		self.spell_announce_check.set_value(settings.spell_check_announce);
		let languages = crate::ui::spell_check::languages();
		let index = languages.iter().position(|(tag, _)| tag.eq_ignore_ascii_case(&settings.spell_language));
		self.spell_language_combo.set_selection(index.map_or(0, |i| i as u32 + 1));
		let theme = serde_json::to_value(settings.theme).ok();
		set_combo_from_value(&self.theme_combo, THEME_CHOICES, theme.as_ref().and_then(|v| v.as_str()).unwrap_or(""));
	}

	pub fn read_chat_settings(&self, settings: &mut crate::settings::Settings) {
		settings.notification_previews = self.notification_previews_check.is_checked();
		settings.reaction_notifications = self.reaction_notifications_check.is_checked();
		settings.group_notifications = self.group_notifications_check.is_checked();
		settings.group_sounds = self.group_sounds_check.is_checked();
		(settings.message_tone, settings.message_tone_file) = self.message_tone_picker.tone();
		(settings.group_tone, settings.group_tone_file) = self.group_tone_picker.tone();
		settings.call_ringtone_sound = self.call_ringtone_picker.ringtone();
		settings.group_call_ringtone_sound = self.group_call_ringtone_picker.ringtone();
		settings.outgoing_sounds = self.outgoing_sounds_check.is_checked();
		settings.call_notifications = self.call_notifications_check.is_checked();
		settings.call_ringtone = self.call_ringtone_check.is_checked();
		settings.enter_is_send = self.enter_is_send_check.is_checked();
		settings.type_to_compose = self.type_to_compose_check.is_checked();
		settings.keep_list_position = self.keep_list_position_check.is_checked();
		settings.chat_lock_hello = self.chat_lock_hello_check.is_checked();
		settings.app_lock = self.app_lock_check.is_checked();
		settings.app_lock_hello = combo_value(&self.app_lock_method_combo, APP_LOCK_METHOD_CHOICES) == "hello";
		settings.announce_incoming = self.announce_incoming_check.is_checked();
		settings.announce_outgoing = self.announce_outgoing_check.is_checked();
		settings.replace_text_with_emoji = self.replace_emoji_check.is_checked();
		settings.spell_check = self.spell_check_check.is_checked();
		settings.spell_check_announce = self.spell_announce_check.is_checked();
		let languages = crate::ui::spell_check::languages();
		settings.spell_language = self
			.spell_language_combo
			.get_selection()
			.and_then(|i| (i as usize).checked_sub(1))
			.and_then(|i| languages.get(i))
			.map(|(tag, _)| tag.clone())
			.unwrap_or_default();
		let theme = combo_value(&self.theme_combo, THEME_CHOICES);
		settings.theme = serde_json::from_value(serde_json::Value::String(theme)).unwrap_or_default();
	}

	pub fn show_about(&self, about: Option<(&str, i64)>) {
		if let Some((text, duration)) = about {
			self.about_field.set_value(text);
			set_combo_from_value(&self.about_duration_combo, ABOUT_DURATION_CHOICES, &duration.to_string());
		}
		self.about_field.enable(about.is_some());
		self.about_duration_combo.enable(about.is_some());
	}
}

pub fn combo_value(combo: &ComboBox, choices: &[(&str, &str)]) -> String {
	let label = combo.get_string_selection().unwrap_or_default();
	choices.iter().find(|(l, _)| *l == label).map(|(_, v)| v.to_string()).unwrap_or_else(|| choices[0].1.to_string())
}
