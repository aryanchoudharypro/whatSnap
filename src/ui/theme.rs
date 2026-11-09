use wxdragon::prelude::*;

pub struct Palette;

macro_rules! palette {
	($($(#[$doc:meta])* $name:ident = $light:expr, $dark:expr;)*) => {
		#[allow(dead_code, non_snake_case)]
		impl Palette {
			$($(#[$doc])* pub fn $name() -> Colour {
				if is_dark() { $dark } else { $light }
			})*
		}

		const PAIRS: &[(Colour, Colour)] = &[$(($light, $dark)),*];
	};
}

const fn rgb(r: u8, g: u8, b: u8) -> Colour {
	Colour::rgb(r, g, b)
}

palette! {
	SIDEBAR = rgb(0x11, 0x3B, 0x33), rgb(0x11, 0x3B, 0x33);
	SIDEBAR_TEXT = rgb(0xEA, 0xF2, 0xEF), rgb(0xEA, 0xF2, 0xEF);
	SIDEBAR_MUTED = rgb(0x9F, 0xC2, 0xB9), rgb(0x9F, 0xC2, 0xB9);

	CONTENT_BG = rgb(0xFF, 0xFF, 0xFF), rgb(0x11, 0x1B, 0x21);
	PANEL_BG = rgb(0xF7, 0xF7, 0xF6), rgb(0x20, 0x2C, 0x33);
	CHAT_BG = rgb(0xF3, 0xF0, 0xEA), rgb(0x0B, 0x14, 0x1A);

	ACCENT = rgb(0x15, 0x7F, 0x45), rgb(0x00, 0xA8, 0x84);
	ACCENT_PRESSED = rgb(0x0F, 0x63, 0x36), rgb(0x00, 0x80, 0x69);
	ACCENT_TEXT = rgb(0xFF, 0xFF, 0xFF), rgb(0x11, 0x1B, 0x21);

	TEXT_PRIMARY = rgb(0x1B, 0x1B, 0x1B), rgb(0xE9, 0xED, 0xEF);
	TEXT_MUTED = rgb(0x62, 0x66, 0x64), rgb(0x8B, 0x9B, 0xA5);
	DIVIDER = rgb(0x94, 0x90, 0x89), rgb(0x66, 0x77, 0x81);
	DIVIDER_SOFT = rgb(0xE9, 0xED, 0xEF), rgb(0x22, 0x2D, 0x34);

	FIELD_BG = rgb(0xF0, 0xF2, 0xF5), rgb(0x2A, 0x39, 0x42);
	SELECTED_ROW = rgb(0xE4, 0xE8, 0xEA), rgb(0x2A, 0x39, 0x42);
	ICON = rgb(0x3B, 0x4A, 0x54), rgb(0xAE, 0xBA, 0xC1);
	PRESSED = rgb(0xD9, 0xDB, 0xDF), rgb(0x37, 0x42, 0x48);
	AVATAR_BG = rgb(0xDF, 0xE5, 0xE7), rgb(0x6A, 0x71, 0x75);
	LINE = rgb(0x86, 0x96, 0xA0), rgb(0x86, 0x96, 0xA0);
	OUTLINE = rgb(0xD1, 0xD7, 0xDB), rgb(0x3B, 0x4A, 0x54);
	DANGER = rgb(0xD1, 0x1A, 0x3A), rgb(0xF1, 0x5C, 0x6D);
	READ = rgb(0x0B, 0x6F, 0xA4), rgb(0x53, 0xBD, 0xEB);
	BUBBLE_MINE = rgb(0xD9, 0xFD, 0xD3), rgb(0x00, 0x5C, 0x4B);
	BUBBLE_THEIRS = rgb(0xFF, 0xFF, 0xFF), rgb(0x20, 0x2C, 0x33);
	QUOTE_MINE = rgb(0xC9, 0xEE, 0xC3), rgb(0x02, 0x51, 0x44);
	QUOTE_THEIRS = rgb(0xF0, 0xF2, 0xF5), rgb(0x1D, 0x28, 0x2F);
	BAND_FOCUSED = rgb(0xCF, 0xE3, 0xD7), rgb(0x1F, 0x3D, 0x35);
	BAND = rgb(0xE2, 0xDE, 0xD6), rgb(0x26, 0x32, 0x3A);
	WAVE_MINE = rgb(0x8F, 0xB8, 0x8A), rgb(0x7F, 0xB9, 0xA9);
	WAVE = rgb(0xA0, 0xA6, 0xAA), rgb(0x86, 0x96, 0xA0);
	PLACEHOLDER = rgb(0xD5, 0xD9, 0xDB), rgb(0x2A, 0x39, 0x42);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
	#[default]
	System,
	Light,
	Dark,
}

static DARK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_theme(choice: ThemeChoice) {
	let dark = match choice {
		ThemeChoice::Light => false,
		ThemeChoice::Dark => true,
		ThemeChoice::System => system_prefers_dark(),
	};
	DARK.store(dark && use_custom_colours(), std::sync::atomic::Ordering::Relaxed);
}

/// Whether the dark theme is on. Never in High Contrast, whose colours are
/// the system's.
pub fn is_dark() -> bool {
	DARK.load(std::sync::atomic::Ordering::Relaxed)
}

fn swapped(colour: Colour, to_dark: bool) -> Option<Colour> {
	PAIRS.iter().find_map(|&(light, dark)| {
		let (from, to) = if to_dark { (light, dark) } else { (dark, light) };
		(from == colour && from != to).then_some(to)
	})
}

fn is_palette(colour: Colour) -> bool {
	PAIRS.iter().any(|&(light, dark)| colour == light || colour == dark)
}

pub fn now(colour: Colour) -> Colour {
	let dark = is_dark();
	if PAIRS.iter().any(|&(light, d)| colour == if dark { d } else { light }) {
		return colour;
	}
	swapped(colour, dark).unwrap_or(colour)
}

#[cfg(windows)]
pub fn match_theme(root: &impl WxWidget) {
	use windows::Win32::{
		Foundation::{HWND, LPARAM},
		Graphics::Gdi::{RDW_ALLCHILDREN, RDW_ERASE, RDW_FRAME, RDW_INVALIDATE, RedrawWindow},
		UI::WindowsAndMessaging::{EnumChildWindows, GetDlgCtrlID},
	};
	if !use_custom_colours() {
		return;
	}
	unsafe extern "system" fn collect(hwnd: HWND, list: LPARAM) -> windows::core::BOOL {
		unsafe { (*(list.0 as *mut Vec<HWND>)).push(hwnd) };
		true.into()
	}
	let root_hwnd = HWND(root.get_handle().cast());
	let mut all = Vec::new();
	unsafe {
		let _ = EnumChildWindows(Some(root_hwnd), Some(collect), LPARAM(&mut all as *mut _ as isize));
	}
	for hwnd in all {
		let Some(win) =
			root.find_window_by_id(unsafe { GetDlgCtrlID(hwnd) }).filter(|w| HWND(w.get_handle().cast()) == hwnd)
		else {
			continue;
		};
		let background = win.get_background_color();
		if is_palette(background) && now(background) != background {
			win.set_background_color(now(background));
		}
		let foreground = win.get_foreground_color();
		if is_palette(foreground) && now(foreground) != foreground {
			win.set_foreground_color(now(foreground));
		}
	}
	unsafe {
		let _ = RedrawWindow(Some(root_hwnd), None, None, RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_FRAME);
	}
}

#[cfg(windows)]
pub fn change_theme(choice: ThemeChoice) {
	use windows::Win32::{
		Foundation::{HWND, LPARAM, WPARAM},
		Graphics::Gdi::{RDW_ALLCHILDREN, RDW_ERASE, RDW_FRAME, RDW_INVALIDATE, RedrawWindow},
		System::Threading::GetCurrentThreadId,
		UI::WindowsAndMessaging::{EnumChildWindows, EnumThreadWindows, GetDlgCtrlID, SendMessageW, WM_SYSCOLORCHANGE},
	};

	let was = is_dark();
	set_theme(choice);
	let dark = is_dark();
	if dark == was {
		return;
	}
	let appearance =
		if dark { wxdragon::appearance::Appearance::Dark } else { wxdragon::appearance::Appearance::Light };
	let _ = wxdragon::app::set_appearance(appearance);
	let Some(top) = wxdragon::app::get_app_instance().and_then(|app| app.get_top_window()) else { return };

	unsafe extern "system" fn collect(hwnd: HWND, list: LPARAM) -> windows::core::BOOL {
		unsafe { (*(list.0 as *mut Vec<HWND>)).push(hwnd) };
		true.into()
	}
	let mut top_levels: Vec<HWND> = Vec::new();
	unsafe {
		let _ = EnumThreadWindows(GetCurrentThreadId(), Some(collect), LPARAM(&mut top_levels as *mut _ as isize));
	}
	let top_hwnd = HWND(top.get_handle().cast());
	for &window in &top_levels {
		let mut all = vec![window];
		unsafe {
			let _ = EnumChildWindows(Some(window), Some(collect), LPARAM(&mut all as *mut _ as isize));
			if window != top_hwnd {
				SendMessageW(window, WM_SYSCOLORCHANGE, Some(WPARAM(0)), Some(LPARAM(0)));
			}
		}
		let mut found = Vec::new();
		for &hwnd in &all {
			let win = if hwnd == top_hwnd {
				Some(top)
			} else {
				top.find_window_by_id(unsafe { GetDlgCtrlID(hwnd) }).filter(|w| HWND(w.get_handle().cast()) == hwnd)
			};
			found.extend(win);
		}
		if window != top_hwnd
			&& let Some(dialog) =
				found.iter().find_map(|w| w.get_parent().filter(|p| HWND(p.get_handle().cast()) == window))
		{
			found.insert(0, dialog);
		}
		for win in found {
			let background = swapped(win.get_background_color(), dark);
			if let Some(colour) = background {
				win.set_background_color(colour);
			}
			let foreground = win.get_foreground_color();
			if let Some(colour) = swapped(foreground, dark) {
				win.set_foreground_color(colour);
			} else if background.is_some() && !is_palette(foreground) {
				win.set_foreground_color(Palette::TEXT_PRIMARY());
			}
		}
		unsafe {
			let _ = RedrawWindow(Some(window), None, None, RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_FRAME);
		}
	}
}

/// Turns on wxWidgets' own dark mode for what Windows draws (title bars,
/// menus, scroll bars, check boxes, lists, tabs) when the dark theme is on.
/// Call before the first window. wx keeps each control's kind as it was for
/// screen readers: its dark check boxes still report a check box and whether
/// it's checked.
pub fn apply_to_windows() {
	if is_dark() {
		let _ = wxdragon::app::set_appearance(wxdragon::appearance::Appearance::Dark);
	}
}

pub fn tuple(c: Colour) -> (u8, u8, u8) {
	(c.r, c.g, c.b)
}

#[cfg(windows)]
fn system_prefers_dark() -> bool {
	use windows::{
		Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW},
		core::w,
	};
	let mut value = 1u32;
	let mut size = size_of::<u32>() as u32;
	let read = unsafe {
		RegGetValueW(
			HKEY_CURRENT_USER,
			w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
			w!("AppsUseLightTheme"),
			RRF_RT_REG_DWORD,
			None,
			Some(std::ptr::from_mut(&mut value).cast()),
			Some(&mut size),
		)
	};
	read.is_ok() && value == 0
}

#[cfg(not(windows))]
fn system_prefers_dark() -> bool {
	false
}

pub fn use_custom_colours() -> bool {
	use std::sync::OnceLock;

	static USE_CUSTOM: OnceLock<bool> = OnceLock::new();
	*USE_CUSTOM.get_or_init(|| !high_contrast_active())
}

#[cfg(windows)]
fn high_contrast_active() -> bool {
	use windows::Win32::UI::{
		Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW},
		WindowsAndMessaging::{SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW},
	};

	let size = size_of::<HIGHCONTRASTW>() as u32;
	let mut info = HIGHCONTRASTW { cbSize: size, ..Default::default() };
	let queried = unsafe {
		SystemParametersInfoW(
			SPI_GETHIGHCONTRAST,
			size,
			Some(std::ptr::from_mut(&mut info).cast()),
			SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
		)
	};
	queried.is_err() || info.dwFlags.contains(HCF_HIGHCONTRASTON)
}

