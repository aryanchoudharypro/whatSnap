#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct TextStyle {
	pub bold: bool,
	pub italic: bool,
	pub strike: bool,
	pub mono: bool,
}

impl TextStyle {
	pub const fn bold() -> Self {
		Self { bold: true, italic: false, strike: false, mono: false }
	}

	pub const fn italic() -> Self {
		Self { bold: false, italic: true, strike: false, mono: false }
	}

	pub const fn strike() -> Self {
		Self { bold: false, italic: false, strike: true, mono: false }
	}

	pub const fn mono() -> Self {
		Self { bold: false, italic: false, strike: false, mono: true }
	}

	pub const fn with_bold(self) -> Self {
		Self { bold: true, ..self }
	}

	pub const fn with_italic(self) -> Self {
		Self { italic: true, ..self }
	}

	pub const fn with_strike(self) -> Self {
		Self { strike: true, ..self }
	}

	pub const fn with_mono(self) -> Self {
		Self { mono: true, ..self }
	}
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StyledSpan {
	pub text: String,
	pub style: TextStyle,
}

pub fn parse_markdown_spans(text: &str) -> Vec<StyledSpan> {
	let chars: Vec<char> = text.chars().collect();
	coalesce_spans(parse_chars(&chars, TextStyle::default()))
}

fn parse_chars(chars: &[char], current_style: TextStyle) -> Vec<StyledSpan> {
	let mut spans: Vec<StyledSpan> = Vec::new();
	let mut scan_start = 0;
	let mut i = 0;

	while i < chars.len() {
		let c = chars[i];

		// A link or address is never formatted: underscores and stars in it
		// are part of it, and a screen reader must read it as it is.
		if (i == 0 || chars[i - 1].is_whitespace()) && starts_link(&chars[i..]) {
			while i < chars.len() && !chars[i].is_whitespace() {
				i += 1;
			}
			continue;
		}

		// 1. Triple backtick code block or inline ```...```
		if c == '`' && i + 2 < chars.len() && chars[i + 1] == '`' && chars[i + 2] == '`' {
			let mut close_idx = None;
			let mut j = i + 3;
			while j + 2 < chars.len() {
				if chars[j] == '`' && chars[j + 1] == '`' && chars[j + 2] == '`' {
					close_idx = Some(j);
					break;
				}
				j += 1;
			}
			if let Some(j) = close_idx {
				if i > scan_start {
					spans.push(StyledSpan { text: chars[scan_start..i].iter().collect(), style: current_style });
				}
				spans.push(StyledSpan { text: chars[i + 3..j].iter().collect(), style: current_style.with_mono() });
				i = j + 3;
				scan_start = i;
				continue;
			}
		}

		// 2. Single backtick inline code `...`
		if c == '`' {
			let mut close_idx = None;
			let mut j = i + 1;
			while j < chars.len() {
				if chars[j] == '`' {
					if j > i + 1 {
						close_idx = Some(j);
					}
					break;
				}
				if chars[j] == '\n' {
					break;
				}
				j += 1;
			}
			if let Some(j) = close_idx {
				if i > scan_start {
					spans.push(StyledSpan { text: chars[scan_start..i].iter().collect(), style: current_style });
				}
				spans.push(StyledSpan { text: chars[i + 1..j].iter().collect(), style: current_style.with_mono() });
				i = j + 1;
				scan_start = i;
				continue;
			}
		}

		// 3. Formatting markers: '*' (bold), '_' (italic), '~' (strikethrough)
		if c == '*' || c == '_' || c == '~' {
			let delim = c;
			let next_ok = i + 1 < chars.len() && !chars[i + 1].is_whitespace() && chars[i + 1] != delim;
			let prev_ok = i == 0 || opens_after(chars[i - 1]);

			if next_ok && prev_ok {
				let mut close_idx = None;
				let mut j = i + 1;
				while j < chars.len() {
					if chars[j] == delim {
						let prev_j_ok = !chars[j - 1].is_whitespace();
						let next_j_ok = j + 1 >= chars.len() || closes_before(chars[j + 1]);
						if prev_j_ok && next_j_ok && j > i + 1 {
							close_idx = Some(j);
							break;
						}
					}
					if chars[j] == '\n' {
						break;
					}
					j += 1;
				}

				if let Some(j) = close_idx {
					if i > scan_start {
						spans.push(StyledSpan { text: chars[scan_start..i].iter().collect(), style: current_style });
					}
					let inner = &chars[i + 1..j];
					let inner_style = match delim {
						'*' => current_style.with_bold(),
						'_' => current_style.with_italic(),
						'~' => current_style.with_strike(),
						_ => current_style,
					};
					let mut inner_spans = parse_chars(inner, inner_style);
					spans.append(&mut inner_spans);
					i = j + 1;
					scan_start = i;
					continue;
				}
			}
		}

		i += 1;
	}

	if scan_start < chars.len() {
		spans.push(StyledSpan { text: chars[scan_start..].iter().collect(), style: current_style });
	}

	spans
}

/// A marker opens only at the start of a word, like WhatsApp: after a space
/// or opening punctuation, never inside a path like `a/_b_/c`.
fn opens_after(p: char) -> bool {
	p.is_whitespace() || matches!(p, '(' | '[' | '{' | '"' | '\'' | '*' | '_' | '~' | '\u{201C}' | '\u{2018}')
}

/// A marker closes only at the end of a word.
fn closes_before(n: char) -> bool {
	n.is_whitespace()
		|| matches!(
			n,
			'.' | ','
				| '!'
				| '?'
				| ';'
				| ':'
				| ')'
				| ']'
				| '}'
				| '"'
				| '\''
				| '*'
				| '_'
				| '~'
				| '\u{201D}'
				| '\u{2019}'
		)
}

fn starts_link(rest: &[char]) -> bool {
	let lower: String = rest.iter().take(8).flat_map(|c| c.to_lowercase()).collect();
	lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("www.")
}

pub fn coalesce_spans(spans: Vec<StyledSpan>) -> Vec<StyledSpan> {
	let mut out: Vec<StyledSpan> = Vec::with_capacity(spans.len());
	for span in spans {
		if span.text.is_empty() {
			continue;
		}
		if let Some(last) = out.last_mut()
			&& last.style == span.style
		{
			last.text.push_str(&span.text);
		} else {
			out.push(span);
		}
	}
	out
}

/// Strips WhatsApp/Markdown formatting syntax characters so screen readers
/// speak natural words instead of punctuation symbols.
pub fn strip_markdown_for_speech(text: &str) -> String {
	if text.is_empty() {
		return String::new();
	}
	let mut lines_out: Vec<String> = Vec::new();
	let mut in_code_block = false;

	for raw_line in text.split('\n') {
		let line = raw_line.trim_end_matches('\r');
		let trimmed = line.trim();
		if trimmed.starts_with("```") {
			in_code_block = !in_code_block;
			continue;
		}
		if in_code_block {
			lines_out.push(line.to_string());
			continue;
		}
		let spans = parse_markdown_spans(line);
		let line_text: String = spans.into_iter().map(|s| s.text).collect();
		lines_out.push(line_text);
	}

	lines_out.join("\n")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_plain_text() {
		let spans = parse_markdown_spans("hello world");
		assert_eq!(spans.len(), 1);
		assert_eq!(spans[0].text, "hello world");
		assert_eq!(spans[0].style, TextStyle::default());
	}

	#[test]
	fn test_bold_text() {
		let spans = parse_markdown_spans("*bold message*");
		assert_eq!(spans.len(), 1);
		assert_eq!(spans[0].text, "bold message");
		assert_eq!(spans[0].style, TextStyle::bold());
	}

	#[test]
	fn test_italic_text() {
		let spans = parse_markdown_spans("_italic message_");
		assert_eq!(spans.len(), 1);
		assert_eq!(spans[0].text, "italic message");
		assert_eq!(spans[0].style, TextStyle::italic());
	}

	#[test]
	fn test_strikethrough_text() {
		let spans = parse_markdown_spans("~strike message~");
		assert_eq!(spans.len(), 1);
		assert_eq!(spans[0].text, "strike message");
		assert_eq!(spans[0].style, TextStyle::strike());
	}

	#[test]
	fn test_inline_code() {
		let spans = parse_markdown_spans("`inline code`");
		assert_eq!(spans.len(), 1);
		assert_eq!(spans[0].text, "inline code");
		assert_eq!(spans[0].style, TextStyle::mono());
	}

	#[test]
	fn test_nested_bold_italic() {
		let spans = parse_markdown_spans("*_bold italic_*");
		assert_eq!(spans.len(), 1);
		assert_eq!(spans[0].text, "bold italic");
		assert_eq!(spans[0].style, TextStyle::bold().with_italic());
	}

	#[test]
	fn test_mixed_spans() {
		let spans = parse_markdown_spans("normal *bold* and _italic_");
		assert_eq!(spans.len(), 4);
		assert_eq!(spans[0].text, "normal ");
		assert_eq!(spans[0].style, TextStyle::default());
		assert_eq!(spans[1].text, "bold");
		assert_eq!(spans[1].style, TextStyle::bold());
		assert_eq!(spans[2].text, " and ");
		assert_eq!(spans[2].style, TextStyle::default());
		assert_eq!(spans[3].text, "italic");
		assert_eq!(spans[3].style, TextStyle::italic());
	}

	#[test]
	fn test_identifiers_and_math_preserved() {
		let spans = parse_markdown_spans("var_name and 2 * 3 = 6");
		assert_eq!(spans.len(), 1);
		assert_eq!(spans[0].text, "var_name and 2 * 3 = 6");
		assert_eq!(spans[0].style, TextStyle::default());
	}

	#[test]
	fn test_unclosed_markers_preserved() {
		let spans = parse_markdown_spans("*unclosed bold");
		assert_eq!(spans.len(), 1);
		assert_eq!(spans[0].text, "*unclosed bold");
		assert_eq!(spans[0].style, TextStyle::default());
	}

	#[test]
	fn test_links_and_paths_preserved() {
		for text in [
			"see https://example.com/_secret_/page",
			"www.x.com/*a*",
			"C:\\Users\\_x_\\a",
			"a/_b_/c",
			"x_y_z and 2*3*4",
		] {
			assert_eq!(strip_markdown_for_speech(text), text);
		}
		assert_eq!(strip_markdown_for_speech("(*bold*), _it_."), "(bold), it.");
	}

	#[test]
	fn test_strip_markdown_for_speech() {
		let raw = "*Hello* _world_ ~test~ `code`";
		assert_eq!(strip_markdown_for_speech(raw), "Hello world test code");

		let fenced = "intro\n```\nfunc foo() {}\n```\noutro";
		assert_eq!(strip_markdown_for_speech(fenced), "intro\nfunc foo() {}\noutro");
	}
}
