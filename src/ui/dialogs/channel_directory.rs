use std::{cell::RefCell, rc::Rc};

use wxdragon::prelude::*;

use crate::ui::{
	dialog_look::{self, Field},
	dialogs::channel_countries::{CATEGORIES, COUNTRIES},
	theme,
};

#[derive(Clone)]
struct Entry {
	jid: String,
	name: String,
	description: String,
	followers: u64,
	verified: bool,
	following: bool,
}

impl Entry {
	fn label(&self) -> String {
		let mut parts =
			vec![if self.name.is_empty() { crate::i18n::tr("Unnamed channel").to_string() } else { self.name.clone() }];
		if self.verified {
			parts.push(crate::i18n::tr("verified").to_string());
		}
		parts.push(match self.followers {
			1 => crate::i18n::tr("1 follower").to_string(),
			n => {
				let num = whantv_group_thousands(n);
				crate::i18n::tr_fmt("{} followers", &[&num])
			}
		});
		if self.following {
			parts.push(crate::i18n::tr("following").to_string());
		}
		if !self.description.is_empty() {
			let short: String = self.description.chars().take(120).collect();
			parts.push(short.replace('\n', " "));
		}
		parts.join(", ")
	}
}

fn whantv_group_thousands(n: u64) -> String {
	let s = n.to_string();
	let mut out = String::new();
	for (i, c) in s.chars().enumerate() {
		if i > 0 && (s.len() - i) % 3 == 0 {
			out.push(',');
		}
		out.push(c);
	}
	out
}

#[derive(Default)]
struct DirectoryState {
	entries: Vec<Entry>,
	query: String,
	next_cursor: String,
	loading: bool,
	similar_to: Option<String>,
	country: String,
	category: String,
}

pub struct ChannelDirectory {
	pub dialog: Dialog,
	list: ListBox,
	more_btn: Button,
	follow_btn: Button,
	country_combo: ComboBox,
	category_combo: ComboBox,
	announcer: StaticText,
	state: RefCell<DirectoryState>,
	ws_tx: std::sync::mpsc::Sender<String>,
}

impl ChannelDirectory {
	fn request(&self, query: &str, cursor: &str) {
		let mut s = self.state.borrow_mut();
		if s.loading {
			return;
		}
		s.loading = true;
		s.query = query.to_string();
		s.similar_to = None;
		let (country, category) = (s.country.clone(), s.category.clone());
		drop(s);
		let payload = serde_json::json!({
			"action": "channel_directory",
			"data": { "query": query, "cursor": cursor, "country": country, "category": category }
		});
		let _ = self.ws_tx.send(payload.to_string());
		let place = COUNTRIES
			.iter()
			.find(|(code, _)| *code == country)
			.map(|(_, name)| {
				let localized = crate::i18n::translate_country_name(name);
				crate::i18n::tr_fmt(" in {}", &[&localized])
			})
			.unwrap_or_default();
		let loading = if !query.is_empty() {
			crate::i18n::tr("Searching...").to_string()
		} else if let Some((_, name)) = CATEGORIES.iter().find(|(code, _)| *code == category) {
			crate::i18n::tr_fmt("Loading {} channels{}...", &[&crate::i18n::tr(name), &place])
		} else {
			crate::i18n::tr_fmt("Loading recommended channels{}...", &[&place])
		};
		crate::platform::announce::announce(self.announcer, &loading);
	}

	pub fn request_similar(&self, jid: &str, name: &str) {
		let mut s = self.state.borrow_mut();
		s.loading = true;
		s.query.clear();
		s.next_cursor.clear();
		s.similar_to = Some(name.to_string());
		drop(s);
		let payload = serde_json::json!({ "action": "get_similar_channels", "data": { "jid": jid } });
		let _ = self.ws_tx.send(payload.to_string());
		self.dialog.set_label(&crate::i18n::tr_fmt("Channels like {}", &[&name]));
		crate::platform::announce::announce(
			self.announcer,
			&crate::i18n::tr_fmt("Loading channels like {}...", &[&name]),
		);
	}

