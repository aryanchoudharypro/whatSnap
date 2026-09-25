use std::fmt::Display;

use patois::t;
use wxdragon::prelude::*;

/// Default padding used between widgets and dialog edges, in device-independent pixels.
///
/// This is the value at 100% display scaling. Pass it through [`WxWidget::from_dip_int`] before
/// handing it to a sizer, or use [`dialog_padding`], which does that for you - on a 150%
/// display a raw 10 here is two thirds of the intended gap.
pub const DIALOG_PADDING: i32 = 10;

/// [`DIALOG_PADDING`] scaled for the display `reference` is on.
///
/// Read it once per dialog and reuse it, rather than calling this at every sizer `add`.
#[must_use]
pub fn dialog_padding(reference: &dyn WxWidget) -> i32 {
	reference.from_dip_int(DIALOG_PADDING)
}

/// Shows a modal error dialog.
pub fn show_error(parent: &dyn WxWidget, message: impl Display, title: &str) {
	let dialog = MessageDialog::builder(parent, &message.to_string(), title)
		.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
		.build();
	dialog.show_modal();
}

/// Shows a modal warning dialog.
pub fn show_warning(parent: &dyn WxWidget, message: impl Display, title: &str) {
	let dialog = MessageDialog::builder(parent, &message.to_string(), title)
		.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconWarning)
		.build();
	dialog.show_modal();
}

/// Builds a standard OK/Cancel button pair parented to `dialog`.
///
/// `ok_label` is caller-supplied since it varies by dialog (e.g. "OK" vs. a verb like
/// "Go"); the Cancel button is always the localized "Cancel", translated via
/// [`patois::t`]. Uses the stock `ID_OK`/`ID_CANCEL` IDs (so a plain click ends the
/// modal without extra wiring), wires `dialog`'s Escape key and affirmative
/// (Enter-default) behavior to Cancel/OK, and makes OK the visual default button.
///
/// Use [`build_ok_cancel_buttons_on`] when the dialog's content lives in a `Panel` and the
/// buttons need to be parented to that instead.
#[must_use]
pub fn build_ok_cancel_buttons(dialog: &Dialog, ok_label: &str) -> (Button, Button) {
	build_ok_cancel_buttons_on(dialog, dialog, ok_label)
}

/// [`build_ok_cancel_buttons`] with the buttons parented to `parent` rather than to `dialog`
/// itself.
///
/// `parent` is typically a `Panel` filling `dialog`. The dialog is still the thing that gets
/// its Escape and affirmative IDs wired, so both are needed.
#[must_use]
pub fn build_ok_cancel_buttons_on(parent: &dyn WxWidget, dialog: &Dialog, ok_label: &str) -> (Button, Button) {
	let ok_button = Button::builder(parent).with_id(ID_OK).with_label(ok_label).build();
	// TRANSLATORS: Label for the cancellation button
	let cancel_button = Button::builder(parent).with_id(ID_CANCEL).with_label(&t("Cancel")).build();
	dialog.set_escape_id(ID_CANCEL);
	dialog.set_affirmative_id(ID_OK);
	ok_button.set_default();
	(ok_button, cancel_button)
}

/// Builds a Yes/No button pair parented to `dialog`, both labels localized via [`patois::t`].
///
/// The reason to build these rather than reach for a `MessageDialog` with `YesNo`: on Windows
/// that is the native task dialog, whose buttons are labeled by the OS in the *system*
/// language. An app translated into French running on an English Windows shows a French
/// question over English buttons. These are real `Button`s, so they read in the app's own
/// language like everything else in the dialog.
///
/// Uses the stock `ID_YES`/`ID_NO` IDs, wires Escape to No and the affirmative action to Yes,
/// and makes Yes the default button. See [`confirm`] for the whole dialog in one call.
#[must_use]
pub fn build_yes_no_buttons(dialog: &Dialog) -> (Button, Button) {
	build_yes_no_buttons_on(dialog, dialog)
}

/// [`build_yes_no_buttons`] with the buttons parented to `parent` rather than to `dialog`.
#[must_use]
pub fn build_yes_no_buttons_on(parent: &dyn WxWidget, dialog: &Dialog) -> (Button, Button) {
	// TRANSLATORS: Label for the confirmation dialog "Yes" button
	let yes_button = Button::builder(parent).with_id(ID_YES).with_label(&t("&Yes")).build();
	// TRANSLATORS: Label for the confirmation dialog "No" button
	let no_button = Button::builder(parent).with_id(ID_NO).with_label(&t("&No")).build();
	dialog.set_escape_id(ID_NO);
	dialog.set_affirmative_id(ID_YES);
	yes_button.set_default();
	(yes_button, no_button)
}

/// Appends an OK/Cancel button row to `content_sizer`.
///
/// Uses a native `wxStdDialogButtonSizer`, which reorders the buttons and applies
/// spacing to match the platform HIG (Cancel/OK on macOS, OK/Cancel on Windows).
pub fn add_ok_cancel_footer(content_sizer: BoxSizer, ok_button: Button, cancel_button: Button) {
	let button_sizer = StdDialogButtonSizerBuilder::new().build();
	button_sizer.add_button(&ok_button);
	button_sizer.add_button(&cancel_button);
	button_sizer.realize();
	content_sizer.add_sizer(&button_sizer, 0, SizerFlag::Expand | SizerFlag::All, dialog_padding(&ok_button));
}

