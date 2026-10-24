//! Windows toasts for chat messages. Each chat's toasts share a group, so
//! opening that chat (or pressing "Mark as read" on one) removes them from
//! Action Center, not just the popup. wx's notification API has no tags or
//! groups, so the toasts are built here with WinRT.

use std::{
	collections::hash_map::DefaultHasher,
	hash::{Hash, Hasher},
	sync::atomic::{AtomicU64, Ordering},
};

use windows::{
	Data::Xml::Dom::XmlDocument,
	UI::Notifications::{ToastNotification, ToastNotificationManager},
	core::HSTRING,
};

use crate::app_link;

/// The app ID toasts are sent under. It has to match the app ID that
/// `msw_use_toasts` writes onto the Start menu shortcut.
pub const APP_ID: &str = "whatSnap";

/// Where the Start menu shortcut that carries [`APP_ID`] lives. The installer
/// creates the same shortcut here; the app repairs it on startup if needed.
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

/// Tags and groups are limited to 16 characters, so the group is a short hash
/// of the chat's JID.
fn group_for(jid: &str) -> HSTRING {
	let mut hasher = DefaultHasher::new();
	jid.hash(&mut hasher);
	HSTRING::from(format!("c{:015x}", hasher.finish() & 0x0fff_ffff_ffff_ffff))
}

/// A tag unique to one toast.
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

/// The toast's XML. Clicking the toast opens the chat, and its "Mark as read"
/// button marks the chat read; both are `whatsnap://` links the app handles.
fn toast_xml(title: &str, body: &str, jid: &str) -> String {
	let open = escape_xml(&app_link::open_link(jid));
	let mark_read = escape_xml(&app_link::mark_read_link(jid));
	format!(
		concat!(
			r#"<toast launch="{open}" activationType="protocol">"#,
			r#"<visual><binding template="ToastGeneric">"#,
			r#"<text>{title}</text><text>{body}</text>"#,
			r#"</binding></visual>"#,
			r#"<actions><action content="Mark as read" arguments="{mark_read}" activationType="protocol"/></actions>"#,
			r#"</toast>"#,
		),
		open = open,
		mark_read = mark_read,
		title = escape_xml(title),
		body = escape_xml(body),
	)
}

/// Shows a message toast for `jid`, filed in that chat's group. Must run on the
/// UI thread, like the rest of the WinRT toast code in this app.
pub fn show_chat_toast(jid: &str, title: &str, body: &str) -> windows::core::Result<()> {
	let document = XmlDocument::new()?;
	document.LoadXml(&HSTRING::from(toast_xml(title, body, jid)))?;
	let toast = ToastNotification::CreateToastNotification(&document)?;
	toast.SetTag(&next_tag())?;
	toast.SetGroup(&group_for(jid))?;
	let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))?;
	notifier.Show(&toast)
}

/// Removes every toast for `jid` from Action Center. Quietly does nothing if
/// Windows keeps no history.
pub fn clear_chat_toasts(jid: &str) {
	if let Ok(history) = ToastNotificationManager::History() {
		let _ = history.RemoveGroupWithId(&group_for(jid), &HSTRING::from(APP_ID));
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn xml_escapes_user_text_and_links() {
		let xml = toast_xml("A & <B>", "say \"hi\"", "1@g.us");
		assert!(xml.contains("<text>A &amp; &lt;B&gt;</text>"));
		assert!(xml.contains("<text>say &quot;hi&quot;</text>"));
		assert!(xml.contains("arguments=\"whatsnap://markread?jid=1%40g.us\""));
	}

	#[test]
	fn group_is_within_the_16_character_limit() {
		let group = group_for("919810012345@s.whatsapp.net").to_string();
		assert_eq!(group.len(), 16);
	}
}
