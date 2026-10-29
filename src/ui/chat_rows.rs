//! Draws the chat list's rows the way WhatsApp Web does: the profile picture,
//! the name in bold with the time on the right, and below it the last
//! message with an icon for its kind (voice message, photo, ...), the unread
//! count and the pinned / muted marks.
//!
//! The list stays the native list box. It's made owner-drawn (wx's
//! `wxLB_OWNERDRAW`, which keeps `LBS_HASSTRINGS`), so every row still holds
//! its full text, and that text is all a screen reader is given: names,
//! order, selection and keys are exactly what they were. Only the painting is
//! ours, done here on the parent's `WM_DRAWITEM`.
//!
//! In a High Contrast theme the rows keep the pictures but use the system's
//! colours throughout (see `theme::use_custom_colours`).

use std::cell::RefCell;

use windows::Win32::{
	Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
	Graphics::Gdi::{
		BitBlt, COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT,
		CreateCompatibleBitmap, CreateCompatibleDC, DT_CENTER, DT_RIGHT, DeleteDC, DeleteObject, GetDC, GetObjectW,
		GetSysColor, GetTextMetricsW, HDC, HFONT, HGDIOBJ, InvalidateRect, LOGFONTW, ReleaseDC, SRCCOPY, SelectObject,
		SetBkMode, TEXTMETRICW, TRANSPARENT,
	},
	UI::{
		Controls::DRAWITEMSTRUCT,
		Input::KeyboardAndMouse::GetFocus,
		Shell::{DefSubclassProc, SetWindowSubclass},
		WindowsAndMessaging::{
			GetParent, LB_GETTEXT, LB_GETTEXTLEN, LB_SETITEMHEIGHT, SendMessageW, WM_DRAWITEM, WM_GETFONT, WM_SETFONT,
			WM_SIZE,
		},
	},
};
use wxdragon::prelude::*;

use super::{
	avatars::{self, Avatar},
	gdi::{self, colorref},
	theme::{self, Palette},
};

/// wx's `wxLB_OWNERDRAW`, which wxdragon has no name for.
pub(crate) const OWNER_DRAW: i64 = 0x0100;
const SUBCLASS_ID: usize = 0x5752_4f57;

/// Icons, from the Windows icon font (Segoe Fluent Icons / MDL2 Assets).
pub(crate) mod icon {
	pub const MICROPHONE: char = '\u{E720}';
	pub const CAMERA: char = '\u{E722}';
	pub const VIDEO: char = '\u{E714}';
	pub const DOCUMENT: char = '\u{E8A5}';
	pub const AUDIO: char = '\u{E8D6}';
	pub const LOCATION: char = '\u{E707}';
	pub const CONTACT: char = '\u{E77B}';
	pub const GROUP: char = '\u{E716}';
	pub const POLL: char = '\u{E8FD}';
	pub const STICKER: char = '\u{E76E}';
	pub const DELETED: char = '\u{E711}';
	pub const PIN: char = '\u{E718}';
	pub const MUTED: char = '\u{E74F}';
	pub const LOCK: char = '\u{E72E}';
}

/// What a chat row shows, besides its text.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct RowArt {
	pub jid: String,
	pub title: String,
	/// Tags with no icon of their own ("Community", "In Family").
	pub extra: String,
	pub time: String,
	/// "You: " or "Name: " before the message.
	pub preview_prefix: String,
	pub icon: Option<char>,
	pub preview: String,
	/// Your own last message's ticks, drawn before it.
	pub ticks: Option<super::message_art::Ticks>,
	pub unread: u32,
	pub pinned: bool,
	pub muted: bool,
	pub locked: bool,
}

impl RowArt {
	/// For a row that only has its text (Status, Starred, Calls): the first
	/// part is the title, the rest the line under it.
	pub(crate) fn from_text(jid: &str, text: &str) -> Self {
		let mut parts = text.split(" | ");
		let title = parts.next().unwrap_or_default().to_string();
		let rest: Vec<&str> = parts.collect();
		Self { jid: jid.to_string(), title, preview: rest.join(" · "), ..Default::default() }
	}
}

