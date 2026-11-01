use wxdragon::prelude::*;

#[allow(dead_code)]
#[derive(Clone)]
pub struct GroupDataDialogParts {
	pub dialog: Dialog,
	pub back_btn: Button,
	pub notebook: Notebook,

	pub members_panel: Panel,
	pub members_search_label: StaticText,
	pub members_search: TextCtrl,
	pub members_list: ListBox,
	pub photo: StaticBitmap,
	pub add_member_btn: Button,
	pub make_admin_btn: Button,
	pub remove_member_btn: Button,
	pub invite_link_btn: Button,
	pub join_requests_btn: Button,
	pub my_tag_label: StaticText,
	pub my_tag_field: TextCtrl,
	pub set_tag_btn: Button,

	pub media_panel: Panel,
	pub media_label: StaticText,
	pub media_list: ListBox,

	pub documents_panel: Panel,
	pub documents_label: StaticText,
	pub documents_list: ListBox,

	pub links_panel: Panel,
	pub links_label: StaticText,
	pub links_list: ListBox,

	pub groups_panel: Panel,
	pub groups_label: StaticText,
	pub groups_list: ListBox,
	pub join_group_btn: Button,
	pub link_group_btn: Button,
	pub unlink_group_btn: Button,

	pub settings_panel: Panel,
	pub name_label: StaticText,
	pub group_name_field: TextCtrl,
	pub desc_label: StaticText,
	pub group_desc_field: TextCtrl,
	pub announce_chk: CheckBox,
	pub locked_chk: CheckBox,
	pub forwarded_chk: CheckBox,
	pub admin_reports_chk: CheckBox,
	pub history_chk: CheckBox,
	pub admins_add_chk: CheckBox,
	pub approve_members_chk: CheckBox,
	pub deactivate_btn: Button,
	pub save_settings_btn: Button,
	pub reported_btn: Button,
}

