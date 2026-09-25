# key-chord

A keyboard shortcut chord: some modifiers plus one key, with the parsing, formatting and
matching an app needs to store shortcuts in a config file and dispatch them at runtime.

Dependency-free by design. `serde` is an optional feature and nothing else is pulled in, so a
GUI-free core library can own an app's keymap and its defaults without linking a toolkit, and the
same type reaches the shortcut-editing UI unchanged.

## What it does

```rust
use key_chord::KeyChord;

// Build one, and write it to a config file.
let chord = KeyChord::new(true, false, true, "o");
assert_eq!(chord.to_shortcut_string(), "Ctrl+Shift+O");

// Read it back. `None` means the action is deliberately unbound.
assert_eq!(KeyChord::parse("Ctrl+Shift+O"), Some(chord.clone()));
assert_eq!(KeyChord::parse("none"), None);

// Dispatch a key event against it.
assert!(chord.matches(79, true, false, true));
```

Key names are normalized, so `"esc"`, `"Escape"` and `"ESC"` are the same chord and a config file
edited by hand still matches.

`matches` accepts both a key's ASCII code and its numpad or extended equivalent, so a binding on
`.` also fires from the numpad decimal point.

## Ctrl and raw Ctrl

macOS has two keys where Windows and Linux have one, so a chord carries both.

* `ctrl` is the *command* modifier: Cmd on macOS, Ctrl elsewhere. This is what an ordinary menu
  accelerator wants, and what `KeyChord::new` produces.
* `raw_ctrl` is the physical Control key, which stays Control on macOS. Build one with
  `KeyChord::new_raw_ctrl` when a binding has to survive as physical Control on a Mac.

They serialize as `Ctrl+` and `RawCtrl+`, and `conflicts_with` treats a `ctrl` chord and a
`raw_ctrl` chord on the same key as a collision, since they are the same keystroke everywhere
except macOS.

## Key codes

`from_key_code` and `matches` speak wxWidgets key codes. The values are plain integers and the
crate has no toolkit dependency, but the numbers themselves come from `wxKeyCode`.

## Features

* `serde` — derives `Serialize` and `Deserialize` on `KeyChord`. `raw_ctrl` is `#[serde(default)]`,
  so a config written before that field existed still loads.
