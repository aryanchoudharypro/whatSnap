use wxdragon::prelude::*;

#[derive(Clone, Copy)]
pub struct AttachmentComposerParts {
	pub panel: Panel,
	pub attachment_info_label: StaticText,
	pub _caption_label: StaticText,
	pub caption_field: TextCtrl,
	pub add_more_btn: Button,
	pub discard_btn: Button,
	pub send_btn: Button,
}

pub fn build_attachment_composer(parent: &impl WxWidget) -> AttachmentComposerParts {
	let panel = Panel::builder(parent).build();
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let attachment_info_label = StaticText::builder(&panel).with_label("").build();
	sizer.add(&attachment_info_label, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let caption_label = StaticText::builder(&panel).with_label(crate::i18n::tr("Caption (optional)")).build();
	sizer.add(&caption_label, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let caption_field = TextCtrl::builder(&panel)
		.with_style(TextCtrlStyle::ProcessEnter)
		.build();
	sizer.add(&caption_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();

	let add_more_btn = Button::builder(&panel).with_label(crate::i18n::tr("Add more files")).build();
	btn_sizer.add(&add_more_btn, 0, SizerFlag::Right, 5);

	let discard_btn = Button::builder(&panel).with_label(crate::i18n::tr("Discard")).build();
	btn_sizer.add(&discard_btn, 0, SizerFlag::Right, 5);

	let send_btn = Button::builder(&panel).with_label(crate::i18n::tr("Send")).build();
	btn_sizer.add(&send_btn, 0, SizerFlag::empty(), 0);

	sizer.add_sizer(&btn_sizer, 0, SizerFlag::AlignRight | SizerFlag::All, 10);

	panel.set_sizer(sizer, true);

	AttachmentComposerParts {
		panel,
		attachment_info_label,
		_caption_label: caption_label,
		caption_field,
		add_more_btn,
		discard_btn,
		send_btn,
	}
}
