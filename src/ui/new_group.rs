use wxdragon::prelude::*;
use std::collections::HashSet;

pub struct NewGroupState {
	pub all_contact_labels: Vec<String>,
	pub all_contact_jids: Vec<String>,
	pub checked_jids: HashSet<String>,
	pub current_jids: Vec<String>,
}

pub struct NewGroupDialogParts {
	pub dialog: Dialog,
	pub name_field: TextCtrl,
	pub search_field: TextCtrl,
	pub contacts_listbox: CheckListBox,
	pub extra_number_field: TextCtrl,
	pub ok_btn: Button,
	pub cancel_btn: Button,
}

pub fn build_new_group_dialog(parent: &impl WxWidget) -> NewGroupDialogParts {
	let dialog = Dialog::builder(parent, "Novo grupo").with_size(460, 560).with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder).build();
	let panel = Panel::builder(&dialog).build();
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let name_label = StaticText::builder(&panel).with_label("Nome do grupo").build();
	sizer.add(&name_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let name_field = TextCtrl::builder(&panel).build();
	sizer.add(&name_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let contacts_label = StaticText::builder(&panel).with_label("Contatos").build();
	sizer.add(&contacts_label, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let search_label = StaticText::builder(&panel).with_label("Pesquisar contatos").build();
	sizer.add(&search_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let search_field = TextCtrl::builder(&panel).build();
	sizer.add(&search_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let contacts_listbox = CheckListBox::builder(&panel).build();
	sizer.add(&contacts_listbox, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let extra_label = StaticText::builder(&panel).with_label("Número extra (opcional)").build();
	sizer.add(&extra_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let extra_number_field = TextCtrl::builder(&panel).build();
	sizer.add(&extra_number_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let ok_btn = Button::builder(&panel).with_id(ID_OK).with_label("Criar grupo").build();
	let cancel_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Cancelar").build();
	btn_sizer.add(&ok_btn, 0, SizerFlag::Right, 8);
	btn_sizer.add_stretch_spacer(1);
	btn_sizer.add(&cancel_btn, 0, SizerFlag::Right, 8);

	sizer.add_sizer(&btn_sizer, 0, SizerFlag::Expand | SizerFlag::All, 10);
	panel.set_sizer(sizer, true);

	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	NewGroupDialogParts {
		dialog,
		name_field,
		search_field,
		contacts_listbox,
		extra_number_field,
		ok_btn,
		cancel_btn,
	}
}
