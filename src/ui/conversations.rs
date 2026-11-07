use wxdragon::prelude::*;

use crate::ui::theme;

#[allow(dead_code)]
#[derive(Clone, Copy)]
pub struct ConversationsPanelParts {
	pub panel: Panel,

	pub search_label: StaticText,
	pub search_field: TextCtrl,
	pub chat_filter_label: StaticText,
	pub chat_filter_combo: ComboBox,
	pub new_conv_btn: Button,
	pub locked_chats_btn: Button,
	pub post_media_status_btn: Button,

	pub conversations_label: StaticText,
	pub conversations_list: ListBox,
	pub calls_btn_panel: Panel,
	pub calls_call_btn: Button,
	pub calls_video_btn: Button,

	pub incoming_call_banner: Panel,
	pub incoming_call_banner_label: StaticText,
	pub incoming_call_restore_btn: Button,
	pub incoming_call_answer_btn: Button,
	pub incoming_call_decline_btn: Button,

	pub conversation_panel: Panel,
	pub conv_data_btn: Button,
	pub add_attachment_btn: Button,
	pub emoji_btn: Button,
	pub search_in_conv_btn: Button,
	pub place_call_btn: Button,
	pub place_video_call_btn: Button,
	pub join_call_btn: Button,
	pub top_btns_panel: Panel,

	pub search_panel: Panel,
	pub search_close_btn: Button,
	pub search_conv_label: StaticText,
	pub search_conv_field: TextCtrl,

	pub messages_label: StaticText,
	pub messages_list: crate::ui::message_list::MessageList,
	pub messages_select_list: crate::ui::message_list::MessageList,

	pub selection_bar_panel: Panel,
	pub selection_count_label: StaticText,
	pub selection_copy_btn: Button,
	pub selection_star_btn: Button,
	pub selection_keep_btn: Button,
	pub selection_delete_btn: Button,
	pub selection_forward_btn: Button,
	pub selection_save_btn: Button,
	pub selection_cancel_btn: Button,

	pub play_voice_btn: Button,
	pub messages_announcer: StaticText,

	pub read_more_btn: Button,

	pub message_label: StaticText,
	pub message_field: TextCtrl,
	/// @mention suggestions, shown above the composer while completing one.
	/// Never focused: the composer keeps focus and announces the highlight.
	pub mention_list: ListBox,
	pub cancel_edit_btn: Button,
	pub remove_quote_btn: Button,
	pub send_message_btn: Button,
	pub record_voice_btn: Button,

	pub discard_voice_btn: Button,
	pub pause_resume_btn: Button,
	pub preview_voice_btn: Button,
	pub view_once_voice_check: CheckBox,
	pub send_voice_btn: Button,

	pub attachment_panel: Panel,
	pub caption_label: StaticText,
	pub caption_field: TextCtrl,
	pub send_attachment_btn: Button,
	pub add_more_btn: Button,
	pub attachment_info_label: StaticText,
	pub upload_gauge: Gauge,

	pub audio_speed_btn: Button,
	pub audio_progress_label: Button,
	pub audio_slider: Slider,

	pub action_panel: Panel,
	pub action_retry_btn: Button,
	pub action_cancel_upload_btn: Button,
	pub action_open_btn: Button,
	pub action_save_as_btn: Button,
	pub action_download_btn: Button,
}

/// Gives a list a fixed minimum size, so laying out its panel never asks the
/// list for its best size. wxListBox works that out by measuring the text of
/// every row, and any change to the list (a new message, a tick, the unread
/// divider going) throws the answer away. A whole chat is tens of thousands
/// of rows, so every layout after a change - showing or hiding Remove quote
/// when replying, the composer emptying after a send - froze the app for
/// seconds at a time. Both sides must be set: wx falls back to the best size
/// for any side left at -1. The sizer stretches the list as before.
fn skip_best_size(list: &impl WxWidget) {
	list.set_min_size(Size::new(100, 100));
}

const BORDER_NONE: i64 = 0x0020_0000;

#[derive(Clone, Copy)]
pub struct RecordingClock {
	pub start: std::time::Instant,
	pub paused: std::time::Duration,
	pub paused_at: Option<std::time::Instant>,
}

impl RecordingClock {
	fn elapsed(&self) -> std::time::Duration {
		let end = self.paused_at.unwrap_or_else(std::time::Instant::now);
		end.saturating_duration_since(self.start).saturating_sub(self.paused)
	}
}

thread_local! {
	static RECORDING: std::cell::Cell<Option<RecordingClock>> = const { std::cell::Cell::new(None) };
	static RECORDING_TICK: std::cell::RefCell<Option<Timer<Panel>>> = const { std::cell::RefCell::new(None) };
}

pub fn set_recording_clock(clock: Option<RecordingClock>) {
	RECORDING.with(|r| r.set(clock));
	RECORDING_TICK.with(|t| {
		if let Some(timer) = t.borrow().as_ref() {
			if clock.is_some_and(|c| c.paused_at.is_none()) {
				timer.start(500, false);
			} else {
				timer.stop();
			}
		}
	});
}

struct ComposerParts {
	field: TextCtrl,
	pause: Button,
	preview: Button,
	view_once: CheckBox,
	send_voice: Button,
}

