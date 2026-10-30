use wxdragon::prelude::*;

use crate::{audio_devices::DEFAULT_LABEL, ui::theme};

/// Display label -> wire value pairs for a privacy dropdown, matching the
/// values WhatsApp Web offers for that category (WAWebPrivacySettings).
/// "My Contacts Except..." comes with its own contact picker.
pub const VISIBILITY_CHOICES: &[(&str, &str)] =
	&[("Everyone", "all"), ("My Contacts", "contacts"), ("My Contacts Except...", EXCEPT_VALUE), ("Nobody", "none")];
/// The wire value of "My Contacts Except...".
pub const EXCEPT_VALUE: &str = "contact_blacklist";
/// Read receipts are a switch in WhatsApp, not an audience.
pub const READ_RECEIPTS_CHOICES: &[(&str, &str)] = &[("On", "all"), ("Off", "none")];
pub const ONLINE_CHOICES: &[(&str, &str)] = &[("Everyone", "all"), ("Same as Last Seen", "match_last_seen")];
/// "calladd" is WhatsApp's "Silence unknown callers": with "known", calls
/// from numbers that aren't contacts still come in, they just don't ring.
/// Nobody is stopped from calling, so it isn't "who can call me".
pub const CALL_CHOICES: &[(&str, &str)] = &[("Off", "all"), ("On", "known")];
/// The default timer for disappearing messages in new chats, in seconds, as
/// WhatsApp offers it.
pub const DEFAULT_TIMER_CHOICES: &[(&str, &str)] =
	&[("Off", "0"), ("24 Hours", "86400"), ("7 Days", "604800"), ("90 Days", "7776000")];
/// "Media upload quality" (WAWebMediaUploadQualitySettings): HD sends photos
/// bigger (up to 2560 pixels on the longest side instead of 1600).
pub const UPLOAD_QUALITY_CHOICES: &[(&str, &str)] = &[("Standard Quality", "sd"), ("HD Quality", "hd")];
/// How long an About shows before it clears, in seconds: WhatsApp's
/// text_status_ttl_seconds_allowlist, plus -1 for one that stays.
pub const ABOUT_DURATION_CHOICES: &[(&str, &str)] = &[
	("Never", "-1"),
	("30 Minutes", "1800"),
	("1 Hour", "3600"),
	("2 Hours", "7200"),
	("4 Hours", "14400"),
	("8 Hours", "28800"),
	("24 Hours", "86400"),
];

/// One privacy dropdown: the setting's wire name, what it's called, and its
/// "My Contacts Except..." button for the settings that have one.
pub struct PrivacyControl {
	pub name: &'static str,
	pub label: &'static str,
	pub combo: ComboBox,
	pub choices: &'static [(&'static str, &'static str)],
	pub except_btn: Option<Button>,
}

impl PrivacyControl {
	/// Whether the dropdown is on "My Contacts Except...".
	pub fn is_except(&self) -> bool {
		self.except_btn.is_some() && combo_value(&self.combo, self.choices) == EXCEPT_VALUE
	}
}

