use std::{cell::Cell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState,
	ui::{dialog_look, keys, theme, window::WindowParts},
};

pub const STATUS_PALETTE: [(&str, u32); 21] = [
	("Colour 1", 0xFFA6_2C71),
	("Colour 2", 0xFF90_A841),
	("Colour 3", 0xFFC1_A03F),
	("Colour 4", 0xFF79_2138),
	("Colour 5", 0xFFAE_8774),
	("Colour 6", 0xFFF0_B330),
	("Colour 7", 0xFFB6_B327),
	("Colour 8", 0xFFC6_9FCC),
	("Colour 9", 0xFF8B_6990),
	("Colour 10", 0xFFFF_8A8C),
	("Colour 11", 0xFF54_C265),
	("Colour 12", 0xFFFF_7B6B),
	("Colour 13", 0xFF26_C4DC),
	("Colour 14", 0xFF57_C9FF),
	("Colour 15", 0xFF74_676A),
	("Colour 16", 0xFF7E_90A3),
	("Colour 17", 0xFF56_96FF),
	("Colour 18", 0xFF6E_257E),
	("Colour 19", 0xFF7A_CBA5),
	("Colour 20", 0xFF24_3640),
	("Colour 21", 0xFF82_94CA),
];

pub const STATUS_FONTS: [(&str, i32, &str); 3] = [
	("Sans-Serif (Default)", 0, "Segoe UI"),
	("Serif", 1, "Georgia"),
	("Script", 2, "Segoe Script"),
];

pub const STATUS_TEXT_MAX_CHARS: usize = 700;
pub const STATUS_TEXT_MAX_LINES: usize = 10;
pub const STATUS_CAPTION_MAX_CHARS: usize = 1024;

/// Cuts the field back to the limits WhatsApp Web enforces; true when something was cut.
fn clamp_field(field: &TextCtrl, max_chars: usize, max_lines: usize) -> bool {
	let value = field.get_value();
	let mut kept = String::new();
	let mut lines = 1usize;
	let mut count = 0usize;
	let mut cut = false;
	for ch in value.chars() {
		if ch == '\n' {
			if lines >= max_lines {
				cut = true;
				break;
			}
			lines += 1;
		}
		if count >= max_chars {
			cut = true;
			break;
		}
		kept.push(ch);
		count += 1;
	}
	if cut {
		field.set_value(&kept);
		field.set_insertion_point_end();
	}
	cut
}

const POPULAR_EMOJIS: &[(&str, &str)] = &[
	("❤️", "Red Heart"),
	("😂", "Face with Tears of Joy"),
	("😍", "Heart Eyes"),
	("🔥", "Fire"),
	("👍", "Thumbs Up"),
	("🙏", "Folded Hands"),
	("🎉", "Party Popper"),
	("✨", "Sparkles"),
	("👏", "Clapping Hands"),
	("💯", "Hundred Points"),
	("😊", "Smiling Face"),
	("😎", "Sunglasses"),
	("☕", "Hot Beverage"),
	("🎂", "Birthday Cake"),
];

fn argb_colour(colour: u32) -> Colour {
	Colour::rgb((colour >> 16) as u8, (colour >> 8) as u8, colour as u8)
}

pub fn show_status_composer_choice(
	parent: &impl WxWidget,
	state: &Rc<std::cell::RefCell<AppState>>,
	parts: &Rc<WindowParts>,
) {
	let choices = ["Text status update...", "Photo or video status update..."];
	let dialog =
		SingleChoiceDialog::builder(parent, "Choose what kind of status update to post:", "Post Status", &choices)
			.build();
	if dialog.show_modal() != wxdragon::id::ID_OK {
		return;
	}
	match dialog.get_selection() {
		0 => show_text_status_composer(parent, state, parts),
		1 => show_media_status_composer(parent, state, parts),
		_ => {}
	}
}

