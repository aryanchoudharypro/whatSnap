//! @mention autocomplete in the composer, shaped like an editor's completion
//! list: typing "@" offers everyone who can be mentioned, typing on narrows
//! it, Up/Down pick, Enter or Tab insert "@Name ", Escape dismisses.
//!
//! Focus never leaves the message field - a list that took focus would break
//! typing, and one that doesn't is invisible to a screen reader - so the
//! highlighted suggestion is spoken through the conversation's announcer, and
//! the list under the field is only there for sighted users.
//!
//! The field keeps the readable "@Name"; `composer_mentions` remembers which
//! JID each picked name stands for, and the backend swaps them for the
//! "@<number>" WhatsApp sends when the message goes out (backend/mentions.go).

use std::{
	cell::RefCell,
	rc::Rc,
	time::{Duration, Instant},
};

use wxdragon::prelude::*;

use crate::{
	AppState,
	ui::{conversations::ConversationsPanelParts, keys},
};

/// Group members are fetched on the first "@" in a chat, then reused this long.
const CANDIDATES_TTL: Duration = Duration::from_secs(10 * 60);
/// A query longer than this is someone typing prose after an "@", not a name.
const MAX_QUERY_CHARS: usize = 30;

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct MentionEntry {
	pub jid: String,
	pub name: String,
}

pub(crate) struct MentionCandidates {
	jid: String,
	/// Sorted by name; never includes us.
	members: Vec<MentionEntry>,
	/// Advanced chat privacy: Meta AI can't be mentioned while it's on.
	limit_sharing: bool,
	fetched: Instant,
}

pub(crate) struct MentionPopup {
	/// Char index of the "@" being completed.
	at: usize,
	matches: Vec<MentionEntry>,
	selected: usize,
}

fn meta_ai_entry() -> MentionEntry {
	MentionEntry { jid: crate::types::META_AI_JID.to_string(), name: "Meta AI".to_string() }
}

/// Only groups and 1:1 chats with people take mentions - not channels,
/// broadcasts, or a chat with Meta AI itself.
fn can_mention_in(jid: &str) -> bool {
	jid.ends_with("@g.us") || jid.ends_with("@s.whatsapp.net") || jid.ends_with("@lid")
}

/// The "@" the caret is completing, as (char index of the "@", text typed
/// after it). The "@" has to start a word, so an email address doesn't
/// count, and the query can hold spaces (names do) but not a line break.
pub(crate) fn find_mention_query(text: &str, caret: usize) -> Option<(usize, String)> {
	let chars: Vec<char> = text.chars().collect();
	let caret = caret.min(chars.len());
	let mut i = caret;
	while i > 0 {
		i -= 1;
		match chars[i] {
			'@' => {
				if i > 0 && chars[i - 1].is_alphanumeric() {
					return None;
				}
				let query: String = chars[i + 1..caret].iter().collect();
				if query.starts_with(char::is_whitespace) {
					return None;
				}
				return Some((i, query));
			}
			'\n' | '\r' => return None,
			_ if caret - i > MAX_QUERY_CHARS => return None,
			_ => {}
		}
	}
	None
}

/// How many positions a char takes in the native Windows edit control: UTF-16
/// units, with a line break counting as two ("\r\n") though GetValue gives "\n".
fn wx_width(c: char) -> i64 {
	if c == '\n' && cfg!(windows) { 2 } else { c.len_utf16() as i64 }
}

fn wx_pos_to_char(text: &str, pos: i64) -> usize {
	let mut at = 0;
	for (i, c) in text.chars().enumerate() {
		if at >= pos {
			return i;
		}
		at += wx_width(c);
	}
	text.chars().count()
}

fn char_to_wx_pos(text: &str, index: usize) -> i64 {
	text.chars().take(index).map(wx_width).sum()
}

fn matches_query(name: &str, query: &str) -> bool {
	if query.is_empty() {
		return true;
	}
	let name = name.to_lowercase();
	let query = query.to_lowercase();
	name.starts_with(&query)
		|| name.split_whitespace().any(|word| word.starts_with(&query))
		|| (name.starts_with('+') && name[1..].replace(' ', "").starts_with(&query.replace(' ', "")))
}

