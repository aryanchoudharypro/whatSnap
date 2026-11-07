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

fn toast_xml(title: &str, body: &str, jid: &str, account: Option<&str>) -> String {
	let open = escape_xml(&app_link::open_link(jid, account));
	let mark_read = escape_xml(&app_link::mark_read_link(jid, account));
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

/// Shows a message toast for `jid`, filed in that chat's group. `account` is
/// set for a chat of a linked account that isn't the one on screen, so a click
/// switches to it. Must run on the UI thread, like the rest of the WinRT toast
/// code in this app.
pub fn show_chat_toast(jid: &str, title: &str, body: &str, account: Option<&str>) -> windows::core::Result<()> {
	let document = XmlDocument::new()?;
	document.LoadXml(&HSTRING::from(toast_xml(title, body, jid, account)))?;
	let toast = ToastNotification::CreateToastNotification(&document)?;
	toast.SetTag(&next_tag())?;
	toast.SetGroup(&group_for(jid))?;
	let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))?;
	notifier.Show(&toast)
}

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
		let xml = toast_xml("A & <B>", "say \"hi\"", "1@g.us", None);
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