	pub fn apply(&self, data: &serde_json::Value) {
		let text = |v: &serde_json::Value, key: &str| v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string();
		let append = data.get("append").and_then(|a| a.as_bool()).unwrap_or(false);
		let mut s = self.state.borrow_mut();
		s.loading = false;
		let country = text(data, "country");
		if s.country.is_empty() && !country.is_empty() {
			s.country = country.clone();
			if let Some(idx) = COUNTRIES.iter().position(|(code, _)| *code == country) {
				self.country_combo.set_selection(idx as u32);
			}
		}
		let stale_filter = text(data, "category") != s.category
			|| (!s.country.is_empty() && !country.is_empty() && country != s.country);
		if text(data, "query") != s.query || (s.similar_to.is_none() && stale_filter) {
			return;
		}
		let page: Vec<Entry> = data
			.get("channels")
			.and_then(|c| c.as_array())
			.map(|channels| {
				channels
					.iter()
					.map(|c| Entry {
						jid: text(c, "jid"),
						name: text(c, "name"),
						description: text(c, "description"),
						followers: c.get("followers").and_then(|f| f.as_u64()).unwrap_or(0),
						verified: c.get("verified").and_then(|f| f.as_bool()).unwrap_or(false),
						following: c.get("following").and_then(|f| f.as_bool()).unwrap_or(false),
					})
					.filter(|e| !e.jid.is_empty())
					.collect()
			})
			.unwrap_or_default();
		s.next_cursor = text(data, "nextCursor");
		let first_new = if append { s.entries.len() } else { 0 };
		let labels: Vec<String> = page.iter().map(Entry::label).collect();
		let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
		if append {
			s.entries.extend(page);
			self.list.append_items(&label_refs);
		} else {
			s.entries = page;
			self.list.set_items(&label_refs);
		}
		self.more_btn.show(!s.next_cursor.is_empty());
		if !s.entries.is_empty() && first_new < s.entries.len() {
			self.list.set_selection(first_new as u32, true);
		}
		let found = labels.len();
		let message = match (found, append) {
			(0, false) if s.similar_to.is_some() => crate::i18n::tr("No similar channels found.").to_string(),
			(0, false) if s.query.is_empty() && !s.category.is_empty() => {
				crate::i18n::tr("No channels in this category.").to_string()
			}
			(0, false) if s.query.is_empty() => crate::i18n::tr("No recommended channels.").to_string(),
			(0, false) => crate::i18n::tr_fmt("No channels found for {}.", &[&s.query]),
			(0, true) => crate::i18n::tr("No more channels.").to_string(),
			(1, _) => crate::i18n::tr("1 channel.").to_string(),
			(n, true) => crate::i18n::tr_fmt("{} more channels.", &[&n]),
			(n, false) => crate::i18n::tr_fmt("{} channels.", &[&n]),
		};
		drop(s);
		self.selection_changed();
		self.dialog.layout();
		crate::platform::announce::announce(self.announcer, &message);
	}

	pub fn set_following(&self, jid: &str, following: bool) {
		let mut s = self.state.borrow_mut();
		if let Some(idx) = s.entries.iter().position(|e| e.jid == jid) {
			s.entries[idx].following = following;
			self.list.set_string(idx as u32, &s.entries[idx].label());
		}
		drop(s);
		self.selection_changed();
	}

	fn selected(&self) -> Option<Entry> {
		let idx = self.list.get_selection()? as usize;
		self.state.borrow().entries.get(idx).cloned()
	}

	fn selection_changed(&self) {
		let following = self.selected().is_some_and(|e| e.following);
		self.follow_btn.set_label(if following { crate::i18n::tr("Unfollow") } else { crate::i18n::tr("Follow") });
	}
}

