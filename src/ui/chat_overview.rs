use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{AppState, UiCommand, ui::window::WindowParts};

#[derive(Clone, Copy)]
pub struct GroupFields {
	pub name_label: StaticText,
	pub name: TextCtrl,
	pub desc_label: StaticText,
	pub desc: TextCtrl,
	pub save_btn: Button,
}

#[derive(Clone, Copy)]
pub struct ChatOverviewParts {
	pub group: Option<GroupFields>,
	pub details_btn: Option<Button>,
	pub panel: Panel,
	pub summary: TextCtrl,
	pub lock_chk: CheckBox,
	pub restricted_chk: CheckBox,
	pub notify_btn: Button,
	pub disappearing_btn: Button,
	pub privacy_btn: Button,
	pub starred_btn: Button,
	pub export_btn: Button,
	pub clear_btn: Button,
	pub block_btn: Button,
	pub report_btn: Button,
	pub delete_btn: Button,
}

pub fn build(notebook: &Notebook, is_group: bool) -> ChatOverviewParts {
	let panel = Panel::builder(notebook).build();
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let group = is_group.then(|| {
		let name_label = StaticText::builder(&panel).with_label("Group name").build();
		sizer.add(&name_label, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 8);
		let name = TextCtrl::builder(&panel).build();
		sizer.add(&name, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 8);
		let desc_label = StaticText::builder(&panel).with_label("Group description").build();
		sizer.add(&desc_label, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 8);
		let desc = TextCtrl::builder(&panel).with_style(TextCtrlStyle::MultiLine).build();
		sizer.add(&desc, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 8);
		let save_btn = Button::builder(&panel).with_label("Save name and description").build();
		sizer.add(&save_btn, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 8);
		GroupFields { name_label, name, desc_label, desc, save_btn }
	});

	let label = StaticText::builder(&panel).with_label("Chat overview").build();
	sizer.add(&label, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 8);
	let summary = TextCtrl::builder(&panel)
		.with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly | TextCtrlStyle::from_bits_retain(0x0020_0000))
		.build();
	sizer.add(&summary, 1, SizerFlag::Expand | SizerFlag::All, 8);

	let lock_chk = CheckBox::builder(&panel).with_label("Lock this chat").build();
	sizer.add(&lock_chk, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 8);
	let restricted_chk = CheckBox::builder(&panel).with_label("Restricted chat (changed on your phone)").build();
	restricted_chk.enable(false);
	sizer.add(&restricted_chk, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 8);

	let rows = BoxSizer::builder(Orientation::Vertical).build();
	let button = |text: &str| {
		let btn = Button::builder(&panel).with_label(text).build();
		rows.add(&btn, 0, SizerFlag::Right | SizerFlag::Top, 6);
		btn
	};
	let details_btn = (!is_group).then(|| button("Contact details..."));
	let notify_btn = button("Notification settings...");
	let disappearing_btn = button("Disappearing messages...");
	let privacy_btn = button("Advanced chat privacy...");
	let starred_btn = button("Starred messages");
	let export_btn = button("Export chat...");
	let clear_btn = button("Clear chat...");
	let block_btn = button("Block contact...");
	let report_btn = button("Report...");
	let delete_btn = button("Delete chat...");
	sizer.add_sizer(&rows, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom, 8);
	panel.set_sizer(sizer, true);
	notebook.add_page(&panel, "Overview", true, None);

	ChatOverviewParts {
		group,
		details_btn,
		panel,
		summary,
		lock_chk,
		restricted_chk,
		notify_btn,
		disappearing_btn,
		privacy_btn,
		starred_btn,
		export_btn,
		clear_btn,
		block_btn,
		report_btn,
		delete_btn,
	}
}

struct Applies {
	disappearing: bool,
	privacy: bool,
	block: bool,
	report: bool,
}

fn applies(jid: &str) -> Applies {
	let is_person = jid.ends_with("@s.whatsapp.net") || jid.ends_with("@lid");
	let is_channel = jid.ends_with("@newsletter");
	let is_broadcast = jid.ends_with("@broadcast");
	Applies {
		disappearing: !is_channel,
		privacy: !is_channel && !is_broadcast,
		block: is_person,
		report: !is_channel && !is_broadcast,
	}
}

