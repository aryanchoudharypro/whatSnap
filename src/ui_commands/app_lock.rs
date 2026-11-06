//! App lock: Lock App (Ctrl+Shift+L, or at start) covers the whole app with a
//! lock screen that has Unlock and Exit buttons. Unlocking asks for Windows
//! Hello or the app lock password, whichever the user chose in Settings.
//!
//! Like the locked chats code this is a convenience lock: it keeps people
//! who walk past the PC out, it isn't a barrier against code running as the
//! user. Calls still ring while locked, and message previews are left out of
//! notifications.

use std::{
	cell::{Cell, RefCell},
	rc::Rc,
	sync::atomic::{AtomicBool, Ordering},
};

use wxdragon::prelude::*;

use crate::{
	AppState, UiCommand,
	platform::windows_hello::{self, Availability},
	ui::{theme, window::WindowParts},
};

const ID_UNLOCK: i32 = 7101;
const ID_LOCK_EXIT: i32 = 7102;

static LOCKED: AtomicBool = AtomicBool::new(false);

thread_local! {
	/// The lock screen while it is up, so the Windows Hello answer can reach it.
	static LOCK_SCREEN: Cell<Option<LockScreen>> = const { Cell::new(None) };
}

#[derive(Clone, Copy)]
struct LockScreen {
	dialog: Dialog,
	status: StaticText,
	unlock_btn: Button,
}

/// Whether the lock screen is up right now.
pub(crate) fn is_locked() -> bool {
	LOCKED.load(Ordering::SeqCst)
}

/// Locks the app. Does nothing, with a spoken hint, when the lock is off in
/// Settings. Returns once the user unlocked it (or chose Exit, which has by
/// then asked the app to quit).
pub(crate) fn lock_app(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let (enabled, use_hello, ui_tx) = {
		let s = state.borrow();
		if !s.main_ui_shown {
			return;
		}
		(s.settings.app_lock, s.settings.app_lock_hello, s.ui_tx.clone())
	};
	if !enabled {
		crate::platform::announce::announce(
			parts.conversations_parts.messages_announcer,
			"App lock is off. Turn it on in Settings, App Lock.",
		);
		return;
	}
	if LOCKED.swap(true, Ordering::SeqCst) {
		return;
	}
	// The app's own window stays, empty, as the lock screen's owner: Windows
	// won't activate a dialog whose owner is hidden. Everything inside is
	// hidden so nothing can be seen or read, and the app stops marking chats
	// read while locked.
	parts.main_panel.show(false);
	let _ = ui_tx.send(UiCommand::WindowHidden);

	let exit = show_lock_screen(state, parts, use_hello);

	LOCKED.store(false, Ordering::SeqCst);
	if exit {
		let _ = ui_tx.send(UiCommand::ExitApp);
		return;
	}
	parts.main_panel.show(true);
	parts.main_panel.layout();
	crate::platform::tray::restore_main_window(&parts.frame);
	let _ = ui_tx.send(UiCommand::SetPresence(true));
	parts.conversations_parts.conversations_list.set_focus();
}

