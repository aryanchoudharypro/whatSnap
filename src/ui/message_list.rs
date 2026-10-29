//! The open chat's message list: a plain panel that draws only the rows on
//! screen and describes itself to screen readers through AccessKit, instead of
//! a native list box.
//!
//! A native list box holds a copy of every row, and screen readers read a
//! virtual list view's rows through a 512-character buffer (NVDA) or not at all
//! (the MSAA proxy), cutting long messages off. This list keeps its rows in one
//! `Vec<String>` and tells the screen reader about the rows near the cursor
//! only, with each row's whole text as its name and its place in the whole chat
//! (`position_in_set` / `size_of_set`), so a 28,000-message chat still reads
//! "2 of 28000" while only a few hundred rows are described.
//!
//! The methods mirror the `ListBox` calls the rest of the app makes, with the
//! same behaviour: `set_selection` is silent (no `on_selection_changed`), and
//! inserting or deleting rows keeps the selection on the same row. Rows can
//! carry a key (the message id): `set_keyed_items` then keeps the cursor, and
//! the screen reader's object for each row, on the same message through a
//! rebuild, so NVDA doesn't read the cursor row again.
//!
//! It scrolls like a list box: a scrollbar with one step per row.
//!
//! The keyboard handling follows Fedra's timeline list (MIT, Quin Gillespie).

use std::{cell::RefCell, collections::HashMap, rc::Rc, time::Instant};

use accesskit::{Action, ActionHandler, ActionRequest, ActivationHandler, Node, NodeId, Role, TreeInfo, TreeUpdate};
use accesskit_windows::SubclassingAdapter;
use windows::Win32::Foundation::HWND;
use wxdragon::{prelude::*, widgets::scrolled_window::ScrollBarConfig};

const ROOT_ID: NodeId = NodeId(1);
/// Row ids start here; they're never reused, so a screen reader never mistakes
/// a new row for one it already knows.
const FIRST_ROW_ID: u64 = 16;
/// Rows described either side of the cursor. Moving within `MARGIN` of the
/// edge of what's described re-centres it.
const HALF_WINDOW: usize = 100;
const MARGIN: usize = 20;
/// Characters drawn per row. The screen reader still gets all of it.
const DRAWN_CHARS: usize = 400;
const TYPE_AHEAD_RESET_MS: u128 = 1000;
const WX_WANTS_CHARS: i64 = 0x0004_0000;

struct Row {
	id: u64,
	/// What `set_keyed_items` knows the row by; empty for none.
	key: String,
	text: String,
	/// Only in a checkable list (see `set_checkable`).
	checked: bool,
}

#[derive(Default)]
struct State {
	rows: Vec<Row>,
	next_id: u64,
	selected: Option<usize>,
	/// First row drawn: the scrollbar's position, which is in rows.
	top: usize,
	row_height: i32,
	/// Row count and height the scrollbar was last set up for.
	scrollbar: (usize, i32),
	/// The rows described to the screen reader.
	exposed: std::ops::Range<usize>,
	/// The label the screen reader has for the focused row, while its text has
	/// changed under it: a changed name on the focused item makes NVDA read it
	/// again, so the new text is only described once the cursor leaves it.
	frozen: Option<(u64, String)>,
	search: String,
	last_search: Option<Instant>,
	/// The chat's messages before and after the loaded rows, so a window of a
	/// big chat is numbered against all of it (see `set_numbering`).
	before: usize,
	after: usize,
	/// Rows have check boxes, as in a `CheckListBox` (see `set_checkable`).
	checkable: bool,
}

impl State {
	fn new_row(&mut self, key: &str, text: &str) -> Row {
		let id = self.next_id.max(FIRST_ROW_ID);
		self.next_id = id + 1;
		Row { id, key: key.to_string(), text: text.to_string(), checked: false }
	}

	/// Re-centres `exposed` on the cursor (or the end of the chat) when it has
	/// drifted near an edge, or the rows changed under it.
	fn expose(&mut self, force: bool) {
		let count = self.rows.len();
		let center = self.selected.unwrap_or(count.saturating_sub(1)).min(count.saturating_sub(1));
		let near_edge = |r: &std::ops::Range<usize>| {
			(r.start > 0 && center < r.start + MARGIN) || (r.end < count && center + MARGIN >= r.end)
		};
		if !force && self.exposed.end <= count && self.exposed.contains(&center) && !near_edge(&self.exposed) {
			return;
		}
		let start = center.saturating_sub(HALF_WINDOW);
		let end = (start + 2 * HALF_WINDOW).min(count);
		self.exposed = end.saturating_sub(2 * HALF_WINDOW).min(start)..end;
	}

	fn focus_id(&self) -> NodeId {
		self.selected.and_then(|i| self.rows.get(i)).map_or(ROOT_ID, |r| NodeId(r.id))
	}