#[cfg(not(windows))]
fn high_contrast_active() -> bool {
	false
}

pub struct Fonts;

impl Fonts {
	pub fn heading() -> Font {
		derive_font(4, FontWeight::SemiBold)
	}

	pub fn label() -> Font {
		derive_font(1, FontWeight::Normal)
	}

	pub fn body() -> Font {
		derive_font(2, FontWeight::Normal)
	}

	pub fn body_strong() -> Font {
		derive_font(2, FontWeight::SemiBold)
	}
}

fn derive_font(point_delta: i32, weight: FontWeight) -> Font {
	let mut font = SystemSettings::get_font(SystemFont::DefaultGuiFont).unwrap_or_default();
	let base = if font.get_point_size() > 0 { font.get_point_size() } else { 9 };
	let size = (base + point_delta).max(1);
	if let Some(face) = modern_face(&font)
		&& let Some(modern) = Font::builder().with_point_size(size).with_weight(weight).with_face_name(face).build()
	{
		return modern;
	}
	font.set_point_size(size);
	font.set_weight(weight);
	font
}

fn modern_face(system: &Font) -> Option<&'static str> {
	use std::sync::OnceLock;

	const FACE: &str = "Segoe UI Variable Text";
	static INSTALLED: OnceLock<bool> = OnceLock::new();
	if !system.get_face_name().eq_ignore_ascii_case("Segoe UI") {
		return None;
	}
	#[cfg(windows)]
	let installed = *INSTALLED.get_or_init(|| super::gdi::face_installed(FACE));
	#[cfg(not(windows))]
	let installed = *INSTALLED.get_or_init(|| false);
	installed.then_some(FACE)
}

