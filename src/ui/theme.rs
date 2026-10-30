//! Visual theme applied on top of the plain default widget look.
//!
//! Every helper here only touches paint-time properties (colour, font,
//! bitmaps, spacing) — never widget type, label text, tab order, or the
//! accessible name/role of a control. Screen reader behaviour is derived
//! from those untouched properties, so this module cannot change it.
//!
//! Two rules keep the paint-time properties themselves from becoming an
//! accessibility problem for sighted users with low vision:
//!
//! 1. **High Contrast wins.** When the user has a Windows High Contrast
//!    theme on, they have explicitly asked the OS to pick every colour.
//!    Hard-coding ours on top of that is exactly the failure the setting
//!    exists to prevent (see [`use_custom_colours`]), so in that mode every
//!    colour helper here becomes a no-op and the native colours show through.
//! 2. **Font sizes are relative, never absolute.** Sizes are derived from the
//!    system GUI font (see [`Fonts`]) rather than written as fixed point
//!    sizes, so raising the system text size scales this app's text with it.

use wxdragon::prelude::*;

/// WhatsApp-inspired palette: white content, warm off-white chat
/// background, green accent for primary actions; and WhatsApp Web's own dark
/// colours when the dark theme is on (see [`is_dark`]).
///
/// Foreground/background pairings that carry text are kept at a contrast
/// ratio of at least 4.5:1 (WCAG 2.2 AA for body text); `DIVIDER` clears the
/// 3:1 required of meaningful non-text elements. In the dark theme the
/// accent is WhatsApp's own green, so text on it is dark (`ACCENT_TEXT`).
pub struct Palette;