	/// The whole described tree: the list and the rows around the cursor.
	/// Nodes that didn't change are cheap, as AccessKit only raises events for
	/// what differs from what it had.
	fn tree(&self, label: &str) -> TreeUpdate {
		let mut nodes = Vec::with_capacity(self.exposed.len() + 1);
		let mut children = Vec::with_capacity(self.exposed.len());
		for i in self.exposed.clone() {
			let Some(row) = self.rows.get(i) else { break };
			let mut node = Node::new(Role::ListBoxOption);
			let text = match &self.frozen {
				Some((id, text)) if *id == row.id => text.clone(),
				_ => row.text.clone(),
			};
			node.set_label(text);
			node.add_action(Action::Focus);
			node.add_action(Action::Click);
			node.set_position_in_set(self.before + i);
			if self.checkable {
				node.set_toggled(row.checked.into());
			}
			node.set_selected(self.selected == Some(i));
			children.push(NodeId(row.id));
			nodes.push((NodeId(row.id), node));
		}
		let mut root = Node::new(Role::ListBox);
		root.set_label(label.to_string());
		root.set_size_of_set(self.before + self.rows.len() + self.after);
		root.set_children(children);
		nodes.push((ROOT_ID, root));
		TreeUpdate { nodes, tree: None, focus: self.focus_id(), tree_id: accesskit::TreeId::ROOT }
	}

	/// Replaces the rows, giving a row whose key was already there the same
	/// id, and keeping the cursor and the top row on their keys. A cursor row
	/// whose text changed keeps its old text for the screen reader while the
	/// list has focus, as `set_string` does.
	fn replace_keyed(&mut self, keys: &[&str], texts: &[&str], has_focus: bool) {
		let old = std::mem::take(&mut self.rows);
		let key_at = |i: Option<usize>| i.and_then(|i| old.get(i)).map(|r| r.key.clone()).filter(|k| !k.is_empty());
		let (selected_key, top_key) = (key_at(self.selected), key_at(Some(self.top)));
		let mut kept: HashMap<String, (u64, String)> = HashMap::with_capacity(old.len());
		for row in old {
			if !row.key.is_empty() {
				kept.entry(row.key).or_insert((row.id, row.text));
			}
		}
		let mut rows = Vec::with_capacity(texts.len());
		let mut cursor_text = None;
		for (i, text) in texts.iter().enumerate() {
			let key = keys.get(i).copied().unwrap_or("");
			match kept.remove(key) {
				Some((id, old_text)) if !key.is_empty() => {
					if selected_key.as_deref() == Some(key) {
						cursor_text = Some((id, old_text));
					}
					rows.push(Row { id, key: key.to_string(), text: text.to_string(), checked: false });
				}
				_ => rows.push(self.new_row(key, text)),
			}
		}
		let position = |key: &Option<String>| key.as_ref().and_then(|k| rows.iter().position(|r| &r.key == k));
		self.selected = position(&selected_key);
		self.top = position(&top_key).unwrap_or(0);
		self.frozen = match (cursor_text, self.frozen.take()) {
			// Still frozen from an earlier change: the screen reader has the older text.
			(Some((id, _)), Some((fid, frozen))) if fid == id => Some((id, frozen)),
			(Some((id, old_text)), _) if has_focus && rows[self.selected.unwrap_or(0)].text != old_text => {
				Some((id, old_text))
			}
			_ => None,
		};
		self.rows = rows;
		self.expose(true);
	}

	fn index_of(&self, id: u64) -> Option<usize> {
		// A screen reader can only point at a described row, so look there first.
		self.rows[self.exposed.start.min(self.rows.len())..self.exposed.end.min(self.rows.len())]
			.iter()
			.position(|r| r.id == id)
			.map(|i| i + self.exposed.start)
			.or_else(|| self.rows.iter().position(|r| r.id == id))
	}
}

type SelectionCallback = Rc<dyn Fn(u32)>;

struct Shared {
	panel: ScrolledWindow,
	label: String,
	state: RefCell<State>,
	adapter: RefCell<Option<SubclassingAdapter>>,
	on_selection_changed: RefCell<Option<SelectionCallback>>,
	on_toggled: RefCell<Option<SelectionCallback>>,
}

thread_local! {
	static LISTS: RefCell<Vec<Rc<Shared>>> = const { RefCell::new(Vec::new()) };
}

struct Activation {
	shared: Rc<Shared>,
}

impl ActivationHandler for Activation {
	fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
		let state = self.shared.state.try_borrow().ok()?;
		let mut update = state.tree(&self.shared.label);
		update.tree = Some(TreeInfo::new(ROOT_ID));
		Some(update)
	}
}

/// A screen reader asking to move to a row (NVDA's object navigation, say).
/// It may come from another thread, so it's handed to the UI thread.
struct Actions {
	slot: usize,
}

impl ActionHandler for Actions {
	fn do_action(&mut self, request: ActionRequest) {
		if !matches!(request.action, Action::Focus | Action::Click) {
			return;
		}
		let slot = self.slot;
		let id = request.target_node.0;
		let click = request.action == Action::Click;
		wxdragon::call_after(Box::new(move || {
			if let Some(list) = MessageList::from_slot(slot) {
				list.select_by_user_id(id);
				// A screen reader's default action on a check box row checks it.
				if click && let Some(index) = list.get_selection() {
					list.toggle_by_user(index as usize);
				}
			}
		}));
	}
}

