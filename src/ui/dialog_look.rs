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
		Controls::{
			CDDS_PREPAINT, CDIS_DISABLED, CDIS_FOCUS, CDRF_SKIPDEFAULT, DRAWITEMSTRUCT, NM_CUSTOMDRAW, NMCUSTOMDRAW,
			NMHDR, ODS_DISABLED, ODS_FOCUS, ODS_SELECTED, ODT_BUTTON, ODT_LISTBOX,
		},
		Input::KeyboardAndMouse::{GetFocus, IsWindowEnabled},
		Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
		WindowsAndMessaging::{
			BM_GETCHECK, BS_OWNERDRAW, GWL_STYLE, GetParent, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW,
			LB_GETTEXT, LB_GETTEXTLEN, LB_SETITEMHEIGHT, SendMessageW, SetWindowLongPtrW, WM_DRAWITEM, WM_GETFONT,
			WM_NCDESTROY, WM_NOTIFY, WM_SETTEXT,
		},
	},
};
use wxdragon::prelude::*;

use super::{
	avatars::{self, Avatar},
	gdi::{self, colorref},
	theme::{self, Palette},
};

const PARENT_SUBCLASS: usize = 0x444c_5052;
const CONTROL_SUBCLASS: usize = 0x444c_4354;

const BORDER_NONE: i64 = 0x0020_0000;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Field {
	Line,
	Search,
	Box,
}

pub(crate) fn field_style(style: TextCtrlStyle) -> TextCtrlStyle {
	if theme::use_custom_colours() { style | TextCtrlStyle::from_bits_retain(BORDER_NONE) } else { style }
}

pub(crate) fn add_field(sizer: &BoxSizer, field: &TextCtrl, kind: Field, proportion: i32) {
	field.set_font(&theme::Fonts::body());
	if !theme::use_custom_colours() {
		sizer.add(field, proportion, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);
		return;
	}
	match kind {
		Field::Line => {
			field.set_background_color(Palette::CONTENT_BG());
			sizer.add(field, proportion, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);
		}
		Field::Search | Field::Box => {
			field.set_background_color(Palette::FIELD_BG());
			let row = BoxSizer::builder(Orientation::Horizontal).build();
			row.add_spacer(if kind == Field::Search { 52 } else { 20 });
			row.add(field, 1, SizerFlag::Expand, 0);
			row.add_spacer(20);
			sizer.add_sizer(&row, proportion, SizerFlag::Expand | SizerFlag::Top, 16);
		}
	}
}

pub(crate) fn paint_fields(panel: &Panel, fields: &[(TextCtrl, Field)]) {
	if !theme::use_custom_colours() {
		return;
	}
	let panel_c = *panel;
	let fields: Vec<(TextCtrl, Field)> = fields.to_vec();
	for &(field, _) in &fields {
		field.on_set_focus(move |event| {
			panel_c.refresh(false, None);
			event.skip(true);
		});
		field.on_kill_focus(move |event| {
			panel_c.refresh(false, None);
			event.skip(true);
		});
	}
	panel.set_background_style(BackgroundStyle::Paint);
	panel.on_paint(move |_| {
		let dc = AutoBufferedPaintDC::new(&panel_c);
		let size = panel_c.get_client_size();
		let background = panel_c.get_background_color();
		dc.set_brush(background, BrushStyle::Solid);
		dc.set_pen(background, 1, PenStyle::Solid);
		dc.draw_rectangle(0, 0, size.width, size.height);
		for &(field, kind) in &fields {
			paint_field(&dc, &field, kind);
		}
	});
	panel.on_size(move |event| {
		panel_c.refresh(true, None);
		event.skip(true);
	});
}

