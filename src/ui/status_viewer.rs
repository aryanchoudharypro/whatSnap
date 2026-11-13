use std::{
	cell::{Cell, RefCell},
	rc::Rc,
};

use wxdragon::prelude::*;

use crate::{
	AppState,
	ui::{keys, media_player::MediaPlayer, theme, window::WindowParts},
	ui_commands::status_extras::{STATUS_LIKE, STATUS_REACTIONS},
};

const PHOTO_EDGE: u32 = 1280;
const SHOW_MS: i32 = 6000;
const PER_CHAR_MS: i32 = 60;
const MAX_SHOW_MS: i32 = 20000;
const DEFAULT_BACKGROUND: u32 = 0xFF07_5E54;

#[derive(Clone)]
struct Entry {
	poster_jid: String,
	poster_name: String,
	from_me: bool,
	muted: bool,
	id: String,
	text: String,
	media_type: String,
	timestamp: i64,
	viewed: bool,
	my_reaction: String,
	background: u32,
	text_color: u32,
	font: i32,
}

pub(crate) enum ViewerOutcome {
	Closed,
	OpenChat(String),
}

struct Viewer {
	state: Rc<RefCell<AppState>>,
	dialog: Dialog,
	announcer: StaticText,
	bar: Panel,
	avatar: Panel,
	header: StaticText,
	stage: Panel,
	content: TextCtrl,
	photo: StaticBitmap,
	player: Rc<MediaPlayer>,
	pause_btn: Button,
	prev_btn: Button,
	next_btn: Button,
	auto_check: CheckBox,
	reactions_panel: Panel,
	reaction_btns: Vec<Button>,
	reply_panel: Panel,
	open_chat_btn: Button,
	reply_label: StaticText,
	reply_field: TextCtrl,
	reply_send_btn: Button,
	like_btn: Button,
	mute_btn: Button,
	viewers_panel: Panel,
	viewers_label: StaticText,
	viewers_list: ListBox,
	delete_btn: Button,
	save_btn: Button,
	timer: Timer<Dialog>,
	bar_timer: Timer<Dialog>,
	run: Cell<Run>,
	picture: RefCell<Option<(String, Option<wxdragon::bitmap::Bitmap>)>>,
	entries: RefCell<Vec<Entry>>,
	index: Cell<usize>,
	paused: Cell<bool>,
	media_path: RefCell<Option<String>>,
	outcome: RefCell<Option<ViewerOutcome>>,
	viewers_data: RefCell<Vec<(String, String)>>,
}

thread_local! {
	static OPEN: RefCell<Option<Rc<Viewer>>> = const { RefCell::new(None) };
}

