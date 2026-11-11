//! Several WhatsApp accounts in one window.
//!
//! The backend keeps every linked account open all the time, each with its own
//! WhatsApp client and database, so each one stays online and its messages and
//! calls still alert. The window shows one of them, the active account. Its
//! events go through the normal handlers;
//! the others' go to [`background_event`], which only alerts (named by account)
//! and keeps what alerting needs (muted, archived and locked chats).
//!
//! Switching starts the window from a fresh `AppState`, so nothing of the
//! previous account can be left over, and asks the backend to have the new
//! account greet the window again ("hello"), which fills it in like a fresh start.

use std::{
	cell::RefCell,
	collections::{HashMap, HashSet},
	rc::Rc,
	sync::Arc,
	time::{Duration, Instant},
};

use serde::Deserialize;
use wxdragon::prelude::*;

use crate::{
	AppState, UiCommand,
	backend::links::{self, Link},
	platform::accounts::{AccountState, CallAlerts, FIRST_ACCOUNT, Registry},
	ui::window::WindowParts,
};

/// How long a removed account's backend gets to log out of WhatsApp before
/// it is stopped anyway.
const LOGOUT_WAIT: Duration = Duration::from_secs(15);

/// What a background account's alerts need to know about its chats.
#[derive(Default)]
struct Background {
	muted: HashSet<String>,
	archived: HashSet<String>,
	locked: HashSet<String>,
	names: HashMap<String, String>,
	unread: HashSet<String>,
	group_quiet_until: HashMap<String, Instant>,
}

type Drafts = (HashMap<String, String>, HashMap<String, Vec<crate::ui_commands::mentions::MentionEntry>>);

pub(crate) struct Accounts {
	pub registry: Registry,
	links: HashMap<String, Link>,
	background: HashMap<String, Background>,
	/// Unsent drafts of the accounts not on screen.
	drafts: HashMap<String, Drafts>,
	/// While a new account is waiting to be linked: the account it was added
	/// from, to go back to if linking is given up.
	adding_from: Option<String>,
	/// Accounts being removed, waiting for their logout to finish.
	removing: HashSet<String>,
}

impl Accounts {
	/// Reads the accounts and finishes what an earlier run left: removals that
	/// couldn't delete everything, and accounts added but never linked.
	pub fn load_at_startup() -> Self {
		let mut registry = Registry::load();
		let mut changed = false;
		let active = registry.active.clone();
		for account in registry.accounts.clone() {
			let unfinished = account.state == AccountState::Deleting;
			let never_linked = account.state == AccountState::Pending
				&& account.id != FIRST_ACCOUNT
				&& account.phone.is_empty()
				&& (account.id != active || registry.live_count() > 1);
			if (unfinished || never_linked) && crate::platform::accounts::purge_files(&account.id) {
				registry.accounts.retain(|a| a.id != account.id);
				changed = true;
			}
		}
		if !registry.live().any(|a| a.id == registry.active) {
			let first = registry.live().next().map(|a| a.id.clone());
			registry.active = first.unwrap_or_else(|| FIRST_ACCOUNT.into());
			if registry.get(FIRST_ACCOUNT).is_none() && registry.live_count() == 0 {
				registry
					.accounts
					.push(crate::platform::accounts::Account { id: FIRST_ACCOUNT.into(), ..Default::default() });
			}
			changed = true;
		}
		if changed {
			let _ = registry.save();
		}
		Self {
			registry,
			links: HashMap::new(),
			background: HashMap::new(),
			drafts: HashMap::new(),
			adding_from: None,
			removing: HashSet::new(),
		}
	}

	pub fn with_links(mut self, links: Vec<Link>) -> Self {
		for link in links {
			self.links.insert(link.account.to_string(), link);
		}
		self
	}

	/// The account's link, opening it in the backend if it isn't open.
	/// The bool is true when it was already open.
	fn link(&mut self, id: &str) -> (Link, bool) {
		if let Some(link) = self.links.get(id) {
			return (link.clone(), true);
		}
		let link = links::start(id);
		self.links.insert(id.to_string(), link.clone());
		(link, false)
	}

	fn send_to(&self, id: &str, payload: serde_json::Value) {
		if let Some(link) = self.links.get(id) {
			let _ = link.ws_tx.send(payload.to_string());
		}
	}

	fn save(&self) {
		if let Err(err) = self.registry.save() {
			crate::backend::events::ui_timing_log(&format!("Accounts: couldn't save accounts.json ({err})"));
		}
	}
}

