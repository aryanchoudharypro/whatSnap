pub mod backend;
pub mod countries;
pub mod locales;

use std::sync::atomic::{AtomicU8, Ordering};

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

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LanguageChoice {
	#[default]
	System,
	En,
	Id,
}

impl LanguageChoice {
	pub fn as_str(&self) -> &'static str {
		match self {
			Self::System => "system",
			Self::En => "en",
			Self::Id => "id",
		}
	}

	pub fn from_code(s: &str) -> Self {
		match s {
			"en" => Self::En,
			"id" => Self::Id,
			_ => Self::System,
		}
	}
}

pub const LANGUAGE_CHOICES: &[(&str, &str)] =
	&[("System Default", "system"), ("English", "en"), ("Bahasa Indonesia", "id")];

static ACTIVE_LANG: AtomicU8 = AtomicU8::new(0);

pub fn set_language(choice: LanguageChoice) {
	let is_id = match choice {
		LanguageChoice::Id => true,
		LanguageChoice::En => false,
		LanguageChoice::System => is_system_indonesian(),
	};
	ACTIVE_LANG.store(if is_id { 1 } else { 0 }, Ordering::Relaxed);
	patois::set_default_domain("whatsnap");
	patois::set_locale(if is_id { "id" } else { "en" });
}

pub fn is_indonesian() -> bool {
	ACTIVE_LANG.load(Ordering::Relaxed) == 1
}

pub fn current_language() -> LanguageChoice {
	if is_indonesian() { LanguageChoice::Id } else { LanguageChoice::En }
}

fn is_system_indonesian() -> bool {
	#[cfg(windows)]
	{
		let lang_id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
		// Primary language ID 0x21 is Indonesian (LANG_INDONESIAN).
		if (lang_id & 0x03ff) == 0x21 {
			return true;
		}
	}

	for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
		if let Ok(val) = std::env::var(var) {
			let lower = val.to_ascii_lowercase();
			if lower.starts_with("id") || lower.starts_with("in_") {
				return true;
			}
		}
	}

	false
}

#[inline]
pub fn tr(text: &'static str) -> &'static str {
	if is_indonesian()
		&& let Some(translated) = locales::id::translate(text)
	{
		return translated;
	}
	text
}

#[inline]
pub fn tr_str(text: &str) -> String {
	if is_indonesian()
		&& let Some(translated) = locales::id::translate(text)
	{
		return translated.to_string();
	}
	text.to_string()
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
		assert_eq!(LanguageChoice::from_code("en"), LanguageChoice::En);
		assert_eq!(LanguageChoice::from_code("id"), LanguageChoice::Id);
		assert_eq!(LanguageChoice::from_code("system"), LanguageChoice::System);
		assert_eq!(LanguageChoice::from_code("unknown"), LanguageChoice::System);
		assert_eq!(LanguageChoice::System.as_str(), "system");
		assert_eq!(LanguageChoice::En.as_str(), "en");
		assert_eq!(LanguageChoice::Id.as_str(), "id");
	}

	#[test]
	fn tr_fmt_fills_placeholders_in_order() {
		assert_eq!(tr_fmt("{} of {}", &[&1, &"two"]), "1 of two");
	}

	#[test]
	fn indonesian_locale_translates_core_strings() {
		assert_eq!(locales::id::translate("Chats"), Some("Chat"));
		assert_eq!(locales::id::translate("&Settings..."), Some("&Setelan..."));
		assert_eq!(locales::id::translate("App Language"), Some("Bahasa Aplikasi"));
		assert_eq!(locales::id::translate("Mark as read"), Some("Tandai sudah dibaca"));
		assert_eq!(locales::id::translate("Send voice message"), Some("Kirim pesan suara"));
		assert_eq!(locales::id::translate("__untranslated_key__"), None);
	}
}
