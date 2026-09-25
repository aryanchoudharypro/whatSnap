//! [`KeyChord`]: a physical key combination, with parsing and formatting for the
//! `"Ctrl+Shift+O"` strings a config file stores, and matching against raw wx key codes.
//!
//! This crate is deliberately dependency-free (`serde` is an optional feature) so that a
//! GUI-free core library can own an app's keymap and defaults without linking a toolkit,
//! while the same type reaches the shortcut-editing UI unchanged.

#![warn(clippy::all, clippy::cargo, clippy::nursery, clippy::pedantic)]

/// A modifier combination plus one key.
///
/// `ctrl` and `raw_ctrl` are separate because macOS has two keys where Windows and Linux have
/// one. `ctrl` is the *command* modifier, which wx maps to Cmd on macOS and to Ctrl elsewhere,
/// and is what an ordinary menu accelerator wants. `raw_ctrl` is the physical Control key,
/// which on macOS is a different key and on other platforms is the same one. A binding only
/// needs `raw_ctrl` when it has to stay physical Control on a Mac, so [`new`](KeyChord::new)
/// produces a `ctrl` chord and [`new_raw_ctrl`](KeyChord::new_raw_ctrl) is the exception you
/// reach for deliberately.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct KeyChord {
	pub ctrl: bool,
	#[cfg_attr(feature = "serde", serde(default))]
	pub raw_ctrl: bool,
	pub alt: bool,
	pub shift: bool,
	pub key: String,
}

impl KeyChord {
	/// A chord on the command modifier: Cmd on macOS, Ctrl elsewhere.
	#[must_use]
	pub fn new(ctrl: bool, alt: bool, shift: bool, key: impl Into<String>) -> Self {
		let key_str = key.into();
		let normalized = Self::normalize_key_name(&key_str);
		Self { ctrl, raw_ctrl: false, alt, shift, key: normalized }
	}

	/// A chord on the physical Control key, which stays Control on macOS rather than becoming Cmd.
	#[must_use]
	pub fn new_raw_ctrl(raw_ctrl: bool, alt: bool, shift: bool, key: impl Into<String>) -> Self {
		let key_str = key.into();
		let normalized = Self::normalize_key_name(&key_str);
		Self { ctrl: false, raw_ctrl, alt, shift, key: normalized }
	}

	/// Canonical spelling of a key name, so `"esc"`, `"Escape"` and `"ESC"` all compare equal
	/// and a config file edited by hand still matches.
	#[must_use]
	pub fn normalize_key_name(key: &str) -> String {
		let trimmed = key.trim();
		if trimmed.eq_ignore_ascii_case("return") || trimmed.eq_ignore_ascii_case("enter") {
			"Enter".to_string()
		} else if trimmed.eq_ignore_ascii_case("space") {
			"Space".to_string()
		} else if trimmed.eq_ignore_ascii_case("tab") {
			"Tab".to_string()
		} else if trimmed.eq_ignore_ascii_case("backspace") || trimmed.eq_ignore_ascii_case("back") {
			"Backspace".to_string()
		} else if trimmed.eq_ignore_ascii_case("delete") || trimmed.eq_ignore_ascii_case("del") {
			"Delete".to_string()
		} else if trimmed.eq_ignore_ascii_case("escape") || trimmed.eq_ignore_ascii_case("esc") {
			"Escape".to_string()
		} else if trimmed.eq_ignore_ascii_case("home") {
			"Home".to_string()
		} else if trimmed.eq_ignore_ascii_case("end") {
			"End".to_string()
		} else if trimmed.eq_ignore_ascii_case("pageup")
			|| trimmed.eq_ignore_ascii_case("page up")
			|| trimmed.eq_ignore_ascii_case("pgup")
		{
			"PageUp".to_string()
		} else if trimmed.eq_ignore_ascii_case("pagedown")
			|| trimmed.eq_ignore_ascii_case("page down")
			|| trimmed.eq_ignore_ascii_case("pgdn")
		{
			"PageDown".to_string()
		} else if trimmed.eq_ignore_ascii_case("left") || trimmed.eq_ignore_ascii_case("left arrow") {
			"Left".to_string()
		} else if trimmed.eq_ignore_ascii_case("right") || trimmed.eq_ignore_ascii_case("right arrow") {
			"Right".to_string()
		} else if trimmed.eq_ignore_ascii_case("up") || trimmed.eq_ignore_ascii_case("up arrow") {
			"Up".to_string()
		} else if trimmed.eq_ignore_ascii_case("down") || trimmed.eq_ignore_ascii_case("down arrow") {
			"Down".to_string()
		} else if trimmed.len() >= 2
			&& trimmed.starts_with(['F', 'f'])
			&& trimmed[1..].chars().all(|c| c.is_ascii_digit())
		{
			format!("F{}", &trimmed[1..])
		} else if trimmed.len() == 1 {
			let ch = trimmed.chars().next().unwrap_or_default();
			if ch.is_ascii_alphabetic() { ch.to_ascii_uppercase().to_string() } else { trimmed.to_string() }
		} else {
			trimmed.to_string()
		}
	}

