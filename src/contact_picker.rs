//! Contact/group-member picker helpers: formatting a contact's display
//! label, filtering+repainting the New Group / Add Members / new-chat
//! contact-picker listboxes, and the in-chat multi-select message list.

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{AppState, Contact, ui::window::WindowParts};

/// Falls back to a formatted phone number (e.g. "+91 96671 74783") instead
/// of the raw JID when a chat has no saved contact name yet — most visibly
/// hit right after `OpenChatByPhone` jumps to a number that isn't a contact.
pub(crate) fn display_name_for_jid(jid: &str) -> String {
	let raw_phone = jid.split('@').next().unwrap_or(jid);
	if !raw_phone.is_empty() && raw_phone.chars().all(|c| c.is_ascii_digit()) {
		if let Ok(parsed) = phonenumber::parse(None, format!("+{}", raw_phone)) {
			return parsed.format().mode(phonenumber::Mode::International).to_string();
		}
		return format!("+{}", raw_phone);
	}
	jid.to_string()
}

/// Formats a raw digits-only phone number (e.g. "919800000003") as
/// "+91 96671 74783", falling back to a plain "+"-prefixed string if it
/// doesn't parse as a phone number at all.
pub(crate) fn format_phone_digits(raw_phone: &str) -> String {
	if let Ok(parsed) = phonenumber::parse(None, format!("+{}", raw_phone)) {
		return parsed.format().mode(phonenumber::Mode::International).to_string();
	}
	format!("+{}", raw_phone)
}

/// Builds the "Name +91 96671 74783" (or just the number, if unnamed) label
/// shown for a contact in the shared contact-picker dialog.
pub(crate) fn contact_picker_label(contact: &Contact) -> String {
	// A "@lid" jid's user part is WhatsApp's own large synthetic identifier,
	// not a phone number - it just happens to also be all digits, so without
	// this check an unresolved lid contact (no known phone) would get that
	// identifier formatted as if it were one, e.g. "Name +1 10000000000004".
	let is_lid = contact.jid.ends_with("@lid");
	let raw_phone = if !contact.phone.is_empty() {
		Some(contact.phone.as_str())
	} else if !is_lid {
		Some(contact.jid.split('@').next().unwrap_or(&contact.jid))
	} else {
		None
	};
	let formatted_phone =
		raw_phone.filter(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())).map(format_phone_digits);
	match (contact.name.trim().is_empty(), formatted_phone) {
		(false, Some(phone)) => format!("{} {}", contact.name.trim(), phone),
		(false, None) => contact.name.trim().to_string(),
		(true, Some(phone)) => phone,
		(true, None) => contact.jid.clone(),
	}
}

/// Rebuilds `messages_select_list` from the current `display_items` while
/// selection mode is active - a no-op otherwise. Called everywhere
/// `messages_list` itself gets a structural rebuild (new message, delete,
/// history load) so the selection list never shows a stale row set while the
/// user is mid-selection. Previously-checked message IDs that no longer
/// exist in the rebuilt rows (e.g. deleted from another device) are dropped
/// rather than left dangling in `selected_message_ids`.
pub(crate) fn refresh_selection_list(s: &mut AppState, parts: &WindowParts) {
	if !s.selection_mode {
		return;
	}
	let is_group = s.selected_chat.as_deref().map(|j| j.ends_with("@g.us")).unwrap_or(false);
	s.selection_rows = s
		.display_items
		.iter()
		.filter(|item| match item {
			whantv::DisplayItem::Single(m) => !m.is_date_divider && !m.is_unread_divider,
			_ => true,
		})
		.cloned()
		.collect();
	let valid_ids: std::collections::HashSet<String> = s.selection_rows.iter().map(|i| i.id().to_string()).collect();
	s.selected_message_ids.retain(|id| valid_ids.contains(id));

	parts.conversations_parts.messages_select_list.freeze();
	parts.conversations_parts.messages_select_list.clear();
	for (idx, item) in s.selection_rows.iter().enumerate() {
		parts.conversations_parts.messages_select_list.append(&item.format_display(is_group));
		if s.selected_message_ids.contains(item.id()) {
			parts.conversations_parts.messages_select_list.check(idx as u32, true);
		}
	}
	parts.conversations_parts.messages_select_list.thaw();
	parts.conversations_parts.selection_count_label.set_label(&format!("{} selected", s.selected_message_ids.len()));
}

/// Sets the group data dialog's `make_admin_btn` label to match whether the
/// currently selected member (by position in the filtered `members_list`) is
/// already an admin, mirroring official WhatsApp's per-member "Make admin" /
/// "Dismiss as admin" toggle instead of one static label regardless of
/// who's selected.
pub(crate) fn update_make_admin_button_label(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let s = state.borrow();
	let is_admin = parts
		.group_data_dialog
		.members_list
		.get_selection()
		.and_then(|idx| s.group_data_visible_member_jids.get(idx as usize))
		.and_then(|jid| s.group_data_member_jids.iter().position(|j| j == jid))
		.and_then(|i| s.group_data_member_admin.get(i).copied())
		.unwrap_or(false);
	drop(s);
	parts.group_data_dialog.make_admin_btn.set_label(if is_admin { "Dismiss as admin" } else { "Make admin" });
}