/// Draws the composer's grey bar across the chat, and the round white box
/// the message field sits in; while recording, WhatsApp's recording bar
/// (red dot, clock) between the pause and send buttons. Only painted: the
/// controls are the same ones, and the screen reader hears the recording
/// start, pause and stop as before.
fn paint_composer(conversation: &Panel, parts: ComposerParts, bar: Colour) {
	if !theme::use_custom_colours() {
		return;
	}
	let panel_c = *conversation;
	let ComposerParts { field: field_c, pause, preview, view_once, send_voice } = parts;
	let timer = Timer::new(&panel_c);
	timer.on_tick(move |_| panel_c.refresh(false, None));
	RECORDING_TICK.with(|t| *t.borrow_mut() = Some(timer));
	panel_c.set_background_style(BackgroundStyle::Paint);
	panel_c.on_paint(move |_| {
		let dc = AutoBufferedPaintDC::new(&panel_c);
		let size = panel_c.get_client_size();
		let background = panel_c.get_background_color();
		dc.set_brush(background, BrushStyle::Solid);
		dc.set_pen(background, 1, PenStyle::Solid);
		dc.draw_rectangle(0, 0, size.width, size.height);
		if send_voice.is_shown() {
			let (p, ps) = (pause.get_position(), pause.get_size());
			let right = if view_once.is_shown() { view_once.get_position().x } else { send_voice.get_position().x };
			let top = p.y.min(send_voice.get_position().y) - 10;
			let height = ps.height.max(send_voice.get_size().height) + 20;
			dc.set_brush(bar, BrushStyle::Solid);
			dc.set_pen(bar, 1, PenStyle::Solid);
			dc.draw_rectangle(0, top, size.width, height);
			let Some(clock) = RECORDING.with(|r| r.get()) else { return };
			let (sp, sps) = if preview.is_shown() { (preview.get_position(), preview.get_size()) } else { (p, ps) };
			let (left, mid) = (sp.x + sps.width + 12, p.y + ps.height / 2);
			let paused = clock.paused_at.is_some();
			let previewing = preview.is_shown() && preview.get_label().starts_with("Pause");
			let secs = clock.elapsed().as_secs();
			let blink = clock.elapsed().as_millis() / 500 % 2 == 0;
			let dot = if previewing {
				theme::Palette::ACCENT()
			} else if paused {
				theme::Palette::TEXT_MUTED()
			} else {
				Colour::rgb(0xEA, 0x00, 0x38)
			};
			if paused || previewing || blink {
				dc.set_brush(dot, BrushStyle::Solid);
				dc.set_pen(dot, 1, PenStyle::Solid);
				dc.draw_circle(left + 6, mid, 6);
			}
			dc.set_font(&theme::Fonts::body());
			dc.set_text_foreground(theme::Palette::TEXT_PRIMARY());
			dc.set_background_mode(BackgroundMode::Transparent);
			let status = if previewing {
				"  Playing preview"
			} else if paused {
				"  Paused"
			} else {
				""
			};
			let text = format!("{}:{:02}{}", secs / 60, secs % 60, status);
			let (tw, th) = dc.get_text_extent(&text);
			dc.draw_text(&text, left + 20, mid - th / 2);
			let grey = theme::Palette::WAVE();
			dc.set_brush(grey, BrushStyle::Solid);
			dc.set_pen(grey, 1, PenStyle::Solid);
			let mut x = left + 20 + tw + 16;
			while x + 3 < right - 12 {
				dc.draw_rectangle(x, mid - 1, 3, 3);
				x += 7;
			}
			return;
		}
		if !field_c.is_shown() {
			return;
		}
		let pos = field_c.get_position();
		let fs = field_c.get_size();
		let top = pos.y - 10;
		dc.set_brush(bar, BrushStyle::Solid);
		dc.set_pen(bar, 1, PenStyle::Solid);
		dc.draw_rectangle(0, top, size.width, fs.height + 20);
		let white = theme::Palette::CONTENT_BG();
		dc.set_brush(white, BrushStyle::Solid);
		dc.set_pen(white, 1, PenStyle::Solid);
		dc.draw_rounded_rectangle(pos.x - 8, pos.y - 5, fs.width + 16, fs.height + 10, 8.0);
	});
	panel_c.on_size(move |event| {
		panel_c.refresh(true, None);
		event.skip(true);
	});
}

fn paint_calls_page(page: &Panel) {
	let page_c = *page;
	page.set_background_style(BackgroundStyle::Paint);
	page.on_paint(move |_| {
		let dc = AutoBufferedPaintDC::new(&page_c);
		let size = page_c.get_client_size();
		let background = page_c.get_background_color();
		dc.set_brush(background, BrushStyle::Solid);
		dc.set_pen(background, 1, PenStyle::Solid);
		dc.draw_rectangle(0, 0, size.width, size.height);
		let custom = theme::use_custom_colours();
		let ink = |c: Colour| if custom { c } else { page_c.get_foreground_color() };
		let line = page_c.get_char_height();
		let buttons_top = size.height / 2 - line * 2 + line * 7 / 2;
		let mut y = buttons_top - line * 7;
		let r = line * 3 / 2;
		let cx = size.width / 2;
		let circle = ink(theme::Palette::ACCENT());
		dc.set_brush(circle, BrushStyle::Solid);
		dc.set_pen(circle, 1, PenStyle::Solid);
		dc.draw_circle(cx, y + r, r);
		dc.set_background_mode(BackgroundMode::Transparent);
		if let Some(font) = crate::ui::message_art::icon_face().and_then(|face| {
			Font::builder().with_point_size(theme::Fonts::heading().get_point_size() + 2).with_face_name(face).build()
		}) {
			dc.set_font(&font);
			dc.set_text_foreground(if custom { theme::Palette::ACCENT_TEXT() } else { background });
			let g = "\u{E717}";
			let (w, h) = dc.get_text_extent(g);
			dc.draw_text(g, cx - w / 2, y + r - h / 2);
		}
		y += r * 2 + line;
		for (text, font, colour) in [
			("Calls", theme::Fonts::heading(), theme::Palette::TEXT_PRIMARY()),
			("Pick someone on the left, then call them.", theme::Fonts::body(), theme::Palette::TEXT_MUTED()),
		] {
			dc.set_font(&font);
			dc.set_text_foreground(ink(colour));
			let (w, h) = dc.get_text_extent(text);
			dc.draw_text(text, cx - w / 2, y);
			y += h + 6;
		}
	});
	page.on_size(move |event| {
		page_c.refresh(true, None);
		event.skip(true);
	});
}

