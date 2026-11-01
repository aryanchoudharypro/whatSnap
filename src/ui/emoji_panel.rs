//! "Emoji and stickers": WhatsApp Web's emoji and sticker panel as a dialog
//! of native controls. Emoji are searched by name and listed by group;
//! stickers are the favorites, the ones recently seen in chats, and every
//! pack in WhatsApp's own sticker store, each read out by what it shows.
//! The backend answers while it's open (`stickers`, `sticker_store`,
//! `sticker_pack` events), so its parts live in AppState until it closes.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use wxdragon::prelude::*;

use crate::ui::theme;

/// Emoji names and groups, from Unicode's emoji-test.txt: a "#Group" line
/// starts each group, then one "emoji<TAB>name" line per emoji.
const EMOJI_DATA: &str = include_str!("emoji.txt");

/// How many emoji "Recently used" keeps.
const RECENT_EMOJI_LIMIT: usize = 24;

thread_local! {
	/// Emoji inserted this session, newest first.
	static RECENT_EMOJI: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

struct Emoji {
	group: usize,
	emoji: &'static str,
	name: &'static str,
}

fn parse_emoji() -> (Vec<&'static str>, Vec<Emoji>) {
	let mut groups = Vec::new();
	let mut emoji = Vec::new();
	for line in EMOJI_DATA.lines() {
		if let Some(group) = line.strip_prefix('#') {
			groups.push(group);
		} else if let Some((e, name)) = line.split_once('\t') {
			emoji.push(Emoji { group: groups.len().saturating_sub(1), emoji: e, name });
		}
	}
	(groups, emoji)
}

/// Whether every word of the search is in the emoji's name.
fn emoji_matches(name: &str, query: &str) -> bool {
	let name = name.to_lowercase();
	query.split_whitespace().all(|word| name.contains(word))
}

/// A sticker the panel can send.
#[derive(Clone)]
pub enum StickerPick {
	Favorite(String),
	Recent(String),
	Store { pack_id: String, hash: String },
}

#[derive(Clone)]
struct Sticker {
	label: String,
	pick: StickerPick,
}

struct Pack {
	id: String,
	label: String,
}

/// What the panel was closed with.
pub enum PanelChoice {
	Emoji(String),
	Sticker(StickerPick),
	StickerFile,
}

#[derive(Default)]
struct PanelState {
	favorites: Vec<Sticker>,
	recent: Vec<Sticker>,
	packs: Vec<Pack>,
	/// Store packs' stickers, by pack id, once loaded.
	pack_stickers: HashMap<String, Vec<Sticker>>,
	/// The stickers the list shows, in its order.
	shown: Vec<Sticker>,
	/// The emoji the list shows, as indexes into `emoji`.
	shown_emoji: Vec<usize>,
	/// "Recently used" leads the group list when there were any.
	has_recent_group: bool,
	stickers_loaded: bool,
	choice: Option<PanelChoice>,
}

/// The open panel.
pub struct EmojiPanel {
	pub dialog: Dialog,
	emoji_search: TextCtrl,
	emoji_group: ComboBox,
	emoji_list: ListBox,
	sticker_set: ComboBox,
	sticker_list: ListBox,
	announcer: StaticText,
	emoji: Vec<Emoji>,
	state: RefCell<PanelState>,
	ws_tx: std::sync::mpsc::Sender<String>,
}

/// The fixed sets ahead of the store's packs in the sticker set list.
const FAVORITES_SET: u32 = 0;
const RECENT_SET: u32 = 1;
const FIRST_PACK_SET: u32 = 2;

impl EmojiPanel {
	fn announce(&self, message: &str) {
		crate::platform::announce::announce(self.announcer, message);
	}

	fn send(&self, action: &str, data: serde_json::Value) {
		let payload = serde_json::json!({ "action": action, "data": data });
		let _ = self.ws_tx.send(payload.to_string());
	}

	/// Fills the emoji list from the search, or else the chosen group.
	fn show_emoji(&self) {
		let query = self.emoji_search.get_value().trim().to_lowercase();
		let mut s = self.state.borrow_mut();
		let recent_group = s.has_recent_group;
		s.shown_emoji = if !query.is_empty() {
			(0..self.emoji.len()).filter(|&i| emoji_matches(self.emoji[i].name, &query)).collect()
		} else {
			let group = self.emoji_group.get_selection().unwrap_or(0) as usize;
			if recent_group && group == 0 {
				RECENT_EMOJI.with(|recent| {
					recent.borrow().iter().filter_map(|r| self.emoji.iter().position(|e| e.emoji == r)).collect()
				})
			} else {
				let group = group - usize::from(recent_group);
				(0..self.emoji.len()).filter(|&i| self.emoji[i].group == group).collect()
			}
		};
		let labels: Vec<String> =
			s.shown_emoji.iter().map(|&i| format!("{} {}", self.emoji[i].emoji, self.emoji[i].name)).collect();
		drop(s);
		let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
		self.emoji_list.set_items(&label_refs);
		if !labels.is_empty() {
			self.emoji_list.set_selection(0, true);
		}
		if !query.is_empty() {
			self.announce(&match labels.len() {
				0 => "No emoji found.".to_string(),
				1 => format!("1 emoji: {}", labels[0]),
				n => format!("{} emoji.", n),
			});
		}
	}

	fn selected_emoji(&self) -> Option<&'static str> {
		let row = self.emoji_list.get_selection()? as usize;
		let i = *self.state.borrow().shown_emoji.get(row)?;
		Some(self.emoji[i].emoji)
	}

	fn pick_emoji(&self) {
		if let Some(emoji) = self.selected_emoji() {
			RECENT_EMOJI.with(|recent| {
				let mut recent = recent.borrow_mut();
				recent.retain(|r| r != emoji);
				recent.insert(0, emoji.to_string());
				recent.truncate(RECENT_EMOJI_LIMIT);
			});
			self.close_with(PanelChoice::Emoji(emoji.to_string()));
		}
	}

	fn close_with(&self, choice: PanelChoice) {
		self.state.borrow_mut().choice = Some(choice);
		self.dialog.end_modal(ID_OK);
	}

	/// The backend listed the favorite and recent stickers.
	pub fn apply_stickers(&self, data: &serde_json::Value) {
		let list = |key: &str| data.get(key).and_then(|v| v.as_array()).cloned().unwrap_or_default();
		let field = |v: &serde_json::Value, key: &str| v.get(key).and_then(|f| f.as_str()).unwrap_or("").to_string();
		let mut s = self.state.borrow_mut();
		s.favorites = list("favorites")
			.iter()
			.map(|f| Sticker { label: field(f, "label"), pick: StickerPick::Favorite(field(f, "hash")) })
			.collect();
		s.recent = list("recent")
			.iter()
			.map(|r| Sticker { label: field(r, "label"), pick: StickerPick::Recent(field(r, "message_id")) })
			.collect();
		let first_load = !s.stickers_loaded;
		s.stickers_loaded = true;
		let (no_favorites, no_recent) = (s.favorites.is_empty(), s.recent.is_empty());
		drop(s);
		// Open on stickers there are: the favorites, else the recent ones.
		if first_load && no_favorites && !no_recent {
			self.sticker_set.set_selection(RECENT_SET);
		}
		self.show_set(first_load);
	}

	/// The backend listed the store's packs.
	pub fn apply_store(&self, data: &serde_json::Value) {
		if let Some(error) = data.get("error").and_then(|e| e.as_str()) {
			self.announce(error);
			return;
		}
		let text = |v: &serde_json::Value, key: &str| v.get(key).and_then(|f| f.as_str()).unwrap_or("").to_string();
		let packs: Vec<Pack> = data
			.get("packs")
			.and_then(|p| p.as_array())
			.map(|packs| {
				packs
					.iter()
					.map(|p| {
						let mut label = text(p, "name");
						let publisher = text(p, "publisher");
						if !publisher.is_empty() {
							label = format!("{}, by {}", label, publisher);
						}
						if p.get("animated").and_then(|a| a.as_bool()).unwrap_or(false) {
							label.push_str(", animated");
						}
						Pack { id: text(p, "id"), label }
					})
					.filter(|p| !p.id.is_empty())
					.collect()
			})
			.unwrap_or_default();
		for pack in &packs {
			self.sticker_set.append(&pack.label);
		}
		self.state.borrow_mut().packs = packs;
	}

	/// The backend listed a store pack's stickers.
	pub fn apply_pack(&self, data: &serde_json::Value) {
		let id = data.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();
		if let Some(error) = data.get("error").and_then(|e| e.as_str()) {
			if self.shown_pack().as_deref() == Some(id.as_str()) {
				self.announce(error);
			}
			return;
		}
		let field = |v: &serde_json::Value, key: &str| v.get(key).and_then(|f| f.as_str()).unwrap_or("").to_string();
		let stickers: Vec<Sticker> = data
			.get("stickers")
			.and_then(|s| s.as_array())
			.map(|stickers| {
				stickers
					.iter()
					.map(|st| Sticker {
						label: field(st, "label"),
						pick: StickerPick::Store { pack_id: id.clone(), hash: field(st, "hash") },
					})
					.collect()
			})
			.unwrap_or_default();
		self.state.borrow_mut().pack_stickers.insert(id.clone(), stickers);
		if self.shown_pack().as_deref() == Some(id.as_str()) {
			self.show_set(true);
		}
	}

	/// The store pack the set list has chosen, if it's one.
	fn shown_pack(&self) -> Option<String> {
		let set = self.sticker_set.get_selection()?;
		let index = set.checked_sub(FIRST_PACK_SET)? as usize;
		self.state.borrow().packs.get(index).map(|p| p.id.clone())
	}

	/// Lists the chosen set's stickers, asking for a store pack's first.
	/// `say` reads out what's there, for a set the user just chose.
	fn show_set(&self, say: bool) {
		let set = self.sticker_set.get_selection().unwrap_or(FAVORITES_SET);
		let mut s = self.state.borrow_mut();
		let (stickers, empty) = match set {
			FAVORITES_SET => (Some(s.favorites.clone()), "No favorite stickers yet."),
			RECENT_SET => (Some(s.recent.clone()), "No recent stickers."),
			_ => {
				let pack = s.packs.get((set - FIRST_PACK_SET) as usize).map(|p| (p.id.clone(), p.label.clone()));
				match pack {
					Some((id, _)) if s.pack_stickers.contains_key(&id) => {
						(s.pack_stickers.get(&id).cloned(), "This pack has no stickers.")
					}
					Some((id, label)) => {
						s.shown.clear();
						drop(s);
						self.sticker_list.clear();
						self.send("get_sticker_pack", serde_json::json!({ "id": id }));
						self.announce(&format!("Loading {}...", label));
						return;
					}
					None => (None, ""),
				}
			}
		};
		let stickers = stickers.unwrap_or_default();
		let labels: Vec<String> = stickers.iter().map(|st| st.label.clone()).collect();
		s.shown = stickers;
		let loaded = s.stickers_loaded;
		drop(s);
		let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
		self.sticker_list.set_items(&label_refs);
		if !labels.is_empty() {
			self.sticker_list.set_selection(0, true);
		}
		if say && loaded {
			self.announce(&match labels.len() {
				0 => empty.to_string(),
				1 => "1 sticker.".to_string(),
				n => format!("{} stickers.", n),
			});
		}
	}

	fn pick_sticker(&self) {
		let Some(row) = self.sticker_list.get_selection() else { return };
		let pick = self.state.borrow().shown.get(row as usize).map(|st| st.pick.clone());
		if let Some(pick) = pick {
			self.close_with(PanelChoice::Sticker(pick));
		}
	}
}