	/// The `"Ctrl+Shift+O"` form, which is both what the user reads and what a config file stores.
	#[must_use]
	pub fn to_shortcut_string(&self) -> String {
		let mut parts = Vec::new();
		if self.raw_ctrl {
			parts.push("RawCtrl");
		}
		if self.ctrl {
			parts.push("Ctrl");
		}
		if self.alt {
			parts.push("Alt");
		}
		if self.shift {
			parts.push("Shift");
		}
		parts.push(&self.key);
		parts.join("+")
	}

	/// Parses [`to_shortcut_string`](KeyChord::to_shortcut_string)'s output back.
	///
	/// Returns `None` for an empty string or the literal `"none"`, which is how a config file
	/// spells "this action deliberately has no shortcut".
	#[must_use]
	pub fn parse(input: &str) -> Option<Self> {
		let trimmed = input.trim();
		if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
			return None;
		}
		let mut ctrl = false;
		let mut raw_ctrl = false;
		let mut alt = false;
		let mut shift = false;
		let mut remaining = trimmed;
		while let Some(plus_idx) = remaining.find('+') {
			let prefix = &remaining[..plus_idx];
			if prefix.eq_ignore_ascii_case("rawctrl") {
				raw_ctrl = true;
				remaining = &remaining[plus_idx + 1..];
			} else if prefix.eq_ignore_ascii_case("ctrl") || prefix.eq_ignore_ascii_case("control") {
				ctrl = true;
				remaining = &remaining[plus_idx + 1..];
			} else if prefix.eq_ignore_ascii_case("alt") {
				alt = true;
				remaining = &remaining[plus_idx + 1..];
			} else if prefix.eq_ignore_ascii_case("shift") {
				shift = true;
				remaining = &remaining[plus_idx + 1..];
			} else {
				break;
			}
		}
		let key = remaining.to_string();
		if key.is_empty() {
			return None;
		}
		let normalized = Self::normalize_key_name(&key);
		Some(Self { ctrl, raw_ctrl, alt, shift, key: normalized })
	}

	/// whaNTV patch: the numpad codes here are wxWidgets' `WXK_NUMPAD_*` values
	/// (375 Home, 376 Left, 377 Up, 378 Right, 379 Down, 380 PageUp, 381
	/// PageDown, 382 End, 385 Delete, 391 Decimal, 392 Divide). The published
	/// 0.1.0 had them shifted, so numpad Up read as Home, 308 (Ctrl) as Delete
	/// and numpad `*` as `.`.
	///
	/// Builds a chord from a live key event.
	///
	/// Returns `None` for a key code with no name here, such as a bare modifier press. That is
	/// what lets a capture field ignore the user reaching for Shift on the way to a chord.
	#[must_use]
	pub fn from_key_code(key_code: i32, ctrl: bool, alt: bool, shift: bool) -> Option<Self> {
		let named = |code: i32| char::from_u32(u32::try_from(code).ok()?).map(|c| c.to_string());
		let key_name = match key_code {
			13 | 370 => "Enter".to_string(),
			9 => "Tab".to_string(),
			32 => "Space".to_string(),
			8 => "Backspace".to_string(),
			127 | 385 => "Delete".to_string(),
			27 => "Escape".to_string(),
			313 | 375 => "Home".to_string(),
			312 | 382 => "End".to_string(),
			366 | 380 => "PageUp".to_string(),
			367 | 381 => "PageDown".to_string(),
			314 | 376 => "Left".to_string(),
			316 | 378 => "Right".to_string(),
			315 | 377 => "Up".to_string(),
			317 | 379 => "Down".to_string(),
			340..=363 => format!("F{}", key_code - 340 + 1),
			65..=90 | 48..=57 => named(key_code)?,
			97..=122 => named(key_code - 32)?,
			324..=333 => named(key_code - 324 + 48)?,
			44 | 188 => ",".to_string(),
			46 | 190 | 391 => ".".to_string(),
			47 | 191 | 392 => "/".to_string(),
			91 | 219 => "[".to_string(),
			93 | 221 => "]".to_string(),
			92 | 220 => "\\".to_string(),
			45 | 189 | 390 => "-".to_string(),
			61 | 187 => "=".to_string(),
			59 | 186 => ";".to_string(),
			39 | 222 => "'".to_string(),
			96 | 192 => "`".to_string(),
			_ => return None,
		};
		Some(Self { ctrl, raw_ctrl: false, alt, shift, key: key_name })
	}

	/// Whether a live key event is this chord.
	///
	/// Each key accepts both its ASCII code and its numpad or extended equivalent, so a binding
	/// on `.` fires from the numpad decimal point too.
	#[must_use]
	pub fn matches(&self, key_code: i32, ctrl: bool, alt: bool, shift: bool) -> bool {
		let self_ctrl = self.ctrl || self.raw_ctrl;
		if self_ctrl != ctrl || self.alt != alt || self.shift != shift {
			return false;
		}
		let key_str = self.key.as_str();
		if key_str.eq_ignore_ascii_case("Enter") {
			key_code == 13 || key_code == 370
		} else if key_str.eq_ignore_ascii_case("Tab") {
			key_code == 9
		} else if key_str.eq_ignore_ascii_case("Space") {
			key_code == 32
		} else if key_str.eq_ignore_ascii_case("Backspace") {
			key_code == 8
		} else if key_str.eq_ignore_ascii_case("Delete") {
			key_code == 127 || key_code == 385
		} else if key_str.eq_ignore_ascii_case("Escape") {
			key_code == 27
		} else if key_str.eq_ignore_ascii_case("Home") {
			key_code == 313 || key_code == 375
		} else if key_str.eq_ignore_ascii_case("End") {
			key_code == 312 || key_code == 382
		} else if key_str.eq_ignore_ascii_case("PageUp") {
			key_code == 366 || key_code == 380
		} else if key_str.eq_ignore_ascii_case("PageDown") {
			key_code == 367 || key_code == 381
		} else if key_str.eq_ignore_ascii_case("Left") {
			key_code == 314 || key_code == 376
		} else if key_str.eq_ignore_ascii_case("Right") {
			key_code == 316 || key_code == 378
		} else if key_str.eq_ignore_ascii_case("Up") {
			key_code == 315 || key_code == 377
		} else if key_str.eq_ignore_ascii_case("Down") {
			key_code == 317 || key_code == 379
		} else if key_str.starts_with(['F', 'f'])
			&& let Ok(num) = key_str[1..].parse::<i32>()
			&& (1..=24).contains(&num)
		{
			key_code == 340 + num - 1
		} else if key_str.len() == 1 {
			let ch = key_str.chars().next().unwrap_or_default();
			if ch.is_ascii_alphabetic() {
				let upper = ch.to_ascii_uppercase() as i32;
				key_code == upper || key_code == upper + 32
			} else if ch.is_ascii_digit() {
				key_code == ch as i32 || key_code == (ch as i32 - 48 + 324)
			} else {
				match ch {
					',' => key_code == 44 || key_code == 188,
					'.' => key_code == 46 || key_code == 190 || key_code == 391,
					'/' => key_code == 47 || key_code == 191 || key_code == 392,
					'\\' => key_code == 92 || key_code == 220,
					'[' => key_code == 91 || key_code == 219,
					']' => key_code == 93 || key_code == 221,
					'-' => key_code == 45 || key_code == 189 || key_code == 390,
					'=' => key_code == 61 || key_code == 187,
					';' => key_code == 59 || key_code == 186,
					'\'' => key_code == 39 || key_code == 222,
					'`' => key_code == 96 || key_code == 192,
					_ => key_code == ch as i32,
				}
			}
		} else {
			false
		}
	}

	/// Whether two chords fire on the same keystroke, and so cannot both be bound.
	///
	/// Not `==`: a `ctrl` chord and a `raw_ctrl` chord on the same key are different values but
	/// the same keystroke everywhere except macOS, so binding both is a conflict.
	#[must_use]
	pub fn conflicts_with(&self, other: &Self) -> bool {
		(self.ctrl || self.raw_ctrl) == (other.ctrl || other.raw_ctrl)
			&& self.alt == other.alt
			&& self.shift == other.shift
			&& self.key == other.key
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn numpad_navigation_keys_are_their_own_names() {
		for (code, name) in [
			(375, "Home"),
			(376, "Left"),
			(377, "Up"),
			(378, "Right"),
			(379, "Down"),
			(380, "PageUp"),
			(381, "PageDown"),
			(382, "End"),
			(385, "Delete"),
			(391, "."),
			(392, "/"),
		] {
			let chord = KeyChord::from_key_code(code, false, false, false).unwrap();
			assert_eq!(chord.key, name);
			assert!(chord.matches(code, false, false, false));
		}
	}

	#[test]
	fn modifier_keys_and_other_numpad_keys_are_not_named_keys() {
		assert_eq!(KeyChord::from_key_code(308, true, false, false), None);
		assert!(!KeyChord::parse("Delete").unwrap().matches(386, false, false, false));
		assert!(!KeyChord::parse(".").unwrap().matches(387, false, false, false));
		assert!(!KeyChord::parse("/").unwrap().matches(388, false, false, false));
	}

	#[test]
	fn parses_and_formats_a_modifier_chord() {
		let chord = KeyChord::parse("Ctrl+Shift+O").unwrap();
		assert!(chord.ctrl);
		assert!(chord.shift);
		assert!(!chord.alt);
		assert!(!chord.raw_ctrl);
		assert_eq!(chord.key, "O");
		assert_eq!(chord.to_shortcut_string(), "Ctrl+Shift+O");
	}

	#[test]
	fn parses_a_bare_key() {
		let single = KeyChord::parse("H").unwrap();
		assert!(!single.ctrl && !single.shift && !single.alt);
		assert_eq!(single.to_shortcut_string(), "H");
	}

	#[test]
	fn an_unbound_action_parses_to_nothing() {
		assert_eq!(KeyChord::parse("none"), None);
		assert_eq!(KeyChord::parse(""), None);
	}

	#[test]
	fn raw_ctrl_survives_a_round_trip() {
		let chord = KeyChord::new_raw_ctrl(true, false, false, "Space");
		assert_eq!(chord.to_shortcut_string(), "RawCtrl+Space");
		assert_eq!(KeyChord::parse("RawCtrl+Space"), Some(chord));
	}

	#[test]
	fn key_names_normalize_to_one_spelling() {
		assert_eq!(KeyChord::normalize_key_name("esc"), "Escape");
		assert_eq!(KeyChord::normalize_key_name("return"), "Enter");
		assert_eq!(KeyChord::normalize_key_name("page up"), "PageUp");
		assert_eq!(KeyChord::normalize_key_name("f5"), "F5");
		assert_eq!(KeyChord::normalize_key_name("o"), "O");
	}

	#[test]
	fn ctrl_and_raw_ctrl_on_one_key_are_a_conflict() {
		// Different values, but the same keystroke everywhere except macOS.
		let cmd = KeyChord::new(true, false, false, "K");
		let control = KeyChord::new_raw_ctrl(true, false, false, "K");
		assert_ne!(cmd, control);
		assert!(cmd.conflicts_with(&control));
	}

	#[test]
	fn a_different_modifier_is_not_a_conflict() {
		let plain = KeyChord::new(true, false, false, "K");
		let shifted = KeyChord::new(true, false, true, "K");
		assert!(!plain.conflicts_with(&shifted));
	}

	#[test]
	fn a_chord_matches_its_own_key_event() {
		let chord = KeyChord::new(true, true, false, "K");
		assert!(chord.matches(75, true, true, false));
		assert!(!chord.matches(75, true, false, false));
	}

	#[test]
	fn a_key_matches_its_numpad_equivalent() {
		let chord = KeyChord::new(false, false, false, ".");
		assert!(chord.matches(46, false, false, false));
		assert!(chord.matches(391, false, false, false));
	}

	#[test]
	fn a_raw_ctrl_chord_matches_a_ctrl_key_event() {
		// wx reports the physical Control key as `control_down` on every platform.
		let chord = KeyChord::new_raw_ctrl(true, false, false, "Space");
		assert!(chord.matches(32, true, false, false));
	}

	#[test]
	fn a_bare_modifier_press_captures_nothing() {
		assert_eq!(KeyChord::from_key_code(306, false, false, true), None);
	}
}
