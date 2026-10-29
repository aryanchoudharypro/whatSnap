//! Native controls the app paints itself, WhatsApp-style, while they stay the
//! same controls to a screen reader: the navigation strip's rows (an icon over
//! a short name), round icon buttons (call, search, emoji, send, ...), and the
//! open chat's header (picture, name, and "online" / "typing..." under it).
//!
//! Each is made owner-drawn (`BS_OWNERDRAW`, or wx's `wxLB_OWNERDRAW` for a
//! list box, which keeps its strings), so its window text - all a screen
//! reader reads - is untouched. Painting happens on the parent's
//! `WM_DRAWITEM`. In a High Contrast theme everything uses system colours.

use std::{cell::RefCell, collections::HashMap};

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
	/// A row of the navigation strip.
	Nav,
	/// A round button showing its label's icon.
	Icon,
	/// The open chat's header button.
	Header,
}

struct Control {
	kind: Kind,
	/// What's behind it, so a round button sits on its bar.
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

#[derive(Default)]
struct Registry {
	controls: HashMap<isize, Control>,
	parents: Vec<isize>,
	fonts: HashMap<isize, Fonts>,
	/// The open chat, for the header's picture.
	header_jid: String,
	header: Option<HWND>,
}

thread_local! {
	static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

/// Icons, from the Windows icon font.
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
}

/// A navigation tab's icon and the short name shown under it.
fn nav_look(text: &str) -> (Option<char>, &str) {
	match text {
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

/// The icon a button with this label shows; None draws the label itself.
fn button_glyph(label: &str) -> Option<char> {
	Some(match label {
		"Search in conversation" => glyph::SEARCH,
		"Call" => glyph::CALLS,
		"Video call" => glyph::VIDEO,
		"Emoji and stickers" => glyph::EMOJI,
		"Add attachment" | "Add more files" => glyph::ADD,
		"Send message" | "Send voice message" => glyph::SEND,
		"Record voice message" => glyph::MIC,
		"Cancel edit" | "Remove quote" | "Close search" => glyph::CLOSE,
		"Discard voice message" => glyph::DELETE,
		"Pause recording" => glyph::PAUSE,
		"Resume recording" => glyph::PLAY,
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

/// Draws the navigation list (built with `chat_rows::OWNER_DRAW`) as
/// WhatsApp's strip of icons, each with its short name under it.
pub(crate) fn install_nav(list: &ListBox) {
	let hwnd = hwnd_of(list);
	register(hwnd, Kind::Nav, Palette::PANEL_BG);
	set_nav_height(hwnd);
}

/// Makes `button` a round icon button, showing the icon for its label (see
/// `button_glyph`), on a bar of `background`. Its label is its tooltip.
pub(crate) fn install_icon_button(button: &Button, background: Colour) {
	let hwnd = hwnd_of(button);
	make_owner_drawn_button(hwnd);
	register(hwnd, Kind::Icon, background);
	button.set_tooltip(&button.get_label());
	let side = button.get_char_height() * 2 + 10;
	button.set_min_size(Size::new(side, side));
}

/// Draws the chat header button (its label is "Name (online)" and the like)
/// with the chat's picture, the name in bold and the status under it.
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

/// Says which chat is open, for the header's picture.
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
	REGISTRY.with(|r| {
		let mut r = r.borrow_mut();
		let key = hwnd.0 as isize;
		if r.fonts.get(&key).is_none_or(|fonts| fonts.base != base) {
			let fonts = make_fonts(hwnd, base);
			r.fonts.insert(key, fonts);
		}
		f(&r.fonts[&key])
	})
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
	let kind = REGISTRY.with(|r| r.borrow().controls.get(&(hwnd.0 as isize)).map(|c| c.kind));
	match (msg, kind) {
		// wx sets one line's height whenever the font changes; the rows are taller.
		(LB_SETITEMHEIGHT, Some(Kind::Nav)) => {
			let height = with_fonts(hwnd, nav_height);
			unsafe { DefSubclassProc(hwnd, msg, wparam, LPARAM(height as isize)) }
		}
		(WM_SETFONT, Some(Kind::Nav)) => {
			let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
			set_nav_height(hwnd);
			result
		}
		// A new label can mean a new icon (Pause recording / Resume recording).
		(WM_SETTEXT, Some(Kind::Icon | Kind::Header)) => {
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
		let control =
			REGISTRY.with(|r| r.borrow().controls.get(&(item.hwndItem.0 as isize)).map(|c| (c.kind, c.background)));
		if let Some((kind, background)) = control {
			let fits = match kind {
				Kind::Nav => item.CtlType == ODT_LISTBOX && item.itemID != u32::MAX,
				Kind::Icon | Kind::Header => item.CtlType == ODT_BUTTON,
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
	/// Under a selected tab or a pressed button.
	pressed: COLORREF,
	/// The keyboard's place: a ring or a bar.
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
		// WhatsApp Web's icon grey, dark enough to read on its bars.
		text: colorref(0x3B, 0x4A, 0x54),
		muted: rgb(Palette::TEXT_MUTED),
		pressed: colorref(0xD9, 0xDB, 0xDF),
		focus: rgb(Palette::ACCENT),
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
		Kind::Nav => (unsafe { GetFocus() }) == hwnd,
		Kind::Icon | Kind::Header => item.itemState.0 & ODS_FOCUS.0 != 0,
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

/// A filled circle (or pill) `w` by `h` at (x, y).
fn blob(dc: HDC, x: i32, y: i32, w: i32, h: i32, colour: COLORREF) {
	let (r, g, b) = ((colour.0 & 0xFF) as u8, (colour.0 >> 8 & 0xFF) as u8, (colour.0 >> 16 & 0xFF) as u8);
	if let Some(shape) = gdi::round_rect(w, h, w.min(h) as f32 / 2.0, (r, g, b)) {
		shape.draw(dc, x, y);
	}
}

/// A ring 2px wide, for the keyboard focus.
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
	let (icon, name) = nav_look(text);
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
	let font = if selected { fonts.small_bold } else { fonts.small };
	gdi::text(dc, font, c.text, name, rect(2, py + pill_h + 2, width - 2, height - 2), DT_CENTER);
}

/// `state` is (pressed, focused, disabled).
fn paint_icon(
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
	let side = width.min(height) - 4;
	let (x, y) = ((width - side) / 2, (height - side) / 2);
	if focused {
		ring(dc, x, y, side, side, c.focus, if pressed { c.pressed } else { c.background });
	} else if pressed {
		blob(dc, x, y, side, side, c.pressed);
	}
	// The send arrow is WhatsApp's green.
	let send = matches!(label, "Send message" | "Send voice message");
	let colour = if disabled {
		c.muted
	} else if send && c.custom {
		rgb(Palette::ACCENT)
	} else {
		c.text
	};
	match (button_glyph(label), fonts.icons) {
		(Some(g), Some(font)) => gdi::text(dc, font, colour, &g.to_string(), rect(0, 0, width, height), DT_CENTER),
		_ => gdi::text(dc, fonts.body, colour, label, rect(2, 0, width - 2, height), DT_CENTER),
	}
}

/// "Mum (online)" -> ("Mum", "online"); "Family, Dad is typing..." ->
/// ("Family", "Dad is typing...").
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
		let circle = if c.custom { (0xDF, 0xE5, 0xE7) } else { (0x80, 0x80, 0x80) };
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
		let colour = if typing && c.custom { rgb(Palette::ACCENT) } else { c.muted };
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
	fn the_header_splits_name_and_status() {
		assert_eq!(split_header("Mum (online)"), ("Mum", "online"));
		assert_eq!(split_header("Mum (last seen today at 10:02)"), ("Mum", "last seen today at 10:02"));
		assert_eq!(split_header("Family, Dad is typing..."), ("Family", "Dad is typing..."));
		assert_eq!(split_header("Family"), ("Family", ""));
	}
}