fn accounts(state: &Rc<RefCell<AppState>>) -> Rc<RefCell<Accounts>> {
	state.borrow().accounts.clone()
}

pub(crate) fn is_active(state: &Rc<RefCell<AppState>>, id: &str) -> bool {
	accounts(state).borrow().registry.active == id
}

/// Whether there is another linked account to go back to from the login window.
pub(crate) fn has_other_account(state: &Rc<RefCell<AppState>>) -> bool {
	let accounts = accounts(state);
	let a = accounts.borrow();
	a.registry.live().any(|x| x.id != a.registry.active && x.state == AccountState::Paired)
}

/// The window title names the account once there is more than one.
pub(crate) fn refresh_title(state: &Rc<RefCell<AppState>>, parts: &WindowParts) {
	let accounts = accounts(state);
	let a = accounts.borrow();
	let mut title = if a.registry.live_count() > 1 {
		format!("{} - whatSnap", a.registry.display_name(&a.registry.active))
	} else {
		"whatSnap".to_string()
	};
	if let Some(elsewhere) = whatsnap::unread_elsewhere_text(unread_elsewhere(&a)) {
		title.push_str(" - ");
		title.push_str(&elsewhere);
	}
	drop(a);
	if parts.frame.get_title() != title {
		parts.frame.set_title(&title);
	}
}

/// Unread chats of the accounts not on screen, for the window title and the
/// tray icon. Like the tray's own count, muted and archived chats don't
/// count, and neither do accounts whose message alerts are off.
pub(crate) fn unread_elsewhere(a: &Accounts) -> usize {
	a.registry
		.live()
		.filter(|x| x.id != a.registry.active && x.messages != crate::platform::accounts::MessageAlerts::Off)
		.filter_map(|x| a.background.get(&x.id))
		.map(|bg| bg.unread.iter().filter(|j| !bg.muted.contains(*j) && !bg.archived.contains(*j)).count())
		.sum()
}

/// The tray icon's text: the unread messages of the account on screen, and
/// the unread chats of the others.
pub(crate) fn tray_text(s: &AppState, unread: u64) -> String {
	let elsewhere = s.accounts.try_borrow().map(|a| unread_elsewhere(&a)).unwrap_or(0);
	match whatsnap::unread_elsewhere_text(elsewhere) {
		Some(elsewhere) => format!("{}, {elsewhere}", whatsnap::tray_tooltip_text(unread)),
		None => whatsnap::tray_tooltip_text(unread),
	}
}

/// Every event from every account comes through here.
pub(crate) fn backend_event(
	account: Arc<str>,
	payload: String,
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
) {
	note_identity(&account, &payload, state, parts);
	if is_active(state, &account) {
		crate::backend::events::process_websocket_message(payload, state, parts);
	} else {
		background_event(&account, &payload, state, parts);
		refresh_title(state, parts);
		crate::platform::tray::refresh_tray_tooltip(&state.borrow(), parts);
	}
}

#[derive(Deserialize)]
struct Envelope {
	#[serde(rename = "type")]
	ty: String,
	#[serde(default)]
	data: serde_json::Value,
}

