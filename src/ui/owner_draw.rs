use std::{cell::RefCell, collections::HashMap, rc::Rc};

use windows::Win32::{
	Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM},
	Graphics::Gdi::{
		BitBlt, COLOR_BTNFACE, COLOR_BTNTEXT, COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW,
		COLOR_WINDOWTEXT, CreateCompatibleBitmap, CreateCompatibleDC, DT_CENTER, DeleteDC, DeleteObject, GetDC,
		GetObjectW, GetSysColor, GetTextMetricsW, HDC, HFONT, HGDIOBJ, InvalidateRect, LOGFONTW, ReleaseDC, SRCCOPY,
		SelectObject, SetBkMode, TEXTMETRICW, TRANSPARENT,
	},
	UI::{
		Controls::{DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS, ODS_SELECTED, ODT_BUTTON, ODT_LISTBOX},
		Input::KeyboardAndMouse::GetFocus,
		Shell::{DefSubclassProc, SetWindowSubclass},
		WindowsAndMessaging::{
			BS_OWNERDRAW, GWL_STYLE, GetParent, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, LB_GETTEXT,
			LB_GETTEXTLEN, LB_SETITEMHEIGHT, SendMessageW, SetWindowLongPtrW, WM_DRAWITEM, WM_GETFONT, WM_SETFONT,
			WM_SETTEXT,
		},
	},
};
use wxdragon::prelude::*;

use super::{
	avatars::{self, Avatar},
	gdi::{self, colorref},
	theme::{self, Palette},
};

const PARENT_SUBCLASS: usize = 0x4f44_5250;
const CONTROL_SUBCLASS: usize = 0x4f44_4354;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
	Nav,
	Icon,
	Header,
	Tile,
	Media,
	People,
}

struct Control {
	kind: Kind,
	background: Colour,
}

struct Fonts {
	base: HFONT,
	body: HFONT,
	bold: HFONT,
	small: HFONT,
	small_bold: HFONT,
	icons: Option<HFONT>,
	icons_big: Option<HFONT>,
	line: i32,
}

impl Drop for Fonts {
	fn drop(&mut self) {
		for font in
			[self.body, self.bold, self.small, self.small_bold].into_iter().chain(self.icons).chain(self.icons_big)
		{
			gdi::delete_font(font);
		}
	}
}

type MediaSource = Rc<dyn Fn(usize) -> Option<(String, bool)>>;

#[derive(Default)]
struct Registry {
	controls: HashMap<isize, Control>,
	parents: Vec<isize>,
	fonts: HashMap<isize, Rc<Fonts>>,
	header_jid: String,
	own_jid: String,
	nav_panel: Option<HWND>,
	media_sources: HashMap<isize, MediaSource>,
	people_sources: HashMap<isize, Rc<dyn Fn(usize) -> Option<String>>>,
	media_pictures: Vec<(String, i32, Option<gdi::Bitmap>)>,
	header: Option<HWND>,
}

