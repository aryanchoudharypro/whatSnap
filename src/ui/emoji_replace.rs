//! WhatsApp's "Replace text with emoji": typing one of a few emoticons turns
//! it into its emoji as soon as its last character is typed, and Backspace
//! straight after turns it back (WAWebEmojisPlugin). Works on the message
//! box's native edit control, in its own UTF-16 positions.

use std::cell::{Cell, RefCell};

use windows::Win32::{
	Foundation::{HWND, LPARAM, WPARAM},
	UI::{
		Controls::{EM_GETSEL, EM_REPLACESEL, EM_SETSEL},
		WindowsAndMessaging::{GetWindowTextLengthW, GetWindowTextW, SendMessageW},
	},
};
use wxdragon::prelude::*;

/// WhatsApp Web's own list, word for word.
const EMOTICONS: &[(&str, &str)] = &[
	("(y)", "\u{1F44D}"),
	("(Y)", "\u{1F44D}"),
	("(n)", "\u{1F44E}"),
	("(N)", "\u{1F44E}"),
	(":-)", "\u{1F642}"),
	(":-(", "\u{1F641}"),
	(":-p", "\u{1F61B}"),
	(":-P", "\u{1F61B}"),
	(":-\\", "\u{1F615}"),
	(":-D", "\u{1F600}"),
	(":-o", "\u{1F62E}"),
	(";-)", "\u{1F609}"),
	(":-*", "\u{1F618}"),
	(":-|", "\u{1F610}"),
	(":'(", "\u{1F622}"),
	("^_^", "\u{1F601}"),
	("<3", "\u{2764}\u{FE0F}"),
	(">_<", "\u{1F606}"),
];

/// The emoticon `before` ends with, as (the emoticon, its emoji). Two
/// characters are tried before three, as WhatsApp does.
fn replacement(before: &[u16]) -> Option<(&'static str, &'static str)> {
	(2..=3).find_map(|len| {
		let tail = before.get(before.len().checked_sub(len)?..)?;
		let tail = String::from_utf16(tail).ok()?;
		EMOTICONS.iter().find(|(text, _)| *text == tail).copied()
	})
}

/// The last replacement, for Backspace: where its emoji starts, how long it
/// is, and the text it replaced.
struct Last {
	start: u32,
	len: u32,
	text: &'static str,
}

thread_local! {
	static LAST: RefCell<Option<Last>> = const { RefCell::new(None) };
	static REPLACING: Cell<bool> = const { Cell::new(false) };
}

fn hwnd(field: &TextCtrl) -> HWND {
	HWND(field.get_handle().cast())
}

fn selection(hwnd: HWND) -> (u32, u32) {
	let (mut start, mut end) = (0u32, 0u32);
	unsafe {
		SendMessageW(
			hwnd,
			EM_GETSEL,
			Some(WPARAM(&mut start as *mut u32 as usize)),
			Some(LPARAM(&mut end as *mut u32 as isize)),
		);
	}
	(start, end)
}

fn text(hwnd: HWND) -> Vec<u16> {
	unsafe {
		let len = GetWindowTextLengthW(hwnd).max(0) as usize;
		let mut buf = vec![0u16; len + 1];
		let got = GetWindowTextW(hwnd, &mut buf).max(0) as usize;
		buf.truncate(got);
		buf
	}
}

/// Replaces `from..to` with `with`, as typing would (it can be undone).
fn replace(hwnd: HWND, from: u32, to: u32, with: &str) {
	let mut wide: Vec<u16> = with.encode_utf16().collect();
	wide.push(0);
	REPLACING.with(|r| r.set(true));
	unsafe {
		SendMessageW(hwnd, EM_SETSEL, Some(WPARAM(from as usize)), Some(LPARAM(to as isize)));
		SendMessageW(hwnd, EM_REPLACESEL, Some(WPARAM(1)), Some(LPARAM(wide.as_ptr() as isize)));
	}
	REPLACING.with(|r| r.set(false));
}

/// Call on every change to the message box, with whether the setting is
/// on. Only typing counts: text put in while the box isn't focused (a draft
/// coming back) is left as it is.
pub fn text_changed(field: &TextCtrl, enabled: bool) {
	if REPLACING.with(Cell::get) {
		return;
	}
	LAST.with(|l| l.borrow_mut().take());
	if !enabled || !field.has_focus() {
		return;
	}
	let hwnd = hwnd(field);
	let (start, end) = selection(hwnd);
	if start != end {
		return;
	}
	let all = text(hwnd);
	let Some(before) = all.get(..end as usize) else { return };
	let Some((typed, emoji)) = replacement(before) else { return };
	// Emoticons are plain ASCII: one UTF-16 unit a character.
	let from = end - typed.len() as u32;
	replace(hwnd, from, end, emoji);
	let len = emoji.encode_utf16().count() as u32;
	LAST.with(|l| *l.borrow_mut() = Some(Last { start: from, len, text: typed }));
}

/// Backspace right after a replacement puts the typed text back. Returns
/// whether it did, so the key isn't also handled as a normal Backspace.
pub fn undo_on_backspace(field: &TextCtrl) -> bool {
	let Some(last) = LAST.with(|l| l.borrow_mut().take()) else { return false };
	let hwnd = hwnd(field);
	let (start, end) = selection(hwnd);
	if start != end || end != last.start + last.len {
		return false;
	}
	replace(hwnd, last.start, end, last.text);
	true
}

#[cfg(test)]
mod tests {
	use super::replacement;

	fn wide(s: &str) -> Vec<u16> {
		s.encode_utf16().collect()
	}

	#[test]
	fn an_emoticon_just_typed_is_found() {
		assert_eq!(replacement(&wide("hi :-)")), Some((":-)", "\u{1F642}")));
		assert_eq!(replacement(&wide("love <3")), Some(("<3", "\u{2764}\u{FE0F}")));
		assert_eq!(replacement(&wide("ok (Y)")), Some(("(Y)", "\u{1F44D}")));
	}

	#[test]
	fn other_text_is_left_alone() {
		assert_eq!(replacement(&wide("hi :)")), None);
		assert_eq!(replacement(&wide(":-) and more")), None);
		assert_eq!(replacement(&wide("")), None);
	}
}
