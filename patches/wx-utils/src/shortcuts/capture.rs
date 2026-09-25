//! The Set Shortcut dialog: a modifier row plus a field that captures whatever key you press.

use std::{cell::Cell, rc::Rc};

use patois::t;
use wxdragon::prelude::*;

use super::KeyChord;
use crate::{build_ok_cancel_buttons_on, dialog_padding};

/// Modal return code for the Clear button, telling [`prompt_for_key_chord`] the user wants the
/// action unbound rather than cancelled. Well above `wxID_HIGHEST` (5999) so it cannot collide
/// with a stock wx id, and private to this dialog, so it cannot collide with an app's own.
const ID_CLEAR_SHORTCUT: i32 = 10099;

/// Key codes the capture field must let through rather than record.
///
/// Tab and Escape keep the dialog operable by keyboard, and the arrow keys keep a screen
/// reader's own navigation working. Anything else the user presses is a shortcut they meant.
const RESERVED_KEYS: [i32; 10] = [9, 27, 314, 315, 316, 317, 378, 380, 382, 383];

/// Asks the user for a key combination for `action_name`.
///
/// The two levels of `Option` are different answers: `None` means the user cancelled and
/// nothing should change, `Some(None)` means they cleared the binding on purpose, and
/// `Some(Some(chord))` is a new binding.
// The nested Option carries three distinct answers, spelled out above; and the body is one
// linear run of widget construction, which splitting would scatter rather than clarify.
#[allow(clippy::option_option, clippy::too_many_lines)]
pub(super) fn prompt_for_key_chord(
	parent: &dyn WxWidget,
	action_name: &str,
	initial: Option<&KeyChord>,
) -> Option<Option<KeyChord>> {
	// TRANSLATORS: Title of the Set Shortcut dialog. The {} placeholder is replaced with the action's display name.
	let title = t("Set Shortcut for {}").replace("{}", action_name);
	let dialog = Dialog::builder(parent, &title).with_size(parent.from_dip_int(400), parent.from_dip_int(260)).build();
	let padding = dialog_padding(&dialog);
	let panel = Panel::builder(&dialog).build();
	let main_sizer = BoxSizer::builder(Orientation::Vertical).build();
	// A zero-sized hidden label, present only to carry screen reader announcements: updating a
	// visible StaticText's text is silent to assistive tech, so the detected chord would never
	// be spoken as the user types it.
	let live_region_label = StaticText::builder(&panel).with_label("").with_size(Size::new(0, 0)).build();
	live_region_label.show(false);
	let _ = live_region::set_live_region(&live_region_label);
	// TRANSLATORS: Instruction text in the Set Shortcut dialog. The {} placeholder is replaced with the action's display name.
	let info_text = t("Configure shortcut for {}:").replace("{}", action_name);
	let info_label = StaticText::builder(&panel).with_label(&info_text).build();
	main_sizer.add(&info_label, 0, SizerFlag::Expand | SizerFlag::All, padding);
	// TRANSLATORS: Checkbox in the Set Shortcut dialog that includes the Ctrl modifier in the shortcut.
	let ctrl_cb = CheckBox::builder(&panel).with_label(&t("&Ctrl")).build();
	// TRANSLATORS: Checkbox in the Set Shortcut dialog that includes the Alt modifier in the shortcut.
	let alt_cb = CheckBox::builder(&panel).with_label(&t("&Alt")).build();
	// TRANSLATORS: Checkbox in the Set Shortcut dialog that includes the Shift modifier in the shortcut.
	let shift_cb = CheckBox::builder(&panel).with_label(&t("&Shift")).build();
	// One checkbox stands for both Ctrl and physical Control, since they are the same key off
	// macOS and the distinction would be noise in the UI. A chord that arrived as raw Control
	// keeps that until the user touches the modifiers or captures a new key, at which point it
	// becomes an ordinary Ctrl chord like anything else they could have typed here.
	let raw_ctrl = Rc::new(Cell::new(initial.is_some_and(|chord| chord.raw_ctrl)));
	if let Some(chord) = initial {
		ctrl_cb.set_value(chord.ctrl || chord.raw_ctrl);
		alt_cb.set_value(chord.alt);
		shift_cb.set_value(chord.shift);
	}
	let mod_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let mod_gap = dialog.from_dip_int(12);
	mod_sizer.add(&ctrl_cb, 0, SizerFlag::Right, mod_gap);
	mod_sizer.add(&alt_cb, 0, SizerFlag::Right, mod_gap);
	mod_sizer.add(&shift_cb, 0, SizerFlag::Right, mod_gap);
	main_sizer.add_sizer(
		&mod_sizer,
		0,
		SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom,
		padding,
	);
	// TRANSLATORS: Label next to the key field in the Set Shortcut dialog.
	let key_label = StaticText::builder(&panel).with_label(&t("&Key:")).build();
	let key_text_ctrl = TextCtrl::builder(&panel)
		.with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly | TextCtrlStyle::DontWrap)
		.build();
	if let Some(chord) = initial {
		key_text_ctrl.set_value(&chord.key);
	}
	let key_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	key_sizer.add(&key_label, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, padding);
	key_sizer.add(&key_text_ctrl, 1, SizerFlag::Expand, 0);
	main_sizer.add_sizer(&key_sizer, 0, SizerFlag::Expand | SizerFlag::All, padding);
	// TRANSLATORS: Hint text in the Set Shortcut dialog explaining how to capture a new key combination.
	let hint_label = StaticText::builder(&panel)
		.with_label(&t("Tip: click in the key field and press the key combination you want."))
		.build();
	main_sizer.add(&hint_label, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, padding);
	// TRANSLATORS: Placeholder shown in the Set Shortcut dialog before any key combination has been captured
	let preview_label = StaticText::builder(&panel).with_label(&t("Detected: (none)")).build();
	main_sizer.add(&preview_label, 0, SizerFlag::Expand | SizerFlag::All, padding);
	let current_shortcut_text = move || {
		let key = key_text_ctrl.get_value();
		let trimmed = key.trim().to_string();
		if trimmed.is_empty() {
			None
		} else {
			let chord = KeyChord::new(ctrl_cb.get_value(), alt_cb.get_value(), shift_cb.get_value(), &trimmed);
			Some(chord.to_shortcut_string())
		}
	};
	let refresh_preview_label = move || {
		// TRANSLATORS: Preview text in the Set Shortcut dialog showing the key combination captured so far. The {} placeholder is replaced with the shortcut, e.g. "Ctrl+Shift+K".
		let text =
			current_shortcut_text().map_or_else(|| t("Detected: (none)"), |s| t("Detected: {}").replace("{}", &s));
		preview_label.set_label(&text);
	};
	refresh_preview_label();
	let update_preview = move || {
		refresh_preview_label();
		// TRANSLATORS: Accessibility announcement in the Set Shortcut dialog spoken when no key combination has been captured yet
		let announce_text =
			current_shortcut_text().map_or_else(|| t("No key detected"), |s| t("Detected: {}").replace("{}", &s));
		live_region::announce(live_region_label, &announce_text);
	};
	let update_preview_key = update_preview;
	let raw_ctrl_key = raw_ctrl.clone();
	key_text_ctrl.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key_event) = event
			&& let Some(k) = key_event.get_key_code()
			&& !RESERVED_KEYS.contains(&k)
			&& let Some(parsed) =
				KeyChord::from_key_code(k, key_event.control_down(), key_event.alt_down(), key_event.shift_down())
		{
			raw_ctrl_key.set(false);
			ctrl_cb.set_value(parsed.ctrl);
			alt_cb.set_value(parsed.alt);
			shift_cb.set_value(parsed.shift);
			key_text_ctrl.set_value(&parsed.key);
			update_preview_key();
			event.skip(false);
			return;
		}
		event.skip(true);
	});
	let update_preview_ctrl = update_preview;
	let raw_ctrl_toggle = raw_ctrl.clone();
	ctrl_cb.on_toggled(move |_| {
		raw_ctrl_toggle.set(false);
		update_preview_ctrl();
	});
	let update_preview_alt = update_preview;
	alt_cb.on_toggled(move |_| update_preview_alt());
	let update_preview_shift = update_preview;
	shift_cb.on_toggled(move |_| update_preview_shift());
	// TRANSLATORS: OK button in the Set Shortcut dialog that accepts the captured key combination.
	let (ok_button, cancel_button) = build_ok_cancel_buttons_on(&panel, &dialog, &t("OK"));
	// TRANSLATORS: Button in the Set Shortcut dialog that clears the captured key combination.
	let clear_button = Button::builder(&panel).with_id(ID_CLEAR_SHORTCUT).with_label(&t("&Clear")).build();
	// Clear is not a stock id, so it can't go in the wxStdDialogButtonSizer. It sits to the
	// left of one instead, which still leaves OK and Cancel in the platform's own order.
	let std_button_sizer = StdDialogButtonSizerBuilder::new().build();
	std_button_sizer.add_button(&ok_button);
	std_button_sizer.add_button(&cancel_button);
	std_button_sizer.realize();
	let button_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	button_sizer.add(&clear_button, 0, SizerFlag::AlignCenterVertical, 0);
	button_sizer.add_stretch_spacer(1);
	button_sizer.add_sizer(&std_button_sizer, 0, SizerFlag::Expand, 0);
	main_sizer.add_sizer(&button_sizer, 0, SizerFlag::Expand | SizerFlag::All, padding);
	panel.set_sizer(main_sizer, true);
	let dialog_sizer = BoxSizer::builder(Orientation::Vertical).build();
	dialog_sizer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(dialog_sizer, true);
	let dialog_clear = dialog;
	clear_button.on_click(move |_| {
		dialog_clear.end_modal(ID_CLEAR_SHORTCUT);
	});
	dialog.centre();
	key_text_ctrl.set_focus();
	let res = dialog.show_modal();
	if res == ID_CLEAR_SHORTCUT {
		return Some(None);
	}
	if res != ID_OK {
		return None;
	}
	let key_text = key_text_ctrl.get_value();
	let trimmed = key_text.trim();
	if trimmed.is_empty() {
		return Some(None);
	}
	let chord = if ctrl_cb.get_value() && raw_ctrl.get() {
		KeyChord::new_raw_ctrl(true, alt_cb.get_value(), shift_cb.get_value(), trimmed)
	} else {
		KeyChord::new(ctrl_cb.get_value(), alt_cb.get_value(), shift_cb.get_value(), trimmed)
	};
	Some(Some(chord))
}