fn send(state: &Rc<RefCell<AppState>>, action: &str, data: serde_json::Value) {
	let payload = serde_json::json!({ "action": action, "data": data });
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

fn text(data: &serde_json::Value, key: &str) -> String {
	data.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn argb(colour: u32) -> Colour {
	Colour::rgb((colour >> 16) as u8, (colour >> 8) as u8, colour as u8)
}

/// The updates the viewer plays, in the Status tab's order: like WA Web,
/// opening someone plays their updates and goes on through the other
/// people in the same part of the list (Recent, Viewed or Muted), and your
/// own row plays only your own updates.
fn entries(s: &AppState, start_id: &str) -> Vec<Entry> {
	use crate::ui_commands::status_extras::{section_of, sorted_groups};
	let Some(section) = s.status_groups.iter().find(|g| g.items.iter().any(|it| it.id == start_id)).map(section_of)
	else {
		return Vec::new();
	};
	sorted_groups(&s.status_groups)
		.into_iter()
		.filter(|(sec, _)| *sec == section)
		.flat_map(|(_, g)| {
			g.items.iter().map(move |it| Entry {
				poster_jid: g.poster_jid.clone(),
				poster_name: g.poster_name.clone(),
				from_me: g.from_me,
				muted: g.muted,
				id: it.id.clone(),
				text: it.text.clone(),
				media_type: it.media_type.clone(),
				timestamp: it.timestamp,
				viewed: it.viewed,
				my_reaction: it.my_reaction.clone(),
				background: it.background,
				text_color: it.text_color,
				font: it.font,
			})
		})
		.collect()
}

fn kind(entry: &Entry) -> &'static str {
	match entry.media_type.as_str() {
		"image" => crate::i18n::tr("Photo"),
		"video" => crate::i18n::tr("Video"),
		"audio" | "voice" => crate::i18n::tr("Voice note"),
		"" => crate::i18n::tr("Text"),
		_ => crate::i18n::tr("Media"),
	}
}

fn show_ms(entry: &Entry) -> i32 {
	(SHOW_MS + PER_CHAR_MS * entry.text.chars().count() as i32).min(MAX_SHOW_MS)
}

pub(crate) fn show(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, start_id: &str) -> ViewerOutcome {
	let (list, auto) = {
		let s = state.borrow();
		(entries(&s, start_id), s.settings.status_auto_advance)
	};
	let Some(start) = list.iter().position(|e| e.id == start_id) else { return ViewerOutcome::Closed };
	let viewer = build(state, parts, list, auto);
	OPEN.with(|open| *open.borrow_mut() = Some(viewer.clone()));
	viewer.show_entry(start);
	viewer.next_btn.set_focus();
	if theme::use_custom_colours() {
		viewer.bar_timer.start(100, false);
	}
	viewer.dialog.show_modal();
	OPEN.with(|open| open.borrow_mut().take());
	viewer.bar_timer.stop();
	viewer.timer.stop();
	viewer.player.stop();
	// A dialog built per use must be destroyed, or it and every closure bound
	// to it live on hidden.
	viewer.dialog.destroy();
	viewer.outcome.borrow_mut().take().unwrap_or(ViewerOutcome::Closed)
}

/// WhatsApp's status viewer is dark whichever theme the app uses, like the
/// call screen; a shade off the palette on purpose so a theme change
/// (which swaps palette colours) leaves it alone.
pub(crate) const STAGE_BG: Colour = Colour::rgb(0x0C, 0x15, 0x1B);
pub(crate) const STAGE_TEXT: Colour = Colour::rgb(0xE9, 0xED, 0xEE);
pub(crate) const STAGE_MUTED: Colour = Colour::rgb(0x8C, 0x9C, 0xA6);
const AVATAR_SIDE: i32 = 40;

fn on_stage(widget: &impl WxWidget, text: Colour) {
	if theme::use_custom_colours() {
		widget.set_background_color(STAGE_BG);
		widget.set_foreground_color(text);
	}
}

/// Keeps a label's text (it names the control after it for a screen
/// reader) while taking no room, as WhatsApp shows no such label.
fn hide_visually(label: &StaticText) {
	if theme::use_custom_colours() {
		label.set_min_size(Size::new(1, 1));
		label.set_size(Size::new(1, 1));
	}
}

fn build(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, list: Vec<Entry>, auto: bool) -> Rc<Viewer> {
	let dialog = Dialog::builder(&parts.frame, crate::i18n::tr("Status"))
		.with_size(820, 760)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder | DialogStyle::MaximizeBox)
		.build();
	theme::style_content(&dialog);
	on_stage(&dialog, STAGE_TEXT);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	on_stage(&panel, STAGE_TEXT);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	// The dialog is the foreground window while it's up, so its own live
	// region is the one a screen reader speaks.
	let announcer = StaticText::builder(&panel).with_label("").build();
	live_region::set_live_region(&announcer);
	sizer.add(&announcer, 0, SizerFlag::All, 0);

	// WhatsApp's top strip: one bar per update from this person, filling as
	// each one plays, then their picture beside the name and time. Only
	// painted; the header text below is what a screen reader reads.
	let bar = Panel::builder(&panel).build();
	on_stage(&bar, STAGE_TEXT);
	bar.set_min_size(Size::new(-1, 12));
	sizer.add(&bar, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let header_row = BoxSizer::builder(Orientation::Horizontal).build();
	let avatar = Panel::builder(&panel).build();
	on_stage(&avatar, STAGE_TEXT);
	avatar.set_min_size(Size::new(AVATAR_SIDE, AVATAR_SIDE));
	header_row.add(&avatar, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 10);
	avatar.show(theme::use_custom_colours());
	let header = StaticText::builder(&panel).with_label("").build();
	theme::style_heading(&header);
	on_stage(&header, STAGE_TEXT);
	header_row.add(&header, 1, SizerFlag::AlignCenterVertical, 0);
	sizer.add_sizer(&header_row, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	// The stage: a text update's text in its colours filling it, or a photo
	// or video with its caption under it. The text is read-only, so a
	// screen reader can go through it line by line.
	let stage_row = BoxSizer::builder(Orientation::Horizontal).build();
	let stage = Panel::builder(&panel).build();
	on_stage(&stage, STAGE_TEXT);
	let stage_column = BoxSizer::builder(Orientation::Vertical).build();
	let content_label = StaticText::builder(&stage).with_label(crate::i18n::tr("Status &update")).build();
	theme::style_muted_label(&content_label);
	on_stage(&content_label, STAGE_MUTED);
	hide_visually(&content_label);
	stage_column.add(&content_label, 0, SizerFlag::Left, 0);

	let content = TextCtrl::builder(&stage)
		.with_style(
			TextCtrlStyle::MultiLine
				| TextCtrlStyle::ReadOnly
				| TextCtrlStyle::Rich2
				| TextCtrlStyle::Centre
				| borderless(),
		)
		.build();
	content.set_font(&theme::Fonts::body());
	content.set_min_size(Size::new(-1, 70));
	on_stage(&content, STAGE_TEXT);

	let empty = wxdragon::bitmap::Bitmap::from_rgba(&[0, 0, 0, 0], 1, 1).expect("1x1 bitmap");
	let photo =
		StaticBitmap::builder(&stage).with_bitmap(Some(empty)).with_scale_mode(Some(ScaleMode::AspectFit)).build();
	on_stage(&photo, STAGE_TEXT);
	photo.set_min_size(Size::new(480, 320));
	photo.show(false);

	let player_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let player = MediaPlayer::build(&stage, &player_sizer, announcer);
	player.panel.show(false);

	// Built in the order a screen reader and Tab go through them (the label
	// right before the update it names, then the player), laid out as
	// WhatsApp does: the media first, its caption below.
	stage_column.add(&photo, 4, SizerFlag::Expand, 0);
	stage_column.add_sizer(&player_sizer, 4, SizerFlag::Expand, 0);
	stage_column.add(&content, 1, SizerFlag::Expand | SizerFlag::Top, 6);
	stage.set_sizer(stage_column, true);

	let prev_btn = Button::builder(&panel).with_label(crate::i18n::tr("&Previous")).build();
	let pause_btn = Button::builder(&panel).with_label(crate::i18n::tr("Pa&use")).build();
	let next_btn = Button::builder(&panel).with_label(crate::i18n::tr("&Next")).build();
	for btn in [&prev_btn, &pause_btn, &next_btn] {
		crate::ui::owner_draw::install_icon_button(btn, STAGE_BG);
	}
	stage_row.add(&prev_btn, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 8);
	stage_row.add(&stage, 1, SizerFlag::Expand, 0);
	stage_row.add(&next_btn, 0, SizerFlag::AlignCenterVertical | SizerFlag::Left, 8);
	header_row.add(&pause_btn, 0, SizerFlag::AlignCenterVertical | SizerFlag::Left, 8);
	sizer.add_sizer(&stage_row, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	// A native check box can't draw light text on the dark stage without
	// losing its check box role, so it sits on a light chip of its own.
	let auto_panel = Panel::builder(&panel).build();
	theme::style_sidebar(&auto_panel);
	let auto_row = BoxSizer::builder(Orientation::Horizontal).build();
	let auto_check =
		CheckBox::builder(&auto_panel).with_label(crate::i18n::tr("Play next update &automatically")).build();
	auto_check.set_value(auto);
	auto_row.add(&auto_check, 0, SizerFlag::AlignCenterVertical | SizerFlag::All, 6);
	auto_panel.set_sizer(auto_row, true);

	let reactions_panel = Panel::builder(&panel).build();
	on_stage(&reactions_panel, STAGE_TEXT);
	let reactions_row = BoxSizer::builder(Orientation::Horizontal).build();
	reactions_row.add_stretch_spacer(1);
	let reactions_label = StaticText::builder(&reactions_panel).with_label(crate::i18n::tr("React:")).build();
	theme::style_muted_label(&reactions_label);
	on_stage(&reactions_label, STAGE_MUTED);
	reactions_row.add(&reactions_label, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 8);
	let reaction_btns: Vec<Button> = STATUS_REACTIONS
		.iter()
		.map(|emoji| {
			let btn = Button::builder(&reactions_panel).with_label(emoji).build();
			btn.set_font(&theme::Fonts::body());
			reactions_row.add(&btn, 0, SizerFlag::Right, 4);
			btn
		})
		.collect();
	reactions_row.add_stretch_spacer(1);
	reactions_panel.set_sizer(reactions_row, true);
	sizer.add(&reactions_panel, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	// WhatsApp's reply bar: a round dark box with the field in it.
	let reply_panel = Panel::builder(&panel).build();
	on_stage(&reply_panel, STAGE_TEXT);
	let reply_row = BoxSizer::builder(Orientation::Horizontal).build();
	let open_chat_btn = Button::builder(&reply_panel).with_label(crate::i18n::tr("&Open chat")).build();
	reply_row.add(&open_chat_btn, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 12);
	let reply_label = StaticText::builder(&reply_panel).with_label(crate::i18n::tr("&Reply")).build();
	theme::style_muted_label(&reply_label);
	on_stage(&reply_label, STAGE_MUTED);
	reply_row.add(&reply_label, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 8);
	let reply_field = TextCtrl::builder(&reply_panel).with_style(TextCtrlStyle::ProcessEnter | borderless()).build();
	reply_field.set_font(&theme::Fonts::body());
	if theme::use_custom_colours() {
		reply_field.set_background_color(REPLY_BOX);
		reply_field.set_foreground_color(STAGE_TEXT);
	}
	reply_row.add(&reply_field, 1, SizerFlag::AlignCenterVertical | SizerFlag::Left | SizerFlag::Right, 14);
	let reply_send_btn = Button::builder(&reply_panel).with_label(crate::i18n::tr("Send reply")).build();
	crate::ui::owner_draw::install_icon_button(&reply_send_btn, STAGE_BG);
	reply_row.add(&reply_send_btn, 0, SizerFlag::AlignCenterVertical, 0);
	// WA Web's like button sits after the reply box (WAWebStatusComposeBox:
	// "Like" or "Unlike").
	let like_btn = Button::builder(&reply_panel).with_label(crate::i18n::tr("&Like")).build();
	reply_row.add(&like_btn, 0, SizerFlag::AlignCenterVertical | SizerFlag::Left, 8);
	reply_panel.set_sizer(reply_row, true);
	paint_reply_box(&reply_panel, reply_field);
	sizer.add(&reply_panel, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let viewers_panel = Panel::builder(&panel).build();
	on_stage(&viewers_panel, STAGE_TEXT);
	let viewers_column = BoxSizer::builder(Orientation::Vertical).build();
	let viewers_label = StaticText::builder(&viewers_panel).with_label(crate::i18n::tr("&Viewed by")).build();
	theme::style_muted_label(&viewers_label);
	on_stage(&viewers_label, STAGE_MUTED);
	viewers_column.add(&viewers_label, 0, SizerFlag::Bottom, 4);
	let viewers_list = ListBox::builder(&viewers_panel).build();
	viewers_list.set_font(&theme::Fonts::body());
	viewers_list.set_min_size(Size::new(-1, 110));
	if theme::use_custom_colours() {
		viewers_list.set_background_color(REPLY_BOX);
		viewers_list.set_foreground_color(STAGE_TEXT);
	}
	viewers_column.add(&viewers_list, 0, SizerFlag::Expand, 0);
	viewers_panel.set_sizer(viewers_column, true);
	sizer.add(&viewers_panel, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let more_row = BoxSizer::builder(Orientation::Horizontal).build();
	more_row.add(&auto_panel, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 16);
	more_row.add_stretch_spacer(1);
	let mute_btn = Button::builder(&panel).with_label(crate::i18n::tr("&Mute")).build();
	more_row.add(&mute_btn, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 8);
	let delete_btn = Button::builder(&panel).with_label(crate::i18n::tr("&Delete...")).build();
	more_row.add(&delete_btn, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 8);
	let save_btn = Button::builder(&panel).with_label(crate::i18n::tr("&Save as...")).build();
	more_row.add(&save_btn, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 8);
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label(crate::i18n::tr("Close")).build();
	more_row.add(&close_btn, 0, SizerFlag::AlignCenterVertical, 0);
	sizer.add_sizer(&more_row, 0, SizerFlag::Expand | SizerFlag::All, 10);

	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let viewer = Rc::new(Viewer {
		state: state.clone(),
		dialog,
		announcer,
		bar,
		avatar,
		header,
		stage,
		content,
		photo,
		player,
		pause_btn,
		prev_btn,
		next_btn,
		auto_check,
		reactions_panel,
		reaction_btns,
		reply_panel,
		open_chat_btn,
		reply_label,
		reply_field,
		reply_send_btn,
		like_btn,
		mute_btn,
		viewers_panel,
		viewers_label,
		viewers_list,
		delete_btn,
		save_btn,
		timer: Timer::new(&dialog),
		bar_timer: Timer::new(&dialog),
		run: Cell::new(Run::default()),
		picture: RefCell::new(None),
		entries: RefCell::new(list),
		index: Cell::new(0),
		paused: Cell::new(false),
		media_path: RefCell::new(None),
		outcome: RefCell::new(None),
		viewers_data: RefCell::new(Vec::new()),
	});
	bind(&viewer, close_btn);
	paint_stage(&viewer);
	viewer
}

const REPLY_BOX: Colour = Colour::rgb(0x23, 0x2E, 0x35);
const BORDER_NONE: i64 = 0x0020_0000;

fn borderless() -> TextCtrlStyle {
	if theme::use_custom_colours() { TextCtrlStyle::from_bits_retain(BORDER_NONE) } else { TextCtrlStyle::Default }
}

fn paint_reply_box(panel: &Panel, field: TextCtrl) {
	if !theme::use_custom_colours() {
		return;
	}
	let panel_c = *panel;
	panel.set_background_style(BackgroundStyle::Paint);
	panel.on_paint(move |_| {
		let dc = AutoBufferedPaintDC::new(&panel_c);
		let size = panel_c.get_client_size();
		dc.set_brush(STAGE_BG, BrushStyle::Solid);
		dc.set_pen(STAGE_BG, 1, PenStyle::Solid);
		dc.draw_rectangle(0, 0, size.width, size.height);
		if !field.is_shown() {
			return;
		}
		let (p, s) = (field.get_position(), field.get_size());
		let (x, y, w, h) = (p.x - 14, p.y - 8, s.width + 28, s.height + 16);
		if field.has_focus() {
			dc.set_brush(STAGE_TEXT, BrushStyle::Solid);
			dc.set_pen(STAGE_TEXT, 1, PenStyle::Solid);
			dc.draw_rounded_rectangle(x - 2, y - 2, w + 4, h + 4, (h + 4) as f64 / 2.0);
		}
		dc.set_brush(REPLY_BOX, BrushStyle::Solid);
		dc.set_pen(REPLY_BOX, 1, PenStyle::Solid);
		dc.draw_rounded_rectangle(x, y, w, h, h as f64 / 2.0);
	});
	field.on_set_focus(move |event| {
		panel_c.refresh(false, None);
		event.skip(true);
	});
	field.on_kill_focus(move |event| {
		panel_c.refresh(false, None);
		event.skip(true);
	});
	panel.on_size(move |event| {
		panel_c.refresh(true, None);
		event.skip(true);
	});
}

/// Paints the progress bars, the poster's picture and, for a text update,
/// the stage in the poster's colour around the text.
fn paint_stage(viewer: &Rc<Viewer>) {
	if !theme::use_custom_colours() {
		return;
	}
	let weak = Rc::downgrade(viewer);
	let bar = viewer.bar;
	bar.set_background_style(BackgroundStyle::Paint);
	bar.on_paint(move |_| {
		let dc = AutoBufferedPaintDC::new(&bar);
		let size = bar.get_client_size();
		dc.set_brush(STAGE_BG, BrushStyle::Solid);
		dc.set_pen(STAGE_BG, 1, PenStyle::Solid);
		dc.draw_rectangle(0, 0, size.width, size.height);
		let Some(v) = weak.upgrade() else { return };
		let Some(entry) = v.current() else { return };
		let (at, count) = v.position_in_group(&entry);
		let fraction = v.fraction();
		let gap = 4;
		let count = count.max(1) as i32;
		let width = ((size.width - gap * (count - 1)) / count).max(1);
		let (top, height) = ((size.height - 3) / 2, 3);
		for i in 0..count {
			let x = i * (width + gap);
			dc.set_brush(BAR_EMPTY, BrushStyle::Solid);
			dc.set_pen(BAR_EMPTY, 1, PenStyle::Solid);
			dc.draw_rounded_rectangle(x, top, width, height, 1.5);
			let filled = match (i + 1).cmp(&(at as i32)) {
				std::cmp::Ordering::Less => width,
				std::cmp::Ordering::Equal => (width as f64 * fraction).round() as i32,
				std::cmp::Ordering::Greater => 0,
			};
			if filled > 0 {
				dc.set_brush(STAGE_TEXT, BrushStyle::Solid);
				dc.set_pen(STAGE_TEXT, 1, PenStyle::Solid);
				dc.draw_rounded_rectangle(x, top, filled, height, 1.5);
			}
		}
	});

	let weak = Rc::downgrade(viewer);
	let avatar = viewer.avatar;
	avatar.set_background_style(BackgroundStyle::Paint);
	avatar.on_paint(move |_| {
		let dc = AutoBufferedPaintDC::new(&avatar);
		let size = avatar.get_client_size();
		dc.set_brush(STAGE_BG, BrushStyle::Solid);
		dc.set_pen(STAGE_BG, 1, PenStyle::Solid);
		dc.draw_rectangle(0, 0, size.width, size.height);
		let Some(v) = weak.upgrade() else { return };
		let Some(entry) = v.current() else { return };
		let side = size.width.min(size.height);
		if let Some(picture) = v.picture_for(&entry.poster_jid, side) {
			dc.draw_bitmap(&picture, 0, (size.height - side) / 2, true);
			return;
		}
		dc.set_brush(AVATAR_EMPTY, BrushStyle::Solid);
		dc.set_pen(AVATAR_EMPTY, 1, PenStyle::Solid);
		dc.draw_circle(side / 2, size.height / 2, side / 2);
		let initial: String = entry.poster_name.chars().find(|c| c.is_alphanumeric()).into_iter().collect();
		dc.set_font(&theme::Fonts::body_strong());
		dc.set_text_foreground(STAGE_TEXT);
		let (tw, th) = dc.get_text_extent(&initial);
		dc.draw_text(&initial, (side - tw) / 2, (size.height - th) / 2);
	});

	let weak = Rc::downgrade(viewer);
	let stage = viewer.stage;
	stage.set_background_style(BackgroundStyle::Paint);
	stage.on_paint(move |_| {
		let dc = AutoBufferedPaintDC::new(&stage);
		let size = stage.get_client_size();
		let colour = weak
			.upgrade()
			.and_then(|v| v.current())
			.filter(|e| e.media_type.is_empty())
			.map_or(STAGE_BG, |e| argb(if e.background == 0 { DEFAULT_BACKGROUND } else { e.background }));
		dc.set_brush(colour, BrushStyle::Solid);
		dc.set_pen(colour, 1, PenStyle::Solid);
		dc.draw_rounded_rectangle(0, 0, size.width, size.height, 8.0);
	});
	stage.on_size(move |event| {
		stage.refresh(true, None);
		event.skip(true);
	});

	let weak = Rc::downgrade(viewer);
	viewer.bar_timer.on_tick(move |_| {
		if let Some(v) = weak.upgrade() {
			v.bar.refresh(false, None);
		}
	});
}

const BAR_EMPTY: Colour = Colour::rgb(0x4A, 0x55, 0x5C);
const AVATAR_EMPTY: Colour = Colour::rgb(0x6A, 0x71, 0x76);

/// The plain-timer side of an update that isn't a video: how long it shows
/// and how much of that is left, so a pause carries on where it stopped.
#[derive(Clone, Copy, Default)]
struct Run {
	total_ms: i32,
	left_ms: i32,
	started: Option<std::time::Instant>,
}

fn bind(viewer: &Rc<Viewer>, close_btn: Button) {
	let weak = Rc::downgrade(viewer);
	viewer.timer.on_tick(handler(&weak, Viewer::advance_tick));
	viewer.prev_btn.on_click(handler(&weak, Viewer::previous));
	viewer.next_btn.on_click(handler(&weak, Viewer::next));
	viewer.pause_btn.on_click(handler(&weak, Viewer::toggle_pause));
	viewer.auto_check.on_toggled(handler(&weak, Viewer::auto_changed));
	viewer.open_chat_btn.on_click(handler(&weak, Viewer::open_chat));
	viewer.reply_send_btn.on_click(handler(&weak, Viewer::send_reply));
	viewer.reply_field.on_text_enter(handler(&weak, Viewer::send_reply));
	viewer.like_btn.on_click(handler(&weak, Viewer::toggle_like));
	viewer.mute_btn.on_click(handler(&weak, Viewer::toggle_mute));
	viewer.delete_btn.on_click(handler(&weak, Viewer::delete));
	viewer.save_btn.on_click(handler(&weak, Viewer::save_as));
	viewer.viewers_list.on_item_double_clicked(handler(&weak, Viewer::open_viewer_chat));
	let dialog = viewer.dialog;
	close_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));
	for (i, btn) in viewer.reaction_btns.iter().enumerate() {
		let weak = Rc::downgrade(viewer);
		btn.on_click(move |_| {
			if let Some(v) = weak.upgrade() {
				v.react(STATUS_REACTIONS[i]);
			}
		});
	}
	{
		let weak = Rc::downgrade(viewer);
		viewer.player.on_ended(move || {
			if let Some(v) = weak.upgrade()
				&& v.auto_check.is_checked()
				&& !v.paused.get()
				&& !v.replying()
			{
				v.next();
			}
		});
	}

	let weak = Rc::downgrade(viewer);
	viewer.dialog.bind_internal(EventType::CHAR_HOOK, move |event| {
		let data = WindowEventData::new(event);
		let WindowEventData::Keyboard(ref key) = data else { return };
		let Some(v) = weak.upgrade() else { return };
		let code = key.get_key_code();
		if keys::is_escape(code) {
			v.dialog.end_modal(ID_CANCEL);
			data.skip(false);
			return;
		}
		let focus = crate::ui::media_player::focused_window();
		let in_reply = focus == v.reply_field.get_handle();
		if in_reply {
			data.skip(true);
			return;
		}
		let in_content = focus == v.content.get_handle();
		if in_content && (keys::is_left(code) || keys::is_right(code) || keys::is_up(code) || keys::is_down(code)) {
			data.skip(true);
			return;
		}
		if v.player.handle_key(key) {
			data.skip(false);
			return;
		}
		let plain = !key.control_down() && !key.alt_down() && !key.shift_down();
		let on_list = focus == v.viewers_list.get_handle();
		if on_list && keys::is_enter(code) {
			v.open_viewer_chat();
			data.skip(false);
			return;
		}
		let handled = plain
			&& if (keys::is_left(code) || keys::is_letter(code, 'P')) && !on_list {
				v.previous();
				true
			} else if (keys::is_right(code) || keys::is_letter(code, 'N')) && !on_list {
				v.next();
				true
			} else if keys::is_space(code) && !is_button_or_check(&v, focus) {
				v.toggle_pause();
				true
			} else if keys::is_letter(code, 'C') && !in_content {
				v.content.set_focus();
				true
			} else if keys::is_letter(code, 'R') && v.reply_panel.is_shown() {
				v.reply_field.set_focus();
				true
			} else if keys::is_letter(code, 'H') && v.reply_panel.is_shown() {
				v.toggle_like();
				true
			} else if keys::is_letter(code, 'M') && v.mute_btn.is_shown() {
				v.toggle_mute();
				true
			} else if (keys::is_letter(code, 'D') || keys::is_delete(code)) && v.delete_btn.is_shown() {
				v.delete();
				true
			} else if keys::is_letter(code, 'V') && v.viewers_panel.is_shown() {
				v.viewers_list.set_focus();
				true
			} else if keys::is_letter(code, 'S') && v.save_btn.is_shown() && v.save_btn.is_enabled() {
				v.save_as();
				true
			} else if (keys::is_letter(code, 'I') || keys::is_letter(code, 'T')) && !on_list {
				v.speak_info();
				true
			} else if let Some(digit) = reaction_digit(code) {
				if v.reactions_panel.is_shown() && digit < STATUS_REACTIONS.len() {
					v.react(STATUS_REACTIONS[digit]);
					true
				} else {
					false
				}
			} else {
				false
			};
		data.skip(!handled);
	});
}

/// What the viewer says about your reaction to an update: a like, an emoji,
/// or nothing.
fn reaction_note(mine: &str) -> String {
	match mine {
		"" => String::new(),
		STATUS_LIKE => crate::i18n::tr(" You liked this.").to_string(),
		emoji => crate::i18n::tr_fmt(" You reacted {}.", &[&emoji]),
	}
}

fn reaction_digit(code: Option<i32>) -> Option<usize> {
	let c = code?;
	if ('1' as i32..='8' as i32).contains(&c) {
		Some((c - '1' as i32) as usize)
	} else if (WXK_NUMPAD1..=WXK_NUMPAD8).contains(&c) {
		Some((c - WXK_NUMPAD1) as usize)
	} else {
		None
	}
}

fn handler<E>(weak: &std::rc::Weak<Viewer>, f: fn(&Viewer)) -> impl FnMut(E) + 'static {
	let weak = weak.clone();
	move |_| {
		if let Some(v) = weak.upgrade() {
			f(&v);
		}
	}
}

fn is_button_or_check(v: &Viewer, hwnd: *mut std::ffi::c_void) -> bool {
	let mut controls = vec![
		v.pause_btn.get_handle(),
		v.prev_btn.get_handle(),
		v.next_btn.get_handle(),
		v.auto_check.get_handle(),
		v.open_chat_btn.get_handle(),
		v.reply_send_btn.get_handle(),
		v.like_btn.get_handle(),
		v.mute_btn.get_handle(),
		v.delete_btn.get_handle(),
		v.save_btn.get_handle(),
	];
	controls.extend(v.reaction_btns.iter().map(|b| b.get_handle()));
	controls.contains(&hwnd) || window_class_is_button(hwnd)
}

fn window_class_is_button(hwnd: *mut std::ffi::c_void) -> bool {
	let mut name = [0u16; 16];
	let len = unsafe {
		windows::Win32::UI::WindowsAndMessaging::GetClassNameW(windows::Win32::Foundation::HWND(hwnd), &mut name)
	};
	String::from_utf16_lossy(&name[..len.max(0) as usize]).eq_ignore_ascii_case("Button")
}

impl Viewer {
	fn current(&self) -> Option<Entry> {
		self.entries.borrow().get(self.index.get()).cloned()
	}

	fn replying(&self) -> bool {
		crate::ui::media_player::focused_window() == self.reply_field.get_handle()
			|| !self.reply_field.get_value().trim().is_empty()
	}

	fn show_entry(&self, index: usize) {
		let Some(entry) = self.entries.borrow().get(index).cloned() else { return };
		self.index.set(index);
		self.timer.stop();
		self.run.set(Run::default());
		self.player.stop();
		*self.media_path.borrow_mut() = None;
		self.reply_field.set_value("");

		let (position, count) = self.position_in_group(&entry);
		let time = whatsnap::LocalTime::from_unix(entry.timestamp).map(whatsnap::time_or_day).unwrap_or_default();
		let who = if entry.from_me { crate::i18n::tr("My status").to_string() } else { entry.poster_name.clone() };
		let header = crate::i18n::tr_fmt("{}, {} of {}, {}", &[&who, &position, &count, &time]);
		self.header.set_label(&header);
		self.dialog.set_label(&format!("Status: {who}"));

		let body = match entry.media_type.as_str() {
			"" => entry.text.clone(),
			"audio" | "voice" if entry.text.is_empty() => crate::i18n::tr("Voice note status.").to_string(),
			"audio" | "voice" => format!("{}: {}", crate::i18n::tr("Voice note"), entry.text),
			_ if entry.text.is_empty() => format!("{}.", kind(&entry)),
			_ => format!("{}. {}", kind(&entry), entry.text),
		};
		self.content.set_value(&body);
		if entry.media_type.is_empty() {
			// The poster's colours, unless Windows' own colours are in use
			// (high contrast), which the app never paints over.
			if theme::use_custom_colours() {
				let background = if entry.background == 0 { DEFAULT_BACKGROUND } else { entry.background };
				let text_color = if entry.text_color == 0 { 0xFFFF_FFFF } else { entry.text_color };
				self.content.set_background_color(argb(background));
				self.content.set_foreground_color(argb(text_color));
			}
			let face = match entry.font {
				1 => "Georgia",
				2 => "Segoe Script",
				6 => "Segoe UI Black",
				7 => "Comic Sans MS",
				8 => "Georgia",
				9 => "Impact",
				10 => "Courier New",
				_ => "Segoe UI",
			};
			let font =
				Font::builder().with_point_size(15).with_face_name(face).build().unwrap_or_else(theme::Fonts::heading);
			self.content.set_font(&font);
			self.content.set_min_size(Size::new(-1, 220));
		} else {
			// The caption under the media, light on the dark stage.
			theme::style_content(&self.content);
			on_stage(&self.content, STAGE_TEXT);
			self.content.set_font(&theme::Fonts::body());
			self.content.set_min_size(Size::new(-1, 50));
		}
		self.content.refresh(true, None);
		self.stage.refresh(true, None);
		self.avatar.refresh(false, None);
		self.bar.refresh(false, None);

		self.photo.show(false);
		self.player.panel.show(false);
		self.save_btn.enable(false);
		self.save_btn.show(!entry.media_type.is_empty());

		self.reactions_panel.show(!entry.from_me);
		self.reply_panel.show(!entry.from_me);
		self.mute_btn.show(!entry.from_me);
		self.viewers_panel.show(entry.from_me);
		self.delete_btn.show(entry.from_me);
		if entry.from_me {
			self.viewers_list.clear();
			self.viewers_label.set_label(crate::i18n::tr("&Viewed by"));
			send(&self.state, "get_status_viewers", serde_json::json!({ "id": entry.id }));
		} else {
			self.open_chat_btn.set_label(&crate::i18n::tr_fmt("&Open chat with {}", &[&entry.poster_name]));
			self.reply_label.set_label(&crate::i18n::tr_fmt("&Reply to {}", &[&entry.poster_name]));
			self.show_mute(&entry);
			self.show_reaction(&entry.my_reaction);
		}
		self.prev_btn.enable(index > 0);
		self.dialog.layout();
		self.keep_focus();

		let mut spoken = format!("{header}. {body}");
		if !entry.from_me {
			spoken.push_str(&reaction_note(&entry.my_reaction));
		}
		if !entry.media_type.is_empty() {
			send(
				&self.state,
				"prepare_media",
				serde_json::json!({ "jid": "status@broadcast", "message_id": entry.id }),
			);
		} else {
			self.start_timer(&entry);
		}
		crate::platform::announce::announce(self.announcer, &spoken);
		self.mark_viewed(&entry);
	}

	fn refresh_bar(&self) {
		self.bar.refresh(false, None);
	}

	fn keep_focus(&self) {
		use windows::Win32::{
			Foundation::HWND,
			UI::{Input::KeyboardAndMouse::IsWindowEnabled, WindowsAndMessaging::IsWindowVisible},
		};
		let focus = HWND(crate::ui::media_player::focused_window());
		let usable =
			!focus.is_invalid() && unsafe { IsWindowVisible(focus).as_bool() && IsWindowEnabled(focus).as_bool() };
		if !usable {
			self.next_btn.set_focus();
		}
	}

	fn position_in_group(&self, entry: &Entry) -> (usize, usize) {
		let entries = self.entries.borrow();
		let theirs: Vec<&Entry> = entries.iter().filter(|e| e.poster_jid == entry.poster_jid).collect();
		let at = theirs.iter().position(|e| e.id == entry.id).unwrap_or(0);
		(at + 1, theirs.len())
	}

	fn mark_viewed(&self, entry: &Entry) {
		if entry.from_me || entry.viewed {
			return;
		}
		if let Some(e) = self.entries.borrow_mut().iter_mut().find(|e| e.id == entry.id) {
			e.viewed = true;
		}
		{
			let mut s = self.state.borrow_mut();
			if let Some(group) = s.status_groups.iter_mut().find(|g| g.poster_jid == entry.poster_jid) {
				if let Some(item) = group.items.iter_mut().find(|it| it.id == entry.id) {
					item.viewed = true;
				}
				group.all_viewed = group.items.iter().all(|it| it.viewed);
			}
		}
		send(&self.state, "mark_status_viewed", serde_json::json!({ "id": entry.id, "poster_jid": entry.poster_jid }));
	}

	/// Starts showing an update that isn't a video for its full time.
	fn start_timer(&self, entry: &Entry) {
		let total = show_ms(entry);
		self.run.set(Run { total_ms: total, left_ms: total, started: None });
		self.resume_run();
	}

	/// Carries on with what's left of the current update's time.
	fn resume_run(&self) {
		let mut run = self.run.get();
		if run.started.is_some() || run.left_ms <= 0 || !self.auto_check.is_checked() || self.paused.get() {
			return;
		}
		run.started = Some(std::time::Instant::now());
		self.run.set(run);
		self.timer.start(run.left_ms, true);
	}

	/// Stops the clock, keeping how much time the update has left.
	fn halt_run(&self) {
		self.timer.stop();
		let mut run = self.run.get();
		if let Some(started) = run.started.take() {
			let spent = started.elapsed().as_millis().min(i32::MAX as u128) as i32;
			run.left_ms = (run.left_ms - spent).max(1);
		}
		self.run.set(run);
	}

	/// How far the current update has played, for the bar at the top.
	fn fraction(&self) -> f64 {
		if let Some(done) = self.player.progress() {
			return done;
		}
		let run = self.run.get();
		if run.total_ms <= 0 {
			return 0.0;
		}
		let running = run.started.map_or(0, |t| t.elapsed().as_millis().min(i32::MAX as u128) as i32);
		((run.total_ms - run.left_ms + running) as f64 / run.total_ms as f64).clamp(0.0, 1.0)
	}

	fn picture_for(&self, jid: &str, side: i32) -> Option<wxdragon::bitmap::Bitmap> {
		if let Some((cached, picture)) = self.picture.borrow().as_ref()
			&& cached == jid
		{
			return picture.clone();
		}
		// Not known yet: the avatar cache asks for it, and the next paint
		// (the bar repaints every 100 ms) tries again.
		let path = crate::ui::avatars::path_of(jid)?;
		let picture = crate::ui::owner_draw::round_picture(&path, side.max(1) as u32);
		*self.picture.borrow_mut() = Some((jid.to_string(), picture.clone()));
		picture
	}

	fn advance_tick(&self) {
		if self.paused.get() || !self.auto_check.is_checked() {
			return;
		}
		if self.replying() {
			// Wait while a reply is being typed, holding the bar full.
			self.run.set(Run { total_ms: 1, left_ms: 1, started: None });
			self.timer.start(2000, true);
			return;
		}
		self.next();
	}

	fn next(&self) {
		let next = self.index.get() + 1;
		if next < self.entries.borrow().len() {
			self.show_entry(next);
		} else {
			self.dialog.end_modal(ID_CANCEL);
		}
	}

	fn previous(&self) {
		match self.index.get().checked_sub(1) {
			Some(prev) => self.show_entry(prev),
			None => {
				crate::platform::announce::announce(self.announcer, crate::i18n::tr("This is the first status update."))
			}
		}
	}

	fn toggle_pause(&self) {
		let paused = !self.paused.get();
		self.paused.set(paused);
		self.pause_btn.set_label(if paused { crate::i18n::tr("Res&ume") } else { crate::i18n::tr("Pa&use") });
		if paused {
			self.halt_run();
			self.player.pause();
			crate::platform::announce::announce(self.announcer, crate::i18n::tr("Paused"));
		} else {
			if self.player.is_loaded() {
				self.player.play();
			} else if let Some(entry) = self.current()
				&& (entry.media_type.is_empty() || self.media_path.borrow().is_some())
			{
				if self.run.get().total_ms > 0 {
					self.resume_run();
				} else {
					self.start_timer(&entry);
				}
			}
			crate::platform::announce::announce(self.announcer, crate::i18n::tr("Playing"));
		}
	}

	fn auto_changed(&self) {
		let auto = self.auto_check.is_checked();
		{
			let mut s = self.state.borrow_mut();
			s.settings.status_auto_advance = auto;
			s.settings.save();
		}
		if !auto {
			self.halt_run();
		} else if let Some(entry) = self.current()
			&& !self.player.is_loaded()
			&& (entry.media_type.is_empty() || self.media_path.borrow().is_some())
		{
			if self.run.get().total_ms > 0 {
				self.resume_run();
			} else {
				self.start_timer(&entry);
			}
		}
	}

	fn open_chat(&self) {
		if let Some(entry) = self.current() {
			*self.outcome.borrow_mut() = Some(ViewerOutcome::OpenChat(entry.poster_jid));
			self.dialog.end_modal(ID_OK);
		}
	}

	fn send_reply(&self) {
		let Some(entry) = self.current() else { return };
		let reply = self.reply_field.get_value();
		if reply.trim().is_empty() {
			crate::platform::announce::announce(self.announcer, crate::i18n::tr("Type a reply first."));
			return;
		}
		send(
			&self.state,
			"send",
			serde_json::json!({
				"to": entry.poster_jid,
				"text": reply,
				"reply_to_id": entry.id,
				"reply_to_sender": entry.poster_jid,
				"mentions": [],
			}),
		);
		self.reply_field.set_value("");
		self.next_btn.set_focus();
		let spoken = crate::i18n::tr_fmt("Reply sent to {}.", &[&entry.poster_name]);
		crate::platform::announce::announce(self.announcer, &spoken);
	}

	fn open_viewer_chat(&self) {
		let Some(sel) = self.viewers_list.get_selection() else { return };
		let viewers = self.viewers_data.borrow();
		if let Some((jid, _name)) = viewers.get(sel as usize) {
			if !jid.is_empty() {
				*self.outcome.borrow_mut() = Some(ViewerOutcome::OpenChat(jid.clone()));
				self.dialog.end_modal(ID_OK);
			}
		}
	}

	fn react(&self, emoji: &str) {
		let Some(entry) = self.current() else { return };
		let emoji = if entry.my_reaction == emoji { "" } else { emoji };
		send(&self.state, "react_status", serde_json::json!({ "id": entry.id, "emoji": emoji }));
		if emoji.is_empty() {
			crate::platform::announce::announce(self.announcer, crate::i18n::tr("Reaction removed."));
		} else {
			let msg = crate::i18n::tr_fmt("Reacted {}.", &[&emoji]);
			crate::platform::announce::announce(self.announcer, &msg);
		}
	}

	fn speak_info(&self) {
		let Some(entry) = self.current() else { return };
		let (position, count) = self.position_in_group(&entry);
		let time = whatsnap::LocalTime::from_unix(entry.timestamp).map(whatsnap::time_or_day).unwrap_or_default();
		let who = if entry.from_me { crate::i18n::tr("My status").to_string() } else { entry.poster_name.clone() };
		let body = match entry.media_type.as_str() {
			"" => entry.text.clone(),
			"audio" | "voice" if entry.text.is_empty() => crate::i18n::tr("Voice note status.").to_string(),
			"audio" | "voice" => format!("{}: {}", crate::i18n::tr("Voice note"), entry.text),
			_ if entry.text.is_empty() => format!("{}.", kind(&entry)),
			_ => format!("{}. {}", kind(&entry), entry.text),
		};
		let mut msg =
			crate::i18n::tr_fmt("{}, status {} of {}, posted {}. {}", &[&who, &position, &count, &time, &body]);
		msg.push_str(&reaction_note(&entry.my_reaction));
		crate::platform::announce::announce(self.announcer, &msg);
	}

	fn show_reaction(&self, mine: &str) {
		let tag = crate::i18n::tr("(your reaction)");
		for (btn, emoji) in self.reaction_btns.iter().zip(STATUS_REACTIONS) {
			let label = if emoji == mine { format!("{emoji} {tag}") } else { emoji.to_string() };
			if btn.get_label() != label {
				btn.set_label(&label);
			}
		}
		let like = if mine == STATUS_LIKE { crate::i18n::tr("Un&like") } else { crate::i18n::tr("&Like") };
		if self.like_btn.get_label() != like {
			self.like_btn.set_label(like);
		}
	}

	/// Likes the update or takes the like back, like WA Web's like button:
	/// a green heart reaction, or an empty one to unlike.
	fn toggle_like(&self) {
		let Some(entry) = self.current() else { return };
		if entry.from_me {
			return;
		}
		let liked = entry.my_reaction != STATUS_LIKE;
		send(&self.state, "like_status", serde_json::json!({ "id": entry.id, "liked": liked }));
		crate::platform::announce::announce(
			self.announcer,
			if liked { crate::i18n::tr("Liked.") } else { crate::i18n::tr("Like removed.") },
		);
	}

	fn toggle_mute(&self) {
		if let Some(entry) = self.current() {
			send(&self.state, "set_status_mute", serde_json::json!({ "jid": entry.poster_jid, "muted": !entry.muted }));
		}
	}

	fn show_mute(&self, entry: &Entry) {
		let first = entry.poster_name.split_whitespace().next().unwrap_or(&entry.poster_name).to_string();
		let label = if entry.muted {
			crate::i18n::tr_fmt("Un&mute {}", &[&first])
		} else {
			crate::i18n::tr_fmt("&Mute {}", &[&first])
		};
		self.mute_btn.set_label(&label);
	}

	fn delete(&self) {
		let Some(entry) = self.current() else { return };
		let was_paused = self.paused.get();
		if !was_paused {
			self.toggle_pause();
		}
		let dialog = MessageDialog::builder(
			&self.dialog,
			crate::i18n::tr("Delete this status update? It will be deleted for everyone who received it."),
			crate::i18n::tr("Delete Status"),
		)
		.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
		.build();
		dialog.set_yes_no_labels(crate::i18n::tr("Delete"), crate::i18n::tr("Cancel"));
		if dialog.show_modal() == ID_YES {
			send(&self.state, "delete_status", serde_json::json!({ "id": entry.id }));
			crate::platform::announce::announce(self.announcer, crate::i18n::tr("Deleting..."));
		}
		if !was_paused {
			self.toggle_pause();
		}
	}

	fn save_as(&self) {
		let Some(path) = self.media_path.borrow().clone() else { return };
		let Some(entry) = self.current() else { return };
		let ext = std::path::Path::new(&path).extension().and_then(|e| e.to_str()).unwrap_or("jpg").to_string();
		let name = format!("Status {} {}.{}", entry.poster_name, entry.id, ext);
		let was_playing = self.player.is_playing();
		self.halt_run();
		self.player.pause();
		crate::ui::video_player::save_copy(&self.dialog, &path, &safe_file_name(&name), self.announcer);
		if was_playing {
			self.player.play();
		} else if !self.player.is_loaded() {
			self.resume_run();
		}
	}

	fn media_ready(&self, path: String) {
		let Some(entry) = self.current() else { return };
		*self.media_path.borrow_mut() = Some(path.clone());
		self.save_btn.enable(true);
		if entry.media_type == "video" || entry.media_type == "audio" || entry.media_type == "voice" {
			self.player.panel.show(true);
			self.dialog.layout();
			self.player.load(&path, !self.paused.get());
			return;
		}
		match crate::media::image_decode::decode_rgba(&path, PHOTO_EDGE) {
			Ok((width, height, rgba)) => {
				if let Some(bitmap) = wxdragon::bitmap::Bitmap::from_rgba(&rgba, width, height) {
					self.photo.set_bitmap(&bitmap);
					self.photo.show(true);
					self.dialog.layout();
				}
			}
			Err(err) => {
				eprintln!("Couldn't decode status photo {path}: {err}");
				crate::platform::announce::announce(self.announcer, crate::i18n::tr("This photo couldn't be shown."));
			}
		}
		self.start_timer(&entry);
	}

	fn media_failed(&self, message: &str) {
		crate::platform::announce::announce(self.announcer, &crate::i18n::translate_backend_message(message));
		if let Some(entry) = self.current() {
			self.start_timer(&entry);
		}
	}

	fn show_viewers(&self, data: &serde_json::Value) {
		let viewers = data.get("viewers").and_then(|v| v.as_array()).cloned().unwrap_or_default();
		let count = viewers.len();
		self.viewers_list.clear();
		let mut items = Vec::new();
		if count == 0 {
			self.viewers_label.set_label(crate::i18n::tr("&Viewed by: no one yet"));
			self.viewers_list.append(crate::i18n::tr("No one has seen this status update yet."));
		} else {
			self.viewers_label.set_label(&crate::i18n::tr_fmt(
				"&Viewed by {} {}",
				&[&count, &crate::i18n::tr(if count == 1 { "person" } else { "people" })],
			));
			for v in &viewers {
				let jid = text(v, "jid");
				let name = text(v, "name");
				items.push((jid, name));
				let time = v
					.get("timestamp")
					.and_then(|t| t.as_i64())
					.and_then(whatsnap::LocalTime::from_unix)
					.map(whatsnap::time_or_day)
					.unwrap_or_default();
				let reaction = text(v, "reaction");
				let mut row = format!("{}, {}", text(v, "name"), time);
				if !reaction.is_empty() {
					row.push_str(&crate::i18n::tr_fmt(", reacted {}", &[&reaction]));
				}
				self.viewers_list.append(&row);
			}
		}
		*self.viewers_data.borrow_mut() = items;
		self.viewers_list.set_selection(0, true);
		self.dialog.layout();
	}

	/// A status update deleted while the viewer is up: by us (Delete), or
	/// revoked by its poster. Like WA Web's revokeMsgs, it drops out of the
	/// list and the viewer moves on to the next one.
	fn remove(&self, id: &str) {
		let current = self.index.get();
		// Take what's needed and let go of `entries` before showing the next
		// update: show_entry marks it viewed, which writes to `entries`, and a
		// borrow still held here (it used to be, inside the show_entry call)
		// panicked and took the app down.
		let left = {
			let mut entries = self.entries.borrow_mut();
			let Some(at) = entries.iter().position(|e| e.id == id) else { return };
			entries.remove(at);
			(at, entries.len())
		};
		let (at, len) = left;
		if len == 0 {
			self.timer.stop();
			self.player.stop();
			self.dialog.end_modal(ID_CANCEL);
			return;
		}
		if at == current {
			crate::platform::announce::announce(self.announcer, crate::i18n::tr("Status update deleted."));
			self.show_entry(current.min(len - 1));
		} else if at < current {
			self.index.set(current - 1);
			self.refresh_bar();
		} else {
			self.refresh_bar();
		}
	}
}

fn safe_file_name(name: &str) -> String {
	name.chars()
		.map(
			|c| {
				if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() {
					'_'
				} else {
					c
				}
			},
		)
		.collect()
}

pub(crate) fn on_event(t: &str, data: &serde_json::Value) -> bool {
	let Some(viewer) = OPEN.with(|open| open.borrow().clone()) else { return false };
	let current = viewer.current();
	let is_current = |key: &str| current.as_ref().is_some_and(|e| e.id == text(data, key));
	match t {
		"media_ready" if is_current("message_id") => viewer.media_ready(text(data, "path")),
		"media_failed" if is_current("message_id") => viewer.media_failed(&text(data, "message")),
		"media_pending" if is_current("message_id") => crate::platform::announce::announce(
			viewer.announcer,
			crate::i18n::tr("Getting a fresh copy from the poster's phone. It will show when it arrives."),
		),
		"status_viewers" if is_current("id") => {
			viewer.show_viewers(data);
			return true;
		}
		"status_viewed" if is_current("id") => {
			send(&viewer.state, "get_status_viewers", serde_json::json!({ "id": text(data, "id") }));
		}
		"status_reacted" => {
			let (id, emoji) = (text(data, "id"), text(data, "emoji"));
			if let Some(e) = viewer.entries.borrow_mut().iter_mut().find(|e| e.id == id) {
				e.my_reaction = emoji.clone();
			}
			if is_current("id") {
				viewer.show_reaction(&emoji);
				let spoken = if data.get("like").and_then(|v| v.as_bool()).unwrap_or(false) {
					if emoji.is_empty() {
						crate::i18n::tr("Like removed.").to_string()
					} else {
						crate::i18n::tr("Liked.").to_string()
					}
				} else if emoji.is_empty() {
					crate::i18n::tr("Reaction removed.").to_string()
				} else {
					crate::i18n::tr_fmt("You reacted {}.", &[&emoji])
				};
				crate::platform::announce::announce(viewer.announcer, &spoken);
			}
		}
		"status_muted" => {
			let (jid, muted) = (text(data, "jid"), data.get("muted").and_then(|v| v.as_bool()).unwrap_or(false));
			for e in viewer.entries.borrow_mut().iter_mut().filter(|e| e.poster_jid == jid) {
				e.muted = muted;
			}
			if let Some(entry) = viewer.current()
				&& entry.poster_jid == jid
			{
				viewer.show_mute(&entry);
				let spoken = crate::i18n::tr_fmt(
					if muted { "{}'s status updates muted." } else { "{}'s status updates unmuted." },
					&[&entry.poster_name],
				);
				crate::platform::announce::announce(viewer.announcer, &spoken);
			}
		}
		"status_deleted" => viewer.remove(&text(data, "id")),
		_ => {}
	}
	false
}
