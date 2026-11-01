use key_chord::KeyChord;
use wx_utils::global_hotkeys::GlobalHotkeys;

use crate::{
	UiCommand,
	keymap::{Keymap, incoming_call},
	ui::wake::UiCommandSender,
};

#[derive(Clone, Copy)]
pub enum CallHotkey {
	Answer,
	Reject,
}

pub(crate) fn start(ui_tx: &UiCommandSender, keymap: &Keymap) -> (Option<GlobalHotkeys>, Vec<String>) {
	let bindings: Vec<(KeyChord, CallHotkey)> =
		[(incoming_call::ANSWER, CallHotkey::Answer), (incoming_call::REJECT, CallHotkey::Reject)]
			.into_iter()
			.filter_map(|(id, hotkey)| keymap.global_chord(id).map(|chord| (chord, hotkey)))
			.collect();
	if bindings.is_empty() {
		return (None, Vec::new());
	}
	let ui_tx = ui_tx.clone();
	let (running, failed) = GlobalHotkeys::register(bindings, move |hotkey| {
		let _ = ui_tx.send(UiCommand::CallHotkey(hotkey));
	});
	(running, failed.iter().map(KeyChord::to_shortcut_string).collect())
}