fn filter(candidates: &MentionCandidates, query: &str) -> Vec<MentionEntry> {
	let meta_ai = (!candidates.limit_sharing).then(meta_ai_entry);
	meta_ai.into_iter().chain(candidates.members.iter().cloned()).filter(|e| matches_query(&e.name, query)).collect()
}

fn describe(popup: &MentionPopup) -> String {
	let entry = &popup.matches[popup.selected];
	format!("{}, {} of {}", entry.name, popup.selected + 1, popup.matches.len())
}

fn hide_list(cp: &ConversationsPanelParts) {
	if cp.mention_list.is_shown() {
		cp.mention_list.show(false);
		cp.conversation_panel.layout();
	}
}

/// Drops the suggestions without dismissing them, e.g. when the chat changes
/// or the message is sent. Takes the already-borrowed state.
pub(crate) fn close(s: &mut AppState, cp: &ConversationsPanelParts) {
	s.mention_popup = None;
	s.mention_dismissed_at = None;
	hide_list(cp);
}

/// Re-reads the composer and opens, narrows or closes the suggestions. Runs on
/// every text change, which set_value() also fires while callers hold the
/// state - those are never typing, so a busy state just means "not now".
pub(crate) fn refresh(state: &Rc<RefCell<AppState>>, cp: &ConversationsPanelParts) {
	let field = cp.message_field;
	let text = field.get_value();
	let caret = wx_pos_to_char(&text, field.get_insertion_point());
	let focused = field.has_focus();
	let Ok(mut s) = state.try_borrow_mut() else { return };
	let jid = s.selected_chat.clone().filter(|j| can_mention_in(j));
	let found = find_mention_query(&text, caret).filter(|_| focused && jid.is_some());
	let Some((at, query)) = found else {
		s.mention_dismissed_at = None;
		let was_open = s.mention_popup.take().is_some();
		drop(s);
		if was_open {
			hide_list(cp);
		}
		return;
	};
	let jid = jid.unwrap_or_default();
	if s.mention_dismissed_at == Some(at) {
		return;
	}
	s.mention_dismissed_at = None;

	let fresh = s.mention_candidates.as_ref().is_some_and(|c| c.jid == jid && c.fetched.elapsed() < CANDIDATES_TTL);
	if !fresh && s.mention_request_pending.as_deref() != Some(jid.as_str()) {
		s.mention_request_pending = Some(jid.clone());
		let payload = serde_json::json!({ "action": "get_mention_candidates", "data": { "jid": jid } });
		let _ = s.ws_tx.send(payload.to_string());
	}
	// Until the first answer for this chat comes back there's nothing to
	// offer; its arrival runs this again.
	let Some(candidates) = s.mention_candidates.as_ref().filter(|c| c.jid == jid) else { return };
	let matches = filter(candidates, &query);
	if matches.is_empty() {
		let was_open = s.mention_popup.take().is_some();
		drop(s);
		if was_open {
			hide_list(cp);
		}
		return;
	}
	let previous = s.mention_popup.take();
	let opening = previous.is_none();
	let changed = previous.as_ref().is_none_or(|p| p.matches != matches);
	// Keep the highlight on the same person while the list narrows around them.
	let selected = previous
		.as_ref()
		.and_then(|p| p.matches.get(p.selected))
		.and_then(|e| matches.iter().position(|m| m.jid == e.jid))
		.unwrap_or(0);
	let popup = MentionPopup { at, matches, selected };
	let announcement = describe(&popup);
	let names: Vec<String> = popup.matches.iter().map(|e| e.name.clone()).collect();
	s.mention_popup = Some(popup);
	let announcer = cp.messages_announcer;
	drop(s);

	if changed {
		cp.mention_list.clear();
		let items: Vec<&str> = names.iter().map(String::as_str).collect();
		cp.mention_list.append_items(&items);
		cp.mention_list.set_selection(selected as u32, true);
		if !cp.mention_list.is_shown() {
			cp.mention_list.show(true);
			cp.conversation_panel.layout();
		}
		let message = if opening { format!("Mention suggestions. {announcement}") } else { announcement };
		crate::announce::announce(announcer, &message);
	}
}

