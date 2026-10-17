//! The remappable keyboard shortcuts: the menu bar's, which are the only
//! app-wide ones, and the message list's action keys.
//!
//! wx builds its accelerator table from the "\tCtrl+N" part of each menu
//! label, so rebinding a menu shortcut means relabelling its menu item. The
//! message list's keys are checked by its key handler through
//! [`Keymap::is_pressed`]. Navigation and editing keys (arrows, Enter, Space,
//! Escape, Ctrl+C/V) stay fixed in their handlers and aren't listed here.
//!
//! What's saved is only what the user changed, keyed by a stable action id:
//! `"new_chat": "Ctrl+Shift+J"`, or `"none"` for a shortcut deliberately
//! removed. Anything absent keeps its default, so a default changed in a later
//! release still reaches everyone who never touched that one.

use std::collections::BTreeMap;

use key_chord::KeyChord;
use wx_utils::shortcuts::{ShortcutModel, TabScope};
use wxdragon::{event::KeyboardEvent, prelude::*};

use crate::ui::window::*;

/// Ids of the message list's actions, for [`Keymap::is_pressed`].
pub mod message_list {
	pub const SEEK_BACK: &str = "message_seek_back";
	pub const SEEK_FORWARD: &str = "message_seek_forward";
	pub const REPLY: &str = "message_reply";
	pub const EDIT: &str = "message_edit";
	pub const STAR: &str = "message_star";
	pub const FORWARD: &str = "message_forward";
	pub const VOTE: &str = "message_vote";
	pub const DELETE: &str = "message_delete";
	pub const SELECTION_MODE: &str = "message_selection_mode";
}

/// One remappable action.
pub struct ActionDef {
	/// Key in settings.json. Never rename one: it would silently drop the
	/// user's binding for it.
	pub id: &'static str,
	/// The menu item it's the shortcut of, or `None` for a message-list key.
	pub menu_id: Option<i32>,
	/// Index into [`TABS`]: the menu the item lives in, or the Messages tab.
	pub tab: usize,
	/// The menu label without its shortcut, `&` mnemonic included; for a
	/// message-list key, the name the shortcuts dialog lists it under.
	pub label: &'static str,
	pub default: &'static str,
}

pub const TABS: [&str; 7] = ["Account", "Go", "Chats", "Conversation", "Voice", "Call", "Messages"];

