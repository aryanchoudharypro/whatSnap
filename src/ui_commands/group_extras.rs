//! Group Info's join requests and community buttons, and Chat menu > "New
//! Community...": what they ask the backend for, and what it answers
//! (`join_requests`, `join_requests_answered`, `linkable_groups`,
//! `community_groups_changed`, `community_created`, `community_deactivated`).

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{AppState, ui::window::WindowParts};

fn send(state: &Rc<RefCell<AppState>>, action: &str, data: serde_json::Value) {
	let payload = serde_json::json!({ "action": action, "data": data });
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

fn announce(parts: &WindowParts, message: &str) {
	crate::announce::announce(parts.conversations_parts.messages_announcer, message);
}

fn text(data: &serde_json::Value, key: &str) -> String {
	data.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn confirm(parent: &impl WxWidget, question: &str, title: &str, action: &str) -> bool {
	let dialog = MessageDialog::builder(parent, question, title)
		.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
		.build();
	dialog.set_yes_no_labels(action, "Cancel");
	dialog.show_modal() == ID_YES
}

/// One of Group Info's buttons was pressed.
pub(crate) fn group_action(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, action: &str) {
	let Some(jid) = state.borrow().selected_chat.clone().filter(|j| j.ends_with("@g.us")) else { return };
	let dialog = &parts.group_data_dialog.dialog;
	match action {
		"join_requests" => send(state, "get_join_requests", serde_json::json!({ "jid": jid })),
		"link_group" => send(state, "get_linkable_groups", serde_json::json!({ "jid": jid })),
		"unlink_group" => {
			let (community, group, name) = {
				let s = state.borrow();
				let Some(idx) = parts.group_data_dialog.groups_list.get_selection() else { return };
				let Some(group) = s.group_data_subgroup_jids.get(idx as usize).cloned() else { return };
				let name = s.contacts.get(&group).cloned().unwrap_or_else(|| "this group".to_string());
				(s.group_data_community.clone().unwrap_or(jid), group, name)
			};
			let question = format!(
				"Remove {} from the community? It stays a group of its own, and its members leave the community.",
				name
			);
			if confirm(dialog, &question, "Remove Group", "Remove") {
				send(state, "unlink_community_group", serde_json::json!({ "community": community, "group": group }));
			}
		}
		"deactivate_community" => {
			let question = "Deactivate this community? Its groups stay as groups of their own, but the community \
			                and its announcements are gone for everyone. This can't be undone.";
			if confirm(dialog, question, "Deactivate Community", "Deactivate") {
				send(state, "deactivate_community", serde_json::json!({ "jid": jid }));
			}
		}
		_ => {}
	}
}

/// Chat menu > "New Community...": a name, and a description if wanted.
pub(crate) fn new_community(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let name_dialog = TextEntryDialog::builder(&parts.frame, "Community name:", "New Community").build();
	if name_dialog.show_modal() != wxdragon::id::ID_OK {
		return;
	}
	let name = name_dialog.get_value().unwrap_or_default().trim().to_string();
	if name.is_empty() {
		return;
	}
	let desc_dialog = TextEntryDialog::builder(
		&parts.frame,
		"What's this community for? (You can leave this empty.)",
		"New Community",
	)
	.build();
	if desc_dialog.show_modal() != wxdragon::id::ID_OK {
		return;
	}
	let description = desc_dialog.get_value().unwrap_or_default().trim().to_string();
	send(state, "create_community", serde_json::json!({ "name": name, "description": description }));
	announce(parts, "Creating community...");
}

pub(crate) fn on_event(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, t: &str, data: &serde_json::Value) {
	let dialog = &parts.group_data_dialog.dialog;
	match t {
		"join_requests" => answer_join_requests(state, parts, data),
		"join_requests_answered" => {
			let count = data.get("count").and_then(|c| c.as_u64()).unwrap_or(0);
			let failed = data.get("failed").and_then(|c| c.as_u64()).unwrap_or(0);
			let approve = data.get("approve").and_then(|c| c.as_bool()).unwrap_or(false);
			let people = |n: u64| if n == 1 { "1 person".to_string() } else { format!("{} people", n) };
			let mut spoken = format!("{} {}.", if approve { "Approved" } else { "Rejected" }, people(count));
			if failed > 0 {
				spoken.push_str(&format!(" {} couldn't be.", people(failed)));
			}
			announce(parts, &spoken);
		}
		"linkable_groups" => {
			let groups = data.get("groups").and_then(|g| g.as_array()).cloned().unwrap_or_default();
			if groups.is_empty() {
				MessageDialog::builder(
					dialog,
					"You aren't an admin of any group that could be added. A group can be in one community at a time.",
					"Add Existing Group",
				)
				.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
				.build()
				.show_modal();
				return;
			}
			let labels: Vec<String> = groups.iter().map(|g| text(g, "name")).collect();
			let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
			let choice =
				SingleChoiceDialog::builder(dialog, "Add which group to the community?", "Add Existing Group", &refs)
					.build();
			if choice.show_modal() != wxdragon::id::ID_OK {
				return;
			}
			let Some(group) = usize::try_from(choice.get_selection()).ok().and_then(|i| groups.get(i)) else { return };
			send(
				state,
				"link_community_group",
				serde_json::json!({ "community": text(data, "community"), "group": text(group, "jid") }),
			);
		}
		"community_groups_changed" => {
			let linked = data.get("linked").and_then(|l| l.as_bool()).unwrap_or(false);
			announce(parts, if linked { "Group added to the community." } else { "Group removed from the community." });
			send(state, "get_community_subgroups", serde_json::json!({ "jid": text(data, "community") }));
		}
		"community_created" => announce(parts, &format!("Community {} created.", text(data, "name"))),
		"community_deactivated" => {
			if dialog.is_shown() {
				dialog.end_modal(wxdragon::id::ID_OK);
			}
			announce(parts, "Community deactivated.");
		}
		_ => {}
	}
}

/// Who's asking to join: approve or reject one, or everyone at once.
fn answer_join_requests(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let dialog = &parts.group_data_dialog.dialog;
	let jid = text(data, "jid");
	let requests = data.get("requests").and_then(|r| r.as_array()).cloned().unwrap_or_default();
	if requests.is_empty() {
		MessageDialog::builder(dialog, "No one is asking to join.", "Join Requests")
			.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
			.build()
			.show_modal();
		return;
	}
	let all: Vec<String> = requests.iter().map(|r| text(r, "jid")).collect();
	let mut labels = vec![format!("Approve everyone ({})", all.len()), format!("Reject everyone ({})", all.len())];
	for r in &requests {
		let when = r
			.get("requested_at")
			.and_then(|t| t.as_i64())
			.and_then(whantv::LocalTime::from_unix)
			.map(|dt| dt.format("%b %d, %I:%M %p"))
			.unwrap_or_default();
		labels.push(format!("{}, asked {}", text(r, "name"), when));
	}
	let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
	let title = format!(
		"{} asking to join:",
		if all.len() == 1 { "1 person".to_string() } else { format!("{} people", all.len()) }
	);
	let choice = SingleChoiceDialog::builder(dialog, &title, "Join Requests", &refs).build();
	if choice.show_modal() != wxdragon::id::ID_OK {
		return;
	}
	let Ok(picked) = usize::try_from(choice.get_selection()) else { return };
	let (jids, approve) = match picked {
		0 => (all, true),
		1 => (all, false),
		i => {
			let Some(person) = all.get(i - 2).cloned() else { return };
			let answer =
				SingleChoiceDialog::builder(dialog, &labels[i], "Join Request", &["Approve", "Reject"]).build();
			if answer.show_modal() != wxdragon::id::ID_OK {
				return;
			}
			(vec![person], answer.get_selection() == 0)
		}
	};
	send(state, "answer_join_requests", serde_json::json!({ "jid": jid, "jids": jids, "approve": approve }));
}
