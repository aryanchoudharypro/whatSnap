use wxdragon::prelude::*;

use crate::ui::theme;

/// Every constructed widget is kept here for consistency, matching this
/// codebase's convention for dialog parts structs.
#[allow(dead_code)]
#[derive(Clone)]
pub struct ConversationDataDialogParts {
	pub dialog: Dialog,
	pub back_btn: Button,
	pub notebook: Notebook,

	// Info tab
	pub info_panel: Panel,
	pub info_field: TextCtrl,
	/// Hidden for a chat that can't be called, such as a broadcast list.
	pub voice_call_btn: Button,
	pub video_call_btn: Button,
	/// "Add contact" when the person isn't saved, "Edit contact" when they are.
	pub contact_btn: Button,
	/// Only shown while the person is saved.
	pub delete_contact_btn: Button,
	pub chat_privacy_btn: Button,
	/// The person's profile photo, big and round as on WhatsApp; hidden
	/// until it arrives (see `show_photo`).
	pub photo: StaticBitmap,

	// Media tab (photos and videos only, matching official WhatsApp)
	pub media_panel: Panel,
	pub media_label: StaticText,
	pub media_list: ListBox,

	// Documents tab
	pub documents_panel: Panel,
	pub documents_label: StaticText,
	pub documents_list: ListBox,

	// Links tab
	pub links_panel: Panel,
	pub links_label: StaticText,
	pub links_list: ListBox,

	// Groups in common tab
	pub common_groups_panel: Panel,
	pub common_groups_label: StaticText,
	pub common_groups_list: ListBox,
}