/// Keys for the open suggestions, from the composer's key-down handler.
/// Returns true when the key was used up here.
pub(crate) fn handle_key(
	state: &Rc<RefCell<AppState>>,
	cp: &ConversationsPanelParts,
	code: Option<i32>,
	shift: bool,
	ctrl: bool,
) -> bool {
	// Ctrl+Space brings back suggestions dismissed with Escape, as in an editor.
	if ctrl && keys::is_space(code) {
		if let Ok(mut s) = state.try_borrow_mut() {
			s.mention_dismissed_at = None;
		}
		refresh(state, cp);
		return state.try_borrow().is_ok_and(|s| s.mention_popup.is_some());
	}
	let Ok(mut s) = state.try_borrow_mut() else { return false };
	let Some(popup) = s.mention_popup.as_mut() else { return false };
	if keys::is_up(code) || keys::is_down(code) {
		let count = popup.matches.len();
		popup.selected =
			if keys::is_down(code) { (popup.selected + 1) % count } else { (popup.selected + count - 1) % count };
		let selected = popup.selected;
		let announcement = describe(popup);
		drop(s);
		cp.mention_list.set_selection(selected as u32, true);
		// Interrupts: arrowing through names should speak the one landed on.
		live_region::announce_with_priority(cp.messages_announcer, &announcement, live_region::Priority::High);
		return true;
	}
	if (keys::is_enter(code) || keys::is_tab(code)) && !shift && !ctrl {
		drop(s);
		accept(state, cp);
		return true;
	}
	if keys::is_escape(code) {
		s.mention_dismissed_at = Some(popup.at);
		s.mention_popup = None;
		drop(s);
		hide_list(cp);
		return true;
	}
	// Moving the caret sideways leaves the name being typed.
	if keys::is_left(code) || keys::is_right(code) || keys::is_home(code) || keys::is_end(code) {
		s.mention_popup = None;
		drop(s);
		hide_list(cp);
	}
	false
}

/// Replaces "@query" with "@Name " for the highlighted suggestion.
pub(crate) fn accept(state: &Rc<RefCell<AppState>>, cp: &ConversationsPanelParts) {
	let field = cp.message_field;
	let text = field.get_value();
	let caret = wx_pos_to_char(&text, field.get_insertion_point());
	let Ok(mut s) = state.try_borrow_mut() else { return };
	let Some(popup) = s.mention_popup.take() else { return };
	let Some(entry) = popup.matches.get(popup.selected).cloned() else { return };
	// Set before the edit: the text change it fires must not reopen the list
	// on this same "@".
	s.mention_dismissed_at = Some(popup.at);
	if !s.composer_mentions.iter().any(|m| m.jid == entry.jid) {
		s.composer_mentions.push(entry.clone());
	}
	let announcer = cp.messages_announcer;
	drop(s);

	hide_list(cp);
	let insert = format!("@{} ", entry.name);
	let from = char_to_wx_pos(&text, popup.at);
	let to = char_to_wx_pos(&text, caret.max(popup.at));
	field.replace(from, to, &insert);
	field.set_insertion_point(from + insert.chars().map(wx_width).sum::<i64>());
	crate::announce::announce(announcer, &format!("{} mentioned", entry.name));
}

/// A suggestion clicked in the list: picked as if highlighted and accepted.
pub(crate) fn pick(state: &Rc<RefCell<AppState>>, cp: &ConversationsPanelParts, index: usize) {
	{
		let Ok(mut s) = state.try_borrow_mut() else { return };
		match s.mention_popup.as_mut() {
			Some(popup) if index < popup.matches.len() => popup.selected = index,
			_ => return,
		}
	}
	cp.message_field.set_focus();
	accept(state, cp);
}

/// The mentions to send with `text`: those picked whose "@Name" is still in it.
pub(crate) fn mentions_payload(s: &AppState, text: &str) -> serde_json::Value {
	serde_json::Value::Array(
		s.composer_mentions
			.iter()
			.filter(|m| text.contains(&format!("@{}", m.name)))
			.map(|m| serde_json::json!({ "jid": m.jid, "name": m.name }))
			.collect(),
	)
}