pub(crate) fn refresh(state: &Rc<RefCell<AppState>>, o: &ChatOverviewParts, jid: &str) {
	let a = applies(jid);
	let mut s = state.borrow_mut();
	let name = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.to_string());
	let is_group = jid.ends_with("@g.us");
	let is_channel = jid.ends_with("@newsletter");
	let locked = s.locked_jids.contains(jid);
	let blocked = crate::ui_commands::blocking::is_blocked(&s, jid);

	let mut lines = Vec::new();
	if is_group {
		lines.push(format!("Group: {}", name));
		let members_count = s.group_data_members.len();
		if members_count > 0 {
			lines.push(format!("Participants: {} members", members_count));
		}
		lines.push("Security: End-to-end encrypted (Messages and calls stay between members of this group)".to_string());
	} else if is_channel {
		lines.push(format!("Channel: {}", name));
		lines.push("Type: Public broadcast channel".to_string());
	} else {
		lines.push(format!("Contact: {}", name));
		let (card_phone, card_push, card_status) = if let Some(ref c) = s.conversation_data_contact {
			(c.phone.clone(), c.push_name.clone(), c.status.clone())
		} else {
			(String::new(), String::new(), String::new())
		};

		let raw_phone = if !card_phone.is_empty() {
			card_phone
		} else {
			jid.split('@').next().unwrap_or(jid).to_string()
		};
		let phone = whatsnap::phone::format_international(&raw_phone).unwrap_or(raw_phone);
		if !phone.is_empty() && phone != name {
			lines.push(format!("Phone: {}", phone));
		}

		if !card_push.is_empty() && card_push != name {
			lines.push(format!("WhatsApp Name: ~{}", card_push));
		}

		if !card_status.is_empty() {
			lines.push(format!("About: {}", card_status));
		}

		lines.push("Security: End-to-end encrypted (Messages and calls stay between you and this contact)".to_string());
	}

	lines.push(format!("Notifications: {}", if s.muted_jids.contains(jid) { "Muted" } else { "On" }));
	lines.push(format!("Chat Lock: {}", if locked { "Locked (Protected)" } else { "Unlocked" }));

	if a.block {
		lines.push(format!("Block Status: {}", if blocked { "Blocked" } else { "Not blocked" }));
	}

	let media_count = s.conversation_data_media.len();
	let doc_count = s.conversation_data_documents.len();
	let link_count = s.conversation_data_links.len();
	if media_count > 0 || doc_count > 0 || link_count > 0 {
		lines.push(format!("Media & Docs: {} media, {} documents, {} links", media_count, doc_count, link_count));
	}

	o.summary.set_value(&lines.join("\n"));
	o.lock_chk.set_value(locked);
	o.restricted_chk.set_value(false);
	o.restricted_chk.show(false);
	o.disappearing_btn.show(a.disappearing);
	o.privacy_btn.show(a.privacy);
	o.block_btn.show(a.block);
	o.block_btn.set_label(if blocked { "Unblock contact" } else { "Block contact..." });
	o.report_btn.show(a.report);
	if let Some(ref btn) = o.details_btn {
		let is_saved = s.conversation_data_contact.as_ref().is_some_and(|c| c.saved);
		btn.set_label(if is_saved { "Edit contact / details..." } else { "Add to contacts / details..." });
	}
	if a.privacy {
		s.overview_probe = true;
		let payload = serde_json::json!({ "action": "get_chat_privacy", "data": { "jid": jid } });
		let _ = s.ws_tx.send(payload.to_string());
	}
	drop(s);
	o.panel.layout();
}

pub(crate) fn privacy_answer(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, restricted: bool) -> bool {
	if !std::mem::take(&mut state.borrow_mut().overview_probe) {
		return false;
	}
	for o in [&parts.conversation_data_dialog.overview, &parts.group_data_dialog.overview] {
		o.restricted_chk.set_value(restricted);
		o.restricted_chk.show(restricted);
		o.panel.layout();
	}
	true
}

fn target(state: &Rc<RefCell<AppState>>) -> Option<String> {
	let s = state.borrow();
	s.info_chat.clone().or_else(|| s.selected_chat.clone())
}