/// Log Out is left out on purpose: it has no shortcut, and giving it one would
/// make signing out of WhatsApp a single stray keypress away.
pub const ACTIONS: &[ActionDef] = &[
	ActionDef { id: "settings", menu_id: Some(MENU_ID_SETTINGS), tab: 0, label: "&Settings...", default: "Ctrl+," },
	ActionDef { id: "reconnect", menu_id: Some(MENU_ID_RECONNECT), tab: 0, label: "&Reconnect Now", default: "F5" },
	ActionDef {
		id: "go_conversations",
		menu_id: Some(MENU_ID_NAV_CONVERSATIONS),
		tab: 1,
		label: "&Conversations",
		default: "Alt+1",
	},
	ActionDef { id: "go_status", menu_id: Some(MENU_ID_NAV_STATUS), tab: 1, label: "&Status", default: "Alt+2" },
	ActionDef {
		id: "go_communities",
		menu_id: Some(MENU_ID_NAV_COMMUNITIES),
		tab: 1,
		label: "C&ommunities",
		default: "Alt+3",
	},
	ActionDef {
		id: "go_starred",
		menu_id: Some(MENU_ID_NAV_STARRED),
		tab: 1,
		label: "S&tarred Messages",
		default: "Alt+4",
	},
	ActionDef {
		id: "go_archived",
		menu_id: Some(MENU_ID_NAV_ARCHIVED),
		tab: 1,
		label: "&Archived Chats",
		default: "Alt+5",
	},
	ActionDef { id: "go_channels", menu_id: Some(MENU_ID_NAV_CHANNELS), tab: 1, label: "C&hannels", default: "Alt+6" },
	ActionDef { id: "new_chat", menu_id: Some(MENU_ID_NEW_CHAT), tab: 2, label: "&New Chat", default: "Ctrl+N" },
	ActionDef {
		id: "new_group",
		menu_id: Some(MENU_ID_NEW_GROUP),
		tab: 2,
		label: "New &Group",
		default: "Ctrl+Shift+N",
	},
	ActionDef { id: "search", menu_id: Some(MENU_ID_SEARCH_GLOBAL), tab: 2, label: "&Search", default: "Ctrl+/" },
	ActionDef {
		id: "search_in_chat",
		menu_id: Some(MENU_ID_SEARCH_CHAT),
		tab: 3,
		label: "Search in &Chat",
		default: "Ctrl+Shift+F",
	},
	ActionDef {
		id: "mute_chat",
		menu_id: Some(MENU_ID_MUTE_CHAT),
		tab: 3,
		label: "&Mute Chat",
		default: "Ctrl+Shift+M",
	},
	ActionDef {
		id: "archive_chat",
		menu_id: Some(MENU_ID_ARCHIVE_CHAT),
		tab: 3,
		label: "&Archive Chat",
		default: "Ctrl+Shift+E",
	},
	ActionDef { id: "pin_chat", menu_id: Some(MENU_ID_PIN_CHAT), tab: 3, label: "&Pin Chat", default: "Ctrl+Shift+P" },
	ActionDef { id: "lock_chat", menu_id: Some(MENU_ID_LOCK_CHAT), tab: 3, label: "&Lock Chat", default: "Ctrl+L" },
	ActionDef {
		id: "mark_unread",
		menu_id: Some(MENU_ID_MARK_UNREAD),
		tab: 3,
		label: "Mark as &Unread",
		default: "Ctrl+Shift+U",
	},
	ActionDef { id: "chat_info", menu_id: Some(MENU_ID_CHAT_INFO), tab: 3, label: "Chat &Info", default: "Alt+I" },
	ActionDef {
		id: "star_message",
		menu_id: Some(MENU_ID_STAR_MESSAGE),
		tab: 3,
		label: "&Star Message",
		default: "Alt+8",
	},
	ActionDef {
		id: "start_recording",
		menu_id: Some(MENU_ID_START_RECORDING),
		tab: 4,
		label: "Start &Recording",
		default: "Ctrl+Shift+R",
	},
	ActionDef {
		id: "pause_recording",
		menu_id: Some(MENU_ID_PAUSE_RECORDING),
		tab: 4,
		label: "&Pause Recording",
		default: "Alt+P",
	},
	ActionDef {
		id: "send_voice_message",
		menu_id: Some(MENU_ID_SEND_PTT),
		tab: 4,
		label: "Sen&d Voice Message",
		default: "Ctrl+Enter",
	},
	ActionDef { id: "call_mute", menu_id: Some(MENU_ID_CALL_MUTE), tab: 5, label: "Toggle &Mute", default: "Ctrl+M" },
	ActionDef { id: "call_end", menu_id: Some(MENU_ID_CALL_END), tab: 5, label: "&End Call", default: "Ctrl+W" },
	ActionDef { id: message_list::REPLY, menu_id: None, tab: 6, label: "Reply", default: "R" },
	ActionDef { id: message_list::EDIT, menu_id: None, tab: 6, label: "Edit", default: "E" },
	ActionDef { id: message_list::STAR, menu_id: None, tab: 6, label: "Star or unstar", default: "S" },
	ActionDef { id: message_list::FORWARD, menu_id: None, tab: 6, label: "Forward", default: "F" },
	ActionDef { id: message_list::VOTE, menu_id: None, tab: 6, label: "Vote in poll", default: "V" },
	ActionDef { id: message_list::DELETE, menu_id: None, tab: 6, label: "Delete", default: "Delete" },
	ActionDef {
		id: message_list::SEEK_BACK,
		menu_id: None,
		tab: 6,
		label: "Voice message: back 5 seconds",
		default: "J",
	},
	ActionDef {
		id: message_list::SEEK_FORWARD,
		menu_id: None,
		tab: 6,
		label: "Voice message: forward 5 seconds",
		default: "L",
	},
	ActionDef {
		id: message_list::SELECTION_MODE,
		menu_id: None,
		tab: 6,
		label: "Selection mode on or off",
		default: "Ctrl+Space",
	},
];

