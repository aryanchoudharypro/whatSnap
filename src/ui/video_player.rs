//! The video window: plays a chat's video inside whatSnap instead of handing
//! it to another app, like WhatsApp Web's media viewer.
//!
//! Keys besides the player's own (see `media_player.rs`): Space plays or
//! pauses unless a button has focus, Left/Right go back or forward 5 seconds,
//! Up/Down change the volume, Escape closes.

use std::{cell::Cell, rc::Rc};

use wxdragon::prelude::*;

use crate::ui::{keys, media_player::MediaPlayer, theme};

/// What the user asked for when the window closed.
pub(crate) enum VideoChoice {
	Closed,
	/// Play it in the app Windows uses for videos instead.
	OpenElsewhere,
}

/// Shows the video at `path` modally. `title` names it ("Video from Sam"),
/// `save_name` is the name Save as suggests.
pub(crate) fn show(parent: &impl WxWidget, path: &str, title: &str, save_name: &str) -> VideoChoice {
	let dialog = Dialog::builder(parent, title)
		.with_size(760, 620)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder | DialogStyle::MaximizeBox)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	// A dialog is its own foreground window: announcements have to come from
	// a live region inside it to be spoken.
	let announcer = StaticText::builder(&panel).with_label("").build();
	live_region::set_live_region(&announcer);
	sizer.add(&announcer, 0, SizerFlag::All, 0);

	let player = MediaPlayer::build(&panel, &sizer, announcer);

	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let save_btn = Button::builder(&panel).with_label("&Save as...").build();
	btn_sizer.add(&save_btn, 0, SizerFlag::Right, 8);
	let elsewhere_btn = Button::builder(&panel).with_label("Open in &another app").build();
	btn_sizer.add(&elsewhere_btn, 0, SizerFlag::Right, 8);
	btn_sizer.add_stretch_spacer(1);
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Close").build();
	btn_sizer.add(&close_btn, 0, SizerFlag::Left, 0);
	sizer.add_sizer(&btn_sizer, 0, SizerFlag::Expand | SizerFlag::All, 10);

	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let elsewhere = Rc::new(Cell::new(false));
	let elsewhere_click = elsewhere.clone();
	elsewhere_btn.on_click(move |_| {
		elsewhere_click.set(true);
		dialog.end_modal(ID_CANCEL);
	});
	close_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));
	let source = path.to_string();
	let save_name = save_name.to_string();
	let player_save = player.clone();
	save_btn.on_click(move |_| {
		let was_playing = player_save.is_playing();
		player_save.pause();
		save_copy(&dialog, &source, &save_name, announcer);
		if was_playing {
			player_save.play();
		}
	});

	let buttons = [save_btn.get_handle(), elsewhere_btn.get_handle(), close_btn.get_handle()];
	let player_keys = player.clone();
	dialog.bind_internal(EventType::CHAR_HOOK, move |event| {
		let data = WindowEventData::new(event);
		let WindowEventData::Keyboard(ref key) = data else { return };
		let code = key.get_key_code();
		if keys::is_escape(code) {
			dialog.end_modal(ID_CANCEL);
			data.skip(false);
			return;
		}
		if player_keys.handle_key(key) {
			data.skip(false);
			return;
		}
		let focus = crate::ui::media_player::focused_window();
		let plain = !key.control_down() && !key.alt_down();
		let handled = plain
			&& if keys::is_space(code) && !buttons.contains(&focus) && !player_keys.owns_arrows(focus) {
				player_keys.toggle_play();
				true
			} else if keys::is_left(code) {
				player_keys.seek_by(-5.0);
				true
			} else if keys::is_right(code) {
				player_keys.seek_by(5.0);
				true
			} else if keys::is_up(code) {
				player_keys.change_volume(10);
				true
			} else if keys::is_down(code) {
				player_keys.change_volume(-10);
				true
			} else {
				false
			};
		data.skip(!handled);
	});

	player.load(path, true);
	player.focus();
	dialog.show_modal();
	player.stop();
	// A dialog built per use must be destroyed, or it and every closure bound
	// to it live on hidden.
	dialog.destroy();
	if elsewhere.get() { VideoChoice::OpenElsewhere } else { VideoChoice::Closed }
}

/// Asks where to save a copy of `source` and copies it there, saying how it went.
pub(crate) fn save_copy(parent: &impl WxWidget, source: &str, name: &str, announcer: StaticText) {
	let dialog = FileDialog::builder(parent)
		.with_message("Save file as")
		.with_default_file(name)
		.with_style(FileDialogStyle::Save | FileDialogStyle::OverwritePrompt)
		.build();
	if dialog.show_modal() != wxdragon::id::ID_OK {
		return;
	}
	let Some(dest) = dialog.get_path() else { return };
	let text = match std::fs::copy(source, &dest) {
		Ok(_) => format!("File saved to {dest}"),
		Err(err) => format!("Couldn't save the file: {err}"),
	};
	crate::platform::announce::announce(announcer, &text);
}
