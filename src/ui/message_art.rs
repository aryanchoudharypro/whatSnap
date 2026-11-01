use std::{cell::RefCell, collections::HashMap, rc::Rc};

use wxdragon::{bitmap::Bitmap, prelude::*};

use super::{
	chat_rows::{icon, preview_icon},
	theme::{self, Palette},
	thumbnails::{self, Thumb},
};

const ALBUM_THUMBS: usize = 3;
/// Lines of a message drawn at most; the rest ends in "…". The screen reader
/// still gets all of it.
const MAX_LINES: usize = 40;
/// A row is never more than this many rows of text tall.
const MAX_UNITS: usize = 60;

mod glyph {
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
	Pill(String),
	Bubble(Bubble),
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Bubble {
	pub from_me: bool,
	pub sender: String,
	pub sender_colour: usize,
	pub quote: Option<(String, String)>,
	pub icon: Option<char>,
	pub body: String,
	pub time: String,
	pub ticks: Option<Ticks>,
	pub starred: bool,
	pub edited: bool,
	pub reactions: String,
	pub thumbs: Vec<String>,
	pub video: bool,
	pub voice: Option<String>,
	pub audio_file: bool,
}

pub(crate) fn tick_glyphs(ticks: Ticks) -> (&'static str, bool) {
	match ticks {
		Ticks::Pending => ("\u{E823}", false),
		Ticks::Failed => ("\u{E783}", false),
		Ticks::Sent => ("\u{E73E}", false),
		Ticks::Delivered => ("\u{E73E}\u{E73E}", false),
		Ticks::Read => ("\u{E73E}\u{E73E}", true),
	}
}

pub(crate) fn ticks_for(status: &str) -> Ticks {
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

fn has_picture(msg: &whatsnap::ChatMessage) -> bool {
	matches!(msg.media_type.as_str(), "image" | "video" | "sticker")
		&& !msg.text.starts_with("[View once]")
		&& !msg.is_deleted
		&& !msg.text.contains("deleted this message")
		&& msg.text != "This message was deleted"
}

fn sender_name(msg: &whatsnap::ChatMessage) -> String {
	match msg.sender_name.as_str() {
		"" | "[]" => msg.sender_jid.split('@').next().unwrap_or_default().to_string(),
		name => name.to_string(),
	}
}

const NAME_COLOURS: [(u8, u8, u8); 10] = [
	(0xB0, 0x2E, 0x26),
	(0x7B, 0x3F, 0xA0),
	(0x17, 0x65, 0xC8),
	(0x0B, 0x75, 0x3D),
	(0x9C, 0x50, 0x00),
	(0xA3, 0x14, 0x52),
	(0x00, 0x70, 0x7C),
	(0x55, 0x45, 0xB0),
	(0x6E, 0x5C, 0x00),
	(0x6D, 0x4C, 0x41),
];

const NAME_COLOURS_DARK: [(u8, u8, u8); 10] = [
	(0xFF, 0x8A, 0x80),
	(0xC7, 0x9B, 0xE8),
	(0x6C, 0xB4, 0xFF),
	(0x5F, 0xD3, 0x8D),
	(0xFF, 0xB3, 0x4D),
	(0xFF, 0x7E, 0xB3),
	(0x4F, 0xD1, 0xD9),
	(0xA9, 0x9C, 0xFF),
	(0xE0, 0xC8, 0x4A),
	(0xD0, 0xA8, 0x96),
];

fn name_colour(jid: &str) -> usize {
	let hash = jid.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193));
	hash as usize % NAME_COLOURS.len()
}

fn quote_of(msg: &whatsnap::ChatMessage) -> Option<(String, String)> {
	let (sender, body) = msg.reply_parts()?;
	let sender = match sender.as_str() {
		"quoted message" => String::new(),
		s => s.trim_start_matches('@').to_string(),
	};
	let body = match preview_icon(&msg.quoted_text) {
		(Some(_), text) if !msg.quoted_text.is_empty() => text,
		_ => body,
	};
	Some((sender, body))
}

