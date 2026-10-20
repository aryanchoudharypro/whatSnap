//! Phone numbers: telling whether digits make a real number and writing it
//! in international format ("+1 650-253-0000"), from a trimmed-down copy of
//! libphonenumber's metadata (phone_data.rs, see tools/gen_phone_data.py).
//!
//! The metadata's patterns only use digits, `\d`, character classes, `(?:)`
//! groups, `|`, `?` and `{n}`/`{n,m}`, so a small matcher over that subset
//! stands in for a regex engine. Numbers are at most 17 digits, which lets it
//! track every position a pattern can reach as one bit mask.

pub(crate) struct Country {
	pub(crate) code: u16,
	/// The trunk prefix dialled before national numbers, "0" in most places.
	pub(crate) national_prefix: &'static str,
	/// The main region for the code comes first.
	pub(crate) regions: &'static [Region],
	/// The main region's international formats.
	pub(crate) formats: &'static [Format],
}

pub(crate) struct Region {
	/// Numbers starting like this belong to this region, if given.
	pub(crate) leading: Option<&'static str>,
	pub(crate) general: Desc,
	/// Fixed line, mobile, toll free and the other number types.
	pub(crate) types: &'static [Desc],
}

/// A number pattern and the national lengths it allows (any, if empty).
pub(crate) type Desc = (&'static str, &'static [u8]);

pub(crate) struct Format {
	pub(crate) leading: Option<&'static str>,
	/// The digit groups the number splits into, as (min, max) lengths.
	pub(crate) groups: &'static [(u8, u8)],
	/// How the groups are written: "$1-$2-$3".
	pub(crate) format: &'static str,
}

/// Longest national number libphonenumber accepts.
const MAX_NATIONAL: usize = 17;

/// A parsed number: its country and national digits.
struct Number<'a> {
	country: &'static Country,
	national: &'a str,
}

fn parse(digits: &str) -> Option<Number<'_>> {
	let digits = digits.strip_prefix('+').unwrap_or(digits);
	// Calling codes never start with 0.
	if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
		return None;
	}
	let (country, national) = (1..=3.min(digits.len())).find_map(|n| {
		let code: u16 = digits[..n].parse().ok()?;
		let i = crate::phone_data::COUNTRIES.binary_search_by_key(&code, |c| c.code).ok()?;
		Some((&crate::phone_data::COUNTRIES[i], &digits[n..]))
	})?;
	// A national prefix written after the country code ("+44 (0)20...") isn't
	// part of the number, if taking it off is what makes it a valid one.
	let national = match national.strip_prefix(country.national_prefix) {
		Some(stripped)
			if !country.national_prefix.is_empty()
				&& !is_valid_national(country, national)
				&& is_valid_national(country, stripped) =>
		{
			stripped
		}
		_ => national,
	};
	(2..=MAX_NATIONAL).contains(&national.len()).then_some(Number { country, national })
}

/// The number in international format, or None if `digits` (all of them,
/// country code first, with or without a leading +) aren't a phone number.
pub fn format_international(digits: &str) -> Option<String> {
	let n = parse(digits)?;
	let mut out = format!("+{} ", n.country.code);
	let format = n.country.formats.iter().find_map(|f| {
		if let Some(leading) = f.leading
			&& Pattern::compile(leading).match_ends(n.national) & !1 == 0
		{
			return None;
		}
		let groups = split_groups(f.groups, n.national)?;
		Some((f, groups))
	});
	match format {
		Some((f, groups)) => {
			let mut chars = f.format.chars().peekable();
			while let Some(c) = chars.next() {
				match (c, chars.peek().and_then(|d| d.to_digit(10))) {
					('$', Some(g)) => {
						chars.next();
						out.push_str(groups.get(g as usize - 1).copied().unwrap_or(""));
					}
					_ => out.push(c),
				}
			}
		}
		None => out.push_str(n.national),
	}
	Some(out)
}