/// Every constructed widget is kept here for consistency, matching this
/// codebase's convention for dialog parts structs.
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub struct SettingsDialogParts {
	pub dialog: Dialog,
	pub notebook: Notebook,

	// General tab
	pub general_panel: Panel,
	pub sounds_check: CheckBox,
	pub notifications_check: CheckBox,
	pub notification_previews_check: CheckBox,
	pub reaction_notifications_check: CheckBox,
	pub follow_dnd_check: CheckBox,
	pub announce_typing_check: CheckBox,
	pub announce_recording_check: CheckBox,
	pub tray_icon_check: CheckBox,
	pub autostart_check: CheckBox,
	pub start_maximized_check: CheckBox,
	pub meta_ai_enabled_check: CheckBox,
	pub verbose_backend_logging_check: CheckBox,
	pub shortcuts_btn: Button,
	pub share_menu_btn: Button,

	// Chats tab
	pub chats_panel: Panel,
	pub enter_is_send_check: CheckBox,
	pub replace_emoji_check: CheckBox,
	pub auto_download_photos_check: CheckBox,
	pub auto_download_audio_check: CheckBox,
	pub auto_download_videos_check: CheckBox,
	pub auto_download_documents_check: CheckBox,
	pub upload_quality_combo: ComboBox,

	// Profile & Privacy tab
	pub privacy_panel: Panel,
	pub name_field: TextCtrl,
	pub about_field: TextCtrl,
	pub about_duration_combo: ComboBox,
	pub change_photo_btn: Button,
	pub remove_photo_btn: Button,
	/// Disabled until the current timer has loaded.
	pub default_timer_combo: ComboBox,
	pub last_seen_combo: ComboBox,
	pub last_seen_except_btn: Button,
	pub profile_photo_combo: ComboBox,
	pub profile_photo_except_btn: Button,
	pub about_privacy_combo: ComboBox,
	pub about_privacy_except_btn: Button,
	pub read_receipts_combo: ComboBox,
	pub groups_combo: ComboBox,
	pub groups_except_btn: Button,
	pub calls_combo: ComboBox,
	pub online_combo: ComboBox,
	pub link_previews_off_check: CheckBox,
	pub security_notifications_check: CheckBox,

	// Input & Output tab
	pub audio_panel: Panel,
	pub input_device_combo: ComboBox,
	pub output_device_combo: ComboBox,
	pub voice_message_noise_suppression_check: CheckBox,
	pub dual_stream_screen_share_check: CheckBox,

	pub ok_btn: Button,
	pub cancel_btn: Button,
}

fn visibility_combo(panel: &Panel, sizer: &BoxSizer, label: &str, choices: &[(&str, &str)]) -> ComboBox {
	let text = StaticText::builder(panel).with_label(label).build();
	text.set_font(&theme::Fonts::body());
	sizer.add(&text, 0, SizerFlag::Left | SizerFlag::Top, 8);

	let labels: Vec<&str> = choices.iter().map(|(l, _)| *l).collect();
	let combo = ComboBox::builder(panel).with_string_choices(&labels).with_style(ComboBoxStyle::ReadOnly).build();
	combo.set_font(&theme::Fonts::body());
	combo.set_selection(0);
	sizer.add(&combo, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom, 8);
	combo
}

/// The "Choose Excluded Contacts..." button under a dropdown that has
/// "My Contacts Except...". It's named for its setting, since a screen reader
/// reads buttons on their own.
fn except_button(panel: &Panel, sizer: &BoxSizer, setting: &str) -> Button {
	let btn = Button::builder(panel).with_label(&format!("Choose Contacts Excluded From {}...", setting)).build();
	btn.set_font(&theme::Fonts::body());
	btn.enable(false);
	sizer.add(&btn, 0, SizerFlag::Left | SizerFlag::Bottom, 8);
	btn
}

