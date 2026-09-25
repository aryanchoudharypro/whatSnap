# wx-utils

A collection of ergonomic Rust utilities for desktop application development using wxDragon.

## Features

### AboutBoxBuilder

A fluent interface for creating and displaying native wxAboutBox dialogs.

* Zero manual FFI: no need to create, destroy, or cast `AboutDialogInfo` pointers yourself.
* Automatic C-string handling for every field you set.
* Takes `impl Into<String>`, so translation macros and plain strings both just work.

### Dialog button footers

Build and lay out a standard OK/Cancel row in two steps, meant to be used together:

* `build_ok_cancel_buttons` constructs the pair and wires the logical behavior that never varies by platform: `ID_OK`/`ID_CANCEL`, Escape cancels, Enter/default confirms. It does not decide on-screen order.
* `add_ok_cancel_footer` (and `add_single_button_footer` for a single dismiss button) then lays the buttons out with a native `wxStdDialogButtonSizer`, which is what actually enforces the platform HIG order (Cancel/OK on macOS, OK/Cancel on Windows), regardless of the order you passed the buttons in.
* `bind_enter_confirms` submits a dialog when Enter is pressed in a text or spin control.
* `build_yes_no_buttons` and `add_yes_no_footer` are the Yes/No counterparts, and `confirm` is the whole dialog in one call.
* `DIALOG_PADDING` is the padding constant these functions use, exposed so your own sizers can match; `dialog_padding` returns it scaled for the display the given window is on, via wxDragon's `WxWidget::from_dip_int`.

Each `build_*_buttons` function has an `_on` variant that parents the buttons to a `Panel` (or any other window) instead of to the dialog itself, for dialogs whose content lives in a panel.

The Cancel label is translated automatically (see Translation below); you only supply the OK label, since that varies by dialog ("OK", "Go", etc).

### Why `confirm` rather than a Yes/No `MessageDialog`

On Windows, `MessageDialogStyle::YesNo` is the native task dialog, and its buttons are labeled by the *operating system*, in the system language. An app translated into French running on an English Windows shows a French question above English Yes/No buttons. `confirm` builds real `Button`s labeled through patois, so they read in the app's own language like the rest of the dialog.

### Keyboard shortcuts

`shortcuts::prompt_for_shortcuts` is a whole Customize Keyboard Shortcuts dialog: a tab per group of actions, a list of them with their current bindings, Set/Clear/Reset/Reset All, conflict detection, and a capture dialog that records whatever key combination you press and announces it to a screen reader as you type.

Your app keeps its own action type and its own storage. Implement `shortcuts::ShortcutModel` over whatever your config already looks like and the dialog does the rest, handing back an edited copy when the user confirms.

`ShortcutModel::tab_kind` is the one thing worth reading twice. `TabKind::SharedKeymap` means the tabs are views onto one keymap, so a chord has to be unique across all of them; that is what tabs-by-category want. `TabKind::SeparateKeymaps` means each tab is a keymap of its own and the same key can mean different things in each; that is what tabs-by-input-mode want. It decides how far conflict detection looks.

Behind the `shortcuts` feature, which is off by default since it pulls in [live-region](https://github.com/trypsynth/live-region) for the announcements. The chord type itself is [key-chord](https://github.com/trypsynth/key-chord), re-exported as `shortcuts::KeyChord`.

### Menus

* `menu_label` joins an item's text and its shortcut into the `"Open	Ctrl+O"` form wx wants. An empty shortcut gives back a bare label rather than a trailing tab.
* `set_menu_item_label` relabels an existing item in a `MenuBar`, and does nothing if the id isn't there, so a menu that varies by platform can be relabelled in one pass.

### Durations

`format_duration_seconds` and `format_duration_ms` write a duration as a localized list of its non-zero segments, such as "1 hour, 5 minutes, 3 seconds". In words rather than as `1:05:03`, because this is text a screen reader reads aloud and a colon-separated form is liable to be read as a time of day.

### Prompts and messages

* `prompt_number` shows a labeled spin control dialog and returns the entered value, or `None` if cancelled.
* `prompt_text` shows a single-line text entry dialog and returns the trimmed value, or `None` if cancelled or blank.
* `show_error` and `show_warning` show a modal message dialog with the matching icon.

## Translation

wx-utils is marked `[package.metadata.patois] translatable = true`. If your app uses [patois](https://github.com/trypsynth/patois) and calls `patois_build::gen_pot` over your workspace, wx-utils's own translatable strings (like "Cancel") are picked up into your app's catalog automatically. No extra setup needed.

## Quick Start

```rust
use wx_utils::AboutBoxBuilder;
use wxdragon::prelude::*;

// Inside a Frame event handler or menu callback
pub fn on_about(parent: &Frame) {
	AboutBoxBuilder::new(parent)
		.name("Paperback")
		.version(env!("CARGO_PKG_VERSION"))
		.description("An accessible, lightweight, fast ebook and document reader.")
		.copyright("Copyright (C) 2025-2026 Quin Gillespie")
		.website("https://paperback.dev")
		.add_developer("Quin Gillespie")
		.show();
}
```

```rust
use wx_utils::confirm;
use wxdragon::prelude::*;

pub fn confirm_delete(parent: &Frame) -> bool {
	confirm(parent, "Delete this file permanently?", "Delete File")
}
```

For a dialog of your own, the button helpers lay out the footer:

```rust
use wx_utils::{add_ok_cancel_footer, build_ok_cancel_buttons, dialog_padding};
use wxdragon::prelude::*;

pub fn prompt_for_name(parent: &Frame) -> Option<String> {
	let dialog = Dialog::builder(parent, "Rename").build();
	let padding = dialog_padding(&dialog);
	let entry = TextCtrl::builder(&dialog).build();
	let (ok_button, cancel_button) = build_ok_cancel_buttons(&dialog, "Rename");
	let content = BoxSizer::builder(Orientation::Vertical).build();
	content.add(&entry, 0, SizerFlag::Expand | SizerFlag::All, padding);
	add_ok_cancel_footer(content, ok_button, cancel_button);
	dialog.set_sizer_and_fit(content, true);
	dialog.centre();
	if dialog.show_modal() == ID_OK { Some(entry.get_value()) } else { None }
}
```

## Todo

* ReadmeOpener: open local/remote readmes in the user's default browser.
* Automatic help menu generation.
