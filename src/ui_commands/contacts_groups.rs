//! `handle_ui_command` arms for contacts, groups and the shared contact picker
//! (new contact, group creation/settings/membership, contact picker filtering).

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState, ContactPickerMode,
	chat_ui::{clear_pending_reply, open_chat_for_jid},
	contact_picker::{
		apply_contact_picker_filter, refresh_add_members_contacts_list, refresh_new_group_contacts_list,
		update_make_admin_button_label,
	},
	message_display::apply_chat_filter,
	tray::refresh_tray_tooltip,
	ui::window::WindowParts,
};

pub(crate) fn prepare_new_group(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	parts.new_group_dialog.ok_btn.enable(true);
	parts.new_group_dialog.name_field.set_value("");
	parts.new_group_dialog.search_field.set_value("");
	parts.new_group_dialog.extra_number_field.set_value("");
	state.borrow_mut().group_state.checked_jids.clear();
	refresh_new_group_contacts_list("", state, parts);
	let payload = serde_json::json!({ "action": "get_contacts" });
	let _ = state.borrow().ws_tx.send(payload.to_string());
	parts.new_group_dialog.dialog.show_modal();
}

pub(crate) fn prepare_add_members(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let group_jid = state.borrow().selected_chat.clone();
	if group_jid.is_none() {
		return;
	}
	parts.add_members_dialog.add_btn.enable(true);
	parts.add_members_dialog.search_field.set_value("");
	parts.add_members_dialog.selection_announcer.set_label("");
	{
		let mut s = state.borrow_mut();
		s.add_members_state.group_jid = group_jid;
		s.add_members_state.checked_jids.clear();
	}
	refresh_add_members_contacts_list("", state, parts);
	let payload = serde_json::json!({ "action": "get_contacts" });
	let _ = state.borrow().ws_tx.send(payload.to_string());
	parts.add_members_dialog.dialog.show_modal();
}

