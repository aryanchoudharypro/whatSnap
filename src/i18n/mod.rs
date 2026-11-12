pub mod backend;
pub mod countries;
pub mod po;

use std::sync::{
	OnceLock,
	atomic::{AtomicUsize, Ordering},
};

pub use backend::{
	translate_backend_message, translate_call_kind_text, translate_rich_message_text, translate_system_message,
};
pub use countries::{translate_country_entry, translate_country_name};

const PATOIS_ID_MO_DEFLATE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/patois_id.mo.deflate"));
pub const WXSTD_ID_MO: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wxstd_id.mo"));

patois::inventory::submit! {
	patois::EmbeddedDomain {
		name: "whatsnap",
		files: &[("id", PATOIS_ID_MO_DEFLATE)],
	}
}

/// The language picked in Settings: `"system"`, `"en"`, or the code of a `po/<code>.po` catalog.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LanguageChoice(&'static str);

impl LanguageChoice {
	pub const SYSTEM: Self = Self("system");
	pub const EN: Self = Self("en");

	pub fn as_str(&self) -> &'static str {
		self.0
	}

	pub fn from_code(s: &str) -> Self {
		if s == "en" {
			return Self::EN;
		}
		po::LANGUAGE_SOURCES.iter().find(|(code, _, _)| *code == s).map_or(Self::SYSTEM, |(code, _, _)| Self(code))
	}
}

impl Default for LanguageChoice {
	fn default() -> Self {
		Self::SYSTEM
	}
}

impl serde::Serialize for LanguageChoice {
	fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.serialize_str(self.0)
	}
}

impl<'de> serde::Deserialize<'de> for LanguageChoice {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let code = String::deserialize(deserializer)?;
		Ok(Self::from_code(&code))
	}
}

/// The Settings choices as (label, value): System Default, English, then one per catalog.
pub fn language_choices() -> &'static [(&'static str, &'static str)] {
	static CHOICES: OnceLock<Vec<(&'static str, &'static str)>> = OnceLock::new();
	CHOICES.get_or_init(|| {
		let mut choices = vec![("System Default", "system"), ("English", "en")];
		choices.extend(po::LANGUAGE_SOURCES.iter().map(|(code, name, _)| (*name, *code)));
		choices
	})
}

/// Index of the active catalog in `po::LANGUAGE_SOURCES` plus one; 0 is English.
static ACTIVE_LANG: AtomicUsize = AtomicUsize::new(0);

pub fn set_language(choice: LanguageChoice) {
	let code = if choice == LanguageChoice::SYSTEM { system_language_code() } else { choice.0.to_string() };
	let index = po::catalog_index(&code).map_or(0, |i| i + 1);
	ACTIVE_LANG.store(index, Ordering::Relaxed);
	patois::set_default_domain("whatsnap");
	patois::set_locale(if index > 0 && code == "id" { "id" } else { "en" });
}

pub fn is_indonesian() -> bool {
	active_index().is_some_and(|i| po::LANGUAGE_SOURCES[i].0 == "id")
}

pub fn current_language() -> LanguageChoice {
	active_index().map_or(LanguageChoice::EN, |i| LanguageChoice(po::LANGUAGE_SOURCES[i].0))
}

fn active_index() -> Option<usize> {
	ACTIVE_LANG.load(Ordering::Relaxed).checked_sub(1)
}

/// The language code of the system, such as "id" or "en".
fn system_language_code() -> String {
	#[cfg(windows)]
	{
		use windows::Win32::Globalization::{GetUserDefaultUILanguage, LCIDToLocaleName};
		let lang_id = unsafe { GetUserDefaultUILanguage() };
		let mut buf = [0u16; 85];
		let len = unsafe { LCIDToLocaleName(u32::from(lang_id), Some(&mut buf), 0) };
		if len > 1 {
			let name = String::from_utf16_lossy(&buf[..len as usize - 1]);
			return primary_subtag(&name);
		}
	}

	for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
		if let Ok(val) = std::env::var(var)
			&& !val.is_empty()
		{
			return primary_subtag(&val);
		}
	}
	"en".to_string()
}

fn primary_subtag(locale: &str) -> String {
	let tag = locale.split(['-', '_', '.']).next().unwrap_or("").to_ascii_lowercase();
	// Windows and POSIX once used "in" for Indonesian.
	if tag == "in" { "id".to_string() } else { tag }
}

#[inline]
pub fn tr(text: &'static str) -> &'static str {
	if let Some(index) = active_index()
		&& let Some(translated) = po::translate(index, text)
	{
		return translated;
	}
	text
}

#[inline]
pub fn tr_str(text: &str) -> String {
	if let Some(index) = active_index()
		&& let Some(translated) = po::translate(index, text)
	{
		return translated.to_string();
	}
	text.to_string()
}

/// "1 person" or "5 people" with the noun translated.
pub fn count_noun(count: usize, one: &str, many: &str) -> String {
	if count == 1 { format!("1 {}", tr_str(one)) } else { format!("{count} {}", tr_str(many)) }
}

/// Translates `template` (English text with `{}` placeholders) and fills the placeholders in order.
pub fn tr_fmt(template: &'static str, args: &[&dyn std::fmt::Display]) -> String {
	use std::fmt::Write;
	let text = tr(template);
	let mut out = String::with_capacity(text.len() + 16);
	let mut rest = text;
	let mut args = args.iter();
	while let Some(at) = rest.find("{}") {
		out.push_str(&rest[..at]);
		if let Some(arg) = args.next() {
			let _ = write!(out, "{arg}");
		}
		rest = &rest[at + 2..];
	}
	out.push_str(rest);
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn language_choice_round_trips_codes() {
		assert_eq!(LanguageChoice::from_code("en"), LanguageChoice::EN);
		assert_eq!(LanguageChoice::from_code("id").as_str(), "id");
		assert_eq!(LanguageChoice::from_code("system"), LanguageChoice::SYSTEM);
		assert_eq!(LanguageChoice::from_code("unknown"), LanguageChoice::SYSTEM);
		assert!(language_choices().iter().any(|(_, code)| *code == "id"));
	}

	#[test]
	fn primary_subtag_handles_windows_and_posix_names() {
		assert_eq!(primary_subtag("id-ID"), "id");
		assert_eq!(primary_subtag("in_ID.UTF-8"), "id");
		assert_eq!(primary_subtag("en_US"), "en");
	}

	#[test]
	fn tr_fmt_fills_placeholders_in_order() {
		assert_eq!(tr_fmt("{} of {}", &[&1, &"two"]), "1 of two");
	}

	#[test]
	fn indonesian_locale_translates_core_strings() {
		assert_eq!(po::translate_id("Chats"), Some("Chat"));
		assert_eq!(po::translate_id("&Settings..."), Some("&Setelan..."));
		assert_eq!(po::translate_id("App Language"), Some("Bahasa Aplikasi"));
		assert_eq!(po::translate_id("Mark as read"), Some("Tandai sudah dibaca"));
		assert_eq!(po::translate_id("Send voice message"), Some("Kirim pesan suara"));
		assert_eq!(po::translate_id("__untranslated_key__"), None);
	}
}