thread_local! {
	static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

mod glyph {
	pub const CHATS: char = '\u{E8BD}';
	pub const STATUS: char = '\u{EA3A}';
	pub const STARRED: char = '\u{E734}';
	pub const ARCHIVED: char = '\u{E7B8}';
	pub const COMMUNITIES: char = '\u{E716}';
	pub const CHANNELS: char = '\u{E789}';
	pub const CALLS: char = '\u{E717}';
	pub const SEARCH: char = '\u{E721}';
	pub const VIDEO: char = '\u{E714}';
	pub const EMOJI: char = '\u{E76E}';
	pub const ADD: char = '\u{E710}';
	pub const SEND: char = '\u{E724}';
	pub const MIC: char = '\u{E720}';
	pub const CLOSE: char = '\u{E711}';
	pub const DELETE: char = '\u{E74D}';
	pub const PAUSE: char = '\u{E769}';
	pub const PLAY: char = '\u{E768}';
	pub const HANG_UP: char = '\u{E778}';
	pub const MIC_OFF: char = '\u{EC54}';
	pub const SCREEN: char = '\u{E7F4}';
	pub const ADD_PERSON: char = '\u{E8FA}';
	pub const HAND: char = '\u{270B}';
	pub const EDIT: char = '\u{E70F}';
	pub const LOCK: char = '\u{E72E}';
	pub const ADMIN: char = '\u{E8D7}';
	pub const REMOVE_PERSON: char = '\u{E8E0}';
	pub const LINK: char = '\u{E71B}';
	pub const BACK: char = '\u{E76B}';
	pub const FORWARD: char = '\u{E76C}';
	pub const VOLUME: char = '\u{E767}';
	pub const VOLUME_OFF: char = '\u{E74F}';
}

fn nav_look(text: &str) -> (Option<char>, &str, u32) {
	let (name, unread) = match text.split_once(": ") {
		Some((name, rest)) => (name, rest.split(' ').next().and_then(|n| n.parse().ok()).unwrap_or(0)),
		None => (text, 0),
	};
	let (icon, short) = nav_icon(name);
	(icon, short, unread)
}

fn nav_icon(name: &str) -> (Option<char>, &str) {
	match name {
		"Chats" => (Some(glyph::CHATS), "Chats"),
		"Status" => (Some(glyph::STATUS), "Status"),
		"Starred Messages" => (Some(glyph::STARRED), "Starred"),
		"Archived Chats" => (Some(glyph::ARCHIVED), "Archived"),
		"Communities" => (Some(glyph::COMMUNITIES), "Communities"),
		"Channels" => (Some(glyph::CHANNELS), "Channels"),
		"Calls" => (Some(glyph::CALLS), "Calls"),
		other => (None, other),
	}
}

fn button_glyph(label: &str) -> Option<char> {
	// Labels may carry a keyboard shortcut ("Pa&use").
	let label = label.replace('&', "");
	Some(match label.as_str() {
		"Search in conversation" => glyph::SEARCH,
		"Call" => glyph::CALLS,
		"Video call" => glyph::VIDEO,
		"Emoji and stickers" => glyph::EMOJI,
		"Add attachment" | "Add more files" => glyph::ADD,
		"Send message" | "Send voice message" => glyph::SEND,
		"Record voice message" => glyph::MIC,
		"Cancel edit" | "Remove quote" | "Close search" => glyph::CLOSE,
		"Discard voice message" => glyph::DELETE,
		"Pause recording" | "Pause" => glyph::PAUSE,
		"Resume recording" | "Resume" | "Play" => glyph::PLAY,
		"Previous" => glyph::BACK,
		"Next" => glyph::FORWARD,
		"Mute" => glyph::VOLUME,
		"Unmute" => glyph::VOLUME_OFF,
		"Send reply" => glyph::SEND,
		_ => return None,
	})
}

fn hwnd_of(widget: &impl WxWidget) -> HWND {
	HWND(widget.get_handle().cast())
}

fn register(hwnd: HWND, kind: Kind, background: Colour) {
	let parent = unsafe { GetParent(hwnd) }.unwrap_or_default();
	let new_parent = REGISTRY.with(|r| {
		let mut r = r.borrow_mut();
		r.controls.insert(hwnd.0 as isize, Control { kind, background });
		if r.parents.contains(&(parent.0 as isize)) {
			false
		} else {
			r.parents.push(parent.0 as isize);
			true
		}
	});
	unsafe {
		if new_parent {
			let _ = SetWindowSubclass(parent, Some(parent_proc), PARENT_SUBCLASS, 0);
		}
		let _ = SetWindowSubclass(hwnd, Some(control_proc), CONTROL_SUBCLASS, 0);
	}
}

fn make_owner_drawn_button(hwnd: HWND) {
	unsafe {
		let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
		SetWindowLongPtrW(hwnd, GWL_STYLE, (style & !0xF) | BS_OWNERDRAW as isize);
	}
}

pub(crate) fn install_nav(list: &ListBox) {
	let hwnd = hwnd_of(list);
	register(hwnd, Kind::Nav, Palette::PANEL_BG());
	set_nav_height(hwnd);
}

pub(crate) fn install_icon_button(button: &Button, background: Colour) {
	let hwnd = hwnd_of(button);
	make_owner_drawn_button(hwnd);
	register(hwnd, Kind::Icon, background);
	button.set_tooltip(&button.get_label().replace('&', ""));
	let side = button.get_char_height() * 2 + 10;
	button.set_min_size(Size::new(side, side));
}

pub(crate) fn install_tile(button: &Button, background: Colour) {
	let hwnd = hwnd_of(button);
	make_owner_drawn_button(hwnd);
	register(hwnd, Kind::Tile, background);
	button.set_tooltip(&button.get_label());
	let line = button.get_char_height();
	button.set_min_size(Size::new(line * 4 + 4, line * 4 + 12));
}

pub(crate) fn install_header(button: &Button, background: Colour) {
	let hwnd = hwnd_of(button);
	make_owner_drawn_button(hwnd);
	register(hwnd, Kind::Header, background);
	REGISTRY.with(|r| r.borrow_mut().header = Some(hwnd));
	button.set_min_size(Size::new(160, button.get_char_height() * 3 + 8));
	avatars::on_change(move || unsafe {
		let _ = InvalidateRect(Some(hwnd), None, false);
	});
}

/// The foot of the navigation strip, as in WhatsApp for Windows: your own
/// picture, which opens Settings (menu item `settings_id`) when clicked. Only drawn: keyboard and screen reader users have the
/// Account menu and Ctrl+comma, so they add no Tab stops.
pub(crate) fn install_nav_footer(panel: &Panel, frame: &Frame, settings_id: i32) -> i32 {
	let line = panel.get_char_height();
	let item = line * 3;
	let height = item + 16;
	let panel_c = *panel;
	let frame_hwnd = hwnd_of(frame).0 as isize;
	REGISTRY.with(|r| r.borrow_mut().nav_panel = Some(hwnd_of(panel)));
	panel.set_background_style(BackgroundStyle::Paint);
	panel.on_paint(move |_| {
		let dc = AutoBufferedPaintDC::new(&panel_c);
		let size = panel_c.get_client_size();
		let background = panel_c.get_background_color();
		dc.set_brush(background, BrushStyle::Solid);
		dc.set_pen(background, 1, PenStyle::Solid);
		dc.draw_rectangle(0, 0, size.width, size.height);
		let custom = theme::use_custom_colours();
		let ink = if custom { Palette::ICON() } else { panel_c.get_foreground_color() };
		let top = size.height - height;
		dc.set_pen(if custom { Palette::DIVIDER_SOFT() } else { ink }, 1, PenStyle::Solid);
		dc.draw_line(8, top, size.width - 8, top);
		let cx = size.width / 2;
		let jid = REGISTRY.with(|r| r.borrow().own_jid.clone());
		let side = (item - 8).max(16);
		let (x, y) = (cx - side / 2, top + 8 + (item - side) / 2);
		match avatars::path_of(&jid).and_then(|path| own_picture(&path, side)) {
			Some(bitmap) => dc.draw_bitmap(&bitmap, x, y, true),
			None => {
				let grey = if custom { Palette::AVATAR_BG() } else { ink };
				dc.set_brush(grey, BrushStyle::Solid);
				dc.set_pen(grey, 1, PenStyle::Solid);
				dc.draw_circle(cx, y + side / 2, side / 2);
			}
		}
	});
	panel.on_mouse_left_down(move |event| {
		if let WindowEventData::MouseButton(ref mouse) = event
			&& let Some(pos) = mouse.get_position()
			&& pos.y >= panel_c.get_client_size().height - height
		{
			use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_COMMAND};
			unsafe {
				let _ = PostMessageW(
					Some(HWND(frame_hwnd as *mut _)),
					WM_COMMAND,
					WPARAM(settings_id as u16 as usize),
					LPARAM(0),
				);
			}
		}
		event.skip(true);
	});
	panel.on_size(move |event| {
		panel_c.refresh(true, None);
		event.skip(true);
	});
	avatars::on_change(move || panel_c.refresh(false, None));
	height
}