fn has_type(payload: &str, ty: &str) -> bool {
	payload.strip_prefix(r#"{"type":""#).is_some_and(|rest| rest.starts_with(ty) && rest[ty.len()..].starts_with('"'))
}

/// Learns who an account is from its own events: its number when it
/// connects, its WhatsApp name from its profile.
fn note_identity(account: &str, payload: &str, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let connected = has_type(payload, "connected");
	if !connected && !has_type(payload, "own_profile") {
		return;
	}
	let Ok(env) = serde_json::from_str::<Envelope>(payload) else { return };
	let accounts = accounts(state);
	let (changed, newly_linked) = {
		let mut a = accounts.borrow_mut();
		let was_pending = a.registry.get(account).is_some_and(|x| x.state == AccountState::Pending);
		let changed = if connected {
			let jid = env.data.get("jid").and_then(|j| j.as_str());
			a.registry.set_identity(account, jid, None)
		} else {
			let name = env.data.get("pushName").and_then(|j| j.as_str());
			a.registry.set_identity(account, None, name)
		};
		let newly_linked = was_pending && a.registry.get(account).is_some_and(|x| x.state == AccountState::Paired);
		if changed {
			a.save();
		}
		if connected {
			// Its WhatsApp name, for menus and notifications.
			a.send_to(account, serde_json::json!({ "action": "get_own_profile" }));
		}
		(changed, newly_linked)
	};
	if newly_linked {
		linked_new_account(account, state, parts);
	}
	if changed {
		refresh_title(state, parts);
	}
}

/// A newly added account has been linked by its phone.
fn linked_new_account(account: &str, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let accounts = accounts(state);
	let duplicate = {
		let mut a = accounts.borrow_mut();
		if a.registry.active == account {
			a.adding_from = None;
		}
		a.registry.duplicate_of(account).map(|d| (d.id.clone(), d.display_name()))
	};
	refresh_title(state, parts);
	// The same number linked twice would be two copies of one account.
	if let Some((existing, name)) = duplicate {
		let msg = crate::i18n::tr_fmt("This phone number is already linked in whatSnap as {}, so the new link is being removed.", &[&name]);
		MessageDialog::builder(&parts.frame, &msg, crate::i18n::tr("Account Already Linked"))
			.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
			.build()
			.show_modal();
		if switch_to(state, parts, &existing) {
			start_removal(state, account, true);
		}
	}
}

#[derive(Deserialize)]
struct BackgroundChat {
	#[serde(rename = "JID", default)]
	jid: String,
	#[serde(rename = "FullName", default)]
	full_name: Option<String>,
	#[serde(rename = "UnreadCount", default)]
	unread_count: u64,
	#[serde(rename = "Muted", default)]
	muted: bool,
	#[serde(rename = "Archived", default)]
	archived: bool,
	#[serde(rename = "IsLocked", default)]
	is_locked: bool,
}

#[derive(Deserialize)]
struct BackgroundChats {
	#[serde(default)]
	data: Option<Vec<BackgroundChat>>,
}

/// An event from an account that isn't on screen.
fn background_event(account: &str, payload: &str, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	if has_type(payload, "chats") {
		if let Ok(BackgroundChats { data: Some(chats) }) = serde_json::from_str::<BackgroundChats>(payload) {
			let accounts = accounts(state);
			let mut a = accounts.borrow_mut();
			let bg = a.background.entry(account.to_string()).or_default();
			let was_unread = std::mem::take(&mut bg.unread);
			bg.muted.clear();
			bg.archived.clear();
			bg.locked.clear();
			for chat in chats {
				if chat.muted {
					bg.muted.insert(chat.jid.clone());
				}
				if chat.archived {
					bg.archived.insert(chat.jid.clone());
				}
				if chat.is_locked {
					bg.locked.insert(chat.jid.clone());
				}
				if chat.unread_count > 0 {
					bg.unread.insert(chat.jid.clone());
				}
				if let Some(name) = chat.full_name.filter(|n| !n.is_empty()) {
					bg.names.insert(chat.jid, name);
				}
			}
			// Read on the phone or another device: its toasts come down.
			for jid in was_unread.difference(&bg.unread) {
				crate::ui::chat_ui::clear_chat_notifications(jid);
			}
		}
		return;
	}
	// Only small events from here on; a big one (a chat's messages) is skipped
	// without being parsed.
	let wanted = ["connected", "message", "chat_updated", "incoming_call", "needs_login", "error"];
	if !wanted.iter().any(|ty| has_type(payload, ty)) {
		return;
	}
	let Ok(env) = serde_json::from_str::<Envelope>(payload) else { return };
	match env.ty.as_str() {
		"connected" => {
			accounts(state).borrow().send_to(account, serde_json::json!({ "action": "get_chats" }));
		}
		"message" => background_message(account, &env.data, state),
		"chat_updated" => {
			let jid = env.data.get("jid").and_then(|j| j.as_str()).unwrap_or_default();
			if env.data.get("unread_count").and_then(|u| u.as_i64()) == Some(0) && !jid.is_empty() {
				crate::ui::chat_ui::clear_chat_notifications(jid);
				if let Some(bg) = accounts(state).borrow_mut().background.get_mut(account) {
					bg.unread.remove(jid);
				}
			}
		}
		"incoming_call" => background_call(account, payload, &env.data, state, parts),
		"needs_login" | "error" => {
			if accounts(state).borrow().removing.contains(account) {
				finish_removal(state, account);
			} else if env.ty == "needs_login"
				&& let Some(reason) = env.data.get("reason").and_then(|r| r.as_str())
			{
				let name = accounts(state).borrow().registry.display_name(account);
				let reason_text = crate::i18n::translate_backend_message(reason);
				let toast_body = crate::i18n::tr_fmt("{} was logged out of WhatsApp: {}", &[&name, &reason_text]);
				crate::ui::chat_ui::show_toast_notification(whatsnap::TRAY_APP_NAME, &toast_body);
			}
		}
		_ => {}
	}
}

/// The alert for a message to an account that isn't on screen. The same rules
/// as for the account on screen (muted, archived, per-chat settings, quiet
/// groups), with the account named in the notification.
fn background_message(account: &str, data: &serde_json::Value, state: &Rc<RefCell<AppState>>) {
	let field = |k: &str| data.get(k).and_then(|v| v.as_str()).unwrap_or("");
	let jid = field("jid");
	let from = field("from");
	let text = field("text");
	let from_me = data.get("from_me").and_then(|f| f.as_bool()).unwrap_or(from == "me" || from == "You");
	let is_system = field("mediaType") == "system";
	let is_call_line = field("mediaType") == "call";
	let ignored = jid.is_empty() || jid.ends_with("@broadcast") || jid.ends_with("@newsletter");
	let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
	let backlog = whatsnap::is_backlog_message(data.get("timestamp").and_then(|t| t.as_i64()).unwrap_or(0), now);
	if from_me || is_system || ignored || backlog || (is_call_line && !text.starts_with("Missed")) {
		return;
	}

	let accounts = accounts(state);
	let mut a = accounts.borrow_mut();
	let account_name = a.registry.display_name(account);
	let (messages, calls) = a.registry.get(account).map(|x| (x.messages, x.calls)).unwrap_or_default();
	let bg = a.background.entry(account.to_string()).or_default();
	bg.unread.insert(jid.to_string());
	let addresses_me = data.get("addressesMe").and_then(|v| v.as_bool()).unwrap_or(false);
	let sender_chat = match (from.split_once(':'), from.split_once('@')) {
		(Some((user, _)), Some((_, server))) => format!("{user}@{server}"),
		_ => from.to_string(),
	};
	let reaches_me = addresses_me && !bg.muted.contains(&sender_chat);
	let s = state.borrow();
	let mut alert =
		s.settings.message_alert(jid, bg.muted.contains(jid) && !reaches_me, bg.archived.contains(jid) && !reaches_me);
	// The account's own choice comes on top of the chat's. A missed call
	// follows the account's call choice instead.
	if is_call_line {
		alert.sound &= calls != CallAlerts::Off && messages.sound();
		alert.toast &= calls != CallAlerts::Off;
	} else {
		alert.sound &= messages.sound();
		alert.toast &= messages.toast();
	}
	if jid.ends_with("@g.us") && (alert.sound || alert.toast) {
		let now = Instant::now();
		if bg.group_quiet_until.get(jid).is_some_and(|until| now < *until) && !addresses_me {
			alert = whatsnap::IncomingAlert { sound: false, toast: false };
		} else {
			let members = data.get("groupSize").and_then(|v| v.as_u64()).unwrap_or(0);
			let secs = (30 + 5 * members).min(120);
			bg.group_quiet_until.insert(jid.to_string(), now + Duration::from_secs(secs));
		}
	}
	if alert.sound && !crate::platform::do_not_disturb::should_silence(&s.settings) {
		let (tone, file) = s.settings.message_tone_for(jid);
		crate::audio::play_message_tone(tone, &file, s.settings.sounds_enabled, s.settings.output_device_name.clone());
	}
	if !alert.toast {
		return;
	}
	let clean = |j: &str| match j.find(':') {
		Some(i) => format!("{}@s.whatsapp.net", &j[..i]),
		None => j.to_string(),
	};
	let named = |key: &str| data.get(key).and_then(|n| n.as_str()).filter(|n| !n.is_empty()).map(str::to_string);
	let chat_name = named("chatName").or_else(|| bg.names.get(jid).cloned());
	let sender = named("senderName").or_else(|| bg.names.get(&clean(from)).cloned()).filter(|_| !from.is_empty());
	let title = if jid.ends_with("@g.us") {
		chat_name.clone().unwrap_or_else(|| crate::i18n::tr("New Group").to_string())
	} else {
		sender.clone().or(chat_name.clone()).unwrap_or_else(|| {
			crate::ui::contact_picker::display_name_for_jid(if from.is_empty() { jid } else { from })
		})
	};
	let title = format!("{title} ({account_name})");
	if bg.locked.contains(jid) {
		crate::ui::chat_ui::show_chat_notification_for(
			jid,
			&format!("WhatsApp ({account_name})"),
			crate::i18n::tr("New message"),
			Some(account),
			true,
			false,
		);
		return;
	}
	let previews = s.settings.notification_previews && !s.settings.chat_notify(jid).hide_content;
	let preview = whatsnap::friendly_preview_text(text);
	let body = match (previews, jid.ends_with("@g.us"), sender) {
		(false, _, _) => crate::i18n::tr("New message").to_string(),
		(true, true, Some(sender)) => format!("{sender}: {preview}"),
		(true, _, _) => preview,
	};
	crate::ui::chat_ui::show_chat_notification_for(
		jid,
		&title,
		&body,
		Some(account),
		false,
		crate::ui::chat_ui::can_reply_from_notification(jid),
	);
}

/// A call to an account that isn't on screen. With nothing else going on, the
/// window switches to that account and rings there, so it can be answered
/// like any call. Otherwise, or when the account is set to only notify, a
/// notification says who is calling which account. Set to off, nothing shows.
fn background_call(
	account: &str,
	payload: &str,
	data: &serde_json::Value,
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
) {
	let calls = accounts(state).borrow().registry.get(account).map(|x| x.calls).unwrap_or_default();
	if calls == CallAlerts::Off {
		return;
	}
	let free = busy(&state.borrow()).is_none();
	if calls == CallAlerts::Switch && free && switch_to(state, parts, account) {
		crate::backend::events::process_websocket_message(payload.to_string(), state, parts);
		return;
	}
	let caller = data
		.get("caller_name")
		.and_then(|n| n.as_str())
		.filter(|n| !n.is_empty())
		.unwrap_or_else(|| crate::i18n::tr("Someone"));
	let name = accounts(state).borrow().registry.display_name(account);
	let msg = crate::i18n::tr_fmt("{} is calling {}.", &[&caller, &name]);
	crate::ui::chat_ui::show_toast_notification(whatsnap::TRAY_APP_NAME, &msg);
}

/// Why the window can't switch accounts right now, if it can't.
fn busy(s: &AppState) -> Option<&'static str> {
	if s.active_call_id.is_some() || s.call_placing {
		Some(crate::i18n::tr("End the call before switching accounts."))
	} else if s.is_voice_recording {
		Some(crate::i18n::tr("Send or cancel the voice message before switching accounts."))
	} else {
		None
	}
}