/// The backend's `mention_candidates` answer.
pub(crate) fn on_candidates(state: &Rc<RefCell<AppState>>, cp: &ConversationsPanelParts, data: &serde_json::Value) {
	let Some(jid) = data.get("jid").and_then(|j| j.as_str()) else { return };
	let mut members: Vec<MentionEntry> = data
		.get("members")
		.and_then(|m| m.as_array())
		.map(|list| {
			list.iter()
				.filter_map(|m| {
					let jid = m.get("jid")?.as_str()?.to_string();
					let name = m.get("name")?.as_str()?.trim().to_string();
					(!name.is_empty()).then_some(MentionEntry { jid, name })
				})
				.collect()
		})
		.unwrap_or_default();
	members.sort_by_key(|e| e.name.to_lowercase());
	let limit_sharing = data.get("limit_sharing").and_then(|l| l.as_bool()).unwrap_or(false);
	{
		let mut s = state.borrow_mut();
		if s.mention_request_pending.as_deref() == Some(jid) {
			s.mention_request_pending = None;
		}
		s.mention_candidates =
			Some(MentionCandidates { jid: jid.to_string(), members, limit_sharing, fetched: Instant::now() });
	}
	refresh(state, cp);
}

/// A chat's advanced chat privacy changed: Meta AI leaves or rejoins its list.
pub(crate) fn on_limit_sharing(state: &Rc<RefCell<AppState>>, cp: &ConversationsPanelParts, jid: &str, enabled: bool) {
	{
		let mut s = state.borrow_mut();
		match s.mention_candidates.as_mut() {
			Some(c) if c.jid == jid => c.limit_sharing = enabled,
			_ => return,
		}
	}
	refresh(state, cp);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn finds_the_mention_being_typed() {
		assert_eq!(find_mention_query("@", 1), Some((0, String::new())));
		assert_eq!(find_mention_query("hi @Har", 7), Some((3, "Har".into())));
		assert_eq!(find_mention_query("hi @Hari Nar", 12), Some((3, "Hari Nar".into())));
		assert_eq!(find_mention_query("(@Ha", 4), Some((1, "Ha".into())));
		assert_eq!(find_mention_query("mail a@b.com", 12), None);
		assert_eq!(find_mention_query("@ x", 3), None);
		assert_eq!(find_mention_query("@Hari\nthere", 11), None);
		assert_eq!(find_mention_query("@Hari there", 5), Some((0, "Hari".into())));
		assert_eq!(find_mention_query(&format!("@{}", "a".repeat(40)), 41), None);
	}

	#[test]
	fn converts_native_positions() {
		let text = "a\nb😀c";
		let nl = if cfg!(windows) { 2 } else { 1 };
		assert_eq!(char_to_wx_pos(text, 3), 2 + nl);
		// The emoji is two UTF-16 units.
		assert_eq!(char_to_wx_pos(text, 4), 4 + nl);
		assert_eq!(wx_pos_to_char(text, 4 + nl), 4);
		assert_eq!(wx_pos_to_char(text, 100), 5);
	}

	#[test]
	fn filters_by_any_word_and_hides_meta_ai_under_chat_privacy() {
		let mut candidates = MentionCandidates {
			jid: "1@g.us".into(),
			members: vec![
				MentionEntry { jid: "1@lid".into(), name: "Hari Narayan".into() },
				MentionEntry { jid: "2@lid".into(), name: "+91 98765 43210".into() },
			],
			limit_sharing: false,
			fetched: Instant::now(),
		};
		assert_eq!(filter(&candidates, "").len(), 3);
		assert_eq!(filter(&candidates, "")[0].name, "Meta AI");
		assert_eq!(filter(&candidates, "nar")[0].name, "Hari Narayan");
		assert_eq!(filter(&candidates, "9198")[0].jid, "2@lid");
		assert_eq!(filter(&candidates, "meta").len(), 1);
		candidates.limit_sharing = true;
		assert!(filter(&candidates, "meta").is_empty());
		assert_eq!(filter(&candidates, "").len(), 2);
	}
}
