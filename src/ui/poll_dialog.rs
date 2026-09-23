use wxdragon::prelude::*;

use crate::ui::theme;

/// Every constructed widget is kept here for consistency even if not all of
/// them are read back after construction (e.g. static labels).
#[allow(dead_code)]
pub struct PollDialogParts {
	pub dialog: Dialog,
	pub question_field: TextCtrl,
	pub options_field: TextCtrl,
	pub multiple_check: CheckBox,
	pub send_btn: Button,
	pub cancel_btn: Button,
}

/// "Create poll", laid out like WhatsApp's: a question, the options, and the
/// "Allow multiple answers" switch (on by default, as in WhatsApp). Options
/// are typed one per line - one plain edit field reads far better with a
/// screen reader than WhatsApp's growing list of separate boxes.
pub fn build_poll_dialog(parent: &impl WxWidget) -> PollDialogParts {
	let dialog = Dialog::builder(parent, "Create poll")
		.with_size(420, 400)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let question_label = StaticText::builder(&panel).with_label("Question").build();
	theme::style_muted_label(&question_label);
	sizer.add(&question_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let question_field = TextCtrl::builder(&panel).build();
	question_field.set_font(&theme::Fonts::body());
	sizer.add(&question_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let options_label = StaticText::builder(&panel).with_label("Options, one per line (2 to 12)").build();
	theme::style_muted_label(&options_label);
	sizer.add(&options_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let options_field = TextCtrl::builder(&panel).with_style(TextCtrlStyle::MultiLine).build();
	options_field.set_font(&theme::Fonts::body());
	sizer.add(&options_field, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let multiple_check = CheckBox::builder(&panel).with_label("Allow multiple answers").build();
	multiple_check.set_value(true);
	multiple_check.set_font(&theme::Fonts::body());
	sizer.add(&multiple_check, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let send_btn = Button::builder(&panel).with_label("Send").build();
	theme::style_primary_button(&send_btn);
	btn_sizer.add(&send_btn, 0, SizerFlag::Right, 8);
	btn_sizer.add_stretch_spacer(1);
	let cancel_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Cancel").build();
	btn_sizer.add(&cancel_btn, 0, SizerFlag::Right, 8);

	sizer.add_sizer(&btn_sizer, 0, SizerFlag::Expand | SizerFlag::All, 10);
	panel.set_sizer(sizer, true);

	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	// Escape cancels from any field, the options box included (a multi-line
	// edit would otherwise keep it).
	let dlg_esc = dialog.clone();
	dialog.bind_internal(EventType::CHAR_HOOK, move |event| {
		let data = WindowEventData::new(event);
		if let WindowEventData::Keyboard(ref key) = data {
			if key.get_key_code() == Some(27) {
				dlg_esc.end_modal(ID_CANCEL);
				data.skip(false);
				return;
			}
		}
		data.skip(true);
	});

	PollDialogParts { dialog, question_field, options_field, multiple_check, send_btn, cancel_btn }
}
