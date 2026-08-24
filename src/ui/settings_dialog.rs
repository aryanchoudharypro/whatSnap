use wxdragon::prelude::*;

use crate::ui::theme;

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

	notebook.add_page(&general_panel, "General", true, None);
	notebook.add_page(&privacy_panel, "Profile & Privacy", false, None);

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

	let dlg_esc = dialog.clone();
	sounds_check.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key) = event {
			if key.get_key_code() == Some(27) {
				dlg_esc.end_modal(ID_CANCEL);
				event.skip(false);
				return;
			}
		}
		event.skip(true);
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
		ok_btn,
		cancel_btn,
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
