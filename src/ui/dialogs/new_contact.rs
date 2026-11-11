use wxdragon::prelude::*;

use crate::ui::{
	dialog_look::{self, Field},
	theme,
};

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
	let dialog = Dialog::builder(parent, crate::i18n::tr("New Contact"))
		.with_size(380, 320)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let name_label = StaticText::builder(&panel).with_label(crate::i18n::tr("Name")).build();
	theme::style_muted_label(&name_label);
	sizer.add(&name_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let name_field = TextCtrl::builder(&panel).with_style(dialog_look::field_style(TextCtrlStyle::Default)).build();
	dialog_look::add_field(&sizer, &name_field, Field::Line, 0);

	let surname_label = StaticText::builder(&panel).with_label(crate::i18n::tr("Surname")).build();
	theme::style_muted_label(&surname_label);
	sizer.add(&surname_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let surname_field = TextCtrl::builder(&panel).with_style(dialog_look::field_style(TextCtrlStyle::Default)).build();
	dialog_look::add_field(&sizer, &surname_field, Field::Line, 0);

	let phone_label = StaticText::builder(&panel).with_label(crate::i18n::tr("Phone Number")).build();
	theme::style_muted_label(&phone_label);
	sizer.add(&phone_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let phone_field = TextCtrl::builder(&panel).with_style(dialog_look::field_style(TextCtrlStyle::Default)).build();
	dialog_look::add_field(&sizer, &phone_field, Field::Line, 0);

	let sync_check =
		CheckBox::builder(&panel).with_label(crate::i18n::tr("Also save to my phone's WhatsApp contacts")).build();
	sync_check.set_value(true);
	dialog_look::switch(&sync_check, theme::Palette::CONTENT_BG());
	sizer.add(&sync_check, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 14);

	let ok_btn = Button::builder(&panel).with_id(ID_OK).with_label(crate::i18n::tr("Create")).build();
	dialog_look::pill_button(&ok_btn, true, theme::Palette::CONTENT_BG());
	let cancel_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label(crate::i18n::tr("Cancel")).build();
	dialog_look::pill_button(&cancel_btn, false, theme::Palette::CONTENT_BG());
	dialog_look::button_row(&sizer, &[&ok_btn, &cancel_btn]);
	panel.set_sizer(sizer, true);
	dialog_look::paint_fields(
		&panel,
		&[(name_field, Field::Line), (surname_field, Field::Line), (phone_field, Field::Line)],
	);

	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let dlg_e1 = dialog;
	name_field.on_key_down(crate::ui::keys::escape_cancels(dlg_e1));

	let dlg_e2 = dialog;
	surname_field.on_key_down(crate::ui::keys::escape_cancels(dlg_e2));

	let dlg_e3 = dialog;
	phone_field.on_key_down(crate::ui::keys::escape_cancels(dlg_e3));

	NewContactDialogParts { dialog, name_field, surname_field, phone_field, sync_check, ok_btn, cancel_btn }
}
