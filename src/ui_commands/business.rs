//! Talking to businesses: the reply buttons, list menus and link / call /
//! copy-code buttons they send under a message. WhatsApp shows them as
//! buttons on the bubble; here Enter on the message asks the backend for them
//! (`get_message_options`) and they open as one list, each with the action it
//! takes. Picking a reply sends what WhatsApp Web sends for that tap.

use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::{
	AppState, UiCommand,
	ui::{keys, theme, window::WindowParts},
};

/// One button, as the backend lists it.
struct MessageOption {
	kind: String,
	label: String,
	section: String,
	description: String,
	value: String,
}

impl MessageOption {
	/// The row a screen reader reads: the button, then what it does.
	fn row(&self) -> String {
		let mut row = self.label.clone();
		if !self.description.is_empty() {
			row.push_str(", ");
			row.push_str(&self.description);
		}
		match self.kind.as_str() {
			"url" => row.push_str(", link"),
			"call" => row.push_str(&format!(", phone number {}", self.value)),
			"copy" => row.push_str(", copies a code"),
			"phone" => row.push_str(", only works on your phone"),
			_ => {}
		}
		row
	}

	/// What the action button says for this option.
	fn action(&self) -> &'static str {
		match self.kind.as_str() {
			"reply" => "&Send",
			"url" => "&Open Link",
			"call" => "&Copy Number",
			"copy" => "&Copy Code",
			_ => "&Send",
		}
	}
}

/// Enter on a business message: fetch its buttons.
pub(crate) fn open_message_options(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>, message_id: String) {
	let s = state.borrow();
	let Some(jid) = s.selected_chat.clone() else { return };
	let payload = serde_json::json!({
		"action": "get_message_options",
		"data": { "jid": jid, "message_id": message_id }
	});
	let _ = s.ws_tx.send(payload.to_string());
}

