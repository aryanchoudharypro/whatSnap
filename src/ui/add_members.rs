use std::collections::HashSet;

use wxdragon::prelude::*;

use crate::ui::theme;

pub struct AddMembersState {
	pub group_jid: Option<String>,
	pub all_contact_labels: Vec<String>,
	pub all_contact_jids: Vec<String>,
	pub checked_jids: HashSet<String>,
	pub current_jids: Vec<String>,
}

pub struct AddMembersDialogParts {
	pub dialog: Dialog,
	pub search_field: TextCtrl,
	pub contacts_listbox: CheckListBox,
	pub selection_announcer: StaticText,
	pub add_btn: Button,
	pub cancel_btn: Button,
}

pub fn build_add_members_dialog(parent: &impl WxWidget) -> AddMembersDialogParts {
	let dialog = Dialog::builder(parent, "Add group members")
		.with_size(420, 520)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let search_label = StaticText::builder(&panel).with_label("Search contacts").build();
	theme::style_muted_label(&search_label);
	sizer.add(&search_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let search_field = TextCtrl::builder(&panel).build();
	search_field.set_font(&theme::Fonts::body());
	sizer.add(&search_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let contacts_listbox = CheckListBox::builder(&panel).build();
	contacts_listbox.set_font(&theme::Fonts::body());
	sizer.add(&contacts_listbox, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	// See the equivalent live region in new_group.rs: checking someone and
	// then narrowing the search filters them out of view (they're still
	// checked, just not shown), which needs a running total to confirm.
	let selection_announcer = StaticText::builder(&panel).with_label("").build();
	live_region::set_live_region(&selection_announcer);
	theme::style_muted_label(&selection_announcer);
	sizer.add(&selection_announcer, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let add_btn = Button::builder(&panel).with_id(ID_OK).with_label("Add").build();
	theme::style_primary_button(&add_btn);
	let cancel_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Cancel").build();
	btn_sizer.add(&add_btn, 0, SizerFlag::Right, 8);
	btn_sizer.add_stretch_spacer(1);
	btn_sizer.add(&cancel_btn, 0, SizerFlag::Right, 8);

	sizer.add_sizer(&btn_sizer, 0, SizerFlag::Expand | SizerFlag::All, 10);
	panel.set_sizer(sizer, true);

	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let dlg_esc1 = dialog;
	search_field.on_key_down(crate::ui::keys::escape_cancels(dlg_esc1));

	let dlg_esc2 = dialog;
	contacts_listbox.on_key_down(crate::ui::keys::escape_cancels(dlg_esc2));

	AddMembersDialogParts { dialog, search_field, contacts_listbox, selection_announcer, add_btn, cancel_btn }
}