pub(crate) fn look_for(item: &whatsnap::DisplayItem, is_group: bool) -> Look {
	match item {
		whatsnap::DisplayItem::Single(msg) => {
			if msg.is_date_divider || msg.is_unread_divider || msg.is_system() {
				return Look::Pill(msg.text.clone());
			}
			let deleted = msg.is_deleted || msg.body_text() == "This message was deleted";
			let voice = !deleted && msg.is_voice();
			let (icon, body) = if deleted {
				let text = if msg.from_me { "You deleted this message" } else { "This message was deleted" };
				(Some(icon::DELETED), text.to_string())
			} else if voice {
				let body = msg.body_text();
				let glyph = if msg.text.starts_with("[Audio]") { icon::AUDIO } else { icon::MICROPHONE };
				let duration = body.split_once("Duration: ").map(|(_, d)| d.to_string());
				(Some(glyph), duration.unwrap_or_default())
			} else {
				match preview_icon(&msg.text) {
					(Some(icon), text) => (Some(icon), text),
					(None, _) => (None, msg.body_text()),
				}
			};
			let picture = has_picture(msg);
			let mine = msg.from_me;
			Look::Bubble(Bubble {
				from_me: mine,
				sender: if is_group && !mine { sender_name(msg) } else { String::new() },
				sender_colour: name_colour(&msg.sender_jid),
				quote: if deleted { None } else { quote_of(msg) },
				icon: if picture && msg.media_type != "video" { None } else { icon },
				body: if picture && matches!(body.as_str(), "Photo" | "Video" | "Sticker") {
					String::new()
				} else {
					body
				},
				time: short_time(msg.timestamp),
				ticks: mine.then(|| ticks_for(&msg.status)),
				starred: msg.starred,
				edited: msg.is_edited,
				reactions: reaction_summary(msg),
				thumbs: if picture { vec![msg.id.clone()] } else { Vec::new() },
				video: msg.media_type == "video",
				voice: voice.then(|| msg.id.clone()),
				audio_file: voice && msg.text.starts_with("[Audio]"),
			})
		}
		whatsnap::DisplayItem::Album { from_me, timestamp, messages, .. } => {
			let first = messages.first();
			Look::Bubble(Bubble {
				from_me: *from_me,
				sender: match first {
					Some(m) if is_group && !from_me => sender_name(m),
					_ => String::new(),
				},
				sender_colour: first.map_or(0, |m| name_colour(&m.sender_jid)),
				body: item.body_text().trim_start_matches("Album: ").to_string(),
				time: short_time(*timestamp),
				ticks: first.filter(|_| *from_me).map(|m| ticks_for(&m.status)),
				thumbs: messages.iter().filter(|m| has_picture(m)).take(ALBUM_THUMBS).map(|m| m.id.clone()).collect(),
				video: messages.iter().all(|m| m.media_type == "video"),
				..Default::default()
			})
		}
	}
}

fn reaction_summary(msg: &whatsnap::ChatMessage) -> String {
	let active: Vec<&str> = msg.reactions.iter().map(|r| r.emoji.as_str()).filter(|e| !e.is_empty()).collect();
	let mut shown: Vec<&str> = Vec::new();
	for emoji in &active {
		if !shown.contains(emoji) && shown.len() < 3 {
			shown.push(emoji);
		}
	}
	match active.len() {
		0 => String::new(),
		1 => shown.concat(),
		n => format!("{} {}", shown.concat(), n),
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Face {
	Body,
	Bold,
	Small,
}

struct Fonts {
	base: (i32, String),
	body: Font,
	bold: Font,
	small: Font,
	icons: Option<Font>,
	icons_small: Option<Font>,
	icons_big: Option<Font>,
	line: i32,
	widths: RefCell<HashMap<(Face, char), i32>>,
}

impl Fonts {
	fn font(&self, face: Face) -> &Font {
		match face {
			Face::Body => &self.body,
			Face::Bold => &self.bold,
			Face::Small => &self.small,
		}
	}

	fn char_width(&self, dc: &impl DeviceContext, face: Face, c: char) -> i32 {
		if let Some(&w) = self.widths.borrow().get(&(face, c)) {
			return w;
		}
		dc.set_font(self.font(face));
		let w = dc.get_text_extent(c.encode_utf8(&mut [0; 4])).0;
		self.widths.borrow_mut().insert((face, c), w);
		w
	}

	fn width(&self, dc: &impl DeviceContext, face: Face, text: &str) -> i32 {
		text.chars().map(|c| self.char_width(dc, face, c)).sum()
	}

	fn fit(&self, dc: &impl DeviceContext, face: Face, text: &str, max: i32) -> String {
		let flat = text.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c });
		let ellipsis = self.char_width(dc, face, '…');
		let mut out = String::new();
		let mut used = 0;
		let total = self.width(dc, face, text);
		if total <= max {
			return flat.collect();
		}
		for c in flat {
			let w = self.char_width(dc, face, c);
			if used + w + ellipsis > max {
				break;
			}
			used += w;
			out.push(c);
		}
		if out.is_empty() { out } else { out + "…" }
	}

	fn wrap(&self, dc: &impl DeviceContext, face: Face, text: &str, first_max: i32, max: i32) -> Vec<String> {
		let mut lines: Vec<String> = Vec::new();
		for paragraph in text.split('\n') {
			let paragraph = paragraph.trim_end_matches('\r');
			let mut line = String::new();
			let mut line_w = 0;
			let room = |lines: &Vec<String>| if lines.is_empty() { first_max } else { max };
			for word in paragraph.split_inclusive(' ') {
				let word_w = self.width(dc, face, word);
				let trimmed_w = word_w - if word.ends_with(' ') { self.char_width(dc, face, ' ') } else { 0 };
				if line_w + trimmed_w <= room(&lines) {
					line.push_str(word);
					line_w += word_w;
					continue;
				}
				if !line.is_empty() {
					lines.push(std::mem::take(&mut line).trim_end().to_string());
					line_w = 0;
				}
				if trimmed_w <= room(&lines) {
					line.push_str(word);
					line_w = word_w;
					continue;
				}
				// A word longer than a line breaks where it must.
				for c in word.chars() {
					let w = self.char_width(dc, face, c);
					if line_w + w > room(&lines) && !line.is_empty() {
						lines.push(std::mem::take(&mut line));
						line_w = 0;
					}
					line.push(c);
					line_w += w;
				}
			}
			lines.push(line.trim_end().to_string());
			if lines.len() > MAX_LINES {
				break;
			}
		}
		if lines.len() > MAX_LINES {
			lines.truncate(MAX_LINES);
			if let Some(last) = lines.last_mut() {
				let cut = self.fit(dc, face, &(last.clone() + " …………"), max);
				*last = cut;
			}
		}
		lines
	}
}

