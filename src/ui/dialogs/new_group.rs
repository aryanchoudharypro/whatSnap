use std::collections::HashSet;

use wxdragon::prelude::*;

use crate::ui::{
	dialog_look::{self, Field},
	theme,
};

pub struct NewGroupState {
	pub all_contact_labels: Vec<String>,
	pub all_contact_jids: Vec<String>,
	pub checked_jids: HashSet<String>,
	pub current_jids: Vec<String>,
}

pub struct NewGroupDialogParts {
	pub dialog: Dialog,
	pub name_field: TextCtrl,
	pub search_field: TextCtrl,
	pub contacts_listbox: CheckListBox,
	pub selection_announcer: StaticText,
	pub extra_number_field: TextCtrl,
	pub ok_btn: Button,
	pub cancel_btn: Button,
}

pub fn build_new_group_dialog(parent: &impl WxWidget) -> NewGroupDialogParts {
	let dialog = Dialog::builder(parent, "New group")
		.with_size(460, 560)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let name_label = StaticText::builder(&panel).with_label("Group name").build();
	theme::style_muted_label(&name_label);
	sizer.add(&name_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let name_field = TextCtrl::builder(&panel).with_style(dialog_look::field_style(TextCtrlStyle::Default)).build();
	dialog_look::add_field(&sizer, &name_field, Field::Line, 0);

	let contacts_label = StaticText::builder(&panel).with_label("Contacts").build();
	theme::style_heading(&contacts_label);
	sizer.add(&contacts_label, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let search_label = StaticText::builder(&panel).with_label("Search contacts").build();
	theme::style_muted_label(&search_label);
	sizer.add(&search_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let search_field = TextCtrl::builder(&panel).with_style(dialog_look::field_style(TextCtrlStyle::Default)).build();
	dialog_look::add_field(&sizer, &search_field, Field::Search, 0);

	let contacts_listbox = CheckListBox::builder(&panel).build();
	contacts_listbox.set_font(&theme::Fonts::body());
	sizer.add(&contacts_listbox, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let selection_announcer = StaticText::builder(&panel).with_label("").build();
	live_region::set_live_region(&selection_announcer);
	theme::style_muted_label(&selection_announcer);
	sizer.add(&selection_announcer, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let extra_label = StaticText::builder(&panel).with_label("Extra number (optional)").build();
	theme::style_muted_label(&extra_label);
	sizer.add(&extra_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let extra_number_field =
		TextCtrl::builder(&panel).with_style(dialog_look::field_style(TextCtrlStyle::Default)).build();
	dialog_look::add_field(&sizer, &extra_number_field, Field::Line, 0);

	let ok_btn = Button::builder(&panel).with_id(ID_OK).with_label("Create group").build();
	dialog_look::pill_button(&ok_btn, true, theme::Palette::CONTENT_BG());
	let cancel_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Cancel").build();
	dialog_look::pill_button(&cancel_btn, false, theme::Palette::CONTENT_BG());
	dialog_look::button_row(&sizer, &[&ok_btn, &cancel_btn]);
	panel.set_sizer(sizer, true);
	dialog_look::paint_fields(
		&panel,
		&[(name_field, Field::Line), (search_field, Field::Search), (extra_number_field, Field::Line)],
	);

	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let dlg_g1 = dialog;
	name_field.on_key_down(crate::ui::keys::escape_cancels(dlg_g1));

	let dlg_g2 = dialog;
	search_field.on_key_down(crate::ui::keys::escape_cancels(dlg_g2));

	let dlg_g3 = dialog;
	contacts_listbox.on_key_down(crate::ui::keys::escape_cancels(dlg_g3));

	let dlg_g4 = dialog;
	extra_number_field.on_key_down(crate::ui::keys::escape_cancels(dlg_g4));

	NewGroupDialogParts {
		dialog,
		name_field,
		search_field,
		contacts_listbox,
		selection_announcer,
		extra_number_field,
		ok_btn,
		cancel_btn,
	}
}
