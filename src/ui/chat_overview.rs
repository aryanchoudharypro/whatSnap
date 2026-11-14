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
		let name_label = StaticText::builder(&panel).with_label(crate::i18n::tr("Group name")).build();
		sizer.add(&name_label, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 8);
		let name = TextCtrl::builder(&panel).build();
		sizer.add(&name, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 8);
		let desc_label = StaticText::builder(&panel).with_label(crate::i18n::tr("Group description")).build();
		sizer.add(&desc_label, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 8);
		let desc = TextCtrl::builder(&panel).with_style(TextCtrlStyle::MultiLine).build();
		sizer.add(&desc, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 8);
		let save_btn = Button::builder(&panel).with_label(crate::i18n::tr("Save name and description")).build();
		sizer.add(&save_btn, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 8);
		GroupFields { name_label, name, desc_label, desc, save_btn }
	});

	let label = StaticText::builder(&panel).with_label(crate::i18n::tr("Chat overview")).build();
	sizer.add(&label, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 8);
	let summary = TextCtrl::builder(&panel)
		.with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly | TextCtrlStyle::from_bits_retain(0x0020_0000))
		.build();
	sizer.add(&summary, 1, SizerFlag::Expand | SizerFlag::All, 8);

	let lock_chk = CheckBox::builder(&panel).with_label(crate::i18n::tr("Lock this chat")).build();
	sizer.add(&lock_chk, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 8);
	let restricted_chk =
		CheckBox::builder(&panel).with_label(crate::i18n::tr("Restricted chat (changed on your phone)")).build();
	restricted_chk.enable(false);
	sizer.add(&restricted_chk, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 8);

	let rows = BoxSizer::builder(Orientation::Vertical).build();
	let button = |text: &'static str| {
		let btn = Button::builder(&panel).with_label(crate::i18n::tr(text)).build();
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
	notebook.add_page(&panel, crate::i18n::tr("Overview"), true, None);

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
	starred: bool,
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
		starred: !is_channel,
	}
}

pub(crate) fn refresh_block_state(s: &AppState, o: &ChatOverviewParts, jid: &str) {
	let a = applies(jid);
	let name = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.to_string());
	let kind = if jid.ends_with("@g.us") { crate::i18n::tr("Group") } else { crate::i18n::tr("Contact") };
	let locked = s.locked_jids.contains(jid);
	let blocked = crate::ui_commands::blocking::is_blocked(s, jid);
	let mut lines = vec![format!("{kind}: {name}")];
	lines.push(format!(
		"{}: {}",
		crate::i18n::tr("Notifications"),
		if s.muted_jids.contains(jid) { crate::i18n::tr("muted") } else { crate::i18n::tr("on") }
	));
	lines.push(format!(
		"{}: {}",
		crate::i18n::tr("Locked"),
		if locked { crate::i18n::tr("yes") } else { crate::i18n::tr("no") }
	));
	if a.block && blocked {
		lines.push(crate::i18n::tr("Blocked: yes").to_string());
	}
	o.summary.set_value(&lines.join("\n"));
	o.block_btn.show(a.block);
	o.block_btn.set_label(if blocked {
		crate::i18n::tr("Unblock contact")
	} else {
		crate::i18n::tr("Block contact...")
	});
	o.panel.layout();
}

pub(crate) fn refresh(state: &Rc<RefCell<AppState>>, o: &ChatOverviewParts, jid: &str) {
	let a = applies(jid);
	let mut s = state.borrow_mut();
	refresh_block_state(&s, o, jid);
	let locked = s.locked_jids.contains(jid);
	o.lock_chk.set_value(locked);
	o.restricted_chk.set_value(false);
	o.restricted_chk.show(false);
	o.disappearing_btn.show(a.disappearing);
	o.privacy_btn.show(a.privacy);
	o.report_btn.show(a.report);
	o.starred_btn.show(a.starred);
	if a.starred {
		let count = s.starred_entries.iter().filter(|e| e.jid == jid).count();
		if count > 0 {
			o.starred_btn.set_label(&format!("{} ({count})", crate::i18n::tr("Starred messages")));
		} else {
			o.starred_btn.set_label(crate::i18n::tr("Starred messages"));
		}
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
		let (tx, nav, st, sf) = (ui_tx.clone(), parts.nav_list, state.clone(), parts.conversations_parts.search_field);
		o.starred_btn.on_click(move |_| {
			let chat_name = target(&st).and_then(|jid| st.borrow().contacts.get(&jid).cloned());
			dialog.end_modal(ID_CANCEL);
			let idx = st.borrow().settings.tab_order().iter().position(|&t| t == 2).unwrap_or(2) as u32;
			nav.set_selection(idx, true);
			if let Some(ref name) = chat_name {
				sf.set_value(name);
			} else {
				sf.set_value("");
			}
			let _ = tx.send(UiCommand::NavSelected(2));
		});
		let (st, tx) = (state.clone(), ui_tx.clone());
		o.export_btn.on_click(move |_| {
			let Some(jid) = target(&st) else { return };
			let name = st.borrow().contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
			let default_name = format!("{}.zip", crate::ui::window::sanitize_export_filename(&name));
			let chooser = FileDialog::builder(&dialog)
				.with_message(crate::i18n::tr("Export chat as"))
				.with_default_file(&default_name)
				.with_wildcard(crate::i18n::tr("Zip archive (*.zip)|*.zip"))
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
			let msg = crate::i18n::tr_fmt(
				"Delete all messages in the chat with {}? The chat itself will stay in your list.",
				&[&name],
			);
			let ask = MessageDialog::builder(&dialog, &msg, crate::i18n::tr("Confirm Clear Chat"))
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
				let choices = [crate::i18n::tr("Delete chat only"), crate::i18n::tr("Delete chat and exit group")];
				let msg = crate::i18n::tr_fmt("Delete chat with {}?", &[&name]);
				let ask =
					SingleChoiceDialog::builder(&dialog, &msg, crate::i18n::tr("Confirm Delete"), &choices).build();
				if ask.show_modal() != ID_OK {
					return;
				}
				match ask.get_selection() {
					0 => UiCommand::DeleteChat { jid },
					1 => UiCommand::DeleteChatAndLeaveGroup { jid },
					_ => return,
				}
			} else {
				let msg = crate::i18n::tr_fmt("Are you sure you want to delete chat with {}?", &[&name]);
				let ask = MessageDialog::builder(&dialog, &msg, crate::i18n::tr("Confirm Delete"))
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