pub fn build_group_data_dialog(parent: &impl WxWidget) -> GroupDataDialogParts {
	let dialog = Dialog::builder(parent, "Group Info").with_size(600, 500).build();
	let panel = Panel::builder(&dialog).build();
	let main_sizer = BoxSizer::builder(Orientation::Vertical).build();

	let back_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("< Back").build();
	main_sizer.add(&back_btn, 0, SizerFlag::All, 5);

	let notebook = Notebook::builder(&panel).build();

	let members_panel = Panel::builder(&notebook).build();
	let members_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let photo_row = BoxSizer::builder(Orientation::Horizontal).build();
	members_sizer.add_sizer(&photo_row, 0, SizerFlag::AlignCenterHorizontal | SizerFlag::Top, 8);

	let members_search_label = StaticText::builder(&members_panel).with_label("Search members").build();
	members_sizer.add(&members_search_label, 0, SizerFlag::All, 5);

	let members_search = TextCtrl::builder(&members_panel).with_style(TextCtrlStyle::ProcessEnter).build();
	members_sizer.add(&members_search, 0, SizerFlag::Expand | SizerFlag::All, 5);

	let members_list = ListBox::builder(&members_panel)
		.with_style(ListBoxStyle::from_bits_retain(crate::ui::chat_rows::OWNER_DRAW))
		.build();
	members_sizer.add(&members_list, 1, SizerFlag::Expand | SizerFlag::All, 5);

	// A nested sizer (not a wrapping Panel) groups these buttons horizontally -
	// an intermediate Panel here would be its own unlabeled tab stop between
	// members_list and these buttons, read by a screen reader as a bare
	// "panel" with nothing on it.
	let mem_btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let add_member_btn = Button::builder(&members_panel).with_label("Add member").build();
	crate::ui::owner_draw::install_tile(&add_member_btn, members_panel.get_background_color());
	mem_btn_sizer.add(&add_member_btn, 0, SizerFlag::Right, 5);

	let make_admin_btn = Button::builder(&members_panel).with_label("Make admin").build();
	crate::ui::owner_draw::install_tile(&make_admin_btn, members_panel.get_background_color());
	mem_btn_sizer.add(&make_admin_btn, 0, SizerFlag::Right, 5);

	let remove_member_btn = Button::builder(&members_panel).with_label("Remove member").build();
	crate::ui::owner_draw::install_tile(&remove_member_btn, members_panel.get_background_color());
	mem_btn_sizer.add(&remove_member_btn, 0, SizerFlag::Right, 5);

	let invite_link_btn = Button::builder(&members_panel).with_label("Invite link...").build();
	crate::ui::owner_draw::install_tile(&invite_link_btn, members_panel.get_background_color());
	mem_btn_sizer.add(&invite_link_btn, 0, SizerFlag::Right, 5);

	let join_requests_btn = Button::builder(&members_panel).with_label("Join requests...").build();
	crate::ui::owner_draw::install_tile(&join_requests_btn, members_panel.get_background_color());
	mem_btn_sizer.add(&join_requests_btn, 0, SizerFlag::Right, 5);

	members_sizer.add_sizer(&mem_btn_sizer, 0, SizerFlag::All, 5);

	let my_tag_label = StaticText::builder(&members_panel).with_label("Your tag in this group").build();
	members_sizer.add(&my_tag_label, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 5);
	let tag_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let my_tag_field = TextCtrl::builder(&members_panel).with_style(TextCtrlStyle::ProcessEnter).build();
	tag_sizer.add(&my_tag_field, 1, SizerFlag::Right, 5);
	let set_tag_btn = Button::builder(&members_panel).with_label("Set tag").build();
	tag_sizer.add(&set_tag_btn, 0, SizerFlag::Right, 5);
	members_sizer.add_sizer(&tag_sizer, 0, SizerFlag::Expand | SizerFlag::All, 5);
	// Made after everything else, so it's never what names a control (Windows
	// names one from the text just before it) and is never in Tab's way.
	let empty = wxdragon::bitmap::Bitmap::from_rgba(&[0, 0, 0, 0], 1, 1).expect("a 1x1 bitmap");
	let photo = StaticBitmap::builder(&members_panel).with_bitmap(Some(empty)).build();
	photo_row.add(&photo, 0, SizerFlag::AlignCenterVertical, 0);
	photo.show(false);
	members_panel.set_sizer(members_sizer, true);

	let media_panel = Panel::builder(&notebook).build();
	let media_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let media_label = StaticText::builder(&media_panel).with_label("Media").build();
	media_sizer.add(&media_label, 0, SizerFlag::All, 5);
	let media_list = ListBox::builder(&media_panel)
		.with_style(ListBoxStyle::from_bits_retain(crate::ui::chat_rows::OWNER_DRAW))
		.build();
	media_sizer.add(&media_list, 1, SizerFlag::Expand | SizerFlag::All, 5);
	media_panel.set_sizer(media_sizer, true);

	let documents_panel = Panel::builder(&notebook).build();
	let documents_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let documents_label = StaticText::builder(&documents_panel).with_label("Documents").build();
	documents_sizer.add(&documents_label, 0, SizerFlag::All, 5);
	let documents_list = ListBox::builder(&documents_panel).build();
	documents_sizer.add(&documents_list, 1, SizerFlag::Expand | SizerFlag::All, 5);
	documents_panel.set_sizer(documents_sizer, true);

	let links_panel = Panel::builder(&notebook).build();
	let links_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let links_label = StaticText::builder(&links_panel).with_label("Links").build();
	links_sizer.add(&links_label, 0, SizerFlag::All, 5);
	let links_list = ListBox::builder(&links_panel).build();
	links_sizer.add(&links_list, 1, SizerFlag::Expand | SizerFlag::All, 5);
	links_panel.set_sizer(links_sizer, true);

	let groups_panel = Panel::builder(&notebook).build();
	let groups_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let groups_label = StaticText::builder(&groups_panel).with_label("Linked groups").build();
	groups_sizer.add(&groups_label, 0, SizerFlag::All, 5);
	let groups_list = ListBox::builder(&groups_panel).build();
	groups_sizer.add(&groups_list, 1, SizerFlag::Expand | SizerFlag::All, 5);
	let join_group_btn = Button::builder(&groups_panel).with_label("Join group").build();
	let groups_btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	groups_btn_sizer.add(&join_group_btn, 0, SizerFlag::Right, 5);
	let link_group_btn = Button::builder(&groups_panel).with_label("Add existing group...").build();
	groups_btn_sizer.add(&link_group_btn, 0, SizerFlag::Right, 5);
	let unlink_group_btn = Button::builder(&groups_panel).with_label("Remove from community...").build();
	groups_btn_sizer.add(&unlink_group_btn, 0, SizerFlag::Right, 5);
	groups_sizer.add_sizer(&groups_btn_sizer, 0, SizerFlag::All, 5);
	groups_panel.set_sizer(groups_sizer, true);

	let settings_panel = Panel::builder(&notebook).build();
	let settings_sizer = BoxSizer::builder(Orientation::Vertical).build();

	let name_label = StaticText::builder(&settings_panel).with_label("Group Name").build();
	settings_sizer.add(&name_label, 0, SizerFlag::All, 5);

	let group_name_field = TextCtrl::builder(&settings_panel).build();
	settings_sizer.add(&group_name_field, 0, SizerFlag::Expand | SizerFlag::All, 5);

	let desc_label = StaticText::builder(&settings_panel).with_label("Group Description").build();
	settings_sizer.add(&desc_label, 0, SizerFlag::All, 5);

	let group_desc_field = TextCtrl::builder(&settings_panel).with_style(TextCtrlStyle::MultiLine).build();
	settings_sizer.add(&group_desc_field, 1, SizerFlag::Expand | SizerFlag::All, 5);

	let announce_chk = CheckBox::builder(&settings_panel).with_label("Only admins can send messages").build();
	settings_sizer.add(&announce_chk, 0, SizerFlag::All, 5);

	let locked_chk = CheckBox::builder(&settings_panel).with_label("Only admins can edit group info").build();
	settings_sizer.add(&locked_chk, 0, SizerFlag::All, 5);

	let forwarded_chk =
		CheckBox::builder(&settings_panel).with_label("Don't allow messages forwarded many times").build();
	settings_sizer.add(&forwarded_chk, 0, SizerFlag::All, 5);

	let admin_reports_chk =
		CheckBox::builder(&settings_panel).with_label("Members can report messages to admins").build();
	settings_sizer.add(&admin_reports_chk, 0, SizerFlag::All, 5);

	let history_chk = CheckBox::builder(&settings_panel).with_label("Share recent messages with people added").build();
	settings_sizer.add(&history_chk, 0, SizerFlag::All, 5);

	let admins_add_chk = CheckBox::builder(&settings_panel).with_label("Only admins can add members").build();
	settings_sizer.add(&admins_add_chk, 0, SizerFlag::All, 5);

	let approve_members_chk = CheckBox::builder(&settings_panel).with_label("Approve new members").build();
	settings_sizer.add(&approve_members_chk, 0, SizerFlag::All, 5);

	let settings_btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let save_settings_btn = Button::builder(&settings_panel).with_label("Save settings").build();
	settings_btn_sizer.add(&save_settings_btn, 0, SizerFlag::Right, 5);
	let reported_btn = Button::builder(&settings_panel).with_label("Reported messages...").build();
	settings_btn_sizer.add(&reported_btn, 0, SizerFlag::Right, 5);
	let deactivate_btn = Button::builder(&settings_panel).with_label("Deactivate community...").build();
	settings_btn_sizer.add(&deactivate_btn, 0, SizerFlag::Right, 5);
	settings_sizer.add_sizer(&settings_btn_sizer, 0, SizerFlag::All, 5);

	settings_panel.set_sizer(settings_sizer, true);

	notebook.add_page(&members_panel, "Members", true, None);
	notebook.add_page(&media_panel, "Media", false, None);
	notebook.add_page(&documents_panel, "Documents", false, None);
	notebook.add_page(&links_panel, "Links", false, None);
	notebook.add_page(&groups_panel, "Groups", false, None);
	notebook.add_page(&settings_panel, "Settings", false, None);

	main_sizer.add(&notebook, 1, SizerFlag::Expand | SizerFlag::All, 5);

	panel.set_sizer(main_sizer, true);

	GroupDataDialogParts {
		dialog,
		back_btn,
		notebook,
		members_panel,
		members_search_label,
		members_search,
		members_list,
		photo,
		add_member_btn,
		make_admin_btn,
		remove_member_btn,
		invite_link_btn,
		join_requests_btn,
		my_tag_label,
		my_tag_field,
		set_tag_btn,
		media_panel,
		media_label,
		media_list,
		documents_panel,
		documents_label,
		documents_list,
		links_panel,
		links_label,
		links_list,
		groups_panel,
		groups_label,
		groups_list,
		join_group_btn,
		link_group_btn,
		unlink_group_btn,
		settings_panel,
		name_label,
		group_name_field,
		desc_label,
		group_desc_field,
		announce_chk,
		locked_chk,
		forwarded_chk,
		admin_reports_chk,
		history_chk,
		admins_add_chk,
		approve_members_chk,
		deactivate_btn,
		save_settings_btn,
		reported_btn,
	}
}