/// Splits a chat's raw last-message text ("[Voice Message]", "[Image] hi")
/// into an icon and what follows it, as WhatsApp shows "🎤 Voice message".
pub(crate) fn preview_icon(raw: &str) -> (Option<char>, String) {
	let friendly = whatsnap::friendly_preview_text(raw);
	let icon = [
		("[Voice Message]", icon::MICROPHONE),
		("[Audio]", icon::AUDIO),
		("[Image]", icon::CAMERA),
		("[Photo]", icon::CAMERA),
		("[Video]", icon::VIDEO),
		("[Document]", icon::DOCUMENT),
		("[Live Location]", icon::LOCATION),
		("[Location]", icon::LOCATION),
		("[Contact]", icon::CONTACT),
		("[Poll]", icon::POLL),
		("[Quiz]", icon::POLL),
		("[Sticker]", icon::STICKER),
		("[Message deleted]", icon::DELETED),
	]
	.iter()
	.find(|(tag, _)| raw.starts_with(tag))
	.map(|(_, icon)| *icon);
	// "Photo: caption" shows as the camera and the caption alone.
	let text = match (icon, friendly.split_once(": ")) {
		(Some(_), Some((_, rest))) if !rest.is_empty() => rest.to_string(),
		_ => friendly,
	};
	(icon, text)
}

type Source = Box<dyn Fn(usize, &str) -> Option<RowArt>>;

struct Fonts {
	base: HFONT,
	title: HFONT,
	body: HFONT,
	small: HFONT,
	small_bold: HFONT,
	icons: Option<HFONT>,
	icons_big: Option<HFONT>,
	line: i32,
}

impl Drop for Fonts {
	fn drop(&mut self) {
		for font in
			[self.title, self.body, self.small, self.small_bold].into_iter().chain(self.icons).chain(self.icons_big)
		{
			gdi::delete_font(font);
		}
	}
}

struct Layout {
	pad: i32,
	avatar: i32,
	row: i32,
}

impl Layout {
	fn for_line(line: i32) -> Self {
		let pad = (line * 3 / 5).max(4);
		let avatar = line * 5 / 2;
		Self { pad, avatar, row: avatar + 2 * pad }
	}
}

#[derive(Default)]
struct Rows {
	list: Option<HWND>,
	source: Option<Source>,
	fonts: Option<Fonts>,
}

thread_local! {
	static ROWS: RefCell<Rows> = RefCell::new(Rows::default());
}

/// Takes over painting `list`'s rows. `list` must have been built with
/// [`OWNER_DRAW`]. `source` gives a row's art from its index and text, or
/// None to draw it from its text alone.
pub(crate) fn install(list: &ListBox, source: impl Fn(usize, &str) -> Option<RowArt> + 'static) {
	let hwnd = HWND(list.get_handle().cast());
	ROWS.with(|r| {
		let mut r = r.borrow_mut();
		r.list = Some(hwnd);
		r.source = Some(Box::new(source));
	});
	unsafe {
		let parent = GetParent(hwnd).unwrap_or_default();
		let _ = SetWindowSubclass(parent, Some(parent_proc), SUBCLASS_ID, 0);
		let _ = SetWindowSubclass(hwnd, Some(list_proc), SUBCLASS_ID, 0);
	}
	set_row_height(hwnd);
	avatars::on_change(move || unsafe {
		let _ = InvalidateRect(Some(hwnd), None, false);
	});
}

/// Redraws every row, for when the art behind them changed but their text
/// didn't (a chat's picture, say).
pub(crate) fn refresh() {
	if let Some(hwnd) = ROWS.with(|r| r.borrow().list) {
		unsafe {
			let _ = InvalidateRect(Some(hwnd), None, false);
		}
	}
}

fn set_row_height(hwnd: HWND) {
	let height = with_fonts(hwnd, |fonts| Layout::for_line(fonts.line).row);
	unsafe {
		SendMessageW(hwnd, LB_SETITEMHEIGHT, Some(WPARAM(0)), Some(LPARAM(height as isize)));
	}
}