pub(crate) fn add_group_members(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>, participants: Vec<String>) {
	let group_jid = state.borrow().add_members_state.group_jid.clone();
	if let Some(group_jid) = group_jid {
		if !participants.is_empty() {
			let payload = serde_json::json!({
				"action": "update_group_participants",
				"data": { "jid": group_jid, "participants": participants, "action": "add" }
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
	}
}

pub(crate) fn create_group(
	state: &Rc<RefCell<AppState>>,
	_parts: &Rc<WindowParts>,
	name: String,
	participants: Vec<String>,
) {
	let payload = serde_json::json!({
		"action": "create_group",
		"data": {
			"name": name,
			"participants": participants
		}
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

pub(crate) fn new_contact(
	state: &Rc<RefCell<AppState>>,
	_parts: &Rc<WindowParts>,
	name: String,
	phone: String,
	sync: bool,
) {
	let payload = serde_json::json!({
		"action": "new_contact",
		"data": {
			"phone": phone,
			"name": name,
			"sync": sync
		}
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

pub(crate) fn filter_group_members(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, query: String) {
	let mut s = state.borrow_mut();
	let lower_query = query.to_lowercase();
	parts.group_data_dialog.members_list.clear();
	let mut visible_jids = Vec::new();
	for (i, member) in s.group_data_members.iter().enumerate() {
		if lower_query.is_empty() || member.to_lowercase().contains(&lower_query) {
			parts.group_data_dialog.members_list.append(member);
			visible_jids.push(s.group_data_member_jids[i].clone());
		}
	}
	s.group_data_visible_member_jids = visible_jids;
	drop(s);
	// Filtering rebuilds the list from scratch, so any prior selection
	// is gone - reset the label to the no-selection default.
	parts.group_data_dialog.make_admin_btn.set_label("Make admin");
}

pub(crate) fn group_member_selection_changed(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	update_make_admin_button_label(state, parts);
}

pub(crate) fn prepare_contact_picker(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, mode: ContactPickerMode) {
	if mode == ContactPickerMode::AddCallParticipant && state.borrow().active_call_id.is_none() {
		crate::announce::announce(parts.conversations_parts.messages_announcer.clone(), "No active call.");
		return;
	}
	state.borrow_mut().contact_picker_mode = mode;
	parts.new_conv_dialog.search_field.set_value("");
	let heading = match mode {
		ContactPickerMode::OpenChat => "Search name or number",
		ContactPickerMode::AddCallParticipant => "Search a contact to add to the call, then press Enter",
	};
	parts.new_conv_dialog.search_label.set_label(heading);
	// "New Group" and "New Contact" only make sense when this dialog
	// is opened to start a chat - shown regardless of mode, they
	// caused exactly the confusion reported when adding a call
	// participant: press Enter on a search match to add someone
	// instead, which is the only action that applies here.
	let is_open_chat = mode == ContactPickerMode::OpenChat;
	parts.new_conv_dialog.new_group_btn.show(is_open_chat);
	parts.new_conv_dialog.new_contact_btn.show(is_open_chat);
	parts.new_conv_dialog.message_meta_ai_btn.show(is_open_chat && state.borrow().settings.meta_ai_enabled);
	parts.new_conv_dialog.dialog.layout();
	apply_contact_picker_filter("", state, parts);
	let payload = serde_json::json!({ "action": "get_contacts" });
	let _ = state.borrow().ws_tx.send(payload.to_string());
	parts.new_conv_dialog.dialog.show_modal();
}

pub(crate) fn filter_contact_picker(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, query: String) {
	apply_contact_picker_filter(&query, state, parts);
}

pub(crate) fn select_contact_picker_result(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, idx: usize) {
	let (jid_opt, mode) = {
		let s = state.borrow();
		(s.contact_picker_current_jids.get(idx).cloned(), s.contact_picker_mode)
	};
	let Some(jid) = jid_opt else {
		return;
	};
	parts.new_conv_dialog.dialog.end_modal(wxdragon::id::ID_OK);
	match mode {
		ContactPickerMode::OpenChat => {
			open_chat_for_jid(&jid, state, parts);
		}
		ContactPickerMode::AddCallParticipant => {
			let s = state.borrow();
			if let Some(call_id) = &s.active_call_id {
				let msg = serde_json::json!({
					"action": "add_participant",
					"data": { "call_id": call_id, "jid": jid }
				});
				let _ = s.ws_tx.send(msg.to_string());
				drop(s);
				// The outcome arrives as call_participant_added /
				// call_participant_add_failed once WhatsApp has answered.
				crate::announce::announce(parts.conversations_parts.messages_announcer.clone(), "Adding to call...");
			}
		}
	}
}

pub(crate) fn update_group_settings(
	state: &Rc<RefCell<AppState>>,
	parts: &Rc<WindowParts>,
	name: String,
	topic: String,
	announce: bool,
	locked: bool,
) {
	let jid = state.borrow().selected_chat.clone();
	if let Some(jid) = jid {
		let s = state.borrow();

		let msg_name = serde_json::json!({
			"action": "set_group_name",
			"data": { "jid": jid, "name": name }
		});
		let _ = s.ws_tx.send(msg_name.to_string());

		let msg_topic = serde_json::json!({
			"action": "set_group_topic",
			"data": { "jid": jid, "topic": topic }
		});
		let _ = s.ws_tx.send(msg_topic.to_string());

		let msg_ann = serde_json::json!({
			"action": "set_group_announce",
			"data": { "jid": jid, "value": announce }
		});
		let _ = s.ws_tx.send(msg_ann.to_string());

		let msg_lock = serde_json::json!({
			"action": "set_group_locked",
			"data": { "jid": jid, "value": locked }
		});
		let _ = s.ws_tx.send(msg_lock.to_string());

		crate::announce::announce(parts.conversations_parts.messages_announcer.clone(), "Group settings updated.");
	}
}

pub(crate) fn toggle_group_member_admin(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>, idx: u32) {
	let s = state.borrow();
	if let Some(jid) = s.selected_chat.clone() {
		if let Some(member_jid) = s.group_data_visible_member_jids.get(idx as usize).cloned() {
			let is_admin = s
				.group_data_member_jids
				.iter()
				.position(|j| j == &member_jid)
				.and_then(|i| s.group_data_member_admin.get(i).copied())
				.unwrap_or(false);
			let action = if is_admin { "demote" } else { "promote" };
			let msg = serde_json::json!({
				"action": "update_group_participants",
				"data": { "jid": jid, "participants": [member_jid], "action": action }
			});
			let _ = s.ws_tx.send(msg.to_string());
		}
	}
}

pub(crate) fn remove_group_member(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, idx: u32) {
	let s = state.borrow();
	if let Some(jid) = s.selected_chat.clone() {
		if let Some(member_jid) = s.group_data_visible_member_jids.get(idx as usize).cloned() {
			let label = s.group_data_members.get(idx as usize).cloned().unwrap_or_default();
			drop(s);
			let confirm =
				MessageDialog::builder(&parts.frame, &format!("Remove {} from the group?", label), "Remove Member")
					.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
					.build();
			if confirm.show_modal() == wxdragon::id::ID_YES {
				let msg = serde_json::json!({
					"action": "update_group_participants",
					"data": { "jid": jid, "participants": [member_jid], "action": "remove" }
				});
				let _ = state.borrow().ws_tx.send(msg.to_string());
			}
		}
	}
}

pub(crate) fn join_community_subgroup(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>, idx: u32) {
	let s = state.borrow();
	if let Some(subgroup_jid) = s.group_data_subgroup_jids.get(idx as usize).cloned() {
		let msg = serde_json::json!({
			"action": "join_community_subgroup",
			"data": { "jid": subgroup_jid }
		});
		let _ = s.ws_tx.send(msg.to_string());
	}
}

pub(crate) fn open_group_invite(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>, message_id: String) {
	let msg = serde_json::json!({
		"action": "get_group_invite_info",
		"data": { "message_id": message_id }
	});
	let _ = state.borrow().ws_tx.send(msg.to_string());
}

pub(crate) fn join_group_invite(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>, message_id: String) {
	let msg = serde_json::json!({
		"action": "join_group_invite",
		"data": { "message_id": message_id }
	});
	let _ = state.borrow().ws_tx.send(msg.to_string());
}

pub(crate) fn leave_group(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, jid: String) {
	let chat_name = {
		let mut s = state.borrow_mut();
		s.chats.retain(|j| j != &jid);
		s.filtered_chats.retain(|j| j != &jid);
		s.archived_jids.remove(&jid);
		s.pinned_jids.remove(&jid);
		s.muted_jids.remove(&jid);
		s.unread_counts.remove(&jid);
		s.draft_messages.remove(&jid);
		let name = s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
		if s.selected_chat.as_ref() == Some(&jid) {
			s.selected_chat = None;
			clear_pending_reply(&mut s, parts);
			parts.conversations_parts.conversation_panel.show(false);
		}
		apply_chat_filter(&mut s, parts);
		refresh_tray_tooltip(&s, parts);
		name
	};
	let payload = serde_json::json!({
		"action": "leave_group",
		"data": {
			"jid": jid
		}
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	crate::announce::announce(
		parts.conversations_parts.messages_announcer.clone(),
		&format!("Left group {}.", chat_name),
	);
}