/// Mark as read from a notification of an account that isn't on screen.
pub(crate) fn mark_read_in_background(state: &Rc<RefCell<AppState>>, account: &str, jid: &str) {
	crate::ui::chat_ui::clear_chat_notifications(jid);
	let accounts = accounts(state);
	let mut a = accounts.borrow_mut();
	a.send_to(account, serde_json::json!({ "action": "mark_read", "data": { "jid": jid, "message_ids": [] } }));
	if let Some(bg) = a.background.get_mut(account) {
		bg.unread.remove(jid);
	}
}

/// A reply typed into a notification from an account that isn't on screen:
/// sent from that account, which then marks the chat read.
pub(crate) fn reply_in_background(state: &Rc<RefCell<AppState>>, account: &str, jid: &str, text: &str) {
	{
		let accounts = accounts(state);
		let a = accounts.borrow();
		if a.background.get(account).is_some_and(|bg| bg.locked.contains(jid)) {
			return;
		}
		a.send_to(
			account,
			serde_json::json!({
				"action": "send",
				"data": { "to": jid, "text": text, "reply_to_id": "", "reply_to_sender": "" }
			}),
		);
	}
	mark_read_in_background(state, account, jid);
}

/// Shows another account in the window. False when it can't right now (a
/// call is on) or the account doesn't exist.
pub(crate) fn switch_to(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, target: &str) -> bool {
	let accounts = accounts(state);
	let from = accounts.borrow().registry.active.clone();
	if from == target {
		return true;
	}
	if !accounts.borrow().registry.live().any(|a| a.id == target) {
		return false;
	}
	// No borrow of the state may be held while the dialog is up: its focus
	// change sets presence, which borrows the state mutably.
	let reason = busy(&state.borrow());
	if let Some(reason) = reason {
		crate::platform::announce::announce(parts.conversations_parts.messages_announcer, reason);
		MessageDialog::builder(&parts.frame, reason, crate::i18n::tr("Switch Account"))
			.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
			.build()
			.show_modal();
		return false;
	}
	if state.borrow().selected_chat.is_some() {
		crate::ui_commands::chat_list::close_conversation(state, parts);
	}

	// Leaving an account that was added and never linked drops it.
	let abandon = from != FIRST_ACCOUNT
		&& accounts
			.borrow()
			.registry
			.get(&from)
			.is_some_and(|x| x.state == AccountState::Pending && x.phone.is_empty());
	let (ws_tx, running) = {
		let mut s = state.borrow_mut();
		let drafts = (std::mem::take(&mut s.draft_messages), std::mem::take(&mut s.draft_mentions));
		drop(s);
		let mut a = accounts.borrow_mut();
		a.drafts.insert(from.clone(), drafts);
		a.registry.active = target.to_string();
		if abandon || a.adding_from.as_deref() == Some(target) {
			a.adding_from = None;
		}
		a.save();
		let (link, running) = a.link(target);
		(link.ws_tx, running)
	};

	// A fresh state, so nothing of the previous account is left over. Only
	// what belongs to the app rather than to an account comes along.
	let fresh = {
		let s = state.borrow();
		crate::new_app_state(
			s.settings.clone(),
			ws_tx.clone(),
			s.ui_tx.clone(),
			crate::audio::OutputSlot::new("alert"),
			crate::audio::OutputSlot::new("call"),
			accounts.clone(),
		)
	};
	let old = {
		let mut s = state.borrow_mut();
		// The account we leave goes offline and stops typing, as if its
		// window had closed: nothing will update its presence from now on.
		if let Some(jid) = s.composing.as_ref().map(|c| c.jid.clone()) {
			crate::ui_commands::messaging::mark_paused(&mut s, &jid);
		}
		s.presence_sent = None;
		crate::ui_commands::misc::send_presence(&mut s, false);
		let mut fresh = fresh;
		// The account we switch to is as present as the window: online right
		// away if the window is in front and in use.
		fresh.presence = std::mem::take(&mut s.presence);
		std::mem::swap(&mut fresh.audio_out, &mut s.audio_out);
		std::mem::swap(&mut fresh.call_audio_out, &mut s.call_audio_out);
		fresh.keymap = s.keymap.clone();
		fresh.window_focused = s.window_focused;
		fresh.active_tab = s.active_tab;
		if let Some((messages, mentions)) = accounts.borrow_mut().drafts.remove(target) {
			fresh.draft_messages = messages;
			fresh.draft_mentions = mentions;
		}
		std::mem::replace(&mut *s, fresh)
	};
	drop(old);
	if abandon {
		start_removal(state, &from, false);
	}

	// Empty the window; the new account's greeting fills it in again.
	parts.pairing_dialog.show(false);
	parts.qrcode_dialog.show(false);
	parts.login_panel.show(false);
	parts.frame.show(true);
	parts.frame.set_status_text(crate::i18n::tr("Status: Connecting..."), 0);
	let tab = state.borrow().active_tab;
	crate::ui_commands::chat_list::nav_selected(state, parts, tab);
	crate::ui::window::refresh_menu_bar(&state.borrow(), &parts.frame);
	crate::platform::tray::refresh_tray_tooltip(&state.borrow(), parts);
	refresh_title(state, parts);
	// A backend that was already running greets again on request; a new one
	// greets the window when its pipe connects.
	if running {
		let _ = ws_tx.send(serde_json::json!({ "action": "hello" }).to_string());
	}
	let name = accounts.borrow().registry.display_name(target);
	let switched_msg =
		crate::i18n::tr_fmt("Switched to {}.", &[&name]);
	crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &switched_msg);
	parts.conversations_parts.conversations_list.set_focus();
	true
}

