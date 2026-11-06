use std::{cell::RefCell, rc::Rc, thread};

use wxdragon::prelude::*;

use crate::{
	platform::whats_new::{fetch_from_github, get_whats_new},
	ui::theme,
};

pub fn show_whats_new_dialog(parent: &impl WxWidget) {
	let dialog = Dialog::builder(parent, "What's New in whatSnap")
		.with_size(700, 560)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);

	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let heading = StaticText::builder(&panel).with_label("What's New in whatSnap").build();
	theme::style_sidebar_heading(&heading);
	sizer.add(&heading, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Right, 12);

	let desc = StaticText::builder(&panel)
		.with_label("Recent updates, new features, and changes fetched directly from GitHub:")
		.build();
	theme::style_muted_label(&desc);
	sizer.add(&desc, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Right, 12);

	let list_label = StaticText::builder(&panel).with_label("Release / Update History:").build();
	theme::style_muted_label(&list_label);
	sizer.add(&list_label, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let list = ListBox::builder(&panel).build();
	list.set_font(&theme::Fonts::body());
	list.set_min_size(Size::new(-1, 100));
	sizer.add(&list, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 6);

	let notes_label = StaticText::builder(&panel).with_label("Changes & Release Notes:").build();
	theme::style_muted_label(&notes_label);
	sizer.add(&notes_label, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let notes_ctrl = TextCtrl::builder(&panel).with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly).build();
	notes_ctrl.set_font(&theme::Fonts::body());
	sizer.add(&notes_ctrl, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 6);

	let announcer = StaticText::builder(&panel).with_label("").build();
	live_region::set_live_region(&announcer);
	theme::style_muted_label(&announcer);
	sizer.add(&announcer, 0, SizerFlag::Left | SizerFlag::Top, 6);

	let buttons = BoxSizer::builder(Orientation::Horizontal).build();
	let refresh_btn = Button::builder(&panel).with_label("&Refresh from GitHub").build();
	let copy_btn = Button::builder(&panel).with_label("&Copy Changes").build();
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Close").build();

	buttons.add(&refresh_btn, 0, SizerFlag::Right, 8);
	buttons.add(&copy_btn, 0, SizerFlag::Right, 8);
	buttons.add_stretch_spacer(1);
	buttons.add(&close_btn, 0, SizerFlag::empty(), 0);
	sizer.add_sizer(&buttons, 0, SizerFlag::Expand | SizerFlag::All, 12);

	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let changes = Rc::new(RefCell::new(get_whats_new()));

	let update_display = {
		let changes = changes.clone();
		let list = list;
		let notes_ctrl = notes_ctrl;
		move || {
			let chs = changes.borrow();
			let items: Vec<String> = chs
				.iter()
				.map(|c| {
					if !c.published_at.is_empty() && c.published_at != "Live from GitHub" {
						format!("{} ({})", c.name, c.published_at)
					} else {
						c.name.clone()
					}
				})
				.collect();

			let item_refs: Vec<&str> = items.iter().map(String::as_str).collect();
			list.set_items(&item_refs);

			if !chs.is_empty() {
				let sel = list.get_selection().map(|i| i as usize).unwrap_or(0).min(chs.len() - 1);
				list.set_selection(sel as u32, true);
				notes_ctrl.set_value(&chs[sel].body);
			} else {
				notes_ctrl.set_value("No release notes available.");
			}
		}
	};

	update_display();

	// Selection change in list updates notes
	let changes_on_select = changes.clone();
	let notes_on_select = notes_ctrl;
	list.on_selection_changed(move |evt| {
		if let Some(sel) = evt.get_selection() {
			let sel = sel as usize;
			let chs = changes_on_select.borrow();
			if let Some(entry) = chs.get(sel) {
				notes_on_select.set_value(&entry.body);
			}
		}
	});

	// Copy Changes
	let notes_copy = notes_ctrl;
	let announcer_copy = announcer;
	copy_btn.on_click(move |_| {
		let text = notes_copy.get_value();
		if !text.is_empty() {
			crate::copy_text(&text);
			crate::platform::announce::announce(announcer_copy, "Release changes copied to clipboard.");
		}
	});

	// Refresh from GitHub
	let changes_refresh = changes.clone();
	let update_refresh = update_display.clone();
	let announcer_refresh = announcer;
	let refresh_btn_handle = refresh_btn;
	refresh_btn.on_click(move |_| {
		crate::platform::announce::announce(announcer_refresh, "Fetching latest changes from GitHub...");
		refresh_btn_handle.enable(false);

		let (tx, rx) = std::sync::mpsc::channel();
		thread::spawn(move || {
			let res = fetch_from_github();
			let _ = tx.send(res);
		});

		// Check response or wait briefly
		if let Ok(Ok(new_changes)) = rx.recv_timeout(std::time::Duration::from_secs(6)) {
			*changes_refresh.borrow_mut() = new_changes;
			update_refresh();
			crate::platform::announce::announce(announcer_refresh, "Updated changes from GitHub.");
		} else {
			crate::platform::announce::announce(
				announcer_refresh,
				"Could not refresh from GitHub. Showing offline cached changes.",
			);
		}
		refresh_btn_handle.enable(true);
	});

	close_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));

	let escape_closes = crate::ui::keys::escape_cancels(dialog);
	list.on_key_down(escape_closes);

	list.set_focus();
	dialog.show_modal();
	dialog.destroy();
}