pub fn show_text_status_composer(
	parent: &impl WxWidget,
	state: &Rc<std::cell::RefCell<AppState>>,
	parts: &Rc<WindowParts>,
) {
	let dialog = Dialog::builder(parent, "Post Text Status")
		.with_size(580, 560)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let announcer = StaticText::builder(&panel).with_label("").build();
	live_region::set_live_region(&announcer);
	sizer.add(&announcer, 0, SizerFlag::All, 0);

	let prompt = StaticText::builder(&panel).with_label("&Type a status update:").build();
	theme::style_heading(&prompt);
	sizer.add(&prompt, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Right, 12);

	let text_field = TextCtrl::builder(&panel)
		.with_style(dialog_look::field_style(TextCtrlStyle::MultiLine | TextCtrlStyle::Rich2))
		.build();
	text_field.set_font(&theme::Fonts::heading());
	text_field.set_min_size(Size::new(-1, 130));
	sizer.add(&text_field, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);

	let count_row = BoxSizer::builder(Orientation::Horizontal).build();
	let count_label = StaticText::builder(&panel).with_label("0 / 700 characters").build();
	theme::style_muted_label(&count_label);
	count_row.add(&count_label, 0, SizerFlag::Left | SizerFlag::AlignCenterVertical, 0);
	count_row.add_stretch_spacer(1);

	let emoji_btn = Button::builder(&panel).with_label("Insert &Emoji...").build();
	dialog_look::pill_button(&emoji_btn, false, theme::Palette::CONTENT_BG());
	count_row.add(&emoji_btn, 0, SizerFlag::AlignCenterVertical, 0);
	sizer.add_sizer(&count_row, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 6);

	// Style controls row (Color & Font)
	let style_row = BoxSizer::builder(Orientation::Horizontal).build();

	let color_label = StaticText::builder(&panel).with_label("Colour:").build();
	theme::style_muted_label(&color_label);
	style_row.add(&color_label, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 6);

	let color_names: Vec<&str> = STATUS_PALETTE.iter().map(|(name, _)| *name).collect();
	let color_combo = ComboBox::builder(&panel)
		.with_choices(color_names.iter().map(|s| s.to_string()).collect())
		.with_style(ComboBoxStyle::ReadOnly)
		.build();
	color_combo.set_selection(0);
	style_row.add(&color_combo, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 6);

	let cycle_color_btn = Button::builder(&panel).with_label("&Next Colour").build();
	dialog_look::pill_button(&cycle_color_btn, false, theme::Palette::CONTENT_BG());
	style_row.add(&cycle_color_btn, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 10);

	let font_label = StaticText::builder(&panel).with_label("Font:").build();
	theme::style_muted_label(&font_label);
	style_row.add(&font_label, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 6);

	let font_names: Vec<&str> = STATUS_FONTS.iter().map(|(name, _, _)| *name).collect();
	let font_combo = ComboBox::builder(&panel)
		.with_choices(font_names.iter().map(|s| s.to_string()).collect())
		.with_style(ComboBoxStyle::ReadOnly)
		.build();
	font_combo.set_selection(0);
	style_row.add(&font_combo, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 6);

	let cycle_font_btn = Button::builder(&panel).with_label("Next &Font").build();
	dialog_look::pill_button(&cycle_font_btn, false, theme::Palette::CONTENT_BG());
	style_row.add(&cycle_font_btn, 0, SizerFlag::AlignCenterVertical, 0);

	sizer.add_sizer(&style_row, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	// Preview box
	let preview_panel = Panel::builder(&panel).build();
	preview_panel.set_min_size(Size::new(-1, 80));
	sizer.add(&preview_panel, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let privacy_row = BoxSizer::builder(Orientation::Horizontal).build();
	let privacy_hint = StaticText::builder(&panel).with_label("Visible to your configured status audience.").build();
	theme::style_muted_label(&privacy_hint);
	privacy_row.add(&privacy_hint, 1, SizerFlag::AlignCenterVertical, 0);
	let privacy_btn = Button::builder(&panel).with_label("Status &Privacy...").build();
	dialog_look::pill_button(&privacy_btn, false, theme::Palette::CONTENT_BG());
	privacy_row.add(&privacy_btn, 0, SizerFlag::AlignCenterVertical | SizerFlag::Left, 8);
	sizer.add_sizer(&privacy_row, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let btn_row = BoxSizer::builder(Orientation::Horizontal).build();
	btn_row.add_stretch_spacer(1);
	let cancel_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Cancel").build();
	dialog_look::pill_button(&cancel_btn, false, theme::Palette::CONTENT_BG());
	btn_row.add(&cancel_btn, 0, SizerFlag::Right, 8);
	let post_btn = Button::builder(&panel).with_id(ID_OK).with_label("Post &Status").build();
	dialog_look::pill_button(&post_btn, true, theme::Palette::CONTENT_BG());
	btn_row.add(&post_btn, 0, SizerFlag::Right, 0);
	sizer.add_sizer(&btn_row, 0, SizerFlag::Expand | SizerFlag::All, 12);

	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let selected_color_idx = Rc::new(Cell::new(0usize));
	let selected_font_idx = Rc::new(Cell::new(0usize));

	let update_preview = {
		let preview = preview_panel;
		let color_idx = selected_color_idx.clone();
		let font_idx = selected_font_idx.clone();
		let field = text_field;
		Rc::new(move || {
			let idx = color_idx.get();
			let (_, bg) = STATUS_PALETTE[idx];
			let c = argb_colour(bg);
			preview.set_background_color(c);
			let dc_prev = preview;
			dc_prev.set_background_style(BackgroundStyle::Paint);
			let text = field.get_value();
			let (_, _, face) = STATUS_FONTS[font_idx.get()];
			let font =
				Font::builder().with_point_size(14).with_face_name(face).build().unwrap_or_else(theme::Fonts::heading);
			field.set_font(&font);
			let f_clone = font.clone();
			preview.on_paint(move |_| {
				let dc = AutoBufferedPaintDC::new(&dc_prev);
				let size = dc_prev.get_client_size();
				dc.set_brush(c, BrushStyle::Solid);
				dc.set_pen(c, 1, PenStyle::Solid);
				dc.draw_rounded_rectangle(0, 0, size.width, size.height, 6.0);
				dc.set_font(&f_clone);
				dc.set_text_foreground(Colour::rgb(0xFF, 0xFF, 0xFF));
				let preview_text = if text.trim().is_empty() { "Preview" } else { text.trim() };
				let (tw, th) = dc.get_text_extent(preview_text);
				let x = ((size.width - tw) / 2).max(10);
				let y = ((size.height - th) / 2).max(6);
				dc.draw_text(preview_text, x, y);
			});
			preview.refresh(true, None);
		})
	};

	(update_preview)();

	{
		let color_idx = selected_color_idx.clone();
		let combo = color_combo;
		let preview_fn = update_preview.clone();
		let ann = announcer;
		cycle_color_btn.on_click(move |_| {
			let next = (color_idx.get() + 1) % STATUS_PALETTE.len();
			color_idx.set(next);
			combo.set_selection(next as u32);
			let (name, _) = STATUS_PALETTE[next];
			crate::platform::announce::announce(ann, &format!("Background colour: {name}"));
			preview_fn();
		});
	}

	{
		let color_idx = selected_color_idx.clone();
		let preview_fn = update_preview.clone();
		let ann = announcer;
		color_combo.on_selection_changed(move |event| {
			if let Some(sel) = event.get_selection() {
				let idx = (sel as usize).min(STATUS_PALETTE.len() - 1);
				color_idx.set(idx);
				let (name, _) = STATUS_PALETTE[idx];
				crate::platform::announce::announce(ann, &format!("Background colour: {name}"));
				preview_fn();
			}
		});
	}

	{
		let font_idx = selected_font_idx.clone();
		let combo = font_combo;
		let preview_fn = update_preview.clone();
		let ann = announcer;
		cycle_font_btn.on_click(move |_| {
			let next = (font_idx.get() + 1) % STATUS_FONTS.len();
			font_idx.set(next);
			combo.set_selection(next as u32);
			let (name, _, _) = STATUS_FONTS[next];
			crate::platform::announce::announce(ann, &format!("Font style: {name}"));
			preview_fn();
		});
	}

	{
		let font_idx = selected_font_idx.clone();
		let preview_fn = update_preview.clone();
		let ann = announcer;
		font_combo.on_selection_changed(move |event| {
			if let Some(sel) = event.get_selection() {
				let idx = (sel as usize).min(STATUS_FONTS.len() - 1);
				font_idx.set(idx);
				let (name, _, _) = STATUS_FONTS[idx];
				crate::platform::announce::announce(ann, &format!("Font style: {name}"));
				preview_fn();
			}
		});
	}

	{
		let count = count_label;
		let field = text_field;
		let preview_fn = update_preview.clone();
		let ann = announcer;
		text_field.on_text_changed(move |_| {
			crate::ui::emoji_replace::text_changed(&field, true);
			if clamp_field(&field, STATUS_TEXT_MAX_CHARS, STATUS_TEXT_MAX_LINES) {
				crate::platform::announce::announce(
					ann,
					"Your status update can't exceed 700 characters or 10 lines.",
				);
			}
			let len = field.get_value().chars().count();
			count.set_label(&format!("{len} / 700 characters"));
			preview_fn();
		});
	}

	{
		let dialog_emoji = dialog;
		let field_emoji = text_field;
		let ann_emoji = announcer;
		let preview_fn = update_preview.clone();
		emoji_btn.on_click(move |_| {
			let choices: Vec<String> = POPULAR_EMOJIS.iter().map(|(e, desc)| format!("{e}  {desc}")).collect();
			let refs: Vec<&str> = choices.iter().map(String::as_str).collect();
			let dlg =
				SingleChoiceDialog::builder(&dialog_emoji, "Choose an emoji to insert:", "Insert Emoji", &refs).build();
			if dlg.show_modal() == wxdragon::id::ID_OK {
				let sel = dlg.get_selection();
				if sel >= 0 && (sel as usize) < POPULAR_EMOJIS.len() {
					let (emoji, desc) = POPULAR_EMOJIS[sel as usize];
					let cur = field_emoji.get_value();
					field_emoji.set_value(&format!("{cur}{emoji}"));
					crate::platform::announce::announce(ann_emoji, &format!("Inserted {desc}"));
					preview_fn();
				}
			}
		});
	}

	{
		let state_p = state.clone();
		let parts_p = parts.clone();
		privacy_btn.on_click(move |_| {
			let payload = serde_json::json!({ "action": "get_status_privacy", "data": {} });
			let _ = state_p.borrow().ws_tx.send(payload.to_string());
			crate::platform::announce::announce(
				parts_p.conversations_parts.messages_announcer,
				"Loading status privacy settings...",
			);
		});
	}

	let diag = dialog;
	cancel_btn.on_click(move |_| diag.end_modal(ID_CANCEL));

	let state_send = state.clone();
	let parts_send = parts.clone();
	let diag_ok = dialog;
	let field_ok = text_field;
	let color_idx_ok = selected_color_idx.clone();
	let font_idx_ok = selected_font_idx.clone();
	let do_post = Rc::new(move || {
		let text = field_ok.get_value().trim().to_string();
		if text.is_empty() {
			crate::platform::announce::announce(announcer, "Status update cannot be empty.");
			return;
		}
		let (_, bg) = STATUS_PALETTE[color_idx_ok.get()];
		let font_id = STATUS_FONTS[font_idx_ok.get()].1;
		let payload = serde_json::json!({
			"action": "post_status",
			"data": {
				"text": text,
				"background_argb": bg,
								"font": font_id,
			}
		});
		let _ = state_send.borrow().ws_tx.send(payload.to_string());
		crate::platform::announce::announce(
			parts_send.conversations_parts.messages_announcer,
			"Posting status update...",
		);
		diag_ok.end_modal(ID_OK);
	});

	let do_post_btn = do_post.clone();
	post_btn.on_click(move |_| do_post_btn());

	let do_post_key = do_post.clone();
	dialog.bind_internal(EventType::CHAR_HOOK, move |event| {
		let data = WindowEventData::new(event);
		let WindowEventData::Keyboard(ref key) = data else { return };
		let code = key.get_key_code();
		if keys::is_escape(code) {
			dialog.end_modal(ID_CANCEL);
			data.skip(false);
			return;
		}
		if keys::is_enter(code) && key.control_down() {
			do_post_key();
			data.skip(false);
			return;
		}
		data.skip(true);
	});

	crate::platform::announce::announce(
		announcer,
		"Post text status. Type your status update, press Next Colour or Next Font to style, or Ctrl+Enter to post.",
	);

	text_field.set_focus();
	dialog.show_modal();
	dialog.destroy();
}

pub fn show_media_status_composer(
	parent: &impl WxWidget,
	state: &Rc<std::cell::RefCell<AppState>>,
	parts: &Rc<WindowParts>,
) {
	let file_dialog = FileDialog::builder(parent)
		.with_message("Choose a photo or video to post as status")
		.with_wildcard(&whatsnap::AttachKind::Media.file_filter())
		.with_style(FileDialogStyle::Open | FileDialogStyle::FileMustExist)
		.build();

	if file_dialog.show_modal() != wxdragon::id::ID_OK {
		return;
	}
	let Some(path) = file_dialog.get_path() else { return };

	let media_type = match whatsnap::AttachKind::Media.send_type(&path) {
		"video" => "video",
		_ => "image",
	};
	build_media_status_dialog(parent, state, parts, path, media_type);
}

fn build_media_status_dialog(
	parent: &impl WxWidget,
	state: &Rc<std::cell::RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	path: String,
	media_type: &'static str,
) {
	let filename = std::path::Path::new(&path).file_name().unwrap_or_default().to_string_lossy().to_string();
	let file_size_str =
		std::fs::metadata(&path).ok().and_then(|m| whatsnap::format_file_size(m.len() as i64)).unwrap_or_default();

	let title = if media_type == "video" { "Post Video Status" } else { "Post Photo Status" };

	let dialog = Dialog::builder(parent, title)
		.with_size(520, 420)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let announcer = StaticText::builder(&panel).with_label("").build();
	live_region::set_live_region(&announcer);
	sizer.add(&announcer, 0, SizerFlag::All, 0);

	let file_info = if file_size_str.is_empty() { filename.clone() } else { format!("{filename} ({file_size_str})") };
	let file_label = StaticText::builder(&panel).with_label("Selected &file:").build();
	theme::style_muted_label(&file_label);
	sizer.add(&file_label, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Right, 12);

	let file_field = TextCtrl::builder(&panel)
		.with_style(dialog_look::field_style(TextCtrlStyle::ReadOnly))
		.build();
	file_field.set_font(&theme::Fonts::body());
	file_field.set_value(&file_info);
	sizer.add(&file_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 4);

	let caption_label = StaticText::builder(&panel).with_label("&Caption (optional):").build();
	theme::style_muted_label(&caption_label);
	sizer.add(&caption_label, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let caption_field =
		TextCtrl::builder(&panel).with_style(dialog_look::field_style(TextCtrlStyle::MultiLine)).build();
	caption_field.set_font(&theme::Fonts::body());
	{
		let field = caption_field;
		caption_field.on_text_changed(move |_| {
			clamp_field(&field, STATUS_CAPTION_MAX_CHARS, usize::MAX);
		});
	}
	caption_field.set_min_size(Size::new(-1, 80));
	sizer.add(&caption_field, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let privacy_row = BoxSizer::builder(Orientation::Horizontal).build();
	let privacy_hint = StaticText::builder(&panel).with_label("Visible to your configured status audience.").build();
	theme::style_muted_label(&privacy_hint);
	privacy_row.add(&privacy_hint, 1, SizerFlag::AlignCenterVertical, 0);

	let privacy_btn = Button::builder(&panel).with_label("Status &Privacy...").build();
	dialog_look::pill_button(&privacy_btn, false, theme::Palette::CONTENT_BG());
	privacy_row.add(&privacy_btn, 0, SizerFlag::AlignCenterVertical | SizerFlag::Left, 8);
	sizer.add_sizer(&privacy_row, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let btn_row = BoxSizer::builder(Orientation::Horizontal).build();
	btn_row.add_stretch_spacer(1);
	let cancel_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Cancel").build();
	dialog_look::pill_button(&cancel_btn, false, theme::Palette::CONTENT_BG());
	btn_row.add(&cancel_btn, 0, SizerFlag::Right, 8);
	let post_btn = Button::builder(&panel).with_id(ID_OK).with_label("Post &Status").build();
	dialog_look::pill_button(&post_btn, true, theme::Palette::CONTENT_BG());
	btn_row.add(&post_btn, 0, SizerFlag::Right, 0);
	sizer.add_sizer(&btn_row, 0, SizerFlag::Expand | SizerFlag::All, 12);

	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let diag_cancel = dialog;
	cancel_btn.on_click(move |_| diag_cancel.end_modal(ID_CANCEL));

	{
		let state_p = state.clone();
		let parts_p = parts.clone();
		privacy_btn.on_click(move |_| {
			let payload = serde_json::json!({ "action": "get_status_privacy", "data": {} });
			let _ = state_p.borrow().ws_tx.send(payload.to_string());
			crate::platform::announce::announce(
				parts_p.conversations_parts.messages_announcer,
				"Loading status privacy settings...",
			);
		});
	}

	let diag_ok = dialog;
	let field_ok = caption_field;
	let path_ok = path.clone();
	let filename_ok = filename.clone();
	let media_type_ok = media_type;
	let state_ok = state.clone();
	let parts_ok = parts.clone();
	let do_post = Rc::new(move || {
		let caption = field_ok.get_value().trim().to_string();
		let mut payload = serde_json::json!({
			"action": "post_media_status",
			"data": {
				"type": media_type_ok,
				"path": path_ok,
				"filename": filename_ok,
				"caption": caption
			}
		});
		let ws_tx = state_ok.borrow().ws_tx.clone();
		if media_type_ok == "video" {
			let path_thumb = path_ok.clone();
			std::thread::spawn(move || {
				if let Some(png) = crate::media::video_thumbnail::first_frame_png(&path_thumb) {
					payload["data"]["thumbnail"] = serde_json::json!(crate::util::base64(&png));
				}
				let _ = ws_tx.send(payload.to_string());
			});
		} else {
			let _ = ws_tx.send(payload.to_string());
		}
		crate::platform::announce::announce(
			parts_ok.conversations_parts.messages_announcer,
			"Posting status update...",
		);
		diag_ok.end_modal(ID_OK);
	});

	let do_post_btn = do_post.clone();
	post_btn.on_click(move |_| do_post_btn());

	let do_post_key = do_post.clone();
	dialog.bind_internal(EventType::CHAR_HOOK, move |event| {
		let data = WindowEventData::new(event);
		let WindowEventData::Keyboard(ref key) = data else { return };
		let code = key.get_key_code();
		if keys::is_escape(code) {
			dialog.end_modal(ID_CANCEL);
			data.skip(false);
			return;
		}
		if keys::is_enter(code) && key.control_down() {
			do_post_key();
			data.skip(false);
			return;
		}
		data.skip(true);
	});

	crate::platform::announce::announce(
		announcer,
		&format!(
			"Post {} status. Selected file: {file_info}. Type optional caption, or press Ctrl+Enter to post.",
			if media_type == "video" { "video" } else { "photo" }
		),
	);

	caption_field.set_focus();
	dialog.show_modal();
	dialog.destroy();
}
