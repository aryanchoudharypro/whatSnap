use wxdragon::prelude::*;

use crate::ui::theme;

/// Every constructed widget is kept here for consistency even if not all of
/// them are read back after construction yet (e.g. the sync checkbox).
#[allow(dead_code)]
pub struct NewContactDialogParts {
	pub dialog: Dialog,
	pub name_field: TextCtrl,
	pub surname_field: TextCtrl,
	pub phone_field: TextCtrl,
	pub sync_check: CheckBox,
	pub ok_btn: Button,
	pub cancel_btn: Button,
}

pub fn build_new_contact_dialog(parent: &impl WxWidget) -> NewContactDialogParts {
	let dialog = Dialog::builder(parent, "New Contact")
		.with_size(380, 320)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let name_label = StaticText::builder(&panel).with_label("Name").build();
	theme::style_muted_label(&name_label);
	sizer.add(&name_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let name_field = TextCtrl::builder(&panel).build();
	name_field.set_font(&theme::Fonts::body());
	sizer.add(&name_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let surname_label = StaticText::builder(&panel).with_label("Surname").build();
	theme::style_muted_label(&surname_label);
	sizer.add(&surname_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let surname_field = TextCtrl::builder(&panel).build();
	surname_field.set_font(&theme::Fonts::body());
	sizer.add(&surname_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let phone_label = StaticText::builder(&panel).with_label("Phone Number").build();
	theme::style_muted_label(&phone_label);
	sizer.add(&phone_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let phone_field = TextCtrl::builder(&panel).build();
	phone_field.set_font(&theme::Fonts::body());
	sizer.add(&phone_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let sync_check = CheckBox::builder(&panel).with_label("Sync Contact").build();
	sync_check.set_value(true);
	sync_check.set_font(&theme::Fonts::body());
	sizer.add(&sync_check, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let ok_btn = Button::builder(&panel).with_id(ID_OK).with_label("Create").build();
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

	let dlg_e1 = dialog.clone();
	name_field.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key) = event {
			if key.get_key_code() == Some(27) {
				dlg_e1.end_modal(ID_CANCEL);
				event.skip(false);
				return;
			}
		}
		event.skip(true);
	});

	let dlg_e2 = dialog.clone();
	surname_field.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key) = event {
			if key.get_key_code() == Some(27) {
				dlg_e2.end_modal(ID_CANCEL);
				event.skip(false);
				return;
			}
		}
		event.skip(true);
	});

	let dlg_e3 = dialog.clone();
	phone_field.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key) = event {
			if key.get_key_code() == Some(27) {
				dlg_e3.end_modal(ID_CANCEL);
				event.skip(false);
				return;
			}
		}
		event.skip(true);
	});

	NewContactDialogParts { dialog, name_field, surname_field, phone_field, sync_check, ok_btn, cancel_btn }
}
