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

/// WhatsApp Web's own list, word for word, with each emoji's name and an
/// example for the help (see `show_help`).
const EMOTICONS: &[(&str, &str, &str, &str)] = &[
	("(y)", "\u{1F44D}", "thumbs up", "Sounds good (y)"),
	("(Y)", "\u{1F44D}", "thumbs up", "See you at 6 (Y)"),
	("(n)", "\u{1F44E}", "thumbs down", "Not a fan (n)"),
	("(N)", "\u{1F44E}", "thumbs down", "The traffic today (N)"),
	(":-)", "\u{1F642}", "slightly smiling face", "Thanks :-)"),
	(":-(", "\u{1F641}", "slightly frowning face", "I missed the bus :-("),
	(":-p", "\u{1F61B}", "face with tongue", "Just kidding :-p"),
	(":-P", "\u{1F61B}", "face with tongue", "Caught you :-P"),
	(":-\\", "\u{1F615}", "confused face", "Wait, what :-\\"),
	(":-D", "\u{1F600}", "grinning face", "That's hilarious :-D"),
	(":-o", "\u{1F62E}", "face with open mouth", "No way :-o"),
	(";-)", "\u{1F609}", "winking face", "Our secret ;-)"),
	(":-*", "\u{1F618}", "face blowing a kiss", "Good night :-*"),
	(":-|", "\u{1F610}", "neutral face", "Another Monday :-|"),
	(":'(", "\u{1F622}", "crying face", "I'll miss you :'("),
	("^_^", "\u{1F601}", "beaming face with smiling eyes", "Happy birthday ^_^"),
	("<3", "\u{2764}\u{FE0F}", "red heart", "Love you <3"),
	(">_<", "\u{1F606}", "grinning squinting face", "Oops >_<"),
];

/// The emoticon `before` ends with, as (the emoticon, its emoji). Two
/// characters are tried before three, as WhatsApp does.
fn replacement(before: &[u16]) -> Option<(&'static str, &'static str)> {
	(2..=3).find_map(|len| {
		let tail = before.get(before.len().checked_sub(len)?..)?;
		let tail = String::from_utf16(tail).ok()?;
		EMOTICONS.iter().find(|(text, ..)| *text == tail).map(|&(text, emoji, ..)| (text, emoji))
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

/// The help's text: what the setting does, then every emoticon with its
/// emoji, its name, and an example as typed and as sent.
fn help_text() -> String {
	let mut text = String::from(
		"Replace text with emoji turns these into emoji as soon as you type their last character. \
		 Press Backspace straight after to get the text back. You can turn it off on the Chats tab of Settings.\r\n",
	);
	for (typed, emoji, name, example) in EMOTICONS {
		let sent = example.replace(typed, emoji);
		text.push_str(&format!("\r\nType {typed} for {emoji} {name}. Example: \"{example}\" becomes \"{sent}\"."));
	}
	text
}

/// Shows `help_text` in a read-only box, for reading line by line.
pub fn show_help(parent: &impl WxWidget) {
	let dlg = Dialog::builder(parent, "Replace Text With Emoji")
		.with_size(520, 420)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	let panel = Panel::builder(&dlg).build();
	let sizer = BoxSizer::builder(Orientation::Vertical).build();
	let label = StaticText::builder(&panel).with_label("Text that turns into emoji").build();
	sizer.add(&label, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 8);
	let view = TextCtrl::builder(&panel)
		.with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly | TextCtrlStyle::Rich2)
		.build();
	view.set_value(&help_text());
	sizer.add(&view, 1, SizerFlag::Expand | SizerFlag::All, 8);
	let close_btn = Button::builder(&panel).with_id(wxdragon::id::ID_OK).with_label("Close").build();
	sizer.add(&close_btn, 0, SizerFlag::AlignRight | SizerFlag::All, 8);
	panel.set_sizer(sizer, true);
	view.on_key_down(super::keys::escape_cancels(dlg));
	view.set_focus();
	view.set_selection(0, 0);
	dlg.show_modal();
	// A dialog built per use is destroyed, or it lives on hidden.
	dlg.destroy();
}

#[cfg(test)]
mod tests {
	use super::{help_text, replacement};

	#[test]
	fn the_help_shows_each_example_as_sent() {
		let help = help_text();
		assert!(help.contains(
			"Type :-) for \u{1F642} slightly smiling face. Example: \"Thanks :-)\" becomes \"Thanks \u{1F642}\"."
		));
		assert!(help.contains("Type <3 for"));
		assert_eq!(help.matches("\r\nType ").count(), super::EMOTICONS.len());
	}

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