/// Re-filters `s.all_contacts` by `query` and repaints the New Group dialog's
/// `contacts_listbox` (a wxCheckListBox) with the matches, preserving which
/// jids are already checked. Shared by opening the dialog and by a fresh
/// `contacts` response landing while it's already open - without the latter,
/// the box stays empty for the entire first `get_contacts` round trip, and
/// tabbing focus into an empty wxCheckListBox is what trips the "bad
/// wxCheckListBox index" debug assert in NVDA's accessibility queries.
pub(crate) fn refresh_new_group_contacts_list(query: &str, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let q = query.trim().to_lowercase();
	let mut s = state.borrow_mut();
	let mut all_labels = Vec::new();
	let mut all_jids = Vec::new();
	for contact in &s.all_contacts {
		all_labels.push(contact_picker_label(contact));
		all_jids.push(contact.jid.clone());
	}
	let mut filtered_labels = Vec::new();
	let mut filtered_jids = Vec::new();
	for (lbl, jid) in all_labels.iter().zip(all_jids.iter()) {
		if q.is_empty() || lbl.to_lowercase().contains(&q) {
			filtered_labels.push(lbl.clone());
			filtered_jids.push(jid.clone());
		}
	}
	parts.new_group_dialog.contacts_listbox.clear();
	for lbl in &filtered_labels {
		parts.new_group_dialog.contacts_listbox.append(lbl);
	}
	let gs = &mut s.group_state;
	gs.all_contact_labels = all_labels;
	gs.all_contact_jids = all_jids;
	gs.current_jids = filtered_jids;
	for (idx, jid) in gs.current_jids.iter().enumerate() {
		if gs.checked_jids.contains(jid) {
			parts.new_group_dialog.contacts_listbox.check(idx as u32, true);
		}
	}
	// See the comment on the equivalent reset in window.rs's search handler:
	// clear()+append() leaves the native listbox caret wherever it was
	// before the rebuild, which NVDA can query past the end of a shorter
	// list and trip wx's "bad wxCheckListBox index" assert.
	// wxListBox::SetSelection asserts/crashes if given index 0 on an empty
	// list (only wxNOT_FOUND, which wxdragon's u32 API can't express, is
	// valid there) - so there's nothing to reset when the filter has no
	// matches, only when it has at least one.
	if !gs.current_jids.is_empty() {
		parts.new_group_dialog.contacts_listbox.set_selection(0, true);
	}
}

/// Recomputes `group_state.all_contact_labels`/`all_contact_jids` from
/// `s.all_contacts` without touching the New Group dialog's listbox widget.
/// Used when a `contacts` response lands while the dialog is open, so the
/// debounced rebuild it then schedules (via `new_group_contacts_timer`) has
/// fresh data to filter once it actually fires.
pub(crate) fn sync_new_group_contact_labels(state: &Rc<RefCell<AppState>>) {
	let mut s = state.borrow_mut();
	let mut all_labels = Vec::new();
	let mut all_jids = Vec::new();
	for contact in &s.all_contacts {
		all_labels.push(contact_picker_label(contact));
		all_jids.push(contact.jid.clone());
	}
	let gs = &mut s.group_state;
	gs.all_contact_labels = all_labels;
	gs.all_contact_jids = all_jids;
}

/// Same as `refresh_new_group_contacts_list` above, but for the Add Members
/// dialog's `contacts_listbox`, and additionally excluding anyone already in
/// the group (`group_data_member_jids`) - there's no reason to offer
/// re-adding an existing member.
pub(crate) fn refresh_add_members_contacts_list(query: &str, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let q = query.trim().to_lowercase();
	let mut s = state.borrow_mut();
	let existing_members: std::collections::HashSet<String> = s.group_data_member_jids.iter().cloned().collect();
	let mut all_labels = Vec::new();
	let mut all_jids = Vec::new();
	for contact in &s.all_contacts {
		if existing_members.contains(&contact.jid) {
			continue;
		}
		all_labels.push(contact_picker_label(contact));
		all_jids.push(contact.jid.clone());
	}
	let mut filtered_labels = Vec::new();
	let mut filtered_jids = Vec::new();
	for (lbl, jid) in all_labels.iter().zip(all_jids.iter()) {
		if q.is_empty() || lbl.to_lowercase().contains(&q) {
			filtered_labels.push(lbl.clone());
			filtered_jids.push(jid.clone());
		}
	}
	parts.add_members_dialog.contacts_listbox.clear();
	for lbl in &filtered_labels {
		parts.add_members_dialog.contacts_listbox.append(lbl);
	}
	let ams = &mut s.add_members_state;
	ams.all_contact_labels = all_labels;
	ams.all_contact_jids = all_jids;
	ams.current_jids = filtered_jids;
	for (idx, jid) in ams.current_jids.iter().enumerate() {
		if ams.checked_jids.contains(jid) {
			parts.add_members_dialog.contacts_listbox.check(idx as u32, true);
		}
	}
	// See the equivalent reset in refresh_new_group_contacts_list/window.rs:
	// clear()+append() leaves the native listbox caret wherever it was
	// before the rebuild, which NVDA can query past the end of a shorter
	// list and trip wx's "bad wxCheckListBox index" assert. SetSelection
	// itself asserts/crashes given index 0 on an empty list (only
	// wxNOT_FOUND, which wxdragon's u32 API can't express, is valid there).
	if !ams.current_jids.is_empty() {
		parts.add_members_dialog.contacts_listbox.set_selection(0, true);
	}
}

