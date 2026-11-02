//! A tone or ringtone list with a Custom entry, and right after it a button
//! to choose the custom sound file, only usable while Custom is picked (like
//! the Except buttons on the Privacy tab).

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	audio::sound_file,
	ui::{dialog_look, theme},
};

/// What a picker picks, which decides its list.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
	/// Settings' message and group tones: Default, Alert 1 to 10, Custom.
	/// Off is the sounds switch next to them.
	Tone,
	/// A chat's tone: the same plus None.
	ChatTone,
	/// Settings' call and group call ringtones: WhatsApp's, the Windows
	/// ringtones, Custom.
	Ringtone,
	/// A chat's ringtone: Default (as in Settings), the same, None.
	ChatRingtone,
}

/// The list entry for Custom; its file is kept as the browse button's name.
const CUSTOM: &str = "custom";

impl Kind {
	/// Each entry's value: a tone number for tones, a ringtone (see
	/// [`whatsnap::RINGTONE_WHATSAPP`]) for ringtones, then labels.
	fn entries(self, default_label: &str) -> Vec<(String, String)> {
		let mut entries: Vec<(String, String)> = Vec::new();
		match self {
			Kind::Tone | Kind::ChatTone => {
				entries.push((default_label.to_string(), whatsnap::TONE_DEFAULT.to_string()));
				for tone in 2..=whatsnap::TONE_LAST {
					entries.push((whatsnap::tone_label(tone), tone.to_string()));
				}
			}
			Kind::Ringtone | Kind::ChatRingtone => {
				if self == Kind::ChatRingtone {
					entries.push((default_label.to_string(), String::new()));
				}
				let whatsapp = if self == Kind::Ringtone { "" } else { whatsnap::RINGTONE_WHATSAPP };
				entries.push(("WhatsApp".to_string(), whatsapp.to_string()));
				for (name, path) in sound_file::windows_ringtones() {
					entries.push((name.clone(), path.clone()));
				}
			}
		}
		entries.push(("Custom sound file".to_string(), CUSTOM.to_string()));
		match self {
			Kind::ChatTone => entries.push(("None".to_string(), whatsnap::TONE_NONE.to_string())),
			Kind::ChatRingtone => entries.push(("None".to_string(), whatsnap::RINGTONE_NONE.to_string())),
			_ => {}
		}
		entries
	}

	fn is_tone(self) -> bool {
		matches!(self, Kind::Tone | Kind::ChatTone)
	}
}

#[derive(Clone, Copy)]
pub struct SoundPicker {
	pub combo: ComboBox,
	pub browse: Button,
	kind: Kind,
}

/// What a picker can't keep in its Copy self, by browse button id: the
/// sound's name for the button, and what plays a pick.
struct Extra {
	what: String,
	preview: Option<Preview>,
}

type Preview = Rc<dyn Fn(&SoundPicker)>;

thread_local! {
	static EXTRAS: RefCell<HashMap<i32, Extra>> = RefCell::new(HashMap::new());
}

impl SoundPicker {
	/// `what` names the sound on the browse button, which a screen reader
	/// reads on its own ("Choose Message Tone File..."). `default_label` is
	/// the Default entry's label, where the list has one.
	pub fn new(panel: &Panel, sizer: &BoxSizer, label: &str, what: &str, kind: Kind, default_label: &str) -> Self {
		let text = StaticText::builder(panel).with_label(label).build();
		text.set_font(&theme::Fonts::body());
		sizer.add(&text, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);
		let entries = kind.entries(default_label);
		let labels: Vec<&str> = entries.iter().map(|(label, _)| label.as_str()).collect();
		let combo = ComboBox::builder(panel).with_string_choices(&labels).with_style(ComboBoxStyle::ReadOnly).build();
		combo.set_font(&theme::Fonts::body());
		combo.set_selection(0);
		sizer.add(&combo, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);

		let browse = Button::builder(panel).with_label(&format!("Choose {what} File...")).build();
		dialog_look::pill_button(&browse, false, theme::Palette::CONTENT_BG());
		browse.enable(false);
		sizer.add(&browse, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Bottom, 8);
		EXTRAS.with(|extras| {
			extras.borrow_mut().insert(browse.get_id(), Extra { what: what.to_string(), preview: None })
		});

		let picker = Self { combo, browse, kind };
		combo.on_selection_changed(move |_| {
			picker.browse.enable(picker.is_custom());
			if !picker.is_custom() || !picker.file().is_empty() {
				picker.play();
			}
		});
		browse.on_click(move |_| {
			let dialog = FileDialog::builder(&picker.browse)
				.with_message("Choose a sound file")
				.with_wildcard(sound_file::BROWSE_WILDCARD)
				.with_style(FileDialogStyle::Open | FileDialogStyle::FileMustExist)
				.build();
			let path = if dialog.show_modal() == ID_OK { dialog.get_path() } else { None };
			dialog.destroy();
			if let Some(path) = path {
				picker.set_file(&path);
				picker.play();
			}
		});
		picker
	}

