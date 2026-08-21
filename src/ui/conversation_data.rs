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

	// Media tab (photos and videos only, matching official WhatsApp)
	pub media_panel: Panel,
	pub media_list: ListBox,

	// Documents tab
	pub documents_panel: Panel,
	pub documents_list: ListBox,

	// Links tab
	pub links_panel: Panel,
	pub links_list: ListBox,
}

pub fn build_conversation_data_dialog(parent: &impl WxWidget) -> ConversationDataDialogParts {
	let dialog = Dialog::builder(parent, "Conversation Data").with_size(600, 500).build();
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

	let info_field =
		TextCtrl::builder(&info_panel).with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly).build();
	info_field.set_font(&theme::Fonts::body());
	info_sizer.add(&info_field, 1, SizerFlag::Expand | SizerFlag::All, 5);

	info_panel.set_sizer(info_sizer, true);

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

	// Add pages to notebook
	notebook.add_page(&info_panel, "Info", true, None);
	notebook.add_page(&media_panel, "Media", false, None);
	notebook.add_page(&documents_panel, "Documents", false, None);
	notebook.add_page(&links_panel, "Links", false, None);

	main_sizer.add(&notebook, 1, SizerFlag::Expand | SizerFlag::All, 5);

	dialog.set_sizer(main_sizer, true);

	let dlg_info = dialog.clone();
	info_field.on_key_down(move |event| {
		if let WindowEventData::Keyboard(ref key) = event {
			if key.get_key_code() == Some(27) {
				dlg_info.end_modal(ID_CANCEL);
				event.skip(false);
				return;
			}
		}
		event.skip(true);
	});

	ConversationDataDialogParts {
		dialog,
		back_btn,
		notebook,
		info_panel,
		info_field,
		media_panel,
		media_list,
		documents_panel,
		documents_list,
		links_panel,
		links_list,
	}
}