/// The backend's `message_options`: the list of buttons, one action button
/// that does what the selected one does, and Close.
pub(crate) fn show_message_options(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let text = |v: &serde_json::Value, key: &str| v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string();
	let (jid, message_id, heading) = (text(data, "jid"), text(data, "message_id"), text(data, "heading"));
	let options: Vec<MessageOption> = data
		.get("options")
		.and_then(|o| o.as_array())
		.into_iter()
		.flatten()
		.map(|o| MessageOption {
			kind: text(o, "kind"),
			label: text(o, "label"),
			section: text(o, "section"),
			description: text(o, "description"),
			value: text(o, "value"),
		})
		.collect();
	if options.is_empty() || state.borrow().selected_chat.as_deref() != Some(jid.as_str()) {
		return;
	}

	let dialog = Dialog::builder(&parts.frame, if heading.is_empty() { "Message buttons" } else { &heading })
		.with_size(460, 420)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	// A list's rows come in sections; each section's name goes in front of
	// its first row, as the list on the phone shows it as a heading.
	let mut rows = Vec::with_capacity(options.len());
	let mut last_section = String::new();
	for option in &options {
		let mut row = option.row();
		if !option.section.is_empty() && option.section != last_section {
			row = format!("{}: {row}", option.section);
			last_section = option.section.clone();
		}
		rows.push(row);
	}
	let count = match options.len() {
		1 => "1 option".to_string(),
		n => format!("{n} options"),
	};
	// Nothing between the label and the list: it names the list.
	let list_label = StaticText::builder(&panel)
		.with_label(&format!("{} ({count})", if heading.is_empty() { "Buttons" } else { &heading }))
		.build();
	theme::style_muted_label(&list_label);
	sizer.add(&list_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let list = ListBox::builder(&panel).build();
	list.set_font(&theme::Fonts::body());
	let row_refs: Vec<&str> = rows.iter().map(String::as_str).collect();
	list.set_items(&row_refs);
	list.set_selection(0, true);
	sizer.add(&list, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let buttons = BoxSizer::builder(Orientation::Horizontal).build();
	let action_btn = Button::builder(&panel).with_id(ID_OK).with_label(options[0].action()).build();
	theme::style_primary_button(&action_btn);
	action_btn.enable(options[0].kind != "phone");
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Close").build();
	buttons.add(&action_btn, 0, SizerFlag::Right, 8);
	buttons.add_stretch_spacer(1);
	buttons.add(&close_btn, 0, SizerFlag::empty(), 0);
	sizer.add_sizer(&buttons, 0, SizerFlag::Expand | SizerFlag::All, 10);
	// Spoken while the dialog is up (the main window's is behind it).
	let announcer = StaticText::builder(&panel).with_label("").build();
	live_region::set_live_region(&announcer);
	theme::style_muted_label(&announcer);
	sizer.add(&announcer, 0, SizerFlag::Left | SizerFlag::Bottom, 10);
	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let options = Rc::new(options);
	let opts_sel = options.clone();
	list.on_selection_changed(move |_| {
		if let Some(option) = list.get_selection().and_then(|i| opts_sel.get(i as usize)) {
			action_btn.set_label(option.action());
			action_btn.enable(option.kind != "phone");
		}
	});

	// Enter on the list, or the action button, does what the row says. A
	// reply closes the dialog; the others leave it open for another pick.
	let chosen: Rc<RefCell<Option<usize>>> = Rc::default();
	let act = {
		let (options, chosen, state) = (options.clone(), chosen.clone(), state.clone());
		move || {
			let Some(i) = list.get_selection().map(|i| i as usize) else { return };
			let Some(option) = options.get(i) else { return };
			match option.kind.as_str() {
				"reply" => {
					*chosen.borrow_mut() = Some(i);
					dialog.end_modal(ID_OK);
				}
				"url" => {
					if whatsnap::parse_whatsapp_link(&option.value).is_some() {
						let _ = state.borrow().ui_tx.send(UiCommand::OpenWhatsAppLink(option.value.clone()));
						dialog.end_modal(ID_CANCEL);
					} else {
						wxdragon::utils::launch_default_browser(
							&option.value,
							wxdragon::utils::BrowserLaunchFlags::Default,
						);
					}
				}
				"call" | "copy" => {
					let what = if option.kind == "call" { "Number" } else { "Code" };
					if crate::platform::clipboard::copy_text(&option.value) {
						crate::platform::announce::announce(announcer, &format!("{what} copied: {}", option.value));
					}
				}
				_ => {
					crate::platform::announce::announce(announcer, "This button only works in WhatsApp on your phone.");
				}
			}
		}
	};
	let act = Rc::new(act);
	let act_btn = act.clone();
	action_btn.on_click(move |_| act_btn());
	keys::on_enter(&list, Rc::default(), move || act());
	close_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));
	keys::escape_cancels_anywhere(dialog);

	list.set_focus();
	dialog.show_modal();
	dialog.destroy();

	let Some(i) = chosen.borrow_mut().take() else { return };
	let payload = serde_json::json!({
		"action": "send_message_option",
		"data": { "jid": jid, "message_id": message_id, "index": i }
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
	crate::platform::announce::announce(
		parts.conversations_parts.messages_announcer,
		&format!("Sending {}...", options[i].label),
	);
}

/// Enter on an order message: ask for what was ordered.
pub(crate) fn open_order_details(state: &Rc<RefCell<AppState>>, message_id: String) {
	let s = state.borrow();
	let Some(jid) = s.selected_chat.clone() else { return };
	let payload = serde_json::json!({ "action": "get_order", "data": { "jid": jid, "message_id": message_id } });
	let _ = s.ws_tx.send(payload.to_string());
}

/// Conversation > Business Catalogue, or Enter on a shared product.
pub(crate) fn open_catalog(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, message_id: Option<String>) {
	let s = state.borrow();
	let Some(jid) = s.selected_chat.clone() else { return };
	let payload = serde_json::json!({
		"action": "get_catalog",
		"data": { "jid": jid, "message_id": message_id.unwrap_or_default() }
	});
	let _ = s.ws_tx.send(payload.to_string());
	crate::platform::announce::announce(parts.conversations_parts.messages_announcer, "Loading catalogue...");
}

/// Read-only text in a field a screen reader can move through line by line,
/// with Close.
fn show_text(parts: &Rc<WindowParts>, title: &str, label: &str, text: &str) {
	let dialog = Dialog::builder(&parts.frame, title)
		.with_size(480, 400)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();
	let field_label = StaticText::builder(&panel).with_label(label).build();
	theme::style_muted_label(&field_label);
	sizer.add(&field_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let field = TextCtrl::builder(&panel)
		.with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly | TextCtrlStyle::Rich2)
		.build();
	field.set_font(&theme::Fonts::body());
	field.set_value(&text.replace("\r\n", "\n").replace('\n', "\r\n"));
	sizer.add(&field, 1, SizerFlag::Expand | SizerFlag::All, 10);
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Close").build();
	sizer.add(&close_btn, 0, SizerFlag::AlignRight | SizerFlag::Right | SizerFlag::Bottom, 10);
	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();
	close_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));
	keys::escape_cancels_anywhere(dialog);
	field.set_focus();
	field.set_insertion_point(0);
	dialog.show_modal();
	dialog.destroy();
}

