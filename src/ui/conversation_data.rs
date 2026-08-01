use wxdragon::prelude::*;

#[derive(Clone)]
pub struct ConversationDataDialogParts {
	pub dialog: Dialog,
	pub info_field: TextCtrl,
	pub close_btn: Button,
}

pub fn build_conversation_data_dialog(parent: &impl WxWidget) -> ConversationDataDialogParts {
	let dialog = Dialog::builder(parent, "Conversation Data").with_size(400, 300).build();
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let info_field = TextCtrl::builder(&dialog).with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly).build();
	sizer.add(&info_field, 1, SizerFlag::Expand | SizerFlag::All, 10);

	let close_btn = Button::builder(&dialog).with_id(ID_CANCEL).with_label("Close").build();
	sizer.add(&close_btn, 0, SizerFlag::AlignCenterHorizontal | SizerFlag::Bottom, 10);

	dialog.set_sizer(sizer, true);

	let dlg_info = dialog.clone();
	info_field.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key) = event {
			if key.get_key_code() == Some(27) {
				dlg_info.end_modal(ID_CANCEL);
				event.skip(false);
				return;
			}
		}
		event.skip(true);
	});

	ConversationDataDialogParts { dialog, info_field, close_btn }
}
