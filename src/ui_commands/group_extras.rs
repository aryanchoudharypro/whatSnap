use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{AppState, ui::window::WindowParts};

fn send(state: &Rc<RefCell<AppState>>, action: &str, data: serde_json::Value) {
	let payload = serde_json::json!({ "action": action, "data": data });
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

fn announce(parts: &WindowParts, message: &str) {
	crate::platform::announce::announce(parts.conversations_parts.messages_announcer, message);
}

fn text(data: &serde_json::Value, key: &str) -> String {
	data.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn confirm(parent: &impl WxWidget, question: &str, title: &str, action: &str) -> bool {
	let dialog = MessageDialog::builder(parent, question, title)
		.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
		.build();
	dialog.set_yes_no_labels(action, crate::i18n::tr("Cancel"));
	dialog.show_modal() == ID_YES
}

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
				let name = s.contacts.get(&group).cloned().unwrap_or_else(|| crate::i18n::tr("this group").to_string());
				(s.group_data_community.clone().unwrap_or(jid), group, name)
			};
			let question = if crate::i18n::is_indonesian() {
				format!(
					"Keluarkan {name} dari komunitas? Grup ini tetap ada sebagai grup mandiri, dan anggotanya keluar dari komunitas."
				)
			} else {
				format!(
					"Remove {name} from the community? It stays a group of its own, and its members leave the community."
				)
			};
			if confirm(dialog, &question, crate::i18n::tr("Remove Group"), crate::i18n::tr("Remove")) {
				send(state, "unlink_community_group", serde_json::json!({ "community": community, "group": group }));
			}
		}
		"deactivate_community" => {
			let question = crate::i18n::tr(
				"Deactivate this community? Its groups stay as groups of their own, but the community \
				 and its announcements are gone for everyone. This can't be undone.",
			);
			if confirm(dialog, question, crate::i18n::tr("Deactivate Community"), crate::i18n::tr("Deactivate")) {
				send(state, "deactivate_community", serde_json::json!({ "jid": jid }));
			}
		}
		_ => {}
	}
}

pub(crate) fn new_community(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let name_dialog =
		TextEntryDialog::builder(&parts.frame, crate::i18n::tr("Community name:"), crate::i18n::tr("New Community"))
			.build();
	if name_dialog.show_modal() != wxdragon::id::ID_OK {
		return;
	}
	let name = name_dialog.get_value().unwrap_or_default().trim().to_string();
	if name.is_empty() {
		return;
	}
	let desc_dialog = TextEntryDialog::builder(
		&parts.frame,
		crate::i18n::tr("What's this community for? (You can leave this empty.)"),
		crate::i18n::tr("New Community"),
	)
	.build();
	if desc_dialog.show_modal() != wxdragon::id::ID_OK {
		return;
	}
	let description = desc_dialog.get_value().unwrap_or_default().trim().to_string();
	send(state, "create_community", serde_json::json!({ "name": name, "description": description }));
	announce(parts, crate::i18n::tr("Creating community..."));
}