/// See the module docs. Copy, like the wx widgets it stands in for.
#[derive(Clone, Copy)]
pub struct MessageList {
	panel: ScrolledWindow,
	slot: usize,
}

impl WxWidget for MessageList {
	fn handle_ptr(&self) -> *mut wxdragon::ffi::wxd_Window_t {
		self.panel.handle_ptr()
	}
}

impl std::ops::Deref for MessageList {
	type Target = ScrolledWindow;

	fn deref(&self) -> &ScrolledWindow {
		&self.panel
	}
}

impl MessageList {
	/// Builds the list. Must run before its window is first shown (AccessKit
	/// hooks the window then). `label` is what a screen reader calls the list.
	pub fn new(parent: &impl WxWidget, label: &str) -> Self {
		let panel = ScrolledWindow::builder(parent).with_style(ScrolledWindowStyle::VScroll).build();
		panel.set_name(label);
		// Arrows, Tab and Enter come to the list rather than moving between controls.
		unsafe {
			let ptr = panel.handle_ptr();
			wxdragon::ffi::wxd_Window_SetWindowStyle(
				ptr,
				wxdragon::ffi::wxd_Window_GetWindowStyle(ptr) | WX_WANTS_CHARS,
			);
		}
		// A tab stop, so Windows' Tab order includes it like the list box it replaces.
		unsafe {
			use windows::Win32::UI::WindowsAndMessaging::{
				GWL_STYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_TABSTOP,
			};
			let hwnd = HWND(panel.get_handle().cast());
			SetWindowLongPtrW(hwnd, GWL_STYLE, GetWindowLongPtrW(hwnd, GWL_STYLE) | WS_TABSTOP.0 as isize);
		}
		panel.set_background_style(BackgroundStyle::Paint);
		// A list box's colours; the theme may set its own after.
		panel.set_background_color(SystemSettings::get_colour(SystemColour::Window));
		panel.set_foreground_color(SystemSettings::get_colour(SystemColour::WindowText));
		let shared = Rc::new(Shared {
			panel,
			label: label.to_string(),
			state: RefCell::new(State {
				next_id: FIRST_ROW_ID,
				row_height: panel.get_char_height().max(1) + 6,
				..Default::default()
			}),
			adapter: RefCell::new(None),
			on_selection_changed: RefCell::new(None),
			on_toggled: RefCell::new(None),
		});
		let slot = LISTS.with(|l| {
			let mut l = l.borrow_mut();
			l.push(shared.clone());
			l.len() - 1
		});
		let adapter = SubclassingAdapter::new(
			HWND(panel.get_handle().cast()),
			Activation { shared: shared.clone() },
			Actions { slot },
		);
		*shared.adapter.borrow_mut() = Some(adapter);
		let list = Self { panel, slot };
		list.bind_events();
		list
	}

	fn from_slot(slot: usize) -> Option<Self> {
		LISTS.with(|l| l.borrow().get(slot).map(|shared| Self { panel: shared.panel, slot }))
	}

	fn shared(&self) -> Rc<Shared> {
		LISTS.with(|l| l.borrow()[self.slot].clone())
	}

	// --- The ListBox-shaped API the app uses ---

	pub fn get_count(&self) -> u32 {
		self.shared().state.borrow().rows.len() as u32
	}

	pub fn get_selection(&self) -> Option<u32> {
		self.shared().state.borrow().selected.map(|i| i as u32)
	}

	pub fn get_string(&self, index: u32) -> Option<String> {
		self.shared().state.borrow().rows.get(index as usize).map(|r| r.text.clone())
	}

	/// Selects (or, with `select` false, deselects) a row without firing
	/// `on_selection_changed`, as `ListBox::set_selection` does. The screen
	/// reader still follows it.
	pub fn set_selection(&self, index: u32, select: bool) {
		let shared = self.shared();
		{
			let mut state = shared.state.borrow_mut();
			let index = index as usize;
			if select {
				if index >= state.rows.len() {
					return;
				}
				state.selected = Some(index);
			} else if state.selected == Some(index) {
				state.selected = None;
			} else {
				return;
			}
		}
		self.after_cursor_move(&shared);
	}

	pub fn set_string(&self, index: u32, text: &str) {
		let shared = self.shared();
		{
			let mut state = shared.state.borrow_mut();
			let index = index as usize;
			let selected = state.selected == Some(index);
			let Some(row) = state.rows.get_mut(index) else { return };
			if row.text == text {
				return;
			}
			let (id, old) = (row.id, std::mem::replace(&mut row.text, text.to_string()));
			if selected && self.panel.has_focus() && state.frozen.as_ref().is_none_or(|(fid, _)| *fid != id) {
				state.frozen = Some((id, old));
			}
		}
		self.sync(&shared);
		self.panel.refresh(false, None);
	}

