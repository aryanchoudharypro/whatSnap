use std::{collections::HashMap, sync::OnceLock};

const ID_PO: &str = include_str!("../../po/id.po");

static ID_CATALOG: OnceLock<HashMap<String, String>> = OnceLock::new();

/// Looks `text` up in the Indonesian catalog (`po/id.po`), parsed on first use.
pub fn translate_id(text: &str) -> Option<&'static str> {
	ID_CATALOG.get_or_init(|| parse(ID_PO)).get(text).map(String::as_str)
}

fn parse(source: &str) -> HashMap<String, String> {
	#[derive(PartialEq)]
	enum Field {
		None,
		Id,
		Str,
	}
	let mut map = HashMap::new();
	let mut msgid = String::new();
	let mut msgstr = String::new();
	let mut field = Field::None;
	let mut flush = |msgid: &mut String, msgstr: &mut String| {
		if !msgid.is_empty() && !msgstr.is_empty() {
			map.insert(std::mem::take(msgid), std::mem::take(msgstr));
		}
		msgid.clear();
		msgstr.clear();
	};
	for line in source.lines() {
		let line = line.trim();
		if line.is_empty() || line.starts_with('#') {
			if line.is_empty() {
				flush(&mut msgid, &mut msgstr);
				field = Field::None;
			}
			continue;
		}
		let (target, rest) = if let Some(rest) = line.strip_prefix("msgid ") {
			flush(&mut msgid, &mut msgstr);
			field = Field::Id;
			(&mut msgid, rest)
		} else if let Some(rest) = line.strip_prefix("msgstr ") {
			field = Field::Str;
			(&mut msgstr, rest)
		} else if field == Field::Id {
			(&mut msgid, line)
		} else if field == Field::Str {
			(&mut msgstr, line)
		} else {
			continue;
		};
		if let Some(quoted) = rest.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
			unescape_into(quoted, target);
		}
	}
	flush(&mut msgid, &mut msgstr);
	map
}

fn unescape_into(quoted: &str, out: &mut String) {
	let mut chars = quoted.chars();
	while let Some(c) = chars.next() {
		if c != '\\' {
			out.push(c);
			continue;
		}
		match chars.next() {
			Some('n') => out.push('\n'),
			Some('r') => out.push('\r'),
			Some('t') => out.push('\t'),
			Some('"') => out.push('"'),
			Some('\\') => out.push('\\'),
			Some(other) => {
				out.push('\\');
				out.push(other);
			}
			None => out.push('\\'),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_escapes_and_continuations() {
		let map = parse("msgid \"\"\nmsgstr \"\"\n\"Language: id\\n\"\n\nmsgid \"a \\\"b\\\"\\n\"\nmsgstr \"x\"\n\"y\"\n\nmsgid \"untranslated\"\nmsgstr \"\"\n");
		assert_eq!(map.get("a \"b\"\n").map(String::as_str), Some("xy"));
		assert!(!map.contains_key("untranslated"));
		assert!(!map.contains_key(""));
	}

	#[test]
	fn bundled_catalog_translates() {
		assert_eq!(translate_id("Chats"), Some("Chat"));
		assert_eq!(translate_id("{} message selected"), Some("{} pesan dipilih"));
		assert_eq!(translate_id("__nope__"), None);
	}
}
