use wxdragon::prelude::*;

/// Every constructed widget is kept here for consistency even if not all of
/// them are read back after construction yet.
#[allow(dead_code)]
pub struct NewConversationDialogParts {
	pub dialog: Dialog,
	pub search_field: TextCtrl,
	pub results_list: ListBox,
	pub new_group_btn: Button,
	pub new_contact_btn: Button,
	pub close_btn: Button,
}

pub fn build_new_conversation_dialog(parent: &impl WxWidget) -> NewConversationDialogParts {
	let dialog = Dialog::builder(parent, "New Conversation")
		.with_size(440, 480)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();

	let panel = Panel::builder(&dialog).build();
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let search_label = StaticText::builder(&panel).with_label("Search name or number").build();
	sizer.add(&search_label, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let search_field = TextCtrl::builder(&panel).with_style(TextCtrlStyle::ProcessEnter).build();
	sizer.add(&search_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let results_list = ListBox::builder(&panel).build();
	sizer.add(&results_list, 1, SizerFlag::Expand | SizerFlag::All, 10);

	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();

	let new_group_btn = Button::builder(&panel).with_label("New Group").build();
	btn_sizer.add(&new_group_btn, 0, SizerFlag::Right, 8);

	let new_contact_btn = Button::builder(&panel).with_label("New Contact").build();
	btn_sizer.add(&new_contact_btn, 0, SizerFlag::Right, 8);

	btn_sizer.add_stretch_spacer(1);

	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Close").build();
	btn_sizer.add(&close_btn, 0, SizerFlag::empty(), 0);

	sizer.add_sizer(&btn_sizer, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom, 10);

	panel.set_sizer(sizer, true);

	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);

	dialog.center();

	let dlg_esc1 = dialog.clone();
	search_field.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key) = event {
			if key.get_key_code() == Some(27) {
				dlg_esc1.end_modal(ID_CANCEL);
				event.skip(false);
				return;
			}
		}
		event.skip(true);
	});

	let dlg_esc2 = dialog.clone();
	results_list.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key) = event {
			if key.get_key_code() == Some(27) {
				dlg_esc2.end_modal(ID_CANCEL);
				event.skip(false);
				return;
			}
		}
		event.skip(true);
	});

	NewConversationDialogParts { dialog, search_field, results_list, new_group_btn, new_contact_btn, close_btn }
}
