use wxdragon::prelude::*;

use crate::ui::{dialog_look, theme};

#[allow(dead_code)]
#[derive(Clone)]
pub struct ConversationDataDialogParts {
	pub dialog: Dialog,
	pub back_btn: Button,
	pub notebook: Notebook,

	pub info_panel: Panel,
	pub info_field: TextCtrl,
	pub voice_call_btn: Button,
	pub video_call_btn: Button,
	pub contact_btn: Button,
	pub delete_contact_btn: Button,
	pub chat_privacy_btn: Button,
	pub photo: StaticBitmap,

	pub media_panel: Panel,
	pub media_label: StaticText,
	pub media_list: ListBox,

	pub documents_panel: Panel,
	pub documents_label: StaticText,
	pub documents_list: ListBox,

	pub links_panel: Panel,
	pub links_label: StaticText,
	pub links_list: ListBox,

	pub common_groups_panel: Panel,
	pub common_groups_label: StaticText,
	pub common_groups_list: ListBox,

	pub overview: crate::ui::chat_overview::ChatOverviewParts,
	pub details_dialog: Dialog,
}

pub fn build_conversation_data_dialog(parent: &impl WxWidget) -> ConversationDataDialogParts {
	let dialog = Dialog::builder(parent, "Contact Info")
		.with_size(620, 560)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let main_sizer = BoxSizer::builder(Orientation::Vertical).build();

	let back_btn = Button::builder(&dialog).with_id(ID_CANCEL).with_label("< Back").build();
	dialog_look::pill_button(&back_btn, false, theme::Palette::CONTENT_BG());
	main_sizer.add(&back_btn, 0, SizerFlag::All, 6);

	let notebook = Notebook::builder(&dialog).build();
	if theme::use_custom_colours() {
		notebook.set_background_color(theme::Palette::CONTENT_BG());
	}

	let overview = crate::ui::chat_overview::build(&notebook, false);

	let details_dialog = Dialog::builder(&dialog, "Contact Details").with_size(560, 420).build();
	theme::style_content(&details_dialog);
	let info_panel = Panel::builder(&details_dialog).build();
	theme::style_content(&info_panel);
	let info_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let photo_row = BoxSizer::builder(Orientation::Horizontal).build();
	info_sizer.add_sizer(&photo_row, 0, SizerFlag::AlignCenterHorizontal | SizerFlag::Top, 10);

	let info_field = TextCtrl::builder(&info_panel)
		.with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly | TextCtrlStyle::from_bits_retain(0x0020_0000))
		.build();
	info_field.set_font(&theme::Fonts::body());
	info_sizer.add(&info_field, 1, SizerFlag::Expand | SizerFlag::All, 12);

	let actions_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let voice_call_btn = Button::builder(&info_panel).with_label("Voice call").build();
	crate::ui::owner_draw::install_tile(&voice_call_btn, theme::Palette::PANEL_BG());
	actions_sizer.add(&voice_call_btn, 0, SizerFlag::Right, 6);
	let video_call_btn = Button::builder(&info_panel).with_label("Video call").build();
	crate::ui::owner_draw::install_tile(&video_call_btn, theme::Palette::PANEL_BG());
	actions_sizer.add(&video_call_btn, 0, SizerFlag::Right, 6);
	let contact_btn = Button::builder(&info_panel).with_label("Edit contact").build();
	crate::ui::owner_draw::install_tile(&contact_btn, theme::Palette::PANEL_BG());
	actions_sizer.add(&contact_btn, 0, SizerFlag::Right, 6);
	let delete_contact_btn = Button::builder(&info_panel).with_label("Delete contact").build();
	crate::ui::owner_draw::install_tile(&delete_contact_btn, theme::Palette::PANEL_BG());
	actions_sizer.add(&delete_contact_btn, 0, SizerFlag::Right, 6);
	let chat_privacy_btn = Button::builder(&info_panel).with_label("Advanced chat privacy...").build();
	crate::ui::owner_draw::install_tile(&chat_privacy_btn, theme::Palette::PANEL_BG());
	actions_sizer.add(&chat_privacy_btn, 0, SizerFlag::Right, 6);
	info_sizer.add_sizer(&actions_sizer, 0, SizerFlag::AlignCenterHorizontal | SizerFlag::All, 8);
	let close_btn = Button::builder(&info_panel).with_id(ID_CANCEL).with_label("Close").build();
	info_sizer.add(&close_btn, 0, SizerFlag::AlignCenterHorizontal | SizerFlag::Bottom, 8);
	// Made after everything else, so it's never what names the info box
	// (Windows names a box from the control just before it) and is never
	// in the way of Tab.
	// wx needs a picture to make it with; the real one comes later.
	let empty = wxdragon::bitmap::Bitmap::from_rgba(&[0, 0, 0, 0], 1, 1).expect("a 1x1 bitmap");
	let photo = StaticBitmap::builder(&info_panel).with_bitmap(Some(empty)).build();
	photo_row.add(&photo, 0, SizerFlag::AlignCenterVertical, 0);
	photo.show(false);

	info_panel.set_sizer(info_sizer, true);
	let details_sizer = BoxSizer::builder(Orientation::Vertical).build();
	details_sizer.add(&info_panel, 1, SizerFlag::Expand, 0);
	details_dialog.set_sizer(details_sizer, true);

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

	let common_groups_panel = Panel::builder(&notebook).build();
	theme::style_content(&common_groups_panel);
	let common_groups_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let common_groups_label = StaticText::builder(&common_groups_panel).with_label("Groups in common").build();
	theme::style_heading(&common_groups_label);
	common_groups_sizer.add(&common_groups_label, 0, SizerFlag::All, 6);
	let common_groups_list = ListBox::builder(&common_groups_panel).build();
	common_groups_list.set_font(&theme::Fonts::body());
	common_groups_sizer.add(&common_groups_list, 1, SizerFlag::Expand | SizerFlag::All, 6);
	common_groups_panel.set_sizer(common_groups_sizer, true);

	notebook.add_page(&media_panel, "Media", false, None);
	notebook.add_page(&documents_panel, "Documents", false, None);
	notebook.add_page(&links_panel, "Links", false, None);
	notebook.add_page(&common_groups_panel, "Groups in Common", false, None);

	main_sizer.add(&notebook, 1, SizerFlag::Expand | SizerFlag::All, 6);

	dialog.set_sizer(main_sizer, true);
	dialog.center();

	crate::ui::keys::escape_cancels_anywhere(dialog);
	crate::ui::keys::escape_cancels_anywhere(details_dialog);

	ConversationDataDialogParts {
		details_dialog,
		overview,
		dialog,
		back_btn,
		notebook,
		info_panel,
		info_field,
		voice_call_btn,
		video_call_btn,
		contact_btn,
		delete_contact_btn,
		chat_privacy_btn,
		photo,
		media_panel,
		media_label,
		media_list,
		documents_panel,
		documents_label,
		documents_list,
		links_panel,
		links_label,
		links_list,
		common_groups_panel,
		common_groups_label,
		common_groups_list,
	}
}

pub fn show_photo(photo: &StaticBitmap, panel: &Panel, path: &str) {
	let size = (photo.get_char_height() * 8).clamp(96, 320) as u32;
	let bitmap = (!path.is_empty()).then(|| crate::ui::owner_draw::round_picture(path, size)).flatten();
	match bitmap {
		Some(bitmap) => {
			photo.set_bitmap(&bitmap);
			photo.show(true);
		}
		None => photo.show(false),
	}
	panel.layout();
}