pub(crate) fn wire(parts: &WindowParts, state: &Rc<RefCell<AppState>>, ui_tx: &crate::UiCommandSender) {
	let dialogs = [
		(parts.conversation_data_dialog.overview, parts.conversation_data_dialog.dialog),
		(parts.group_data_dialog.overview, parts.group_data_dialog.dialog),
	];
	for (o, dialog) in dialogs {
		if let Some(g) = o.group {
			let tx = ui_tx.clone();
			let (announce_chk, locked_chk) = (parts.group_data_dialog.announce_chk, parts.group_data_dialog.locked_chk);
			g.save_btn.on_click(move |_| {
				let _ = tx.send(UiCommand::UpdateGroupSettings {
					name: g.name.get_value(),
					topic: g.desc.get_value(),
					announce: announce_chk.is_checked(),
					locked: locked_chk.is_checked(),
				});
			});
		}
		if let Some(details) = o.details_btn {
			let details_dialog = parts.conversation_data_dialog.details_dialog;
			let info_field = parts.conversation_data_dialog.info_field;
			details.on_click(move |_| {
				info_field.set_focus();
				crate::ui::show_modal_again(&details_dialog);
			});
		}
		let (st, tx) = (state.clone(), ui_tx.clone());
		o.lock_chk.on_toggled(move |event| {
			if let Some(jid) = target(&st) {
				let _ = tx.send(UiCommand::ToggleLockChat { jid, lock: event.is_checked() });
			}
		});
		let (st, tx) = (state.clone(), ui_tx.clone());
		o.notify_btn.on_click(move |_| {
			if let Some(jid) = target(&st) {
				let _ = tx.send(UiCommand::ShowChatNotify { jid });
			}
		});
		let (st, tx) = (state.clone(), ui_tx.clone());
		o.disappearing_btn.on_click(move |_| {
			if let Some(jid) = target(&st) {
				let _ = tx.send(UiCommand::ChooseDisappearingTimer { jid });
			}
		});
		let tx = ui_tx.clone();
		o.privacy_btn.on_click(move |_| {
			let _ = tx.send(UiCommand::ShowChatPrivacy);
		});
		let (tx, nav, st) = (ui_tx.clone(), parts.nav_list, state.clone());
		o.starred_btn.on_click(move |_| {
			dialog.end_modal(ID_CANCEL);
			let idx = st.borrow().settings.tab_order().iter().position(|&t| t == 2).unwrap_or(2) as u32;
			nav.set_selection(idx, true);
			let _ = tx.send(UiCommand::NavSelected(2));
		});
		let (st, tx) = (state.clone(), ui_tx.clone());
		o.export_btn.on_click(move |_| {
			let Some(jid) = target(&st) else { return };
			let name = st.borrow().contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
			let default_name = format!("{}.zip", crate::ui::window::sanitize_export_filename(&name));
			let chooser = FileDialog::builder(&dialog)
				.with_message("Export chat as")
				.with_default_file(&default_name)
				.with_wildcard("Zip archive (*.zip)|*.zip")
				.with_style(FileDialogStyle::Save | FileDialogStyle::OverwritePrompt)
				.build();
			if chooser.show_modal() == ID_OK
				&& let Some(path) = chooser.get_path()
			{
				let _ = tx.send(UiCommand::ExportChat { jid, dest_path: path });
			}
		});
		let (st, tx) = (state.clone(), ui_tx.clone());
		o.clear_btn.on_click(move |_| {
			let Some(jid) = target(&st) else { return };
			let name = st.borrow().contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
			let ask = MessageDialog::builder(
				&dialog,
				&format!("Delete all messages in the chat with {}? The chat itself will stay in your list.", name),
				"Confirm Clear Chat",
			)
			.with_style(MessageDialogStyle::OK | MessageDialogStyle::Cancel | MessageDialogStyle::IconQuestion)
			.build();
			if ask.show_modal() == ID_OK {
				let _ = tx.send(UiCommand::ClearChat { jid });
			}
		});
		let (st, tx) = (state.clone(), ui_tx.clone());
		o.block_btn.on_click(move |_| {
			if let Some(jid) = target(&st) {
				let _ = tx.send(UiCommand::ToggleBlockChat { jid });
			}
		});
		let (st, tx) = (state.clone(), ui_tx.clone());
		o.report_btn.on_click(move |_| {
			if let Some(jid) = target(&st) {
				let _ = tx.send(UiCommand::ReportChat { jid });
			}
		});
		let (st, tx) = (state.clone(), ui_tx.clone());
		o.delete_btn.on_click(move |_| {
			let Some(jid) = target(&st) else { return };
			let name = st.borrow().contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
			let command = if jid.ends_with("@g.us") {
				let choices = ["Delete chat only", "Delete chat and exit group"];
				let ask = SingleChoiceDialog::builder(
					&dialog,
					&format!("Delete chat with {}?", name),
					"Confirm Delete",
					&choices,
				)
				.build();
				if ask.show_modal() != ID_OK {
					return;
				}
				match ask.get_string_selection().as_deref() {
					Some("Delete chat only") => UiCommand::DeleteChat { jid },
					Some("Delete chat and exit group") => UiCommand::DeleteChatAndLeaveGroup { jid },
					_ => return,
				}
			} else {
				let ask = MessageDialog::builder(
					&dialog,
					&format!("Are you sure you want to delete chat with {}?", name),
					"Confirm Delete",
				)
				.with_style(MessageDialogStyle::OK | MessageDialogStyle::Cancel | MessageDialogStyle::IconQuestion)
				.build();
				if ask.show_modal() != ID_OK {
					return;
				}
				UiCommand::DeleteChat { jid }
			};
			dialog.end_modal(ID_CANCEL);
			let _ = tx.send(command);
		});
	}
}