	pub fn append(&self, text: &str) {
		self.insert_items(&[text], self.get_count());
	}

	pub fn append_keyed(&self, key: &str, text: &str) {
		self.insert_keyed_items(&[key], &[text], self.get_count());
	}

	pub fn append_items(&self, texts: &[&str]) {
		self.insert_items(texts, self.get_count());
	}

	pub fn insert(&self, text: &str, pos: usize) {
		self.insert_items(&[text], pos as u32);
	}

	/// Inserts rows before `pos`; the selection stays on the row it was on.
	pub fn insert_items(&self, texts: &[&str], pos: u32) {
		self.insert_keyed_items(&[], texts, pos);
	}

	/// `insert_items`, giving each row the key at the same place in `keys`
	/// (see `set_keyed_items`).
	pub fn insert_keyed_items(&self, keys: &[&str], texts: &[&str], pos: u32) {
		if texts.is_empty() {
			return;
		}
		let shared = self.shared();
		{
			let mut state = shared.state.borrow_mut();
			state.top = self.view_top();
			let pos = (pos as usize).min(state.rows.len());
			let new: Vec<Row> =
				texts.iter().enumerate().map(|(i, t)| state.new_row(keys.get(i).copied().unwrap_or(""), t)).collect();
			state.rows.splice(pos..pos, new);
			let n = texts.len();
			if let Some(sel) = state.selected.filter(|&s| s >= pos) {
				state.selected = Some(sel + n);
			}
			if state.top >= pos && pos > 0 {
				state.top += n;
			}
			state.expose(true);
		}
		self.apply_view(&shared);
		self.sync(&shared);
		self.panel.refresh(false, None);
	}

	/// Removes a row; deleting the selected row leaves nothing selected.
	pub fn delete(&self, index: u32) {
		let shared = self.shared();
		{
			let mut state = shared.state.borrow_mut();
			let index = index as usize;
			if index >= state.rows.len() {
				return;
			}
			state.top = self.view_top();
			state.rows.remove(index);
			state.selected = match state.selected {
				Some(s) if s == index => None,
				Some(s) if s > index => Some(s - 1),
				other => other,
			};
			if state.top > index {
				state.top -= 1;
			}
			state.top = state.top.min(state.rows.len().saturating_sub(1));
			state.expose(true);
		}
		self.apply_view(&shared);
		self.sync(&shared);
		self.panel.refresh(false, None);
	}

	/// Replaces every row and clears the selection, as `ListBox::set_items`.
	pub fn set_items(&self, texts: &[&str]) {
		let shared = self.shared();
		{
			let mut state = shared.state.borrow_mut();
			let rows: Vec<Row> = texts.iter().map(|t| state.new_row("", t)).collect();
			state.rows = rows;
			state.selected = None;
			state.frozen = None;
			state.top = 0;
			state.expose(true);
		}
		self.apply_view(&shared);
		self.sync(&shared);
		self.panel.refresh(false, None);
	}

	/// Replaces every row, each with the key at the same place in `keys` (a
	/// message id; empty for none). Unlike `set_items`, the selection stays on
	/// the row with the same key, if it's still there, and so does the top of
	/// the view; a row that keeps its key stays the same object to the screen
	/// reader, so the cursor row isn't read out again.
	pub fn set_keyed_items(&self, keys: &[&str], texts: &[&str]) {
		let shared = self.shared();
		{
			let mut state = shared.state.borrow_mut();
			state.top = self.view_top();
			state.replace_keyed(keys, texts, self.panel.has_focus());
			self.scroll_into_view(&mut state);
		}
		self.apply_view(&shared);
		self.sync(&shared);
		self.panel.refresh(false, None);
	}

	pub fn clear(&self) {
		let shared = self.shared();
		{
			let mut state = shared.state.borrow_mut();
			// Give the memory back: a big chat's rows are megabytes.
			state.rows = Vec::new();
			state.selected = None;
			state.frozen = None;
			state.top = 0;
			state.exposed = 0..0;
			state.before = 0;
			state.after = 0;
		}
		self.apply_view(&shared);
		self.sync(&shared);
		self.panel.refresh(false, None);
	}

	/// Numbers the rows as if `before` more came above them and `after` below:
	/// the chat's messages that aren't loaded, so the screen reader says "2 of
	/// 28000" for a window of a big chat.
	pub fn set_numbering(&self, before: usize, after: usize) {
		let shared = self.shared();
		{
			let mut state = shared.state.borrow_mut();
			if (state.before, state.after) == (before, after) {
				return;
			}
			state.before = before;
			state.after = after;
		}
		self.sync(&shared);
	}

	/// Gives every row a check box, as a `CheckListBox`: Space (or a screen
	/// reader's default action) checks the cursor row, and screen readers say
	/// "checked" / "not checked".
	pub fn set_checkable(&self, checkable: bool) {
		let shared = self.shared();
		shared.state.borrow_mut().checkable = checkable;
		self.sync(&shared);
		self.panel.refresh(false, None);
	}

