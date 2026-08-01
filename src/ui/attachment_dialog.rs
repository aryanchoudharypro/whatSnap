use wxdragon::prelude::*;

pub struct AttachmentDialogParts {
	pub dialog: Dialog,
	pub attachment_info_label: StaticText,
	pub caption_label: StaticText,
	pub caption_field: TextCtrl,
	pub add_more_btn: Button,
	pub discard_btn: Button,
	pub send_btn: Button,
}

pub fn build_attachment_dialog(parent: &impl WxWidget) -> AttachmentDialogParts {
	let dialog = Dialog::builder(parent, "Send attachment")
		.with_size(420, 220)
		.with_style(DialogStyle::DefaultDialogStyle)
		.build();

	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let attachment_info_label = StaticText::builder(&dialog).with_label("").build();
	sizer.add(&attachment_info_label, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let caption_label = StaticText::builder(&dialog).with_label("Caption (optional)").build();
	sizer.add(&caption_label, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let caption_field = TextCtrl::builder(&dialog).with_style(TextCtrlStyle::ProcessEnter).build();
	sizer.add(&caption_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();

	let add_more_btn = Button::builder(&dialog).with_label("Add more files").build();
	btn_sizer.add(&add_more_btn, 0, SizerFlag::Right, 5);

	let discard_btn = Button::builder(&dialog).with_id(ID_CANCEL).with_label("Discard").build();
	btn_sizer.add(&discard_btn, 0, SizerFlag::Right, 5);

	let send_btn = Button::builder(&dialog).with_id(ID_OK).with_label("Send").build();
	btn_sizer.add(&send_btn, 0, SizerFlag::empty(), 0);

	sizer.add_sizer(&btn_sizer, 0, SizerFlag::AlignRight | SizerFlag::All, 10);

	dialog.center();

	let dlg_att = dialog.clone();
	caption_field.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key) = event {
			if key.get_key_code() == Some(27) {
				dlg_att.end_modal(ID_CANCEL);
				event.skip(false);
				return;
			}
		}
		event.skip(true);
	});

	AttachmentDialogParts {
		dialog,
		attachment_info_label,
		caption_label,
		caption_field,
		add_more_btn,
		discard_btn,
		send_btn,
	}
}