fn action_for_menu(menu_id: i32) -> usize {
	ACTIONS.iter().position(|a| a.menu_id == Some(menu_id)).expect("menu id has no entry in keymap::ACTIONS")
}

/// A shortcut as a wx accelerator reads it. wx has no physical-Control
/// modifier distinct from Ctrl, so a RawCtrl chord is written as Ctrl.
fn accelerator_text(chord: &KeyChord) -> String {
	if chord.raw_ctrl {
		KeyChord::new(true, chord.alt, chord.shift, chord.key.as_str()).to_shortcut_string()
	} else {
		chord.to_shortcut_string()
	}
}

/// The label a menu item is built with, before any saved overrides are
/// applied over it by [`Keymap::apply`].
pub fn default_menu_label(menu_id: i32) -> String {
	Keymap::default().menu_label(action_for_menu(menu_id))
}

/// The shortcuts in effect: the defaults with the user's overrides on top.
/// The action type handed to the shortcuts dialog is an index into [`ACTIONS`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keymap {
	overrides: BTreeMap<String, String>,
}

impl Keymap {
	/// Overrides for actions that no longer exist are dropped rather than kept
	/// around, so a stale settings.json can't resurrect a binding later.
	pub fn from_saved(saved: &BTreeMap<String, String>) -> Self {
		let overrides = saved
			.iter()
			.filter(|(id, _)| ACTIONS.iter().any(|a| a.id == id.as_str()))
			.map(|(id, chord)| (id.clone(), chord.clone()))
			.collect();
		Self { overrides }
	}

	pub fn saved(&self) -> BTreeMap<String, String> {
		self.overrides.clone()
	}

	pub fn chord_for(&self, action: usize) -> Option<KeyChord> {
		let def = &ACTIONS[action];
		KeyChord::parse(self.overrides.get(def.id).map_or(def.default, String::as_str))
	}

	pub fn menu_label(&self, action: usize) -> String {
		let shortcut = self.chord_for(action).map(|c| accelerator_text(&c)).unwrap_or_default();
		wx_utils::menu_label(ACTIONS[action].label, &shortcut)
	}

	/// Relabels every menu item, which is also what rebuilds wx's accelerator
	/// table, so the new shortcuts work straight away.
	pub fn apply(&self, frame: &Frame) {
		let Some(menu_bar) = frame.get_menu_bar() else { return };
		for (action, def) in ACTIONS.iter().enumerate() {
			let Some(menu_id) = def.menu_id else { continue };
			let shortcut = self.chord_for(action).map(|c| accelerator_text(&c)).unwrap_or_default();
			wx_utils::set_menu_item_label(&menu_bar, menu_id, def.label, &shortcut);
		}
	}

	/// Whether `key` is the chord bound to the action `id` (one of
	/// [`message_list`]'s). Modifiers have to match exactly, so R is Reply
	/// but Ctrl+R isn't.
	pub fn is_pressed(&self, id: &str, key: &KeyboardEvent) -> bool {
		let Some(action) = ACTIONS.iter().position(|a| a.id == id) else { return false };
		let Some(code) = key.get_key_code() else { return false };
		self.chord_for(action).is_some_and(|c| c.matches(code, key.control_down(), key.alt_down(), key.shift_down()))
	}
}

impl ShortcutModel for Keymap {
	type Action = usize;

