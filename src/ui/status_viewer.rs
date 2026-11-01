//! The status viewer: WhatsApp Web's full-screen status player, built from
//! native controls. It plays the Status tab's updates in the tab's order,
//! one after another (a photo or text for a few seconds, a video to its
//! end), and offers what WhatsApp Web does around them: reply, react, open
//! the chat, mute the poster, and for our own updates who saw them (with
//! their reactions) and delete. Photos and videos are shown in the window;
//! nothing is handed to another app.
//!
//! Keys: Left/Right go to the previous or next update (outside text fields),
//! Space pauses or resumes unless a button has focus, Escape closes; the
//! video player's keys work too (`media_player.rs`). Typing a reply holds
//! the current update, as on WhatsApp Web.
//!
//! The backend's answers (media_ready, status_viewers, status_reacted, ...)
//! arrive while the viewer's modal loop runs; `on_event` routes them here.

use std::{
	cell::{Cell, RefCell},
	rc::Rc,
};

use wxdragon::prelude::*;

use crate::{
	AppState,
	ui::{keys, media_player::MediaPlayer, theme, window::WindowParts},
	ui_commands::status_extras::STATUS_REACTIONS,
};

/// The longest edge a photo is decoded at.
const PHOTO_EDGE: u32 = 1280;
/// How long a photo or text update stays up before the next, at least.
const SHOW_MS: i32 = 6000;
/// Added per character of text or caption, so a long one can be read, up to
/// `MAX_SHOW_MS`.
const PER_CHAR_MS: i32 = 60;
const MAX_SHOW_MS: i32 = 20000;
/// Text updates without a colour of their own use WhatsApp's teal.
const DEFAULT_BACKGROUND: u32 = 0xFF07_5E54;

/// One update, as the viewer plays it.
#[derive(Clone)]
struct Entry {
	poster_jid: String,
	poster_name: String,
	from_me: bool,
	muted: bool,
	id: String,
	text: String,
	media_type: String,
	timestamp: i64,
	viewed: bool,
	my_reaction: String,
	background: u32,
	text_color: u32,
}

/// What the viewer was closed for.
pub(crate) enum ViewerOutcome {
	Closed,
	/// Open the chat with this person.
	OpenChat(String),
}

struct Viewer {
	state: Rc<RefCell<AppState>>,
	dialog: Dialog,
	announcer: StaticText,
	header: StaticText,
	content: TextCtrl,
	photo: StaticBitmap,
	player: Rc<MediaPlayer>,
	pause_btn: Button,
	prev_btn: Button,
	next_btn: Button,
	auto_check: CheckBox,
	reactions_panel: Panel,
	reaction_btns: Vec<Button>,
	reply_panel: Panel,
	open_chat_btn: Button,
	reply_label: StaticText,
	reply_field: TextCtrl,
	reply_send_btn: Button,
	mute_btn: Button,
	viewers_panel: Panel,
	viewers_label: StaticText,
	viewers_list: ListBox,
	delete_btn: Button,
	save_btn: Button,
	timer: Timer<Dialog>,
	entries: RefCell<Vec<Entry>>,
	index: Cell<usize>,
	paused: Cell<bool>,
	/// The file of the update on screen, once the backend has it.
	media_path: RefCell<Option<String>>,
	outcome: RefCell<Option<ViewerOutcome>>,
}

thread_local! {
	/// The open viewer, for `on_event`.
	static OPEN: RefCell<Option<Rc<Viewer>>> = const { RefCell::new(None) };
}