/// Account, Add Account: a new account waiting to be linked, shown at once
/// on the login window.
fn add_account(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let reason = busy(&state.borrow());
	if let Some(reason) = reason {
		MessageDialog::builder(&parts.frame, reason, crate::i18n::tr("Add Account"))
			.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
			.build()
			.show_modal();
		return;
	}
	let accounts = accounts(state);
	// Only one account at a time can be waiting to be linked.
	let pending = {
		let a = accounts.borrow();
		a.registry.live().find(|x| x.state == AccountState::Pending && x.phone.is_empty()).map(|x| x.id.clone())
	};
	let id = match pending {
		Some(id) => id,
		None => {
			let added = accounts.borrow_mut().registry.add("");
			match added {
				Ok(id) => {
					accounts.borrow().save();
					id
				}
				Err(err) => {
					let msg = crate::i18n::tr_fmt("Couldn't add an account: {}", &[&err]);
					MessageDialog::builder(&parts.frame, &msg, crate::i18n::tr("Add Account"))
						.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
						.build()
						.show_modal();
					return;
				}
			}
		}
	};
	let from = accounts.borrow().registry.active.clone();
	if from == id {
		return;
	}
	accounts.borrow_mut().adding_from = Some(from);
	if !switch_to(state, parts, &id) {
		accounts.borrow_mut().adding_from = None;
	}
}