pub(crate) fn icon_face() -> Option<&'static str> {
	use std::sync::OnceLock;
	static FACE: OnceLock<Option<&'static str>> = OnceLock::new();
	*FACE
		.get_or_init(|| ["Segoe Fluent Icons", "Segoe MDL2 Assets"].into_iter().find(|f| super::gdi::face_installed(f)))
}

fn make_fonts(dc: &impl DeviceContext, base: &Font) -> Fonts {
	let size = base.get_point_size().max(6);
	let face = base.get_face_name();
	let with = |points: i32, weight: FontWeight| {
		Font::builder().with_point_size(points.max(5)).with_weight(weight).with_face_name(&face).build()
	};
	let icon = |points: i32| {
		icon_face().and_then(|f| Font::builder().with_point_size(points.max(5)).with_face_name(f).build())
	};
	dc.set_font(base);
	Fonts {
		base: (size, face.clone()),
		body: base.clone(),
		bold: with(size, FontWeight::SemiBold).unwrap_or_else(|| base.clone()),
		small: with(size - 2, FontWeight::Normal).unwrap_or_else(|| base.clone()),
		icons: icon(size),
		icons_small: icon(size - 2),
		icons_big: icon(size * 2),
		line: dc.get_text_extent("Ag").1.max(8),
		widths: RefCell::new(HashMap::new()),
	}
}

type Corners = [Bitmap; 4];

#[derive(Default)]
struct Cache {
	fonts: Option<Rc<Fonts>>,
	corners: HashMap<(u32, i32), Corners>,
	badges: Vec<(i32, u32, Bitmap)>,
}

