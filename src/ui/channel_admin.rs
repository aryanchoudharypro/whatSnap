//! Dialogs for running our own channels: creating one, editing its details,
//! choosing its picture, and managing its followers and admins.

use wxdragon::prelude::*;

use crate::ui::theme;

/// What the create and edit dialogs collect.
pub struct ChannelDetails {
	pub name: String,
	pub description: String,
	/// A picture file chosen in the create dialog.
	pub picture: Option<String>,
}

/// Asks for a channel's name and description. `creating` adds a picture
/// choice and WhatsApp's note about channels being public; editing starts
/// from the current `name` and `description`.
pub fn ask_channel_details(
	parent: &dyn WxWidget,
	creating: bool,
	name: &str,
	description: &str,
) -> Option<ChannelDetails> {
	let dialog = Dialog::builder(parent, if creating { "Create channel" } else { "Edit channel" })
		.with_size(480, if creating { 460 } else { 400 })
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	if creating {
		let intro = StaticText::builder(&panel)
			.with_label(
				"Channels are public: anyone can find and follow yours, and followers can't see your phone number. \
				 By creating a channel you agree to WhatsApp's Channels Guidelines.",
			)
			.build();
		intro.wrap(440);
		sizer.add(&intro, 0, SizerFlag::All, 10);
	}

	let name_label = StaticText::builder(&panel).with_label("Channel name").build();
	theme::style_muted_label(&name_label);
	sizer.add(&name_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let name_field = TextCtrl::builder(&panel).build();
	name_field.set_font(&theme::Fonts::body());
	name_field.set_value(name);
	sizer.add(&name_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let description_label = StaticText::builder(&panel).with_label("Description (optional)").build();
	theme::style_muted_label(&description_label);
	sizer.add(&description_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let description_field = TextCtrl::builder(&panel).with_style(TextCtrlStyle::MultiLine).build();
	description_field.set_font(&theme::Fonts::body());
	description_field.set_value(description);
	sizer.add(&description_field, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let picture = std::rc::Rc::new(std::cell::RefCell::new(None::<String>));
	let picture_btn = Button::builder(&panel).with_label("Choose picture (optional)...").build();
	sizer.add(&picture_btn, 0, SizerFlag::Left | SizerFlag::Top, 10);
	picture_btn.show(creating);

	let announcer = StaticText::builder(&panel).with_label("").build();
	live_region::set_live_region(&announcer);
	theme::style_muted_label(&announcer);
	sizer.add(&announcer, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let ok_btn =
		Button::builder(&panel).with_id(ID_OK).with_label(if creating { "Create channel" } else { "Save" }).build();
	theme::style_primary_button(&ok_btn);
	let cancel_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Cancel").build();
	btn_sizer.add(&ok_btn, 0, SizerFlag::Right, 8);
	btn_sizer.add_stretch_spacer(1);
	btn_sizer.add(&cancel_btn, 0, SizerFlag::empty(), 0);
	sizer.add_sizer(&btn_sizer, 0, SizerFlag::Expand | SizerFlag::All, 10);
	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let picture_choice = picture.clone();
	picture_btn.on_click(move |_| {
		if let Some(path) = pick_channel_picture(&dialog) {
			let file = std::path::Path::new(&path).file_name().unwrap_or_default().to_string_lossy().to_string();
			picture_btn.set_label(&format!("Picture: {} (change...)", file));
			crate::announce::announce(announcer, &format!("Picture chosen: {}.", file));
			*picture_choice.borrow_mut() = Some(path);
		}
	});
	ok_btn.on_click(move |_| {
		if name_field.get_value().trim().is_empty() {
			crate::announce::announce(announcer, "Give the channel a name.");
			name_field.set_focus();
			return;
		}
		dialog.end_modal(ID_OK);
	});
	cancel_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));

	name_field.set_focus();
	name_field.set_selection(0, name_field.get_last_position());
	let result = dialog.show_modal();
	let details = (result == ID_OK).then(|| ChannelDetails {
		name: name_field.get_value().trim().to_string(),
		description: description_field.get_value().trim().to_string(),
		picture: picture.borrow().clone(),
	});
	dialog.destroy();
	details
}

/// Asks for a picture file for a channel.
pub fn pick_channel_picture(parent: &dyn WxWidget) -> Option<String> {
	let file_dialog = FileDialog::builder(parent)
		.with_message("Choose a picture for the channel")
		.with_wildcard("Pictures|*.jpg;*.jpeg;*.png;*.gif")
		.with_style(FileDialogStyle::Open | FileDialogStyle::FileMustExist)
		.build();
	let path = if file_dialog.show_modal() == ID_OK { file_dialog.get_path() } else { None };
	file_dialog.destroy();
	path
}

/// One of a channel's followers.
pub struct ChannelFollower {
	pub jid: String,
	pub name: String,
	pub role: String,
}

impl ChannelFollower {
	fn label(&self) -> String {
		match self.role.as_str() {
			"owner" => format!("{}, owner", self.name),
			"admin" => format!("{}, admin", self.name),
			_ => self.name.clone(),
		}
	}
}

/// What was chosen in the followers dialog.
pub enum FollowerAction {
	InviteAdmin { jid: String, name: String },
	RemoveAdmin { jid: String, name: String },
	MakeOwner { jid: String, name: String },
}

/// Lists a channel's followers, admins first. The owner can invite a
/// follower to be an admin, remove an admin, or hand the channel over to one.
pub fn show_channel_followers(
	parent: &dyn WxWidget,
	channel_name: &str,
	mut followers: Vec<ChannelFollower>,
	is_owner: bool,
) -> Option<FollowerAction> {
	let rank = |role: &str| match role {
		"owner" => 0,
		"admin" => 1,
		_ => 2,
	};
	followers.sort_by(|a, b| {
		rank(&a.role).cmp(&rank(&b.role)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
	});

	let dialog = Dialog::builder(parent, &format!("Followers of {}", channel_name))
		.with_size(460, 520)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let admins = followers.iter().filter(|f| f.role == "admin").count();
	let summary = format!(
		"{} shown, {}",
		match followers.len() {
			1 => "1 follower".to_string(),
			n => format!("{} followers", n),
		},
		match admins {
			0 => "no admins besides the owner".to_string(),
			1 => "1 admin".to_string(),
			n => format!("{} admins", n),
		}
	);
	let list_label = StaticText::builder(&panel).with_label(&summary).build();
	theme::style_muted_label(&list_label);
	sizer.add(&list_label, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Right, 10);
	let list = ListBox::builder(&panel).build();
	list.set_font(&theme::Fonts::body());
	let labels: Vec<String> = followers.iter().map(ChannelFollower::label).collect();
	let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
	list.set_items(&label_refs);
	if !followers.is_empty() {
		list.set_selection(0, true);
	}
	sizer.add(&list, 1, SizerFlag::Expand | SizerFlag::All, 10);

	let buttons = BoxSizer::builder(Orientation::Horizontal).build();
	let invite_btn = Button::builder(&panel).with_label("Make admin...").build();
	let remove_btn = Button::builder(&panel).with_label("Remove as admin").build();
	let owner_btn = Button::builder(&panel).with_label("Make owner...").build();
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Close").build();
	buttons.add(&invite_btn, 0, SizerFlag::Right, 8);
	buttons.add(&remove_btn, 0, SizerFlag::Right, 8);
	buttons.add(&owner_btn, 0, SizerFlag::Right, 8);
	buttons.add_stretch_spacer(1);
	buttons.add(&close_btn, 0, SizerFlag::empty(), 0);
	sizer.add_sizer(&buttons, 0, SizerFlag::Expand | SizerFlag::All, 10);
	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let followers = std::rc::Rc::new(followers);
	// The admin actions only show for an admin, and only to the owner.
	let update_buttons = {
		let followers = followers.clone();
		move || {
			let role = list.get_selection().and_then(|i| followers.get(i as usize)).map(|f| f.role.clone());
			let on_admin = role.as_deref() == Some("admin");
			let on_follower = role.is_some() && !on_admin && role.as_deref() != Some("owner");
			invite_btn.show(is_owner && on_follower);
			remove_btn.show(is_owner && on_admin);
			owner_btn.show(is_owner && on_admin);
			panel.layout();
		}
	};
	update_buttons();
	let update_on_select = update_buttons.clone();
	list.on_selection_changed(move |_| update_on_select());

	let chosen = std::rc::Rc::new(std::cell::RefCell::new(None::<FollowerAction>));
	let selected = {
		let followers = followers.clone();
		move || list.get_selection().and_then(|i| followers.get(i as usize)).map(|f| (f.jid.clone(), f.name.clone()))
	};
	let chosen_invite = chosen.clone();
	let selected_invite = selected.clone();
	invite_btn.on_click(move |_| {
		let Some((jid, name)) = selected_invite() else { return };
		let confirm = MessageDialog::builder(
			&dialog,
			&format!(
				"Invite {} to be an admin of this channel? They'll get the invite in your chat with them, and become an admin once they accept it.",
				name
			),
			"Make admin",
		)
		.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
		.build();
		if confirm.show_modal() == ID_YES {
			*chosen_invite.borrow_mut() = Some(FollowerAction::InviteAdmin { jid, name });
			dialog.end_modal(ID_OK);
		}
	});
	let chosen_remove = chosen.clone();
	let selected_remove = selected.clone();
	remove_btn.on_click(move |_| {
		let Some((jid, name)) = selected_remove() else { return };
		let confirm =
			MessageDialog::builder(&dialog, &format!("Remove {} as an admin of this channel?", name), "Remove admin")
				.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
				.build();
		if confirm.show_modal() == ID_YES {
			*chosen_remove.borrow_mut() = Some(FollowerAction::RemoveAdmin { jid, name });
			dialog.end_modal(ID_OK);
		}
	});
	let chosen_owner = chosen.clone();
	owner_btn.on_click(move |_| {
		let Some((jid, name)) = selected() else { return };
		let confirm = MessageDialog::builder(
			&dialog,
			&format!(
				"Make {} the owner of this channel? You'll become an admin, and only the new owner can undo this.",
				name
			),
			"Change owner",
		)
		.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconWarning)
		.build();
		if confirm.show_modal() == ID_YES {
			*chosen_owner.borrow_mut() = Some(FollowerAction::MakeOwner { jid, name });
			dialog.end_modal(ID_OK);
		}
	});
	close_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));

	list.set_focus();
	dialog.show_modal();
	dialog.destroy();
	chosen.borrow_mut().take()
}

/// Who reacted to a channel post, or who voted for each option of a channel
/// poll, as one list: "👍: Alice", "Pizza: Bob". A group no one is in reads
/// "Salad: no one". `summary` heads the list.
pub fn show_channel_people(parent: &dyn WxWidget, title: &str, summary: &str, groups: &[(String, Vec<String>)]) {
	let mut rows = Vec::new();
	for (label, names) in groups {
		if names.is_empty() {
			rows.push(format!("{}: no one", label));
		}
		for name in names {
			rows.push(format!("{}: {}", label, name));
		}
	}

	let dialog = Dialog::builder(parent, title)
		.with_size(420, 460)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();
	let list_label = StaticText::builder(&panel).with_label(summary).build();
	theme::style_muted_label(&list_label);
	sizer.add(&list_label, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Right, 10);
	let list = ListBox::builder(&panel).build();
	list.set_font(&theme::Fonts::body());
	let row_refs: Vec<&str> = rows.iter().map(String::as_str).collect();
	list.set_items(&row_refs);
	if !rows.is_empty() {
		list.set_selection(0, true);
	}
	sizer.add(&list, 1, SizerFlag::Expand | SizerFlag::All, 10);
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Close").build();
	sizer.add(&close_btn, 0, SizerFlag::AlignRight | SizerFlag::All, 10);
	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();
	close_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));
	list.set_focus();
	dialog.show_modal();
	dialog.destroy();
}