/// Recomputes `add_members_state.all_contact_labels`/`all_contact_jids` from
/// `s.all_contacts` (excluding existing group members) without touching the
/// Add Members dialog's listbox widget. Used when a `contacts` response
/// lands while the dialog is open, so the debounced rebuild it then
/// schedules (via `add_members_contacts_timer`) has fresh data to filter
/// once it actually fires.
pub(crate) fn sync_add_members_contact_labels(state: &Rc<RefCell<AppState>>) {
	let mut s = state.borrow_mut();
	let existing_members: std::collections::HashSet<String> = s.group_data_member_jids.iter().cloned().collect();
	let mut all_labels = Vec::new();
	let mut all_jids = Vec::new();
	for contact in &s.all_contacts {
		if existing_members.contains(&contact.jid) {
			continue;
		}
		all_labels.push(contact_picker_label(contact));
		all_jids.push(contact.jid.clone());
	}
	let ams = &mut s.add_members_state;
	ams.all_contact_labels = all_labels;
	ams.all_contact_jids = all_jids;
}

/// Same as `refresh_new_group_contacts_list` above, but for the Attach
/// Contact dialog's `contacts_listbox` - no membership exclusion, every
/// contact is a valid pick.
pub(crate) fn refresh_attach_contact_list(query: &str, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let q = query.trim().to_lowercase();
	let mut s = state.borrow_mut();
	let mut all_labels = Vec::new();
	let mut all_jids = Vec::new();
	for contact in &s.all_contacts {
		all_labels.push(contact_picker_label(contact));
		all_jids.push(contact.jid.clone());
	}
	let mut filtered_labels = Vec::new();
	let mut filtered_jids = Vec::new();
	for (lbl, jid) in all_labels.iter().zip(all_jids.iter()) {
		if q.is_empty() || lbl.to_lowercase().contains(&q) {
			filtered_labels.push(lbl.clone());
			filtered_jids.push(jid.clone());
		}
	}
	parts.attach_contact_dialog.contacts_listbox.clear();
	for lbl in &filtered_labels {
		parts.attach_contact_dialog.contacts_listbox.append(lbl);
	}
	let acs = &mut s.attach_contact_state;
	acs.all_contact_labels = all_labels;
	acs.all_contact_jids = all_jids;
	acs.current_jids = filtered_jids;
	for (idx, jid) in acs.current_jids.iter().enumerate() {
		if acs.checked_jids.contains(jid) {
			parts.attach_contact_dialog.contacts_listbox.check(idx as u32, true);
		}
	}
	if !acs.current_jids.is_empty() {
		parts.attach_contact_dialog.contacts_listbox.set_selection(0, true);
	}
}

/// Recomputes `attach_contact_state.all_contact_labels`/`all_contact_jids`
/// from `s.all_contacts` without touching the dialog's listbox widget. Used
/// when a `contacts` response lands while the dialog is open - see the
/// equivalent `sync_add_members_contact_labels`.
pub(crate) fn sync_attach_contact_labels(state: &Rc<RefCell<AppState>>) {
	let mut s = state.borrow_mut();
	let mut all_labels = Vec::new();
	let mut all_jids = Vec::new();
	for contact in &s.all_contacts {
		all_labels.push(contact_picker_label(contact));
		all_jids.push(contact.jid.clone());
	}
	let acs = &mut s.attach_contact_state;
	acs.all_contact_labels = all_labels;
	acs.all_contact_jids = all_jids;
}

/// Re-filters `s.all_contacts` by `query` (against the same "Name +phone"
/// text shown to the user) and repaints `new_conv_dialog.results_list` with
/// the matches, recording their jids in `contact_picker_current_jids` so a
/// selected row index can be resolved back to a jid. Shared by opening the
/// dialog (empty query - the whole contact list), typing in its search
/// field, and a fresh `contacts` response landing while it's already open.
pub(crate) fn apply_contact_picker_filter(query: &str, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let q = query.trim().to_lowercase();
	let mut s = state.borrow_mut();
	let mut labels = Vec::new();
	let mut jids = Vec::new();
	for contact in &s.all_contacts {
		let label = contact_picker_label(contact);
		if q.is_empty() || label.to_lowercase().contains(&q) {
			labels.push(label);
			jids.push(contact.jid.clone());
		}
	}
	parts.new_conv_dialog.results_list.clear();
	for label in &labels {
		parts.new_conv_dialog.results_list.append(label);
	}
	s.contact_picker_current_jids = jids;
}