/// Appends a Yes/No button row to `content_sizer`.
///
/// Uses a native `wxStdDialogButtonSizer`, which reorders the buttons and applies spacing to
/// match the platform HIG, the same as [`add_ok_cancel_footer`].
pub fn add_yes_no_footer(content_sizer: BoxSizer, yes_button: Button, no_button: Button) {
	let button_sizer = StdDialogButtonSizerBuilder::new().build();
	button_sizer.add_button(&yes_button);
	button_sizer.add_button(&no_button);
	button_sizer.realize();
	content_sizer.add_sizer(&button_sizer, 0, SizerFlag::Expand | SizerFlag::All, dialog_padding(&yes_button));
}

/// Appends a single-button row (e.g. "Close") to `content_sizer`.
///
/// Uses a native `wxStdDialogButtonSizer`. The single-button counterpart to
/// [`add_ok_cancel_footer`], for dialogs that only need a dismiss action.
pub fn add_single_button_footer(content_sizer: BoxSizer, button: Button) {
	let button_sizer = StdDialogButtonSizerBuilder::new().build();
	button_sizer.add_button(&button);
	button_sizer.realize();
	content_sizer.add_sizer(&button_sizer, 0, SizerFlag::Expand | SizerFlag::All, dialog_padding(&button));
}

/// Binds `ctrl`'s Enter key (`TEXT_ENTER`) to confirm `dialog` as if its OK button were
/// clicked. For single-field entry dialogs where pressing Enter in the control should
/// submit the dialog.
// `ctrl` is taken by value (not consumed) to match how every widget handle in this
// file is passed, since callers always have a cheap Copy handle in hand already.
#[allow(clippy::needless_pass_by_value)]
pub fn bind_enter_confirms<W: WxEvtHandler>(dialog: &Dialog, ctrl: W) {
	let dialog = *dialog;
	ctrl.bind_internal(EventType::TEXT_ENTER, move |event| {
		event.skip(false);
		dialog.end_modal(ID_OK);
	});
}

/// Shows a modal Yes/No confirmation and returns whether the user chose Yes.
///
/// A translated alternative to a `MessageDialog` with `MessageDialogStyle::YesNo`, whose
/// buttons on Windows come from the native task dialog and so are labeled in the *system*
/// language rather than the app's (see [`build_yes_no_buttons`]). Escape and closing the
/// dialog both mean No.
#[must_use]
pub fn confirm(parent: &dyn WxWidget, message: &str, title: &str) -> bool {
	let dialog = Dialog::builder(parent, title).build();
	let padding = dialog_padding(&dialog);
	let label = StaticText::builder(&dialog).with_label(message).build();
	let (yes_button, no_button) = build_yes_no_buttons(&dialog);
	let content = BoxSizer::builder(Orientation::Vertical).build();
	content.add(&label, 0, SizerFlag::All, padding);
	add_yes_no_footer(content, yes_button, no_button);
	dialog.set_sizer_and_fit(content, true);
	dialog.centre();
	dialog.show_modal() == ID_YES
}

/// Shows a modal integer-entry dialog with a label and a bounded spin control.
///
/// Returns `None` if the user cancelled. Enter key submits.
#[must_use]
pub fn prompt_number(
	parent: &dyn WxWidget,
	title: &str,
	label: &str,
	ok_label: &str,
	min: i32,
	max: i32,
	initial: i32,
) -> Option<i32> {
	let max = max.max(min);
	let dialog = Dialog::builder(parent, title).build();
	let lbl = StaticText::builder(&dialog).with_label(label).build();
	let ctrl = SpinCtrl::builder(&dialog)
		.with_range(min, max)
		.with_style(SpinCtrlStyle::Default | SpinCtrlStyle::ProcessEnter)
		.build();
	ctrl.set_value(initial.clamp(min, max));
	bind_enter_confirms(&dialog, ctrl);
	let padding = dialog_padding(&dialog);
	let row = BoxSizer::builder(Orientation::Horizontal).build();
	row.add(&lbl, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, dialog.from_dip_int(5));
	row.add(&ctrl, 1, SizerFlag::Expand, 0);
	let content = BoxSizer::builder(Orientation::Vertical).build();
	content.add_sizer(&row, 0, SizerFlag::Expand | SizerFlag::All, padding);
	let (ok_button, cancel_button) = build_ok_cancel_buttons(&dialog, ok_label);
	add_ok_cancel_footer(content, ok_button, cancel_button);
	dialog.set_sizer_and_fit(content, true);
	dialog.centre();
	ctrl.set_focus();
	if dialog.show_modal() == ID_OK { Some(ctrl.value()) } else { None }
}

/// Shows a modal single-line text entry dialog.
///
/// Returns `None` if the user cancelled or entered only whitespace.
#[must_use]
pub fn prompt_text(parent: &dyn WxWidget, message: &str, title: &str) -> Option<String> {
	let dialog = TextEntryDialog::builder(parent, message, title)
		.with_style(TextEntryDialogStyle::Default | TextEntryDialogStyle::ProcessEnter)
		.build();
	if dialog.show_modal() != ID_OK {
		dialog.destroy();
		return None;
	}
	let value = dialog.get_value().unwrap_or_default();
	dialog.destroy();
	let trimmed = value.trim();
	if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
}
