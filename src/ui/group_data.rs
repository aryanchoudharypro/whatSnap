use wxdragon::prelude::*;

use crate::ui::{dialog_look, theme};

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
	pub member_changes_btn: Button,

	pub overview: crate::ui::chat_overview::ChatOverviewParts,
}

pub fn build_group_data_dialog(parent: &impl WxWidget) -> GroupDataDialogParts {
	let dialog = Dialog::builder(parent, "Group Info")
		.with_size(640, 580)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let main_sizer = BoxSizer::builder(Orientation::Vertical).build();

	let back_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("< Back").build();
	dialog_look::pill_button(&back_btn, false, theme::Palette::CONTENT_BG());
	main_sizer.add(&back_btn, 0, SizerFlag::All, 6);

	let notebook = Notebook::builder(&panel).build();
	if theme::use_custom_colours() {
		notebook.set_background_color(theme::Palette::CONTENT_BG());
	}
	let overview = crate::ui::chat_overview::build(&notebook, true);
	let fields = overview.group.expect("the group overview has the name fields");
	let (name_label, group_name_field, desc_label, group_desc_field) =
		(fields.name_label, fields.name, fields.desc_label, fields.desc);

	let members_panel = Panel::builder(&notebook).build();
	theme::style_content(&members_panel);
	let members_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let photo_row = BoxSizer::builder(Orientation::Horizontal).build();
	members_sizer.add_sizer(&photo_row, 0, SizerFlag::AlignCenterHorizontal | SizerFlag::Top, 8);

	let members_search_label = StaticText::builder(&members_panel).with_label("Search members").build();
	theme::style_muted_label(&members_search_label);
	members_sizer.add(&members_search_label, 0, SizerFlag::Left | SizerFlag::Top, 6);

	let members_search = TextCtrl::builder(&members_panel).with_style(TextCtrlStyle::ProcessEnter).build();
	members_search.set_font(&theme::Fonts::body());
	members_sizer.add(&members_search, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 6);

	let members_list = ListBox::builder(&members_panel)
		.with_style(ListBoxStyle::from_bits_retain(crate::ui::chat_rows::OWNER_DRAW))
		.build();
	members_list.set_font(&theme::Fonts::body());
	members_sizer.add(&members_list, 1, SizerFlag::Expand | SizerFlag::All, 6);

	// A nested sizer (not a wrapping Panel) groups these buttons horizontally -
	// an intermediate Panel here would be its own unlabeled tab stop between
	// members_list and these buttons, read by a screen reader as a bare
	// "panel" with nothing on it.
	let mem_btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let add_member_btn = Button::builder(&members_panel).with_label("Add member").build();
	crate::ui::owner_draw::install_tile(&add_member_btn, members_panel.get_background_color());
	mem_btn_sizer.add(&add_member_btn, 0, SizerFlag::Right, 6);

	let invite_link_btn = Button::builder(&members_panel).with_label("Invite link...").build();
	crate::ui::owner_draw::install_tile(&invite_link_btn, members_panel.get_background_color());
	mem_btn_sizer.add(&invite_link_btn, 0, SizerFlag::Right, 6);

	let join_requests_btn = Button::builder(&members_panel).with_label("Join requests...").build();
	crate::ui::owner_draw::install_tile(&join_requests_btn, members_panel.get_background_color());
	mem_btn_sizer.add(&join_requests_btn, 0, SizerFlag::Right, 6);

	let member_changes_btn = Button::builder(&members_panel).with_label("See member changes").build();
	crate::ui::owner_draw::install_tile(&member_changes_btn, members_panel.get_background_color());
	mem_btn_sizer.add(&member_changes_btn, 0, SizerFlag::Right, 6);

	members_sizer.add_sizer(&mem_btn_sizer, 0, SizerFlag::All, 6);

	let my_tag_label = StaticText::builder(&members_panel).with_label("Your tag in this group").build();
	theme::style_muted_label(&my_tag_label);
	members_sizer.add(&my_tag_label, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 6);
	let tag_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let my_tag_field = TextCtrl::builder(&members_panel).with_style(TextCtrlStyle::ProcessEnter).build();
	my_tag_field.set_font(&theme::Fonts::body());
	tag_sizer.add(&my_tag_field, 1, SizerFlag::Right | SizerFlag::AlignCenterVertical, 6);
	let set_tag_btn = Button::builder(&members_panel).with_label("Set tag").build();
	dialog_look::pill_button(&set_tag_btn, false, theme::Palette::CONTENT_BG());
	tag_sizer.add(&set_tag_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 6);
	members_sizer.add_sizer(&tag_sizer, 0, SizerFlag::Expand | SizerFlag::All, 6);
	// Made after everything else, so it's never what names a control (Windows
	// names one from the text just before it) and is never in Tab's way.
	let empty = wxdragon::bitmap::Bitmap::from_rgba(&[0, 0, 0, 0], 1, 1).expect("a 1x1 bitmap");
	let photo = StaticBitmap::builder(&members_panel).with_bitmap(Some(empty)).build();
	photo_row.add(&photo, 0, SizerFlag::AlignCenterVertical, 0);
	photo.show(false);
	members_panel.set_sizer(members_sizer, true);

	let media_panel = Panel::builder(&notebook).build();
	theme::style_content(&media_panel);
	let media_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let media_label = StaticText::builder(&media_panel).with_label("Media").build();
	theme::style_heading(&media_label);
	media_sizer.add(&media_label, 0, SizerFlag::All, 6);
	let media_list = ListBox::builder(&media_panel)
		.with_style(ListBoxStyle::from_bits_retain(crate::ui::chat_rows::OWNER_DRAW))
		.build();
	media_list.set_font(&theme::Fonts::body());
	media_sizer.add(&media_list, 1, SizerFlag::Expand | SizerFlag::All, 6);
	media_panel.set_sizer(media_sizer, true);

	let documents_panel = Panel::builder(&notebook).build();
	theme::style_content(&documents_panel);
	let documents_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let documents_label = StaticText::builder(&documents_panel).with_label("Documents").build();
	theme::style_heading(&documents_label);
	documents_sizer.add(&documents_label, 0, SizerFlag::All, 6);
	let documents_list = ListBox::builder(&documents_panel).build();
	documents_list.set_font(&theme::Fonts::body());
	documents_sizer.add(&documents_list, 1, SizerFlag::Expand | SizerFlag::All, 6);
	documents_panel.set_sizer(documents_sizer, true);

	let links_panel = Panel::builder(&notebook).build();
	theme::style_content(&links_panel);
	let links_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let links_label = StaticText::builder(&links_panel).with_label("Links").build();
	theme::style_heading(&links_label);
	links_sizer.add(&links_label, 0, SizerFlag::All, 6);
	let links_list = ListBox::builder(&links_panel).build();
	links_list.set_font(&theme::Fonts::body());
	links_sizer.add(&links_list, 1, SizerFlag::Expand | SizerFlag::All, 6);
	links_panel.set_sizer(links_sizer, true);

	let groups_panel = Panel::builder(&notebook).build();
	theme::style_content(&groups_panel);
	let groups_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let groups_label = StaticText::builder(&groups_panel).with_label("Linked groups").build();
	theme::style_heading(&groups_label);
	groups_sizer.add(&groups_label, 0, SizerFlag::All, 6);
	let groups_list = ListBox::builder(&groups_panel).build();
	groups_list.set_font(&theme::Fonts::body());
	groups_sizer.add(&groups_list, 1, SizerFlag::Expand | SizerFlag::All, 6);
	let join_group_btn = Button::builder(&groups_panel).with_label("Join group").build();
	dialog_look::pill_button(&join_group_btn, true, theme::Palette::CONTENT_BG());
	let groups_btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	groups_btn_sizer.add(&join_group_btn, 0, SizerFlag::Right, 6);
	let link_group_btn = Button::builder(&groups_panel).with_label("Add existing group...").build();
	dialog_look::pill_button(&link_group_btn, false, theme::Palette::CONTENT_BG());
	groups_btn_sizer.add(&link_group_btn, 0, SizerFlag::Right, 6);
	let unlink_group_btn = Button::builder(&groups_panel).with_label("Remove from community...").build();
	dialog_look::pill_button(&unlink_group_btn, false, theme::Palette::CONTENT_BG());
	groups_btn_sizer.add(&unlink_group_btn, 0, SizerFlag::Right, 6);
	groups_sizer.add_sizer(&groups_btn_sizer, 0, SizerFlag::All, 6);
	groups_panel.set_sizer(groups_sizer, true);

	let settings_panel = Panel::builder(&notebook).build();
	theme::style_content(&settings_panel);
	let settings_sizer = BoxSizer::builder(Orientation::Vertical).build();

	let announce_chk = CheckBox::builder(&settings_panel).with_label("Only admins can send messages").build();
	dialog_look::switch(&announce_chk, theme::Palette::CONTENT_BG());
	settings_sizer.add(&announce_chk, 0, SizerFlag::Expand | SizerFlag::All, 4);

	let locked_chk = CheckBox::builder(&settings_panel).with_label("Only admins can edit group info").build();
	dialog_look::switch(&locked_chk, theme::Palette::CONTENT_BG());
	settings_sizer.add(&locked_chk, 0, SizerFlag::Expand | SizerFlag::All, 4);

	let forwarded_chk =
		CheckBox::builder(&settings_panel).with_label("Don't allow messages forwarded many times").build();
	dialog_look::switch(&forwarded_chk, theme::Palette::CONTENT_BG());
	settings_sizer.add(&forwarded_chk, 0, SizerFlag::Expand | SizerFlag::All, 4);

	let admin_reports_chk =
		CheckBox::builder(&settings_panel).with_label("Members can report messages to admins").build();
	dialog_look::switch(&admin_reports_chk, theme::Palette::CONTENT_BG());
	settings_sizer.add(&admin_reports_chk, 0, SizerFlag::Expand | SizerFlag::All, 4);

	let history_chk = CheckBox::builder(&settings_panel).with_label("Share recent messages with people added").build();
	dialog_look::switch(&history_chk, theme::Palette::CONTENT_BG());
	settings_sizer.add(&history_chk, 0, SizerFlag::Expand | SizerFlag::All, 4);

	let admins_add_chk = CheckBox::builder(&settings_panel).with_label("Only admins can add members").build();
	dialog_look::switch(&admins_add_chk, theme::Palette::CONTENT_BG());
	settings_sizer.add(&admins_add_chk, 0, SizerFlag::Expand | SizerFlag::All, 4);

	let approve_members_chk = CheckBox::builder(&settings_panel).with_label("Approve new members").build();
	dialog_look::switch(&approve_members_chk, theme::Palette::CONTENT_BG());
	settings_sizer.add(&approve_members_chk, 0, SizerFlag::Expand | SizerFlag::All, 4);

	let settings_btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let save_settings_btn = Button::builder(&settings_panel).with_label("Save settings").build();
	dialog_look::pill_button(&save_settings_btn, true, theme::Palette::CONTENT_BG());
	settings_btn_sizer.add(&save_settings_btn, 0, SizerFlag::Right, 6);
	let reported_btn = Button::builder(&settings_panel).with_label("Reported messages...").build();
	dialog_look::pill_button(&reported_btn, false, theme::Palette::CONTENT_BG());
	settings_btn_sizer.add(&reported_btn, 0, SizerFlag::Right, 6);
	let deactivate_btn = Button::builder(&settings_panel).with_label("Deactivate community...").build();
	dialog_look::pill_button(&deactivate_btn, false, theme::Palette::CONTENT_BG());
	settings_btn_sizer.add(&deactivate_btn, 0, SizerFlag::Right, 6);
	settings_sizer.add_sizer(&settings_btn_sizer, 0, SizerFlag::All, 6);

	settings_panel.set_sizer(settings_sizer, true);

	notebook.add_page(&members_panel, "Members", false, None);
	notebook.add_page(&media_panel, "Media", false, None);
	notebook.add_page(&documents_panel, "Documents", false, None);
	notebook.add_page(&links_panel, "Links", false, None);
	notebook.add_page(&groups_panel, "Groups", false, None);
	notebook.add_page(&settings_panel, "Settings", false, None);

	main_sizer.add(&notebook, 1, SizerFlag::Expand | SizerFlag::All, 6);

	panel.set_sizer(main_sizer, true);

	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	crate::ui::keys::escape_cancels_anywhere(dialog);

	GroupDataDialogParts {
		overview,
		dialog,
		back_btn,
		notebook,
		members_panel,
		members_search_label,
		members_search,
		members_list,
		photo,
		add_member_btn,
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
		member_changes_btn,
	}
}