	pub fn is_checked(&self, index: u32) -> bool {
		self.shared().state.borrow().rows.get(index as usize).is_some_and(|r| r.checked)
	}

	/// Checks or unchecks a row without firing `on_toggled`.
	pub fn check(&self, index: u32, checked: bool) {
		let shared = self.shared();
		{
			let mut state = shared.state.borrow_mut();
			let Some(row) = state.rows.get_mut(index as usize) else { return };
			if row.checked == checked {
				return;
			}
			row.checked = checked;
		}
		self.sync(&shared);
		self.panel.refresh(false, None);
	}

	/// Runs `callback` with the row whenever the user checks or unchecks it.
	pub fn on_toggled(&self, callback: impl Fn(u32) + 'static) {
		*self.shared().on_toggled.borrow_mut() = Some(Rc::new(callback));
	}

	/// Runs `callback` with the new row whenever the user moves the cursor
	/// (keys, mouse, a screen reader), not when the app calls `set_selection`.
	pub fn on_selection_changed(&self, callback: impl Fn(u32) + 'static) {
		*self.shared().on_selection_changed.borrow_mut() = Some(Rc::new(callback));
	}

	// --- Internals ---

	/// Hands the screen reader the current tree, if one is listening.
	fn sync(&self, shared: &Shared) {
		let events = shared
			.adapter
			.borrow_mut()
			.as_mut()
			.and_then(|a| a.update_if_active(|| shared.state.borrow().tree(&shared.label)));
		if let Some(events) = events {
			events.raise();
		}
	}

	/// Follows a cursor move: re-centres what's described, scrolls it into
	/// view, and tells the screen reader. A row whose text changed while it
	/// had the cursor gets its new text in a second update, once the cursor
	/// has left it, so NVDA doesn't read the row it's leaving again.
	fn after_cursor_move(&self, shared: &Shared) {
		let thawed = {
			let mut state = shared.state.borrow_mut();
			state.expose(false);
			state.top = self.view_top();
			self.scroll_into_view(&mut state);
			let focus = state.focus_id();
			state.frozen.as_ref().is_some_and(|(id, _)| NodeId(*id) != focus)
		};
		self.apply_view(shared);
		self.sync(shared);
		if thawed {
			shared.state.borrow_mut().frozen = None;
			self.sync(shared);
		}
		self.panel.refresh(false, None);
	}

	fn visible_rows(&self, state: &State) -> usize {
		let height = self.panel.get_client_size().height;
		(height / state.row_height.max(1)).max(1) as usize
	}

	/// The top row as the scrollbar has it (the user may have scrolled).
	fn view_top(&self) -> usize {
		self.panel.get_view_start().1.max(0) as usize
	}

	/// Sets the scrollbar up for the rows (one unit per row) and moves it to
	/// `state.top`. Runs outside any borrow of the state: scrolling can paint.
	fn apply_view(&self, shared: &Shared) {
		let (len, row_height, top, changed) = {
			let mut state = shared.state.borrow_mut();
			let shape = (state.rows.len(), state.row_height.max(1));
			let changed = state.scrollbar != shape;
			state.scrollbar = shape;
			state.top = state.top.min(shape.0.saturating_sub(1));
			(shape.0, shape.1, state.top, changed)
		};
		let top = top as i32;
		if changed {
			self.panel.set_scrollbars(ScrollBarConfig {
				pixels_per_unit_x: 0,
				pixels_per_unit_y: row_height,
				no_units_x: 0,
				no_units_y: len.min(i32::MAX as usize) as i32,
				x_pos: 0,
				y_pos: top,
				no_refresh: true,
			});
		}
		if self.panel.get_view_start().1 != top {
			self.panel.scroll_coords(-1, top);
		}
	}

	fn scroll_into_view(&self, state: &mut State) {
		let Some(sel) = state.selected else { return };
		let visible = self.visible_rows(state);
		if sel < state.top {
			state.top = sel;
		} else if sel >= state.top + visible {
			state.top = sel + 1 - visible;
		}
	}

	/// A cursor move the user made: like `set_selection`, then tells the app.
	fn select_by_user(&self, index: usize) {
		let shared = self.shared();
		{
			let mut state = shared.state.borrow_mut();
			if index >= state.rows.len() || state.selected == Some(index) {
				return;
			}
			state.selected = Some(index);
		}
		self.after_cursor_move(&shared);
		let callback = shared.on_selection_changed.borrow().clone();
		if let Some(callback) = callback {
			callback(index as u32);
		}
	}

	/// A check the user made: flips the row, then tells the app.
	fn toggle_by_user(&self, index: usize) {
		let shared = self.shared();
		{
			let mut state = shared.state.borrow_mut();
			if !state.checkable {
				return;
			}
			let Some(row) = state.rows.get_mut(index) else { return };
			row.checked = !row.checked;
		}
		self.sync(&shared);
		self.panel.refresh(false, None);
		let callback = shared.on_toggled.borrow().clone();
		if let Some(callback) = callback {
			callback(index as u32);
		}
	}

