#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppLink {
	/// `account` names the linked account the chat belongs to, for a
	/// notification from an account that isn't the one on screen.
	Open {
		jid: String,
		account: Option<String>,
	},
	MarkRead {
		jid: String,
		account: Option<String>,
	},
	/// The Send button of a notification's reply box. The typed text never
	/// travels in the link: only the toast activator (see
	/// `platform::toast_activator`) can pair it with what was typed, so a link
	/// from a web page can't make whatSnap send anything. Opened on its own,
	/// it just opens the chat.
	Reply {
		jid: String,
		account: Option<String>,
	},
}

const SCHEME: &str = "whatsnap://";

pub fn open_link(jid: &str, account: Option<&str>) -> String {
	format!("{SCHEME}open?jid={}{}", encode(jid), account_param(account))
}

pub fn mark_read_link(jid: &str, account: Option<&str>) -> String {
	format!("{SCHEME}markread?jid={}{}", encode(jid), account_param(account))
}

/// Brings whatSnap to the front without opening anything.
pub fn activate_link() -> String {
	format!("{SCHEME}activate")
}

pub fn reply_link(jid: &str, account: Option<&str>) -> String {
	format!("{SCHEME}reply?jid={}{}", encode(jid), account_param(account))
}

fn account_param(account: Option<&str>) -> String {
	account.map(|a| format!("&account={}", encode(a))).unwrap_or_default()
}

pub fn parse(link: &str) -> Option<AppLink> {
	let link = link.trim().trim_matches('"');
	let rest = link.get(..SCHEME.len()).filter(|s| s.eq_ignore_ascii_case(SCHEME)).map(|_| &link[SCHEME.len()..])?;
	let (action, query) = rest.split_once('?').unwrap_or((rest, ""));
	let jid = query.split('&').find_map(|pair| pair.strip_prefix("jid=")).map(decode).filter(|jid| !jid.is_empty())?;
	let account = query
		.split('&')
		.find_map(|pair| pair.strip_prefix("account="))
		.map(decode)
		.filter(|a| crate::platform::accounts::is_valid_id(a));
	match action.trim_end_matches('/').to_ascii_lowercase().as_str() {
		"open" => Some(AppLink::Open { jid, account }),
		"markread" => Some(AppLink::MarkRead { jid, account }),
		"reply" => Some(AppLink::Reply { jid, account }),
		_ => None,
	}
}

fn encode(value: &str) -> String {
	let mut out = String::with_capacity(value.len());
	for byte in value.bytes() {
		if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
			out.push(byte as char);
		} else {
			out.push_str(&format!("%{byte:02X}"));
		}
	}
	out
}

fn decode(value: &str) -> String {
	let bytes = value.as_bytes();
	let mut out = Vec::with_capacity(bytes.len());
	let mut i = 0;
	while i < bytes.len() {
		if bytes[i] == b'%'
			&& i + 2 < bytes.len()
			&& let Some(byte) =
				std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|hex| u8::from_str_radix(hex, 16).ok())
		{
			out.push(byte);
			i += 3;
		} else {
			out.push(bytes[i]);
			i += 1;
		}
	}
	String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn round_trips_a_jid_through_both_links() {
		let jid = "919810012345@s.whatsapp.net";
		assert_eq!(parse(&mark_read_link(jid, None)), Some(AppLink::MarkRead { jid: jid.to_string(), account: None }));
		assert_eq!(parse(&open_link(jid, None)), Some(AppLink::Open { jid: jid.to_string(), account: None }));
		assert_eq!(parse(&reply_link(jid, None)), Some(AppLink::Reply { jid: jid.to_string(), account: None }));
	}

	#[test]
	fn carries_the_account_and_drops_a_bad_one() {
		let jid = "1@g.us";
		let id = "0123456789abcdef0123456789abcdef";
		assert_eq!(parse(&open_link(jid, Some(id))), Some(AppLink::Open { jid: jid.into(), account: Some(id.into()) }));
		assert_eq!(
			parse(&mark_read_link(jid, Some("default"))),
			Some(AppLink::MarkRead { jid: jid.into(), account: Some("default".into()) })
		);
		assert_eq!(
			parse("whatsnap://open?jid=1%40g.us&account=..%2F..%2Fx"),
			Some(AppLink::Open { jid: jid.into(), account: None })
		);
	}

	#[test]
	fn accepts_quoted_and_case_varied_links() {
		assert_eq!(
			parse("\"whatsnap://markread?jid=1%40g.us\""),
			Some(AppLink::MarkRead { jid: "1@g.us".into(), account: None })
		);
		assert_eq!(parse("WHATSNAP://OPEN/?jid=1%40g.us"), Some(AppLink::Open { jid: "1@g.us".into(), account: None }));
	}

	#[test]
	fn rejects_other_links_and_missing_jids() {
		assert_eq!(parse("whatsapp://send?text=hi"), None);
		assert_eq!(parse("whatsnap://markread"), None);
		assert_eq!(parse("whatsnap://markread?jid="), None);
		assert_eq!(parse("whatsnap://delete?jid=1%40g.us"), None);
	}
}
