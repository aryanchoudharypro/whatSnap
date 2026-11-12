# Localization in whatSnap

whatSnap uses an embedded localization module in `src/i18n/` alongside build-time `.mo` gettext catalogs generated in `build.rs` (for `wxWidgets`, `wx-utils`, and `ship-shape` stock dialogs via `patois`). Strings without a translation in the active locale fall back to their English source text.

## Layout

- `src/i18n/mod.rs`: `LanguageChoice`, `language_choices()`, `tr()`, `tr_str()`, `tr_fmt()`, and embedded `.mo` registration.
- `po/<lang>.po`: the UI translations, a plain gettext file. `msgid` is the English text used in the code, `msgstr` is the translation. Any gettext editor works.
- `src/i18n/po.rs`: looks strings up in the `po/*.po` files that `build.rs` embeds.
- `src/i18n/backend.rs`: Pattern-based translation for backend error messages, system events, and call summaries.
- `src/i18n/countries.rs`: Localized country names for phone country-code selectors.
- `build.rs`: Compiles embedded `patois` and `wxstd` `.mo` catalogs at build time.
- `tools/i18n.py`: regenerates `po/whatsnap.pot` and syncs the catalogs.

## Translating

Edit `po/<lang>.po`. Each entry is a `msgid` (the English text exactly as it appears in the code) and a `msgstr` (your translation). Keep `{}` placeholders and `&` keyboard-accelerator marks in the translation. An empty or missing `msgstr` falls back to English. Plural forms are separate entries, for example `{} message selected` and `{} messages selected`.

## Adding a New Language

1. Copy `po/id.po` to `po/<language_code>.po` (for example `po/es.po`).
2. Change the `Language:` and `X-Language-Name:` header lines. `X-Language-Name` is the label shown in Settings -> Personalization -> App Language, written in that language.
3. Translate the `msgstr` lines.

That is all: `build.rs` picks up every `po/*.po` file, so there is no Rust to change. The language is also chosen automatically when the Windows display language matches its code and App Language is System Default.

## Keeping catalogs in step with the code

```
python3 tools/i18n.py          # writes po/whatsnap.pot and adds new strings to each .po with an empty msgstr
python3 tools/i18n.py --check  # only reports; exits 1 if a catalog is missing strings or has untranslated ones
```

The script finds strings passed straight to `tr()`, `tr_str()` and `tr_fmt()`. Strings that reach `tr()` through a variable, such as table entries, are not found, so it never removes entries.

## What a catalog does not cover

`src/i18n/backend.rs` and `src/i18n/countries.rs` rewrite text that comes from the Go backend, and date, time and number formats are chosen in code. Those are Indonesian-specific today, so another language gets its interface translated but those pieces stay in English until they are ported.