	fn select_by_user_id(&self, id: u64) {
		let index = self.shared().state.borrow().index_of(id);
		if let Some(index) = index {
			self.select_by_user(index);
		}
	}

	fn bind_events(&self) {
		let list = *self;
		self.panel.on_paint(move |_| list.paint());
		let list = *self;
		self.panel.on_size(move |event| {
			let shared = list.shared();
			{
				let mut state = shared.state.borrow_mut();
				state.top = list.view_top();
				list.scroll_into_view(&mut state);
			}
			list.apply_view(&shared);
			list.panel.refresh(false, None);
			event.skip(true);
		});
		// The cursor row is drawn differently with and without focus.
		let list = *self;
		self.panel.on_set_focus(move |event| {
			list.panel.refresh(false, None);
			event.skip(true);
		});
		// Leaving the list, a row that changed while it had the cursor gets
		// its new text, so coming back reads what's there now.
		let list = *self;
		self.panel.on_kill_focus(move |event| {
			let shared = list.shared();
			if shared.state.borrow_mut().frozen.take().is_some() {
				list.sync(&shared);
			}
			list.panel.refresh(false, None);
			event.skip(true);
		});
		// Tab is taken on key down: the app's own key-down handler (bound later,
		// so it runs first) lets it through, and acting here doesn't depend on
		// wx turning it into a char event.
		let list = *self;
		self.panel.on_key_down(move |event| {
			if let WindowEventData::Keyboard(ref key) = event
				&& key.get_key_code() == Some(WXK_TAB)
				&& !key.control_down()
				&& !key.alt_down()
			{
				list.tab_away(key.shift_down());
				event.skip(false);
				return;
			}
			event.skip(true);
		});
		// Moving keys come as char events, which wx only sends for a key the
		// app's own key-down handler let through - so its Home, End and
		// shortcuts win, as they did over the list box.
		let list = *self;
		self.panel.on_char(move |event| {
			let WindowEventData::Keyboard(ref key) = event else { return };
			// Moving keys the list leaves alone (with Ctrl, say) mustn't reach
			// the scrolled window either, which would scroll away from the cursor.
			let moving = key.get_key_code().is_some_and(|code| {
				matches!(
					code,
					WXK_UP
						| WXK_DOWN
						| WXK_PAGEUP
						| WXK_PAGEDOWN
						| WXK_HOME
						| WXK_END
						| WXK_LEFT
						| WXK_RIGHT
						| WXK_NUMPAD_UP
						| WXK_NUMPAD_DOWN
						| WXK_NUMPAD_PAGEUP
						| WXK_NUMPAD_PAGEDOWN
						| WXK_NUMPAD_HOME
						| WXK_NUMPAD_END
						| WXK_NUMPAD_LEFT
						| WXK_NUMPAD_RIGHT
				)
			});
			let handled = list.handle_char(key) || moving;
			event.skip(!handled);
		});
		let list = *self;
		self.panel.on_mouse_left_down(move |event| {
			if let WindowEventData::MouseButton(ref mouse) = event
				&& let Some(pos) = mouse.get_position()
			{
				list.panel.set_focus();
				if let Some(index) = list.row_at(pos.y) {
					list.select_by_user(index);
				}
			}
			event.skip(true);
		});
		// A right click picks its row first, as in a list box; the context
		// menu itself comes from the window's WM_CONTEXTMENU.
		let list = *self;
		self.panel.on_mouse_right_down(move |event| {
			if let WindowEventData::MouseButton(ref mouse) = event
				&& let Some(pos) = mouse.get_position()
				&& let Some(index) = list.row_at(pos.y)
			{
				list.panel.set_focus();
				list.select_by_user(index);
			}
			event.skip(true);
		});
		// The mouse wheel and the scrollbar are the scrolled window's own.
	}

	fn row_at(&self, y: i32) -> Option<usize> {
		let shared = self.shared();
		let state = shared.state.borrow();
		let index = self.view_top() + (y.max(0) / state.row_height.max(1)) as usize;
		(index < state.rows.len()).then_some(index)
	}

