use std::rc::Rc;

use wxdragon::prelude::*;

use crate::ui::{dialog_look, keys, theme};

pub fn show_album_items(
	parent: &impl WxWidget,
	title: &str,
	labels: &[String],
	open: impl Fn(usize) + 'static,
	save_as: impl Fn(usize) + 'static,
) {
	let dialog = Dialog::builder(parent, title)
		.with_size(420, 440)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let list_label = StaticText::builder(&panel).with_label("Items").build();
	list_label.set_font(&theme::Fonts::body_strong());
	sizer.add(&list_label, 0, SizerFlag::Left | SizerFlag::Top, 12);
	let list = ListBox::builder(&panel).build();
	list.set_font(&theme::Fonts::body());
	let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
	list.set_items(&label_refs);
	if !labels.is_empty() {
		list.set_selection(0, true);
	}
	sizer.add(&list, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let buttons = BoxSizer::builder(Orientation::Horizontal).build();
	let open_btn = Button::builder(&panel).with_label("Open").build();
	dialog_look::pill_button(&open_btn, true, theme::Palette::CONTENT_BG());
	let save_btn = Button::builder(&panel).with_label("Save as...").build();
	dialog_look::pill_button(&save_btn, false, theme::Palette::CONTENT_BG());
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Close").build();
	dialog_look::pill_button(&close_btn, false, theme::Palette::CONTENT_BG());
	buttons.add(&open_btn, 0, SizerFlag::Right, 8);
	buttons.add(&save_btn, 0, SizerFlag::Right, 8);
	buttons.add_stretch_spacer(1);
	buttons.add(&close_btn, 0, SizerFlag::empty(), 0);
	sizer.add_sizer(&buttons, 0, SizerFlag::Expand | SizerFlag::All, 12);
	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let open = Rc::new(open);
	let save_as = Rc::new(save_as);
	let count = labels.len();

	let open_click = open.clone();
	open_btn.on_click(move |_| {
		if let Some(i) = list.get_selection() {
			open_click(i as usize);
		}
	});
	let save_click = save_as.clone();
	save_btn.on_click(move |_| {
		if let Some(i) = list.get_selection() {
			save_click(i as usize);
		}
	});
	close_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));

	let open_key = open.clone();
	list.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key) = event {
			let code = key.get_key_code();
			let current = list.get_selection().map(|i| i as usize);
			if keys::is_enter(code) {
				if let Some(i) = current {
					open_key(i);
				}
				event.skip(false);
				return;
			}
			let target = if keys::is_left(code) {
				current.map(|i| i.saturating_sub(1))
			} else if keys::is_right(code) {
				current.map(|i| (i + 1).min(count.saturating_sub(1)))
			} else {
				None
			};
			if keys::is_left(code) || keys::is_right(code) {
				if let Some(t) = target {
					list.set_selection(t as u32, true);
				}
				event.skip(false);
				return;
			}
		}
		event.skip(true);
	});

	keys::escape_cancels_anywhere(dialog);
	list.set_focus();
	dialog.show_modal();
	dialog.destroy();
}
