use wxdragon::prelude::*;

use crate::ui::{
	dialog_look::{self, Field},
	theme,
};

#[allow(dead_code)]
pub struct PollDialogParts {
	pub dialog: Dialog,
	pub question_field: TextCtrl,
	pub options_field: TextCtrl,
	pub multiple_check: CheckBox,
	pub quiz_check: CheckBox,
	pub send_btn: Button,
	pub cancel_btn: Button,
}

/// "Create poll", laid out like WhatsApp's: a question, the options, and the
/// "Allow multiple answers" switch (on by default, as in WhatsApp). Options
/// are typed one per line - one plain edit field reads far better with a
/// screen reader than WhatsApp's growing list of separate boxes.
pub fn build_poll_dialog(parent: &impl WxWidget) -> PollDialogParts {
	let dialog = Dialog::builder(parent, crate::i18n::tr("Create poll"))
		.with_size(420, 400)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let question_label = StaticText::builder(&panel).with_label(crate::i18n::tr("Question")).build();
	theme::style_muted_label(&question_label);
	sizer.add(&question_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let question_field = TextCtrl::builder(&panel).with_style(dialog_look::field_style(TextCtrlStyle::Default)).build();
	dialog_look::add_field(&sizer, &question_field, Field::Line, 0);

	let options_label =
		StaticText::builder(&panel).with_label(crate::i18n::tr("Options, one per line (2 to 12)")).build();
	theme::style_muted_label(&options_label);
	sizer.add(&options_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let options_field =
		TextCtrl::builder(&panel).with_style(dialog_look::field_style(TextCtrlStyle::MultiLine)).build();
	dialog_look::add_field(&sizer, &options_field, Field::Box, 1);

	let multiple_check = CheckBox::builder(&panel).with_label(crate::i18n::tr("Allow multiple answers")).build();
	multiple_check.set_value(true);
	dialog_look::switch(&multiple_check, theme::Palette::CONTENT_BG());
	sizer.add(&multiple_check, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let quiz_check = CheckBox::builder(&panel).with_label(crate::i18n::tr("Quiz (one correct answer)")).build();
	dialog_look::switch(&quiz_check, theme::Palette::CONTENT_BG());
	sizer.add(&quiz_check, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);
	let multiple_for_quiz = multiple_check;
	quiz_check.on_toggled(move |event| multiple_for_quiz.enable(!event.is_checked()));

	let send_btn = Button::builder(&panel).with_label(crate::i18n::tr("Send")).build();
	dialog_look::pill_button(&send_btn, true, theme::Palette::CONTENT_BG());
	let cancel_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label(crate::i18n::tr("Cancel")).build();
	dialog_look::pill_button(&cancel_btn, false, theme::Palette::CONTENT_BG());
	dialog_look::button_row(&sizer, &[&send_btn, &cancel_btn]);
	panel.set_sizer(sizer, true);
	dialog_look::paint_fields(&panel, &[(question_field, Field::Line), (options_field, Field::Box)]);

	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	crate::ui::keys::escape_cancels_anywhere(dialog);

	PollDialogParts { dialog, question_field, options_field, multiple_check, quiz_check, send_btn, cancel_btn }
}