/// Splits `national` into consecutive groups of the given lengths, if they
/// cover it exactly.
fn split_groups<'a>(groups: &[(u8, u8)], national: &'a str) -> Option<Vec<&'a str>> {
	fn go<'a>(groups: &[(u8, u8)], rest: &'a str, out: &mut Vec<&'a str>) -> bool {
		let Some(&(min, max)) = groups.first() else { return rest.is_empty() };
		// Groups are greedy, as a regex's \d{n,m} would be.
		for len in (min as usize..=(max as usize).min(rest.len())).rev() {
			out.push(&rest[..len]);
			if go(&groups[1..], &rest[len..], out) {
				return true;
			}
			out.pop();
		}
		false
	}
	let mut out = Vec::with_capacity(groups.len());
	go(groups, national, &mut out).then_some(out)
}

/// Whether the digits (country code first, with or without a leading +)
/// are a valid number of one of the country's number types.
pub fn is_valid(digits: &str) -> bool {
	parse(digits).is_some_and(|n| is_valid_national(n.country, n.national))
}

/// The number's digits as a JID holds them (country code and national
/// number, without a trunk prefix written in between), if it's valid.
pub fn valid_digits(digits: &str) -> Option<String> {
	let n = parse(digits)?;
	is_valid_national(n.country, n.national).then(|| format!("{}{}", n.country.code, n.national))
}

fn is_valid_national(country: &Country, national: &str) -> bool {
	let regions = country.regions;
	// Which region the number is from: the first whose leading digits it
	// starts with, or else the first it's a valid number of.
	let region = if regions.len() == 1 {
		Some(&regions[0])
	} else {
		regions.iter().find(|r| match r.leading {
			Some(leading) => Pattern::compile(leading).match_ends(national) & !1 != 0,
			None => is_valid_in(r, national),
		})
	};
	region.is_some_and(|r| is_valid_in(r, national))
}

fn is_valid_in(region: &Region, national: &str) -> bool {
	matches_desc(region.general, national) && region.types.iter().any(|&d| matches_desc(d, national))
}

/// Like libphonenumber's descriptor check: a length the type allows, and
/// the pattern matching from the start of the number.
fn matches_desc((pattern, lengths): Desc, national: &str) -> bool {
	(lengths.is_empty() || lengths.contains(&(national.len() as u8)))
		&& Pattern::compile(pattern).match_ends(national) != 0
}

/// A parsed pattern: alternatives of sequences.
struct Pattern(Vec<Vec<Node>>);

enum Node {
	/// One digit out of a set, as a bit per digit.
	Digit(u16),
	Group(Pattern),
	Repeat(Box<Node>, u8, u8),
}

impl Pattern {
	/// Parses the pattern; a malformed one matches nothing.
	fn compile(src: &str) -> Pattern {
		let mut bytes = src.as_bytes();
		let p = Pattern::parse(&mut bytes);
		if bytes.is_empty() { p } else { Pattern(Vec::new()) }
	}

	fn parse(s: &mut &[u8]) -> Pattern {
		let mut alts = vec![Vec::new()];
		while let Some(&c) = s.first() {
			let atom = match c {
				b')' => break,
				b'|' => {
					*s = &s[1..];
					alts.push(Vec::new());
					continue;
				}
				b'0'..=b'9' => {
					*s = &s[1..];
					Node::Digit(1 << (c - b'0'))
				}
				b'\\' if s.get(1) == Some(&b'd') => {
					*s = &s[2..];
					Node::Digit(0x3ff)
				}
				b'[' => {
					let end = s.iter().position(|&b| b == b']').unwrap_or(s.len());
					let class = &s[1..end];
					*s = &s[(end + 1).min(s.len())..];
					let mut mask = 0u16;
					let mut i = 0;
					while i < class.len() {
						let lo = class[i].wrapping_sub(b'0');
						let hi = if class.get(i + 1) == Some(&b'-') && i + 2 < class.len() {
							i += 2;
							class[i].wrapping_sub(b'0')
						} else {
							lo
						};
						for d in lo..=hi.min(9) {
							mask |= 1 << d;
						}
						i += 1;
					}
					Node::Digit(mask)
				}
				b'(' => {
					*s = s.strip_prefix(b"(?:").unwrap_or(&s[1..]);
					let inner = Pattern::parse(s);
					*s = s.strip_prefix(b")").unwrap_or(&[]);
					Node::Group(inner)
				}
				// Anything else isn't part of the subset; stop so `compile`
				// sees the leftover.
				_ => break,
			};
			let node = match s.first() {
				Some(b'?') => {
					*s = &s[1..];
					Node::Repeat(Box::new(atom), 0, 1)
				}
				Some(b'{') => {
					let end = s.iter().position(|&b| b == b'}').unwrap_or(s.len());
					let body = std::str::from_utf8(&s[1..end]).unwrap_or("");
					*s = &s[(end + 1).min(s.len())..];
					let (min, max) = body.split_once(',').unwrap_or((body, body));
					let min = min.parse().unwrap_or(0);
					Node::Repeat(Box::new(atom), min, max.parse().unwrap_or(min))
				}
				_ => atom,
			};
			alts.last_mut().unwrap().push(node);
		}
		Pattern(alts)
	}

