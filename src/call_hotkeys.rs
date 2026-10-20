//! System-wide keys for an incoming call: Alt+Shift+Y answers it and
//! Alt+Shift+R rejects it, whichever window has focus.
//!
//! They're registered only while the call rings (see `start`, and
//! `stop_call_ringing_sound`, which gives them back), so no other program loses
//! these chords the rest of the time.

use key_chord::KeyChord;
use wx_utils::global_hotkeys::GlobalHotkeys;

use crate::{UiCommand, ui_wake::UiCommandSender};

#[derive(Clone, Copy)]
pub enum CallHotkey {
	Answer,
	Reject,
}

/// Registers the two keys for the call ringing now. Keep the handle for as long
/// as the call rings: dropping it gives the chords back. Also returns the chords
/// another program already holds, so the caller can say so.
pub(crate) fn start(ui_tx: &UiCommandSender) -> (Option<GlobalHotkeys>, Vec<String>) {
	let bindings = vec![
		(KeyChord::new(false, true, true, "Y"), CallHotkey::Answer),
		(KeyChord::new(false, true, true, "R"), CallHotkey::Reject),
	];
	// The callback runs on the hotkey thread, so it only hands the press to the UI thread.
	let ui_tx = ui_tx.clone();
	let (running, failed) = GlobalHotkeys::register(bindings, move |hotkey| {
		let _ = ui_tx.send(UiCommand::CallHotkey(hotkey));
	});
	(running, failed.iter().map(KeyChord::to_shortcut_string).collect())
}