/// The login window was closed while another account is linked: go back to
/// the account this one was added from, or to another linked one.
pub(crate) fn leave_login(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let accounts = accounts(state);
	let target = {
		let a = accounts.borrow();
		a.adding_from.clone().filter(|id| a.registry.live().any(|x| &x.id == id)).or_else(|| {
			a.registry
				.live()
				.find(|x| x.id != a.registry.active && x.state == AccountState::Paired)
				.map(|x| x.id.clone())
		})
	};
	if let Some(target) = target {
		switch_to(state, parts, &target);
	}
}

/// Marks an account removed and closes it in the backend; its files go once the
/// backend is down. A linked account logs out of WhatsApp first, so it also
/// leaves the phone's linked devices.
fn start_removal(state: &Rc<RefCell<AppState>>, id: &str, log_out: bool) {
	let accounts = accounts(state);
	{
		let mut a = accounts.borrow_mut();
		if let Some(account) = a.registry.get_mut(id) {
			account.state = AccountState::Deleting;
		}
		a.save();
		a.background.remove(id);
		a.drafts.remove(id);
		a.removing.insert(id.to_string());
		if log_out {
			a.send_to(id, serde_json::json!({ "action": "logout" }));
		}
	}
	if !log_out {
		finish_removal(state, id);
		return;
	}
	let ui_tx = state.borrow().ui_tx.clone();
	let id = id.to_string();
	std::thread::spawn(move || {
		std::thread::sleep(LOGOUT_WAIT);
		let _ = ui_tx.send(UiCommand::FinishAccountRemoval(id));
	});
}

