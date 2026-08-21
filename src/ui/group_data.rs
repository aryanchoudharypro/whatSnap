use wxdragon::prelude::*;

/// Every constructed widget is kept here for consistency, matching this
/// codebase's convention for dialog parts structs.
#[allow(dead_code)]
#[derive(Clone)]
pub struct GroupDataDialogParts {
	pub dialog: Dialog,
	pub back_btn: Button,
	pub notebook: Notebook,

	// Members tab
	pub members_panel: Panel,
	pub members_search_label: StaticText,
	pub members_search: TextCtrl,
	pub members_list: ListBox,
	pub add_member_btn: Button,
	pub make_admin_btn: Button,
	pub remove_member_btn: Button,

	// Media tab (photos and videos only, matching official WhatsApp)
	pub media_panel: Panel,
	pub media_list: ListBox,

	// Documents tab
	pub documents_panel: Panel,
	pub documents_list: ListBox,

	// Links tab
	pub links_panel: Panel,
	pub links_list: ListBox,

	// Settings tab
	pub settings_panel: Panel,
	pub name_label: StaticText,
	pub group_name_field: TextCtrl,
	pub desc_label: StaticText,
	pub group_desc_field: TextCtrl,
	pub announce_chk: CheckBox,
	pub locked_chk: CheckBox,
	pub save_settings_btn: Button,
}

pub fn build_group_data_dialog(parent: &impl WxWidget) -> GroupDataDialogParts {
	let dialog = Dialog::builder(parent, "Group Info").with_size(600, 500).build();
	let panel = Panel::builder(&dialog).build();
	let main_sizer = BoxSizer::builder(Orientation::Vertical).build();

	// The back button comes first, both in tab order and on screen, matching
	// the old WhatsApp UWP app's pattern of a "Back" control at the top of a
	// detail screen rather than a "Close" button at the bottom.
	let back_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("< Back").build();
	main_sizer.add(&back_btn, 0, SizerFlag::All, 5);

	let notebook = Notebook::builder(&panel).build();

	// --- Members Tab ---
	let members_panel = Panel::builder(&notebook).build();
	let members_sizer = BoxSizer::builder(Orientation::Vertical).build();

	let members_search_label = StaticText::builder(&members_panel).with_label("Search members").build();
	members_sizer.add(&members_search_label, 0, SizerFlag::All, 5);

	let members_search = TextCtrl::builder(&members_panel).with_style(TextCtrlStyle::ProcessEnter).build();
	members_sizer.add(&members_search, 0, SizerFlag::Expand | SizerFlag::All, 5);

	let members_list = ListBox::builder(&members_panel).build();
	members_sizer.add(&members_list, 1, SizerFlag::Expand | SizerFlag::All, 5);

	let mem_btn_panel = Panel::builder(&members_panel).build();
	let mem_btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let add_member_btn = Button::builder(&mem_btn_panel).with_label("Add member").build();
	mem_btn_sizer.add(&add_member_btn, 0, SizerFlag::Right, 5);

	let make_admin_btn = Button::builder(&mem_btn_panel).with_label("Make admin").build();
	mem_btn_sizer.add(&make_admin_btn, 0, SizerFlag::Right, 5);

	let remove_member_btn = Button::builder(&mem_btn_panel).with_label("Remove member").build();
	mem_btn_sizer.add(&remove_member_btn, 0, SizerFlag::Right, 5);

	mem_btn_panel.set_sizer(mem_btn_sizer, true);
	members_sizer.add(&mem_btn_panel, 0, SizerFlag::All, 5);
	members_panel.set_sizer(members_sizer, true);

	// --- Media Tab ---
	let media_panel = Panel::builder(&notebook).build();
	let media_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let media_list = ListBox::builder(&media_panel).build();
	media_sizer.add(&media_list, 1, SizerFlag::Expand | SizerFlag::All, 5);
	media_panel.set_sizer(media_sizer, true);

	// --- Documents Tab ---
	let documents_panel = Panel::builder(&notebook).build();
	let documents_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let documents_list = ListBox::builder(&documents_panel).build();
	documents_sizer.add(&documents_list, 1, SizerFlag::Expand | SizerFlag::All, 5);
	documents_panel.set_sizer(documents_sizer, true);

	// --- Links Tab ---
	let links_panel = Panel::builder(&notebook).build();
	let links_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let links_list = ListBox::builder(&links_panel).build();
	links_sizer.add(&links_list, 1, SizerFlag::Expand | SizerFlag::All, 5);
	links_panel.set_sizer(links_sizer, true);

	// --- Settings Tab ---
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

	let save_settings_btn = Button::builder(&settings_panel).with_label("Save settings").build();
	settings_sizer.add(&save_settings_btn, 0, SizerFlag::All, 5);

	settings_panel.set_sizer(settings_sizer, true);

	// Add pages to notebook
	notebook.add_page(&members_panel, "Members", true, None);
	notebook.add_page(&media_panel, "Media", false, None);
	notebook.add_page(&documents_panel, "Documents", false, None);
	notebook.add_page(&links_panel, "Links", false, None);
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
		add_member_btn,
		make_admin_btn,
		remove_member_btn,
		media_panel,
		media_list,
		documents_panel,
		documents_list,
		links_panel,
		links_list,
		settings_panel,
		name_label,
		group_name_field,
		desc_label,
		group_desc_field,
		announce_chk,
		locked_chk,
		save_settings_btn,
	}
}