/// Runs `f` with fonts made from the list's own font, (re)making them when
/// that font changed.
fn with_fonts<R>(hwnd: HWND, f: impl FnOnce(&Fonts) -> R) -> R {
	let base = HFONT(unsafe { SendMessageW(hwnd, WM_GETFONT, None, None) }.0 as *mut _);
	ROWS.with(|r| {
		let mut r = r.borrow_mut();
		if r.fonts.as_ref().is_none_or(|fonts| fonts.base != base) {
			r.fonts = Some(make_fonts(hwnd, base));
		}
		f(r.fonts.as_ref().expect("fonts were just made"))
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
		title: gdi::derived_font(&lf, 1.08, 600),
		body,
		small: gdi::derived_font(&lf, 0.86, 400),
		small_bold: gdi::derived_font(&lf, 0.86, 700),
		icons: gdi::icon_font(dc, &lf, 1.0),
		icons_big: gdi::icon_font(dc, &lf, 1.35),
		line: tm.tmHeight.max(8),
	};
	unsafe { ReleaseDC(Some(hwnd), dc) };
	fonts
}

unsafe extern "system" fn list_proc(
	hwnd: HWND,
	msg: u32,
	wparam: WPARAM,
	lparam: LPARAM,
	_id: usize,
	_data: usize,
) -> LRESULT {
	match msg {
		// wx sets the height for one line of text whenever the font changes;
		// the rows are taller.
		LB_SETITEMHEIGHT => {
			let height = with_fonts(hwnd, |fonts| Layout::for_line(fonts.line).row);
			unsafe { DefSubclassProc(hwnd, msg, wparam, LPARAM(height as isize)) }
		}
		WM_SETFONT => {
			let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
			set_row_height(hwnd);
			result
		}
		// The time sits at the right edge, so a new width moves every row.
		WM_SIZE => {
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
		let ours = ROWS.with(|r| r.borrow().list) == Some(item.hwndItem);
		if ours && item.itemID != u32::MAX {
			draw_item(item);
			return LRESULT(1);
		}
	}
	unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

fn item_text(hwnd: HWND, index: u32) -> String {
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

struct Colours {
	background: windows::Win32::Foundation::COLORREF,
	text: windows::Win32::Foundation::COLORREF,
	muted: windows::Win32::Foundation::COLORREF,
	accent: windows::Win32::Foundation::COLORREF,
	read: windows::Win32::Foundation::COLORREF,
	divider: Option<windows::Win32::Foundation::COLORREF>,
	focus_bar: Option<windows::Win32::Foundation::COLORREF>,
}

fn rgb(colour: Colour) -> windows::Win32::Foundation::COLORREF {
	colorref(colour.r, colour.g, colour.b)
}

fn colours(selected: bool, focused: bool) -> Colours {
	if !theme::use_custom_colours() {
		let sys = |i| windows::Win32::Foundation::COLORREF(unsafe { GetSysColor(i) });
		return if selected {
			let text = sys(COLOR_HIGHLIGHTTEXT);
			Colours {
				background: sys(COLOR_HIGHLIGHT),
				text,
				muted: text,
				accent: text,
				read: text,
				divider: None,
				focus_bar: None,
			}
		} else {
			let text = sys(COLOR_WINDOWTEXT);
			Colours {
				background: sys(COLOR_WINDOW),
				text,
				muted: sys(COLOR_GRAYTEXT),
				accent: text,
				read: text,
				divider: None,
				focus_bar: None,
			}
		};
	}
	Colours {
		// WhatsApp Web's grey for the open chat, a little darker so it shows.
		background: if selected { colorref(0xE4, 0xE8, 0xEA) } else { rgb(Palette::CONTENT_BG) },
		text: rgb(Palette::TEXT_PRIMARY),
		muted: rgb(Palette::TEXT_MUTED),
		accent: rgb(Palette::ACCENT),
		// WhatsApp's read blue, darkened to read on white.
		read: colorref(0x0B, 0x6F, 0xA4),
		divider: Some(colorref(0xE3, 0xE6, 0xE8)),
		// Where the keyboard is, beyond the grey: a bar in the accent green.
		focus_bar: (selected && focused).then(|| rgb(Palette::ACCENT)),
	}
}

fn draw_item(item: &DRAWITEMSTRUCT) {
	let hwnd = item.hwndItem;
	let text = item_text(hwnd, item.itemID);
	let art = ROWS
		.with(|r| r.try_borrow().ok().and_then(|r| r.source.as_ref().map(|source| source(item.itemID as usize, &text))))
		.flatten()
		.unwrap_or_else(|| RowArt::from_text("", &text));
	let selected = item.itemState.0 & windows::Win32::UI::Controls::ODS_SELECTED.0 != 0;
	let focused = unsafe { GetFocus() } == hwnd;
	let rect = item.rcItem;
	let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
	if width <= 0 || height <= 0 {
		return;
	}
	// Drawn off screen and copied in one go, so a row never flickers.
	unsafe {
		let mem = CreateCompatibleDC(Some(item.hDC));
		let bitmap = CreateCompatibleBitmap(item.hDC, width, height);
		let old = SelectObject(mem, HGDIOBJ(bitmap.0));
		SetBkMode(mem, TRANSPARENT);
		with_fonts(hwnd, |fonts| paint_row(mem, width, height, &art, fonts, colours(selected, focused)));
		let _ = BitBlt(item.hDC, rect.left, rect.top, width, height, Some(mem), 0, 0, SRCCOPY);
		SelectObject(mem, old);
		let _ = DeleteObject(HGDIOBJ(bitmap.0));
		let _ = DeleteDC(mem);
	}
}

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
	RECT { left, top, right, bottom }
}

fn paint_row(dc: HDC, width: i32, height: i32, art: &RowArt, fonts: &Fonts, c: Colours) {
	let layout = Layout::for_line(fonts.line);
	let (pad, size) = (layout.pad, layout.avatar);
	gdi::fill(dc, rect(0, 0, width, height), c.background);
	if let Some(bar) = c.focus_bar {
		gdi::fill(dc, rect(0, 0, (pad / 3).max(3), height), bar);
	}

	// The picture, or WhatsApp's grey silhouette.
	let avatar_top = (height - size) / 2;
	let drawn = avatars::with_avatar(&art.jid, size, |avatar| match avatar {
		Avatar::Picture(bitmap) => {
			bitmap.draw(dc, pad, avatar_top);
			true
		}
		Avatar::Nothing | Avatar::Waiting => false,
	});
	if !drawn {
		let custom = theme::use_custom_colours();
		let circle = if custom { (0xDF, 0xE5, 0xE7) } else { (0x80, 0x80, 0x80) };
		if let Some(circle) = gdi::round_rect(size, size, size as f32 / 2.0, circle) {
			circle.draw(dc, pad, avatar_top);
		}
		let glyph = if art.jid.ends_with("@g.us") { icon::GROUP } else { icon::CONTACT };
		if let Some(font) = fonts.icons_big {
			let white = colorref(0xFF, 0xFF, 0xFF);
			gdi::text(
				dc,
				font,
				white,
				&glyph.to_string(),
				rect(pad, avatar_top, pad + size, avatar_top + size),
				DT_CENTER,
			);
		}
	}

	let left = pad + size + pad;
	let right = width - pad;
	let mid = avatar_top + size / 2;
	let line1 = (avatar_top, mid);
	let line2 = (mid, avatar_top + size);
	let gap = (pad / 2).max(4);

	// Line one: name (and tags), time on the right.
	let time_w = gdi::text_width(dc, fonts.small, &art.time);
	let time_colour = if art.unread > 0 { c.accent } else { c.muted };
	gdi::text(dc, fonts.small, time_colour, &art.time, rect(right - time_w, line1.0, right, line1.1), DT_RIGHT);
	let name_right = if time_w > 0 { right - time_w - gap } else { right };
	let name_w = gdi::text_width(dc, fonts.title, &art.title);
	gdi::text(dc, fonts.title, c.text, &art.title, rect(left, line1.0, name_right, line1.1), Default::default());
	if !art.extra.is_empty() && left + name_w + gap < name_right {
		gdi::text(
			dc,
			fonts.small,
			c.muted,
			&art.extra,
			rect(left + name_w + gap, line1.0, name_right, line1.1),
			Default::default(),
		);
	}

	// Line two, from the right: unread count, then pinned / muted / locked.
	let mut line_right = right;
	if art.unread > 0 {
		let count = if art.unread > 999 { "999+".to_string() } else { art.unread.to_string() };
		let badge_h = (fonts.line * 6 / 5).min(line2.1 - line2.0);
		let badge_w = (gdi::text_width(dc, fonts.small_bold, &count) + badge_h * 3 / 5).max(badge_h);
		let top = line2.0 + (line2.1 - line2.0 - badge_h) / 2;
		let (fill, ink) = if theme::use_custom_colours() {
			((Palette::ACCENT.r, Palette::ACCENT.g, Palette::ACCENT.b), rgb(Palette::ACCENT_TEXT))
		} else {
			let t = c.text.0;
			let bg = c.background;
			((t as u8, (t >> 8) as u8, (t >> 16) as u8), bg)
		};
		if let Some(pill) = gdi::round_rect(badge_w, badge_h, badge_h as f32 / 2.0, fill) {
			pill.draw(dc, line_right - badge_w, top);
		}
		gdi::text(
			dc,
			fonts.small_bold,
			ink,
			&count,
			rect(line_right - badge_w, top, line_right, top + badge_h),
			DT_CENTER,
		);
		line_right -= badge_w + gap;
	}
	if let Some(font) = fonts.icons {
		for (on, glyph) in [(art.pinned, icon::PIN), (art.muted, icon::MUTED), (art.locked, icon::LOCK)] {
			if !on {
				continue;
			}
			let glyph = glyph.to_string();
			let w = gdi::text_width(dc, font, &glyph);
			gdi::text(
				dc,
				font,
				c.muted,
				&glyph,
				rect(line_right - w, line2.0, line_right, line2.1),
				Default::default(),
			);
			line_right -= w + gap;
		}
	}

	// Then the last message: your ticks or "Name: ", its kind's icon, its text.
	let mut x = left;
	if let (Some(ticks), Some(font)) = (art.ticks, fonts.icons) {
		let (glyphs, blue) = super::message_art::tick_glyphs(ticks);
		let colour = if blue { c.read } else { c.muted };
		let mut chars = glyphs.chars();
		if let Some(first) = chars.next() {
			let first = first.to_string();
			let w = gdi::text_width(dc, font, &first);
			gdi::text(dc, font, colour, &first, rect(x, line2.0, line_right, line2.1), Default::default());
			// A second tick overlaps the first, as WhatsApp draws them.
			let total = if let Some(second) = chars.next() {
				gdi::text(
					dc,
					font,
					colour,
					&second.to_string(),
					rect(x + w / 3, line2.0, line_right, line2.1),
					Default::default(),
				);
				w + w / 3
			} else {
				w
			};
			x += total + gap / 2;
		}
	}
	if !art.preview_prefix.is_empty() {
		gdi::text(
			dc,
			fonts.body,
			c.muted,
			&art.preview_prefix,
			rect(x, line2.0, line_right, line2.1),
			Default::default(),
		);
		x += gdi::text_width(dc, fonts.body, &art.preview_prefix);
	}
	if let (Some(glyph), Some(font)) = (art.icon, fonts.icons)
		&& x < line_right
	{
		let glyph = glyph.to_string();
		gdi::text(dc, font, c.muted, &glyph, rect(x, line2.0, line_right, line2.1), Default::default());
		x += gdi::text_width(dc, font, &glyph) + gap / 2;
	}
	if x < line_right {
		gdi::text(dc, fonts.body, c.muted, &art.preview, rect(x, line2.0, line_right, line2.1), Default::default());
	}

	if let Some(divider) = c.divider {
		gdi::fill(dc, rect(left, height - 1, width, height), divider);
	}
}

#[cfg(test)]
mod tests {
	use super::{RowArt, icon, preview_icon};

	#[test]
	fn a_voice_message_gets_the_microphone() {
		assert_eq!(preview_icon("[Voice Message]"), (Some(icon::MICROPHONE), "Voice message".to_string()));
	}

	#[test]
	fn a_photo_with_a_caption_shows_just_the_caption() {
		assert_eq!(preview_icon("[Image] at the beach"), (Some(icon::CAMERA), "at the beach".to_string()));
		assert_eq!(preview_icon("[Image]"), (Some(icon::CAMERA), "Photo".to_string()));
	}

	#[test]
	fn plain_text_has_no_icon() {
		assert_eq!(preview_icon("see you: soon"), (None, "see you: soon".to_string()));
	}

	#[test]
	fn a_text_row_splits_into_title_and_the_rest() {
		let art = RowArt::from_text("x", "Mum | 10:02 AM | Photo");
		assert_eq!(art.title, "Mum");
		assert_eq!(art.preview, "10:02 AM · Photo");
	}
}
