use std::{
	collections::{HashMap, hash_map::DefaultHasher},
	hash::{Hash, Hasher},
	sync::{
		Mutex, MutexGuard, OnceLock,
		atomic::{AtomicU64, Ordering},
	},
	time::Instant,
};

use windows::{
	Data::Xml::Dom::XmlDocument,
	UI::Notifications::{ToastNotification, ToastNotificationManager},
	core::HSTRING,
};

use crate::platform::app_link;

pub const APP_ID: &str = "whatSnap";

pub fn start_menu_shortcut_path() -> String {
	match std::env::var_os("APPDATA") {
		Some(appdata) => std::path::PathBuf::from(appdata)
			.join("Microsoft")
			.join("Windows")
			.join("Start Menu")
			.join("Programs")
			.join("whatSnap.lnk")
			.display()
			.to_string(),
		None => "whatSnap".to_string(),
	}
}

fn group_for(jid: &str) -> HSTRING {
	let mut hasher = DefaultHasher::new();
	jid.hash(&mut hasher);
	HSTRING::from(format!("c{:015x}", hasher.finish() & 0x0fff_ffff_ffff_ffff))
}

fn next_tag() -> HSTRING {
	static COUNTER: AtomicU64 = AtomicU64::new(0);
	let nanos =
		std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
	let n = COUNTER.fetch_add(1, Ordering::Relaxed);
	HSTRING::from(format!("{:016x}", nanos.wrapping_add(n)))
}

fn escape_xml(text: &str) -> String {
	let mut out = String::with_capacity(text.len());
	for c in text.chars() {
		match c {
			'&' => out.push_str("&amp;"),
			'<' => out.push_str("&lt;"),
			'>' => out.push_str("&gt;"),
			'"' => out.push_str("&quot;"),
			'\'' => out.push_str("&apos;"),
			_ => out.push(c),
		}
	}
	out
}

/// How a click reaches the app: the registered activator when it's up (see
/// [`crate::platform::toast_activator`]), else a `whatsnap://` link that
/// starts a second copy which hands it on.
fn activation(foreground: bool) -> &'static str {
	match (crate::platform::toast_activator::is_ready(), foreground) {
		(false, _) => "protocol",
		(true, true) => "foreground",
		(true, false) => "background",
	}
}

fn toast_xml(title: &str, body: &str, jid: &str, account: Option<&str>, is_locked: bool, can_reply: bool) -> String {
	let open = escape_xml(&app_link::open_link(jid, account));
	let actions = if is_locked {
		String::new()
	} else {
		let mark_read = escape_xml(&app_link::mark_read_link(jid, account));
		// Typed text only reaches the app through the activator, so without
		// it there is no reply box.
		let reply = if can_reply && crate::platform::toast_activator::is_ready() {
			let reply = escape_xml(&app_link::reply_link(jid, account));
			format!(
				concat!(
					r#"<input id="{input}" type="text" placeHolderContent="{placeholder}"/>"#,
					r#"<action content="{send}" arguments="{reply}" activationType="background" hint-inputId="{input}"/>"#,
				),
				input = REPLY_INPUT,
				reply = reply,
				placeholder = escape_xml(crate::i18n::tr("Type a reply")),
				send = escape_xml(crate::i18n::tr("Send")),
			)
		} else {
			String::new()
		};
		format!(
			r#"<actions>{reply}<action content="{mark_read_label}" arguments="{mark_read}" activationType="{background}"/></actions>"#,
			reply = reply,
			mark_read_label = escape_xml(crate::i18n::tr("Mark as read")),
			mark_read = mark_read,
			background = activation(false),
		)
	};
	format!(
		concat!(
			r#"<toast launch="{open}" activationType="{foreground}">"#,
			r#"<visual><binding template="ToastGeneric">"#,
			r#"<text>{title}</text><text>{body}</text>"#,
			r#"</binding></visual>"#,
			"{actions}",
			r#"</toast>"#,
		),
		open = open,
		foreground = activation(true),
		actions = actions,
		title = escape_xml(title),
		body = escape_xml(body),
	)
}

