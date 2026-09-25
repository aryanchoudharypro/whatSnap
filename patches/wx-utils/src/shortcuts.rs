//! A Customize Keyboard Shortcuts dialog for any app that can describe its own keymap.
//!
//! The app keeps its own action type and its own storage; this module owns the window. Implement
//! [`ShortcutModel`] over whatever your config already looks like and hand it to
//! [`prompt_for_shortcuts`], which returns an edited copy when the user confirms.
//!
//! ```no_run
//! # use wx_utils::shortcuts::{KeyChord, ShortcutModel, TabKind, prompt_for_shortcuts};
//! # use wxdragon::prelude::*;
//! # #[derive(Clone)]
//! # struct MyShortcuts;
//! impl ShortcutModel for MyShortcuts {
//!     type Action = u32;
//!     fn tabs(&self) -> Vec<String> { vec!["File".to_string(), "Go".to_string()] }
//!     fn tab_kind(&self) -> TabKind { TabKind::SharedKeymap }
//!     # fn actions(&self, _tab: usize) -> Vec<u32> { vec![] }
//!     # fn action_name(&self, _action: u32) -> String { String::new() }
//!     # fn chord(&self, _tab: usize, _action: u32) -> Option<KeyChord> { None }
//!     # fn set_chord(&mut self, _tab: usize, _action: u32, _chord: Option<KeyChord>) {}
//!     # fn reset_action(&mut self, _tab: usize, _action: u32) {}
//!     # fn reset_all(&mut self, _tab: usize) {}
//!     // ... the rest of the trait
//! }
//!
//! # fn example(parent: &Frame, current: &MyShortcuts) {
//! if let Some(updated) = prompt_for_shortcuts(parent, current) {
//!     // save `updated`
//! }
//! # }
//! ```

use std::{cell::RefCell, rc::Rc};

use patois::t;
use wxdragon::prelude::*;

mod capture;

pub use key_chord::KeyChord;

/// How a model's tabs relate to each other, which is what decides whether two bindings in
/// different tabs can collide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabKind {
	/// The tabs are views onto one keymap, so a chord has to be unique across all of them.
	/// Tabs that group actions by category work this way.
	SharedKeymap,
	/// Each tab is a keymap of its own, so a chord only has to be unique within its tab, and
	/// the same key can mean different things in different tabs. Tabs that are input modes
	/// work this way.
	SeparateKeymaps,
}

/// An app's keymap, as much of it as the dialog needs to see.
///
/// `tab` is an index into [`tabs`](ShortcutModel::tabs) throughout. For [`TabKind::SharedKeymap`]
/// it only says which tab the user is looking at and can be ignored by the lookups; for
/// [`TabKind::SeparateKeymaps`] it selects which keymap to read or write.
pub trait ShortcutModel: Clone {
	/// The app's action identifier. Usually a fieldless enum.
	type Action: Copy + Eq;

	/// Tab labels, in order, already translated. One tab is fine.
	fn tabs(&self) -> Vec<String>;

	/// Whether the tabs share one keymap. See [`TabKind`].
	fn tab_kind(&self) -> TabKind;

	/// The actions listed under `tab`, in the order they should appear.
	fn actions(&self, tab: usize) -> Vec<Self::Action>;

	/// The action's name as the user should read it, already translated.
	fn action_name(&self, action: Self::Action) -> String;

	/// The chord currently bound to `action`, or `None` when it is unbound.
	fn chord(&self, tab: usize, action: Self::Action) -> Option<KeyChord>;

	/// Binds `action` to `chord`, or unbinds it when `chord` is `None`.
	///
	/// Unbinding is not the same as resetting: this has to survive as an explicit "no shortcut"
	/// rather than falling back to the default, which is what
	/// [`reset_action`](ShortcutModel::reset_action) is for.
	fn set_chord(&mut self, tab: usize, action: Self::Action, chord: Option<KeyChord>);

	/// Drops any override on `action`, so it goes back to its default chord.
	fn reset_action(&mut self, tab: usize, action: Self::Action);

	/// Drops every override the user could have reached from `tab`.
	///
	/// For [`TabKind::SharedKeymap`] that is the whole keymap; for [`TabKind::SeparateKeymaps`]
	/// it is only this tab's.
	fn reset_all(&mut self, tab: usize);
}

/// Per-tab list refreshers, called after every edit.
///
/// Every tab is refreshed rather than just the edited one, because reassigning a chord away
/// from another action changes a row the user may be able to see in a different tab.
type Refreshers = Rc<RefCell<Vec<Box<dyn Fn()>>>>;