/// The backend's `order_details`.
pub(crate) fn show_order_details(parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let title = data.get("title").and_then(|t| t.as_str()).unwrap_or("Order");
	let text = data.get("text").and_then(|t| t.as_str()).unwrap_or("");
	show_text(parts, title, "Order details", text);
}

/// One product, as the backend sends it.
struct Product {
	name: String,
	description: String,
	price: String,
	sale_price: String,
	sale_dates: String,
	availability: String,
	url: String,
	retailer_id: String,
	link: String,
	id: String,
}

impl Product {
	/// The list row: name, price, and what stands out.
	fn row(&self) -> String {
		let mut row = self.name.clone();
		if !self.sale_price.is_empty() {
			row.push_str(&format!(", {} on sale", self.sale_price));
			if !self.price.is_empty() {
				row.push_str(&format!(", was {}", self.price));
			}
		} else if !self.price.is_empty() {
			row.push_str(&format!(", {}", self.price));
		}
		if !self.availability.is_empty() {
			row.push_str(&format!(", {}", self.availability));
		}
		row
	}

	/// Everything about it, for the details field.
	fn details(&self) -> String {
		let mut lines = vec![self.name.clone()];
		if !self.price.is_empty() {
			lines.push(format!("Price: {}", self.price));
		}
		if !self.sale_price.is_empty() {
			let dates = if self.sale_dates.is_empty() { String::new() } else { format!(" ({})", self.sale_dates) };
			lines.push(format!("Sale price: {}{dates}", self.sale_price));
		}
		if !self.availability.is_empty() {
			lines.push(format!("Availability: {}", self.availability));
		}
		if !self.description.is_empty() {
			lines.push(String::new());
			lines.push(self.description.clone());
		}
		if !self.url.is_empty() || !self.retailer_id.is_empty() {
			lines.push(String::new());
		}
		if !self.url.is_empty() {
			lines.push(format!("Website: {}", self.url));
		}
		if !self.retailer_id.is_empty() {
			lines.push(format!("Item code: {}", self.retailer_id));
		}
		lines.join("\r\n")
	}
}