thread_local! {
	static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

fn fonts_for(dc: &impl DeviceContext, base_font: &Font) -> Rc<Fonts> {
	CACHE.with(|cache| {
		let mut cache = cache.borrow_mut();
		let base = (base_font.get_point_size(), base_font.get_face_name());
		if cache.fonts.as_ref().is_none_or(|f| f.base != base) {
			cache.fonts = Some(Rc::new(make_fonts(dc, base_font)));
			cache.corners.clear();
		}
		cache.fonts.clone().expect("fonts were just made")
	})
}

const PAD: i32 = 8;
const GAP: i32 = 6;
const MARGIN: i32 = 12;

struct Layout {
	width: i32,
	height: i32,
	sender: String,
	quote: Option<(String, String)>,
	quote_h: i32,
	thumb_w: i32,
	thumb_h: i32,
	voice_h: i32,
	lines: Vec<String>,
	icon_w: i32,
	meta: String,
	meta_w: i32,
	star_w: i32,
	ticks_w: i32,
	meta_inline: bool,
	reactions_h: i32,
}

fn icon_width(dc: &impl DeviceContext, font: &Option<Font>, glyphs: &str) -> i32 {
	let Some(font) = font else { return 0 };
	dc.set_font(font);
	let w = dc.get_text_extent(glyphs).0;
	if glyphs.chars().count() == 2 { w * 2 / 3 } else { w }
}

fn layout(dc: &impl DeviceContext, fonts: &Fonts, b: &Bubble, room: i32) -> Layout {
	let line = fonts.line;
	let max_w = (room * 13 / 20).max(200.min(room)).min(room);
	let content_max = (max_w - 2 * PAD).max(40);

	let mut meta = String::new();
	if b.edited {
		meta.push_str("Edited ");
	}
	meta.push_str(&b.time);
	let meta_w = fonts.width(dc, Face::Small, &meta);
	let star_w = if b.starred { icon_width(dc, &fonts.icons_small, &glyph::STAR.to_string()) + 3 } else { 0 };
	let ticks_w = b.ticks.map_or(0, |t| icon_width(dc, &fonts.icons_small, tick_glyphs(t).0) + 3);
	let corner_w = star_w + meta_w + ticks_w;

	let (thumb_w, thumb_h) = match b.thumbs.len() {
		0 => (0, 0),
		1 => (content_max.min(line * 14), line * 10),
		n => {
			let side = ((content_max.min(line * 18) - (n as i32 - 1) * 4) / n as i32).max(line * 3);
			(side, side)
		}
	};
	let thumbs_w = if b.thumbs.is_empty() { 0 } else { b.thumbs.len() as i32 * (thumb_w + 4) - 4 };
	let voice_h = if b.voice.is_some() { line * 2 } else { 0 };
	let voice_w = if b.voice.is_some() { content_max.min(line * 14) } else { 0 };

	let text_max = if thumbs_w > 0 { thumbs_w.max(line * 6) } else { content_max };
	let icon_w = match (b.icon, &fonts.icons, b.voice.is_some()) {
		(Some(g), Some(_), false) => icon_width(dc, &fonts.icons, &g.to_string()) + 4,
		_ => 0,
	};
	let lines = if b.voice.is_some() || b.body.is_empty() {
		Vec::new()
	} else {
		fonts.wrap(dc, Face::Body, &b.body, text_max - icon_w, text_max)
	};
	let line_widths: Vec<i32> = lines.iter().map(|l| fonts.width(dc, Face::Body, l)).collect();
	let widest = line_widths.iter().enumerate().map(|(i, w)| if i == 0 { w + icon_w } else { *w }).max().unwrap_or(0);
	let last_w = match line_widths.len() {
		0 => 0,
		1 => line_widths[0] + icon_w,
		n => line_widths[n - 1],
	};
	let meta_inline = !lines.is_empty() && last_w + GAP + corner_w <= text_max;

	let sender = if b.sender.is_empty() { String::new() } else { fonts.fit(dc, Face::Bold, &b.sender, content_max) };
	let sender_w = fonts.width(dc, Face::Bold, &sender);
	let quote_h = if b.quote.is_some() { line * 2 + PAD } else { 0 };
	let quote_w = b.quote.as_ref().map_or(0, |(who, what)| {
		(fonts.width(dc, Face::Bold, who).max(fonts.width(dc, Face::Body, what)) + PAD * 2).min(content_max)
	});

	let inner_w = [
		sender_w,
		quote_w,
		thumbs_w - 2 * PAD + 8,
		voice_w,
		if meta_inline { last_w + GAP + corner_w } else { widest.max(corner_w) },
	]
	.into_iter()
	.max()
	.unwrap_or(0)
	.min(content_max.max(thumbs_w - 2 * PAD + 8));
	let width = inner_w + 2 * PAD;

	let mut height = PAD / 2 + PAD / 2;
	if !sender.is_empty() {
		height += line;
	}
	if quote_h > 0 {
		height += quote_h + 4;
	}
	if thumb_h > 0 {
		height += thumb_h + 4;
	}
	height += voice_h;
	height += lines.len() as i32 * line;
	if !meta_inline {
		height += line;
	}
	let reactions_h = if b.reactions.is_empty() { 0 } else { line * 3 / 4 + 4 };

	Layout {
		width,
		height,
		sender,
		quote: b.quote.clone(),
		quote_h,
		thumb_w,
		thumb_h,
		voice_h,
		lines,
		icon_w,
		meta,
		meta_w,
		star_w,
		ticks_w,
		meta_inline,
		reactions_h,
	}
}

fn bubble_room(width: i32, left: i32) -> i32 {
	(width - left - MARGIN).max(80)
}

pub(crate) fn units(
	dc: &impl DeviceContext,
	base_font: &Font,
	look: &Look,
	width: i32,
	unit: i32,
	checkable: bool,
) -> u8 {
	let Look::Bubble(b) = look else { return 1 };
	let fonts = fonts_for(dc, base_font);
	let left = MARGIN + if checkable { fonts.line + 8 } else { 0 };
	let l = layout(dc, &fonts, b, bubble_room(width, left));
	let needed = l.height + 4 + l.reactions_h;
	(needed as usize).div_ceil(unit.max(1) as usize).clamp(1, MAX_UNITS) as u8
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

#[allow(clippy::too_many_arguments)]
fn round_box(dc: &impl DeviceContext, x: i32, y: i32, w: i32, h: i32, fill: Colour, bg: Colour) {
	if w <= 0 || h <= 0 {
		return;
	}
	dc.set_brush(fill, BrushStyle::Solid);
	dc.set_pen(fill, 1, PenStyle::Solid);
	dc.draw_rectangle(x, y, w, h);
	let radius = (h / 2).min(w / 2).clamp(1, 8);
	CACHE.with(|cache| {
		let mut cache = cache.borrow_mut();
		let pieces = cache.corners.entry((key(bg), radius)).or_insert_with(|| corners(bg, radius));
		dc.draw_bitmap(&pieces[0], x, y, true);
		dc.draw_bitmap(&pieces[1], x + w - radius, y, true);
		dc.draw_bitmap(&pieces[2], x, y + h - radius, true);
		dc.draw_bitmap(&pieces[3], x + w - radius, y + h - radius, true);
	});
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
	quote_mine: Colour,
	quote_theirs: Colour,
	pill: Colour,
	custom: bool,
}

impl Colours {
	fn name(&self, index: usize) -> Colour {
		if !self.custom {
			return self.text;
		}
		let names = if theme::is_dark() { &NAME_COLOURS_DARK } else { &NAME_COLOURS };
		let (r, g, b) = names[index % names.len()];
		Colour::rgb(r, g, b)
	}
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
			quote_mine: bg,
			quote_theirs: bg,
			pill: bg,
			custom: false,
		};
	}
	let band = selected.then(|| if focused { Palette::BAND_FOCUSED() } else { Palette::BAND() });
	Colours {
		background,
		band,
		focus_bar: (selected && focused).then_some(Palette::ACCENT()),
		text: Palette::TEXT_PRIMARY(),
		muted: Palette::TEXT_MUTED(),
		accent: Palette::ACCENT(),
		read: Palette::READ(),
		mine: Palette::BUBBLE_MINE(),
		theirs: Palette::BUBBLE_THEIRS(),
		quote_mine: Palette::QUOTE_MINE(),
		quote_theirs: Palette::QUOTE_THEIRS(),
		pill: Palette::BUBBLE_THEIRS(),
		custom: true,
	}
}

#[allow(clippy::too_many_arguments)]
fn shape(dc: &impl DeviceContext, c: &Colours, x: i32, y: i32, w: i32, h: i32, fill: Colour, bg: Colour) {
	if c.custom {
		round_box(dc, x, y, w, h, fill, bg);
	} else {
		dc.set_brush(fill, BrushStyle::Transparent);
		dc.set_pen(c.text, 1, PenStyle::Solid);
		dc.draw_rounded_rectangle(x, y, w, h, 4.0);
	}
}

