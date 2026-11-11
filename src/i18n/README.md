# Localization in whatSnap

whatSnap uses an embedded localization module in `src/i18n/` alongside build-time `.mo` gettext catalogs generated in `build.rs` (for `wxWidgets`, `wx-utils`, and `ship-shape` stock dialogs via `patois`). Strings without a translation in the active locale fall back to their English source text.

## Layout

- `src/i18n/mod.rs`: `LanguageChoice`, `LANGUAGE_CHOICES`, `tr()`, `tr_str()`, and embedded `.mo` registration.
- `src/i18n/locales/<lang>.rs`: UI string lookup table (`translate(&str) -> Option<&'static str>`).
- `src/i18n/backend.rs`: Pattern-based translation for backend error messages, system events, and call summaries.
- `src/i18n/countries.rs`: Localized country names for phone country-code selectors.
- `build.rs`: Compiles embedded `patois` and `wxstd` `.mo` catalogs at build time.

## Adding a New Language

Adding a language takes three steps:

### 1. Create the locale translation file

Create `src/i18n/locales/<language_code>.rs` (for example `es.rs` for Spanish):

```rust
pub fn translate(text: &str) -> Option<&'static str> {
	match text {
		"Chats" => Some("Chats"),
		"Status" => Some("Estados"),
		"Starred Messages" => Some("Mensajes destacados"),
		"&Settings..." => Some("&Configuración..."),
		_ => None,
	}
}
```

You can copy `src/i18n/locales/id.rs` as a template.

### 2. Export the locale module

In `src/i18n/locales/mod.rs`, expose the new module:

```rust
pub mod es;
pub mod id;
```

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
	LanguageChoice::Id => locales::id::translate(text),
	LanguageChoice::Es => locales::es::translate(text),
	_ => None,
}
```

The new option will appear in Settings under Personalization -> App Language.
