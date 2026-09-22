use wxdragon::prelude::*;

use crate::{audio_devices::DEFAULT_LABEL, ui::theme};

/// Display label -> wire value pairs for a privacy dropdown, matching the
/// subset of `types.PrivacySetting` values whatsmeow documents as valid for
/// that category. "Except specific contacts" isn't offered here since it
/// needs its own contact picker; only the plain All/Contacts/Nobody-style
/// choices are exposed.
pub const VISIBILITY_CHOICES: &[(&str, &str)] = &[("Everyone", "all"), ("My Contacts", "contacts"), ("Nobody", "none")];
pub const READ_RECEIPTS_CHOICES: &[(&str, &str)] = &[("Everyone", "all"), ("Nobody", "none")];
pub const ONLINE_CHOICES: &[(&str, &str)] = &[("Everyone", "all"), ("Same as Last Seen", "match_last_seen")];
pub const CALL_CHOICES: &[(&str, &str)] = &[("Everyone", "all"), ("My Contacts", "known")];

/// Every constructed widget is kept here for consistency, matching this
/// codebase's convention for dialog parts structs.
#[allow(dead_code)]
pub struct SettingsDialogParts {
	pub dialog: Dialog,
	pub notebook: Notebook,

	// General tab
	pub general_panel: Panel,
	pub sounds_check: CheckBox,
	pub notifications_check: CheckBox,
	pub announce_typing_check: CheckBox,
	pub announce_recording_check: CheckBox,
	pub tray_icon_check: CheckBox,
	pub autostart_check: CheckBox,
	pub meta_ai_enabled_check: CheckBox,
	pub verbose_backend_logging_check: CheckBox,

	// Profile & Privacy tab
	pub privacy_panel: Panel,
	pub name_field: TextCtrl,
	pub about_field: TextCtrl,
	pub last_seen_combo: ComboBox,
	pub profile_photo_combo: ComboBox,
	pub about_privacy_combo: ComboBox,
	pub read_receipts_combo: ComboBox,
	pub groups_combo: ComboBox,
	pub calls_combo: ComboBox,
	pub online_combo: ComboBox,

	// Input & Output tab
	pub audio_panel: Panel,
	pub input_device_combo: ComboBox,
	pub output_device_combo: ComboBox,
	pub voice_message_noise_suppression_check: CheckBox,

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

	let autostart_check = CheckBox::builder(&general_panel).with_label("Start whaNTV when Windows starts").build();
	autostart_check.set_font(&theme::Fonts::body());
	general_sizer.add(&autostart_check, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let meta_ai_enabled_check =
		CheckBox::builder(&general_panel).with_label("Show a button to message Meta AI in New Conversation").build();
	meta_ai_enabled_check.set_font(&theme::Fonts::body());
	general_sizer.add(&meta_ai_enabled_check, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let verbose_backend_logging_check = CheckBox::builder(&general_panel)
		.with_label(
			"Verbose backend logging, for troubleshooting connection issues (takes effect next time whaNTV starts)",
		)
		.build();
	verbose_backend_logging_check.set_font(&theme::Fonts::body());
	general_sizer.add(&verbose_backend_logging_check, 0, SizerFlag::Left | SizerFlag::Top, 10);

	general_panel.set_sizer(general_sizer, true);

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

	let last_seen_combo = visibility_combo(&privacy_panel, &privacy_sizer, "Last Seen & Online", VISIBILITY_CHOICES);
	let profile_photo_combo = visibility_combo(&privacy_panel, &privacy_sizer, "Profile Photo", VISIBILITY_CHOICES);
	let about_privacy_combo = visibility_combo(&privacy_panel, &privacy_sizer, "About", VISIBILITY_CHOICES);
	let read_receipts_combo = visibility_combo(&privacy_panel, &privacy_sizer, "Read Receipts", READ_RECEIPTS_CHOICES);
	let groups_combo = visibility_combo(&privacy_panel, &privacy_sizer, "Who Can Add Me to Groups", VISIBILITY_CHOICES);
	let calls_combo = visibility_combo(&privacy_panel, &privacy_sizer, "Who Can Call Me", CALL_CHOICES);
	let online_combo = visibility_combo(&privacy_panel, &privacy_sizer, "Online", ONLINE_CHOICES);

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
		.with_label("Reduce background noise when recording voice messages (Windows communications audio effects)")
		.build();
	voice_message_noise_suppression_check.set_font(&theme::Fonts::body());
	audio_sizer.add(&voice_message_noise_suppression_check, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 8);

	let noise_suppression_note = StaticText::builder(&audio_panel)
		.with_label("Off by default: keeps the raw mic signal. Calls always use this processing.")
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

	audio_panel.set_sizer(audio_sizer, true);

	notebook.add_page(&general_panel, "General", true, None);
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
	let dlg_esc = dialog.clone();
	dialog.bind_internal(EventType::CHAR_HOOK, move |event| {
		let data = WindowEventData::new(event);
		if let WindowEventData::Keyboard(ref key) = data {
			if key.get_key_code() == Some(27) {
				dlg_esc.end_modal(ID_CANCEL);
				data.skip(false);
				return;
			}
		}
		data.skip(true);
	});

	SettingsDialogParts {
		dialog,
		notebook,
		general_panel,
		sounds_check,
		notifications_check,
		announce_typing_check,
		announce_recording_check,
		tray_icon_check,
		autostart_check,
		meta_ai_enabled_check,
		verbose_backend_logging_check,
		privacy_panel,
		name_field,
		about_field,
		last_seen_combo,
		profile_photo_combo,
		about_privacy_combo,
		read_receipts_combo,
		groups_combo,
		calls_combo,
		online_combo,
		audio_panel,
		input_device_combo,
		output_device_combo,
		voice_message_noise_suppression_check,
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
/// matches `value`, defaulting to the first choice if there's no match
/// (e.g. the setting hasn't loaded from the server yet).
pub fn set_combo_from_value(combo: &ComboBox, choices: &[(&str, &str)], value: &str) {
	let idx = choices.iter().position(|(_, v)| *v == value).unwrap_or(0);
	combo.set_selection(idx as u32);
}

/// The wire value for a ReadOnly combo box's current selection.
pub fn combo_value(combo: &ComboBox, choices: &[(&str, &str)]) -> String {
	let label = combo.get_string_selection().unwrap_or_default();
	choices.iter().find(|(l, _)| *l == label).map(|(_, v)| v.to_string()).unwrap_or_else(|| choices[0].1.to_string())
}