pub(crate) struct RowPaint<'a> {
	pub y: i32,
	pub width: i32,
	pub unit: i32,
	pub units: u8,
	pub look: Option<&'a Look>,
	pub text: &'a str,
	pub selected: bool,
	pub focused: bool,
	pub check: Option<bool>,
}

pub(crate) fn paint_row(
	dc: &impl DeviceContext,
	base_font: &Font,
	background: Colour,
	foreground: Colour,
	row: RowPaint,
) {
	let fonts = fonts_for(dc, base_font);
	let fonts = &*fonts;
	let c = colours(background, foreground, row.selected, row.focused);
	let height = row.unit * row.units as i32;
	let row_bg = c.band.unwrap_or(c.background);
	if let Some(band) = c.band {
		dc.set_brush(band, BrushStyle::Solid);
		dc.set_pen(band, 1, PenStyle::Solid);
		dc.draw_rectangle(0, row.y, row.width + 64, height);
	}
	if let Some(bar) = c.focus_bar {
		dc.set_brush(bar, BrushStyle::Solid);
		dc.set_pen(bar, 1, PenStyle::Solid);
		dc.draw_rectangle(0, row.y, 4, height);
	}
	let mut left = MARGIN;
	if let Some(checked) = row.check {
		let (glyph, font) = match &fonts.icons {
			Some(font) => ((if checked { glyph::BOX_CHECKED } else { glyph::BOX }).to_string(), font),
			None => ((if checked { "[x]" } else { "[ ]" }).to_string(), &fonts.body),
		};
		dc.set_font(font);
		dc.set_text_foreground(c.text);
		let (_, gh) = dc.get_text_extent(&glyph);
		dc.draw_text(&glyph, left, row.y + (row.unit - gh) / 2);
		left += fonts.line + 8;
	}
	match row.look {
		Some(Look::Pill(text)) => pill(dc, fonts, &row, &c, row_bg, text),
		Some(Look::Bubble(bubble)) => draw_bubble(dc, fonts, &row, &c, row_bg, left, bubble),
		None => {
			dc.set_font(&fonts.body);
			dc.set_text_foreground(c.text);
			let line = fonts.fit(dc, Face::Body, row.text, row.width - left - MARGIN);
			dc.draw_text(&line, left, row.y + (row.unit - fonts.line) / 2);
		}
	}
}

fn pill(dc: &impl DeviceContext, fonts: &Fonts, row: &RowPaint, c: &Colours, row_bg: Colour, text: &str) {
	let text = fonts.fit(dc, Face::Small, text, row.width - 64);
	dc.set_font(&fonts.small);
	let (tw, th) = dc.get_text_extent(&text);
	let (w, h) = (tw + 24, (th + 8).min(row.unit - 4));
	let (x, y) = ((row.width - w) / 2, row.y + (row.unit - h) / 2);
	shape(dc, c, x, y, w, h, c.pill, row_bg);
	dc.set_font(&fonts.small);
	dc.set_text_foreground(c.muted);
	dc.draw_text(&text, x + 12, y + (h - th) / 2);
}