pub fn style_sidebar(widget: &impl WxWidget) {
	if !use_custom_colours() {
		return;
	}
	widget.set_background_color(Palette::PANEL_BG());
	widget.set_foreground_color(Palette::TEXT_PRIMARY());
}

pub fn style_content(widget: &impl WxWidget) {
	if !use_custom_colours() {
		return;
	}
	widget.set_background_color(Palette::CONTENT_BG());
	widget.set_foreground_color(Palette::TEXT_PRIMARY());
}

pub fn style_chat_area(widget: &impl WxWidget) {
	if !use_custom_colours() {
		return;
	}
	widget.set_background_color(Palette::CHAT_BG());
	widget.set_foreground_color(Palette::TEXT_PRIMARY());
}

pub fn style_divider(line: &StaticLine) {
	if !use_custom_colours() {
		return;
	}
	line.set_foreground_color(Palette::DIVIDER());
}

pub fn style_primary_button(button: &Button) {
	button.set_font(&Fonts::body_strong());
	if !use_custom_colours() {
		return;
	}
	button.set_background_color(Palette::ACCENT());
	button.set_foreground_color(Palette::ACCENT_TEXT());
}

pub fn style_heading(label: &StaticText) {
	label.set_font(&Fonts::heading());
}

pub fn style_sidebar_heading(label: &StaticText) {
	label.set_font(&Fonts::heading());
	if !use_custom_colours() {
		return;
	}
	label.set_foreground_color(Palette::SIDEBAR_TEXT());
}