/// The removed account has logged out (or ran out of time): close it.
pub(crate) fn finish_removal(state: &Rc<RefCell<AppState>>, id: &str) {
	let accounts = accounts(state);
	if !accounts.borrow_mut().removing.remove(id) {
		return;
	}
	accounts.borrow_mut().links.remove(id);
	let ui_tx = state.borrow().ui_tx.clone();
	let stopped = id.to_string();
	links::stop(id, move || {
		let _ = ui_tx.send(UiCommand::AccountStopped(stopped));
	});
}

/// The removed account's backend is down: delete its files. Whatever is still
/// in use stays marked and is finished on the next start.
pub(crate) fn account_stopped(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, id: &str) {
	let accounts = accounts(state);
	if crate::platform::accounts::purge_files(id) {
		let mut a = accounts.borrow_mut();
		a.registry.accounts.retain(|x| x.id != id);
		a.save();
	}
	refresh_title(state, parts);
}

fn account_rows(a: &Accounts) -> Vec<crate::ui::dialogs::accounts::AccountRow> {
	a.registry
		.live()
		.map(|x| {
			let mut label = x.display_name();
			if !x.phone.is_empty() && !label.starts_with('+') {
				label.push_str(&format!(", +{}", x.phone));
			}
			if x.id == a.registry.active {
				label.push_str(crate::i18n::tr(", on screen"));
			} else if x.state == AccountState::Pending {
				label.push_str(crate::i18n::tr(", not linked yet"));
			} else if let Some(bg) = a.background.get(&x.id)
				&& !bg.unread.is_empty()
			{
				let n = bg.unread.len();
				label.push_str(&crate::i18n::tr_fmt(if n == 1 { ", {} unread chat" } else { ", {} unread chats" }, &[&n]));
			}
			if x.messages == crate::platform::accounts::MessageAlerts::Off && x.calls == CallAlerts::Off {
				label.push_str(crate::i18n::tr(", notifications off"));
			}
			crate::ui::dialogs::accounts::AccountRow { id: x.id.clone(), label }
		})
		.collect()
}