/// The admin invites of a channel no one has accepted yet. Returns the
/// invite (JID and name) the owner chose to cancel.
pub fn show_pending_admin_invites(
	parent: &dyn WxWidget,
	channel_name: &str,
	invites: Vec<(String, String)>,
) -> Option<(String, String)> {
	let dialog = Dialog::builder(parent, &format!("Pending admin invites for {}", channel_name))
		.with_size(420, 420)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();
	let summary = match invites.len() {
		0 => "No admin invites are waiting to be accepted.".to_string(),
		1 => "1 invite waiting to be accepted".to_string(),
		n => format!("{} invites waiting to be accepted", n),
	};
	let list_label = StaticText::builder(&panel).with_label(&summary).build();
	theme::style_muted_label(&list_label);
	sizer.add(&list_label, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Right, 10);
	let list = ListBox::builder(&panel).build();
	list.set_font(&theme::Fonts::body());
	let names: Vec<&str> = invites.iter().map(|(_, name)| name.as_str()).collect();
	list.set_items(&names);
	if !invites.is_empty() {
		list.set_selection(0, true);
	}
	sizer.add(&list, 1, SizerFlag::Expand | SizerFlag::All, 10);
	let buttons = BoxSizer::builder(Orientation::Horizontal).build();
	let cancel_invite_btn = Button::builder(&panel).with_label("Cancel invite...").build();
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Close").build();
	buttons.add(&cancel_invite_btn, 0, SizerFlag::Right, 8);
	buttons.add_stretch_spacer(1);
	buttons.add(&close_btn, 0, SizerFlag::empty(), 0);
	sizer.add_sizer(&buttons, 0, SizerFlag::Expand | SizerFlag::All, 10);
	cancel_invite_btn.show(!invites.is_empty());
	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let invites = std::rc::Rc::new(invites);
	let chosen = std::rc::Rc::new(std::cell::RefCell::new(None::<(String, String)>));
	let chosen_cancel = chosen.clone();
	let invites_cancel = invites.clone();
	cancel_invite_btn.on_click(move |_| {
		let Some(invite) = list.get_selection().and_then(|i| invites_cancel.get(i as usize)).cloned() else { return };
		let confirm = MessageDialog::builder(
			&dialog,
			&format!("Cancel the invite for {} to be an admin of this channel?", invite.1),
			"Cancel invite",
		)
		.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
		.build();
		if confirm.show_modal() == ID_YES {
			*chosen_cancel.borrow_mut() = Some(invite);
			dialog.end_modal(ID_OK);
		}
	});
	close_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));
	list.set_focus();
	dialog.show_modal();
	dialog.destroy();
	chosen.borrow_mut().take()
}