fn draw_bubble(
	dc: &impl DeviceContext,
	fonts: &Fonts,
	row: &RowPaint,
	c: &Colours,
	row_bg: Colour,
	left: i32,
	b: &Bubble,
) {
	let line = fonts.line;
	let l = layout(dc, fonts, b, bubble_room(row.width, left));
	let x = if b.from_me { row.width - MARGIN - l.width } else { left };
	let top = row.y + 2;
	let fill = if b.from_me { c.mine } else { c.theirs };
	shape(dc, c, x, top, l.width, l.height, fill, row_bg);

	let inner_x = x + PAD;
	let inner_w = l.width - 2 * PAD;
	let mut y = top + PAD / 2;

	if !l.sender.is_empty() {
		dc.set_font(&fonts.bold);
		dc.set_text_foreground(c.name(b.sender_colour));
		dc.draw_text(&l.sender, inner_x, y);
		y += line;
	}

	if let Some((who, what)) = &l.quote {
		let quote_fill = if b.from_me { c.quote_mine } else { c.quote_theirs };
		shape(dc, c, inner_x - 2, y, inner_w + 4, l.quote_h, quote_fill, fill);
		let bar = if who == "You" { c.accent } else { c.name(name_colour(who)) };
		dc.set_brush(bar, BrushStyle::Solid);
		dc.set_pen(bar, 1, PenStyle::Solid);
		dc.draw_rectangle(inner_x - 2, y, 4, l.quote_h);
		let text_x = inner_x + PAD;
		let text_w = inner_w - PAD - 4;
		let who = if who.is_empty() { "Quoted message".to_string() } else { who.clone() };
		dc.set_font(&fonts.bold);
		dc.set_text_foreground(bar);
		dc.draw_text(&fonts.fit(dc, Face::Bold, &who, text_w), text_x, y + PAD / 2);
		dc.set_font(&fonts.body);
		dc.set_text_foreground(c.muted);
		dc.draw_text(&fonts.fit(dc, Face::Body, what, text_w), text_x, y + PAD / 2 + line);
		y += l.quote_h + 4;
	}

	if l.thumb_h > 0 {
		let mut tx = x + 4;
		for id in &b.thumbs {
			let drawn = thumbnails::with_thumb(id, l.thumb_w, l.thumb_h, |thumb| match thumb {
				Thumb::Picture(bitmap) => {
					let (bw, bh) = (bitmap.get_width(), bitmap.get_height());
					dc.draw_bitmap(bitmap, tx + (l.thumb_w - bw) / 2, y + (l.thumb_h - bh) / 2, true);
					true
				}
				Thumb::Nothing | Thumb::Waiting => false,
			});
			if !drawn {
				let grey = if c.custom { Palette::PLACEHOLDER() } else { fill };
				shape(dc, c, tx, y, l.thumb_w, l.thumb_h, grey, fill);
				if let Some(font) = &fonts.icons_big {
					dc.set_font(font);
					dc.set_text_foreground(if c.custom { Colour::rgb(0xFF, 0xFF, 0xFF) } else { c.text });
					let g = if b.video { icon::VIDEO } else { icon::CAMERA }.to_string();
					let (gw, gh) = dc.get_text_extent(&g);
					dc.draw_text(&g, tx + (l.thumb_w - gw) / 2, y + (l.thumb_h - gh) / 2);
				}
			} else if b.video {
				play_button(dc, fonts, tx + l.thumb_w / 2, y + l.thumb_h / 2, line * 2, (0, 0, 0), 140);
			}
			tx += l.thumb_w + 4;
		}
		y += l.thumb_h + 4;
	}

	if b.voice.is_some() && b.audio_file {
		let size = l.voice_h;
		let mid = y + size / 2;
		let orange = if c.custom { (0xF3, 0x9C, 0x12) } else { (0x80, 0x80, 0x80) };
		round_badge(dc, inner_x, y, size, orange);
		if let Some(font) = &fonts.icons {
			dc.set_font(font);
			let g = icon::AUDIO.to_string();
			let (gw, gh) = dc.get_text_extent(&g);
			dc.set_text_foreground(Colour::rgb(0xFF, 0xFF, 0xFF));
			dc.draw_text(&g, inner_x + (size - gw) / 2, mid - gh / 2);
		}
		let (r, g, bl) =
			if c.custom { (Palette::ACCENT().r, Palette::ACCENT().g, Palette::ACCENT().b) } else { (0x80, 0x80, 0x80) };
		let play = size * 3 / 4;
		play_button(dc, fonts, inner_x + size + GAP + play / 2, mid, play, (r, g, bl), 255);
		let track_x = inner_x + size + 2 * GAP + play;
		let track_w = (inner_w - size - 2 * GAP - play).max(8);
		let colour = if c.custom { Palette::WAVE() } else { c.text };
		dc.set_brush(colour, BrushStyle::Solid);
		dc.set_pen(colour, 1, PenStyle::Solid);
		dc.draw_rectangle(track_x, mid - 1, track_w, 3);
		let knob = (line / 2).max(6);
		round_badge(dc, track_x, mid - knob / 2, knob, (r, g, bl));
		y += l.voice_h;
	} else if let Some(id) = &b.voice {
		let size = l.voice_h;
		let (r, g, bl) =
			if c.custom { (Palette::ACCENT().r, Palette::ACCENT().g, Palette::ACCENT().b) } else { (0x80, 0x80, 0x80) };
		play_button(dc, fonts, inner_x + size / 2, y + size / 2, size, (r, g, bl), 255);
		let bars_x = inner_x + size + GAP;
		let bars_w = inner_w - size - GAP;
		thumbnails::with_waveform(id, |bars| {
			let bars: Vec<u8> = match bars {
				Some(bars) => bars.to_vec(),
				None => vec![8; 48],
			};
			let count = bars.len().max(1) as i32;
			let step = (bars_w / count).max(2);
			let shown = (bars_w / step).min(count) as usize;
			let bar_w = (step * 2 / 3).max(1);
			let mid = y + size / 2;
			let colour = if b.from_me { Palette::WAVE_MINE() } else { Palette::WAVE() };
			let colour = if c.custom { colour } else { c.text };
			dc.set_brush(colour, BrushStyle::Solid);
			dc.set_pen(colour, 1, PenStyle::Solid);
			for (i, &v) in bars.iter().take(shown).enumerate() {
				let h = ((size - 6) * v.max(6) as i32 / 100).max(2);
				dc.draw_rectangle(bars_x + i as i32 * step, mid - h / 2, bar_w, h);
			}
		});
		y += l.voice_h;
	}

	let text_x = if l.thumb_h > 0 { x + 4 + 4 } else { inner_x };
	for (i, text) in l.lines.iter().enumerate() {
		let mut lx = text_x;
		if i == 0
			&& let (Some(g), Some(font)) = (b.icon, &fonts.icons)
			&& l.icon_w > 0
		{
			dc.set_font(font);
			dc.set_text_foreground(c.muted);
			let g = g.to_string();
			let gh = dc.get_text_extent(&g).1;
			dc.draw_text(&g, lx, y + (line - gh) / 2);
			lx += l.icon_w;
		}
		dc.set_font(&fonts.body);
		dc.set_text_foreground(c.text);
		dc.draw_text(text, lx, y);
		y += line;
	}

	let corner_y = if l.meta_inline { y - line } else { y };
	if let Some(_id) = &b.voice
		&& !b.body.is_empty()
	{
		dc.set_font(&fonts.small);
		dc.set_text_foreground(c.muted);
		let skip = if b.audio_file { l.voice_h + 2 * GAP + l.voice_h * 3 / 4 } else { l.voice_h + GAP };
		dc.draw_text(&b.body, inner_x + skip, corner_y + (line - dc.get_text_extent("Ag").1) / 2);
	}
	let mut mx = x + l.width - PAD - (l.star_w + l.meta_w + l.ticks_w);
	if b.starred
		&& let Some(font) = &fonts.icons_small
	{
		dc.set_font(font);
		dc.set_text_foreground(c.muted);
		let s = glyph::STAR.to_string();
		let sh = dc.get_text_extent(&s).1;
		dc.draw_text(&s, mx, corner_y + (line - sh) / 2);
		mx += l.star_w;
	}
	dc.set_font(&fonts.small);
	dc.set_text_foreground(c.muted);
	let mh = dc.get_text_extent("Ag").1;
	dc.draw_text(&l.meta, mx, corner_y + (line - mh) / 2);
	mx += l.meta_w + 3;
	if let (Some(t), Some(font)) = (b.ticks, &fonts.icons_small) {
		let (glyphs, blue) = tick_glyphs(t);
		dc.set_font(font);
		dc.set_text_foreground(if blue { c.read } else { c.muted });
		let mut chars = glyphs.chars();
		if let Some(first) = chars.next() {
			let one = first.to_string();
			let (w, h) = dc.get_text_extent(&one);
			let ty = corner_y + (line - h) / 2;
			dc.draw_text(&one, mx, ty);
			if let Some(second) = chars.next() {
				dc.draw_text(&second.to_string(), mx + w / 3, ty);
			}
		}
	}

	if !b.reactions.is_empty() {
		dc.set_font(&fonts.small);
		let (tw, th) = dc.get_text_extent(&b.reactions);
		let (w, h) = (tw + 14, th + 6);
		let px = if b.from_me { x + l.width - w - PAD } else { x + PAD };
		let py = top + l.height - h / 3;
		if c.custom {
			round_box(dc, px - 1, py - 1, w + 2, h + 2, Palette::OUTLINE(), row_bg);
			round_box(dc, px, py, w, h, c.pill, Palette::OUTLINE());
		} else {
			shape(dc, c, px, py, w, h, c.pill, row_bg);
		}
		dc.set_font(&fonts.small);
		dc.set_text_foreground(c.text);
		dc.draw_text(&b.reactions, px + 7, py + (h - th) / 2);
	}
}