/// A toast with no chat behind it (the catch-up summary, an incoming call, a
/// problem to report). A click brings whatSnap to the front.
fn plain_toast_xml(title: &str, body: &str) -> String {
	let launch = escape_xml(&app_link::activate_link());
	format!(
		concat!(
			r#"<toast launch="{launch}" activationType="{foreground}">"#,
			r#"<visual><binding template="ToastGeneric">"#,
			r#"<text>{title}</text><text>{body}</text>"#,
			r#"</binding></visual>"#,
			r#"</toast>"#,
		),
		launch = launch,
		foreground = activation(true),
		title = escape_xml(title),
		body = escape_xml(body),
	)
}

/// The id of the reply box, which the activator gets the typed text under.
pub const REPLY_INPUT: &str = "reply";

fn show(xml: &str, group: Option<&HSTRING>) -> windows::core::Result<()> {
	let document = XmlDocument::new()?;
	document.LoadXml(&HSTRING::from(xml))?;
	let toast = ToastNotification::CreateToastNotification(&document)?;
	toast.SetTag(&next_tag())?;
	if let Some(group) = group {
		toast.SetGroup(group)?;
	}
	let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))?;
	notifier.Show(&toast)
}

/// Shows a message toast for `jid`, filed in that chat's group. `account` is
/// set for a chat of a linked account that isn't the one on screen, so a click
/// switches to it. When `is_locked` is true, the toast has no reply box or
/// "Mark as read". Must run on the UI thread, like the rest of the WinRT toast
/// code in this app.
pub fn show_chat_toast(
	jid: &str,
	title: &str,
	body: &str,
	account: Option<&str>,
	is_locked: bool,
	can_reply: bool,
) -> windows::core::Result<()> {
	show(&toast_xml(title, body, jid, account, is_locked, can_reply), Some(&group_for(jid)))?;
	shown_chats().insert(jid.to_string(), Instant::now());
	Ok(())
}

/// Shows a toast that isn't about one chat. Must run on the UI thread.
pub fn show_plain_toast(title: &str, body: &str) -> windows::core::Result<()> {
	show(&plain_toast_xml(title, body), None)
}

/// The chats that have toasts up, and when the last one was shown.
fn shown_chats() -> MutexGuard<'static, HashMap<String, Instant>> {
	static SHOWN: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
	SHOWN.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

pub fn clear_chat_toasts(jid: &str) {
	shown_chats().remove(jid);
	if let Ok(history) = ToastNotificationManager::History() {
		let _ = history.RemoveGroupWithId(&group_for(jid), &HSTRING::from(APP_ID));
	}
}

/// A chat list just came in: takes down the toasts of chats it shows as read
/// (on the phone, on another device, or here). A toast younger than a few
/// seconds is left alone, since the list may predate its message.
pub fn clear_read_chat_toasts(is_read: impl Fn(&str) -> bool) {
	const SETTLE: std::time::Duration = std::time::Duration::from_secs(5);
	let read: Vec<String> = shown_chats()
		.iter()
		.filter(|(jid, at)| at.elapsed() >= SETTLE && is_read(jid))
		.map(|(jid, _)| jid.clone())
		.collect();
	for jid in read {
		clear_chat_toasts(&jid);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn xml_escapes_user_text_and_links() {
		let xml = toast_xml("A & <B>", "say \"hi\"", "1@g.us", None, false, true);
		assert!(xml.contains("<text>A &amp; &lt;B&gt;</text>"));
		assert!(xml.contains("<text>say &quot;hi&quot;</text>"));
		assert!(xml.contains("arguments=\"whatsnap://markread?jid=1%40g.us\""));

		let locked_xml = toast_xml("WhatsApp", "1 new message", "1@g.us", None, true, false);
		assert!(!locked_xml.contains("arguments=\"whatsnap://markread"));
		assert!(!locked_xml.contains("Mark as read"));
	}

	#[test]
	fn group_is_within_the_16_character_limit() {
		let group = group_for("919810012345@s.whatsapp.net").to_string();
		assert_eq!(group.len(), 16);
	}
}