type ResizedCallback = Box<dyn Fn(i32)>;

thread_local! {
	static LIST_COLUMN: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
	static ON_RESIZED: std::cell::RefCell<Option<ResizedCallback>> = const { std::cell::RefCell::new(None) };
}

pub fn on_list_column_resized(f: impl Fn(i32) + 'static) {
	ON_RESIZED.with(|r| *r.borrow_mut() = Some(Box::new(f)));
}

fn list_column_width(width: i32) -> i32 {
	let chosen = LIST_COLUMN.with(|c| c.get());
	if chosen > 0 {
		return chosen.clamp(min_list_column(width), max_list_column(width));
	}
	(width * 7 / 20).clamp(320, 520).min(max_list_column(width))
}

/// At most 40% of the window, as WhatsApp for Windows allows, so the list
/// can never crowd out the chat.
fn max_list_column(width: i32) -> i32 {
	(width * 2 / 5).max(min_list_column(width))
}

/// At least a quarter of the window, and never so narrow that a chat's name
/// and preview don't fit.
fn min_list_column(width: i32) -> i32 {
	(width / 4).max(300)
}

/// Keeps the chat list column a third of the window as it's resized (or
/// where the user dragged its edge, like WhatsApp for Windows' sidebar),
/// and draws WhatsApp's "pick a chat" page where the open chat goes while
/// none is open. It's all mouse and paint: a screen reader isn't told.
fn fit_columns(panel: &Panel, search: &TextCtrl, list: &ListBox, conversation: &Panel, calls_page: &Panel) {
	let (panel_c, list_c, conversation_c, calls_c) = (*panel, *list, *conversation, *calls_page);
	let search_c = *search;
	search.on_set_focus(move |event| {
		panel_c.refresh(false, None);
		event.skip(true);
	});
	search.on_kill_focus(move |event| {
		panel_c.refresh(false, None);
		event.skip(true);
	});
	LIST_COLUMN.with(|c| c.set(crate::settings::Settings::load().chat_list_width));
	let fit = move || {
		let width = panel_c.get_client_size().width;
		list_c.set_min_size(Size::new(list_column_width(width) - 20, 100));
	};
	panel.on_size(move |event| {
		fit();
		panel_c.refresh(true, None);
		event.skip(true);
	});
	let on_edge = move |x: i32| {
		let right = list_c.get_position().x + list_c.get_size().width;
		(right - 2..=right + 12).contains(&x)
	};
	let dragging = std::rc::Rc::new(std::cell::Cell::new(false));
	let resize_cursor = Cursor::from_stock(StockCursor::SizeWE);
	let arrow = Cursor::from_stock(StockCursor::Arrow);
	let drag = dragging.clone();
	panel.on_mouse_left_down(move |event| {
		if let WindowEventData::MouseButton(ref mouse) = event
			&& let Some(pos) = mouse.get_position()
			&& on_edge(pos.x)
		{
			drag.set(true);
			panel_c.capture_mouse();
		}
		event.skip(true);
	});
	let drag = dragging.clone();
	panel.on_mouse_motion(move |event| {
		if let WindowEventData::MouseMotion(ref mouse) = event
			&& let Some(pos) = mouse.get_position()
		{
			if drag.get() && !panel_c.has_capture() {
				drag.set(false);
			}
			if drag.get() {
				let width = panel_c.get_client_size().width;
				let column = pos.x.clamp(min_list_column(width), max_list_column(width));
				LIST_COLUMN.with(|c| c.set(column));
				fit();
				panel_c.layout();
				panel_c.refresh(true, None);
			}
			let cursor = if drag.get() || on_edge(pos.x) { &resize_cursor } else { &arrow };
			panel_c.set_cursor(cursor.as_ref());
		}
		event.skip(true);
	});
	let drag = dragging;
	panel.on_mouse_left_up(move |event| {
		if drag.replace(false) {
			if panel_c.has_capture() {
				panel_c.release_mouse();
			}
			let column = LIST_COLUMN.with(|c| c.get());
			ON_RESIZED.with(|r| {
				if let Some(f) = r.borrow().as_ref() {
					f(column);
				}
			});
		}
		event.skip(true);
	});
	panel.set_background_style(BackgroundStyle::Paint);
	panel.on_paint(move |_| {
		let dc = AutoBufferedPaintDC::new(&panel_c);
		let size = panel_c.get_client_size();
		let background = panel_c.get_background_color();
		dc.set_brush(background, BrushStyle::Solid);
		dc.set_pen(background, 1, PenStyle::Solid);
		dc.draw_rectangle(0, 0, size.width, size.height);
		if theme::use_custom_colours() {
			crate::ui::dialog_look::paint_field(&dc, &search_c, crate::ui::dialog_look::Field::Search);
		}
		let left = list_c.get_position().x + list_c.get_size().width + 10;
		let custom = theme::use_custom_colours();
		let line = if custom { theme::Palette::DIVIDER_SOFT() } else { panel_c.get_foreground_color() };
		dc.set_pen(line, 1, PenStyle::Solid);
		dc.draw_line(left, 0, left, size.height);
		if conversation_c.is_shown() || calls_c.is_shown() {
			return;
		}
		let area = Size::new(size.width - left - 1, size.height);
		if area.width < 80 {
			return;
		}
		if custom {
			dc.set_brush(theme::Palette::PANEL_BG(), BrushStyle::Solid);
			dc.set_pen(theme::Palette::PANEL_BG(), 1, PenStyle::Solid);
			dc.draw_rectangle(left + 1, 0, area.width, area.height);
		}
		let icon = theme::build_app_icon(96);
		let cx = left + 1 + area.width / 2;
		let mut y = area.height / 2 - 110;
		dc.draw_bitmap(&icon, cx - 48, y, true);
		y += 120;
		dc.set_background_mode(BackgroundMode::Transparent);
		for (text, font, colour) in [
			(
				"whatSnap for Windows",
				theme::Fonts::heading(),
				if custom { theme::Palette::TEXT_PRIMARY() } else { panel_c.get_foreground_color() },
			),
			(
				"Pick a chat on the left to start messaging.",
				theme::Fonts::body(),
				if custom { theme::Palette::TEXT_MUTED() } else { panel_c.get_foreground_color() },
			),
			(
				"Your personal messages are end-to-end encrypted.",
				theme::Fonts::label(),
				if custom { theme::Palette::TEXT_MUTED() } else { panel_c.get_foreground_color() },
			),
		] {
			dc.set_font(&font);
			dc.set_text_foreground(colour);
			let (w, h) = dc.get_text_extent(text);
			dc.draw_text(text, cx - w / 2, y);
			y += h + 10;
		}
	});
}