fn round_badge(dc: &impl DeviceContext, x: i32, y: i32, size: i32, rgb: (u8, u8, u8)) {
	round_badge_alpha(dc, x, y, size, rgb, 255);
}

fn round_badge_alpha(dc: &impl DeviceContext, x: i32, y: i32, size: i32, rgb: (u8, u8, u8), alpha: u8) {
	let key = (rgb.0 as u32) << 24 | (rgb.1 as u32) << 16 | (rgb.2 as u32) << 8 | alpha as u32;
	CACHE.with(|cache| {
		let mut cache = cache.borrow_mut();
		let slot = cache.badges.iter().position(|(s, k, _)| *s == size && *k == key);
		let slot = match slot {
			Some(i) => i,
			None => {
				let rgba = super::gdi::round_rect_rgba(size, size, size as f32 / 2.0, rgb, alpha);
				let Some(bitmap) = Bitmap::from_rgba(&rgba, size.max(1) as u32, size.max(1) as u32) else { return };
				if cache.badges.len() >= 8 {
					cache.badges.remove(0);
				}
				cache.badges.push((size, key, bitmap));
				cache.badges.len() - 1
			}
		};
		dc.draw_bitmap(&cache.badges[slot].2, x, y, true);
	});
}

fn play_button(dc: &impl DeviceContext, fonts: &Fonts, cx: i32, cy: i32, size: i32, rgb: (u8, u8, u8), alpha: u8) {
	round_badge_alpha(dc, cx - size / 2, cy - size / 2, size, rgb, alpha);
	let Some(font) = &fonts.icons else { return };
	dc.set_font(font);
	let g = glyph::PLAY.to_string();
	let (gw, gh) = dc.get_text_extent(&g);
	dc.set_text_foreground(Colour::rgb(0xFF, 0xFF, 0xFF));
	dc.draw_text(&g, cx - gw / 2 + 1, cy - gh / 2);
}

const WALLPAPER_TILE: i32 = 192;

thread_local! {
	static WALLPAPER: RefCell<Option<(u32, Bitmap)>> = const { RefCell::new(None) };
}