macro_rules! palette {
	($($(#[$doc:meta])* $name:ident = $light:expr, $dark:expr;)*) => {
		/// Some tokens are part of the palette for consistency/future use
		/// even if not every one is drawn on yet, matching this codebase's
		/// convention for "kept for consistency" widget fields.
		#[allow(dead_code, non_snake_case)]
		impl Palette {
			$($(#[$doc])* pub fn $name() -> Colour {
				if is_dark() { $dark } else { $light }
			})*
		}
	};
}

const fn rgb(r: u8, g: u8, b: u8) -> Colour {
	Colour::rgb(r, g, b)
}

palette! {
	/// 10.87:1 against `SIDEBAR_TEXT`, 6.42:1 against `SIDEBAR_MUTED`.
	SIDEBAR = rgb(0x11, 0x3B, 0x33), rgb(0x11, 0x3B, 0x33);
	SIDEBAR_TEXT = rgb(0xEA, 0xF2, 0xEF), rgb(0xEA, 0xF2, 0xEF);
	SIDEBAR_MUTED = rgb(0x9F, 0xC2, 0xB9), rgb(0x9F, 0xC2, 0xB9);

	CONTENT_BG = rgb(0xFF, 0xFF, 0xFF), rgb(0x11, 0x1B, 0x21);
	PANEL_BG = rgb(0xF7, 0xF7, 0xF6), rgb(0x20, 0x2C, 0x33);
	CHAT_BG = rgb(0xF3, 0xF0, 0xEA), rgb(0x0B, 0x14, 0x1A);

	/// Darker than WhatsApp's brand green on purpose in the light theme: the
	/// brand value only reaches 3.06:1 against white, which fails AA for the
	/// button labels drawn on it. This one is 5.06:1. The dark theme's is
	/// WhatsApp's, 6.4:1 on `CONTENT_BG`.
	ACCENT = rgb(0x15, 0x7F, 0x45), rgb(0x00, 0xA8, 0x84);
	/// `ACCENT` while pressed.
	ACCENT_PRESSED = rgb(0x0F, 0x63, 0x36), rgb(0x00, 0x80, 0x69);
	/// Text on `ACCENT`.
	ACCENT_TEXT = rgb(0xFF, 0xFF, 0xFF), rgb(0x11, 0x1B, 0x21);

	/// 17.22:1 on `CONTENT_BG`, 15.14:1 on `CHAT_BG` (dark: 14.5:1).
	TEXT_PRIMARY = rgb(0x1B, 0x1B, 0x1B), rgb(0xE9, 0xED, 0xEF);
	/// 5.83:1 on `CONTENT_BG`, 5.12:1 on `CHAT_BG` (dark: 5.9:1).
	TEXT_MUTED = rgb(0x62, 0x66, 0x64), rgb(0x8B, 0x9B, 0xA5);
	/// 3.18:1 on `CONTENT_BG`. A hairline at the 1.37:1 of the usual
	/// near-white divider is invisible to a low-vision user.
	DIVIDER = rgb(0x94, 0x90, 0x89), rgb(0x66, 0x77, 0x81);
	/// WhatsApp Web's faint line between panes, for drawing only (it isn't
	/// text, so it needn't meet a contrast ratio).
	DIVIDER_SOFT = rgb(0xE9, 0xED, 0xEF), rgb(0x22, 0x2D, 0x34);

	/// The selected row of a list.
	SELECTED_ROW = rgb(0xE4, 0xE8, 0xEA), rgb(0x2A, 0x39, 0x42);
	/// Icons on bars and buttons.
	ICON = rgb(0x3B, 0x4A, 0x54), rgb(0xAE, 0xBA, 0xC1);
	/// Under a pressed or selected round button.
	PRESSED = rgb(0xD9, 0xDB, 0xDF), rgb(0x37, 0x42, 0x48);
	/// Behind the person glyph when there's no picture.
	AVATAR_BG = rgb(0xDF, 0xE5, 0xE7), rgb(0x6A, 0x71, 0x75);
	/// Search boxes and other filled fields.
	FIELD_BG = rgb(0xF0, 0xF2, 0xF5), rgb(0x2A, 0x39, 0x42);
	/// A field's underline, an unticked box's ring.
	LINE = rgb(0x86, 0x96, 0xA0), rgb(0x86, 0x96, 0xA0);
	/// Outlines of pills and plain buttons.
	OUTLINE = rgb(0xD1, 0xD7, 0xDB), rgb(0x3B, 0x4A, 0x54);
	/// Missed calls, End, Delete.
	DANGER = rgb(0xD1, 0x1A, 0x3A), rgb(0xF1, 0x5C, 0x6D);
	/// Read ticks: WhatsApp's blue, darkened to show on the light green bubble.
	READ = rgb(0x0B, 0x6F, 0xA4), rgb(0x53, 0xBD, 0xEB);
	/// Message bubbles: yours, theirs.
	BUBBLE_MINE = rgb(0xD9, 0xFD, 0xD3), rgb(0x00, 0x5C, 0x4B);
	BUBBLE_THEIRS = rgb(0xFF, 0xFF, 0xFF), rgb(0x20, 0x2C, 0x33);
	/// The reply box inside a bubble: yours, theirs.
	QUOTE_MINE = rgb(0xC9, 0xEE, 0xC3), rgb(0x02, 0x51, 0x44);
	QUOTE_THEIRS = rgb(0xF0, 0xF2, 0xF5), rgb(0x1D, 0x28, 0x2F);
	/// The band behind the selected message: the list focused, not.
	BAND_FOCUSED = rgb(0xCF, 0xE3, 0xD7), rgb(0x1F, 0x3D, 0x35);
	BAND = rgb(0xE2, 0xDE, 0xD6), rgb(0x26, 0x32, 0x3A);
	/// A voice message's waveform not yet played: in yours, in theirs.
	WAVE_MINE = rgb(0x8F, 0xB8, 0x8A), rgb(0x7F, 0xB9, 0xA9);
	WAVE = rgb(0xA0, 0xA6, 0xAA), rgb(0x86, 0x96, 0xA0);
	/// A photo's place while it loads.
	PLACEHOLDER = rgb(0xD5, 0xD9, 0xDB), rgb(0x2A, 0x39, 0x42);
}

/// Which theme to use, as Settings has it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
	/// Windows' own "app mode", as WhatsApp's "System default".
	#[default]
	System,
	Light,
	Dark,
}

static DARK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Picks the theme for this run. Called once at startup, before any window
/// is made; changing it takes a restart, as every colour is set as windows
/// are made.
pub fn set_theme(choice: ThemeChoice) {
	let dark = match choice {
		ThemeChoice::Light => false,
		ThemeChoice::Dark => true,
		ThemeChoice::System => system_prefers_dark(),
	};
	let _ = DARK.set(dark && use_custom_colours());
}

/// Whether the dark theme is on. Never in High Contrast, whose colours are
/// the system's.
pub fn is_dark() -> bool {
	*DARK.get_or_init(|| false)
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

/// A colour as the (r, g, b) the GDI helpers take.
pub fn tuple(c: Colour) -> (u8, u8, u8) {
	(c.r, c.g, c.b)
}

/// Windows' "Choose your default app mode": dark when AppsUseLightTheme is 0.
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

/// Whether this app should paint its own colours at all.
///
/// False when a Windows High Contrast theme is active. Those themes exist
/// because the user cannot reliably read arbitrary colour pairings, and they
/// work by having applications *stop* choosing colours and inherit the
/// system's. An app that keeps calling `set_background_color` regardless
/// punches its own palette through the accommodation — the deep teal sidebar
/// would survive into High Contrast Black while the surrounding native
/// chrome went black, and any control we did not happen to colour would be
/// left as system-coloured text on our hard-coded background.
///
/// Detected once, at first use. Windows delivers `WM_SETTINGCHANGE` when the
/// setting is toggled mid-session, but nothing here re-reads the value, so a
/// toggle only takes effect on the next launch.
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
	// If the query fails we cannot prove High Contrast is off, so assume it
	// might be on: losing the theme is a cosmetic regression, ignoring High
	// Contrast is an accessibility one.
	queried.is_err() || info.dwFlags.contains(HCF_HIGHCONTRASTON)
}

#[cfg(not(windows))]
fn high_contrast_active() -> bool {
	false
}

/// Named font roles used consistently across the app.
///
/// Sizes are expressed as offsets from the system GUI font rather than fixed
/// point sizes. Writing `10` here would mean a user who raised their system
/// text size to make Windows readable still got this app at the size they
/// could not read.
pub struct Fonts;

impl Fonts {
	/// Section/dialog headings (e.g. "Conversations", instructions).
	pub fn heading() -> Font {
		derive_font(4, FontWeight::SemiBold)
	}

	/// Small caps-ish labels above fields ("Search conversations").
	pub fn label() -> Font {
		derive_font(1, FontWeight::Normal)
	}

	/// Regular body text used by lists, inputs, message text.
	pub fn body() -> Font {
		derive_font(2, FontWeight::Normal)
	}

	/// Emphasized body text (chat names, primary buttons).
	pub fn body_strong() -> Font {
		derive_font(2, FontWeight::SemiBold)
	}
}

/// Builds a font `point_delta` points away from the system GUI font, keeping
/// that font's face so the app follows the system UI typeface instead of
/// naming one that may not be installed or may not cover the user's script.
fn derive_font(point_delta: i32, weight: FontWeight) -> Font {
	let mut font = SystemSettings::get_font(SystemFont::DefaultGuiFont).unwrap_or_default();
	// A non-positive size means the platform gave us nothing usable; 9pt is
	// the Windows default GUI size and a safe base to offset from.
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

/// Windows 11's own UI typeface, Segoe UI Variable, where the system font is
/// plain Segoe UI and it's installed: the same family, drawn for screens
/// (it's what WhatsApp's Windows app uses). A system font that isn't Segoe
/// UI (Yu Gothic UI, Microsoft YaHei UI, ... for other scripts, or one the
/// user picked) is kept as it is.
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

/// Applies the navigation strip's look: WhatsApp's light grey rail.
pub fn style_sidebar(widget: &impl WxWidget) {
	if !use_custom_colours() {
		return;
	}
	widget.set_background_color(Palette::PANEL_BG());
	widget.set_foreground_color(Palette::TEXT_PRIMARY());
}

/// Applies the plain content-area look: white background, dark text.
pub fn style_content(widget: &impl WxWidget) {
	if !use_custom_colours() {
		return;
	}
	widget.set_background_color(Palette::CONTENT_BG());
	widget.set_foreground_color(Palette::TEXT_PRIMARY());
}

/// Applies the warm chat-area look used behind the message list/composer.
pub fn style_chat_area(widget: &impl WxWidget) {
	if !use_custom_colours() {
		return;
	}
	widget.set_background_color(Palette::CHAT_BG());
	widget.set_foreground_color(Palette::TEXT_PRIMARY());
}

/// Paints a decorative rule so it stays visible without competing with text.
pub fn style_divider(line: &StaticLine) {
	if !use_custom_colours() {
		return;
	}
	line.set_foreground_color(Palette::DIVIDER());
}

/// Styles a button as the primary call-to-action (Send, Continue, etc).
pub fn style_primary_button(button: &Button) {
	button.set_font(&Fonts::body_strong());
	if !use_custom_colours() {
		return;
	}
	button.set_background_color(Palette::ACCENT());
	button.set_foreground_color(Palette::ACCENT_TEXT());
}

/// Styles a heading StaticText.
pub fn style_heading(label: &StaticText) {
	label.set_font(&Fonts::heading());
}

/// Styles a heading StaticText that sits on the sidebar rather than on
/// content, so it keeps the sidebar's light-on-dark pairing.
pub fn style_sidebar_heading(label: &StaticText) {
	label.set_font(&Fonts::heading());
	if !use_custom_colours() {
		return;
	}
	label.set_foreground_color(Palette::SIDEBAR_TEXT());
}

/// Builds a plain `size`x`size` app glyph (a filled circle in the accent
/// colour) for use where a real icon asset isn't warranted, e.g. the tray
/// icon. Not affected by [`use_custom_colours`] — unlike widget chrome, a
/// tray icon has no native fallback to defer to.
pub fn build_app_icon(size: u32) -> wxdragon::bitmap::Bitmap {
	let mut rgba = vec![0u8; (size * size * 4) as usize];
	let center = (size as f32 - 1.0) / 2.0;
	let radius = size as f32 / 2.0 - 1.0;
	for y in 0..size {
		for x in 0..size {
			let dx = x as f32 - center;
			let dy = y as f32 - center;
			let dist = (dx * dx + dy * dy).sqrt();
			// One pixel of antialiasing at the edge keeps the circle from
			// looking jagged at the small sizes a tray icon is shown at.
			let coverage = (radius + 1.0 - dist).clamp(0.0, 1.0);
			let idx = ((y * size + x) * 4) as usize;
			rgba[idx] = Palette::ACCENT().r;
			rgba[idx + 1] = Palette::ACCENT().g;
			rgba[idx + 2] = Palette::ACCENT().b;
			rgba[idx + 3] = (coverage * 255.0) as u8;
		}
	}
	wxdragon::bitmap::Bitmap::from_rgba(&rgba, size, size).expect("failed to build app icon bitmap")
}

/// Styles a muted/secondary hint StaticText.
///
/// "Muted" is only a colour change: these labels stay at the full system
/// text size, because most of them name the field below them and shrinking a
/// field's label is the one place small text hurts most.
pub fn style_muted_label(label: &StaticText) {
	label.set_font(&Fonts::label());
	if !use_custom_colours() {
		return;
	}
	label.set_foreground_color(Palette::TEXT_MUTED());
}
