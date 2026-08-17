use wxdragon::prelude::*;

use crate::ui::theme;

/// Every constructed widget is kept here for consistency, matching this
/// codebase's convention for dialog parts structs.
#[allow(dead_code)]
pub struct SettingsDialogParts {
	pub dialog: Dialog,
	pub sounds_check: CheckBox,
	pub notifications_check: CheckBox,
	pub announce_typing_check: CheckBox,
	pub announce_recording_check: CheckBox,
	pub tray_icon_check: CheckBox,
	pub autostart_check: CheckBox,
	pub ok_btn: Button,
	pub cancel_btn: Button,
}

pub fn build_settings_dialog(parent: &impl WxWidget) -> SettingsDialogParts {
	let dialog = Dialog::builder(parent, "Settings")
		.with_size(400, 400)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let sounds_check = CheckBox::builder(&panel).with_label("Play a sound for new messages").build();
	sounds_check.set_font(&theme::Fonts::body());
	sizer.add(&sounds_check, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let notifications_check = CheckBox::builder(&panel).with_label("Show desktop notifications").build();
	notifications_check.set_font(&theme::Fonts::body());
	sizer.add(&notifications_check, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let announce_typing_check = CheckBox::builder(&panel).with_label("Announce when someone is typing").build();
	announce_typing_check.set_font(&theme::Fonts::body());
	sizer.add(&announce_typing_check, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let announce_recording_check =
		CheckBox::builder(&panel).with_label("Announce when someone is recording a voice message").build();
	announce_recording_check.set_font(&theme::Fonts::body());
	sizer.add(&announce_recording_check, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let tray_icon_check = CheckBox::builder(&panel)
		.with_label("Show an icon in the notification area; closing the window minimizes to it")
		.build();
	tray_icon_check.set_font(&theme::Fonts::body());
	sizer.add(&tray_icon_check, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let autostart_check = CheckBox::builder(&panel).with_label("Start whaNTV when Windows starts").build();
	autostart_check.set_font(&theme::Fonts::body());
	sizer.add(&autostart_check, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let ok_btn = Button::builder(&panel).with_id(ID_OK).with_label("Save").build();
	theme::style_primary_button(&ok_btn);
	btn_sizer.add(&ok_btn, 0, SizerFlag::Right, 8);
	btn_sizer.add_stretch_spacer(1);
	let cancel_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Cancel").build();
	btn_sizer.add(&cancel_btn, 0, SizerFlag::Right, 8);

	sizer.add_sizer(&btn_sizer, 0, SizerFlag::Expand | SizerFlag::All, 10);
	panel.set_sizer(sizer, true);

	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
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
		sounds_check,
		notifications_check,
		announce_typing_check,
		announce_recording_check,
		tray_icon_check,
		autostart_check,
		ok_btn,
		cancel_btn,
	}
}