const APP_ICON_32: &[u8] = include_bytes!("../../assets/icon-32.rgba");
const APP_ICON_96: &[u8] = include_bytes!("../../assets/icon-96.rgba");

pub fn build_app_icon(size: u32) -> wxdragon::bitmap::Bitmap {
	let (data, side) = if size <= 48 { (APP_ICON_32, 32u32) } else { (APP_ICON_96, 96u32) };
	if size == side {
		return wxdragon::bitmap::Bitmap::from_rgba(data, side, side).expect("failed to build app icon bitmap");
	}
	let mut rgba = vec![0u8; (size * size * 4) as usize];
	for y in 0..size {
		let sy = (y * side / size) as usize;
		for x in 0..size {
			let sx = (x * side / size) as usize;
			let from = (sy * side as usize + sx) * 4;
			let to = ((y * size + x) * 4) as usize;
			rgba[to..to + 4].copy_from_slice(&data[from..from + 4]);
		}
	}
	wxdragon::bitmap::Bitmap::from_rgba(&rgba, size, size).expect("failed to build app icon bitmap")
}

pub fn style_muted_label(label: &StaticText) {
	label.set_font(&Fonts::label());
	if !use_custom_colours() {
		return;
	}
	label.set_foreground_color(Palette::TEXT_MUTED());
}