pub(crate) fn paint_field(dc: &impl DeviceContext, field: &TextCtrl, kind: Field) {
	if !field.is_shown() {
		return;
	}
	let (p, s) = (field.get_position(), field.get_size());
	let focused = field.has_focus();
	match kind {
		Field::Line => {
			let (colour, thick) = if focused { (Palette::ACCENT(), 2) } else { (Palette::LINE(), 1) };
			dc.set_brush(colour, BrushStyle::Solid);
			dc.set_pen(colour, 1, PenStyle::Solid);
			dc.draw_rectangle(p.x, p.y + s.height + 3, s.width, thick);
		}
		Field::Search | Field::Box => {
			let (left, pad) = if kind == Field::Search { (40, 8) } else { (10, 8) };
			let (x, y, w, h) = (p.x - left, p.y - pad, s.width + left + 10, s.height + pad * 2);
			let radius = if kind == Field::Search { h as f64 / 2.0 } else { 8.0 };
			if focused {
				dc.set_brush(Palette::ACCENT(), BrushStyle::Solid);
				dc.set_pen(Palette::ACCENT(), 1, PenStyle::Solid);
				dc.draw_rounded_rectangle(x - 2, y - 2, w + 4, h + 4, radius + 2.0);
			}
			dc.set_brush(Palette::FIELD_BG(), BrushStyle::Solid);
			dc.set_pen(Palette::FIELD_BG(), 1, PenStyle::Solid);
			dc.draw_rounded_rectangle(x, y, w, h, radius);
			if kind == Field::Search {
				draw_magnifier(dc, x + 16, y + h / 2);
			}
		}
	}
}

fn draw_magnifier(dc: &impl DeviceContext, x: i32, y: i32) {
	let grey = Palette::ICON();
	dc.set_brush(Palette::FIELD_BG(), BrushStyle::Solid);
	dc.set_pen(grey, 2, PenStyle::Solid);
	dc.draw_circle(x - 1, y - 1, 6);
	dc.draw_line(x + 3, y + 3, x + 8, y + 8);
}

pub(crate) fn button_row(sizer: &BoxSizer, buttons: &[&Button]) {
	let row = BoxSizer::builder(Orientation::Horizontal).build();
	row.add_stretch_spacer(1);
	for (i, button) in buttons.iter().enumerate() {
		row.add(*button, 0, SizerFlag::Left, if i == 0 { 0 } else { 8 });
	}
	sizer.add_sizer(&row, 0, SizerFlag::Expand | SizerFlag::All, 14);
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
	Pill(bool),
	Switch,
	Rows,
}

struct Control {
	kind: Kind,
	background: Colour,
}

struct Rows {
	checks: Option<CheckListBox>,
	jid: Box<dyn Fn(usize) -> Option<String>>,
}

#[derive(Default)]
struct Registry {
	controls: HashMap<isize, Control>,
	rows: HashMap<isize, Rows>,
	switches: HashMap<isize, CheckBox>,
	parents: Vec<isize>,
	avatars_hooked: bool,
}

