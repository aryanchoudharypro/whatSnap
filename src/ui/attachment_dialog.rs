use wxdragon::prelude::*;

use crate::ui::{dialog_look, theme};

/// Every constructed widget is kept here for consistency even if not all of
/// them are read back after construction yet (e.g. static labels).
#[allow(dead_code)]
pub struct AttachmentDialogParts {
	pub dialog: Dialog,
	pub attachment_info_label: StaticText,
	pub caption_label: StaticText,
	pub caption_field: TextCtrl,
	/// Photos and videos only: open once, then gone.
	pub view_once_check: CheckBox,
	pub add_more_btn: Button,
	pub discard_btn: Button,
	pub send_btn: Button,
}

pub fn build_attachment_dialog(parent: &impl WxWidget) -> AttachmentDialogParts {
	let dialog = Dialog::builder(parent, "Send attachment")
		.with_size(420, 220)
		.with_style(DialogStyle::DefaultDialogStyle)
		.build();
	theme::style_content(&dialog);

	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let attachment_info_label = StaticText::builder(&dialog).with_label("").build();
	theme::style_muted_label(&attachment_info_label);
	sizer.add(&attachment_info_label, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let caption_label = StaticText::builder(&dialog).with_label("Caption (optional)").build();
	theme::style_muted_label(&caption_label);
	sizer.add(&caption_label, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let caption_field = TextCtrl::builder(&dialog).with_style(TextCtrlStyle::ProcessEnter).build();
	caption_field.set_font(&theme::Fonts::body());
	sizer.add(&caption_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let view_once_check = CheckBox::builder(&dialog).with_label("&View once").build();
	dialog_look::switch(&view_once_check, theme::Palette::CONTENT_BG);
	sizer.add(&view_once_check, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();

	let add_more_btn = Button::builder(&dialog).with_label("Add more files").build();
	dialog_look::pill_button(&add_more_btn, false, theme::Palette::CONTENT_BG);
	btn_sizer.add(&add_more_btn, 0, SizerFlag::Right, 6);

	let discard_btn = Button::builder(&dialog).with_id(ID_CANCEL).with_label("Discard").build();
	dialog_look::pill_button(&discard_btn, false, theme::Palette::CONTENT_BG);
	btn_sizer.add(&discard_btn, 0, SizerFlag::Right, 6);

	let send_btn = Button::builder(&dialog).with_id(ID_OK).with_label("Send").build();
	dialog_look::pill_button(&send_btn, true, theme::Palette::CONTENT_BG);
	btn_sizer.add(&send_btn, 0, SizerFlag::empty(), 0);

	sizer.add_sizer(&btn_sizer, 0, SizerFlag::AlignRight | SizerFlag::All, 12);

	dialog.center();

	// CHAR_HOOK fires on this dialog (a top-level window) before the
	// keypress reaches whichever control has focus, so Escape discards the
	// attachment regardless of what's focused, not just caption_field.
	crate::ui::keys::escape_cancels_anywhere(dialog);

	AttachmentDialogParts {
		dialog,
		attachment_info_label,
		caption_label,
		caption_field,
		view_once_check,
		add_more_btn,
		discard_btn,
		send_btn,
	}
}
