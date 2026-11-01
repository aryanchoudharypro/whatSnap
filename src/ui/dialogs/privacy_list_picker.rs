//! "My contacts except...": pick the contacts a privacy setting leaves out,
//! as WhatsApp Web's privacy settings do for last seen, profile photo, About
//! and who can add you to groups.

use std::{cell::RefCell, collections::HashSet, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	Contact,
	ui::{contact_picker::contact_picker_label, theme},
};

struct PickerState {
	/// (jid, label) for every contact offered.
	all: Vec<(String, String)>,
	/// JIDs of the rows currently shown, in list order.
	shown: Vec<String>,
	checked: HashSet<String>,
}

/// The contacts offered and which of them start checked: every contact,
/// plus anyone on the server's list who isn't one (they stay on it unless
/// unchecked). Someone on the list is matched to a contact by JID, or by
/// phone number, since the list and the contacts can know the same person by
/// different addresses.
fn picker_rows(contacts: &[Contact], excluded: &[Contact]) -> (Vec<(String, String)>, HashSet<String>) {
	let mut rows: Vec<(String, String)> = Vec::new();
	let mut checked = HashSet::new();
	for person in excluded {
		let found = contacts
			.iter()
			.find(|c| c.jid == person.jid || (!person.phone.is_empty() && c.phone == person.phone))
			.map(|c| c.jid.clone());
		match found {
			Some(jid) => {
				checked.insert(jid);
			}
			None => {
				checked.insert(person.jid.clone());
				rows.push((person.jid.clone(), contact_picker_label(person)));
			}
		}
	}
	rows.extend(contacts.iter().map(|c| (c.jid.clone(), contact_picker_label(c))));
	(rows, checked)
}

/// Shows the picker modally and returns the JIDs chosen, or None if
/// cancelled. `excluded` is who the setting leaves out now.
pub fn pick_excluded_contacts(
	parent: &impl WxWidget,
	setting: &str,
	contacts: &[Contact],
	excluded: &[Contact],
) -> Option<Vec<String>> {
	pick_contacts(
		parent,
		&format!("{}: My Contacts Except...", setting),
		"Checked contacts won't be able to see it",
		contacts,
		excluded,
	)
}

/// What the picker calls its rows and a checked one, and what it says when
/// Done is pressed with none checked (if that isn't allowed).
pub struct PickerWords<'a> {
	pub search: &'a str,
	pub on: &'a str,
	pub off: &'a str,
	pub one: &'a str,
	pub many: &'a str,
	pub require_one: Option<&'a str>,
}

const EXCLUDED: PickerWords<'static> = PickerWords {
	search: "Search contacts",
	on: "excluded",
	off: "not excluded",
	one: "contact",
	many: "contacts",
	// WhatsApp's own picker won't save "except" with nobody excluded.
	require_one: Some("Choose at least one contact to exclude."),
};

/// The same picker with its own title and hint, `listed` starting checked.
pub fn pick_contacts(
	parent: &impl WxWidget,
	title: &str,
	hint: &str,
	contacts: &[Contact],
	listed: &[Contact],
) -> Option<Vec<String>> {
	pick_with(parent, title, hint, contacts, listed, &EXCLUDED)
}