pub(crate) fn on_event(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, t: &str, data: &serde_json::Value) {
	let dialog = &parts.group_data_dialog.dialog;
	match t {
		"join_requests" => answer_join_requests(state, parts, data),
		"join_requests_answered" => {
			let count = data.get("count").and_then(|c| c.as_u64()).unwrap_or(0);
			let failed = data.get("failed").and_then(|c| c.as_u64()).unwrap_or(0);
			let approve = data.get("approve").and_then(|c| c.as_bool()).unwrap_or(false);
			let spoken = if crate::i18n::is_indonesian() {
				let mut s = format!("{} {count} orang.", if approve { "Disetujui" } else { "Ditolak" });
				if failed > 0 {
					s.push_str(&format!(" {failed} orang tidak dapat diproses."));
				}
				s
			} else {
				let people = |n: u64| if n == 1 { "1 person".to_string() } else { format!("{} people", n) };
				let mut s = format!("{} {}.", if approve { "Approved" } else { "Rejected" }, people(count));
				if failed > 0 {
					s.push_str(&format!(" {} couldn't be.", people(failed)));
				}
				s
			};
			announce(parts, &spoken);
		}
		"linkable_groups" => {
			let groups = data.get("groups").and_then(|g| g.as_array()).cloned().unwrap_or_default();
			if groups.is_empty() {
				MessageDialog::builder(
					dialog,
					crate::i18n::tr(
						"You aren't an admin of any group that could be added. A group can be in one community at a time.",
					),
					crate::i18n::tr("Add Existing Group"),
				)
				.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
				.build()
				.show_modal();
				return;
			}
			let labels: Vec<String> = groups.iter().map(|g| text(g, "name")).collect();
			let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
			let choice = SingleChoiceDialog::builder(
				dialog,
				crate::i18n::tr("Add which group to the community?"),
				crate::i18n::tr("Add Existing Group"),
				&refs,
			)
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
			announce(
				parts,
				if linked {
					crate::i18n::tr("Group added to the community.")
				} else {
					crate::i18n::tr("Group removed from the community.")
				},
			);
			send(state, "get_community_subgroups", serde_json::json!({ "jid": text(data, "community") }));
		}
		"community_created" => {
			let name = text(data, "name");
			let msg = if crate::i18n::is_indonesian() {
				format!("Komunitas {name} dibuat.")
			} else {
				format!("Community {name} created.")
			};
			announce(parts, &msg);
		}
		"community_deactivated" => {
			if dialog.is_shown() {
				dialog.end_modal(wxdragon::id::ID_OK);
			}
			announce(parts, crate::i18n::tr("Community deactivated."));
		}
		_ => {}
	}
}

fn answer_join_requests(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let dialog = &parts.group_data_dialog.dialog;
	let jid = text(data, "jid");
	let requests = data.get("requests").and_then(|r| r.as_array()).cloned().unwrap_or_default();
	if requests.is_empty() {
		MessageDialog::builder(dialog, crate::i18n::tr("No one is asking to join."), crate::i18n::tr("Join Requests"))
			.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
			.build()
			.show_modal();
		return;
	}
	let all: Vec<String> = requests.iter().map(|r| text(r, "jid")).collect();
	let mut labels = if crate::i18n::is_indonesian() {
		vec![format!("Setujui semua ({})", all.len()), format!("Tolak semua ({})", all.len())]
	} else {
		vec![format!("Approve everyone ({})", all.len()), format!("Reject everyone ({})", all.len())]
	};
	for r in &requests {
		let when = r
			.get("requested_at")
			.and_then(|t| t.as_i64())
			.and_then(whatsnap::LocalTime::from_unix)
			.map(whatsnap::day_and_time)
			.unwrap_or_default();
		let entry = if crate::i18n::is_indonesian() {
			format!("{}, meminta pada {}", text(r, "name"), when)
		} else {
			format!("{}, asked {}", text(r, "name"), when)
		};
		labels.push(entry);
	}
	let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
	let title = if crate::i18n::is_indonesian() {
		format!("{} orang meminta bergabung:", all.len())
	} else {
		format!(
			"{} asking to join:",
			if all.len() == 1 { "1 person".to_string() } else { format!("{} people", all.len()) }
		)
	};
	let choice = SingleChoiceDialog::builder(dialog, &title, crate::i18n::tr("Join Requests"), &refs).build();
	if choice.show_modal() != wxdragon::id::ID_OK {
		return;
	}
	let Ok(picked) = usize::try_from(choice.get_selection()) else { return };
	let (jids, approve) = match picked {
		0 => (all, true),
		1 => (all, false),
		i => {
			let Some(person) = all.get(i - 2).cloned() else { return };
			let choices = [crate::i18n::tr("Approve"), crate::i18n::tr("Reject")];
			let answer =
				SingleChoiceDialog::builder(dialog, &labels[i], crate::i18n::tr("Join Request"), &choices).build();
			if answer.show_modal() != wxdragon::id::ID_OK {
				return;
			}
			(vec![person], answer.get_selection() == 0)
		}
	};
	send(state, "answer_join_requests", serde_json::json!({ "jid": jid, "jids": jids, "approve": approve }));
}
