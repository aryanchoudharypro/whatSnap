use wxdragon::prelude::*;

use crate::{
	audio::sound_file,
	ui::{
		dialog_look,
		sound_picker::{Kind, SoundPicker},
		theme,
	},
};

const ALERTS: [(&str, whatsnap::ChatAlert); 5] = [
	("As set in Settings", whatsnap::ChatAlert::Default),
	("Notification and sound", whatsnap::ChatAlert::Full),
	("Notification only, no sound", whatsnap::ChatAlert::NoSound),
	("Sound only, no notification", whatsnap::ChatAlert::SoundOnly),
	("Off on this computer", whatsnap::ChatAlert::Off),
];

/// `default_tone` and `default_ringtone` are what Default plays for this
/// chat: the message or group tone and the call or group call ringtone from
/// Settings.
pub fn ask(
	parent: &impl WxWidget,
	chat: &str,
	current: whatsnap::ChatNotify,
	default_tone: (u8, String),
	default_ringtone: String,
	output_device: Option<String>,
) -> Option<whatsnap::ChatNotify> {
	let dialog = Dialog::builder(parent, &format!("Notification settings for {chat}"))
		.with_size(460, 480)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let note = StaticText::builder(&panel)
		.with_label("The notification tone is shared with WhatsApp on your other computers, unless it is a custom sound file; the rest only change this computer. To mute this chat on all your devices, use Mute in its menu.")
		.build();
	theme::style_muted_label(&note);
	sizer.add(&note, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);

	let alert_label = StaticText::builder(&panel).with_label("When a message comes in").build();
	alert_label.set_font(&theme::Fonts::body());
	sizer.add(&alert_label, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);
	let labels: Vec<&str> = ALERTS.iter().map(|(label, _)| *label).collect();
	let alert_combo =
		ComboBox::builder(&panel).with_string_choices(&labels).with_style(ComboBoxStyle::ReadOnly).build();
	alert_combo.set_font(&theme::Fonts::body());
	let at = ALERTS.iter().position(|(_, alert)| *alert == current.alert).unwrap_or(0);
	alert_combo.set_selection(at as u32);
	sizer.add(&alert_combo, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);

	let tone_default = match default_tone.0 {
		whatsnap::TONE_DEFAULT => "Default".to_string(),
		whatsnap::TONE_CUSTOM => format!("Default ({})", file_name(&default_tone.1)),
		tone => format!("Default ({})", whatsnap::tone_label(tone)),
	};
	let tone_picker =
		SoundPicker::new(&panel, &sizer, "Notification tone", "Notification Tone", Kind::ChatTone, &tone_default);
	tone_picker.set_tone(current.tone, &current.tone_file);
	let device = output_device.clone();
	tone_picker.on_preview(move |picker| match picker.tone() {
		(whatsnap::TONE_DEFAULT, _) => {
			crate::audio::preview_message_tone(default_tone.0, &default_tone.1, device.clone())
		}
		_ => picker.preview(device.clone()),
	});

	let ringtone_default = format!("Default ({})", ringtone_name(&default_ringtone));
	let ringtone_picker =
		SoundPicker::new(&panel, &sizer, "Ringtone", "Ringtone", Kind::ChatRingtone, &ringtone_default);
	ringtone_picker.set_ringtone(&current.ringtone);
	ringtone_picker.on_preview(move |picker| match picker.ringtone().as_str() {
		"" => crate::audio::preview_ringtone(&default_ringtone, output_device.clone()),
		_ => picker.preview(output_device.clone()),
	});

	let hide_check = CheckBox::builder(&panel)
		.with_label("Hide message content: notifications from this chat don't show what the message says")
		.build();
	hide_check.set_value(current.hide_content);
	dialog_look::switch(&hide_check, theme::Palette::CONTENT_BG());
	sizer.add(&hide_check, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);

	let save_btn = Button::builder(&panel).with_id(ID_OK).with_label("Save").build();
	dialog_look::pill_button(&save_btn, true, theme::Palette::CONTENT_BG());
	let cancel_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Cancel").build();
	dialog_look::pill_button(&cancel_btn, false, theme::Palette::CONTENT_BG());
	dialog_look::button_row(&sizer, &[&save_btn, &cancel_btn]);
	panel.set_sizer(sizer, true);

	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();
	crate::ui::keys::escape_cancels_anywhere(dialog);
	alert_combo.set_focus();

	let saved = dialog.show_modal() == ID_OK;
	let picked = alert_combo.get_selection().map(|i| i as usize).unwrap_or(0);
	let (tone, tone_file) = tone_picker.tone();
	let notify = whatsnap::ChatNotify {
		alert: ALERTS.get(picked).map(|(_, alert)| *alert).unwrap_or_default(),
		hide_content: hide_check.is_checked(),
		tone,
		tone_file: if tone == whatsnap::TONE_CUSTOM { tone_file } else { String::new() },
		ringtone: ringtone_picker.ringtone(),
	};
	tone_picker.forget();
	ringtone_picker.forget();
	dialog.destroy();
	saved.then_some(notify)
}

fn file_name(path: &str) -> String {
	std::path::Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}

/// How a ringtone (see [`whatsnap::RINGTONE_WHATSAPP`]) reads in a list.
fn ringtone_name(ringtone: &str) -> String {
	match ringtone {
		"" | whatsnap::RINGTONE_WHATSAPP => "WhatsApp".to_string(),
		whatsnap::RINGTONE_NONE => "None".to_string(),
		path => sound_file::windows_ringtones()
			.iter()
			.find(|(_, p)| p == path)
			.map_or_else(|| file_name(path), |(name, _)| name.clone()),
	}
}