/// Builds and shows the panel on its emoji or its stickers, and returns what
/// was picked once it's closed. `store` keeps it reachable for the
/// backend's answers while it's open.
pub fn show_emoji_panel(
	parent: &impl WxWidget,
	ws_tx: std::sync::mpsc::Sender<String>,
	stickers: bool,
	store: impl Fn(Option<Rc<EmojiPanel>>),
) -> Option<PanelChoice> {
	let dialog = Dialog::builder(parent, "Emoji and stickers")
		.with_size(480, 540)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	let notebook = Notebook::builder(&dialog).build();

	// --- Emoji ---
	let emoji_page = Panel::builder(&notebook).build();
	theme::style_content(&emoji_page);
	let emoji_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let search_label = StaticText::builder(&emoji_page).with_label("Search emoji").build();
	theme::style_muted_label(&search_label);
	emoji_sizer.add(&search_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let emoji_search = TextCtrl::builder(&emoji_page).with_style(TextCtrlStyle::ProcessEnter).build();
	emoji_search.set_font(&theme::Fonts::body());
	emoji_sizer.add(&emoji_search, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);
	let group_row = BoxSizer::builder(Orientation::Horizontal).build();
	let group_label = StaticText::builder(&emoji_page).with_label("Group:").build();
	let (groups, emoji) = parse_emoji();
	let has_recent_group = RECENT_EMOJI.with(|r| !r.borrow().is_empty());
	let mut group_names: Vec<&str> = Vec::new();
	if has_recent_group {
		group_names.push("Recently used");
	}
	group_names.extend(groups.iter().copied());
	let emoji_group =
		ComboBox::builder(&emoji_page).with_string_choices(&group_names).with_style(ComboBoxStyle::ReadOnly).build();
	emoji_group.set_font(&theme::Fonts::body());
	emoji_group.set_selection(0);
	group_row.add(&group_label, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 6);
	group_row.add(&emoji_group, 1, SizerFlag::empty(), 0);
	emoji_sizer.add_sizer(&group_row, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);
	let emoji_list = ListBox::builder(&emoji_page).build();
	emoji_list.set_font(&theme::Fonts::body());
	emoji_sizer.add(&emoji_list, 1, SizerFlag::Expand | SizerFlag::All, 10);
	let insert_btn = Button::builder(&emoji_page).with_label("&Insert emoji").build();
	emoji_sizer.add(&insert_btn, 0, SizerFlag::Left | SizerFlag::Bottom, 10);
	emoji_page.set_sizer(emoji_sizer, true);

	// --- Stickers ---
	let sticker_page = Panel::builder(&notebook).build();
	theme::style_content(&sticker_page);
	let sticker_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let set_row = BoxSizer::builder(Orientation::Horizontal).build();
	let set_label = StaticText::builder(&sticker_page).with_label("Stickers from:").build();
	let sticker_set = ComboBox::builder(&sticker_page)
		.with_string_choices(&["Favorites", "Recent"])
		.with_style(ComboBoxStyle::ReadOnly)
		.build();
	sticker_set.set_font(&theme::Fonts::body());
	sticker_set.set_selection(FAVORITES_SET);
	set_row.add(&set_label, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 6);
	set_row.add(&sticker_set, 1, SizerFlag::empty(), 0);
	sticker_sizer.add_sizer(&set_row, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);
	let sticker_list = ListBox::builder(&sticker_page).build();
	sticker_list.set_font(&theme::Fonts::body());
	sticker_sizer.add(&sticker_list, 1, SizerFlag::Expand | SizerFlag::All, 10);
	let sticker_buttons = BoxSizer::builder(Orientation::Horizontal).build();
	let send_btn = Button::builder(&sticker_page).with_label("&Send sticker").build();
	let file_btn = Button::builder(&sticker_page).with_label("Send a sticker &file (.webp)...").build();
	sticker_buttons.add(&send_btn, 0, SizerFlag::Right, 8);
	sticker_buttons.add(&file_btn, 0, SizerFlag::empty(), 0);
	sticker_sizer.add_sizer(&sticker_buttons, 0, SizerFlag::Left | SizerFlag::Bottom, 10);
	sticker_page.set_sizer(sticker_sizer, true);

	notebook.add_page(&emoji_page, "Emoji", !stickers, None);
	notebook.add_page(&sticker_page, "Stickers", stickers, None);
	outer.add(&notebook, 1, SizerFlag::Expand | SizerFlag::All, 5);

	let announcer = StaticText::builder(&dialog).with_label("").build();
	live_region::set_live_region(&announcer);
	theme::style_muted_label(&announcer);
	outer.add(&announcer, 0, SizerFlag::Left | SizerFlag::Right, 10);
	let close_btn = Button::builder(&dialog).with_id(ID_CANCEL).with_label("Close").build();
	outer.add(&close_btn, 0, SizerFlag::AlignRight | SizerFlag::All, 10);
	dialog.set_sizer(outer, true);
	dialog.center();

	let panel = Rc::new(EmojiPanel {
		dialog,
		emoji_search,
		emoji_group,
		emoji_list,
		sticker_set,
		sticker_list,
		announcer,
		emoji,
		state: RefCell::new(PanelState { has_recent_group, ..Default::default() }),
		ws_tx,
	});
	store(Some(panel.clone()));

	// Emoji: the list follows the search as it's typed (it's all local),
	// and the group when there's no search.
	let p = panel.clone();
	emoji_search.on_text_changed(move |_| p.show_emoji());
	let p = panel.clone();
	emoji_search.on_text_enter(move |_| p.pick_emoji());
	let p = panel.clone();
	emoji_group.on_selection_changed(move |_| {
		if !p.emoji_search.get_value().trim().is_empty() {
			p.emoji_search.set_value("");
		}
		p.show_emoji();
	});
	let p = panel.clone();
	insert_btn.on_click(move |_| p.pick_emoji());
	let p = panel.clone();
	emoji_list.on_item_double_clicked(move |_| p.pick_emoji());
	let p = panel.clone();
	crate::ui::keys::on_enter(&emoji_list, Rc::default(), move || p.pick_emoji());

	// Stickers: a store pack is only asked for once the choice has settled,
	// not for every pack arrowed past on the way.
	let settle = Rc::new(Timer::new(&dialog));
	let p = panel.clone();
	settle.on_tick(move |_| p.show_set(true));
	let settle_set = settle.clone();
	sticker_set.on_selection_changed(move |_| {
		settle_set.stop();
		settle_set.start(400, true);
	});
	let p = panel.clone();
	send_btn.on_click(move |_| p.pick_sticker());
	let p = panel.clone();
	sticker_list.on_item_double_clicked(move |_| p.pick_sticker());
	let p = panel.clone();
	crate::ui::keys::on_enter(&sticker_list, Rc::default(), move || p.pick_sticker());
	let p = panel.clone();
	file_btn.on_click(move |_| p.close_with(PanelChoice::StickerFile));
	close_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));
	crate::ui::keys::escape_cancels_anywhere(dialog);

	panel.show_emoji();
	panel.send("get_stickers", serde_json::Value::Null);
	panel.send("get_sticker_store", serde_json::Value::Null);
	if stickers {
		panel.sticker_list.set_focus();
	} else {
		panel.emoji_search.set_focus();
	}
	dialog.show_modal();
	store(None);
	let choice = panel.state.borrow_mut().choice.take();
	settle.stop();
	dialog.destroy();
	choice
}
