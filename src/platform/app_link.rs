#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppLink {
	Open { jid: String },
	MarkRead { jid: String },
}

const SCHEME: &str = "whatsnap://";

pub fn open_link(jid: &str) -> String {
	format!("{SCHEME}open?jid={}", encode(jid))
}

pub fn mark_read_link(jid: &str) -> String {
	format!("{SCHEME}markread?jid={}", encode(jid))
}

pub fn parse(link: &str) -> Option<AppLink> {
	let link = link.trim().trim_matches('"');
	let rest = link.get(..SCHEME.len()).filter(|s| s.eq_ignore_ascii_case(SCHEME)).map(|_| &link[SCHEME.len()..])?;
	let (action, query) = rest.split_once('?').unwrap_or((rest, ""));
	let jid = query.split('&').find_map(|pair| pair.strip_prefix("jid=")).map(decode).filter(|jid| !jid.is_empty())?;
	match action.trim_end_matches('/').to_ascii_lowercase().as_str() {
		"open" => Some(AppLink::Open { jid }),
		"markread" => Some(AppLink::MarkRead { jid }),
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
		assert_eq!(parse(&mark_read_link(jid)), Some(AppLink::MarkRead { jid: jid.to_string() }));
		assert_eq!(parse(&open_link(jid)), Some(AppLink::Open { jid: jid.to_string() }));
	}

	#[test]
	fn accepts_quoted_and_case_varied_links() {
		assert_eq!(parse("\"whatsnap://markread?jid=1%40g.us\""), Some(AppLink::MarkRead { jid: "1@g.us".into() }));
		assert_eq!(parse("WHATSNAP://OPEN/?jid=1%40g.us"), Some(AppLink::Open { jid: "1@g.us".into() }));
	}

	#[test]
	fn rejects_other_links_and_missing_jids() {
		assert_eq!(parse("whatsapp://send?text=hi"), None);
		assert_eq!(parse("whatsnap://markread"), None);
		assert_eq!(parse("whatsnap://markread?jid="), None);
		assert_eq!(parse("whatsnap://delete?jid=1%40g.us"), None);
	}
}