pub fn show_channel_directory(
	parent: &impl WxWidget,
	ws_tx: std::sync::mpsc::Sender<String>,
	similar_to: Option<(String, String)>,
	store: impl Fn(Option<Rc<ChannelDirectory>>),
) {
	let dialog = Dialog::builder(parent, crate::i18n::tr("Find channels"))
		.with_size(520, 560)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let search_label =
		StaticText::builder(&panel).with_label(crate::i18n::tr("Search channels, then press Enter")).build();
	theme::style_muted_label(&search_label);
	sizer.add(&search_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let search_field =
		TextCtrl::builder(&panel).with_style(dialog_look::field_style(TextCtrlStyle::ProcessEnter)).build();
	dialog_look::add_field(&sizer, &search_field, Field::Search, 0);

	let filters = BoxSizer::builder(Orientation::Horizontal).build();
	let country_label = StaticText::builder(&panel).with_label(crate::i18n::tr("Country:")).build();
	let country_names: Vec<&str> =
		COUNTRIES.iter().map(|(_, name)| crate::i18n::translate_country_name(name)).collect();
	let country_combo =
		ComboBox::builder(&panel).with_string_choices(&country_names).with_style(ComboBoxStyle::ReadOnly).build();
	country_combo.set_font(&theme::Fonts::body());
	let category_label = StaticText::builder(&panel).with_label(crate::i18n::tr("Category:")).build();
	let mut category_names = vec![crate::i18n::tr("All (recommended)")];
	category_names.extend(CATEGORIES.iter().map(|(_, name)| crate::i18n::tr(name)));
	let category_combo =
		ComboBox::builder(&panel).with_string_choices(&category_names).with_style(ComboBoxStyle::ReadOnly).build();
	category_combo.set_font(&theme::Fonts::body());
	category_combo.set_selection(0);
	filters.add(&country_label, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 6);
	filters.add(&country_combo, 1, SizerFlag::Right, 12);
	filters.add(&category_label, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 6);
	filters.add(&category_combo, 1, SizerFlag::empty(), 0);
	sizer.add_sizer(&filters, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let list_label = StaticText::builder(&panel).with_label(crate::i18n::tr("Channels")).build();
	theme::style_muted_label(&list_label);
	sizer.add(&list_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let list = ListBox::builder(&panel).build();
	list.set_font(&theme::Fonts::body());
	sizer.add(&list, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let announcer = StaticText::builder(&panel).with_label("").build();
	live_region::set_live_region(&announcer);
	theme::style_muted_label(&announcer);
	sizer.add(&announcer, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let buttons = BoxSizer::builder(Orientation::Horizontal).build();
	let view_btn = Button::builder(&panel).with_label(crate::i18n::tr("View channel...")).build();
	let follow_btn = Button::builder(&panel).with_label(crate::i18n::tr("Follow")).build();
	let more_btn = Button::builder(&panel).with_label(crate::i18n::tr("More channels")).build();
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label(crate::i18n::tr("Close")).build();
	for (i, button) in [&view_btn, &follow_btn, &more_btn].into_iter().enumerate() {
		dialog_look::pill_button(button, i == 1, theme::Palette::CONTENT_BG());
		buttons.add(button, 0, SizerFlag::Right, 6);
	}
	dialog_look::pill_button(&close_btn, false, theme::Palette::CONTENT_BG());
	buttons.add_stretch_spacer(1);
	buttons.add(&close_btn, 0, SizerFlag::empty(), 0);
	sizer.add_sizer(&buttons, 0, SizerFlag::Expand | SizerFlag::All, 12);
	more_btn.show(false);
	panel.set_sizer(sizer, true);
	dialog_look::paint_fields(&panel, &[(search_field, Field::Search)]);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let directory = Rc::new(ChannelDirectory {
		dialog,
		list,
		more_btn,
		follow_btn,
		country_combo,
		category_combo,
		announcer,
		state: RefCell::new(DirectoryState::default()),
		ws_tx: ws_tx.clone(),
	});
	store(Some(directory.clone()));

	let dir_search = directory.clone();
	search_field.on_text_enter(move |_| {
		dir_search.dialog.set_label(crate::i18n::tr("Find channels"));
		dir_search.request(search_field.get_value().trim(), "");
	});
	let dir_more = directory.clone();
	more_btn.on_click(move |_| {
		let (query, cursor) = {
			let s = dir_more.state.borrow();
			(s.query.clone(), s.next_cursor.clone())
		};
		if !cursor.is_empty() {
			dir_more.request(&query, &cursor);
		}
	});
	let settle = Rc::new(Timer::new(&dialog));
	let dir_settle = directory.clone();
	settle.on_tick(move |_| {
		let country = dir_settle
			.country_combo
			.get_selection()
			.and_then(|i| COUNTRIES.get(i as usize))
			.map(|(code, _)| code.to_string());
		let category = dir_settle
			.category_combo
			.get_selection()
			.and_then(|i| (i as usize).checked_sub(1))
			.and_then(|i| CATEGORIES.get(i))
			.map(|(code, _)| code.to_string())
			.unwrap_or_default();
		{
			let mut s = dir_settle.state.borrow_mut();
			let country_changed = country.as_ref().is_some_and(|c| *c != s.country);
			if !country_changed && category == s.category {
				return;
			}
			if let Some(country) = country {
				s.country = country;
			}
			s.category = category;
			s.loading = false;
		}
		dir_settle.dialog.set_label(crate::i18n::tr("Find channels"));
		dir_settle.request("", "");
	});
	let settle_country = settle.clone();
	country_combo.on_selection_changed(move |_| {
		settle_country.stop();
		settle_country.start(700, true);
	});
	let settle_category = settle.clone();
	category_combo.on_selection_changed(move |_| {
		settle_category.stop();
		settle_category.start(700, true);
	});
	let dir_select = directory.clone();
	list.on_selection_changed(move |_| dir_select.selection_changed());

	let view_tx = ws_tx.clone();
	let dir_view = directory.clone();
	let view = Rc::new(move || {
		if let Some(entry) = dir_view.selected() {
			let payload = serde_json::json!({ "action": "get_channel_info", "data": { "jid": entry.jid } });
			let _ = view_tx.send(payload.to_string());
		}
	});
	let view_click = view.clone();
	view_btn.on_click(move |_| view_click());
	let view_double = view.clone();
	list.on_item_double_clicked(move |_| view_double());
	let view_enter = view.clone();
	crate::ui::keys::on_enter(&list, std::rc::Rc::default(), move || view_enter());

	let dir_follow = directory.clone();
	let follow_tx = ws_tx;
	follow_btn.on_click(move |_| {
		if let Some(entry) = dir_follow.selected() {
			let action = if entry.following { "unfollow_channel" } else { "follow_channel" };
			let payload = serde_json::json!({ "action": action, "data": { "jid": entry.jid } });
			let _ = follow_tx.send(payload.to_string());
		}
	});
	close_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));

	let escape_closes = crate::ui::keys::escape_cancels(dialog);
	search_field.on_key_down(escape_closes);
	list.on_key_down(escape_closes);
	country_combo.on_key_down(escape_closes);
	category_combo.on_key_down(escape_closes);

	match similar_to {
		Some((jid, name)) => directory.request_similar(&jid, &name),
		None => directory.request("", ""),
	}
	search_field.set_focus();
	dialog.show_modal();
	store(None);
	dialog.destroy();
}