/// Runs the lock screen until the user unlocks or exits; true means Exit.
fn show_lock_screen(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, use_hello: bool) -> bool {
	let dialog = Dialog::builder(&parts.frame, "whatSnap Locked")
		.with_size(420, 260)
		.with_style(DialogStyle::DefaultDialogStyle)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	// Status first, so nothing sits between the password's label and its field.
	let intro = if use_hello {
		"whatSnap is locked. Choose Unlock to confirm it's you with Windows Hello."
	} else {
		"whatSnap is locked. Enter your password to unlock it."
	};
	let status = StaticText::builder(&panel).with_label(intro).build();
	live_region::set_live_region(&status);
	theme::style_muted_label(&status);
	sizer.add(&status, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);

	let password_field = if use_hello {
		None
	} else {
		let label = StaticText::builder(&panel).with_label("Password").build();
		theme::style_muted_label(&label);
		sizer.add(&label, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);
		let field = TextCtrl::builder(&panel).with_style(TextCtrlStyle::Password).build();
		field.set_font(&theme::Fonts::body());
		sizer.add(&field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);
		Some(field)
	};

	let buttons = BoxSizer::builder(Orientation::Horizontal).build();
	let unlock_btn = Button::builder(&panel).with_id(ID_UNLOCK).with_label("Unlock").build();
	let exit_btn = Button::builder(&panel).with_id(ID_LOCK_EXIT).with_label("Exit").build();
	unlock_btn.set_default();
	buttons.add(&unlock_btn, 0, SizerFlag::Right, 8);
	buttons.add(&exit_btn, 0, SizerFlag::empty(), 0);
	sizer.add_sizer(&buttons, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);
	sizer.add_spacer(12);
	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let screen = LockScreen { dialog, status, unlock_btn };
	LOCK_SCREEN.set(Some(screen));
	let exit_requested = Rc::new(Cell::new(false));

	let state_unlock = state.clone();
	let ui_tx = state.borrow().ui_tx.clone();
	unlock_btn.on_click(move |_| {
		if use_hello {
			if windows_hello::availability() != Availability::Ready {
				say(&screen, "Windows Hello isn't available. Set it up in Windows Settings, or choose Exit.");
				return;
			}
			let tx = ui_tx.clone();
			let started = windows_hello::verify("Unlock whatSnap".to_string(), move |verified| {
				let _ = tx.send(UiCommand::AppLockHello { verified });
			});
			if !started {
				say(&screen, "Windows Hello is already asking. Finish that prompt first.");
			}
		} else if let Some(field) = password_field {
			let stored = state_unlock.borrow().settings.app_lock_password.clone();
			if whatsnap::passhash::verify_password(&stored, &field.get_value()) {
				dialog.end_modal(ID_OK);
			} else {
				field.set_value("");
				field.set_focus();
				say(&screen, "That password is not right. Try again.");
			}
		}
	});
	let exit_flag = exit_requested.clone();
	exit_btn.on_click(move |_| {
		exit_flag.set(true);
		dialog.end_modal(ID_CANCEL);
	});

	match password_field {
		Some(field) => field.set_focus(),
		None => unlock_btn.set_focus(),
	}
	let unlocked = loop {
		let result = dialog.show_modal();
		if result == ID_OK {
			break true;
		}
		// Exit was chosen. Anything else that closed the dialog (Escape, the
		// close box) leaves the app locked, so the screen comes straight back.
		if exit_requested.get() {
			break false;
		}
	};
	LOCK_SCREEN.set(None);
	dialog.destroy();
	!unlocked
}

fn say(screen: &LockScreen, message: &str) {
	screen.status.set_label(message);
	crate::platform::announce::announce(screen.status, message);
}

/// The Windows Hello prompt on the lock screen finished.
pub(crate) fn finish_hello(verified: bool) {
	let Some(screen) = LOCK_SCREEN.get() else { return };
	// The prompt has closed: take the keyboard back from it.
	crate::platform::focus::bring_to_front(windows::Win32::Foundation::HWND(screen.dialog.get_handle()));
	if verified {
		screen.dialog.end_modal(ID_OK);
	} else {
		say(&screen, "Windows Hello didn't confirm it's you. Choose Unlock to try again.");
		screen.unlock_btn.set_focus();
	}
}

/// Asks for a new app lock password twice and returns its hash, or None when
/// cancelled or the two don't match. With a password already set it asks for
/// that first.
pub(crate) fn ask_new_password(parent: &impl WxWidget, current_hash: &str) -> Option<String> {
	let ask = |message: &str, title: &str| -> Option<String> {
		let dlg = TextEntryDialog::builder(parent, message, title).password().build();
		let answer = (dlg.show_modal() == ID_OK).then(|| dlg.get_value()).flatten();
		dlg.destroy();
		answer
	};
	let notice = |text: &str| {
		MessageDialog::builder(parent, text, "App Lock Password")
			.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
			.build()
			.show_modal();
	};
	if !current_hash.is_empty() {
		let old = ask("Current password:", "App Lock Password")?;
		if !whatsnap::passhash::verify_password(current_hash, &old) {
			notice("That is not the current password. The password was not changed.");
			return None;
		}
	}
	let first = ask("New password:", "App Lock Password")?;
	if first.is_empty() {
		notice("The password can't be empty.");
		return None;
	}
	let second = ask("Type the new password again:", "App Lock Password")?;
	if first != second {
		notice("The two passwords don't match. The password was not changed.");
		return None;
	}
	let hash = whatsnap::passhash::hash_password(&first);
	if hash.is_none() {
		notice("Windows couldn't make random numbers for the password. The password was not changed.");
	}
	hash
}