	/// Every position in `digits` a match starting at 0 can end at, as bits.
	fn match_ends(&self, digits: &str) -> u32 {
		self.step(digits.as_bytes(), 1)
	}

	fn step(&self, input: &[u8], from: u32) -> u32 {
		self.0.iter().fold(0, |ends, seq| ends | seq.iter().fold(from, |at, node| node.step(input, at)))
	}
}

impl Node {
	fn step(&self, input: &[u8], from: u32) -> u32 {
		match self {
			Node::Digit(mask) => {
				let mut out = 0;
				for (i, &b) in input.iter().enumerate() {
					if from & (1 << i) != 0 && mask & (1 << (b - b'0')) != 0 {
						out |= 1 << (i + 1);
					}
				}
				out
			}
			Node::Group(p) => p.step(input, from),
			Node::Repeat(node, min, max) => {
				let mut at = from;
				for _ in 0..*min {
					at = node.step(input, at);
				}
				let mut ends = at;
				for _ in *min..*max {
					at = node.step(input, at);
					if at == 0 {
						break;
					}
					ends |= at;
				}
				ends
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn formats_like_libphonenumber() {
		assert_eq!(format_international("16502530000").as_deref(), Some("+1 650-253-0000"));
		assert_eq!(format_international("+447911123456").as_deref(), Some("+44 7911 123456"));
		assert_eq!(format_international("919876543210").as_deref(), Some("+91 98765 43210"));
		assert_eq!(format_international("33612345678").as_deref(), Some("+33 6 12 34 56 78"));
		assert_eq!(format_international("4407911123456").as_deref(), Some("+44 7911 123456"));
		assert_eq!(format_international("999123"), None);
		assert_eq!(format_international(""), None);
	}

	#[test]
	fn validates_like_libphonenumber() {
		assert!(is_valid("+16502530000"));
		assert!(is_valid("447911123456"));
		assert!(!is_valid("10000000000"));
		assert!(!is_valid("2024"));
		assert!(!is_valid("12345678"));
		assert_eq!(valid_digits("+4407911123456").as_deref(), Some("447911123456"));
		assert_eq!(valid_digits("447911123456").as_deref(), Some("447911123456"));
		assert_eq!(valid_digits("4400"), None);
	}

	#[test]
	fn matcher_handles_the_pattern_subset() {
		let ends = |p: &str, s: &str| Pattern::compile(p).match_ends(s);
		assert_eq!(ends(r"\d{2}", "123"), 1 << 2);
		assert_eq!(ends(r"1[2-4]?5", "135"), 1 << 3);
		assert_eq!(ends(r"1(?:2|34)", "1345"), 1 << 3);
		assert_eq!(ends(r"\d{2,3}", "1234"), 1 << 2 | 1 << 3);
		assert_eq!(ends(r"[02-9]", "1"), 0);
		assert_eq!(ends(r"5+", "5"), 0, "unsupported syntax matches nothing");
	}
}