/// A faint doodle pattern like WhatsApp's chat wallpaper, as a tile to repeat
/// over `background`: rings, dots, crosses and short strokes a shade darker.
/// Made once (about 150 KB) from a fixed seed, so it never changes.
fn wallpaper_tile(background: Colour) -> Option<Bitmap> {
	let side = WALLPAPER_TILE;
	let ink = if theme::is_dark() {
		(background.r.saturating_add(10), background.g.saturating_add(10), background.b.saturating_add(10))
	} else {
		(background.r.saturating_sub(14), background.g.saturating_sub(14), background.b.saturating_sub(12))
	};
	let mut coverage = vec![0f32; (side * side) as usize];
	let mut seed: u32 = 0x5EED_1234;
	let mut next = |range: i32| {
		seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
		((seed >> 8) % range.max(1) as u32) as i32
	};
	let plot = |coverage: &mut Vec<f32>, cx: f32, cy: f32, distance: &dyn Fn(f32, f32) -> f32, reach: i32| {
		for dy in -reach..=reach {
			for dx in -reach..=reach {
				let (x, y) = (cx as i32 + dx, cy as i32 + dy);
				let d = distance(x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
				let c = (1.0 - (d - 0.65).max(0.0)).clamp(0.0, 1.0);
				if c > 0.0 {
					let at = (y.rem_euclid(side) * side + x.rem_euclid(side)) as usize;
					coverage[at] = coverage[at].max(c);
				}
			}
		}
	};
	for _ in 0..34 {
		let (cx, cy) = (next(side) as f32, next(side) as f32);
		match next(4) {
			0 => {
				let r = 4.0 + next(5) as f32;
				plot(&mut coverage, cx, cy, &|x, y| ((x * x + y * y).sqrt() - r).abs(), r as i32 + 2);
			}
			1 => plot(&mut coverage, cx, cy, &|x, y| ((x * x + y * y).sqrt() - 1.2).max(0.0), 3),
			2 => {
				let a = 4.0 + next(3) as f32;
				plot(
					&mut coverage,
					cx,
					cy,
					&|x, y| if x.abs() <= a && y.abs() <= a { x.abs().min(y.abs()) } else { 9.0 },
					a as i32 + 2,
				);
			}
			_ => {
				let a = 5.0 + next(5) as f32;
				plot(
					&mut coverage,
					cx,
					cy,
					&|x, y| if (x + y).abs() <= a * 1.4 { (x - y).abs() / 1.414 } else { 9.0 },
					a as i32 + 2,
				);
			}
		}
	}
	let mut rgba = Vec::with_capacity((side * side * 4) as usize);
	for c in coverage {
		let mix = |b: u8, i: u8| (b as f32 + (i as f32 - b as f32) * c).round() as u8;
		rgba.extend_from_slice(&[mix(background.r, ink.0), mix(background.g, ink.1), mix(background.b, ink.2), 255]);
	}
	Bitmap::from_rgba(&rgba, side as u32, side as u32)
}

pub(crate) fn paint_wallpaper(dc: &impl DeviceContext, background: Colour, width: i32, height: i32) {
	let key = key(background);
	WALLPAPER.with(|w| {
		let mut w = w.borrow_mut();
		if w.as_ref().is_none_or(|(k, _)| *k != key) {
			*w = wallpaper_tile(background).map(|b| (key, b));
		}
		let Some((_, tile)) = w.as_ref() else { return };
		let mut y = 0;
		while y < height {
			let mut x = 0;
			while x < width {
				dc.draw_bitmap(tile, x, y, false);
				x += WALLPAPER_TILE;
			}
			y += WALLPAPER_TILE;
		}
	});
}

#[cfg(test)]
mod tests {
	use super::{Look, Ticks, look_for, name_colour, ticks_for};

	fn msg(text: &str, media: &str, from_me: bool) -> whatsnap::DisplayItem {
		let m = whatsnap::ChatMessage {
			id: "A1".into(),
			text: text.into(),
			media_type: media.into(),
			from_me,
			sender_name: "Bob".into(),
			sender_jid: "1@s.whatsapp.net".into(),
			..Default::default()
		};
		whatsnap::DisplayItem::Single(std::rc::Rc::new(m))
	}

	#[test]
	fn a_photo_shows_its_picture_and_caption() {
		let Look::Bubble(b) = look_for(&msg("[Image] beach", "image", false), true) else { panic!() };
		assert_eq!((b.sender.as_str(), b.body.as_str(), b.thumbs.len()), ("Bob", "beach", 1));
	}

	#[test]
	fn a_view_once_photo_shows_no_picture() {
		let Look::Bubble(b) = look_for(&msg("[View once] [Image]", "image", false), false) else { panic!() };
		assert!(b.thumbs.is_empty());
	}

	#[test]
	fn a_voice_message_shows_its_length_and_waveform() {
		let Look::Bubble(b) = look_for(&msg("[Voice Message] (0:12)", "audio", true), false) else { panic!() };
		assert_eq!(b.body, "0:12");
		assert_eq!(b.voice.as_deref(), Some("A1"));
		assert!(b.sender.is_empty());
		assert!(!b.audio_file);
	}

	#[test]
	fn an_audio_file_is_drawn_unlike_a_voice_message() {
		let Look::Bubble(b) = look_for(&msg("[Audio] (3:05)", "audio", false), false) else { panic!() };
		assert!(b.audio_file);
		assert_eq!(b.body, "3:05");
	}

	#[test]
	fn read_and_played_both_show_blue_ticks() {
		assert_eq!(ticks_for("read"), Ticks::Read);
		assert_eq!(ticks_for("played"), Ticks::Read);
		assert_eq!(ticks_for(""), Ticks::Sent);
	}

	#[test]
	fn a_person_keeps_their_name_colour() {
		assert_eq!(name_colour("1@s.whatsapp.net"), name_colour("1@s.whatsapp.net"));
	}
}
