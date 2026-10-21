use wxdragon::prelude::*;

use crate::ui::theme;

/// Every constructed widget is kept here for consistency even if not all of
/// them are read back after construction yet.
#[allow(dead_code)]
pub struct NewConversationDialogParts {
	pub dialog: Dialog,
	pub search_label: StaticText,
	pub search_field: TextCtrl,
	pub results_list: ListBox,
	pub new_group_btn: Button,
	pub new_contact_btn: Button,
	pub message_meta_ai_btn: Button,
	pub close_btn: Button,
}

pub fn build_new_conversation_dialog(parent: &impl WxWidget) -> NewConversationDialogParts {
	let dialog = Dialog::builder(parent, "New Chat")
		.with_size(440, 480)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);

	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let search_label = StaticText::builder(&panel).with_label("Search name or number").build();
	theme::style_muted_label(&search_label);
	sizer.add(&search_label, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let search_field = TextCtrl::builder(&panel).with_style(TextCtrlStyle::ProcessEnter).build();
	search_field.set_font(&theme::Fonts::body());
	sizer.add(&search_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let results_list = ListBox::builder(&panel).build();
	results_list.set_font(&theme::Fonts::body());
	sizer.add(&results_list, 1, SizerFlag::Expand | SizerFlag::All, 10);

	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();

	let new_group_btn = Button::builder(&panel).with_label("New Group").build();
	new_group_btn.set_font(&theme::Fonts::body());
	btn_sizer.add(&new_group_btn, 0, SizerFlag::Right, 8);

	let new_contact_btn = Button::builder(&panel).with_label("New Contact").build();
	new_contact_btn.set_font(&theme::Fonts::body());
	btn_sizer.add(&new_contact_btn, 0, SizerFlag::Right, 8);

	let message_meta_ai_btn = Button::builder(&panel).with_label("Message Meta AI").build();
	message_meta_ai_btn.set_font(&theme::Fonts::body());
	btn_sizer.add(&message_meta_ai_btn, 0, SizerFlag::Right, 8);

	btn_sizer.add_stretch_spacer(1);

	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Close").build();
	btn_sizer.add(&close_btn, 0, SizerFlag::empty(), 0);

	sizer.add_sizer(&btn_sizer, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom, 10);

	panel.set_sizer(sizer, true);

	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);

	dialog.center();

	let dlg_esc1 = dialog;
	search_field.on_key_down(crate::ui::keys::escape_cancels(dlg_esc1));

	// Enter-to-select and Escape-to-cancel for `results_list` are bound in
	// `ui::window::bind_input_handlers`, which has access to the app state
	// needed to resolve a selected row back to a jid.

	NewConversationDialogParts {
		dialog,
		search_label,
		search_field,
		results_list,
		new_group_btn,
		new_contact_btn,
		message_meta_ai_btn,
		close_btn,
	}
}