thread_local! {
	static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
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

pub(crate) fn pill_button(button: &Button, primary: bool, background: Colour) {
	button.set_font(&if primary { theme::Fonts::body_strong() } else { theme::Fonts::body() });
	let hwnd = hwnd_of(button);
	unsafe {
		let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
		SetWindowLongPtrW(hwnd, GWL_STYLE, (style & !0xF) | BS_OWNERDRAW as isize);
	}
	register(hwnd, Kind::Pill(primary), background);
	let text = button.get_text_extent(&strip_mnemonic(&button.get_label()));
	let line = button.get_char_height();
	button.set_min_size(Size::new((text.width + line * 2 + 8).max(line * 5), line * 2 + 8));
}

/// Draws `check` as a switch at the right of its label, on `background`.
/// Add it to its sizer with `SizerFlag::Expand` so the switch lines up on
/// the right. A long label wraps, and one written "Name: what it does" shows
/// as WhatsApp lays out a setting, its name with what it does in grey under
/// it; the label a screen reader reads stays as it is.
pub(crate) fn switch(check: &CheckBox, background: Colour) {
	check.set_font(&theme::Fonts::body());
	let hwnd = hwnd_of(check);
	REGISTRY.with(|r| r.borrow_mut().switches.insert(hwnd.0 as isize, *check));
	register(hwnd, Kind::Switch, background);
	let line = check.get_char_height();
	check.set_min_size(Size::new(-1, line + 14));
	let check_c = *check;
	check.on_size(move |event| {
		let width = check_c.get_size().width;
		let height = switch_height(hwnd, width);
		if width > 0 && check_c.get_min_size().height != height {
			check_c.set_min_size(Size::new(-1, height));
			wxdragon::call_after(Box::new(move || {
				if let Some(parent) = check_c.get_parent() {
					parent.layout();
				}
			}));
		}
		event.skip(true);
	});
}

fn track_size(line: i32) -> (i32, i32) {
	let track_h = (line * 4 / 5).max(12);
	(track_h, track_h * 2 - 2)
}

fn switch_lines(dc: HDC, font: HFONT, line: i32, label: &str, width: i32) -> (Vec<String>, Vec<String>) {
	let (_, track_w) = track_size(line);
	let room = (width - track_w - 4 - 14).max(40);
	let (name, more) = match label.split_once(": ") {
		Some((name, more)) if name.chars().count() <= 40 => (name, more),
		_ => (label, ""),
	};
	let mut more_chars = more.chars();
	let more = match more_chars.next() {
		Some(first) => first.to_uppercase().chain(more_chars).collect(),
		None => String::new(),
	};
	(wrap(dc, font, name, room), wrap(dc, font, &more, room))
}

fn wrap(dc: HDC, font: HFONT, text: &str, width: i32) -> Vec<String> {
	let mut lines = Vec::new();
	let mut current = String::new();
	for word in text.split_whitespace() {
		let candidate = if current.is_empty() { word.to_string() } else { format!("{current} {word}") };
		if !current.is_empty() && gdi::text_width(dc, font, &candidate) > width {
			lines.push(std::mem::replace(&mut current, word.to_string()));
		} else {
			current = candidate;
		}
	}
	if !current.is_empty() {
		lines.push(current);
	}
	lines
}

fn switch_height(hwnd: HWND, width: i32) -> i32 {
	let line = line_height(hwnd);
	let label = strip_mnemonic(&window_text(hwnd));
	let font = control_font(hwnd);
	let (name, more) = unsafe {
		let dc = GetDC(Some(hwnd));
		let lines = switch_lines(dc, font, line, &label, width);
		ReleaseDC(Some(hwnd), dc);
		lines
	};
	let gap = if more.is_empty() { 0 } else { 2 };
	(name.len() + more.len()).max(1) as i32 * line + gap + 14
}

pub(crate) fn contact_rows(list: &ListBox, jid: impl Fn(usize) -> Option<String> + 'static) {
	install_rows(hwnd_of(list), None, Box::new(jid));
}

pub(crate) fn contact_check_rows(list: &CheckListBox, jid: impl Fn(usize) -> Option<String> + 'static) {
	install_rows(hwnd_of(list), Some(*list), Box::new(jid));
}

fn install_rows(hwnd: HWND, checks: Option<CheckListBox>, jid: Box<dyn Fn(usize) -> Option<String>>) {
	register(hwnd, Kind::Rows, Palette::CONTENT_BG());
	let hook = REGISTRY.with(|r| {
		let mut r = r.borrow_mut();
		r.rows.insert(hwnd.0 as isize, Rows { checks, jid });
		!std::mem::replace(&mut r.avatars_hooked, true)
	});
	if hook {
		avatars::on_change(|| {
			let lists: Vec<isize> = REGISTRY.with(|r| r.borrow().rows.keys().copied().collect());
			for list in lists {
				unsafe {
					let _ = InvalidateRect(Some(HWND(list as *mut _)), None, false);
				}
			}
		});
	}
	set_row_height(hwnd);
}

fn picture_size(line: i32) -> i32 {
	avatars::size().unwrap_or(line * 2 + 6)
}

fn row_height(hwnd: HWND) -> i32 {
	picture_size(line_height(hwnd)) + 14
}

fn set_row_height(hwnd: HWND) {
	let height = row_height(hwnd);
	unsafe {
		SendMessageW(hwnd, LB_SETITEMHEIGHT, Some(WPARAM(0)), Some(LPARAM(height as isize)));
	}
}

fn control_font(hwnd: HWND) -> HFONT {
	HFONT(unsafe { SendMessageW(hwnd, WM_GETFONT, None, None) }.0 as *mut _)
}

fn logfont(font: HFONT) -> LOGFONTW {
	let mut lf = LOGFONTW::default();
	unsafe {
		GetObjectW(HGDIOBJ(font.0), size_of::<LOGFONTW>() as i32, Some(std::ptr::from_mut(&mut lf).cast()));
	}
	if lf.lfHeight == 0 {
		lf.lfHeight = -16;
	}
	lf
}

fn line_height(hwnd: HWND) -> i32 {
	let font = control_font(hwnd);
	let mut tm = TEXTMETRICW::default();
	unsafe {
		let dc = GetDC(Some(hwnd));
		let old = SelectObject(dc, HGDIOBJ(font.0));
		let _ = GetTextMetricsW(dc, &mut tm);
		SelectObject(dc, old);
		ReleaseDC(Some(hwnd), dc);
	}
	tm.tmHeight.max(8)
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
		(LB_SETITEMHEIGHT, Some(Kind::Rows)) => {
			let height = row_height(hwnd);
			unsafe { DefSubclassProc(hwnd, msg, wparam, LPARAM(height as isize)) }
		}
		(WM_SETTEXT, Some(_)) => {
			let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
			unsafe {
				let _ = InvalidateRect(Some(hwnd), None, false);
			}
			result
		}
		(WM_NCDESTROY, _) => {
			REGISTRY.with(|r| {
				if let Ok(mut r) = r.try_borrow_mut() {
					r.controls.remove(&(hwnd.0 as isize));
					r.rows.remove(&(hwnd.0 as isize));
					r.switches.remove(&(hwnd.0 as isize));
				}
			});
			unsafe {
				let _ = RemoveWindowSubclass(hwnd, Some(control_proc), CONTROL_SUBCLASS);
				DefSubclassProc(hwnd, msg, wparam, lparam)
			}
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
	let control = |child: HWND| {
		REGISTRY.with(|r| {
			r.try_borrow()
				.ok()
				.and_then(|r| r.controls.get(&(child.0 as isize)).map(|c| (c.kind, theme::now(c.background))))
		})
	};
	match msg {
		WM_DRAWITEM if lparam.0 != 0 => {
			let item = unsafe { &*(lparam.0 as *const DRAWITEMSTRUCT) };
			let fits = match control(item.hwndItem) {
				Some((Kind::Pill(primary), background)) if item.CtlType == ODT_BUTTON => {
					Some((Kind::Pill(primary), background))
				}
				Some((Kind::Rows, background)) if item.CtlType == ODT_LISTBOX && item.itemID != u32::MAX => {
					Some((Kind::Rows, background))
				}
				Some((Kind::Switch, background)) if item.CtlType == ODT_BUTTON => {
					let disabled = item.itemState.0 & ODS_DISABLED.0 != 0;
					let focused = item.itemState.0 & ODS_FOCUS.0 != 0;
					paint_switch(item.hwndItem, item.hDC, item.rcItem, disabled, focused, background);
					return LRESULT(1);
				}
				_ => None,
			};
			if let Some((kind, background)) = fits {
				draw_item(item, kind, background);
				return LRESULT(1);
			}
		}
		WM_NOTIFY if lparam.0 != 0 => {
			let header = unsafe { &*(lparam.0 as *const NMHDR) };
			if header.code == NM_CUSTOMDRAW
				&& let Some((Kind::Switch, background)) = control(header.hwndFrom)
			{
				let draw = unsafe { &*(lparam.0 as *const NMCUSTOMDRAW) };
				if draw.dwDrawStage == CDDS_PREPAINT {
					let disabled = draw.uItemState.0 & CDIS_DISABLED.0 != 0;
					let focused = draw.uItemState.0 & CDIS_FOCUS.0 != 0;
					paint_switch(header.hwndFrom, draw.hdc, draw.rc, disabled, focused, background);
					return LRESULT(CDRF_SKIPDEFAULT as isize);
				}
			}
		}
		WM_NCDESTROY => {
			REGISTRY.with(|r| {
				if let Ok(mut r) = r.try_borrow_mut() {
					r.parents.retain(|&p| p != hwnd.0 as isize);
				}
			});
			unsafe {
				let _ = RemoveWindowSubclass(hwnd, Some(parent_proc), PARENT_SUBCLASS);
			}
		}
		_ => {}
	}
	unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

fn rgb(colour: Colour) -> COLORREF {
	colorref(colour.r, colour.g, colour.b)
}

fn sys(index: windows::Win32::Graphics::Gdi::SYS_COLOR_INDEX) -> COLORREF {
	COLORREF(unsafe { GetSysColor(index) })
}

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
	RECT { left, top, right, bottom }
}

fn blob(dc: HDC, x: i32, y: i32, w: i32, h: i32, colour: COLORREF) {
	if w <= 0 || h <= 0 {
		return;
	}
	let (r, g, b) = ((colour.0 & 0xFF) as u8, (colour.0 >> 8 & 0xFF) as u8, (colour.0 >> 16 & 0xFF) as u8);
	if let Some(shape) = gdi::round_rect(w, h, w.min(h) as f32 / 2.0, (r, g, b)) {
		shape.draw(dc, x, y);
	}
}

fn strip_mnemonic(label: &str) -> String {
	let mut out = String::with_capacity(label.len());
	let mut chars = label.chars();
	while let Some(c) = chars.next() {
		if c == '&' {
			if let Some(next) = chars.next() {
				out.push(next);
			}
		} else {
			out.push(c);
		}
	}
	out
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

fn buffered(target: HDC, r: RECT, paint: impl FnOnce(HDC, i32, i32)) {
	let (width, height) = (r.right - r.left, r.bottom - r.top);
	if width <= 0 || height <= 0 {
		return;
	}
	unsafe {
		let mem = CreateCompatibleDC(Some(target));
		let bitmap = CreateCompatibleBitmap(target, width, height);
		let old = SelectObject(mem, HGDIOBJ(bitmap.0));
		SetBkMode(mem, TRANSPARENT);
		paint(mem, width, height);
		let _ = BitBlt(target, r.left, r.top, width, height, Some(mem), 0, 0, SRCCOPY);
		SelectObject(mem, old);
		let _ = DeleteObject(HGDIOBJ(bitmap.0));
		let _ = DeleteDC(mem);
	}
}

fn draw_item(item: &DRAWITEMSTRUCT, kind: Kind, background: Colour) {
	let hwnd = item.hwndItem;
	let selected = item.itemState.0 & ODS_SELECTED.0 != 0;
	let disabled = item.itemState.0 & ODS_DISABLED.0 != 0;
	let font = control_font(hwnd);
	match kind {
		Kind::Pill(primary) => {
			let focused = item.itemState.0 & ODS_FOCUS.0 != 0;
			let label = strip_mnemonic(&window_text(hwnd));
			buffered(item.hDC, item.rcItem, |dc, w, h| {
				paint_pill(dc, w, h, &label, font, primary, (selected, focused, disabled), background)
			});
		}
		Kind::Rows => {
			let focused = (unsafe { GetFocus() }) == hwnd;
			let index = item.itemID;
			let text = list_text(hwnd, index);
			let (checked, jid) = REGISTRY.with(|r| {
				let Ok(r) = r.try_borrow() else { return (None, None) };
				let Some(rows) = r.rows.get(&(hwnd.0 as isize)) else { return (None, None) };
				(rows.checks.map(|list| list.is_checked(index)), (rows.jid)(index as usize))
			});
			let line = line_height(hwnd);
			buffered(item.hDC, item.rcItem, |dc, w, h| {
				let look = RowLook { text: &text, jid: jid.as_deref().unwrap_or(""), checked, selected, focused };
				paint_row(dc, w, h, font, line, &look)
			});
		}
		Kind::Switch => {}
	}
}

#[allow(clippy::too_many_arguments)]
fn paint_pill(
	dc: HDC,
	width: i32,
	height: i32,
	label: &str,
	font: HFONT,
	primary: bool,
	(pressed, focused, disabled): (bool, bool, bool),
	background: Colour,
) {
	let custom = theme::use_custom_colours();
	let back = if custom { rgb(background) } else { sys(COLOR_BTNFACE) };
	gdi::fill(dc, rect(0, 0, width, height), back);
	let (x, y, w, h) = (3, 3, width - 6, height - 6);
	let (fill, ink, edge) = if !custom {
		let (fill, ink) =
			if pressed { (sys(COLOR_HIGHLIGHT), sys(COLOR_HIGHLIGHTTEXT)) } else { (back, sys(COLOR_BTNTEXT)) };
		(fill, ink, Some(sys(COLOR_BTNTEXT)))
	} else if disabled {
		(rgb(Palette::DIVIDER_SOFT()), rgb(Palette::TEXT_MUTED()), None)
	} else if primary {
		(rgb(if pressed { Palette::ACCENT_PRESSED() } else { Palette::ACCENT() }), rgb(Palette::ACCENT_TEXT()), None)
	} else {
		let fill = rgb(if pressed { Palette::FIELD_BG() } else { Palette::CONTENT_BG() });
		(fill, rgb(Palette::ACCENT()), Some(rgb(Palette::OUTLINE())))
	};
	if focused {
		let ring = if custom { rgb(Palette::TEXT_PRIMARY()) } else { sys(COLOR_HIGHLIGHT) };
		blob(dc, 0, 0, width, height, ring);
		blob(dc, 2, 2, width - 4, height - 4, back);
	}
	match edge {
		Some(edge) => {
			blob(dc, x, y, w, h, edge);
			blob(dc, x + 1, y + 1, w - 2, h - 2, fill);
		}
		None => blob(dc, x, y, w, h, fill),
	}
	let ink = if disabled && !custom { sys(COLOR_GRAYTEXT) } else { ink };
	gdi::text(dc, font, ink, label, rect(x + 4, y, x + w - 4, y + h), DT_CENTER);
}

fn paint_switch(hwnd: HWND, hdc: HDC, rc: RECT, disabled: bool, focused: bool, background: Colour) {
	let check = REGISTRY.with(|r| r.try_borrow().ok().and_then(|r| r.switches.get(&(hwnd.0 as isize)).copied()));
	let checked = match check {
		Some(check) => check.is_checked(),
		None => unsafe { SendMessageW(hwnd, BM_GETCHECK, None, None) }.0 == 1,
	};
	let disabled = disabled || !unsafe { IsWindowEnabled(hwnd) }.as_bool();
	let label = strip_mnemonic(&window_text(hwnd));
	let font = control_font(hwnd);
	let line = line_height(hwnd);
	let custom = theme::use_custom_colours();
	buffered(hdc, rc, |dc, width, height| {
		let back = if custom { rgb(background) } else { sys(COLOR_BTNFACE) };
		gdi::fill(dc, rect(0, 0, width, height), back);
		let (track_h, track_w) = track_size(line);
		let (name, more) = switch_lines(dc, font, line, &label, width);
		let lines = (name.len() + more.len()).max(1) as i32;
		let gap = if more.is_empty() { 0 } else { 2 };
		let top = ((height - lines * line - gap) / 2).max(0);
		let (tx, ty) = (width - track_w - 4, top + (line - track_h) / 2);
		let (text, muted) = if disabled {
			let grey = if custom { rgb(Palette::TEXT_MUTED()) } else { sys(COLOR_GRAYTEXT) };
			(grey, grey)
		} else if custom {
			(rgb(Palette::TEXT_PRIMARY()), rgb(Palette::TEXT_MUTED()))
		} else {
			(sys(COLOR_BTNTEXT), sys(COLOR_BTNTEXT))
		};
		let mut y = top;
		for (i, part) in name.iter().chain(&more).enumerate() {
			if i == name.len() {
				y += gap;
			}
			let colour = if i < name.len() { text } else { muted };
			gdi::text(dc, font, colour, part, rect(2, y, tx - 10, y + line), Default::default());
			y += line;
		}
		if focused {
			let ring = if custom { rgb(Palette::TEXT_PRIMARY()) } else { sys(COLOR_HIGHLIGHT) };
			blob(dc, tx - 4, ty - 4, track_w + 8, track_h + 8, ring);
			blob(dc, tx - 2, ty - 2, track_w + 4, track_h + 4, back);
		}
		let (track, knob) = match (custom, checked, disabled) {
			(true, true, false) => (rgb(Palette::ACCENT()), colorref(0xFF, 0xFF, 0xFF)),
			(true, true, true) => (colorref(0x9F, 0xC2, 0xB9), colorref(0xFF, 0xFF, 0xFF)),
			(true, false, false) => (rgb(Palette::LINE()), colorref(0xFF, 0xFF, 0xFF)),
			(true, false, true) => (rgb(Palette::OUTLINE()), colorref(0xFF, 0xFF, 0xFF)),
			(false, true, _) => (sys(COLOR_HIGHLIGHT), sys(COLOR_HIGHLIGHTTEXT)),
			(false, false, _) => (sys(COLOR_BTNTEXT), back),
		};
		blob(dc, tx, ty, track_w, track_h, track);
		if !custom && !checked {
			blob(dc, tx + 2, ty + 2, track_w - 4, track_h - 4, back);
		}
		let knob_side = track_h - 6;
		let kx = if checked { tx + track_w - knob_side - 3 } else { tx + 3 };
		let knob = if !custom && !checked { sys(COLOR_BTNTEXT) } else { knob };
		blob(dc, kx, ty + 3, knob_side, knob_side, knob);
	});
}

struct RowLook<'a> {
	text: &'a str,
	jid: &'a str,
	checked: Option<bool>,
	selected: bool,
	focused: bool,
}

fn split_contact(text: &str) -> (&str, &str) {
	match text.rfind(" +") {
		Some(at) if at > 0 => (&text[..at], &text[at + 1..]),
		_ => (text, ""),
	}
}

fn paint_row(dc: HDC, width: i32, height: i32, font: HFONT, line: i32, look: &RowLook) {
	let custom = theme::use_custom_colours();
	let (back, text, muted) = if !custom {
		if look.selected {
			(sys(COLOR_HIGHLIGHT), sys(COLOR_HIGHLIGHTTEXT), sys(COLOR_HIGHLIGHTTEXT))
		} else {
			(sys(COLOR_WINDOW), sys(COLOR_WINDOWTEXT), sys(COLOR_GRAYTEXT))
		}
	} else {
		let back = match (look.selected, look.focused) {
			(true, true) => rgb(Palette::SELECTED_ROW()),
			(true, false) => rgb(Palette::FIELD_BG()),
			_ => rgb(Palette::CONTENT_BG()),
		};
		(back, rgb(Palette::TEXT_PRIMARY()), rgb(Palette::TEXT_MUTED()))
	};
	gdi::fill(dc, rect(0, 0, width, height), back);
	if look.selected && look.focused {
		let bar = if custom { rgb(Palette::ACCENT()) } else { sys(COLOR_HIGHLIGHTTEXT) };
		gdi::fill(dc, rect(0, 2, 3, height - 2), bar);
	}
	let lf = logfont(font);
	let mut x = 10;
	if let Some(checked) = look.checked {
		let side = (line + 2).min(height - 4);
		let y = (height - side) / 2;
		let x0 = 4;
		let accent = if custom { rgb(Palette::ACCENT()) } else { text };
		if checked {
			blob(dc, x0, y, side, side, accent);
			let tick = if custom { rgb(Palette::ACCENT_TEXT()) } else { back };
			if let Some(icons) = gdi::icon_font(dc, &lf, 0.8) {
				gdi::text(dc, icons, tick, "\u{E73E}", rect(x0, y, x0 + side, y + side), DT_CENTER);
				gdi::delete_font(icons);
			}
		} else {
			blob(dc, x0, y, side, side, if custom { rgb(Palette::LINE()) } else { text });
			blob(dc, x0 + 2, y + 2, side - 4, side - 4, back);
		}
		x = x0 + side + 10;
	}
	let size = picture_size(line).min(height - 4);
	let top = (height - size) / 2;
	let drawn = avatars::with_avatar(look.jid, size, |avatar| match avatar {
		Avatar::Picture(bitmap) => {
			bitmap.draw(dc, x, top);
			true
		}
		Avatar::Nothing | Avatar::Waiting => false,
	});
	if !drawn {
		let circle = if custom { rgb(Palette::AVATAR_BG()) } else { colorref(0x80, 0x80, 0x80) };
		blob(dc, x, top, size, size, circle);
		let glyph = if look.jid.ends_with("@g.us") { "\u{E716}" } else { "\u{E77B}" };
		if let Some(icons) = gdi::icon_font(dc, &lf, 1.3) {
			gdi::text(dc, icons, colorref(0xFF, 0xFF, 0xFF), glyph, rect(x, top, x + size, top + size), DT_CENTER);
			gdi::delete_font(icons);
		}
	}
	let tx = x + size + 12;
	let (name, number) = split_contact(look.text);
	if number.is_empty() {
		gdi::text(dc, font, text, name, rect(tx, 0, width - 8, height), Default::default());
	} else {
		let small = gdi::derived_font(&lf, 0.88, 400);
		let mid = height / 2;
		gdi::text(dc, font, text, name, rect(tx, mid - line - 1, width - 8, mid + 1), Default::default());
		gdi::text(dc, small, muted, number, rect(tx, mid + 1, width - 8, mid + line + 1), Default::default());
		gdi::delete_font(small);
	}
	if custom && !look.selected {
		gdi::fill(dc, rect(tx, height - 1, width, height), rgb(Palette::DIVIDER_SOFT()));
	}
}

/// Gives every window this app opens (the main window, dialogs, message
/// boxes) fully rounded corners, as WhatsApp Web's popups have. Windows 11
/// draws them: each window is asked as it's first activated, through a hook
/// on this (the UI) thread. Only the frame's shape changes, so a screen
/// reader sees nothing different; Windows 10 has no rounded corners and
/// quietly ignores the request. Call once, at startup.
pub(crate) fn round_window_corners() {
	use windows::Win32::{
		System::Threading::GetCurrentThreadId,
		UI::WindowsAndMessaging::{SetWindowsHookExW, WH_CBT},
	};
	unsafe {
		let _ = SetWindowsHookExW(WH_CBT, Some(cbt_proc), None, GetCurrentThreadId());
	}
}

unsafe extern "system" fn cbt_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
	use windows::Win32::{
		Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute},
		UI::WindowsAndMessaging::{CallNextHookEx, HCBT_ACTIVATE, WS_CHILD},
	};
	if code == HCBT_ACTIVATE as i32 {
		let hwnd = HWND(wparam.0 as *mut _);
		let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
		if style & WS_CHILD.0 == 0 {
			let round = DWMWCP_ROUND;
			unsafe {
				let _ = DwmSetWindowAttribute(
					hwnd,
					DWMWA_WINDOW_CORNER_PREFERENCE,
					std::ptr::from_ref(&round).cast(),
					size_of_val(&round) as u32,
				);
			}
		}
	}
	unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

#[cfg(test)]
mod tests {
	use super::{split_contact, strip_mnemonic};

	#[test]
	fn a_contact_row_splits_name_and_number() {
		assert_eq!(split_contact("Mum +44 7700 900123"), ("Mum", "+44 7700 900123"));
		assert_eq!(split_contact("+44 7700 900123"), ("+44 7700 900123", ""));
		assert_eq!(split_contact("Mum"), ("Mum", ""));
	}

	#[test]
	fn mnemonics_are_not_drawn() {
		assert_eq!(strip_mnemonic("&View once"), "View once");
		assert_eq!(strip_mnemonic("Tom && Jerry"), "Tom & Jerry");
	}
}