/// Shows the Customize Keyboard Shortcuts dialog for `initial`.
///
/// Edits are made against a copy, so nothing changes unless the user confirms, in which case the
/// edited model is returned for the caller to save.
#[must_use]
pub fn prompt_for_shortcuts<M: ShortcutModel + 'static>(parent: &dyn WxWidget, initial: &M) -> Option<M> {
	let state = Rc::new(RefCell::new(initial.clone()));
	let refreshers: Refreshers = Rc::new(RefCell::new(Vec::new()));
	// TRANSLATORS: Title of the Keyboard Shortcuts dialog.
	let dialog = Dialog::builder(parent, &t("Customize Keyboard Shortcuts"))
		.with_size(parent.from_dip_int(600), parent.from_dip_int(560))
		.build();
	let padding = crate::dialog_padding(&dialog);
	let notebook = Notebook::builder(&dialog).build();
	for (index, label) in initial.tabs().iter().enumerate() {
		let panel = build_tab(notebook, dialog, &state, index, &refreshers);
		notebook.add_page(&panel, label, index == 0, None);
	}
	// TRANSLATORS: OK button that saves the customized shortcuts in the Keyboard Shortcuts dialog.
	let (ok_button, cancel_button) = crate::build_ok_cancel_buttons(&dialog, &t("OK"));
	let content_sizer = BoxSizer::builder(Orientation::Vertical).build();
	content_sizer.add(&notebook, 1, SizerFlag::Expand | SizerFlag::All, padding);
	crate::add_ok_cancel_footer(content_sizer, ok_button, cancel_button);
	dialog.set_sizer(content_sizer, true);
	dialog.centre();
	if dialog.show_modal() == ID_OK {
		let result = state.borrow().clone();
		Some(result)
	} else {
		None
	}
}