thread_local! {
	static OWN_PICTURE: RefCell<Option<(String, i32, wxdragon::bitmap::Bitmap)>> = const { RefCell::new(None) };
}

fn own_picture(path: &str, side: i32) -> Option<wxdragon::bitmap::Bitmap> {
	OWN_PICTURE.with(|p| {
		let mut p = p.borrow_mut();
		if p.as_ref().is_none_or(|(f, s, _)| f != path || *s != side) {
			*p = round_picture(path, side as u32).map(|b| (path.to_string(), side, b));
		}
		p.as_ref().map(|(_, _, b)| b.clone())
	})
}

pub(crate) fn round_picture(path: &str, side: u32) -> Option<wxdragon::bitmap::Bitmap> {
	let (w, h, mut rgba) = crate::media::image_decode::decode_rgba_in_box(path, side, side).ok()?;
	let (cx, cy, r) = (w as f32 / 2.0, h as f32 / 2.0, w.min(h) as f32 / 2.0);
	for y in 0..h {
		for x in 0..w {
			let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
			let cover = (r - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
			let a = &mut rgba[((y * w + x) * 4 + 3) as usize];
			*a = (*a as f32 * cover) as u8;
		}
	}
	wxdragon::bitmap::Bitmap::from_rgba(&rgba, w, h)
}

/// Draws an info page's Media list (built with `chat_rows::OWNER_DRAW`) with
/// each photo's or video's picture beside its text, which is still all a
/// screen reader reads. `message` gives a row's message id and whether
/// it's a video.
pub(crate) fn install_media_list(list: &ListBox, message: impl Fn(usize) -> Option<(String, bool)> + 'static) {
	let hwnd = hwnd_of(list);
	REGISTRY.with(|r| r.borrow_mut().media_sources.insert(hwnd.0 as isize, Rc::new(message)));
	register(hwnd, Kind::Media, Palette::CONTENT_BG());
	let height = with_fonts(hwnd, |f| row_height(Kind::Media, f));
	unsafe {
		SendMessageW(hwnd, LB_SETITEMHEIGHT, Some(WPARAM(0)), Some(LPARAM(height as isize)));
	}
}

fn row_height(kind: Kind, fonts: &Fonts) -> i32 {
	match kind {
		Kind::People => person_side(fonts) + 18,
		_ => fonts.line * 3 + 16,
	}
}

fn person_side(fonts: &Fonts) -> i32 {
	avatars::size().unwrap_or(fonts.line * 5 / 2).clamp(fonts.line, fonts.line * 3)
}

/// Draws a list of people (built with `chat_rows::OWNER_DRAW`) with each
/// one's picture before their row's text, which is still all a screen
/// reader reads. `jid` gives a row's person.
pub(crate) fn install_people_list(list: &ListBox, jid: impl Fn(usize) -> Option<String> + 'static) {
	let hwnd = hwnd_of(list);
	REGISTRY.with(|r| r.borrow_mut().people_sources.insert(hwnd.0 as isize, Rc::new(jid)));
	register(hwnd, Kind::People, Palette::CONTENT_BG());
	let height = with_fonts(hwnd, |f| row_height(Kind::People, f));
	unsafe {
		SendMessageW(hwnd, LB_SETITEMHEIGHT, Some(WPARAM(0)), Some(LPARAM(height as isize)));
	}
	avatars::on_change(move || unsafe {
		let _ = InvalidateRect(Some(hwnd), None, false);
	});
}

fn paint_person(
	dc: HDC,
	width: i32,
	height: i32,
	text: &str,
	jid: &str,
	fonts: &Fonts,
	(selected, focused): (bool, bool),
) {
	let custom = theme::use_custom_colours();
	let sys = |i| COLORREF(unsafe { GetSysColor(i) });
	let (background, ink) = match (custom, selected) {
		(true, true) => (rgb(Palette::SELECTED_ROW()), rgb(Palette::TEXT_PRIMARY())),
		(true, false) => (rgb(Palette::CONTENT_BG()), rgb(Palette::TEXT_PRIMARY())),
		(false, true) => (sys(COLOR_HIGHLIGHT), sys(COLOR_HIGHLIGHTTEXT)),
		(false, false) => (sys(COLOR_WINDOW), sys(COLOR_WINDOWTEXT)),
	};
	gdi::fill(dc, rect(0, 0, width, height), background);
	if custom && selected && focused {
		gdi::fill(dc, rect(0, 0, 3, height), rgb(Palette::ACCENT()));
	}
	let side = person_side(fonts);
	let (x, y) = (10, (height - side) / 2);
	let drawn = avatars::with_avatar(jid, side, |avatar| match avatar {
		Avatar::Picture(bitmap) => {
			bitmap.draw(dc, x, y);
			true
		}
		Avatar::Nothing | Avatar::Waiting => false,
	});
	if !drawn {
		let grey = if custom { theme::tuple(Palette::AVATAR_BG()) } else { (0x80, 0x80, 0x80) };
		if let Some(circle) = gdi::round_rect(side, side, side as f32 / 2.0, grey) {
			circle.draw(dc, x, y);
		}
		if let Some(font) = fonts.icons_big {
			gdi::text(dc, font, colorref(0xFF, 0xFF, 0xFF), "\u{E77B}", rect(x, y, x + side, y + side), DT_CENTER);
		}
	}
	gdi::text(dc, fonts.body, ink, text, rect(x + side + 12, 0, width - 8, height), Default::default());
}

const MEDIA_PICTURES: usize = 32;

fn with_media_picture(id: &str, side: i32, draw: impl FnOnce(Option<&gdi::Bitmap>)) {
	let cached = REGISTRY.with(|r| {
		let r = r.borrow();
		r.media_pictures.iter().position(|(i, s, _)| i == id && *s == side)
	});
	if cached.is_none() {
		let Some(path) = super::thumbnails::path_of(id) else { return draw(None) };
		let bitmap = path.and_then(|path| {
			let (w, h, rgba) = crate::media::image_decode::decode_rgba(&path, (side * 3) as u32).ok()?;
			let scale = side as f32 / w.min(h).max(1) as f32;
			let (sw, sh) = (((w as f32 * scale) as i32).max(side), ((h as f32 * scale) as i32).max(side));
			let (ox, oy) = ((sw - side) / 2, (sh - side) / 2);
			let mut out = vec![0u8; (side * side * 4) as usize];
			for y in 0..side {
				for x in 0..side {
					let sx = (((x + ox) as f32 / scale) as u32).min(w - 1);
					let sy = (((y + oy) as f32 / scale) as u32).min(h - 1);
					let from = ((sy * w + sx) * 4) as usize;
					let to = ((y * side + x) * 4) as usize;
					out[to..to + 4].copy_from_slice(&rgba[from..from + 4]);
				}
			}
			gdi::Bitmap::from_rgba(side, side, &out)
		});
		REGISTRY.with(|r| {
			let mut r = r.borrow_mut();
			if r.media_pictures.len() >= MEDIA_PICTURES {
				r.media_pictures.remove(0);
			}
			r.media_pictures.push((id.to_string(), side, bitmap));
		});
	}
	// Taken out while drawing, so drawing never runs with the registry borrowed.
	let entry = REGISTRY.with(|r| {
		let mut r = r.borrow_mut();
		let at = r.media_pictures.iter().position(|(i, s, _)| i == id && *s == side)?;
		Some(r.media_pictures.remove(at))
	});
	match entry {
		Some(entry) => {
			draw(entry.2.as_ref());
			REGISTRY.with(|r| r.borrow_mut().media_pictures.push(entry));
		}
		None => draw(None),
	}
}

pub(crate) fn refresh_media_lists() {
	let lists: Vec<isize> = REGISTRY.with(|r| {
		let mut r = r.borrow_mut();
		r.media_pictures.retain(|(_, _, b)| b.is_some());
		r.media_sources.keys().copied().collect()
	});
	for hwnd in lists {
		unsafe {
			let _ = InvalidateRect(Some(HWND(hwnd as *mut _)), None, false);
		}
	}
}

fn paint_media(
	dc: HDC,
	width: i32,
	height: i32,
	text: &str,
	message: Option<(String, bool)>,
	fonts: &Fonts,
	(selected, focused): (bool, bool),
) {
	let custom = theme::use_custom_colours();
	let c = if custom {
		colours(if selected { Palette::SELECTED_ROW() } else { Palette::CONTENT_BG() }, false)
	} else {
		colours(Palette::CONTENT_BG(), selected)
	};
	let background = if !custom && !selected { COLORREF(unsafe { GetSysColor(COLOR_WINDOW) }) } else { c.background };
	let text_colour = if !custom && !selected { COLORREF(unsafe { GetSysColor(COLOR_WINDOWTEXT) }) } else { c.text };
	gdi::fill(dc, rect(0, 0, width, height), background);
	if selected && focused && custom {
		gdi::fill(dc, rect(0, 0, 3, height), c.focus);
	}
	let pad = 6;
	let side = height - 2 * pad;
	let mut drawn = false;
	if let Some((id, video)) = &message {
		with_media_picture(id, side, |bitmap| {
			if let Some(bitmap) = bitmap {
				bitmap.draw(dc, pad + 4, pad);
				drawn = true;
			}
		});
		if !drawn {
			gdi::fill(dc, rect(pad + 4, pad, pad + 4 + side, pad + side), rgb(Palette::PLACEHOLDER()));
		}
		if *video && let Some(font) = fonts.icons {
			let r = side / 3;
			let (cx, cy) = (pad + 4 + side / 2, pad + side / 2);
			blob(dc, cx - r / 2, cy - r / 2, r, r, colorref(0x30, 0x30, 0x30));
			gdi::text(
				dc,
				font,
				colorref(0xFF, 0xFF, 0xFF),
				&glyph::PLAY.to_string(),
				rect(cx - r / 2, cy - r / 2, cx + r / 2, cy + r / 2),
				DT_CENTER,
			);
		}
	}
	let left = if message.is_some() { pad + 4 + side + 12 } else { pad + 4 };
	gdi::text(dc, fonts.body, text_colour, text, rect(left, 0, width - pad, height), Default::default());
}

pub(crate) fn set_own_jid(jid: &str) {
	let bare = match jid.split_once('@') {
		Some((user, server)) => format!("{}@{}", user.split(':').next().unwrap_or(user), server),
		None => jid.to_string(),
	};
	let panel = REGISTRY.with(|r| {
		let mut r = r.borrow_mut();
		r.own_jid = bare;
		r.nav_panel
	});
	if let Some(hwnd) = panel {
		unsafe {
			let _ = InvalidateRect(Some(hwnd), None, false);
		}
	}
}

pub(crate) fn set_header_chat(jid: &str) {
	let header = REGISTRY.with(|r| {
		let mut r = r.borrow_mut();
		r.header_jid = jid.to_string();
		r.header
	});
	if let Some(hwnd) = header {
		unsafe {
			let _ = InvalidateRect(Some(hwnd), None, false);
		}
	}
}

fn nav_height(fonts: &Fonts) -> i32 {
	fonts.line * 2 + fonts.line * 4 / 5 + 14
}

fn set_nav_height(hwnd: HWND) {
	let height = with_fonts(hwnd, nav_height);
	unsafe {
		SendMessageW(hwnd, LB_SETITEMHEIGHT, Some(WPARAM(0)), Some(LPARAM(height as isize)));
	}
}

fn with_fonts<R>(hwnd: HWND, f: impl FnOnce(&Fonts) -> R) -> R {
	let base = HFONT(unsafe { SendMessageW(hwnd, WM_GETFONT, None, None) }.0 as *mut _);
	let key = hwnd.0 as isize;
	let cached = REGISTRY.with(|r| r.borrow().fonts.get(&key).filter(|fonts| fonts.base == base).cloned());
	let fonts = match cached {
		Some(fonts) => fonts,
		None => {
			let fonts = Rc::new(make_fonts(hwnd, base));
			REGISTRY.with(|r| r.borrow_mut().fonts.insert(key, fonts.clone()));
			fonts
		}
	};
	f(&fonts)
}

fn make_fonts(hwnd: HWND, base: HFONT) -> Fonts {
	let mut lf = LOGFONTW::default();
	unsafe {
		GetObjectW(HGDIOBJ(base.0), size_of::<LOGFONTW>() as i32, Some(std::ptr::from_mut(&mut lf).cast()));
	}
	if lf.lfHeight == 0 {
		lf.lfHeight = -16;
	}
	let dc = unsafe { GetDC(Some(hwnd)) };
	let body = gdi::derived_font(&lf, 1.0, 400);
	let mut tm = TEXTMETRICW::default();
	unsafe {
		let old = SelectObject(dc, HGDIOBJ(body.0));
		let _ = GetTextMetricsW(dc, &mut tm);
		SelectObject(dc, old);
	}
	let fonts = Fonts {
		base,
		body,
		bold: gdi::derived_font(&lf, 1.05, 600),
		small: gdi::derived_font(&lf, 0.8, 400),
		small_bold: gdi::derived_font(&lf, 0.8, 600),
		icons: gdi::icon_font(dc, &lf, 1.15),
		icons_big: gdi::icon_font(dc, &lf, 1.45),
		line: tm.tmHeight.max(8),
	};
	unsafe { ReleaseDC(Some(hwnd), dc) };
	fonts
}

unsafe extern "system" fn control_proc(
	hwnd: HWND,
	msg: u32,
	wparam: WPARAM,
	lparam: LPARAM,
	_id: usize,
	_data: usize,
) -> LRESULT {
	let kind = REGISTRY.with(|r| r.try_borrow().ok().and_then(|r| r.controls.get(&(hwnd.0 as isize)).map(|c| c.kind)));
	match (msg, kind) {
		(LB_SETITEMHEIGHT, Some(kind @ (Kind::Media | Kind::People))) => {
			let height = with_fonts(hwnd, |f| row_height(kind, f));
			unsafe { DefSubclassProc(hwnd, msg, wparam, LPARAM(height as isize)) }
		}
		(WM_SETFONT, Some(kind @ (Kind::Media | Kind::People))) => {
			let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
			let height = with_fonts(hwnd, |f| row_height(kind, f));
			unsafe {
				SendMessageW(hwnd, LB_SETITEMHEIGHT, Some(WPARAM(0)), Some(LPARAM(height as isize)));
			}
			result
		}
		(LB_SETITEMHEIGHT, Some(Kind::Nav)) => {
			let height = with_fonts(hwnd, nav_height);
			unsafe { DefSubclassProc(hwnd, msg, wparam, LPARAM(height as isize)) }
		}
		(WM_SETFONT, Some(Kind::Nav)) => {
			let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
			set_nav_height(hwnd);
			result
		}
		(WM_SETTEXT, Some(Kind::Icon | Kind::Header | Kind::Tile)) => {
			let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
			unsafe {
				let _ = InvalidateRect(Some(hwnd), None, false);
			}
			result
		}
		_ => unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
	}
}

unsafe extern "system" fn parent_proc(
	hwnd: HWND,
	msg: u32,
	wparam: WPARAM,
	lparam: LPARAM,
	_id: usize,
	_data: usize,
) -> LRESULT {
	if msg == WM_DRAWITEM && lparam.0 != 0 {
		let item = unsafe { &*(lparam.0 as *const DRAWITEMSTRUCT) };
		let control = REGISTRY.with(|r| {
			r.try_borrow()
				.ok()
				.and_then(|r| r.controls.get(&(item.hwndItem.0 as isize)).map(|c| (c.kind, theme::now(c.background))))
		});
		if let Some((kind, background)) = control {
			let fits = match kind {
				Kind::Nav | Kind::Media | Kind::People => item.CtlType == ODT_LISTBOX && item.itemID != u32::MAX,
				Kind::Icon | Kind::Header | Kind::Tile => item.CtlType == ODT_BUTTON,
			};
			if fits {
				draw(item, kind, background);
				return LRESULT(1);
			}
		}
	}
	unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

fn list_text(hwnd: HWND, index: u32) -> String {
	unsafe {
		let len = SendMessageW(hwnd, LB_GETTEXTLEN, Some(WPARAM(index as usize)), None).0;
		if len <= 0 {
			return String::new();
		}
		let mut buf = vec![0u16; len as usize + 1];
		let got =
			SendMessageW(hwnd, LB_GETTEXT, Some(WPARAM(index as usize)), Some(LPARAM(buf.as_mut_ptr() as isize))).0;
		String::from_utf16_lossy(&buf[..got.clamp(0, len) as usize])
	}
}

fn window_text(hwnd: HWND) -> String {
	unsafe {
		let len = GetWindowTextLengthW(hwnd);
		if len <= 0 {
			return String::new();
		}
		let mut buf = vec![0u16; len as usize + 1];
		let got = GetWindowTextW(hwnd, &mut buf);
		String::from_utf16_lossy(&buf[..got.clamp(0, len) as usize])
	}
}

struct Colours {
	background: COLORREF,
	text: COLORREF,
	muted: COLORREF,
	pressed: COLORREF,
	focus: COLORREF,
	custom: bool,
}

fn rgb(colour: Colour) -> COLORREF {
	colorref(colour.r, colour.g, colour.b)
}

fn colours(background: Colour, selected: bool) -> Colours {
	if !theme::use_custom_colours() {
		let sys = |i| COLORREF(unsafe { GetSysColor(i) });
		return if selected {
			Colours {
				background: sys(COLOR_HIGHLIGHT),
				text: sys(COLOR_HIGHLIGHTTEXT),
				muted: sys(COLOR_HIGHLIGHTTEXT),
				pressed: sys(COLOR_HIGHLIGHT),
				focus: sys(COLOR_HIGHLIGHTTEXT),
				custom: false,
			}
		} else {
			Colours {
				background: sys(COLOR_BTNFACE),
				text: sys(COLOR_BTNTEXT),
				muted: sys(COLOR_GRAYTEXT),
				pressed: sys(COLOR_WINDOW),
				focus: sys(COLOR_WINDOWTEXT),
				custom: false,
			}
		};
	}
	Colours {
		background: rgb(background),
		text: rgb(Palette::ICON()),
		muted: rgb(Palette::TEXT_MUTED()),
		pressed: rgb(Palette::PRESSED()),
		focus: rgb(Palette::ACCENT()),
		custom: true,
	}
}

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
	RECT { left, top, right, bottom }
}

fn draw(item: &DRAWITEMSTRUCT, kind: Kind, background: Colour) {
	let hwnd = item.hwndItem;
	let r = item.rcItem;
	let (width, height) = (r.right - r.left, r.bottom - r.top);
	if width <= 0 || height <= 0 {
		return;
	}
	let selected = item.itemState.0 & ODS_SELECTED.0 != 0;
	let disabled = item.itemState.0 & ODS_DISABLED.0 != 0;
	let focused = match kind {
		Kind::Nav | Kind::Media | Kind::People => (unsafe { GetFocus() }) == hwnd,
		Kind::Icon | Kind::Header | Kind::Tile => item.itemState.0 & ODS_FOCUS.0 != 0,
	};
	// Drawn off screen and copied in one go, so it never flickers.
	unsafe {
		let mem = CreateCompatibleDC(Some(item.hDC));
		let bitmap = CreateCompatibleBitmap(item.hDC, width, height);
		let old = SelectObject(mem, HGDIOBJ(bitmap.0));
		SetBkMode(mem, TRANSPARENT);
		with_fonts(hwnd, |fonts| match kind {
			Kind::Nav => {
				let text = list_text(hwnd, item.itemID);
				paint_nav(mem, width, height, &text, fonts, selected, focused, background);
			}
			Kind::Media => {
				let text = list_text(hwnd, item.itemID);
				let source = REGISTRY.with(|r| r.borrow().media_sources.get(&(hwnd.0 as isize)).cloned());
				let message = source.and_then(|source| source(item.itemID as usize));
				paint_media(mem, width, height, &text, message, fonts, (selected, focused));
			}
			Kind::People => {
				let text = list_text(hwnd, item.itemID);
				let source = REGISTRY.with(|r| r.borrow().people_sources.get(&(hwnd.0 as isize)).cloned());
				let jid = source.and_then(|source| source(item.itemID as usize)).unwrap_or_default();
				paint_person(mem, width, height, &text, &jid, fonts, (selected, focused));
			}
			Kind::Tile => {
				let state = (selected, focused, disabled);
				paint_tile(mem, width, height, &window_text(hwnd), fonts, state, background)
			}
			Kind::Icon => {
				let state = (selected, focused, disabled);
				paint_icon(mem, width, height, &window_text(hwnd), fonts, state, background)
			}
			Kind::Header => paint_header(mem, width, height, &window_text(hwnd), fonts, selected, focused, background),
		});
		let _ = BitBlt(item.hDC, r.left, r.top, width, height, Some(mem), 0, 0, SRCCOPY);
		SelectObject(mem, old);
		let _ = DeleteObject(HGDIOBJ(bitmap.0));
		let _ = DeleteDC(mem);
	}
}

fn blob(dc: HDC, x: i32, y: i32, w: i32, h: i32, colour: COLORREF) {
	let (r, g, b) = ((colour.0 & 0xFF) as u8, (colour.0 >> 8 & 0xFF) as u8, (colour.0 >> 16 & 0xFF) as u8);
	if let Some(shape) = gdi::round_rect(w, h, w.min(h) as f32 / 2.0, (r, g, b)) {
		shape.draw(dc, x, y);
	}
}

fn ring(dc: HDC, x: i32, y: i32, w: i32, h: i32, colour: COLORREF, inside: COLORREF) {
	blob(dc, x, y, w, h, colour);
	blob(dc, x + 2, y + 2, w - 4, h - 4, inside);
}

#[allow(clippy::too_many_arguments)]
fn paint_nav(
	dc: HDC,
	width: i32,
	height: i32,
	text: &str,
	fonts: &Fonts,
	selected: bool,
	focused: bool,
	background: Colour,
) {
	let c = colours(background, selected && !theme::use_custom_colours());
	gdi::fill(dc, rect(0, 0, width, height), c.background);
	let (icon, name, unread) = nav_look(text);
	let pill_w = (fonts.line * 3).min(width - 8);
	let pill_h = fonts.line * 3 / 2 + 4;
	let px = (width - pill_w) / 2;
	let py = 6;
	if c.custom && selected {
		if focused {
			ring(dc, px - 2, py - 2, pill_w + 4, pill_h + 4, c.focus, c.background);
		}
		blob(dc, px, py, pill_w, pill_h, c.pressed);
	} else if !c.custom && focused {
		gdi::fill(dc, rect(0, 0, 3, height), c.focus);
	}
	let icon_rect = rect(px, py, px + pill_w, py + pill_h);
	match (icon, fonts.icons_big) {
		(Some(g), Some(font)) => gdi::text(dc, font, c.text, &g.to_string(), icon_rect, DT_CENTER),
		_ => gdi::text(dc, fonts.bold, c.text, &name.chars().take(2).collect::<String>(), icon_rect, DT_CENTER),
	}
	if unread > 0 {
		let count = if unread > 99 { "99+".to_string() } else { unread.to_string() };
		let h = fonts.line * 4 / 5 + 2;
		let w = (gdi::text_width(dc, fonts.small_bold, &count) + h / 2).max(h);
		let (bx, by) = ((px + pill_w - w + 4).min(width - w - 1), py - 2);
		let (fill, ink) =
			if c.custom { (rgb(Palette::ACCENT()), rgb(Palette::ACCENT_TEXT())) } else { (c.text, c.background) };
		blob(dc, bx, by.max(0), w, h, fill);
		gdi::text(dc, fonts.small_bold, ink, &count, rect(bx, by.max(0), bx + w, by.max(0) + h), DT_CENTER);
	}
	let font = if selected { fonts.small_bold } else { fonts.small };
	gdi::text(dc, font, c.text, name, rect(2, py + pill_h + 2, width - 2, height - 2), DT_CENTER);
}

fn paint_icon(
	dc: HDC,
	width: i32,
	height: i32,
	label: &str,
	fonts: &Fonts,
	(pressed, focused, disabled): (bool, bool, bool),
	background: Colour,
) {
	let mut c = colours(background, false);
	// On a dark stage (status viewer, video player) in either theme: light
	// icons, as WhatsApp draws them there.
	if c.custom && is_dark(background) {
		c.text = colorref(0xE9, 0xED, 0xEE);
		c.muted = colorref(0x6B, 0x7A, 0x84);
		c.pressed = colorref(0x3B, 0x4A, 0x54);
		c.focus = colorref(0xE9, 0xED, 0xEE);
	}
	gdi::fill(dc, rect(0, 0, width, height), c.background);
	let side = width.min(height) - 4;
	let (x, y) = ((width - side) / 2, (height - side) / 2);
	if focused {
		ring(dc, x, y, side, side, c.focus, if pressed { c.pressed } else { c.background });
	} else if pressed {
		blob(dc, x, y, side, side, c.pressed);
	}
	let send = matches!(label, "Send message" | "Send voice message" | "Send reply");
	let colour = if disabled {
		c.muted
	} else if send && c.custom {
		rgb(Palette::ACCENT())
	} else {
		c.text
	};
	match (button_glyph(label), fonts.icons) {
		(Some(g), Some(font)) => gdi::text(dc, font, colour, &g.to_string(), rect(0, 0, width, height), DT_CENTER),
		_ => {
			// No icon (the player's "Speed 1.5x"): the words, shortest form.
			let short = label.strip_prefix("Speed ").unwrap_or(label).replace('&', "");
			gdi::text(dc, fonts.body, colour, &short, rect(2, 0, width - 2, height), DT_CENTER)
		}
	}
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Tone {
	Plain,
	On,
	Go,
	Stop,
}

fn tile_look(label: &str) -> (Option<char>, &str, Tone) {
	match label {
		"Accept call" => (Some(glyph::CALLS), "Accept", Tone::Go),
		"Reject call" => (Some(glyph::HANG_UP), "Decline", Tone::Stop),
		"Leave call" => (Some(glyph::HANG_UP), "End", Tone::Stop),
		"Mute microphone" => (Some(glyph::MIC), "Mute", Tone::Plain),
		"Unmute microphone" => (Some(glyph::MIC_OFF), "Unmute", Tone::On),
		"Turn camera on" => (Some(glyph::VIDEO), "Camera", Tone::Plain),
		"Turn camera off" => (Some(glyph::VIDEO), "Camera", Tone::On),
		"Share screen" => (Some(glyph::SCREEN), "Share", Tone::Plain),
		"Stop sharing screen" => (Some(glyph::SCREEN), "Sharing", Tone::On),
		"Add participant" => (Some(glyph::ADD_PERSON), "Add", Tone::Plain),
		"Raise hand" => (Some(glyph::HAND), "Raise hand", Tone::Plain),
		"Lower hand" => (Some(glyph::HAND), "Lower hand", Tone::On),
		"React..." => (Some(glyph::EMOJI), "React", Tone::Plain),
		"Voice call" => (Some(glyph::CALLS), "Audio", Tone::Plain),
		"Call" => (Some(glyph::CALLS), "Voice call", Tone::Plain),
		"Video call" => (Some(glyph::VIDEO), "Video", Tone::Plain),
		"Edit contact" => (Some(glyph::EDIT), "Edit", Tone::Plain),
		"Add contact" => (Some(glyph::ADD_PERSON), "Add", Tone::Plain),
		"Delete contact" => (Some(glyph::DELETE), "Delete", Tone::Stop),
		"Advanced chat privacy..." => (Some(glyph::LOCK), "Privacy", Tone::Plain),
		"Add member" => (Some(glyph::ADD_PERSON), "Add", Tone::Plain),
		"Make admin" => (Some(glyph::ADMIN), "Make admin", Tone::Plain),
		"Dismiss as admin" => (Some(glyph::ADMIN), "Dismiss admin", Tone::On),
		"Remove member" => (Some(glyph::REMOVE_PERSON), "Remove", Tone::Stop),
		"Invite link..." => (Some(glyph::LINK), "Invite link", Tone::Plain),
		"Join requests..." => (Some(glyph::COMMUNITIES), "Requests", Tone::Plain),
		other => (None, other, Tone::Plain),
	}
}

fn is_dark(colour: Colour) -> bool {
	(colour.r as u32 * 299 + colour.g as u32 * 587 + colour.b as u32 * 114) / 1000 < 110
}

fn paint_tile(
	dc: HDC,
	width: i32,
	height: i32,
	label: &str,
	fonts: &Fonts,
	(pressed, focused, disabled): (bool, bool, bool),
	background: Colour,
) {
	let c = colours(background, false);
	gdi::fill(dc, rect(0, 0, width, height), c.background);
	let (icon, caption, tone) = tile_look(label);
	let dark = c.custom && is_dark(background);
	let side = (fonts.line * 5 / 2 + 4).min(width - 6).min(height - fonts.line - 10);
	let (x, y) = ((width - side) / 2, 4);
	let (fill, ink) = if !c.custom {
		if tone == Tone::On { (c.text, c.background) } else { (c.background, c.text) }
	} else {
		match tone {
			Tone::Go => (rgb(Palette::ACCENT()), rgb(Palette::ACCENT_TEXT())),
			Tone::Stop => (rgb(Palette::DANGER()), colorref(0xFF, 0xFF, 0xFF)),
			Tone::On if dark => (colorref(0xFF, 0xFF, 0xFF), colorref(0x11, 0x1B, 0x21)),
			Tone::On => (c.text, colorref(0xFF, 0xFF, 0xFF)),
			Tone::Plain if dark => (colorref(0x3B, 0x4A, 0x54), colorref(0xFF, 0xFF, 0xFF)),
			Tone::Plain => (colorref(0xFF, 0xFF, 0xFF), rgb(Palette::ACCENT())),
		}
	};
	if focused {
		let ring_colour = if dark { colorref(0xFF, 0xFF, 0xFF) } else { c.focus };
		ring(dc, x - 3, y - 3, side + 6, side + 6, ring_colour, c.background);
	}
	let fill = if pressed && c.custom { c.pressed } else { fill };
	blob(dc, x, y, side, side, fill);
	if !c.custom {
		ring(dc, x, y, side, side, c.text, fill);
	}
	let ink = if disabled { c.muted } else { ink };
	let circle = rect(x, y, x + side, y + side);
	match (icon, fonts.icons_big) {
		(Some(g), Some(font)) if ('\u{E000}'..='\u{F8FF}').contains(&g) => {
			gdi::text(dc, font, ink, &g.to_string(), circle, DT_CENTER)
		}
		(Some(g), _) => gdi::text(dc, fonts.bold, ink, &g.to_string(), circle, DT_CENTER),
		_ => gdi::text(dc, fonts.small_bold, ink, caption, circle, DT_CENTER),
	}
	let text = if dark {
		colorref(0xE9, 0xED, 0xEF)
	} else if disabled {
		c.muted
	} else {
		c.text
	};
	gdi::text(dc, fonts.small, text, caption, rect(0, y + side + 4, width, height), DT_CENTER);
}

fn split_header(label: &str) -> (&str, &str) {
	if let Some(open) = label.rfind(" (")
		&& label.ends_with(')')
	{
		return (&label[..open], &label[open + 2..label.len() - 1]);
	}
	if (label.ends_with("is typing...") || label.ends_with("is recording audio..."))
		&& let Some((name, status)) = label.rsplit_once(", ")
	{
		return (name, status);
	}
	(label, "")
}

#[allow(clippy::too_many_arguments)]
fn paint_header(
	dc: HDC,
	width: i32,
	height: i32,
	label: &str,
	fonts: &Fonts,
	pressed: bool,
	focused: bool,
	background: Colour,
) {
	let c = colours(background, false);
	gdi::fill(dc, rect(0, 0, width, height), if pressed { c.pressed } else { c.background });
	if focused {
		gdi::fill(dc, rect(0, 2, 3, height - 2), c.focus);
	}
	let pad = 8;
	let size = avatars::size().unwrap_or(height - 8).min(height - 4);
	let top = (height - size) / 2;
	let jid = REGISTRY.with(|r| r.borrow().header_jid.clone());
	let drawn = avatars::with_avatar(&jid, size, |avatar| match avatar {
		Avatar::Picture(bitmap) => {
			bitmap.draw(dc, pad, top);
			true
		}
		Avatar::Nothing | Avatar::Waiting => false,
	});
	if !drawn {
		let circle = if c.custom { theme::tuple(Palette::AVATAR_BG()) } else { (0x80, 0x80, 0x80) };
		if let Some(circle) = gdi::round_rect(size, size, size as f32 / 2.0, circle) {
			circle.draw(dc, pad, top);
		}
		let g = if jid.ends_with("@g.us") { '\u{E716}' } else { '\u{E77B}' };
		if let Some(font) = fonts.icons_big {
			gdi::text(
				dc,
				font,
				colorref(0xFF, 0xFF, 0xFF),
				&g.to_string(),
				rect(pad, top, pad + size, top + size),
				DT_CENTER,
			);
		}
	}
	let (name, status) = split_header(label);
	let x = pad + size + 12;
	let typing = status.contains("typing") || status.contains("recording");
	if status.is_empty() {
		gdi::text(dc, fonts.bold, c.text, name, rect(x, 0, width - pad, height), Default::default());
	} else {
		let mid = height / 2;
		gdi::text(
			dc,
			fonts.bold,
			c.text,
			name,
			rect(x, mid - fonts.line - 1, width - pad, mid + 1),
			Default::default(),
		);
		let colour = if typing && c.custom { rgb(Palette::ACCENT()) } else { c.muted };
		gdi::text(
			dc,
			fonts.small,
			colour,
			status,
			rect(x, mid + 1, width - pad, mid + fonts.line + 2),
			Default::default(),
		);
	}
}

#[cfg(test)]
mod tests {
	use super::split_header;

	#[test]
	fn a_tab_with_unread_chats_keeps_its_icon_and_shows_the_count() {
		assert_eq!(super::nav_look("Chats: 5 unread"), (Some(super::glyph::CHATS), "Chats", 5));
		assert_eq!(super::nav_look("Calls"), (Some(super::glyph::CALLS), "Calls", 0));
	}

	#[test]
	fn call_buttons_show_their_state() {
		assert_eq!(super::tile_look("Unmute microphone").2, super::Tone::On);
		assert_eq!(super::tile_look("Mute microphone").2, super::Tone::Plain);
		assert_eq!(super::tile_look("Leave call").2, super::Tone::Stop);
		assert_eq!(super::tile_look("Accept call").1, "Accept");
	}

	#[test]
	fn the_header_splits_name_and_status() {
		assert_eq!(split_header("Mum (online)"), ("Mum", "online"));
		assert_eq!(split_header("Mum (last seen today at 10:02)"), ("Mum", "last seen today at 10:02"));
		assert_eq!(split_header("Family, Dad is typing..."), ("Family", "Dad is typing..."));
		assert_eq!(split_header("Family"), ("Family", ""));
	}
}
