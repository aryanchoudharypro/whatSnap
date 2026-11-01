//! "Share screen": what to share, asked when a screen share starts - each
//! monitor, or one application window, like WhatsApp Web's own picker.

use std::rc::Rc;

use wxdragon::prelude::*;

use crate::{
	calls::screen_capture::{ShareChoice, share_choices},
	ui::{dialog_look, theme},
};

/// Shows the picker and returns what was chosen, or `None` if it was cancelled.
/// The first choice, the main screen, is selected to start with.
pub fn pick_share_target(parent: &impl WxWidget) -> Option<ShareChoice> {
	let choices = share_choices();
	let dialog = Dialog::builder(parent, "Share screen")
		.with_size(440, 420)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let list_label = StaticText::builder(&panel).with_label("What to share").build();
	theme::style_muted_label(&list_label);
	sizer.add(&list_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let list = ListBox::builder(&panel).build();
	list.set_font(&theme::Fonts::body());
	let labels: Vec<&str> = choices.iter().map(|choice| choice.label.as_str()).collect();
	list.set_items(&labels);
	list.set_selection(0, true);
	sizer.add(&list, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let share_btn = Button::builder(&panel).with_id(ID_OK).with_label("Share").build();
	dialog_look::pill_button(&share_btn, true, theme::Palette::CONTENT_BG());
	let cancel_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Cancel").build();
	dialog_look::pill_button(&cancel_btn, false, theme::Palette::CONTENT_BG());
	dialog_look::button_row(&sizer, &[&share_btn, &cancel_btn]);
	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	share_btn.on_click(move |_| dialog.end_modal(ID_OK));
	cancel_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));
	list.on_item_double_clicked(move |_| dialog.end_modal(ID_OK));
	crate::ui::keys::on_enter(&list, Rc::default(), move || dialog.end_modal(ID_OK));
	list.on_key_down(crate::ui::keys::escape_cancels(dialog));

	list.set_focus();
	let accepted = dialog.show_modal() == ID_OK;
	let picked = if accepted { list.get_selection().map(|i| i as usize) } else { None };
	dialog.destroy();
	picked.and_then(|i| choices.into_iter().nth(i))
}