/// The picker with its own words, for rows that needn't be contacts.
pub fn pick_with(
	parent: &impl WxWidget,
	title: &str,
	hint: &str,
	contacts: &[Contact],
	listed: &[Contact],
	words: &PickerWords,
) -> Option<Vec<String>> {
	let excluded = listed;
	let dialog = Dialog::builder(parent, title)
		.with_size(440, 560)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let search_label = StaticText::builder(&panel).with_label(words.search).build();
	theme::style_muted_label(&search_label);
	sizer.add(&search_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let search_field = TextCtrl::builder(&panel).build();
	search_field.set_font(&theme::Fonts::body());
	sizer.add(&search_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let list_label = StaticText::builder(&panel).with_label(hint).build();
	theme::style_muted_label(&list_label);
	sizer.add(&list_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let list = CheckListBox::builder(&panel).build();
	list.set_font(&theme::Fonts::body());
	sizer.add(&list, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let announcer = StaticText::builder(&panel).with_label("").build();
	live_region::set_live_region(&announcer);
	theme::style_muted_label(&announcer);
	sizer.add(&announcer, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let done_btn = Button::builder(&panel).with_id(ID_OK).with_label("Done").build();
	theme::style_primary_button(&done_btn);
	let cancel_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Cancel").build();
	btn_sizer.add(&done_btn, 0, SizerFlag::Right, 8);
	btn_sizer.add_stretch_spacer(1);
	btn_sizer.add(&cancel_btn, 0, SizerFlag::Right, 8);
	sizer.add_sizer(&btn_sizer, 0, SizerFlag::Expand | SizerFlag::All, 10);
	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let (all, checked) = picker_rows(contacts, excluded);
	let state = Rc::new(RefCell::new(PickerState { shown: Vec::new(), all, checked }));
	let fill = {
		let state = state.clone();
		move |query: &str| {
			let mut s = state.borrow_mut();
			let shown: Vec<(String, String)> = s
				.all
				.iter()
				.filter(|(_, label)| query.is_empty() || label.to_lowercase().contains(query))
				.cloned()
				.collect();
			list.clear();
			for (_, label) in &shown {
				list.append(label);
			}
			for (idx, (jid, _)) in shown.iter().enumerate() {
				if s.checked.contains(jid) {
					list.check(idx as u32, true);
				}
			}
			// SetSelection(0) on an empty list asserts.
			if !shown.is_empty() {
				list.set_selection(0, true);
			}
			s.shown = shown.into_iter().map(|(jid, _)| jid).collect();
			s.shown.len()
		}
	};
	fill("");
	let counts = (words.on.to_string(), words.one.to_string(), words.many.to_string());
	crate::platform::announce::announce(announcer, &counted(&counts, state.borrow().checked.len()));

	// Rebuilt only once typing pauses, as in the other contact pickers:
	// rebuilding a check list per keystroke leaves NVDA querying rows that
	// no longer exist.
	let search_timer = Rc::new(Timer::new(&dialog));
	let state_search = state.clone();
	let search_words = counts.clone();
	search_timer.on_tick(move |_| {
		let query = search_field.get_value().to_lowercase().trim().to_string();
		let shown = fill(&query);
		let count = state_search.borrow().checked.len();
		crate::platform::announce::announce(announcer, &format!("{} shown. {}", shown, counted(&search_words, count)));
	});
	let timer_for_typing = search_timer.clone();
	search_field.on_text_changed(move |_| {
		timer_for_typing.stop();
		timer_for_typing.start(250, true);
	});

	let state_toggle = state.clone();
	let toggle_words = counts;
	let off = words.off.to_string();
	list.on_toggled(move |event| {
		let Some(idx) = event.get_selection() else { return };
		let mut s = state_toggle.borrow_mut();
		let Some(jid) = s.shown.get(idx as usize).cloned() else { return };
		let label = list.get_string(idx as usize).unwrap_or_default();
		let now = if list.is_checked(idx) {
			s.checked.insert(jid);
			toggle_words.0.as_str()
		} else {
			s.checked.remove(&jid);
			off.as_str()
		};
		crate::platform::announce::announce(
			announcer,
			&format!("{}: {}. {}", label, now, counted(&toggle_words, s.checked.len())),
		);
	});

	let escape_closes = crate::ui::keys::escape_cancels(dialog);
	search_field.on_key_down(escape_closes);
	list.on_key_down(escape_closes);

	let state_done = state.clone();
	let require_one = words.require_one.map(str::to_string);
	done_btn.on_click(move |_| {
		if let Some(message) = &require_one
			&& state_done.borrow().checked.is_empty()
		{
			crate::platform::announce::announce(announcer, message);
			return;
		}
		dialog.end_modal(ID_OK);
	});
	cancel_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));

	search_field.set_focus();
	let result = dialog.show_modal();
	let chosen = (result == ID_OK).then(|| {
		let s = state.borrow();
		// In list order, so the choice reads back predictably.
		s.all.iter().filter(|(jid, _)| s.checked.contains(jid)).map(|(jid, _)| jid.clone()).collect()
	});
	search_timer.stop();
	// A dialog built per use must be destroyed, or it and every closure bound
	// to it live on hidden.
	dialog.destroy();
	chosen
}

/// "No contacts excluded.", "1 contact excluded.", "3 contacts excluded."
fn counted((on, one, many): &(String, String, String), count: usize) -> String {
	match count {
		0 => format!("No {} {}.", many, on),
		1 => format!("1 {} {}.", one, on),
		n => format!("{} {} {}.", n, many, on),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn contact(jid: &str, name: &str, phone: &str) -> Contact {
		Contact { jid: jid.to_string(), name: name.to_string(), phone: phone.to_string() }
	}

	#[test]
	fn excluded_people_match_contacts_by_jid_or_phone() {
		let contacts = vec![
			contact("911@s.whatsapp.net", "Asha", "911"),
			contact("5@lid", "Ravi", "915"),
			contact("916@s.whatsapp.net", "Meena", "916"),
		];
		let excluded = vec![
			contact("911@s.whatsapp.net", "", "911"),
			contact("915@s.whatsapp.net", "", "915"),
			contact("9@lid", "", ""),
		];
		let (rows, checked) = picker_rows(&contacts, &excluded);
		// The stranger is offered too, ahead of the contacts.
		assert_eq!(rows.len(), 4);
		assert_eq!(rows[0].0, "9@lid");
		let want: HashSet<String> = ["911@s.whatsapp.net", "5@lid", "9@lid"].iter().map(|s| s.to_string()).collect();
		assert_eq!(checked, want);
	}
}
