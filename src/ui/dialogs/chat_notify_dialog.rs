use wxdragon::prelude::*;

use crate::ui::{dialog_look, theme};

const ALERTS: [(&str, whatsnap::ChatAlert); 5] = [
	("As set in Settings", whatsnap::ChatAlert::Default),
	("Notification and sound", whatsnap::ChatAlert::Full),
	("Notification only, no sound", whatsnap::ChatAlert::NoSound),
	("Sound only, no notification", whatsnap::ChatAlert::SoundOnly),
	("Off on this computer", whatsnap::ChatAlert::Off),
];

/// WhatsApp for Windows' per-chat tone list, in its order.
const TONES: [u8; 12] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, whatsnap::TONE_NONE];

/// `default_tone` is what Default plays for this chat: the message or group
/// tone from Settings.
pub fn ask(
	parent: &impl WxWidget,
	chat: &str,
	current: whatsnap::ChatNotify,
	default_tone: u8,
	output_device: Option<String>,
) -> Option<whatsnap::ChatNotify> {
	let dialog = Dialog::builder(parent, &format!("Notification settings for {chat}"))
		.with_size(460, 360)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let note = StaticText::builder(&panel)
		.with_label("The tone is shared with WhatsApp on your other computers; the rest only change this computer. To mute this chat on all your devices, use Mute in its menu.")
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

	let tone_label = StaticText::builder(&panel).with_label("Notification tone").build();
	tone_label.set_font(&theme::Fonts::body());
	sizer.add(&tone_label, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);
	let tone_names: Vec<String> = TONES
		.iter()
		.map(|&tone| match tone {
			whatsnap::TONE_DEFAULT if default_tone != tone => {
				format!("Default ({})", whatsnap::tone_label(default_tone))
			}
			_ => whatsnap::tone_label(tone),
		})
		.collect();
	let tone_names: Vec<&str> = tone_names.iter().map(String::as_str).collect();
	let tone_combo =
		ComboBox::builder(&panel).with_string_choices(&tone_names).with_style(ComboBoxStyle::ReadOnly).build();
	tone_combo.set_font(&theme::Fonts::body());
	tone_combo.set_selection(TONES.iter().position(|&tone| tone == current.tone).unwrap_or(0) as u32);
	sizer.add(&tone_combo, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);
	let picked_tone = move || TONES[tone_combo.get_selection().map_or(0, |i| i as usize).min(TONES.len() - 1)];
	tone_combo.on_selection_changed(move |_| {
		let tone = match picked_tone() {
			whatsnap::TONE_DEFAULT => default_tone,
			tone => tone,
		};
		crate::audio::play_message_tone(tone, true, output_device.clone());
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
	let notify = whatsnap::ChatNotify {
		alert: ALERTS.get(picked).map(|(_, alert)| *alert).unwrap_or_default(),
		hide_content: hide_check.is_checked(),
		tone: picked_tone(),
	};
	dialog.destroy();
	saved.then_some(notify)
}