	/// Moves focus to the next (or previous) control in tab order, as Tab
	/// does from a list box. The list takes every key (so arrows reach it), so
	/// Tab has to be passed on by hand. Neither wx's `Navigate` (from a panel
	/// with no children) nor Windows' `GetNextDlgTabItem` (wx panels aren't
	/// dialog containers to it) went anywhere, so this walks the window's
	/// controls itself, in creation order, as both of those do.
	fn tab_away(&self, backwards: bool) {
		use windows::Win32::UI::{
			Input::KeyboardAndMouse::{IsWindowEnabled, SetFocus},
			WindowsAndMessaging::{
				GA_ROOT, GW_CHILD, GW_HWNDNEXT, GWL_STYLE, GetAncestor, GetWindow, GetWindowLongPtrW, IsWindowVisible,
				WS_TABSTOP,
			},
		};
		/// Appends the visible, enabled tab stops under `parent` in order. A
		/// container with stops inside it is passed over for them. The list
		/// itself always counts, whatever wx has done to its window style.
		fn collect(parent: HWND, me: HWND, stops: &mut Vec<HWND>) {
			let mut child = unsafe { GetWindow(parent, GW_CHILD) }.ok();
			while let Some(hwnd) = child {
				if unsafe { IsWindowVisible(hwnd) }.as_bool() {
					let before = stops.len();
					collect(hwnd, me, stops);
					let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
					let tab_stop = style & WS_TABSTOP.0 != 0 && unsafe { IsWindowEnabled(hwnd) }.as_bool();
					if hwnd == me || (stops.len() == before && tab_stop) {
						stops.push(hwnd);
					}
				}
				child = unsafe { GetWindow(hwnd, GW_HWNDNEXT) }.ok();
			}
		}
		let me = HWND(self.panel.get_handle().cast());
		let mut stops = Vec::new();
		collect(unsafe { GetAncestor(me, GA_ROOT) }, me, &mut stops);
		let Some(at) = stops.iter().position(|&h| h == me) else {
			crate::websocket_events::ui_timing_log("Tab from the message list: the list isn't visible");
			return;
		};
		let n = stops.len();
		let next = stops[if backwards { (at + n - 1) % n } else { (at + 1) % n }];
		let moved = next != me && unsafe { SetFocus(Some(next)) }.is_ok();
		crate::websocket_events::ui_timing_log(&format!(
			"Tab from the message list: {} stops, list at {}, moved to {}: {}",
			n,
			at,
			if backwards { (at + n - 1) % n } else { (at + 1) % n },
			moved
		));
	}

	/// Arrows, Page Up/Down, Tab and type-ahead. Home and End are the app's.
	fn handle_char(&self, key: &wxdragon::event::KeyboardEvent) -> bool {
		let Some(code) = key.get_key_code() else { return false };
		if code == WXK_TAB {
			self.tab_away(key.shift_down());
			return true;
		}
		if key.control_down() || key.alt_down() {
			return false;
		}
		let (count, current, page) = {
			let shared = self.shared();
			let state = shared.state.borrow();
			(state.rows.len(), state.selected, self.visible_rows(&state).saturating_sub(1).max(1))
		};
		if count == 0 {
			return false;
		}
		let target = match code {
			WXK_UP | WXK_NUMPAD_UP => Some(current.map_or(0, |c| c.saturating_sub(1))),
			WXK_DOWN | WXK_NUMPAD_DOWN => Some(current.map_or(0, |c| (c + 1).min(count - 1))),
			WXK_PAGEUP | WXK_NUMPAD_PAGEUP => Some(current.map_or(0, |c| c.saturating_sub(page))),
			WXK_PAGEDOWN | WXK_NUMPAD_PAGEDOWN => Some(current.map_or(0, |c| (c + page).min(count - 1))),
			WXK_HOME | WXK_NUMPAD_HOME => Some(0),
			WXK_END | WXK_NUMPAD_END => Some(count - 1),
			_ => None,
		};
		if let Some(target) = target {
			self.select_by_user(target);
			return true;
		}
		let checkable = self.shared().state.borrow().checkable;
		if checkable && code == WXK_SPACE {
			if let Some(current) = current {
				self.toggle_by_user(current);
			}
			return true;
		}
		match key.get_unicode_key().and_then(|c| char::from_u32(c as u32)) {
			Some(ch) if !ch.is_control() => {
				self.type_ahead(ch);
				true
			}
			_ => false,
		}
	}

	/// Jumps to the next row starting with what's been typed, as a list box
	/// does. The same letter again moves on to the next match.
	fn type_ahead(&self, ch: char) {
		let found = {
			let shared = self.shared();
			let mut state = shared.state.borrow_mut();
			let now = Instant::now();
			if state.last_search.is_none_or(|t| now.duration_since(t).as_millis() > TYPE_AHEAD_RESET_MS) {
				state.search.clear();
			}
			state.last_search = Some(now);
			let ch: String = ch.to_lowercase().collect();
			let repeat = state.search == ch;
			if !repeat {
				state.search.push_str(&ch);
			}
			let prefix = state.search.clone();
			let count = state.rows.len();
			let start = match state.selected {
				Some(s) if repeat || prefix.chars().count() == 1 => s + 1,
				Some(s) => s,
				None => 0,
			};
			(0..count).map(|offset| (start + offset) % count).find(|&i| {
				let text = &state.rows[i].text;
				text.chars().take(prefix.chars().count()).collect::<String>().to_lowercase() == prefix
			})
		};
		if let Some(index) = found {
			self.select_by_user(index);
		}
	}

