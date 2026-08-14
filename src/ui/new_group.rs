use std::collections::HashSet;

use wxdragon::prelude::*;

use crate::ui::theme;

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
	let name_field = TextCtrl::builder(&panel).build();
	name_field.set_font(&theme::Fonts::body());
	sizer.add(&name_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let contacts_label = StaticText::builder(&panel).with_label("Contacts").build();
	theme::style_heading(&contacts_label);
	sizer.add(&contacts_label, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let search_label = StaticText::builder(&panel).with_label("Search contacts").build();
	theme::style_muted_label(&search_label);
	sizer.add(&search_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let search_field = TextCtrl::builder(&panel).build();
	search_field.set_font(&theme::Fonts::body());
	sizer.add(&search_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let contacts_listbox = CheckListBox::builder(&panel).build();
	contacts_listbox.set_font(&theme::Fonts::body());
	sizer.add(&contacts_listbox, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let extra_label = StaticText::builder(&panel).with_label("Extra number (optional)").build();
	theme::style_muted_label(&extra_label);
	sizer.add(&extra_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let extra_number_field = TextCtrl::builder(&panel).build();
	extra_number_field.set_font(&theme::Fonts::body());
	sizer.add(&extra_number_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let ok_btn = Button::builder(&panel).with_id(ID_OK).with_label("Create group").build();
	theme::style_primary_button(&ok_btn);
	let cancel_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Cancel").build();
	btn_sizer.add(&ok_btn, 0, SizerFlag::Right, 8);
	btn_sizer.add_stretch_spacer(1);
	btn_sizer.add(&cancel_btn, 0, SizerFlag::Right, 8);

	sizer.add_sizer(&btn_sizer, 0, SizerFlag::Expand | SizerFlag::All, 10);
	panel.set_sizer(sizer, true);

	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let dlg_g1 = dialog.clone();
	name_field.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key) = event {
			if key.get_key_code() == Some(27) {
				dlg_g1.end_modal(ID_CANCEL);
				event.skip(false);
				return;
			}
		}
		event.skip(true);
	});

	let dlg_g2 = dialog.clone();
	search_field.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key) = event {
			if key.get_key_code() == Some(27) {
				dlg_g2.end_modal(ID_CANCEL);
				event.skip(false);
				return;
			}
		}
		event.skip(true);
	});

	let dlg_g3 = dialog.clone();
	contacts_listbox.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key) = event {
			if key.get_key_code() == Some(27) {
				dlg_g3.end_modal(ID_CANCEL);
				event.skip(false);
				return;
			}
		}
		event.skip(true);
	});

	let dlg_g4 = dialog.clone();
	extra_number_field.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key) = event {
			if key.get_key_code() == Some(27) {
				dlg_g4.end_modal(ID_CANCEL);
				event.skip(false);
				return;
			}
		}
		event.skip(true);
	});

	NewGroupDialogParts { dialog, name_field, search_field, contacts_listbox, extra_number_field, ok_btn, cancel_btn }
}