/// The backend's `catalog`: the products in a list, the selected one's
/// details in a read-only field under it, and what you can do with it.
pub(crate) fn show_catalog(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, data: &serde_json::Value) {
	let text = |v: &serde_json::Value, key: &str| v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string();
	let products: Vec<Product> = data
		.get("products")
		.and_then(|p| p.as_array())
		.into_iter()
		.flatten()
		.map(|p| Product {
			name: text(p, "name"),
			description: text(p, "description"),
			price: text(p, "price"),
			sale_price: text(p, "sale_price"),
			sale_dates: text(p, "sale_dates"),
			availability: text(p, "availability"),
			url: text(p, "url"),
			retailer_id: text(p, "retailer_id"),
			link: text(p, "link"),
			id: text(p, "id"),
		})
		.collect();
	if products.is_empty() {
		return;
	}
	let business = text(data, "jid");
	let select = text(data, "select");
	let more = data.get("more").and_then(|m| m.as_bool()).unwrap_or(false);
	let name = {
		let s = state.borrow();
		s.contacts.get(&business).cloned().filter(|n| !n.is_empty())
	};
	let title = match name {
		Some(name) => format!("{name}'s catalogue"),
		None => "Catalogue".to_string(),
	};

	let dialog = Dialog::builder(&parts.frame, &title)
		.with_size(560, 560)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let count =
		if more { format!("Products (first {})", products.len()) } else { format!("Products ({})", products.len()) };
	let list_label = StaticText::builder(&panel).with_label(&count).build();
	theme::style_muted_label(&list_label);
	sizer.add(&list_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let list = ListBox::builder(&panel).build();
	list.set_font(&theme::Fonts::body());
	let rows: Vec<String> = products.iter().map(Product::row).collect();
	let row_refs: Vec<&str> = rows.iter().map(String::as_str).collect();
	list.set_items(&row_refs);
	sizer.add(&list, 3, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let details_label = StaticText::builder(&panel).with_label("Product details").build();
	theme::style_muted_label(&details_label);
	sizer.add(&details_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let details = TextCtrl::builder(&panel)
		.with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly | TextCtrlStyle::Rich2)
		.build();
	details.set_font(&theme::Fonts::body());
	sizer.add(&details, 2, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let buttons = BoxSizer::builder(Orientation::Horizontal).build();
	let ask_btn = Button::builder(&panel).with_label("Put Link in &Message Box").build();
	theme::style_primary_button(&ask_btn);
	let copy_btn = Button::builder(&panel).with_label("&Copy Link").build();
	let site_btn = Button::builder(&panel).with_label("Open &Website").build();
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Close").build();
	buttons.add(&ask_btn, 0, SizerFlag::Right, 8);
	buttons.add(&copy_btn, 0, SizerFlag::Right, 8);
	buttons.add(&site_btn, 0, SizerFlag::Right, 8);
	buttons.add_stretch_spacer(1);
	buttons.add(&close_btn, 0, SizerFlag::empty(), 0);
	sizer.add_sizer(&buttons, 0, SizerFlag::Expand | SizerFlag::All, 10);
	let announcer = StaticText::builder(&panel).with_label("").build();
	live_region::set_live_region(&announcer);
	theme::style_muted_label(&announcer);
	sizer.add(&announcer, 0, SizerFlag::Left | SizerFlag::Bottom, 10);
	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let products = Rc::new(products);
	let find = {
		let products = products.clone();
		move |id: &str| products.iter().position(|p| p.id == id)
	};
	let show = {
		let products = products.clone();
		move || {
			let Some(p) = list.get_selection().and_then(|i| products.get(i as usize)) else { return };
			details.set_value(&p.details());
			details.set_insertion_point(0);
			ask_btn.enable(!p.link.is_empty());
			copy_btn.enable(!p.link.is_empty());
			site_btn.enable(!p.url.is_empty());
		}
	};
	list.set_selection(find(&select).unwrap_or(0) as u32, true);
	show();
	list.on_selection_changed(move |_| show());

	// The link is what WhatsApp shares a product as; in a message it shows
	// as the product, so the business knows what you're asking about.
	let link_of = {
		let products = products.clone();
		move || list.get_selection().and_then(|i| products.get(i as usize)).map(|p| p.link.clone())
	};
	let message_field = parts.conversations_parts.message_field;
	let link_ask = link_of.clone();
	ask_btn.on_click(move |_| {
		let Some(link) = link_ask().filter(|l| !l.is_empty()) else { return };
		let current = message_field.get_value();
		let joined =
			if current.trim().is_empty() { format!("{link} ") } else { format!("{} {link} ", current.trim_end()) };
		message_field.set_value(&joined);
		message_field.set_insertion_point_end();
		dialog.end_modal(ID_OK);
	});
	let link_copy = link_of.clone();
	copy_btn.on_click(move |_| {
		if let Some(link) = link_copy().filter(|l| !l.is_empty())
			&& crate::platform::clipboard::copy_text(&link)
		{
			crate::platform::announce::announce(announcer, "Link copied.");
		}
	});
	let products_site = products.clone();
	site_btn.on_click(move |_| {
		if let Some(p) = list.get_selection().and_then(|i| products_site.get(i as usize))
			&& !p.url.is_empty()
		{
			wxdragon::utils::launch_default_browser(&p.url, wxdragon::utils::BrowserLaunchFlags::Default);
		}
	});
	close_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));
	keys::escape_cancels_anywhere(dialog);

	list.set_focus();
	let asked = dialog.show_modal() == ID_OK;
	dialog.destroy();
	if asked {
		message_field.set_focus();
		crate::platform::announce::announce(
			parts.conversations_parts.messages_announcer,
			"Product link is in the message box. Add your question and send.",
		);
	}
}