	fn paint(&self) {
		let dc = AutoBufferedPaintDC::new(&self.panel);
		let shared = self.shared();
		// Painting can happen inside a scroll the list itself started.
		let Ok(mut state) = shared.state.try_borrow_mut() else { return };
		state.top = self.view_top().min(state.rows.len().saturating_sub(1));
		let (width, height) = {
			let size = self.panel.get_client_size();
			(size.width, size.height)
		};
		let background = self.panel.get_background_color();
		let foreground = self.panel.get_foreground_color();
		dc.set_brush(background, BrushStyle::Solid);
		dc.set_pen(background, 1, PenStyle::Solid);
		dc.draw_rectangle(0, 0, width, height);
		if let Some(font) = self.panel.get_font() {
			dc.set_font(&font);
		}
		dc.set_background_mode(BackgroundMode::Transparent);
		let row_height = (dc.get_text_extent("Ag").1 + 6).max(1);
		if state.row_height != row_height {
			// A new font (the theme's, or another screen's scale): the
			// scrollbar's steps are rows, so it's set up again after this paint.
			state.row_height = row_height;
			let list = *self;
			wxdragon::call_after(Box::new(move || {
				let shared = list.shared();
				list.apply_view(&shared);
				list.panel.refresh(false, None);
			}));
		}
		let focused = self.panel.has_focus();
		let (highlight, highlight_text) = if focused {
			(
				SystemSettings::get_colour(SystemColour::Highlight),
				SystemSettings::get_colour(SystemColour::HighlightText),
			)
		} else {
			(SystemSettings::get_colour(SystemColour::BtnFace), SystemSettings::get_colour(SystemColour::BtnText))
		};
		let mut y = 0;
		let mut index = state.top;
		while y < height && index < state.rows.len() {
			let selected = state.selected == Some(index);
			if selected {
				dc.set_brush(highlight, BrushStyle::Solid);
				dc.set_pen(highlight, 1, PenStyle::Solid);
				dc.draw_rectangle(0, y, width, row_height);
				dc.set_text_foreground(highlight_text);
			} else {
				dc.set_text_foreground(foreground);
			}
			let row = &state.rows[index];
			let check = match (state.checkable, row.checked) {
				(false, _) => "",
				(true, true) => "[x] ",
				(true, false) => "[ ] ",
			};
			let line: String = check
				.chars()
				.chain(row.text.chars().take(DRAWN_CHARS))
				.map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
				.collect();
			dc.draw_text(&line, 4, y + 3);
			y += row_height;
			index += 1;
		}
	}
}

/// The lists the "go back to where I was" shortcut can return to: the nav and
/// chat list boxes and this list.
pub trait CursorList {
	fn count(&self) -> u32;
	fn selection(&self) -> Option<u32>;
	fn select_and_show(&self, index: u32);
	fn focus(&self);
	fn hwnd(&self) -> usize;
}

impl CursorList for ListBox {
	fn count(&self) -> u32 {
		self.get_count()
	}

	fn selection(&self) -> Option<u32> {
		self.get_selection()
	}

	fn select_and_show(&self, index: u32) {
		self.set_selection(index, true);
		self.ensure_visible(index as i32);
	}

	fn focus(&self) {
		self.set_focus();
	}

	fn hwnd(&self) -> usize {
		self.get_handle() as usize
	}
}

impl CursorList for MessageList {
	fn count(&self) -> u32 {
		self.get_count()
	}

	fn selection(&self) -> Option<u32> {
		self.get_selection()
	}

	fn select_and_show(&self, index: u32) {
		self.set_selection(index, true);
	}

	fn focus(&self) {
		self.set_focus();
	}

	fn hwnd(&self) -> usize {
		self.get_handle() as usize
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	fn st(keys: &[&str]) -> State {
		let mut s = State { next_id: FIRST_ROW_ID, ..Default::default() };
		s.replace_keyed(keys, keys, false);
		s
	}
	#[test]
	fn keyed_rebuild_keeps_cursor_ids_and_frozen_text() {
		let mut s = st(&["a", "b", "c"]);
		s.selected = Some(1);
		s.top = 2;
		let id_b = s.rows[1].id;
		s.replace_keyed(&["x", "y", "a", "b", "c"], &["x", "y", "a", "b2", "c"], true);
		assert_eq!(s.selected, Some(3));
		assert_eq!(s.rows[3].id, id_b);
		assert_eq!(s.top, 4);
		assert_eq!(s.frozen, Some((id_b, "b".to_string())));
		s.replace_keyed(&["x", "b"], &["x", "b3"], true);
		assert_eq!(s.selected, Some(1));
		assert_eq!(s.frozen, Some((id_b, "b".to_string())));
		assert_eq!(s.top, 0);
		s.replace_keyed(&["x"], &["x"], true);
		assert_eq!(s.selected, None);
		assert_eq!(s.frozen, None);
		let mut s = st(&["", "", "a"]);
		s.selected = Some(0);
		s.replace_keyed(&["", "a"], &["", "a"], false);
		assert_eq!(s.selected, None);
		assert_ne!(s.rows[0].id, 0);
		assert!(s.rows[0].id >= FIRST_ROW_ID);
	}
}