fn send(state: &Rc<RefCell<AppState>>, action: &str, data: serde_json::Value) {
	let payload = serde_json::json!({ "action": action, "data": data });
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

fn text(data: &serde_json::Value, key: &str) -> String {
	data.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn argb(colour: u32) -> Colour {
	Colour::rgb((colour >> 16) as u8, (colour >> 8) as u8, colour as u8)
}

/// Every live update in the Status tab's order.
fn entries(s: &AppState) -> Vec<Entry> {
	s.status_groups
		.iter()
		.flat_map(|g| {
			g.items.iter().map(move |it| Entry {
				poster_jid: g.poster_jid.clone(),
				poster_name: g.poster_name.clone(),
				from_me: g.from_me,
				muted: g.muted,
				id: it.id.clone(),
				text: it.text.clone(),
				media_type: it.media_type.clone(),
				timestamp: it.timestamp,
				viewed: it.viewed,
				my_reaction: it.my_reaction.clone(),
				background: it.background,
				text_color: it.text_color,
			})
		})
		.collect()
}

/// What kind of update `entry` is, as spoken.
fn kind(entry: &Entry) -> &'static str {
	match entry.media_type.as_str() {
		"image" => "Photo",
		"video" => "Video",
		"" => "Text",
		_ => "Media",
	}
}

/// How long a photo or text update stays up.
fn show_ms(entry: &Entry) -> i32 {
	(SHOW_MS + PER_CHAR_MS * entry.text.chars().count() as i32).min(MAX_SHOW_MS)
}

/// Opens the viewer on the update `start_id`, modally, and returns once it's
/// closed. Rows the viewer marked seen are updated in `state`; the caller
/// refreshes the list.
pub(crate) fn show(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, start_id: &str) -> ViewerOutcome {
	let (list, auto) = {
		let s = state.borrow();
		(entries(&s), s.settings.status_auto_advance)
	};
	let Some(start) = list.iter().position(|e| e.id == start_id) else { return ViewerOutcome::Closed };
	let viewer = build(state, parts, list, auto);
	OPEN.with(|open| *open.borrow_mut() = Some(viewer.clone()));
	viewer.show_entry(start);
	viewer.next_btn.set_focus();
	viewer.dialog.show_modal();
	OPEN.with(|open| open.borrow_mut().take());
	viewer.timer.stop();
	viewer.player.stop();
	// A dialog built per use must be destroyed, or it and every closure bound
	// to it live on hidden.
	viewer.dialog.destroy();
	viewer.outcome.borrow_mut().take().unwrap_or(ViewerOutcome::Closed)
}

fn build(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, list: Vec<Entry>, auto: bool) -> Rc<Viewer> {
	let dialog = Dialog::builder(&parts.frame, "Status")
		.with_size(820, 760)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder | DialogStyle::MaximizeBox)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	// The dialog is the foreground window while it's up, so its own live
	// region is the one a screen reader speaks.
	let announcer = StaticText::builder(&panel).with_label("").build();
	live_region::set_live_region(&announcer);
	sizer.add(&announcer, 0, SizerFlag::All, 0);

	// Who posted it, where it is in the list, and when.
	let header = StaticText::builder(&panel).with_label("").build();
	theme::style_heading(&header);
	sizer.add(&header, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	// The update itself: a text update's text (drawn in its colours), or
	// what a photo or video is and its caption. Read-only, so a screen
	// reader can go through it line by line.
	let content_label = StaticText::builder(&panel).with_label("Status &update").build();
	theme::style_muted_label(&content_label);
	sizer.add(&content_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let content = TextCtrl::builder(&panel)
		.with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly | TextCtrlStyle::Rich2)
		.build();
	content.set_font(&theme::Fonts::body());
	content.set_min_size(Size::new(-1, 70));
	sizer.add(&content, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let empty = wxdragon::bitmap::Bitmap::from_rgba(&[0, 0, 0, 0], 1, 1).expect("1x1 bitmap");
	let photo =
		StaticBitmap::builder(&panel).with_bitmap(Some(empty)).with_scale_mode(Some(ScaleMode::AspectFit)).build();
	photo.set_min_size(Size::new(480, 320));
	sizer.add(&photo, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);
	photo.show(false);

	let player = MediaPlayer::build(&panel, &sizer, announcer);
	player.panel.show(false);

	let nav_row = BoxSizer::builder(Orientation::Horizontal).build();
	let prev_btn = Button::builder(&panel).with_label("&Previous").build();
	nav_row.add(&prev_btn, 0, SizerFlag::Right, 8);
	let pause_btn = Button::builder(&panel).with_label("Pa&use").build();
	nav_row.add(&pause_btn, 0, SizerFlag::Right, 8);
	let next_btn = Button::builder(&panel).with_label("&Next").build();
	theme::style_primary_button(&next_btn);
	nav_row.add(&next_btn, 0, SizerFlag::Right, 16);
	let auto_check = CheckBox::builder(&panel).with_label("Play next update &automatically").build();
	auto_check.set_value(auto);
	nav_row.add(&auto_check, 0, SizerFlag::AlignCenterVertical, 0);
	sizer.add_sizer(&nav_row, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	// Someone else's update: react, reply, open the chat, mute.
	let reactions_panel = Panel::builder(&panel).build();
	theme::style_content(&reactions_panel);
	let reactions_row = BoxSizer::builder(Orientation::Horizontal).build();
	let reactions_label = StaticText::builder(&reactions_panel).with_label("React:").build();
	theme::style_muted_label(&reactions_label);
	reactions_row.add(&reactions_label, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 8);
	let reaction_btns: Vec<Button> = STATUS_REACTIONS
		.iter()
		.map(|emoji| {
			let btn = Button::builder(&reactions_panel).with_label(emoji).build();
			btn.set_font(&theme::Fonts::body());
			reactions_row.add(&btn, 0, SizerFlag::Right, 4);
			btn
		})
		.collect();
	reactions_panel.set_sizer(reactions_row, true);
	sizer.add(&reactions_panel, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	// Open chat comes ahead of the label: nothing may sit between a field's
	// label and the field, or Windows names the field after it.
	let reply_panel = Panel::builder(&panel).build();
	theme::style_content(&reply_panel);
	let reply_row = BoxSizer::builder(Orientation::Horizontal).build();
	let open_chat_btn = Button::builder(&reply_panel).with_label("&Open chat").build();
	reply_row.add(&open_chat_btn, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 12);
	let reply_label = StaticText::builder(&reply_panel).with_label("&Reply").build();
	theme::style_muted_label(&reply_label);
	reply_row.add(&reply_label, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 8);
	let reply_field = TextCtrl::builder(&reply_panel).with_style(TextCtrlStyle::ProcessEnter).build();
	reply_field.set_font(&theme::Fonts::body());
	reply_row.add(&reply_field, 1, SizerFlag::Expand | SizerFlag::Right, 8);
	let reply_send_btn = Button::builder(&reply_panel).with_label("Send reply").build();
	reply_row.add(&reply_send_btn, 0, SizerFlag::AlignCenterVertical, 0);
	reply_panel.set_sizer(reply_row, true);
	sizer.add(&reply_panel, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	// Our own update: who saw it, and deleting it.
	let viewers_panel = Panel::builder(&panel).build();
	theme::style_content(&viewers_panel);
	let viewers_column = BoxSizer::builder(Orientation::Vertical).build();
	let viewers_label = StaticText::builder(&viewers_panel).with_label("&Viewed by").build();
	theme::style_muted_label(&viewers_label);
	viewers_column.add(&viewers_label, 0, SizerFlag::Bottom, 4);
	let viewers_list = ListBox::builder(&viewers_panel).build();
	viewers_list.set_font(&theme::Fonts::body());
	viewers_list.set_min_size(Size::new(-1, 110));
	viewers_column.add(&viewers_list, 0, SizerFlag::Expand, 0);
	viewers_panel.set_sizer(viewers_column, true);
	sizer.add(&viewers_panel, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let more_row = BoxSizer::builder(Orientation::Horizontal).build();
	let mute_btn = Button::builder(&panel).with_label("&Mute").build();
	more_row.add(&mute_btn, 0, SizerFlag::Right, 8);
	let delete_btn = Button::builder(&panel).with_label("&Delete...").build();
	more_row.add(&delete_btn, 0, SizerFlag::Right, 8);
	let save_btn = Button::builder(&panel).with_label("&Save as...").build();
	more_row.add(&save_btn, 0, SizerFlag::Right, 8);
	more_row.add_stretch_spacer(1);
	let close_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("Close").build();
	more_row.add(&close_btn, 0, SizerFlag::Left, 0);
	sizer.add_sizer(&more_row, 0, SizerFlag::Expand | SizerFlag::All, 10);

	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let viewer = Rc::new(Viewer {
		state: state.clone(),
		dialog,
		announcer,
		header,
		content,
		photo,
		player,
		pause_btn,
		prev_btn,
		next_btn,
		auto_check,
		reactions_panel,
		reaction_btns,
		reply_panel,
		open_chat_btn,
		reply_label,
		reply_field,
		reply_send_btn,
		mute_btn,
		viewers_panel,
		viewers_label,
		viewers_list,
		delete_btn,
		save_btn,
		timer: Timer::new(&dialog),
		entries: RefCell::new(list),
		index: Cell::new(0),
		paused: Cell::new(false),
		media_path: RefCell::new(None),
		outcome: RefCell::new(None),
	});
	bind(&viewer, close_btn);
	viewer
}

fn bind(viewer: &Rc<Viewer>, close_btn: Button) {
	let weak = Rc::downgrade(viewer);
	viewer.timer.on_tick(handler(&weak, Viewer::advance_tick));
	viewer.prev_btn.on_click(handler(&weak, Viewer::previous));
	viewer.next_btn.on_click(handler(&weak, Viewer::next));
	viewer.pause_btn.on_click(handler(&weak, Viewer::toggle_pause));
	viewer.auto_check.on_toggled(handler(&weak, Viewer::auto_changed));
	viewer.open_chat_btn.on_click(handler(&weak, Viewer::open_chat));
	viewer.reply_send_btn.on_click(handler(&weak, Viewer::send_reply));
	viewer.reply_field.on_text_enter(handler(&weak, Viewer::send_reply));
	viewer.mute_btn.on_click(handler(&weak, Viewer::toggle_mute));
	viewer.delete_btn.on_click(handler(&weak, Viewer::delete));
	viewer.save_btn.on_click(handler(&weak, Viewer::save_as));
	let dialog = viewer.dialog;
	close_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));
	for (i, btn) in viewer.reaction_btns.iter().enumerate() {
		let weak = Rc::downgrade(viewer);
		btn.on_click(move |_| {
			if let Some(v) = weak.upgrade() {
				v.react(STATUS_REACTIONS[i]);
			}
		});
	}
	{
		let weak = Rc::downgrade(viewer);
		viewer.player.on_ended(move || {
			if let Some(v) = weak.upgrade()
				&& v.auto_check.is_checked()
				&& !v.paused.get()
				&& !v.replying()
			{
				v.next();
			}
		});
	}

	let weak = Rc::downgrade(viewer);
	viewer.dialog.bind_internal(EventType::CHAR_HOOK, move |event| {
		let data = WindowEventData::new(event);
		let WindowEventData::Keyboard(ref key) = data else { return };
		let Some(v) = weak.upgrade() else { return };
		let code = key.get_key_code();
		if keys::is_escape(code) {
			v.dialog.end_modal(ID_CANCEL);
			data.skip(false);
			return;
		}
		let focus = crate::ui::media_player::focused_window();
		let in_text = focus == v.reply_field.get_handle() || focus == v.content.get_handle();
		if in_text {
			data.skip(true);
			return;
		}
		if v.player.handle_key(key) {
			data.skip(false);
			return;
		}
		let plain = !key.control_down() && !key.alt_down() && !key.shift_down();
		let on_list = focus == v.viewers_list.get_handle();
		let handled = plain
			&& if keys::is_left(code) && !on_list {
				v.previous();
				true
			} else if keys::is_right(code) && !on_list {
				v.next();
				true
			} else if keys::is_space(code) && !is_button_or_check(&v, focus) {
				v.toggle_pause();
				true
			} else {
				false
			};
		data.skip(!handled);
	});
}

/// An event handler running `f` on the viewer while it's still there.
fn handler<E>(weak: &std::rc::Weak<Viewer>, f: fn(&Viewer)) -> impl FnMut(E) + 'static {
	let weak = weak.clone();
	move |_| {
		if let Some(v) = weak.upgrade() {
			f(&v);
		}
	}
}

/// Whether `hwnd` is a control Space already does something on.
fn is_button_or_check(v: &Viewer, hwnd: *mut std::ffi::c_void) -> bool {
	let mut controls = vec![
		v.pause_btn.get_handle(),
		v.prev_btn.get_handle(),
		v.next_btn.get_handle(),
		v.auto_check.get_handle(),
		v.open_chat_btn.get_handle(),
		v.reply_send_btn.get_handle(),
		v.mute_btn.get_handle(),
		v.delete_btn.get_handle(),
		v.save_btn.get_handle(),
	];
	controls.extend(v.reaction_btns.iter().map(|b| b.get_handle()));
	// Anything else that's a button (Close, the player's own) counts too.
	controls.contains(&hwnd) || window_class_is_button(hwnd)
}

fn window_class_is_button(hwnd: *mut std::ffi::c_void) -> bool {
	let mut name = [0u16; 16];
	let len = unsafe {
		windows::Win32::UI::WindowsAndMessaging::GetClassNameW(windows::Win32::Foundation::HWND(hwnd), &mut name)
	};
	String::from_utf16_lossy(&name[..len.max(0) as usize]).eq_ignore_ascii_case("Button")
}

impl Viewer {
	fn current(&self) -> Option<Entry> {
		self.entries.borrow().get(self.index.get()).cloned()
	}

	fn replying(&self) -> bool {
		crate::ui::media_player::focused_window() == self.reply_field.get_handle()
			|| !self.reply_field.get_value().trim().is_empty()
	}

	fn show_entry(&self, index: usize) {
		let Some(entry) = self.entries.borrow().get(index).cloned() else { return };
		self.index.set(index);
		self.timer.stop();
		self.player.stop();
		*self.media_path.borrow_mut() = None;
		self.reply_field.set_value("");

		let (position, count) = self.position_in_group(&entry);
		let time = whatsnap::LocalTime::from_unix(entry.timestamp).map(|t| t.format("%I:%M %p")).unwrap_or_default();
		let who = if entry.from_me { "My status".to_string() } else { entry.poster_name.clone() };
		let header = format!("{who}, {position} of {count}, {time}");
		self.header.set_label(&header);
		self.dialog.set_label(&format!("Status: {who}"));

		let body = match entry.media_type.as_str() {
			"" => entry.text.clone(),
			_ if entry.text.is_empty() => format!("{}.", kind(&entry)),
			_ => format!("{}. {}", kind(&entry), entry.text),
		};
		self.content.set_value(&body);
		if entry.media_type.is_empty() {
			// The poster's colours, unless Windows' own colours are in use
			// (high contrast), which the app never paints over.
			if theme::use_custom_colours() {
				let background = if entry.background == 0 { DEFAULT_BACKGROUND } else { entry.background };
				let text_color = if entry.text_color == 0 { 0xFFFF_FFFF } else { entry.text_color };
				self.content.set_background_color(argb(background));
				self.content.set_foreground_color(argb(text_color));
			}
			self.content.set_font(&theme::Fonts::heading());
			self.content.set_min_size(Size::new(-1, 220));
		} else {
			theme::style_content(&self.content);
			self.content.set_font(&theme::Fonts::body());
			self.content.set_min_size(Size::new(-1, 70));
		}
		self.content.refresh(true, None);

		self.photo.show(false);
		self.player.panel.show(false);
		self.save_btn.enable(false);
		self.save_btn.show(!entry.media_type.is_empty());

		self.reactions_panel.show(!entry.from_me);
		self.reply_panel.show(!entry.from_me);
		self.mute_btn.show(!entry.from_me);
		self.viewers_panel.show(entry.from_me);
		self.delete_btn.show(entry.from_me);
		if entry.from_me {
			self.viewers_list.clear();
			self.viewers_label.set_label("&Viewed by");
			send(&self.state, "get_status_viewers", serde_json::json!({ "id": entry.id }));
		} else {
			self.open_chat_btn.set_label(&format!("&Open chat with {}", entry.poster_name));
			self.reply_label.set_label(&format!("&Reply to {}", entry.poster_name));
			self.show_mute(&entry);
			self.show_reaction(&entry.my_reaction);
		}
		self.prev_btn.enable(index > 0);
		self.dialog.layout();
		self.keep_focus();

		let mut spoken = format!("{header}. {body}");
		if !entry.from_me && !entry.my_reaction.is_empty() {
			spoken.push_str(&format!(" You reacted {}.", entry.my_reaction));
		}
		if !entry.media_type.is_empty() {
			send(
				&self.state,
				"prepare_media",
				serde_json::json!({ "jid": "status@broadcast", "message_id": entry.id }),
			);
		} else {
			self.start_timer(&entry);
		}
		crate::platform::announce::announce(self.announcer, &spoken);
		self.mark_viewed(&entry);
	}

	/// Puts focus on Next when the control that had it was just hidden or
	/// disabled, which would leave the keyboard nowhere.
	fn keep_focus(&self) {
		use windows::Win32::{
			Foundation::HWND,
			UI::{Input::KeyboardAndMouse::IsWindowEnabled, WindowsAndMessaging::IsWindowVisible},
		};
		let focus = HWND(crate::ui::media_player::focused_window());
		let usable =
			!focus.is_invalid() && unsafe { IsWindowVisible(focus).as_bool() && IsWindowEnabled(focus).as_bool() };
		if !usable {
			self.next_btn.set_focus();
		}
	}

	/// Where `entry` is among its poster's updates, counting from 1.
	fn position_in_group(&self, entry: &Entry) -> (usize, usize) {
		let entries = self.entries.borrow();
		let theirs: Vec<&Entry> = entries.iter().filter(|e| e.poster_jid == entry.poster_jid).collect();
		let at = theirs.iter().position(|e| e.id == entry.id).unwrap_or(0);
		(at + 1, theirs.len())
	}

	fn mark_viewed(&self, entry: &Entry) {
		if entry.from_me || entry.viewed {
			return;
		}
		if let Some(e) = self.entries.borrow_mut().iter_mut().find(|e| e.id == entry.id) {
			e.viewed = true;
		}
		{
			let mut s = self.state.borrow_mut();
			if let Some(group) = s.status_groups.iter_mut().find(|g| g.poster_jid == entry.poster_jid) {
				if let Some(item) = group.items.iter_mut().find(|it| it.id == entry.id) {
					item.viewed = true;
				}
				group.all_viewed = group.items.iter().all(|it| it.viewed);
			}
		}
		send(&self.state, "mark_status_viewed", serde_json::json!({ "id": entry.id, "poster_jid": entry.poster_jid }));
	}

	fn start_timer(&self, entry: &Entry) {
		if self.auto_check.is_checked() && !self.paused.get() {
			self.timer.start(show_ms(entry), true);
		}
	}

	fn advance_tick(&self) {
		if self.paused.get() || !self.auto_check.is_checked() {
			return;
		}
		// Typing a reply holds the update, as on WhatsApp Web.
		if self.replying() {
			self.timer.start(2000, true);
			return;
		}
		self.next();
	}

	fn next(&self) {
		let next = self.index.get() + 1;
		if next < self.entries.borrow().len() {
			self.show_entry(next);
		} else {
			// WhatsApp closes the viewer after the last update.
			self.dialog.end_modal(ID_CANCEL);
		}
	}

	fn previous(&self) {
		match self.index.get().checked_sub(1) {
			Some(prev) => self.show_entry(prev),
			None => crate::platform::announce::announce(self.announcer, "This is the first status update."),
		}
	}

	fn toggle_pause(&self) {
		let paused = !self.paused.get();
		self.paused.set(paused);
		self.pause_btn.set_label(if paused { "Res&ume" } else { "Pa&use" });
		if paused {
			self.timer.stop();
			self.player.pause();
			crate::platform::announce::announce(self.announcer, "Paused");
		} else {
			if self.player.is_loaded() {
				self.player.play();
			} else if let Some(entry) = self.current()
				&& (entry.media_type.is_empty() || self.media_path.borrow().is_some())
			{
				self.start_timer(&entry);
			}
			crate::platform::announce::announce(self.announcer, "Playing");
		}
	}

	fn auto_changed(&self) {
		let auto = self.auto_check.is_checked();
		{
			let mut s = self.state.borrow_mut();
			s.settings.status_auto_advance = auto;
			s.settings.save();
		}
		if !auto {
			self.timer.stop();
		} else if let Some(entry) = self.current()
			&& !self.player.is_loaded()
			&& (entry.media_type.is_empty() || self.media_path.borrow().is_some())
		{
			self.start_timer(&entry);
		}
	}

	fn open_chat(&self) {
		if let Some(entry) = self.current() {
			*self.outcome.borrow_mut() = Some(ViewerOutcome::OpenChat(entry.poster_jid));
			self.dialog.end_modal(ID_OK);
		}
	}

	fn send_reply(&self) {
		let Some(entry) = self.current() else { return };
		let reply = self.reply_field.get_value();
		if reply.trim().is_empty() {
			crate::platform::announce::announce(self.announcer, "Type a reply first.");
			return;
		}
		send(
			&self.state,
			"send",
			serde_json::json!({
				"to": entry.poster_jid,
				"text": reply,
				"reply_to_id": entry.id,
				"reply_to_sender": entry.poster_jid,
				"mentions": [],
			}),
		);
		self.reply_field.set_value("");
		crate::platform::announce::announce(self.announcer, &format!("Reply sent to {}.", entry.poster_name));
	}

	fn react(&self, emoji: &str) {
		let Some(entry) = self.current() else { return };
		// Choosing the reaction already there takes it back, as on WhatsApp.
		let emoji = if entry.my_reaction == emoji { "" } else { emoji };
		send(&self.state, "react_status", serde_json::json!({ "id": entry.id, "emoji": emoji }));
	}

	fn show_reaction(&self, mine: &str) {
		for (btn, emoji) in self.reaction_btns.iter().zip(STATUS_REACTIONS) {
			let label = if emoji == mine { format!("{emoji} (your reaction)") } else { emoji.to_string() };
			if btn.get_label() != label {
				btn.set_label(&label);
			}
		}
	}

	fn toggle_mute(&self) {
		if let Some(entry) = self.current() {
			send(&self.state, "set_status_mute", serde_json::json!({ "jid": entry.poster_jid, "muted": !entry.muted }));
		}
	}

	fn show_mute(&self, entry: &Entry) {
		let first = entry.poster_name.split_whitespace().next().unwrap_or(&entry.poster_name).to_string();
		let label = if entry.muted { format!("Un&mute {first}") } else { format!("&Mute {first}") };
		self.mute_btn.set_label(&label);
	}

	fn delete(&self) {
		let Some(entry) = self.current() else { return };
		let was_paused = self.paused.get();
		if !was_paused {
			self.toggle_pause();
		}
		let dialog = MessageDialog::builder(
			&self.dialog,
			"Delete this status update? It will be deleted for everyone who received it.",
			"Delete Status",
		)
		.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
		.build();
		dialog.set_yes_no_labels("Delete", "Cancel");
		if dialog.show_modal() == ID_YES {
			send(&self.state, "delete_status", serde_json::json!({ "id": entry.id }));
			crate::platform::announce::announce(self.announcer, "Deleting...");
		}
		if !was_paused {
			self.toggle_pause();
		}
	}

	fn save_as(&self) {
		let Some(path) = self.media_path.borrow().clone() else { return };
		let Some(entry) = self.current() else { return };
		let ext = std::path::Path::new(&path).extension().and_then(|e| e.to_str()).unwrap_or("jpg").to_string();
		let name = format!("Status {} {}.{}", entry.poster_name, entry.id, ext);
		let was_playing = self.player.is_playing();
		self.timer.stop();
		self.player.pause();
		crate::ui::video_player::save_copy(&self.dialog, &path, &safe_file_name(&name), self.announcer);
		if was_playing {
			self.player.play();
		} else if !self.player.is_loaded() {
			self.start_timer(&entry);
		}
	}

	fn media_ready(&self, path: String) {
		let Some(entry) = self.current() else { return };
		*self.media_path.borrow_mut() = Some(path.clone());
		self.save_btn.enable(true);
		if entry.media_type == "video" {
			self.player.panel.show(true);
			self.dialog.layout();
			self.player.load(&path, !self.paused.get());
			return;
		}
		match crate::media::image_decode::decode_rgba(&path, PHOTO_EDGE) {
			Ok((width, height, rgba)) => {
				if let Some(bitmap) = wxdragon::bitmap::Bitmap::from_rgba(&rgba, width, height) {
					self.photo.set_bitmap(&bitmap);
					self.photo.show(true);
					self.dialog.layout();
				}
			}
			Err(err) => {
				eprintln!("Couldn't decode status photo {path}: {err}");
				crate::platform::announce::announce(self.announcer, "This photo couldn't be shown.");
			}
		}
		self.start_timer(&entry);
	}

	fn media_failed(&self, message: &str) {
		crate::platform::announce::announce(self.announcer, message);
		if let Some(entry) = self.current() {
			self.start_timer(&entry);
		}
	}

	fn show_viewers(&self, data: &serde_json::Value) {
		let viewers = data.get("viewers").and_then(|v| v.as_array()).cloned().unwrap_or_default();
		let count = viewers.len();
		self.viewers_list.clear();
		if count == 0 {
			self.viewers_label.set_label("&Viewed by: no one yet");
			self.viewers_list.append("No one has seen this status update yet.");
		} else {
			let noun = if count == 1 { "person" } else { "people" };
			self.viewers_label.set_label(&format!("&Viewed by {count} {noun}"));
			for v in &viewers {
				let time = v
					.get("timestamp")
					.and_then(|t| t.as_i64())
					.and_then(whatsnap::LocalTime::from_unix)
					.map(|t| t.format("%I:%M %p"))
					.unwrap_or_default();
				let reaction = text(v, "reaction");
				let mut row = format!("{}, {}", text(v, "name"), time);
				if !reaction.is_empty() {
					row.push_str(&format!(", reacted {reaction}"));
				}
				self.viewers_list.append(&row);
			}
		}
		self.viewers_list.set_selection(0, true);
		self.dialog.layout();
	}

	/// Drops `id` from the list after it was deleted, moving on if it was
	/// the one on screen.
	fn remove(&self, id: &str) {
		let Some(at) = self.entries.borrow().iter().position(|e| e.id == id) else { return };
		let current = self.index.get();
		self.entries.borrow_mut().remove(at);
		if self.entries.borrow().is_empty() {
			self.dialog.end_modal(ID_CANCEL);
			return;
		}
		if at == current {
			crate::platform::announce::announce(self.announcer, "Status update deleted.");
			self.show_entry(current.min(self.entries.borrow().len() - 1));
		} else if at < current {
			self.index.set(current - 1);
		}
	}
}

/// `name` with the characters Windows doesn't allow in file names replaced.
fn safe_file_name(name: &str) -> String {
	name.chars()
		.map(
			|c| {
				if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() {
					'_'
				} else {
					c
				}
			},
		)
		.collect()
}

/// Hands a backend event to the open viewer. True when the viewer used it up
/// and nothing else should act on it (the Viewed By list it asked for).
pub(crate) fn on_event(t: &str, data: &serde_json::Value) -> bool {
	let Some(viewer) = OPEN.with(|open| open.borrow().clone()) else { return false };
	let current = viewer.current();
	let is_current = |key: &str| current.as_ref().is_some_and(|e| e.id == text(data, key));
	match t {
		"media_ready" if is_current("message_id") => viewer.media_ready(text(data, "path")),
		"media_failed" if is_current("message_id") => viewer.media_failed(&text(data, "message")),
		"media_pending" if is_current("message_id") => crate::platform::announce::announce(
			viewer.announcer,
			"Getting a fresh copy from the poster's phone. It will show when it arrives.",
		),
		"status_viewers" if is_current("id") => {
			viewer.show_viewers(data);
			return true;
		}
		"status_viewed" if is_current("id") => {
			send(&viewer.state, "get_status_viewers", serde_json::json!({ "id": text(data, "id") }));
		}
		"status_reacted" => {
			let (id, emoji) = (text(data, "id"), text(data, "emoji"));
			if let Some(e) = viewer.entries.borrow_mut().iter_mut().find(|e| e.id == id) {
				e.my_reaction = emoji.clone();
			}
			if is_current("id") {
				viewer.show_reaction(&emoji);
				let spoken =
					if emoji.is_empty() { "Reaction removed.".to_string() } else { format!("You reacted {emoji}.") };
				crate::platform::announce::announce(viewer.announcer, &spoken);
			}
		}
		"status_muted" => {
			let (jid, muted) = (text(data, "jid"), data.get("muted").and_then(|v| v.as_bool()).unwrap_or(false));
			for e in viewer.entries.borrow_mut().iter_mut().filter(|e| e.poster_jid == jid) {
				e.muted = muted;
			}
			if let Some(entry) = viewer.current()
				&& entry.poster_jid == jid
			{
				viewer.show_mute(&entry);
				let verb = if muted { "muted" } else { "unmuted" };
				crate::platform::announce::announce(
					viewer.announcer,
					&format!("{}'s status updates {verb}.", entry.poster_name),
				);
			}
		}
		"status_deleted" => viewer.remove(&text(data, "id")),
		_ => {}
	}
	false
}