// One linear run of widget construction and event wiring; splitting it would scatter the
// layout across functions without making any of it clearer.
#[allow(clippy::too_many_lines)]
fn build_tab<M: ShortcutModel + 'static>(
	notebook: Notebook,
	parent_dialog: Dialog,
	state: &Rc<RefCell<M>>,
	tab: usize,
	refreshers: &Refreshers,
) -> Panel {
	let panel = Panel::builder(&notebook).with_style(PanelStyle::TabTraversal).build();
	let padding = crate::dialog_padding(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();
	// TRANSLATORS: Label above the list of shortcuts for a tab in the Keyboard Shortcuts dialog.
	let list_label = StaticText::builder(&panel).with_label(&t("&Shortcuts:")).build();
	sizer.add(&list_label, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, padding);
	let list_box = ListBox::builder(&panel).build();
	let actions = state.borrow().actions(tab);
	for &action in &actions {
		list_box.append(&format_list_item(&*state.borrow(), tab, action));
	}
	if !actions.is_empty() {
		list_box.set_selection(0, true);
	}
	sizer.add(&list_box, 1, SizerFlag::Expand | SizerFlag::All, padding);
	let buttons_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	// TRANSLATORS: Button in the Keyboard Shortcuts dialog that opens a prompt to assign a new shortcut to the selected action.
	let set_button = Button::builder(&panel).with_label(&t("&Set Shortcut...")).build();
	// TRANSLATORS: Button in the Keyboard Shortcuts dialog that removes the shortcut assigned to the selected action.
	let clear_button = Button::builder(&panel).with_label(&t("&Clear Shortcut")).build();
	// TRANSLATORS: Button in the Keyboard Shortcuts dialog that restores the selected action's shortcut to its default.
	let reset_button = Button::builder(&panel).with_label(&t("&Reset to Default")).build();
	// TRANSLATORS: Button in the Keyboard Shortcuts dialog that restores every action's shortcut to its default.
	let reset_all_button = Button::builder(&panel).with_label(&t("Reset &All to Defaults")).build();
	buttons_sizer.add(&set_button, 0, SizerFlag::Right, padding);
	buttons_sizer.add(&clear_button, 0, SizerFlag::Right, padding);
	buttons_sizer.add(&reset_button, 0, SizerFlag::Right, padding);
	buttons_sizer.add(&reset_all_button, 0, SizerFlag::Right, padding);
	sizer.add_sizer(&buttons_sizer, 0, SizerFlag::Expand | SizerFlag::All, padding);
	panel.set_sizer(sizer, true);
	// Rewrites each row in place rather than clearing and refilling, so the selection and the
	// screen reader's position in the list survive an edit.
	let refresh_this_tab = {
		let state = state.clone();
		let actions = actions.clone();
		move || {
			for (idx, &action) in actions.iter().enumerate() {
				let item_text = format_list_item(&*state.borrow(), tab, action);
				list_box.set_string(u32::try_from(idx).unwrap_or(u32::MAX), &item_text);
			}
		}
	};
	refreshers.borrow_mut().push(Box::new(refresh_this_tab));
	let refresh_all = {
		let refreshers = refreshers.clone();
		move || {
			for cb in refreshers.borrow().iter() {
				cb();
			}
		}
	};
	let selected_action = {
		move || {
			let sel = list_box.get_selection()?;
			let idx = usize::try_from(sel).ok()?;
			actions.get(idx).copied()
		}
	};
	let trigger_set_shortcut = {
		let state = state.clone();
		let parent = parent_dialog;
		let refresh_all = refresh_all.clone();
		let selected_action = selected_action.clone();
		move || {
			let Some(action) = selected_action() else { return };
			let (name, current_chord) = {
				let model = state.borrow();
				(model.action_name(action), model.chord(tab, action))
			};
			let Some(result) = capture::prompt_for_key_chord(&parent, &name, current_chord.as_ref()) else {
				return;
			};
			if let Some(new_chord) = &result {
				let conflict = find_conflict(&*state.borrow(), tab, action, new_chord);
				if let Some((other_tab, other_action)) = conflict {
					// TRANSLATORS: the three {} are, in order: the key chord, the action it's currently assigned to, and the action being (re)assigned to it
					let msg = t("'{}' is already assigned to '{}'. Reassign it to '{}'?")
						.replacen("{}", &new_chord.to_shortcut_string(), 1)
						.replacen("{}", &state.borrow().action_name(other_action), 1)
						.replacen("{}", &name, 1);
					// TRANSLATORS: Title of the dialog warning that a shortcut is already taken
					if !crate::confirm(&parent, &msg, &t("Shortcut Conflict")) {
						return;
					}
					state.borrow_mut().set_chord(other_tab, other_action, None);
				}
			}
			state.borrow_mut().set_chord(tab, action, result);
			refresh_all();
		}
	};
	let set_on_click = trigger_set_shortcut.clone();
	set_button.on_click(move |_| {
		set_on_click();
	});
	let set_on_dclick = trigger_set_shortcut;
	list_box.on_item_double_clicked(move |_| {
		set_on_dclick();
	});
	let state_on_clear = state.clone();
	let refresh_on_clear = refresh_all.clone();
	let selected_on_clear = selected_action.clone();
	clear_button.on_click(move |_| {
		let Some(action) = selected_on_clear() else { return };
		state_on_clear.borrow_mut().set_chord(tab, action, None);
		refresh_on_clear();
	});
	let state_on_reset = state.clone();
	let refresh_on_reset = refresh_all.clone();
	let selected_on_reset = selected_action;
	reset_button.on_click(move |_| {
		let Some(action) = selected_on_reset() else { return };
		state_on_reset.borrow_mut().reset_action(tab, action);
		refresh_on_reset();
	});
	let state_on_reset_all = state.clone();
	let refresh_on_reset_all = refresh_all;
	let parent_reset_all = parent_dialog;
	reset_all_button.on_click(move |_| {
		let message = match state_on_reset_all.borrow().tab_kind() {
			// TRANSLATORS: Confirmation prompt shown when resetting all keyboard shortcuts to their defaults.
			TabKind::SharedKeymap => t("Reset all shortcuts to their default values?"),
			// TRANSLATORS: Confirmation prompt shown when resetting the current tab's keyboard shortcuts to their defaults.
			TabKind::SeparateKeymaps => t("Reset all shortcuts in this tab to their default values?"),
		};
		// TRANSLATORS: Title of the confirmation dialog for resetting keyboard shortcuts to their defaults.
		if crate::confirm(&parent_reset_all, &message, &t("Reset Shortcuts")) {
			state_on_reset_all.borrow_mut().reset_all(tab);
			refresh_on_reset_all();
		}
	});
	panel
}

fn format_list_item<M: ShortcutModel>(model: &M, tab: usize, action: M::Action) -> String {
	// TRANSLATORS: Shown in the Customize Keyboard Shortcuts list in place of a key combination when an action has no shortcut assigned
	let chord_str = model.chord(tab, action).map_or_else(|| t("None"), |c| c.to_shortcut_string());
	format!("{}: {}", model.action_name(action), chord_str)
}

/// The action already holding `target_chord`, if any, and the tab it lives in.
///
/// How far this looks depends on [`ShortcutModel::tab_kind`]: across every tab when they share
/// one keymap, and only within `tab` when each tab is its own.
fn find_conflict<M: ShortcutModel>(
	model: &M,
	tab: usize,
	target_action: M::Action,
	target_chord: &KeyChord,
) -> Option<(usize, M::Action)> {
	let tabs: Vec<usize> = match model.tab_kind() {
		TabKind::SharedKeymap => (0..model.tabs().len()).collect(),
		TabKind::SeparateKeymaps => vec![tab],
	};
	for candidate_tab in tabs {
		for action in model.actions(candidate_tab) {
			if action == target_action {
				continue;
			}
			if model.chord(candidate_tab, action).is_some_and(|c| c.conflicts_with(target_chord)) {
				return Some((candidate_tab, action));
			}
		}
	}
	None
}
