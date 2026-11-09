//! Chat list search, as WA Web matches chats and contacts against what's
//! typed in the search box (WAWebSearchUtils, WAWebPhoneNumberSearch,
//! WAWebSearchMatchStrategies and WAWebContactSearchMatcher).

/// The query as WA Web compares it: trimmed, lower case, accents folded
/// (WAWebSearchUtils.normalizeString).
pub fn normalize(query: &str) -> String {
	fold(query.trim())
}

/// Lower case with accents folded, so "José" is found by "jose" (WA Web's
/// accentFold). Covers the Latin letters with diacritics and drops combining
/// marks; other scripts pass through lower-cased.
pub fn fold(text: &str) -> String {
	let mut out = String::with_capacity(text.len());
	for c in text.chars() {
		if ('\u{300}'..='\u{36f}').contains(&c) {
			continue;
		}
		for lower in c.to_lowercase() {
			match base_letter(lower) {
				Some(base) => out.push_str(base),
				None => out.push(lower),
			}
		}
	}
	out
}

fn base_letter(c: char) -> Option<&'static str> {
	Some(match c {
		'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' | 'ǎ' | 'ạ' | 'ả' | 'ấ' | 'ầ' | 'ẩ' | 'ẫ' | 'ậ' | 'ắ'
		| 'ằ' | 'ẳ' | 'ẵ' | 'ặ' => "a",
		'æ' => "ae",
		'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => "c",
		'ď' | 'đ' | 'ð' => "d",
		'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' | 'ẹ' | 'ẻ' | 'ẽ' | 'ế' | 'ề' | 'ể' | 'ễ' | 'ệ' => {
			"e"
		}
		'ĝ' | 'ğ' | 'ġ' | 'ģ' => "g",
		'ĥ' | 'ħ' => "h",
		'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' | 'ǐ' | 'ỉ' | 'ị' => "i",
		'ĵ' => "j",
		'ķ' => "k",
		'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => "l",
		'ñ' | 'ń' | 'ņ' | 'ň' => "n",
		'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' | 'ơ' | 'ǒ' | 'ọ' | 'ỏ' | 'ố' | 'ồ' | 'ổ' | 'ỗ' | 'ộ'
		| 'ớ' | 'ờ' | 'ở' | 'ỡ' | 'ợ' => "o",
		'œ' => "oe",
		'ŕ' | 'ŗ' | 'ř' => "r",
		'ś' | 'ŝ' | 'ş' | 'š' | 'ș' => "s",
		'ß' => "ss",
		'ţ' | 'ť' | 'ŧ' | 'ț' => "t",
		'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' | 'ư' | 'ǔ' | 'ụ' | 'ủ' | 'ứ' | 'ừ' | 'ử' | 'ữ'
		| 'ự' => "u",
		'ŵ' => "w",
		'ý' | 'ÿ' | 'ŷ' | 'ỳ' | 'ỵ' | 'ỷ' | 'ỹ' => "y",
		'ź' | 'ż' | 'ž' => "z",
		'þ' => "th",
		_ => return None,
	})
}

/// The digits of a query that looks like a phone number ("+44 (0)20-7946"),
/// or None when it has anything else in it (WAWebPhoneNumberSearch.numberSearch).
pub fn number_search(query: &str) -> Option<String> {
	let cleaned: String = query
		.chars()
		.filter(|&c| !is_invisible(c))
		.map(|c| if matches!(c, '\u{a0}' | '\u{202f}') { ' ' } else { c })
		.collect();
	if cleaned.is_empty() || !cleaned.chars().all(|c| c.is_ascii_digit() || matches!(c, '+' | '(' | ')' | '-' | ' ')) {
		return None;
	}
	Some(cleaned.chars().filter(char::is_ascii_digit).collect())
}

/// Direction marks, zero-width spaces and soft hyphens, which a pasted phone
/// number often carries.
fn is_invisible(c: char) -> bool {
	matches!(
		c,
		'\u{200b}'..='\u{200f}'
			| '\u{61c}'
			| '\u{202a}'..='\u{202e}'
			| '\u{2060}'
			| '\u{2066}'..='\u{2069}'
			| '\u{feff}'
			| '\u{ad}'
	)
}

/// Every word of the query is somewhere in the field, in any order
/// (WAWebSearchMatchStrategies.substringMatch). Both are already folded.
pub fn words_match(field: &str, query: &str) -> bool {
	let mut words = query.split_whitespace().peekable();
	words.peek().is_some() && words.all(|word| field.contains(word))
}

