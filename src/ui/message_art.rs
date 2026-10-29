//! How a message list row looks: a bubble like WhatsApp Web's (yours on the
//! right in green, theirs on the left in white), with the sender in a group,
//! an icon for the message's kind, the time and ticks in the corner, and a
//! picture for photos, videos and stickers. Dates and notices sit in a pill
//! in the middle.
//!
//! Drawing only. The screen reader reads the row's text, which this never
//! changes (see `ui::message_list`).

use std::{cell::RefCell, collections::HashMap};

use wxdragon::{bitmap::Bitmap, prelude::*};

use super::{
	chat_rows::{icon, preview_icon},
	theme::{self, Palette},
	thumbnails::{self, Thumb},
};

/// Rows a message with a picture takes, in rows of plain text.
pub(crate) const TALL_UNITS: u8 = 5;
/// Pictures an album row shows.
const ALBUM_THUMBS: usize = 3;

mod glyph {
	pub const CHECK: char = '\u{E73E}';
	pub const CLOCK: char = '\u{E823}';
	pub const ERROR: char = '\u{E783}';
	pub const STAR: char = '\u{E735}';
	pub const PLAY: char = '\u{E768}';
	pub const BOX: char = '\u{E739}';
	pub const BOX_CHECKED: char = '\u{E73A}';
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Ticks {
	Pending,
	Sent,
	Delivered,
	Read,
	Failed,
}

#[derive(Clone, Debug)]
pub(crate) enum Look {
	/// A date, "unread messages" or a notice ("Mum changed the group name").
	Pill(String),
	Bubble(Bubble),
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Bubble {
	pub from_me: bool,
	/// Who sent it, in a group.
	pub sender: String,
	pub icon: Option<char>,
	pub body: String,
	pub time: String,
	pub ticks: Option<Ticks>,
	pub starred: bool,
	pub edited: bool,
	pub reactions: String,
	/// The messages whose pictures it shows.
	pub thumbs: Vec<String>,
	pub video: bool,
}

impl Look {
	#[cfg(test)]
	pub(crate) fn units(&self) -> u8 {
		match self {
			Look::Bubble(b) if !b.thumbs.is_empty() => TALL_UNITS,
			_ => 1,
		}
	}
}

fn ticks(status: &str) -> Ticks {
	match status {
		"pending" | "uploading" => Ticks::Pending,
		"failed" => Ticks::Failed,
		"delivered" => Ticks::Delivered,
		"read" | "played" => Ticks::Read,
		_ => Ticks::Sent,
	}
}

fn short_time(ts: i64) -> String {
	whatsnap::LocalTime::from_unix(ts).map(|t| t.format("%l:%M %p").trim().to_string()).unwrap_or_default()
}

/// A photo, video or sticker that shows its picture: not view once, and not
/// deleted (its preview may still be stored).
fn has_picture(msg: &whatsnap::ChatMessage) -> bool {
	matches!(msg.media_type.as_str(), "image" | "video" | "sticker")
		&& !msg.text.starts_with("[View once]")
		&& !msg.is_deleted
		&& !msg.text.contains("deleted this message")
		&& msg.text != "This message was deleted"
}

/// How many rows of text tall `item`'s row is: `look_for(item).units()`,
/// without building the look.
pub(crate) fn units_of(item: &whatsnap::DisplayItem) -> u8 {
	if item.messages().iter().any(|m| has_picture(m)) { TALL_UNITS } else { 1 }
}

/// The tall rows among `items`, for `MessageList::remember_units`.
pub(crate) fn tall_rows(items: &[whatsnap::DisplayItem]) -> Vec<(&str, u8)> {
	items.iter().map(|item| (item.id(), units_of(item))).filter(|&(_, units)| units > 1).collect()
}

fn sender_name(msg: &whatsnap::ChatMessage) -> String {
	match msg.sender_name.as_str() {
		"" | "[]" => msg.sender_jid.split('@').next().unwrap_or_default().to_string(),
		name => name.to_string(),
	}
}

/// What a message list row draws for `item`.
pub(crate) fn look_for(item: &whatsnap::DisplayItem, is_group: bool) -> Look {
	match item {
		whatsnap::DisplayItem::Single(msg) => {
			if msg.is_date_divider || msg.is_unread_divider || msg.is_system() {
				return Look::Pill(msg.text.clone());
			}
			let deleted = msg.is_deleted || msg.body_text() == "This message was deleted";
			let (icon, body) = if deleted {
				let text = if msg.from_me { "You deleted this message" } else { "This message was deleted" };
				(Some(icon::DELETED), text.to_string())
			} else if msg.is_voice() {
				// "Voice message Duration: 0:12" shows as the microphone and 0:12.
				let body = msg.body_text();
				let glyph = if msg.text.starts_with("[Audio]") { icon::AUDIO } else { icon::MICROPHONE };
				let duration = body.split_once("Duration: ").map(|(_, d)| d.to_string());
				(Some(glyph), duration.unwrap_or(body))
			} else {
				match preview_icon(&msg.text) {
					(Some(icon), text) => (Some(icon), text),
					(None, _) => (None, msg.body_text()),
				}
			};
			let picture = has_picture(msg);
			Look::Bubble(Bubble {
				from_me: msg.from_me,
				sender: if is_group && !msg.from_me { sender_name(msg) } else { String::new() },
				// The picture says it's a photo; a caption stands alone.
				icon: if picture && msg.media_type != "video" { None } else { icon },
				body,
				time: short_time(msg.timestamp),
				ticks: msg.from_me.then(|| ticks(&msg.status)),
				starred: msg.starred,
				edited: msg.is_edited,
				reactions: msg.reactions.iter().map(|r| r.emoji.as_str()).collect(),
				thumbs: if picture { vec![msg.id.clone()] } else { Vec::new() },
				video: msg.media_type == "video",
			})
		}
		whatsnap::DisplayItem::Album { from_me, timestamp, messages, .. } => {
			let first = messages.first();
			let (icon, body) = preview_icon(&format!("[Image] {}", item.body_text().trim_start_matches("Album: ")));
			Look::Bubble(Bubble {
				from_me: *from_me,
				sender: match first {
					Some(m) if is_group && !from_me => sender_name(m),
					_ => String::new(),
				},
				icon,
				body,
				time: short_time(*timestamp),
				ticks: first.filter(|_| *from_me).map(|m| ticks(&m.status)),
				thumbs: messages.iter().filter(|m| has_picture(m)).take(ALBUM_THUMBS).map(|m| m.id.clone()).collect(),
				video: messages.iter().all(|m| m.media_type == "video"),
				..Default::default()
			})
		}
	}
}

#[derive(Clone)]
struct Fonts {
	base: (i32, String),
	body: Font,
	bold: Font,
	small: Font,
	icons: Option<Font>,
	icons_small: Option<Font>,
	icons_big: Option<Font>,
}

/// Corner pieces that round off a square box: the background colour, see-
/// through where the box's rounded corner is. Keyed by (background, radius).
type Corners = [Bitmap; 4];

#[derive(Default)]
struct Cache {
	fonts: Option<std::rc::Rc<Fonts>>,
	corners: HashMap<(u32, i32), Corners>,
	play: Option<(i32, Bitmap)>,
}

thread_local! {
	static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

fn icon_face() -> Option<&'static str> {
	use std::sync::OnceLock;
	static FACE: OnceLock<Option<&'static str>> = OnceLock::new();
	*FACE
		.get_or_init(|| ["Segoe Fluent Icons", "Segoe MDL2 Assets"].into_iter().find(|f| super::gdi::face_installed(f)))
}

fn make_fonts(base: &Font) -> Fonts {
	let size = base.get_point_size().max(6);
	let face = base.get_face_name();
	let with = |points: i32, weight: FontWeight| {
		Font::builder().with_point_size(points.max(5)).with_weight(weight).with_face_name(&face).build()
	};
	let icon = |points: i32| {
		icon_face().and_then(|f| Font::builder().with_point_size(points.max(5)).with_face_name(f).build())
	};
	Fonts {
		base: (size, face.clone()),
		body: base.clone(),
		bold: with(size, FontWeight::SemiBold).unwrap_or_else(|| base.clone()),
		small: with(size - 2, FontWeight::Normal).unwrap_or_else(|| base.clone()),
		icons: icon(size),
		icons_small: icon(size - 2),
		icons_big: icon(size * 2),
	}
}

fn key(c: Colour) -> u32 {
	(c.r as u32) << 16 | (c.g as u32) << 8 | c.b as u32
}

fn corners(background: Colour, radius: i32) -> Corners {
	let side = radius * 2;
	let mut pieces = Vec::with_capacity(4);
	for (cx, cy) in [(0, 0), (radius, 0), (0, radius), (radius, radius)] {
		let mut rgba = vec![0u8; (radius * radius * 4) as usize];
		for y in 0..radius {
			for x in 0..radius {
				let coverage = super::gdi::round_rect_coverage(cx + x, cy + y, side, side, radius as f32);
				let i = ((y * radius + x) * 4) as usize;
				rgba[i] = background.r;
				rgba[i + 1] = background.g;
				rgba[i + 2] = background.b;
				rgba[i + 3] = ((1.0 - coverage) * 255.0) as u8;
			}
		}
		pieces.push(Bitmap::from_rgba(&rgba, radius as u32, radius as u32).expect("corner bitmap"));
	}
	pieces.try_into().unwrap_or_else(|_| unreachable!())
}

/// A filled box with rounded, smoothed corners.
#[allow(clippy::too_many_arguments)]
fn round_box(dc: &impl DeviceContext, cache: &mut Cache, x: i32, y: i32, w: i32, h: i32, fill: Colour, bg: Colour) {
	dc.set_brush(fill, BrushStyle::Solid);
	dc.set_pen(fill, 1, PenStyle::Solid);
	dc.draw_rectangle(x, y, w, h);
	let radius = (h / 2).min(w / 2).clamp(1, 8);
	let pieces = cache.corners.entry((key(bg), radius)).or_insert_with(|| corners(bg, radius));
	dc.draw_bitmap(&pieces[0], x, y, true);
	dc.draw_bitmap(&pieces[1], x + w - radius, y, true);
	dc.draw_bitmap(&pieces[2], x, y + h - radius, true);
	dc.draw_bitmap(&pieces[3], x + w - radius, y + h - radius, true);
}

/// `text` cut with "…" to fit in `max` pixels.
fn fit(dc: &impl DeviceContext, text: &str, max: i32) -> String {
	let flat: String = text.chars().take(400).map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();
	if max <= 0 {
		return String::new();
	}
	if dc.get_text_extent(&flat).0 <= max {
		return flat;
	}
	let chars: Vec<char> = flat.chars().collect();
	let (mut lo, mut hi) = (0, chars.len());
	while lo < hi {
		let mid = (lo + hi).div_ceil(2);
		let candidate: String = chars[..mid].iter().collect::<String>() + "…";
		if dc.get_text_extent(&candidate).0 <= max { lo = mid } else { hi = mid - 1 }
	}
	if lo == 0 { String::new() } else { chars[..lo].iter().collect::<String>() + "…" }
}

struct Colours {
	background: Colour,
	band: Option<Colour>,
	focus_bar: Option<Colour>,
	text: Colour,
	muted: Colour,
	accent: Colour,
	read: Colour,
	mine: Colour,
	theirs: Colour,
	pill: Colour,
	outline: bool,
}

fn colours(background: Colour, foreground: Colour, selected: bool, focused: bool) -> Colours {
	if !theme::use_custom_colours() {
		let sys = SystemSettings::get_colour;
		let (bg, text) = if selected {
			(sys(SystemColour::Highlight), sys(SystemColour::HighlightText))
		} else {
			(background, foreground)
		};
		return Colours {
			background,
			band: selected.then_some(bg),
			focus_bar: None,
			text,
			muted: text,
			accent: text,
			read: text,
			mine: bg,
			theirs: bg,
			pill: bg,
			outline: true,
		};
	}
	let band = selected.then(|| if focused { Colour::rgb(0xCF, 0xE3, 0xD7) } else { Colour::rgb(0xE2, 0xDE, 0xD6) });
	Colours {
		background,
		band,
		focus_bar: (selected && focused).then_some(Palette::ACCENT),
		text: Palette::TEXT_PRIMARY,
		muted: Palette::TEXT_MUTED,
		accent: Palette::ACCENT,
		// Read ticks: WhatsApp's blue, darkened to show on the green bubble.
		read: Colour::rgb(0x0B, 0x6F, 0xA4),
		mine: Colour::rgb(0xD9, 0xFD, 0xD3),
		theirs: Colour::rgb(0xFF, 0xFF, 0xFF),
		pill: Colour::rgb(0xFF, 0xFF, 0xFF),
		outline: false,
	}
}

/// One row of the list, `units` rows of `unit` pixels tall, at `y`.
pub(crate) struct RowPaint<'a> {
	pub y: i32,
	pub width: i32,
	pub unit: i32,
	pub units: u8,
	pub look: Option<&'a Look>,
	pub text: &'a str,
	pub selected: bool,
	pub focused: bool,
	/// In a checkable list: whether it's checked.
	pub check: Option<bool>,
}

pub(crate) fn paint_row(
	dc: &impl DeviceContext,
	base_font: &Font,
	background: Colour,
	foreground: Colour,
	row: RowPaint,
) {
	CACHE.with(|cache| {
		let mut cache = cache.borrow_mut();
		let base = (base_font.get_point_size(), base_font.get_face_name());
		if cache.fonts.as_ref().is_none_or(|f| f.base != base) {
			cache.fonts = Some(std::rc::Rc::new(make_fonts(base_font)));
			cache.corners.clear();
		}
		let fonts = cache.fonts.clone().expect("fonts were just made");
		let fonts = &*fonts;
		let c = colours(background, foreground, row.selected, row.focused);
		let height = row.unit * row.units as i32;
		let row_bg = c.band.unwrap_or(c.background);
		if let Some(band) = c.band {
			dc.set_brush(band, BrushStyle::Solid);
			dc.set_pen(band, 1, PenStyle::Solid);
			dc.draw_rectangle(0, row.y, row.width, height);
		}
		if let Some(bar) = c.focus_bar {
			dc.set_brush(bar, BrushStyle::Solid);
			dc.set_pen(bar, 1, PenStyle::Solid);
			dc.draw_rectangle(0, row.y, 4, height);
		}
		let mut left = 12;
		if let Some(checked) = row.check {
			match &fonts.icons {
				Some(font) => {
					dc.set_font(font);
					dc.set_text_foreground(c.text);
					let glyph = if checked { glyph::BOX_CHECKED } else { glyph::BOX };
					let (gw, gh) = dc.get_text_extent(&glyph.to_string());
					dc.draw_text(&glyph.to_string(), left, row.y + (height - gh) / 2);
					left += gw + 8;
				}
				None => {
					dc.set_font(&fonts.body);
					let mark = if checked { "[x]" } else { "[ ]" };
					dc.set_text_foreground(c.text);
					let (gw, gh) = dc.get_text_extent(mark);
					dc.draw_text(mark, left, row.y + (height - gh) / 2);
					left += gw + 8;
				}
			}
		}
		match row.look {
			Some(Look::Pill(text)) => pill(dc, &mut cache, fonts, &row, &c, row_bg, text),
			Some(Look::Bubble(bubble)) => draw_bubble(dc, &mut cache, fonts, &row, &c, row_bg, left, bubble),
			None => {
				dc.set_font(&fonts.body);
				dc.set_text_foreground(c.text);
				let line = fit(dc, row.text, row.width - left - 12);
				let th = dc.get_text_extent("Ag").1;
				dc.draw_text(&line, left, row.y + (height - th) / 2);
			}
		}
	});
}

fn pill(
	dc: &impl DeviceContext,
	cache: &mut Cache,
	fonts: &Fonts,
	row: &RowPaint,
	c: &Colours,
	row_bg: Colour,
	text: &str,
) {
	dc.set_font(&fonts.small);
	let text = fit(dc, text, row.width - 64);
	let (tw, th) = dc.get_text_extent(&text);
	let (w, h) = (tw + 24, (th + 8).min(row.unit - 4));
	let (x, y) = ((row.width - w) / 2, row.y + (row.unit - h) / 2);
	if c.outline {
		dc.set_brush(c.pill, BrushStyle::Transparent);
		dc.set_pen(c.text, 1, PenStyle::Solid);
		dc.draw_rounded_rectangle(x, y, w, h, 4.0);
	} else {
		round_box(dc, cache, x, y, w, h, c.pill, row_bg);
	}
	dc.set_font(&fonts.small);
	dc.set_text_foreground(c.muted);
	dc.draw_text(&text, x + 12, y + (h - th) / 2);
}

#[allow(clippy::too_many_arguments)]
fn draw_bubble(
	dc: &impl DeviceContext,
	cache: &mut Cache,
	fonts: &Fonts,
	row: &RowPaint,
	c: &Colours,
	row_bg: Colour,
	left: i32,
	b: &Bubble,
) {
	let pad = 8;
	let gap = 6;
	let height = row.unit * row.units as i32 - 4;
	let top = row.y + 2;
	let max_w = ((row.width - left - 12) * 13 / 20).max(160).min(row.width - left - 12);

	// The corner: reactions, star, "Edited", time, ticks.
	dc.set_font(&fonts.small);
	let mut meta = String::new();
	if !b.reactions.is_empty() {
		meta.push_str(&b.reactions);
		meta.push(' ');
	}
	if b.edited {
		meta.push_str("Edited ");
	}
	meta.push_str(&b.time);
	let meta_w = dc.get_text_extent(&meta).0;
	let star = b.starred.then(|| glyph::STAR.to_string());
	let ticks: Option<(String, Colour)> = b.ticks.map(|t| {
		let two = format!("{0}{0}", glyph::CHECK);
		match t {
			Ticks::Pending => (glyph::CLOCK.to_string(), c.muted),
			Ticks::Failed => (glyph::ERROR.to_string(), c.muted),
			Ticks::Sent => (glyph::CHECK.to_string(), c.muted),
			Ticks::Delivered => (two, c.muted),
			Ticks::Read => (two, c.read),
		}
	});
	// Two ticks overlap, as WhatsApp draws them.
	let icon_width = |s: &str| -> i32 {
		let Some(font) = &fonts.icons_small else { return 0 };
		dc.set_font(font);
		let w = dc.get_text_extent(s).0;
		(if s.chars().count() == 2 { w * 2 / 3 } else { w }) + 3
	};
	let star_w = star.as_deref().map_or(0, icon_width);
	let ticks_w = ticks.as_ref().map_or(0, |(s, _)| icon_width(s));
	let corner_w = star_w + meta_w + ticks_w;

	// The pictures, in a tall row.
	let thumb_h = if b.thumbs.is_empty() { 0 } else { height - 8 };
	let thumb_box_w = if b.thumbs.len() > 1 { thumb_h } else { thumb_h * 4 / 3 };

	// The text: sender (in a group), the kind's icon, the message.
	dc.set_font(&fonts.bold);
	let sender_w = if b.sender.is_empty() { 0 } else { dc.get_text_extent(&b.sender).0 };
	let glyph_w = match (b.icon, &fonts.icons) {
		(Some(g), Some(font)) => {
			dc.set_font(font);
			dc.get_text_extent(&g.to_string()).0 + 4
		}
		_ => 0,
	};
	dc.set_font(&fonts.body);
	let body_full_w = dc.get_text_extent(&b.body).0;

	let (bubble_w, text_left_in) = if b.thumbs.is_empty() {
		let sender_part = if sender_w > 0 { sender_w + gap } else { 0 };
		let want = pad + sender_part + glyph_w + body_full_w + gap + corner_w + pad;
		(want.min(max_w), pad)
	} else {
		let thumbs_w = b.thumbs.len() as i32 * (thumb_box_w + 4) - 4;
		let text_w = sender_w.max(glyph_w + body_full_w).max(corner_w);
		let want = 4 + thumbs_w + pad + text_w + pad;
		(want.min(max_w).max(4 + thumbs_w + pad + corner_w + pad).min(row.width - left - 12), 4 + thumbs_w + pad)
	};
	let x = if b.from_me { row.width - 12 - bubble_w } else { left };
	let fill = if b.from_me { c.mine } else { c.theirs };
	if c.outline {
		dc.set_brush(fill, BrushStyle::Transparent);
		dc.set_pen(c.text, 1, PenStyle::Solid);
		dc.draw_rounded_rectangle(x, top, bubble_w, height, 6.0);
	} else {
		round_box(dc, cache, x, top, bubble_w, height, fill, row_bg);
	}

	// Pictures, or a grey box until they come.
	let mut tx = x + 4;
	for id in &b.thumbs {
		let ty = top + 4;
		let drawn = thumbnails::with_thumb(id, thumb_box_w, thumb_h, |thumb| match thumb {
			Thumb::Picture(bitmap) => {
				let (bw, bh) = (bitmap.get_width(), bitmap.get_height());
				dc.draw_bitmap(bitmap, tx + (thumb_box_w - bw) / 2, ty + (thumb_h - bh) / 2, true);
				true
			}
			Thumb::Nothing | Thumb::Waiting => false,
		});
		if !drawn {
			let grey = if c.outline { fill } else { Colour::rgb(0xD5, 0xD9, 0xDB) };
			round_box(dc, cache, tx, ty, thumb_box_w, thumb_h, grey, fill);
			if let Some(font) = &fonts.icons_big {
				dc.set_font(font);
				dc.set_text_foreground(Colour::rgb(0xFF, 0xFF, 0xFF));
				let g = if b.video { icon::VIDEO } else { icon::CAMERA }.to_string();
				let (gw, gh) = dc.get_text_extent(&g);
				dc.draw_text(&g, tx + (thumb_box_w - gw) / 2, ty + (thumb_h - gh) / 2);
			}
		} else if b.video {
			play_button(dc, cache, fonts, tx + thumb_box_w / 2, ty + thumb_h / 2);
		}
		tx += thumb_box_w + 4;
	}

	let text_x = x + text_left_in;
	let text_right = x + bubble_w - pad;
	let line_h = row.unit - 4;
	// Plain rows: everything on one line, the corner on the right. Picture
	// rows: sender and message stacked beside the picture, corner at the bottom.
	let (line_y, corner_y) = if b.thumbs.is_empty() {
		(top, top)
	} else {
		let lines = if b.sender.is_empty() { 1 } else { 2 };
		(top + (height - lines * line_h - line_h) / 2, top + height - line_h - 2)
	};
	// Where text in the current font sits to be centred on a line at `y`.
	let centre = |y: i32, text: &str| y + (line_h - dc.get_text_extent(text).1) / 2;

	let mut cx = text_x;
	let mut cy = line_y;
	let text_room = if b.thumbs.is_empty() { text_right - corner_w - gap } else { text_right };
	if !b.sender.is_empty() {
		dc.set_font(&fonts.bold);
		dc.set_text_foreground(c.accent);
		let s = fit(dc, &b.sender, text_room - cx);
		dc.draw_text(&s, cx, centre(cy, &s));
		if b.thumbs.is_empty() {
			cx += dc.get_text_extent(&s).0 + gap;
		} else {
			cy += line_h;
		}
	}
	if let (Some(g), Some(font)) = (b.icon, &fonts.icons)
		&& cx < text_room
	{
		dc.set_font(font);
		dc.set_text_foreground(c.muted);
		let g = g.to_string();
		dc.draw_text(&g, cx, centre(cy, &g));
		cx += dc.get_text_extent(&g).0 + 4;
	}
	dc.set_font(&fonts.body);
	dc.set_text_foreground(c.text);
	let body = fit(dc, &b.body, text_room - cx);
	dc.draw_text(&body, cx, centre(cy, &body));

	// The corner, right-aligned.
	let mut mx = text_right - corner_w;
	if let (Some(s), Some(font)) = (&star, &fonts.icons_small) {
		dc.set_font(font);
		dc.set_text_foreground(c.muted);
		dc.draw_text(s, mx, centre(corner_y, s));
		mx += star_w;
	}
	dc.set_font(&fonts.small);
	dc.set_text_foreground(c.muted);
	dc.draw_text(&meta, mx, centre(corner_y, &meta));
	mx += meta_w + 3;
	if let (Some((s, colour)), Some(font)) = (&ticks, &fonts.icons_small) {
		dc.set_font(font);
		dc.set_text_foreground(*colour);
		let mut chars = s.chars();
		if let Some(first) = chars.next() {
			let one = first.to_string();
			let y = centre(corner_y, &one);
			dc.draw_text(&one, mx, y);
			if let Some(second) = chars.next() {
				dc.draw_text(&second.to_string(), mx + dc.get_text_extent(&one).0 / 3, y);
			}
		}
	}
}

/// A dark round button with a white triangle, over a video's picture.
fn play_button(dc: &impl DeviceContext, cache: &mut Cache, fonts: &Fonts, cx: i32, cy: i32) {
	let Some(font) = &fonts.icons else { return };
	dc.set_font(font);
	let g = glyph::PLAY.to_string();
	let (gw, gh) = dc.get_text_extent(&g);
	let size = gw.max(gh) * 2;
	if cache.play.as_ref().is_none_or(|(s, _)| *s != size) {
		let rgba = super::gdi::round_rect_rgba(size, size, size as f32 / 2.0, (0, 0, 0), 140);
		cache.play = Bitmap::from_rgba(&rgba, size as u32, size as u32).map(|b| (size, b));
	}
	if let Some((_, circle)) = &cache.play {
		dc.draw_bitmap(circle, cx - size / 2, cy - size / 2, true);
	}
	dc.set_text_foreground(Colour::rgb(0xFF, 0xFF, 0xFF));
	dc.draw_text(&g, cx - gw / 2 + 1, cy - gh / 2);
}

#[cfg(test)]
mod tests {
	use super::{Look, TALL_UNITS, Ticks, look_for, ticks};

