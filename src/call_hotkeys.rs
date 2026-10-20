//! System-wide keys for an incoming call. By default Alt+Shift+Y answers and
//! Alt+Shift+N rejects, whichever window has focus. Both are remappable in the
//! Keyboard Shortcuts dialog (Call tab), or removed there.
//!
//! They're registered only while the call rings (see `start`, and
//! `stop_call_ringing_sound`, which gives them back), so no other program loses
//! these chords the rest of the time. A change made in the dialog takes effect
//! the next time a call rings.

use key_chord::KeyChord;
use wx_utils::global_hotkeys::GlobalHotkeys;

use crate::{
	UiCommand,
	keymap::{Keymap, incoming_call},
	ui_wake::UiCommandSender,
};

#[derive(Clone, Copy)]
pub enum CallHotkey {
	Answer,
	Reject,
}

/// Registers the call keys from `keymap` for the call ringing now. Keep the
/// handle for as long as the call rings: dropping it gives the chords back.
/// Also returns the chords another program already holds, so the caller can
/// say so.
pub(crate) fn start(ui_tx: &UiCommandSender, keymap: &Keymap) -> (Option<GlobalHotkeys>, Vec<String>) {
	let bindings: Vec<(KeyChord, CallHotkey)> =
		[(incoming_call::ANSWER, CallHotkey::Answer), (incoming_call::REJECT, CallHotkey::Reject)]
			.into_iter()
			.filter_map(|(id, hotkey)| keymap.global_chord(id).map(|chord| (chord, hotkey)))
			.collect();
	if bindings.is_empty() {
		return (None, Vec::new());
	}
	// The callback runs on the hotkey thread, so it only hands the press to the UI thread.
	let ui_tx = ui_tx.clone();
	let (running, failed) = GlobalHotkeys::register(bindings, move |hotkey| {
		let _ = ui_tx.send(UiCommand::CallHotkey(hotkey));
	});
	(running, failed.iter().map(KeyChord::to_shortcut_string).collect())
}
