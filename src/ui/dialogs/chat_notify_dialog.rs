use wxdragon::prelude::*;

use crate::ui::{dialog_look, theme};

const ALERTS: [(&str, whatsnap::ChatAlert); 5] = [
	("As set in Settings", whatsnap::ChatAlert::Default),
	("Notification and sound", whatsnap::ChatAlert::Full),
	("Notification only, no sound", whatsnap::ChatAlert::NoSound),
	("Sound only, no notification", whatsnap::ChatAlert::SoundOnly),
	("Off on this computer", whatsnap::ChatAlert::Off),
];

pub fn ask(parent: &impl WxWidget, chat: &str, current: whatsnap::ChatNotify) -> Option<whatsnap::ChatNotify> {
	let dialog = Dialog::builder(parent, &format!("Notification settings for {chat}"))
		.with_size(460, 300)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let note = StaticText::builder(&panel)
		.with_label("These only change this computer. To mute this chat on all your devices, use Mute in its menu.")
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
	};
	dialog.destroy();
	saved.then_some(notify)
}