/// Account, Accounts...
pub(crate) fn open_accounts(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, focus: Option<String>) {
	use crate::ui::dialogs::accounts::AccountsAction;
	let accounts = accounts(state);
	let (rows, active) = {
		let a = accounts.borrow();
		(account_rows(&a), a.registry.active.clone())
	};
	let action = if parts.login_panel.is_shown() {
		crate::ui::dialogs::accounts::show(&parts.login_panel, &rows, &active, focus.as_deref())
	} else {
		crate::ui::dialogs::accounts::show(&parts.frame, &rows, &active, focus.as_deref())
	};
	match action {
		Some(AccountsAction::Switch(id)) => {
			switch_to(state, parts, &id);
		}
		Some(AccountsAction::Add) => add_account(state, parts),
		Some(AccountsAction::Rename(id)) => {
			rename(state, parts, &id);
			let _ = state.borrow().ui_tx.send(UiCommand::OpenAccounts(Some(id)));
		}
		Some(AccountsAction::Remove(id)) => {
			remove(state, parts, &id);
		}
		Some(AccountsAction::Notifications(id)) => {
			notifications(state, parts, &id);
			let _ = state.borrow().ui_tx.send(UiCommand::OpenAccounts(Some(id)));
		}
		None => {}
	}
}

/// Accounts, Notifications...: how an account alerts while it isn't on screen.
fn notifications(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, id: &str) {
	let accounts = accounts(state);
	let Some((name, messages, calls)) =
		accounts.borrow().registry.get(id).map(|x| (x.display_name(), x.messages, x.calls))
	else {
		return;
	};
	let Some((messages, calls)) = crate::ui::dialogs::accounts::ask_notifications(&parts.frame, &name, messages, calls)
	else {
		return;
	};
	let mut a = accounts.borrow_mut();
	if let Some(account) = a.registry.get_mut(id) {
		account.messages = messages;
		account.calls = calls;
	}
	a.save();
	drop(a);
	refresh_title(state, parts);
	crate::platform::tray::refresh_tray_tooltip(&state.borrow(), parts);
}

fn rename(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, id: &str) {
	let accounts = accounts(state);
	let current = accounts.borrow().registry.get(id).map(|a| a.label.clone()).unwrap_or_default();
	let dialog = TextEntryDialog::builder(
		&parts.frame,
		crate::i18n::tr("Name for this account in whatSnap (leave empty to use its WhatsApp name):"),
		crate::i18n::tr("Rename Account"),
	)
	.with_default_value(&current)
	.build();
	if dialog.show_modal() != ID_OK {
		return;
	}
	let label = dialog.get_value().unwrap_or_default();
	{
		let mut a = accounts.borrow_mut();
		if let Some(account) = a.registry.get_mut(id) {
			account.label = label.trim().to_string();
		}
		a.save();
	}
	refresh_title(state, parts);
}

fn remove(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, id: &str) {
	let accounts = accounts(state);
	let (name, linked, others) = {
		let a = accounts.borrow();
		let Some(account) = a.registry.get(id) else { return };
		let others: Vec<String> = a.registry.live().filter(|x| x.id != id).map(|x| x.id.clone()).collect();
		(account.display_name(), account.state == AccountState::Paired, others)
	};
	if others.is_empty() {
		MessageDialog::builder(
			&parts.frame,
			crate::i18n::tr("This is the only account in whatSnap. To unlink it, use Account, Log Out."),
			crate::i18n::tr("Remove Account"),
		)
		.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
		.build()
		.show_modal();
		return;
	}
	let question = if linked {
		crate::i18n::tr_fmt("Remove {} from whatSnap?\n\nThis logs it out of WhatsApp on this computer and deletes its messages and media saved here. Your phone keeps its chats.", &[&name])
	} else { crate::i18n::tr_fmt("Remove {} from whatSnap?", &[&name]) };
	let answer = MessageDialog::builder(&parts.frame, &question, crate::i18n::tr("Remove Account"))
		.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconWarning)
		.build()
		.show_modal();
	if answer != ID_YES {
		return;
	}
	if is_active(state, id) {
		let next = {
			let a = accounts.borrow();
			others
				.iter()
				.find(|o| a.registry.get(o).is_some_and(|x| x.state == AccountState::Paired))
				.unwrap_or(&others[0])
				.clone()
		};
		if !switch_to(state, parts, &next) {
			return;
		}
	}
	// Switching away from a never-linked account already removed it.
	if accounts.borrow().registry.get(id).is_some_and(|x| x.state != AccountState::Deleting) {
		start_removal(state, id, linked);
	}
	let removed_msg =
		crate::i18n::tr_fmt("Removed {}.", &[&name]);
	crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &removed_msg);
	refresh_title(state, parts);
}
