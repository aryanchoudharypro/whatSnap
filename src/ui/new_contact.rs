use wxdragon::prelude::*;

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
	let dialog = Dialog::builder(parent, "New Contact").with_size(380, 320).with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder).build();
	let panel = Panel::builder(&dialog).build();
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let name_label = StaticText::builder(&panel).with_label("Name").build();
	sizer.add(&name_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let name_field = TextCtrl::builder(&panel).build();
	sizer.add(&name_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let surname_label = StaticText::builder(&panel).with_label("Surname").build();
	sizer.add(&surname_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let surname_field = TextCtrl::builder(&panel).build();
	sizer.add(&surname_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let phone_label = StaticText::builder(&panel).with_label("Phone Number").build();
	sizer.add(&phone_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let phone_field = TextCtrl::builder(&panel).build();
	sizer.add(&phone_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let sync_check = CheckBox::builder(&panel).with_label("Sync Contact").build();
	sync_check.set_value(true);
	sizer.add(&sync_check, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let ok_btn = Button::builder(&panel).with_label("Create").build();
	btn_sizer.add(&ok_btn, 0, SizerFlag::Right, 8);
	btn_sizer.add_stretch_spacer(1);
	let cancel_btn = Button::builder(&panel).with_label("Cancel").build();
	btn_sizer.add(&cancel_btn, 0, SizerFlag::Right, 8);

	sizer.add_sizer(&btn_sizer, 0, SizerFlag::Expand | SizerFlag::All, 10);
	panel.set_sizer(sizer, true);

	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	NewContactDialogParts {
		dialog,
		name_field,
		surname_field,
		phone_field,
		sync_check,
		ok_btn,
		cancel_btn,
	}
}