	fn msg(text: &str, media: &str, from_me: bool) -> whatsnap::DisplayItem {
		let m = whatsnap::ChatMessage {
			id: "A1".into(),
			text: text.into(),
			media_type: media.into(),
			from_me,
			sender_name: "Bob".into(),
			..Default::default()
		};
		whatsnap::DisplayItem::Single(std::rc::Rc::new(m))
	}

	#[test]
	fn a_photo_is_a_tall_bubble_with_its_picture() {
		let look = look_for(&msg("[Image] beach", "image", false), true);
		assert_eq!(look.units(), TALL_UNITS);
		let Look::Bubble(b) = look else { panic!() };
		assert_eq!((b.sender.as_str(), b.body.as_str(), b.thumbs.len()), ("Bob", "beach", 1));
	}

	#[test]
	fn a_view_once_photo_shows_no_picture() {
		assert_eq!(look_for(&msg("[View once] [Image]", "image", false), false).units(), 1);
	}

	#[test]
	fn a_voice_message_shows_its_length() {
		let Look::Bubble(b) = look_for(&msg("[Voice Message] (0:12)", "audio", true), false) else { panic!() };
		assert_eq!(b.body, "0:12");
		assert_eq!(b.icon, Some(super::icon::MICROPHONE));
		assert!(b.sender.is_empty());
	}

	#[test]
	fn read_and_played_both_show_blue_ticks() {
		assert_eq!(ticks("read"), Ticks::Read);
		assert_eq!(ticks("played"), Ticks::Read);
		assert_eq!(ticks(""), Ticks::Sent);
	}
}