pub fn build_settings_dialog(parent: &impl WxWidget) -> SettingsDialogParts {
	let dialog = Dialog::builder(parent, "Settings")
		.with_size(420, 520)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);

	let main_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let notebook = Notebook::builder(&dialog).build();

	// --- General tab ---
	let general_panel = Panel::builder(&notebook).build();
	theme::style_content(&general_panel);
	let general_sizer = BoxSizer::builder(Orientation::Vertical).build();

	let sounds_check = CheckBox::builder(&general_panel).with_label("Play a sound for new messages").build();
	sounds_check.set_font(&theme::Fonts::body());
	general_sizer.add(&sounds_check, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let notifications_check = CheckBox::builder(&general_panel).with_label("Show desktop notifications").build();
	notifications_check.set_font(&theme::Fonts::body());
	general_sizer.add(&notifications_check, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let notification_previews_check =
		CheckBox::builder(&general_panel).with_label("Show previews: message text in notifications").build();
	notification_previews_check.set_font(&theme::Fonts::body());
	general_sizer.add(&notification_previews_check, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let reaction_notifications_check =
		CheckBox::builder(&general_panel).with_label("Show reaction notifications").build();
	reaction_notifications_check.set_font(&theme::Fonts::body());
	general_sizer.add(&reaction_notifications_check, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let follow_dnd_check = CheckBox::builder(&general_panel)
		.with_label("No message sound while Windows Do Not Disturb, a full-screen game or a presentation is on")
		.build();
	follow_dnd_check.set_font(&theme::Fonts::body());
	general_sizer.add(&follow_dnd_check, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let announce_typing_check = CheckBox::builder(&general_panel).with_label("Announce when someone is typing").build();
	announce_typing_check.set_font(&theme::Fonts::body());
	general_sizer.add(&announce_typing_check, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let announce_recording_check =
		CheckBox::builder(&general_panel).with_label("Announce when someone is recording a voice message").build();
	announce_recording_check.set_font(&theme::Fonts::body());
	general_sizer.add(&announce_recording_check, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let tray_icon_check = CheckBox::builder(&general_panel)
		.with_label("Show an icon in the notification area; closing the window minimizes to it")
		.build();
	tray_icon_check.set_font(&theme::Fonts::body());
	general_sizer.add(&tray_icon_check, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let autostart_check = CheckBox::builder(&general_panel).with_label("Start whatSnap when Windows starts").build();
	autostart_check.set_font(&theme::Fonts::body());
	general_sizer.add(&autostart_check, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let start_maximized_check = CheckBox::builder(&general_panel).with_label("Start maximized").build();
	start_maximized_check.set_font(&theme::Fonts::body());
	general_sizer.add(&start_maximized_check, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let meta_ai_enabled_check =
		CheckBox::builder(&general_panel).with_label("Show a button to message Meta AI in New Chat").build();
	meta_ai_enabled_check.set_font(&theme::Fonts::body());
	general_sizer.add(&meta_ai_enabled_check, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let verbose_backend_logging_check = CheckBox::builder(&general_panel)
		.with_label(
			"Verbose backend logging, for troubleshooting connection issues (takes effect next time whatSnap starts)",
		)
		.build();
	verbose_backend_logging_check.set_font(&theme::Fonts::body());
	general_sizer.add(&verbose_backend_logging_check, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let shortcuts_btn = Button::builder(&general_panel).with_label("&Keyboard Shortcuts...").build();
	shortcuts_btn.set_font(&theme::Fonts::body());
	general_sizer.add(&shortcuts_btn, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let share_menu_btn =
		Button::builder(&general_panel).with_label("Add whatSnap to the Windows Share Menu...").build();
	share_menu_btn.set_font(&theme::Fonts::body());
	general_sizer.add(&share_menu_btn, 0, SizerFlag::Left | SizerFlag::Top, 10);

	general_panel.set_sizer(general_sizer, true);

	// --- Chats tab ---
	// WhatsApp Web's Settings > Chats, in its words.
	let chats_panel = Panel::builder(&notebook).build();
	theme::style_content(&chats_panel);
	let chats_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let chats_check = |label: &str| {
		let check = CheckBox::builder(&chats_panel).with_label(label).build();
		check.set_font(&theme::Fonts::body());
		chats_sizer.add(&check, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);
		check
	};
	let enter_is_send_check = chats_check("Enter is send: the Enter key sends your message");
	let replace_emoji_check = chats_check("Replace text with emoji as you type, like :-) or (y)");
	// Disabled, like the privacy settings, until they've been read.
	let auto_download_photos_check = chats_check("Download photos automatically");
	let auto_download_audio_check = chats_check("Download audio automatically");
	let auto_download_videos_check = chats_check("Download videos automatically");
	let auto_download_documents_check = chats_check("Download documents automatically");
	let auto_download_note = StaticText::builder(&chats_panel)
		.with_label("Voice messages are always downloaded. Other media downloads by itself only from contacts and chats you've written in.")
		.build();
	auto_download_note.set_font(&theme::Fonts::body());
	chats_sizer.add(&auto_download_note, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);
	let upload_quality_combo =
		visibility_combo(&chats_panel, &chats_sizer, "Media Upload Quality", UPLOAD_QUALITY_CHOICES);
	chats_panel.set_sizer(chats_sizer, true);

	// --- Profile & Privacy tab ---
	let privacy_panel = Panel::builder(&notebook).build();
	theme::style_content(&privacy_panel);
	let privacy_sizer = BoxSizer::builder(Orientation::Vertical).build();

	let name_label = StaticText::builder(&privacy_panel).with_label("Name").build();
	name_label.set_font(&theme::Fonts::body());
	privacy_sizer.add(&name_label, 0, SizerFlag::Left | SizerFlag::Top, 8);
	let name_field = TextCtrl::builder(&privacy_panel).build();
	name_field.set_font(&theme::Fonts::body());
	privacy_sizer.add(&name_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 8);

	let about_label = StaticText::builder(&privacy_panel).with_label("About").build();
	about_label.set_font(&theme::Fonts::body());
	privacy_sizer.add(&about_label, 0, SizerFlag::Left | SizerFlag::Top, 8);
	let about_field = TextCtrl::builder(&privacy_panel).build();
	about_field.set_font(&theme::Fonts::body());
	privacy_sizer.add(&about_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom, 8);
	let about_duration_combo =
		visibility_combo(&privacy_panel, &privacy_sizer, "Clear About After", ABOUT_DURATION_CHOICES);

	let photo_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let change_photo_btn = Button::builder(&privacy_panel).with_label("Change Profile Photo...").build();
	change_photo_btn.set_font(&theme::Fonts::body());
	photo_sizer.add(&change_photo_btn, 0, SizerFlag::Right, 8);
	let remove_photo_btn = Button::builder(&privacy_panel).with_label("Remove Profile Photo...").build();
	remove_photo_btn.set_font(&theme::Fonts::body());
	photo_sizer.add(&remove_photo_btn, 0, SizerFlag::Right, 8);
	privacy_sizer.add_sizer(&photo_sizer, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Bottom, 8);

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
	let read_receipts_combo = visibility_combo(&privacy_panel, &privacy_sizer, "Read Receipts", READ_RECEIPTS_CHOICES);
	let groups_combo = visibility_combo(&privacy_panel, &privacy_sizer, "Who Can Add Me to Groups", VISIBILITY_CHOICES);
	let groups_except_btn = except_button(&privacy_panel, &privacy_sizer, "Groups");
	let calls_combo = visibility_combo(&privacy_panel, &privacy_sizer, "Silence Unknown Callers", CALL_CHOICES);

	let link_previews_off_check = CheckBox::builder(&privacy_panel)
		.with_label("Turn off link previews: links you send go without a preview, on all your devices")
		.build();
	link_previews_off_check.set_font(&theme::Fonts::body());
	privacy_sizer.add(&link_previews_off_check, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 8);
	let security_notifications_check = CheckBox::builder(&privacy_panel)
		.with_label("Show security notifications: say in a chat when a contact's security code changes")
		.build();
	security_notifications_check.set_font(&theme::Fonts::body());
	privacy_sizer.add(&security_notifications_check, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 8);

	privacy_panel.set_sizer(privacy_sizer, true);

	// --- Input & Output tab ---
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

	let voice_message_noise_suppression_check = CheckBox::builder(&audio_panel)
		.with_label("Clean up voice messages: reduce background noise and even out the volume")
		.build();
	voice_message_noise_suppression_check.set_font(&theme::Fonts::body());
	audio_sizer.add(&voice_message_noise_suppression_check, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 8);

	let noise_suppression_note = StaticText::builder(&audio_panel)
		.with_label("Like WhatsApp Web: removes background noise and brings quiet speech up to a steady, loud level. Off sends the raw mic signal.")
		.build();
	noise_suppression_note.set_font(&theme::Fonts::body());
	audio_sizer.add(&noise_suppression_note, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom, 8);

	let output_label = StaticText::builder(&audio_panel).with_label("Speaker (calls & playback)").build();
	output_label.set_font(&theme::Fonts::body());
	audio_sizer.add(&output_label, 0, SizerFlag::Left | SizerFlag::Top, 8);
	let output_device_combo = ComboBox::builder(&audio_panel).with_style(ComboBoxStyle::ReadOnly).build();
	output_device_combo.set_font(&theme::Fonts::body());
	audio_sizer.add(&output_device_combo, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 8);

	let output_note = StaticText::builder(&audio_panel)
		.with_label("Used for call audio, voice message playback, and notification/ringtone sounds.")
		.build();
	output_note.set_font(&theme::Fonts::body());
	audio_sizer.add(&output_note, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom, 8);

	let dual_stream_screen_share_check = CheckBox::builder(&audio_panel)
		.with_label("Experimental: keep the camera on while sharing the screen (applies to the next call)")
		.build();
	dual_stream_screen_share_check.set_font(&theme::Fonts::body());
	audio_sizer.add(&dual_stream_screen_share_check, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 8);

	audio_panel.set_sizer(audio_sizer, true);

	notebook.add_page(&general_panel, "General", true, None);
	notebook.add_page(&chats_panel, "Chats", false, None);
	notebook.add_page(&privacy_panel, "Profile & Privacy", false, None);
	notebook.add_page(&audio_panel, "Input & Output", false, None);

	main_sizer.add(&notebook, 1, SizerFlag::Expand | SizerFlag::All, 5);

	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let ok_btn = Button::builder(&dialog).with_id(ID_OK).with_label("Save").build();
	theme::style_primary_button(&ok_btn);
	btn_sizer.add(&ok_btn, 0, SizerFlag::Right, 8);
	btn_sizer.add_stretch_spacer(1);
	let cancel_btn = Button::builder(&dialog).with_id(ID_CANCEL).with_label("Cancel").build();
	btn_sizer.add(&cancel_btn, 0, SizerFlag::Right, 8);

	main_sizer.add_sizer(&btn_sizer, 0, SizerFlag::Expand | SizerFlag::All, 10);
	dialog.set_sizer(main_sizer, true);
	dialog.center();

	// CHAR_HOOK fires on this dialog (a top-level window) before the
	// keypress reaches whichever control has focus, so Escape closes the
	// dialog regardless of which checkbox/combo/field/tab is focused -
	// binding it per-widget (as this used to, on sounds_check only) misses
	// everything else in the dialog.
	crate::ui::keys::escape_cancels_anywhere(dialog);

	SettingsDialogParts {
		dialog,
		notebook,
		general_panel,
		sounds_check,
		notifications_check,
		notification_previews_check,
		reaction_notifications_check,
		follow_dnd_check,
		announce_typing_check,
		announce_recording_check,
		tray_icon_check,
		autostart_check,
		start_maximized_check,
		meta_ai_enabled_check,
		verbose_backend_logging_check,
		shortcuts_btn,
		share_menu_btn,
		chats_panel,
		enter_is_send_check,
		replace_emoji_check,
		auto_download_photos_check,
		auto_download_audio_check,
		auto_download_videos_check,
		auto_download_documents_check,
		upload_quality_combo,
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
		read_receipts_combo,
		groups_combo,
		groups_except_btn,
		calls_combo,
		online_combo,
		link_previews_off_check,
		security_notifications_check,
		audio_panel,
		input_device_combo,
		output_device_combo,
		voice_message_noise_suppression_check,
		dual_stream_screen_share_check,
		ok_btn,
		cancel_btn,
	}
}

/// Repopulates a device-selection combo box with "System Default" followed by
/// `names`, selecting whichever entry matches `selected` (or "System Default"
/// if `selected` is `None` or no longer present in `names`).
pub fn set_device_combo(combo: &ComboBox, names: &[String], selected: Option<&str>) {
	combo.clear();
	combo.append(DEFAULT_LABEL);
	for name in names {
		combo.append(name);
	}
	let idx = selected.and_then(|s| names.iter().position(|n| n == s)).map(|i| i + 1).unwrap_or(0);
	combo.set_selection(idx as u32);
}

/// The selected device's friendly name, or `None` for "System Default".
pub fn device_combo_value(combo: &ComboBox) -> Option<String> {
	match combo.get_string_selection() {
		Some(label) if label != DEFAULT_LABEL => Some(label),
		_ => None,
	}
}

/// Sets a ReadOnly combo box's selection to whichever choice's wire value
/// matches `value`, defaulting to the first choice if there's no match.
pub fn set_combo_from_value(combo: &ComboBox, choices: &[(&str, &str)], value: &str) {
	let idx = choices.iter().position(|(_, v)| *v == value).unwrap_or(0);
	combo.set_selection(idx as u32);
}

/// The wire value a combo shows for `value`: `value` itself, or the first
/// choice when it isn't one of them. Saving compares against this, so a
/// setting the dropdown can't show is only sent if the user picks another.
pub fn shown_value<'a>(choices: &[(&'a str, &'a str)], value: &str) -> &'a str {
	choices.iter().find(|(_, v)| *v == value).unwrap_or(&choices[0]).1
}

impl SettingsDialogParts {
	pub fn privacy_controls(&self) -> [PrivacyControl; 7] {
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
			control("readreceipts", "Read Receipts", self.read_receipts_combo, READ_RECEIPTS_CHOICES, None),
			control("groupadd", "Groups", self.groups_combo, VISIBILITY_CHOICES, Some(self.groups_except_btn)),
			control("calladd", "Calls", self.calls_combo, CALL_CHOICES, None),
		]
	}

	/// Shows the privacy settings as the server has them, or, with None,
	/// disables the dropdowns until they arrive, so nothing is shown (or
	/// saved) that isn't the real setting.
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
	}

	/// The switches the backend keeps (see backend app_prefs.go), by name.
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

	/// Shows the settings the backend keeps, or, with None, disables them
	/// until they've been read.
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
		}
		self.upload_quality_combo.enable(prefs.is_some());
	}

	/// What changed from `loaded` (as `show_app_prefs` was given it), to send
	/// as set_app_prefs; empty when nothing did.
	pub fn app_prefs_changes(&self, loaded: &serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
		let was = |name: &str| loaded.get(name).and_then(|v| v.as_bool()).unwrap_or(false);
		let hd = combo_value(&self.upload_quality_combo, UPLOAD_QUALITY_CHOICES) == "hd";
		self.app_pref_checks()
			.into_iter()
			.map(|(name, check)| (name, check.is_checked()))
			.chain([("hd_media", hd)])
			.filter(|&(name, now)| now != was(name))
			.map(|(name, now)| (name.to_string(), serde_json::Value::Bool(now)))
			.collect()
	}

	/// Shows the settings this app keeps itself that WhatsApp Web has too.
	pub fn show_chat_settings(&self, settings: &crate::settings::Settings) {
		self.notification_previews_check.set_value(settings.notification_previews);
		self.reaction_notifications_check.set_value(settings.reaction_notifications);
		self.enter_is_send_check.set_value(settings.enter_is_send);
		self.replace_emoji_check.set_value(settings.replace_text_with_emoji);
	}

	pub fn read_chat_settings(&self, settings: &mut crate::settings::Settings) {
		settings.notification_previews = self.notification_previews_check.is_checked();
		settings.reaction_notifications = self.reaction_notifications_check.is_checked();
		settings.enter_is_send = self.enter_is_send_check.is_checked();
		settings.replace_text_with_emoji = self.replace_emoji_check.is_checked();
	}

	/// Shows the About and how long it lasts, or, with None, disables them
	/// until the real one arrives.
	pub fn show_about(&self, about: Option<(&str, i64)>) {
		if let Some((text, duration)) = about {
			self.about_field.set_value(text);
			set_combo_from_value(&self.about_duration_combo, ABOUT_DURATION_CHOICES, &duration.to_string());
		}
		self.about_field.enable(about.is_some());
		self.about_duration_combo.enable(about.is_some());
	}
}

/// The wire value for a ReadOnly combo box's current selection.
pub fn combo_value(combo: &ComboBox, choices: &[(&str, &str)]) -> String {
	let label = combo.get_string_selection().unwrap_or_default();
	choices.iter().find(|(l, _)| *l == label).map(|(_, v)| v.to_string()).unwrap_or_else(|| choices[0].1.to_string())
}