/// What a chat or contact can be found by. `names` are the saved name or
/// group subject, verified business name, push name and the like, as shown;
/// `phone` is the digits of its phone number, where known.
pub struct Searchable<'a> {
	pub names: &'a [&'a str],
	pub phone: Option<&'a str>,
}

/// A normalized query, ready to match many chats against.
pub struct Query {
	pub text: String,
	pub numeric: Option<String>,
	/// The number with the user's own country code in front and the trunk
	/// prefix taken off, so a local "07700 900123" finds "447700900123".
	pub international: Option<String>,
}

impl Query {
	pub fn new(raw: &str, own_phone: Option<&str>) -> Self {
		let text = normalize(raw);
		let numeric = number_search(&text).filter(|n| !n.is_empty());
		let international =
			numeric.as_deref().and_then(|n| own_phone.and_then(|own| crate::phone::with_own_country(own, n)));
		Self { text, numeric, international }
	}

	pub fn is_empty(&self) -> bool {
		self.text.is_empty()
	}

	/// WAWebContactSearchMatcher's exact match: the name fields word by word,
	/// then the phone number by the typed text or its digits.
	pub fn matches(&self, item: &Searchable) -> bool {
		if self.text.is_empty() {
			return true;
		}
		if item.names.iter().any(|name| !name.is_empty() && words_match(&fold(name), &self.text)) {
			return true;
		}
		let Some(phone) = item.phone.filter(|p| !p.is_empty()) else { return false };
		phone.contains(self.text.as_str())
			|| self.numeric.as_deref().is_some_and(|n| phone.contains(n))
			|| self.international.as_deref().is_some_and(|n| phone.contains(n))
	}

	/// Whether WA Web would also search message text for this query: two
	/// characters or more, or a single one that is a word on its own (an
	/// emoji or a CJK character).
	pub fn searches_messages(&self) -> bool {
		let mut chars = self.text.chars();
		match (chars.next(), chars.next()) {
			(Some(_), Some(_)) => true,
			(Some(c), None) => (!c.is_ascii() && !c.is_alphabetic()) || is_cjk(c),
			_ => false,
		}
	}
}

fn is_cjk(c: char) -> bool {
	matches!(
		c,
		'\u{3040}'..='\u{30ff}'
			| '\u{3400}'..='\u{4dbf}'
			| '\u{4e00}'..='\u{9fff}'
			| '\u{ac00}'..='\u{d7af}'
			| '\u{f900}'..='\u{faff}'
	)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn chat<'a>(names: &'a [&'a str], phone: Option<&'a str>) -> Searchable<'a> {
		Searchable { names, phone }
	}

	#[test]
	fn the_query_is_trimmed_lower_cased_and_accent_folded() {
		assert_eq!(normalize("  José MÜLLER "), "jose muller");
		assert_eq!(fold("Ça va"), "ca va");
		assert_eq!(fold("e\u{301}"), "e");
	}

	#[test]
	fn every_word_must_be_in_the_same_field_in_any_order() {
		let names = ["Jane Doe", "JD Plumbing"];
		let q = Query::new("doe jane", None);
		assert!(q.matches(&chat(&names, None)));
		let q = Query::new("jane plumbing", None);
		assert!(!q.matches(&chat(&names, None)));
		let q = Query::new("plumb", None);
		assert!(q.matches(&chat(&names, None)));
	}

	#[test]
	fn a_phone_number_matches_by_its_digits() {
		let q = Query::new("+44 7700-900", None);
		assert_eq!(q.numeric.as_deref(), Some("447700900"));
		assert!(q.matches(&chat(&["Jane"], Some("447700900123"))));
		assert!(Query::new("900123", None).matches(&chat(&[], Some("447700900123"))));
		assert!(!Query::new("jane 44", None).matches(&chat(&["Bob"], Some("447700900123"))));
	}

	#[test]
	fn an_empty_query_matches_everything_and_never_searches_messages() {
		let q = Query::new("   ", None);
		assert!(q.is_empty());
		assert!(q.matches(&chat(&[], None)));
		assert!(!q.searches_messages());
	}

	#[test]
	fn one_letter_searches_chats_only_but_an_emoji_searches_messages() {
		assert!(!Query::new("a", None).searches_messages());
		assert!(Query::new("ab", None).searches_messages());
		assert!(Query::new("😀", None).searches_messages());
		assert!(Query::new("猫", None).searches_messages());
	}

	#[test]
	fn non_phone_text_is_not_a_number_search() {
		assert_eq!(number_search("12a"), None);
		assert_eq!(number_search("(020) 7946-0018"), Some("02079460018".to_string()));
	}
}