	fn selected_value(&self) -> String {
		let entries = self.kind.entries("");
		let at = self.combo.get_selection().map_or(0, |i| i as usize).min(entries.len() - 1);
		entries[at].1.clone()
	}

	fn is_custom(&self) -> bool {
		self.selected_value() == CUSTOM
	}

	/// The custom file, kept as the button's (never shown) window name.
	fn file(&self) -> String {
		self.browse.get_name()
	}

	/// Remembers the custom file and names it on the button.
	fn set_file(&self, path: &str) {
		self.browse.set_name(path);
		let what = EXTRAS.with(|extras| extras.borrow().get(&self.browse.get_id()).map(|e| e.what.clone()));
		let what = what.unwrap_or_else(|| "Sound".to_string());
		let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned());
		match name {
			Some(name) => self.browse.set_label(&format!("Choose {what} File... ({name})")),
			None => self.browse.set_label(&format!("Choose {what} File...")),
		}
		if let Some(parent) = self.browse.get_parent() {
			parent.layout();
		}
	}

	fn select(&self, value: &str, file: &str) {
		let entries = self.kind.entries("");
		let at = entries.iter().position(|(_, v)| v == value).unwrap_or(0);
		self.combo.set_selection(at as u32);
		self.set_file(file);
		self.browse.enable(self.is_custom());
	}

	pub fn set_tone(&self, tone: u8, file: &str) {
		if tone == whatsnap::TONE_CUSTOM { self.select(CUSTOM, file) } else { self.select(&tone.to_string(), file) }
	}

	/// The tone picked, with the custom file (kept even when another tone is
	/// picked, so going back to Custom finds it). Custom without a file is
	/// Default.
	pub fn tone(&self) -> (u8, String) {
		match self.selected_value().as_str() {
			CUSTOM if !self.file().is_empty() => (whatsnap::TONE_CUSTOM, self.file()),
			value => (value.parse().unwrap_or(whatsnap::TONE_DEFAULT), self.file()),
		}
	}

	/// A ringtone that is a file not in the list is Custom.
	pub fn set_ringtone(&self, ringtone: &str) {
		let listed = self.kind.entries("").iter().any(|(_, v)| v == ringtone);
		if listed { self.select(ringtone, "") } else { self.select(CUSTOM, ringtone) }
	}

	/// The ringtone picked. Custom without a file is the first entry.
	pub fn ringtone(&self) -> String {
		match self.selected_value().as_str() {
			CUSTOM if !self.file().is_empty() => self.file(),
			CUSTOM => self.kind.entries("")[0].1.clone(),
			value => value.to_string(),
		}
	}

	/// Sets what plays a pick from the list or a chosen file, as WhatsApp's
	/// tone list plays each pick.
	pub fn on_preview(&self, play: impl Fn(&SoundPicker) + 'static) {
		EXTRAS.with(|extras| {
			if let Some(extra) = extras.borrow_mut().get_mut(&self.browse.get_id()) {
				extra.preview = Some(Rc::new(play));
			}
		});
	}

	/// Drops what [`Self::new`] kept for a picker in a dialog being destroyed.
	pub fn forget(&self) {
		EXTRAS.with(|extras| extras.borrow_mut().remove(&self.browse.get_id()));
	}

	fn play(&self) {
		let play = EXTRAS.with(|extras| extras.borrow().get(&self.browse.get_id()).and_then(|e| e.preview.clone()));
		if let Some(play) = play {
			play(self);
		}
	}

	/// Plays what's picked: a tone, or a short bit of a ringtone. What Default
	/// stands for in a chat's list is the caller's to play.
	pub fn preview(&self, output_device: Option<String>) {
		if self.kind.is_tone() {
			let (tone, file) = self.tone();
			crate::audio::preview_message_tone(tone, &file, output_device);
		} else {
			crate::audio::preview_ringtone(&self.ringtone(), output_device);
		}
	}
}