pub fn build_conversation_data_dialog(parent: &impl WxWidget) -> ConversationDataDialogParts {
	let dialog = Dialog::builder(parent, "Contact Info").with_size(600, 500).build();
	theme::style_content(&dialog);
	let main_sizer = BoxSizer::builder(Orientation::Vertical).build();

	// The back button comes first, both in tab order and on screen, matching
	// the old WhatsApp UWP app's pattern of a "Back" control at the top of a
	// detail screen rather than a "Close" button at the bottom.
	let back_btn = Button::builder(&dialog).with_id(ID_CANCEL).with_label("< Back").build();
	main_sizer.add(&back_btn, 0, SizerFlag::All, 5);

	let notebook = Notebook::builder(&dialog).build();

	// --- Info Tab ---
	let info_panel = Panel::builder(&notebook).build();
	let info_sizer = BoxSizer::builder(Orientation::Vertical).build();
	// The photo goes at the top, though it's made last (see below).
	let photo_row = BoxSizer::builder(Orientation::Horizontal).build();
	info_sizer.add_sizer(&photo_row, 0, SizerFlag::AlignCenterHorizontal | SizerFlag::Top, 10);

	let info_field =
		TextCtrl::builder(&info_panel).with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly).build();
	info_field.set_font(&theme::Fonts::body());
	info_sizer.add(&info_field, 1, SizerFlag::Expand | SizerFlag::All, 5);

	// What WhatsApp's contact info offers for the person and the chat.
	let actions_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let voice_call_btn = Button::builder(&info_panel).with_label("Voice call").build();
	actions_sizer.add(&voice_call_btn, 0, SizerFlag::Right, 5);
	let video_call_btn = Button::builder(&info_panel).with_label("Video call").build();
	actions_sizer.add(&video_call_btn, 0, SizerFlag::Right, 5);
	let contact_btn = Button::builder(&info_panel).with_label("Edit contact").build();
	actions_sizer.add(&contact_btn, 0, SizerFlag::Right, 5);
	let delete_contact_btn = Button::builder(&info_panel).with_label("Delete contact").build();
	actions_sizer.add(&delete_contact_btn, 0, SizerFlag::Right, 5);
	let chat_privacy_btn = Button::builder(&info_panel).with_label("Advanced chat privacy...").build();
	actions_sizer.add(&chat_privacy_btn, 0, SizerFlag::Right, 5);
	info_sizer.add_sizer(&actions_sizer, 0, SizerFlag::All, 5);
	// Made after everything else, so it's never what names the info box
	// (Windows names a box from the control just before it) and is never
	// in the way of Tab.
	let photo = StaticBitmap::builder(&info_panel).build();
	photo_row.add(&photo, 0, SizerFlag::AlignCenterVertical, 0);
	photo.show(false);

	info_panel.set_sizer(info_sizer, true);

	// --- Media Tab ---
	let media_panel = Panel::builder(&notebook).build();
	let media_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let media_label = StaticText::builder(&media_panel).with_label("Media").build();
	media_sizer.add(&media_label, 0, SizerFlag::All, 5);
	let media_list = ListBox::builder(&media_panel).build();
	media_sizer.add(&media_list, 1, SizerFlag::Expand | SizerFlag::All, 5);
	media_panel.set_sizer(media_sizer, true);

	// --- Documents Tab ---
	let documents_panel = Panel::builder(&notebook).build();
	let documents_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let documents_label = StaticText::builder(&documents_panel).with_label("Documents").build();
	documents_sizer.add(&documents_label, 0, SizerFlag::All, 5);
	let documents_list = ListBox::builder(&documents_panel).build();
	documents_sizer.add(&documents_list, 1, SizerFlag::Expand | SizerFlag::All, 5);
	documents_panel.set_sizer(documents_sizer, true);

	// --- Links Tab ---
	let links_panel = Panel::builder(&notebook).build();
	let links_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let links_label = StaticText::builder(&links_panel).with_label("Links").build();
	links_sizer.add(&links_label, 0, SizerFlag::All, 5);
	let links_list = ListBox::builder(&links_panel).build();
	links_sizer.add(&links_list, 1, SizerFlag::Expand | SizerFlag::All, 5);
	links_panel.set_sizer(links_sizer, true);

	// --- Groups in common Tab ---
	let common_groups_panel = Panel::builder(&notebook).build();
	let common_groups_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let common_groups_label = StaticText::builder(&common_groups_panel).with_label("Groups in common").build();
	common_groups_sizer.add(&common_groups_label, 0, SizerFlag::All, 5);
	let common_groups_list = ListBox::builder(&common_groups_panel).build();
	common_groups_sizer.add(&common_groups_list, 1, SizerFlag::Expand | SizerFlag::All, 5);
	common_groups_panel.set_sizer(common_groups_sizer, true);

	// Add pages to notebook
	notebook.add_page(&info_panel, "Info", true, None);
	notebook.add_page(&media_panel, "Media", false, None);
	notebook.add_page(&documents_panel, "Documents", false, None);
	notebook.add_page(&links_panel, "Links", false, None);
	notebook.add_page(&common_groups_panel, "Groups in Common", false, None);

	main_sizer.add(&notebook, 1, SizerFlag::Expand | SizerFlag::All, 5);

	dialog.set_sizer(main_sizer, true);

	let dlg_info = dialog;
	info_field.on_key_down(crate::ui::keys::escape_cancels(dlg_info));

	ConversationDataDialogParts {
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

/// Shows the picture at `path` (a profile photo) round in `photo`, `size`
/// pixels across, and lays out `panel` around it. An empty path hides it.
pub fn show_photo(photo: &StaticBitmap, panel: &Panel, path: &str) {
	let size = (photo.get_char_height() * 8).clamp(96, 320) as u32;
	let bitmap = (!path.is_empty())
		.then(|| crate::image_decode::decode_rgba_in_box(path, size, size).ok())
		.flatten()
		.and_then(|(w, h, mut rgba)| {
			// Cut to a circle, its edge smoothed.
			let (cx, cy, r) = (w as f32 / 2.0, h as f32 / 2.0, w.min(h) as f32 / 2.0);
			for y in 0..h {
				for x in 0..w {
					let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
					let cover = (r - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
					let a = &mut rgba[((y * w + x) * 4 + 3) as usize];
					*a = (*a as f32 * cover) as u8;
				}
			}
			wxdragon::bitmap::Bitmap::from_rgba(&rgba, w, h)
		});
	match bitmap {
		Some(bitmap) => {
			photo.set_bitmap(&bitmap);
			photo.show(true);
		}
		None => photo.show(false),
	}
	panel.layout();
}