	fn tabs(&self) -> Vec<String> {
		TABS.iter().map(|t| t.to_string()).collect()
	}

	/// One keymap across every tab: the menu tabs share wx's single
	/// accelerator table, and that table sees keys before the message list
	/// does, so a message-list key taken by a menu would never fire.
	fn tab_scope(&self, _tab: usize) -> TabScope {
		TabScope::Mode(0)
	}

	fn actions(&self, tab: usize) -> Vec<usize> {
		(0..ACTIONS.len()).filter(|&i| ACTIONS[i].tab == tab).collect()
	}

	fn action_name(&self, action: usize) -> String {
		ACTIONS[action].label.replace('&', "").trim_end_matches("...").to_string()
	}

	fn chord(&self, _tab: usize, action: usize) -> Option<KeyChord> {
		self.chord_for(action)
	}

	/// Setting an action back to its own default drops the override instead of
	/// storing it, so it keeps following the default from then on.
	fn set_chord(&mut self, _tab: usize, action: usize, chord: Option<KeyChord>) {
		let def = &ACTIONS[action];
		if chord == KeyChord::parse(def.default) {
			self.overrides.remove(def.id);
		} else {
			let text = chord.map_or_else(|| "none".to_string(), |c| c.to_shortcut_string());
			self.overrides.insert(def.id.to_string(), text);
		}
	}

	fn reset_action(&mut self, _tab: usize, action: usize) {
		self.overrides.remove(ACTIONS[action].id);
	}

