# Localization in whatSnap

whatSnap uses an embedded localization module in `src/i18n/` alongside build-time `.mo` gettext catalogs generated in `build.rs` (for `wxWidgets`, `wx-utils`, and `ship-shape` stock dialogs via `patois`). Strings without a translation in the active locale fall back to their English source text.

## Layout

- `src/i18n/mod.rs`: `LanguageChoice`, `LANGUAGE_CHOICES`, `tr()`, `tr_str()`, and embedded `.mo` registration.
- `po/<lang>.po`: the UI translations, a plain gettext file. `msgid` is the English text used in the code, `msgstr` is the translation. Any gettext editor works.
- `src/i18n/po.rs`: embeds `po/id.po` and looks strings up in it (`translate_id`).
- `src/i18n/backend.rs`: Pattern-based translation for backend error messages, system events, and call summaries.
- `src/i18n/countries.rs`: Localized country names for phone country-code selectors.
- `build.rs`: Compiles embedded `patois` and `wxstd` `.mo` catalogs at build time.

## Translating

Edit `po/<lang>.po`. Each entry is a `msgid` (the English text exactly as it appears in the code) and a `msgstr` (your translation). Keep `{}` placeholders and `&` keyboard-accelerator marks in the translation. An empty or missing `msgstr` falls back to English. Plural forms are separate entries, for example `{} message selected` and `{} messages selected`.

## Adding a New Language

Adding a language takes three steps:

### 1. Create the catalog

Copy `po/id.po` to `po/<language_code>.po` and translate the `msgstr` lines.

### 2. Embed it

In `src/i18n/po.rs`, add a `translate_<language_code>` function next to `translate_id` that embeds the new file with `include_str!` and parses it with `parse`.

### 3. Register the language in `src/i18n/mod.rs`

1. Add the variant to `LanguageChoice`:

```rust
pub enum LanguageChoice {
	#[default]
	System,
	En,
	Id,
	Es,
}
```

2. Update `as_str` and `from_code` in `LanguageChoice`, and add an entry to `LANGUAGE_CHOICES`:

```rust
pub const LANGUAGE_CHOICES: &[(&str, &str)] =
	&[("System Default", "system"), ("English", "en"), ("Bahasa Indonesia", "id"), ("Español", "es")];
```

3. Route lookups for the new language in `tr` and `tr_str` (or extract a shared `translate_for(choice, text)` helper):

```rust
match current_language() {
	LanguageChoice::Id => po::translate_id(text),
	LanguageChoice::Es => po::translate_es(text),
	_ => None,
}
```

The new option will appear in Settings under Personalization -> App Language.