pub fn build_conversations_panel(parent: &impl WxWidget) -> ConversationsPanelParts {
	let panel = Panel::builder(parent).build();
	theme::style_content(&panel);
	let outer_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let left_sizer = BoxSizer::builder(Orientation::Vertical).build();

	let search_label = StaticText::builder(&panel).with_label("Search chats").build();
	theme::style_muted_label(&search_label);
	left_sizer.add(&search_label, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let search_field =
		TextCtrl::builder(&panel).with_style(crate::ui::dialog_look::field_style(TextCtrlStyle::Default)).build();
	crate::ui::dialog_look::add_field(&left_sizer, &search_field, crate::ui::dialog_look::Field::Search, 0);

	let chat_filter_label = StaticText::builder(&panel).with_label("Filters").build();
	theme::style_muted_label(&chat_filter_label);
	left_sizer.add(&chat_filter_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let chat_filter_combo = ComboBox::builder(&panel)
		.with_string_choices(&crate::ui::message_display::BUILT_IN_CHAT_FILTERS)
		.with_style(ComboBoxStyle::ReadOnly)
		.build();
	chat_filter_combo.set_font(&theme::Fonts::body());
	chat_filter_combo.set_selection(0);
	left_sizer.add(&chat_filter_combo, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let new_conv_btn = Button::builder(&panel).with_label("New Chat").build();
	theme::style_primary_button(&new_conv_btn);
	left_sizer.add(&new_conv_btn, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let locked_chats_btn = Button::builder(&panel).with_label("Locked Chats").build();
	left_sizer.add(
		&locked_chats_btn,
		0,
		SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top | SizerFlag::Bottom,
		10,
	);
	locked_chats_btn.show(false);

	let post_media_status_btn = Button::builder(&panel).with_label("Post Photo/Video Status...").build();
	left_sizer.add(
		&post_media_status_btn,
		0,
		SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom,
		10,
	);
	post_media_status_btn.show(false);

	let list_divider = StaticLine::builder(&panel).build();
	theme::style_divider(&list_divider);
	left_sizer.add(&list_divider, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 10);

	let conversations_label = StaticText::builder(&panel).with_label("Chats").build();
	theme::style_heading(&conversations_label);
	left_sizer.add(&conversations_label, 0, SizerFlag::Left | SizerFlag::Top, 10);

	// Owner-drawn so its rows can show pictures and icons; a screen reader
	// still reads each row's text, as before (see ui::chat_rows).
	let conversations_list =
		ListBox::builder(&panel).with_style(ListBoxStyle::from_bits_retain(crate::ui::chat_rows::OWNER_DRAW)).build();
	conversations_list.set_font(&theme::Fonts::body());
	skip_best_size(&conversations_list);
	left_sizer.add(&conversations_list, 1, SizerFlag::Expand | SizerFlag::All, 10);

	let calls_btn_panel = Panel::builder(&panel).build();
	if theme::use_custom_colours() {
		calls_btn_panel.set_background_color(theme::Palette::PANEL_BG());
	}
	let calls_page = BoxSizer::builder(Orientation::Vertical).build();
	calls_page.add_stretch_spacer(1);
	calls_page.add_spacer(calls_btn_panel.get_char_height() * 7);
	let calls_btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let calls_call_btn = Button::builder(&calls_btn_panel).with_label("Call").build();
	calls_call_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_tile(&calls_call_btn, theme::Palette::PANEL_BG());
	calls_btn_sizer.add(&calls_call_btn, 0, SizerFlag::Right, 16);

	let calls_video_btn = Button::builder(&calls_btn_panel).with_label("Video call").build();
	calls_video_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_tile(&calls_video_btn, theme::Palette::PANEL_BG());
	calls_btn_sizer.add(&calls_video_btn, 0, SizerFlag::empty(), 0);
	calls_page.add_sizer(&calls_btn_sizer, 0, SizerFlag::AlignCenterHorizontal, 0);
	calls_page.add_stretch_spacer(1);

	calls_btn_panel.set_sizer(calls_page, true);
	paint_calls_page(&calls_btn_panel);
	calls_btn_panel.show(false);

	let conversation_panel = Panel::builder(&panel).build();
	theme::style_chat_area(&conversation_panel);
	let conv_sizer = BoxSizer::builder(Orientation::Vertical).build();

	// WhatsApp's header bar: the chat's picture, name and status, then its
	// buttons as icons. Each keeps its label for the screen reader.
	let top_btns_panel = Panel::builder(&conversation_panel).build();
	let header_bg = theme::Palette::PANEL_BG();
	if theme::use_custom_colours() {
		top_btns_panel.set_background_color(header_bg);
	}
	let top_btns_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let conv_data_btn = Button::builder(&top_btns_panel).with_label("Conversation data").build();
	conv_data_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_header(&conv_data_btn, header_bg);
	top_btns_sizer.add(&conv_data_btn, 1, SizerFlag::Right | SizerFlag::Expand, 5);

	let search_in_conv_btn = Button::builder(&top_btns_panel).with_label("Search in conversation").build();
	search_in_conv_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_icon_button(&search_in_conv_btn, header_bg);
	top_btns_sizer.add(&search_in_conv_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 2);

	let place_call_btn = Button::builder(&top_btns_panel).with_label("Call").build();
	place_call_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_icon_button(&place_call_btn, header_bg);
	top_btns_sizer.add(&place_call_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 2);

	let place_video_call_btn = Button::builder(&top_btns_panel).with_label("Video call").build();
	place_video_call_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_icon_button(&place_video_call_btn, header_bg);
	top_btns_sizer.add(&place_video_call_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 2);

	let join_call_btn = Button::builder(&top_btns_panel).with_label("Join call").build();
	join_call_btn.set_font(&theme::Fonts::body());
	theme::style_primary_button(&join_call_btn);
	top_btns_sizer.add(&join_call_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 5);
	join_call_btn.show(false);

	top_btns_panel.set_sizer(top_btns_sizer, true);
	conv_sizer.add(&top_btns_panel, 0, SizerFlag::Expand, 0);

	let incoming_call_banner = Panel::builder(&panel).build();
	if theme::use_custom_colours() {
		incoming_call_banner.set_background_color(if theme::is_dark() {
			Colour::rgb(0x1B, 0x2A, 0x32)
		} else {
			Colour::rgb(0xE7, 0xF5, 0xF0)
		});
	}
	let icb_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let incoming_call_banner_label = StaticText::builder(&incoming_call_banner).with_label("Incoming call...").build();
	incoming_call_banner_label.set_font(&theme::Fonts::body());
	if theme::use_custom_colours() {
		incoming_call_banner_label.set_foreground_color(if theme::is_dark() {
			Colour::rgb(0xE9, 0xED, 0xEE)
		} else {
			Colour::rgb(0x11, 0x1B, 0x21)
		});
	}
	icb_sizer.add(&incoming_call_banner_label, 1, SizerFlag::AlignCenterVertical | SizerFlag::Left, 12);

	let incoming_call_restore_btn = Button::builder(&incoming_call_banner).with_label("Open call window").build();
	incoming_call_restore_btn.set_font(&theme::Fonts::body());
	icb_sizer.add(&incoming_call_restore_btn, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 8);

	let incoming_call_answer_btn = Button::builder(&incoming_call_banner).with_label("Answer").build();
	incoming_call_answer_btn.set_font(&theme::Fonts::body());
	theme::style_primary_button(&incoming_call_answer_btn);
	icb_sizer.add(&incoming_call_answer_btn, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 8);

	let incoming_call_decline_btn = Button::builder(&incoming_call_banner).with_label("Decline").build();
	incoming_call_decline_btn.set_font(&theme::Fonts::body());
	icb_sizer.add(&incoming_call_decline_btn, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 12);

	incoming_call_banner.set_sizer(icb_sizer, true);
	incoming_call_banner.show(false);

	let search_panel = Panel::builder(&conversation_panel).build();
	theme::style_content(&search_panel);
	let search_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let search_close_btn = Button::builder(&search_panel).with_label("Close search").build();
	crate::ui::owner_draw::install_icon_button(&search_close_btn, theme::Palette::CONTENT_BG());
	search_sizer.add(&search_close_btn, 0, SizerFlag::Right, 5);
	let search_conv_label = StaticText::builder(&search_panel).with_label("Search conversation").build();
	theme::style_muted_label(&search_conv_label);
	search_sizer.add(&search_conv_label, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 5);
	let search_conv_field = TextCtrl::builder(&search_panel).with_style(TextCtrlStyle::ProcessEnter).build();
	search_conv_field.set_font(&theme::Fonts::body());
	search_sizer.add(&search_conv_field, 1, SizerFlag::Expand | SizerFlag::Right, 5);
	search_panel.set_sizer(search_sizer, true);
	search_panel.show(false);
	conv_sizer.add(&search_panel, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom, 10);

	let messages_label = StaticText::builder(&conversation_panel).with_label("Messages").build();
	theme::style_muted_label(&messages_label);
	messages_label.set_min_size(Size::new(1, 1));
	conv_sizer.add(&messages_label, 0, SizerFlag::Left, 0);

	let messages_list = crate::ui::message_list::MessageList::new(&conversation_panel, "Messages");
	messages_list.set_font(&theme::Fonts::body());
	skip_best_size(&messages_list);
	theme::style_chat_area(&messages_list);
	conv_sizer.add(&messages_list, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 10);

	let messages_select_list = crate::ui::message_list::MessageList::new(&conversation_panel, "Select messages");
	messages_select_list.set_checkable(true);
	messages_select_list.set_font(&theme::Fonts::body());
	skip_best_size(&messages_select_list);
	theme::style_chat_area(&messages_select_list);
	conv_sizer.add(&messages_select_list, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 10);
	messages_select_list.show(false);

	let selection_bar_panel = Panel::builder(&conversation_panel).build();
	theme::style_content(&selection_bar_panel);
	let selection_bar_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let selection_count_label = StaticText::builder(&selection_bar_panel).with_label("0 selected").build();
	theme::style_muted_label(&selection_count_label);
	selection_bar_sizer.add(&selection_count_label, 0, SizerFlag::Left | SizerFlag::AlignCenterVertical, 5);
	selection_bar_sizer.add_stretch_spacer(1);
	// The buttons of WhatsApp Web's select bar, in its order; which show and
	// which are available is decided by ui_commands::selection::update_bar.
	let bar_button = |label: &str| {
		let button = Button::builder(&selection_bar_panel).with_label(label).build();
		button.set_font(&theme::Fonts::body());
		selection_bar_sizer.add(&button, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 5);
		button
	};
	let selection_copy_btn = bar_button("Copy");
	let selection_star_btn = bar_button("Star");
	let selection_keep_btn = bar_button("Keep");
	let selection_delete_btn = bar_button("Delete");
	let selection_forward_btn = bar_button("Forward");
	let selection_save_btn = bar_button("Save as...");
	let selection_cancel_btn = bar_button("Cancel");
	selection_keep_btn.show(false);
	selection_bar_panel.set_sizer(selection_bar_sizer, true);
	conv_sizer.add(
		&selection_bar_panel,
		0,
		SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom,
		10,
	);
	selection_bar_panel.show(false);

	let action_panel = Panel::builder(&conversation_panel).build();
	theme::style_chat_area(&action_panel);
	let action_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let action_spacer = StaticText::builder(&action_panel).with_size(Size::new(1, 90)).with_label("").build();
	action_sizer.add(&action_spacer, 0, SizerFlag::Left, 0);

	let play_voice_btn = Button::builder(&action_panel).with_label("Play voice message").build();
	action_sizer.add(&play_voice_btn, 0, SizerFlag::Left | SizerFlag::Bottom, 5);
	play_voice_btn.show(false);

	let messages_announcer = StaticText::builder(&action_panel).with_label("").build();
	live_region::set_live_region(&messages_announcer);
	action_sizer.add(&messages_announcer, 0, SizerFlag::Left, 0);
	messages_announcer.show(false);

	let read_more_btn = Button::builder(&action_panel).with_label("Read more").build();
	action_sizer.add(&read_more_btn, 0, SizerFlag::Left | SizerFlag::Bottom, 5);
	read_more_btn.show(false);

	let action_retry_btn = Button::builder(&action_panel).with_label("Retry send").build();
	action_sizer.add(&action_retry_btn, 0, SizerFlag::Left | SizerFlag::Bottom, 5);
	action_retry_btn.show(false);

	let action_cancel_upload_btn = Button::builder(&action_panel).with_label("Cancel upload, 0% complete").build();
	action_sizer.add(&action_cancel_upload_btn, 0, SizerFlag::Left | SizerFlag::Bottom, 5);
	action_cancel_upload_btn.show(false);

	let action_open_btn = Button::builder(&action_panel).with_label("Open").build();
	action_sizer.add(&action_open_btn, 0, SizerFlag::Left | SizerFlag::Bottom, 5);
	action_open_btn.show(false);

	let action_save_as_btn = Button::builder(&action_panel).with_label("Save as").build();
	action_sizer.add(&action_save_as_btn, 0, SizerFlag::Left | SizerFlag::Bottom, 5);
	action_save_as_btn.show(false);

	let action_download_btn = Button::builder(&action_panel).with_label("Download").build();
	action_sizer.add(&action_download_btn, 0, SizerFlag::Left | SizerFlag::Bottom, 5);
	action_download_btn.show(false);

	let audio_speed_btn = Button::builder(&action_panel).with_label("1.0x").build();
	action_sizer.add(&audio_speed_btn, 0, SizerFlag::Left | SizerFlag::Bottom, 5);
	audio_speed_btn.show(false);

	let audio_progress_label = Button::builder(&action_panel).with_label("0:00 / 0:00").build();
	action_sizer.add(&audio_progress_label, 0, SizerFlag::Left | SizerFlag::Bottom, 5);
	audio_progress_label.show(false);

	let audio_slider = Slider::builder(&action_panel).with_min_value(0).with_max_value(1000).build();
	action_sizer.add(&audio_slider, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 5);
	audio_slider.show(false);

	action_panel.set_sizer(action_sizer, true);
	action_panel.show(false);
	conv_sizer.add(&action_panel, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom, 0);

	// The emoji and stickers panel's button (WhatsApp Web's sits beside its
	// message box), then the label, then the box itself. Created in that
	// order: Windows names the box from the text just before it, so the
	// button must come ahead of the label, and Shift+Tab from the box still
	// reaches the button.
	// WhatsApp's composer: one grey bar with the emoji button, the message
	// box (drawn white and round, see `paint_composer`), then the attach,
	// send and mic buttons as icons. Creation order - which decides tab
	// order and names the box from the label just before it - is the same as
	// it always was, and the row is laid out in that same order, so what a
	// sighted user reads left to right is what a keyboard user tabs through:
	// emoji, label, box, attach, cancel edit, remove quote, send, record.
	let composer_bg = theme::Palette::PANEL_BG();
	let message_row = BoxSizer::builder(Orientation::Horizontal).build();
	let emoji_btn = Button::builder(&conversation_panel).with_label("Emoji and stickers").build();
	emoji_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_icon_button(&emoji_btn, composer_bg);
	message_row.add(&emoji_btn, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 4);
	let message_label = StaticText::builder(&conversation_panel).with_label("Type a message").build();
	theme::style_muted_label(&message_label);
	if theme::use_custom_colours() {
		message_label.set_background_color(composer_bg);
	}
	message_row.add(&message_label, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 12);
	let message_field = TextCtrl::builder(&conversation_panel)
		.with_style(
			TextCtrlStyle::MultiLine | TextCtrlStyle::ProcessEnter | TextCtrlStyle::from_bits_retain(BORDER_NONE),
		)
		.build();
	message_field.set_font(&theme::Fonts::body());
	message_field.set_min_size(Size::new(-1, message_field.get_char_height() * 2 + 6));
	message_row.add(&message_field, 1, SizerFlag::AlignCenterVertical | SizerFlag::Right, 12);
	// Created after the field, so it's never ahead of it in tab order.
	let mention_list = ListBox::builder(&conversation_panel).build();
	mention_list.set_font(&theme::Fonts::body());
	mention_list.set_min_size(Size::new(-1, 120));
	conv_sizer.add(&mention_list, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);
	mention_list.show(false);
	let add_attachment_btn = Button::builder(&conversation_panel).with_label("Add attachment").build();
	add_attachment_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_icon_button(&add_attachment_btn, composer_bg);
	message_row.add(&add_attachment_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 2);
	let cancel_edit_btn = Button::builder(&conversation_panel).with_label("Cancel edit").build();
	cancel_edit_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_icon_button(&cancel_edit_btn, composer_bg);
	message_row.add(&cancel_edit_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 2);
	cancel_edit_btn.show(false);
	let remove_quote_btn = Button::builder(&conversation_panel).with_label("Remove quote").build();
	remove_quote_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_icon_button(&remove_quote_btn, composer_bg);
	message_row.add(&remove_quote_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 2);
	remove_quote_btn.show(false);
	let send_message_btn = Button::builder(&conversation_panel).with_label("Send message").build();
	send_message_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_icon_button(&send_message_btn, composer_bg);
	message_row.add(&send_message_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 2);
	let record_voice_btn = Button::builder(&conversation_panel).with_label("Record voice message").build();
	record_voice_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_icon_button(&record_voice_btn, composer_bg);
	message_row.add(&record_voice_btn, 0, SizerFlag::AlignCenterVertical, 0);
	conv_sizer.add_sizer(&message_row, 0, SizerFlag::Expand | SizerFlag::All, 10);
	let attachment_panel = Panel::builder(&conversation_panel).build();
	theme::style_content(&attachment_panel);
	let attach_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let caption_label = StaticText::builder(&attachment_panel).with_label("Caption").build();
	theme::style_muted_label(&caption_label);
	attach_sizer.add(&caption_label, 0, SizerFlag::Left | SizerFlag::Top, 5);
	let caption_field = TextCtrl::builder(&attachment_panel).with_style(TextCtrlStyle::ProcessEnter).build();
	caption_field.set_font(&theme::Fonts::body());
	attach_sizer.add(&caption_field, 0, SizerFlag::Expand | SizerFlag::All, 5);
	let attachment_info_label = StaticText::builder(&attachment_panel).with_label("").build();
	theme::style_muted_label(&attachment_info_label);
	attach_sizer.add(&attachment_info_label, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 5);
	let add_more_btn = Button::builder(&attachment_panel).with_label("Add more files").build();
	add_more_btn.set_font(&theme::Fonts::body());
	attach_sizer.add(&add_more_btn, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Bottom, 5);
	let send_attachment_btn = Button::builder(&attachment_panel).with_label("Send attachment").build();
	theme::style_primary_button(&send_attachment_btn);
	attach_sizer.add(&send_attachment_btn, 0, SizerFlag::Left | SizerFlag::Bottom, 5);
	attachment_panel.set_sizer(attach_sizer, true);
	attachment_panel.show(false);
	conv_sizer.add(&attachment_panel, 0, SizerFlag::Expand | SizerFlag::All, 5);
	let upload_gauge = Gauge::builder(&conversation_panel).with_range(100).build();
	upload_gauge.set_name("Upload progress");
	conv_sizer.add(&upload_gauge, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 10);
	upload_gauge.show(false);

	let voice_toolbar_sizer = BoxSizer::builder(Orientation::Horizontal).build();

	let discard_voice_btn = Button::builder(&conversation_panel).with_label("Discard voice message").build();
	discard_voice_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_icon_button(&discard_voice_btn, composer_bg);
	voice_toolbar_sizer.add(&discard_voice_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 5);
	discard_voice_btn.show(false);

	let pause_resume_btn = Button::builder(&conversation_panel).with_label("Pause recording").build();
	pause_resume_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_icon_button(&pause_resume_btn, composer_bg);
	voice_toolbar_sizer.add(&pause_resume_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 5);
	pause_resume_btn.show(false);

	let preview_voice_btn = Button::builder(&conversation_panel).with_label("Play preview").build();
	preview_voice_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_icon_button(&preview_voice_btn, composer_bg);
	voice_toolbar_sizer.add(&preview_voice_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 5);
	preview_voice_btn.show(false);

	voice_toolbar_sizer.add_stretch_spacer(1);

	let view_once_voice_check = CheckBox::builder(&conversation_panel).with_label("View once").build();
	view_once_voice_check.set_font(&theme::Fonts::body());
	if theme::use_custom_colours() {
		view_once_voice_check.set_background_color(composer_bg);
	}
	voice_toolbar_sizer.add(&view_once_voice_check, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 5);
	view_once_voice_check.show(false);

	let send_voice_btn = Button::builder(&conversation_panel).with_label("Send voice message").build();
	send_voice_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_icon_button(&send_voice_btn, composer_bg);
	voice_toolbar_sizer.add(&send_voice_btn, 0, SizerFlag::AlignCenterVertical, 0);
	send_voice_btn.show(false);

	conv_sizer.add_sizer(&voice_toolbar_sizer, 0, SizerFlag::Expand | SizerFlag::All, 10);
	paint_composer(
		&conversation_panel,
		ComposerParts {
			field: message_field,
			pause: pause_resume_btn,
			preview: preview_voice_btn,
			view_once: view_once_voice_check,
			send_voice: send_voice_btn,
		},
		composer_bg,
	);

	conversation_panel.set_sizer(conv_sizer, true);
	conversation_panel.show(false);

	outer_sizer.add_sizer(&left_sizer, 0, SizerFlag::Expand, 0);
	outer_sizer.add(&calls_btn_panel, 1, SizerFlag::Expand, 0);
	outer_sizer.add(&conversation_panel, 1, SizerFlag::Expand, 0);

	let root_sizer = BoxSizer::builder(Orientation::Vertical).build();
	root_sizer.add(&incoming_call_banner, 0, SizerFlag::Expand | SizerFlag::All, 0);
	root_sizer.add_sizer(&outer_sizer, 1, SizerFlag::Expand, 0);
	panel.set_sizer(root_sizer, true);
	fit_columns(&panel, &search_field, &conversations_list, &conversation_panel, &calls_btn_panel);

	ConversationsPanelParts {
		panel,
		search_label,
		search_field,
		chat_filter_label,
		chat_filter_combo,
		new_conv_btn,
		locked_chats_btn,
		post_media_status_btn,
		conversations_label,
		conversations_list,
		calls_btn_panel,
		calls_call_btn,
		calls_video_btn,
		incoming_call_banner,
		incoming_call_banner_label,
		incoming_call_restore_btn,
		incoming_call_answer_btn,
		incoming_call_decline_btn,
		conversation_panel,
		conv_data_btn,
		add_attachment_btn,
		emoji_btn,
		search_in_conv_btn,
		place_call_btn,
		place_video_call_btn,
		join_call_btn,
		top_btns_panel,
		search_panel,
		search_close_btn,
		search_conv_label,
		search_conv_field,
		messages_label,
		messages_list,
		messages_select_list,
		selection_bar_panel,
		selection_count_label,
		selection_copy_btn,
		selection_star_btn,
		selection_keep_btn,
		selection_delete_btn,
		selection_forward_btn,
		selection_save_btn,
		selection_cancel_btn,
		play_voice_btn,
		messages_announcer,
		read_more_btn,
		message_label,
		message_field,
		mention_list,
		cancel_edit_btn,
		remove_quote_btn,
		send_message_btn,
		record_voice_btn,

		discard_voice_btn,
		pause_resume_btn,
		preview_voice_btn,
		view_once_voice_check,
		send_voice_btn,
		attachment_panel,
		caption_label,
		caption_field,
		send_attachment_btn,
		add_more_btn,
		attachment_info_label,
		upload_gauge,
		audio_speed_btn,
		audio_progress_label,
		audio_slider,
		action_panel,
		action_retry_btn,
		action_cancel_upload_btn,
		action_open_btn,
		action_save_as_btn,
		action_download_btn,
	}
}