	fn reset_all(&mut self, _tab: usize) {
		self.overrides.clear();
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn index_of(id: &str) -> usize {
		ACTIONS.iter().position(|a| a.id == id).unwrap()
	}

	#[test]
	fn ids_and_menu_ids_are_unique() {
		for (i, a) in ACTIONS.iter().enumerate() {
			for b in &ACTIONS[i + 1..] {
				assert_ne!(a.id, b.id);
				if a.menu_id.is_some() {
					assert_ne!(a.menu_id, b.menu_id);
				}
			}
		}
	}

	/// Numpad minus used to delete a message; it was compared as if it were
	/// numpad Delete.
	#[test]
	fn delete_is_the_delete_keys_only() {
		let delete = Keymap::default().chord_for(index_of(message_list::DELETE)).unwrap();
		assert!(delete.matches(WXK_DELETE, false, false, false));
		assert!(delete.matches(WXK_NUMPAD_DELETE, false, false, false));
		assert!(!delete.matches(WXK_NUMPAD_SUBTRACT, false, false, false));
		assert!(!delete.matches(WXK_CONTROL, false, false, false));
	}

	#[test]
	fn message_list_keys_need_their_exact_modifiers() {
		let reply = Keymap::default().chord_for(index_of(message_list::REPLY)).unwrap();
		assert!(reply.matches('R' as i32, false, false, false));
		assert!(!reply.matches('R' as i32, true, false, false));
	}

	#[test]
	fn message_list_keys_default_to_their_old_letters() {
		let keymap = Keymap::default();
		for (id, key) in [
			(message_list::REPLY, "R"),
			(message_list::EDIT, "E"),
			(message_list::STAR, "S"),
			(message_list::FORWARD, "F"),
			(message_list::VOTE, "V"),
			(message_list::SEEK_BACK, "J"),
			(message_list::SEEK_FORWARD, "L"),
			(message_list::DELETE, "Delete"),
			(message_list::SELECTION_MODE, "Ctrl+Space"),
		] {
			assert_eq!(keymap.chord_for(index_of(id)).unwrap().to_shortcut_string(), key);
		}
	}

	#[test]
	fn every_default_parses_and_no_two_collide() {
		let chords: Vec<KeyChord> = ACTIONS.iter().map(|a| KeyChord::parse(a.default).unwrap()).collect();
		for (i, a) in chords.iter().enumerate() {
			for b in &chords[i + 1..] {
				assert!(!a.conflicts_with(b), "{a:?} collides with {b:?}");
			}
		}
	}

	#[test]
	fn default_labels_match_the_old_hardcoded_menu() {
		assert_eq!(default_menu_label(MENU_ID_SETTINGS), "&Settings...\tCtrl+,");
		assert_eq!(default_menu_label(MENU_ID_NEW_GROUP), "New &Group\tCtrl+Shift+N");
		assert_eq!(default_menu_label(MENU_ID_SEARCH_GLOBAL), "&Search\tCtrl+/");
		assert_eq!(default_menu_label(MENU_ID_SEND_PTT), "Sen&d Voice Message\tCtrl+Enter");
		assert_eq!(default_menu_label(MENU_ID_RECONNECT), "&Reconnect Now\tF5");
	}

	#[test]
	fn every_action_is_on_a_tab() {
		let keymap = Keymap::default();
		let listed: usize = (0..TABS.len()).map(|t| keymap.actions(t).len()).sum();
		assert_eq!(listed, ACTIONS.len());
	}

	#[test]
	fn an_override_replaces_the_default_and_survives_a_save() {
		let mut keymap = Keymap::default();
		let new_chat = index_of("new_chat");
		keymap.set_chord(0, new_chat, KeyChord::parse("Ctrl+Shift+J"));
		let reloaded = Keymap::from_saved(&keymap.saved());
		assert_eq!(reloaded.menu_label(new_chat), "&New Chat\tCtrl+Shift+J");
	}

	#[test]
	fn clearing_a_shortcut_is_saved_as_none_not_as_the_default() {
		let mut keymap = Keymap::default();
		let lock = index_of("lock_chat");
		keymap.set_chord(0, lock, None);
		assert_eq!(keymap.saved().get("lock_chat").map(String::as_str), Some("none"));
		assert_eq!(Keymap::from_saved(&keymap.saved()).menu_label(lock), "&Lock Chat");
	}

	#[test]
	fn setting_the_default_again_drops_the_override() {
		let mut keymap = Keymap::default();
		let pin = index_of("pin_chat");
		keymap.set_chord(0, pin, KeyChord::parse("Alt+P"));
		keymap.set_chord(0, pin, KeyChord::parse("ctrl+shift+p"));
		assert!(keymap.saved().is_empty());
	}

	#[test]
	fn reset_drops_overrides() {
		let mut keymap = Keymap::default();
		keymap.set_chord(0, index_of("call_end"), None);
		keymap.set_chord(0, index_of("call_mute"), None);
		keymap.reset_action(0, index_of("call_end"));
		assert_eq!(keymap.saved().len(), 1);
		keymap.reset_all(0);
		assert!(keymap.saved().is_empty());
	}

	#[test]
	fn unknown_saved_ids_are_dropped() {
		let saved = BTreeMap::from([
			("gone_action".to_string(), "Ctrl+Q".to_string()),
			("reconnect".to_string(), "F6".to_string()),
		]);
		let keymap = Keymap::from_saved(&saved);
		assert_eq!(keymap.saved().len(), 1);
		assert_eq!(keymap.menu_label(index_of("reconnect")), "&Reconnect Now\tF6");
	}

	#[test]
	fn raw_ctrl_is_written_as_ctrl_for_wx() {
		let mut keymap = Keymap::default();
		let info = index_of("chat_info");
		keymap.set_chord(0, info, Some(KeyChord::new_raw_ctrl(true, false, false, "i")));
		assert_eq!(keymap.menu_label(info), "Chat &Info\tCtrl+I");
	}

	#[test]
	fn dialog_names_drop_mnemonics_and_ellipses() {
		let keymap = Keymap::default();
		assert_eq!(keymap.action_name(index_of("settings")), "Settings");
		assert_eq!(keymap.action_name(index_of("send_voice_message")), "Send Voice Message");
	}
}
