use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{platform::accounts::AccountsManager, ui::theme};

pub fn show_accounts_dialog(
	parent: &impl WxWidget,
	on_switch: impl Fn(String) + 'static,
	on_add: impl Fn(Option<String>) + 'static,
	on_rename: impl Fn(String, String) + 'static,
	on_remove: impl Fn(String) + 'static,
) {
	let dialog = Dialog::builder(parent, "WhatsApp Accounts")
		.with_size(540, 520)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);

	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let heading = StaticText::builder(&panel).with_label("Manage and Switch WhatsApp Accounts").build();
	theme::style_sidebar_heading(&heading);
	sizer.add(&heading, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Right, 12);

	let desc = StaticText::builder(&panel)
		.with_label(
			"Select an account to switch to. Only the active account connects to WhatsApp and receives notifications.",
		)
		.build();
	theme::style_muted_label(&desc);
	sizer.add(&desc, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Right, 12);

	let list_label = StaticText::builder(&panel).with_label("Configured Accounts:").build();
	theme::style_muted_label(&list_label);
	sizer.add(&list_label, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let list = ListBox::builder(&panel).build();
	list.set_font(&theme::Fonts::body());
	sizer.add(&list, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 8);

	let details_label_header = StaticText::builder(&panel).with_label("Account Details:").build();
	theme::style_muted_label(&details_label_header);
	sizer.add(&details_label_header, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let details_label = StaticText::builder(&panel).with_label("No account selected").build();
	details_label.set_font(&theme::Fonts::body());
	sizer.add(&details_label, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 4);

	let announcer = StaticText::builder(&panel).with_label("").build();
	live_region::set_live_region(&announcer);
	theme::style_muted_label(&announcer);
	sizer.add(&announcer, 0, SizerFlag::Left | SizerFlag::Top, 6);

	let buttons = BoxSizer::builder(Orientation::Horizontal).build();
	let switch_btn = Button::builder(&panel).with_label("&Switch to Account").build();
	let add_btn = Button::builder(&panel).with_label("&Add Account...").build();
	let rename_btn = Button::builder(&panel).with_label("&Rename...").build();
	let remove_btn = Button::builder(&panel).with_label("&Remove...").build();
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Close").build();

	buttons.add(&switch_btn, 0, SizerFlag::Right, 6);
	buttons.add(&add_btn, 0, SizerFlag::Right, 6);
	buttons.add(&rename_btn, 0, SizerFlag::Right, 6);
	buttons.add(&remove_btn, 0, SizerFlag::Right, 6);
	buttons.add_stretch_spacer(1);
	buttons.add(&close_btn, 0, SizerFlag::empty(), 0);
	sizer.add_sizer(&buttons, 0, SizerFlag::Expand | SizerFlag::All, 12);

	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let manager = Rc::new(RefCell::new(AccountsManager::load()));

	let update_ui = {
		let manager = manager.clone();
		let list = list;
		let list_label = list_label;
		let details_label = details_label;
		let switch_btn = switch_btn;
		let remove_btn = remove_btn;
		let rename_btn = rename_btn;

		move || {
			let mgr = manager.borrow();
			let accounts = &mgr.accounts;
			let active_id = &mgr.active_id;

			let items: Vec<String> = accounts
				.iter()
				.map(|acc| {
					if &acc.id == active_id { format!("[Active] {}", acc.display_label()) } else { acc.display_label() }
				})
				.collect();

			let item_refs: Vec<&str> = items.iter().map(String::as_str).collect();
			list.set_items(&item_refs);
			list_label.set_label(&format!("Configured Accounts ({}):", accounts.len()));

			// Select active account by default if nothing selected
			let sel_idx = list
				.get_selection()
				.map(|i| i as usize)
				.unwrap_or_else(|| accounts.iter().position(|a| &a.id == active_id).unwrap_or(0));

			if !accounts.is_empty() {
				let clamped = sel_idx.min(accounts.len() - 1);
				list.set_selection(clamped as u32, true);
				let selected = &accounts[clamped];
				let is_active = &selected.id == active_id;

				let status_str = if is_active {
					"Active (Connected / Receiving notifications)"
				} else {
					"Idle (Disconnected / Completely silent)"
				};

				let phone_str = if selected.phone.is_empty() { "Not yet paired" } else { &selected.phone };

				let push_str = if selected.push_name.is_empty() { "—" } else { &selected.push_name };

				let info = format!(
					"Label: {}\nStatus: {}\nPhone: {}\nWhatsApp Profile Name: {}\nProfile ID: {}",
					selected.name, status_str, phone_str, push_str, selected.id
				);
				details_label.set_label(&info);

				switch_btn.enable(!is_active);
				remove_btn.enable(accounts.len() > 1);
				rename_btn.enable(true);
			} else {
				details_label.set_label("No accounts configured.");
				switch_btn.enable(false);
				remove_btn.enable(false);
				rename_btn.enable(false);
			}
		}
	};

	update_ui();

	// List selection changed
	let update_on_select = update_ui.clone();
	list.on_selection_changed(move |_| {
		update_on_select();
	});

	// Switch Account
	let manager_switch = manager.clone();
	let dialog_switch = dialog;
	let announcer_switch = announcer;
	let on_switch_fn = Rc::new(on_switch);
	let on_switch_action = {
		let on_switch_fn = on_switch_fn.clone();
		move || {
			let mgr = manager_switch.borrow();
			if let Some(idx) = list.get_selection() {
				if let Some(acc) = mgr.accounts.get(idx as usize) {
					if acc.id != mgr.active_id {
						let target_id = acc.id.clone();
						let target_name = acc.display_label();
						drop(mgr);
						crate::platform::announce::announce(
							announcer_switch,
							&format!("Switching to account {}...", target_name),
						);
						on_switch_fn(target_id);
						dialog_switch.end_modal(ID_OK);
					}
				}
			}
		}
	};

	let on_switch_for_btn = on_switch_action.clone();
	switch_btn.on_click(move |_| {
		on_switch_for_btn();
	});

	// Enter key switches account
	let on_switch_for_enter = on_switch_action.clone();
	crate::ui::keys::on_enter(&list, Rc::default(), move || {
		on_switch_for_enter();
	});

	// Add Account
	let dialog_add = dialog;
	let on_add_fn = Rc::new(on_add);
	add_btn.on_click(move |_| {
		let prompt = TextEntryDialog::builder(
			&dialog_add,
			"Enter a label for the new WhatsApp account (e.g., Work, Personal, Business):",
			"Add WhatsApp Account",
		)
		.with_default_value("")
		.build();

		if prompt.show_modal() == ID_OK {
			let name = prompt.get_value().unwrap_or_default();
			let label = if name.trim().is_empty() { None } else { Some(name.trim().to_string()) };
			dialog_add.end_modal(ID_OK);
			on_add_fn(label);
		}
	});

	// Rename Account
	let manager_rename = manager.clone();
	let dialog_rename = dialog;
	let update_rename = update_ui.clone();
	let announcer_rename = announcer;
	let on_rename_fn = Rc::new(on_rename);
	rename_btn.on_click(move |_| {
		let mgr = manager_rename.borrow();
		let sel = list.get_selection().map(|i| i as usize);
		if let Some(idx) = sel {
			if let Some(acc) = mgr.accounts.get(idx) {
				let id = acc.id.clone();
				let current_name = acc.name.clone();
				drop(mgr);

				let prompt =
					TextEntryDialog::builder(&dialog_rename, "Enter a new label for this account:", "Rename Account")
						.with_default_value(&current_name)
						.build();

				if prompt.show_modal() == ID_OK {
					let new_name = prompt.get_value().unwrap_or_default();
					if !new_name.trim().is_empty() {
						manager_rename.borrow_mut().rename_account(&id, new_name.trim());
						on_rename_fn(id, new_name.trim().to_string());
						update_rename();
						crate::platform::announce::announce(
							announcer_rename,
							&format!("Account renamed to {}", new_name.trim()),
						);
					}
				}
			}
		}
	});

	// Remove Account
	let manager_remove = manager.clone();
	let dialog_remove = dialog;
	let update_remove = update_ui.clone();
	let announcer_remove = announcer;
	let on_remove_fn = Rc::new(on_remove);
	remove_btn.on_click(move |_| {
		let mgr = manager_remove.borrow();
		let sel = list.get_selection().map(|i| i as usize);
		if let Some(idx) = sel {
			if let Some(acc) = mgr.accounts.get(idx) {
				let id = acc.id.clone();
				let label = acc.display_label();
				let is_active = acc.id == mgr.active_id;
				drop(mgr);

				let prompt = MessageDialog::builder(
					&dialog_remove,
					&format!(
						"Are you sure you want to remove account '{}'?\n\nAll saved login and session data for this account will be permanently deleted from this device.",
						label
					),
					"Confirm Remove Account",
				)
				.with_style(MessageDialogStyle::OK | MessageDialogStyle::Cancel | MessageDialogStyle::IconWarning)
				.build();

				if prompt.show_modal() == ID_OK {
					let res = manager_remove.borrow_mut().remove_account(&id);
					match res {
						Ok(_) => {
							on_remove_fn(id);
							crate::platform::announce::announce(announcer_remove, "Account removed.");
							if is_active {
								dialog_remove.end_modal(ID_OK);
							} else {
								update_remove();
							}
						}
						Err(err) => {
							MessageDialog::builder(&dialog_remove, &err, "Cannot Remove Account")
								.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
								.build()
								.show_modal();
						}
					}
				}
			}
		}
	});

	close_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));

	let escape_closes = crate::ui::keys::escape_cancels(dialog);
	list.on_key_down(escape_closes);

	list.set_focus();
	dialog.show_modal();
	dialog.destroy();
}
