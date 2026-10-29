//! The in-app video player's picture and controls, shared by the video
//! window (`video_player.rs`) and the status viewer (`status_viewer.rs`).
//! Every control is a standard one, each slider named by the label created
//! just before it, so a screen reader reads them like any other.
//!
//! Keys, wherever the player sits: K plays or pauses, J and L go back and
//! forward 10 seconds, M mutes, T says where playback is. On the Position
//! slider the arrows move 5 seconds and Page Up/Down 30; on Volume the arrows
//! step 10%.

use std::{
	cell::{Cell, RefCell},
	rc::{Rc, Weak},
	time::{Duration, Instant},
};

use wxdragon::{event::KeyboardEvent, prelude::*};

use crate::{
	media_engine::MediaEngine,
	ui::{keys, theme},
};

/// Playback speeds the Speed button steps through.
const SPEEDS: [f64; 4] = [1.0, 1.5, 2.0, 0.5];
const TICK_MS: i32 = 250;

pub(crate) struct MediaPlayer {
	/// Everything below, for showing or hiding the player as one.
	pub(crate) panel: Panel,
	/// Where the engine draws the picture.
	video: Panel,
	play_btn: Button,
	mute_btn: Button,
	speed_btn: Button,
	position: Slider,
	time_text: StaticText,
	volume: Slider,
	announcer: StaticText,
	engine: RefCell<Option<MediaEngine>>,
	timer: Timer<Panel>,
	/// Whole seconds the Position slider spans; 0 until the length is known.
	length: Cell<i32>,
	speed: Cell<usize>,
	/// When the user last moved the Position slider, so the timer doesn't
	/// pull it back before the seek lands.
	last_user_seek: Cell<Option<Instant>>,
	/// Whether playback starts as soon as the file is loaded.
	autoplay: Cell<bool>,
	on_ended: RefCell<Option<Box<dyn Fn()>>>,
}

impl MediaPlayer {
	/// Builds the player into `parent` (added to `sizer`), announcing through
	/// `announcer`, the live region of the window it's in.
	pub(crate) fn build(parent: &Panel, sizer: &BoxSizer, announcer: StaticText) -> Rc<Self> {
		let panel = Panel::builder(parent).build();
		theme::style_content(&panel);
		let column = BoxSizer::builder(Orientation::Vertical).build();

		let video = Panel::builder(&panel).build();
		video.set_background_color(Colour::rgb(0, 0, 0));
		video.set_min_size(Size::new(480, 270));
		column.add(&video, 1, SizerFlag::Expand | SizerFlag::Bottom, 8);

		let position_row = BoxSizer::builder(Orientation::Horizontal).build();
		let position_label = StaticText::builder(&panel).with_label("&Position").build();
		theme::style_muted_label(&position_label);
		position_row.add(&position_label, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 8);
		let position = Slider::builder(&panel).with_min_value(0).with_max_value(1).build();
		position_row.add(&position, 1, SizerFlag::Expand | SizerFlag::Right, 8);
		let time_text = StaticText::builder(&panel).with_label("0:00 of 0:00").build();
		time_text.set_font(&theme::Fonts::body());
		position_row.add(&time_text, 0, SizerFlag::AlignCenterVertical, 0);
		column.add_sizer(&position_row, 0, SizerFlag::Expand | SizerFlag::Bottom, 8);

		let button_row = BoxSizer::builder(Orientation::Horizontal).build();
		let play_btn = Button::builder(&panel).with_label("Play").build();
		theme::style_primary_button(&play_btn);
		button_row.add(&play_btn, 0, SizerFlag::Right, 8);
		let mute_btn = Button::builder(&panel).with_label("Mute").build();
		button_row.add(&mute_btn, 0, SizerFlag::Right, 8);
		let speed_btn = Button::builder(&panel).with_label("Speed 1x").build();
		button_row.add(&speed_btn, 0, SizerFlag::Right, 16);
		let volume_label = StaticText::builder(&panel).with_label("&Volume").build();
		theme::style_muted_label(&volume_label);
		button_row.add(&volume_label, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 8);
		let volume = Slider::builder(&panel).with_min_value(0).with_max_value(100).with_value(100).build();
		button_row.add(&volume, 1, SizerFlag::Expand, 0);
		column.add_sizer(&button_row, 0, SizerFlag::Expand, 0);

		panel.set_sizer(column, true);
		sizer.add(&panel, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

		let player = Rc::new(Self {
			panel,
			video,
			play_btn,
			mute_btn,
			speed_btn,
			position,
			time_text,
			volume,
			announcer,
			engine: RefCell::new(None),
			timer: Timer::new(&panel),
			length: Cell::new(0),
			speed: Cell::new(0),
			last_user_seek: Cell::new(None),
			autoplay: Cell::new(true),
			on_ended: RefCell::new(None),
		});
		player.bind();
		player
	}

	fn bind(self: &Rc<Self>) {
		let weak = Rc::downgrade(self);
		self.timer.on_tick(handler(&weak, Self::tick));
		self.play_btn.on_click(handler(&weak, Self::toggle_play));
		self.mute_btn.on_click(handler(&weak, Self::toggle_mute));
		self.speed_btn.on_click(handler(&weak, Self::next_speed));
		self.video.on_size(handler(&weak, Self::fit));

		let weak = Rc::downgrade(self);
		self.position.on_slider(move |event| {
			if let Some(player) = weak.upgrade() {
				player.last_user_seek.set(Some(Instant::now()));
				if let Some(engine) = player.engine.borrow().as_ref() {
					engine.seek(event.get_value() as f64);
				}
				player.show_time();
			}
		});
		let weak = Rc::downgrade(self);
		self.volume.on_slider(move |event| {
			if let Some(player) = weak.upgrade()
				&& let Some(engine) = player.engine.borrow().as_ref()
			{
				engine.set_volume(event.get_value() as f64 / 100.0);
			}
		});
	}

	/// Calls `action` each time playback reaches the end.
	pub(crate) fn on_ended(&self, action: impl Fn() + 'static) {
		*self.on_ended.borrow_mut() = Some(Box::new(action));
	}

	/// Opens the file at `path`, playing it from the beginning once it's
	/// loaded when `autoplay`.
	pub(crate) fn load(&self, path: &str, autoplay: bool) {
		self.stop();
		self.autoplay.set(autoplay);
		match MediaEngine::open(self.video.get_handle(), path) {
			Ok(engine) => {
				engine.set_volume(self.volume.get_value() as f64 / 100.0);
				engine.set_rate(SPEEDS[self.speed.get()]);
				// Muting carries over from one video to the next.
				engine.set_muted(self.mute_btn.get_label() == "Unmute");
				*self.engine.borrow_mut() = Some(engine);
				if autoplay {
					self.play_btn.set_label("Pause");
				}
				self.timer.start(TICK_MS, false);
			}
			Err(err) => {
				crate::announce::announce(self.announcer, "The video couldn't be played.");
				eprintln!("Media engine failed to open {path}: {err}");
			}
		}
	}

	/// Stops playback and lets go of the file.
	pub(crate) fn stop(&self) {
		self.timer.stop();
		// Taken out first: dropping the engine shuts it down, which can call
		// back into the window.
		let engine = self.engine.borrow_mut().take();
		drop(engine);
		self.length.set(0);
		self.position.set_range(0, 1);
		self.position.set_value(0);
		self.time_text.set_label("0:00 of 0:00");
		self.play_btn.set_label("Play");
	}

	pub(crate) fn is_loaded(&self) -> bool {
		self.engine.borrow().is_some()
	}

	pub(crate) fn is_playing(&self) -> bool {
		self.engine.borrow().as_ref().is_some_and(|e| !e.is_paused() && !e.is_ended())
	}

	pub(crate) fn pause(&self) {
		self.autoplay.set(false);
		if let Some(engine) = self.engine.borrow().as_ref() {
			engine.pause();
			self.play_btn.set_label("Play");
		}
	}

	pub(crate) fn play(&self) {
		self.autoplay.set(true);
		if let Some(engine) = self.engine.borrow().as_ref() {
			if engine.is_ended() {
				engine.seek(0.0);
			}
			engine.play();
			self.play_btn.set_label("Pause");
		}
	}

	pub(crate) fn toggle_play(&self) {
		if self.is_playing() {
			self.pause();
			crate::announce::announce(self.announcer, "Paused");
		} else {
			self.play();
			crate::announce::announce(self.announcer, "Playing");
		}
	}

	pub(crate) fn toggle_mute(&self) {
		let Some(muted) = self.engine.borrow().as_ref().map(|e| {
			let muted = !e.is_muted();
			e.set_muted(muted);
			muted
		}) else {
			return;
		};
		self.mute_btn.set_label(if muted { "Unmute" } else { "Mute" });
		crate::announce::announce(self.announcer, if muted { "Muted" } else { "Sound on" });
	}

	fn next_speed(&self) {
		let next = (self.speed.get() + 1) % SPEEDS.len();
		self.speed.set(next);
		let label = format!("Speed {}x", SPEEDS[next]);
		self.speed_btn.set_label(&label);
		if let Some(engine) = self.engine.borrow().as_ref() {
			engine.set_rate(SPEEDS[next]);
		}
		crate::announce::announce(self.announcer, &label);
	}

	/// Moves playback `seconds` forward (or back, when negative).
	pub(crate) fn seek_by(&self, seconds: f64) {
		let Some(at) = self.engine.borrow().as_ref().map(|e| {
			let target = e.position() + seconds;
			e.seek(target);
			target.clamp(0.0, e.duration())
		}) else {
			return;
		};
		self.last_user_seek.set(Some(Instant::now()));
		self.position.set_value(at as i32);
		self.show_time();
		crate::announce::announce(self.announcer, &clock(at as i32));
	}

	fn seek_to(&self, seconds: f64) {
		let at = self.engine.borrow().as_ref().map(|e| e.position()).unwrap_or(0.0);
		self.seek_by(seconds - at);
	}

	pub(crate) fn change_volume(&self, step: i32) {
		let value = (self.volume.get_value() + step).clamp(0, 100);
		self.volume.set_value(value);
		if let Some(engine) = self.engine.borrow().as_ref() {
			engine.set_volume(value as f64 / 100.0);
		}
		crate::announce::announce(self.announcer, &format!("Volume {value}%"));
	}

	pub(crate) fn speak_time(&self) {
		crate::announce::announce(self.announcer, &self.time_label());
	}

	/// Handles the player's own keys (see the module notes). True when the
	/// key was one of them.
	pub(crate) fn handle_key(&self, key: &KeyboardEvent) -> bool {
		if key.control_down() || key.alt_down() || key.meta_down() || !self.is_loaded() {
			return false;
		}
		let code = key.get_key_code();
		let focus = focused_window();
		if focus == self.position.get_handle() {
			let page = 30.0;
			if keys::is_left(code) || keys::is_down(code) {
				self.seek_by(-5.0);
			} else if keys::is_right(code) || keys::is_up(code) {
				self.seek_by(5.0);
			} else if code == Some(WXK_PAGEDOWN) {
				self.seek_by(-page);
			} else if code == Some(WXK_PAGEUP) {
				self.seek_by(page);
			} else if keys::is_home(code) {
				self.seek_to(0.0);
			} else if keys::is_end(code) {
				self.seek_to(self.length.get() as f64);
			} else if keys::is_space(code) {
				self.toggle_play();
			} else {
				return self.handle_letter(code);
			}
			return true;
		}
		if focus == self.volume.get_handle() {
			if keys::is_left(code) || keys::is_down(code) {
				self.change_volume(-10);
			} else if keys::is_right(code) || keys::is_up(code) {
				self.change_volume(10);
			} else if keys::is_space(code) {
				self.toggle_play();
			} else {
				return self.handle_letter(code);
			}
			return true;
		}
		self.handle_letter(code)
	}

	fn handle_letter(&self, code: Option<i32>) -> bool {
		if keys::is_letter(code, 'k') {
			self.toggle_play();
		} else if keys::is_letter(code, 'j') {
			self.seek_by(-10.0);
		} else if keys::is_letter(code, 'l') {
			self.seek_by(10.0);
		} else if keys::is_letter(code, 'm') {
			self.toggle_mute();
		} else if keys::is_letter(code, 't') {
			self.speak_time();
		} else {
			return false;
		}
		true
	}

	/// Puts focus on Play/Pause, the first control.
	pub(crate) fn focus(&self) {
		self.play_btn.set_focus();
	}

	fn fit(&self) {
		if let Some(engine) = self.engine.borrow().as_ref() {
			engine.fit_to_window();
		}
	}

	fn tick(&self) {
		let (changes, length, at, ended) = {
			let engine = self.engine.borrow();
			let Some(engine) = engine.as_ref() else { return };
			(engine.take_reports(), engine.duration(), engine.position(), engine.is_ended())
		};
		if changes.loaded {
			let whole = length.ceil() as i32;
			self.length.set(whole);
			self.position.set_range(0, whole.max(1));
			self.fit();
			if self.autoplay.get() {
				self.play();
			}
		}
		if let Some(error) = changes.error {
			self.stop();
			crate::announce::announce(self.announcer, error);
			return;
		}
		if self.length.get() == 0 && length > 0.0 {
			self.length.set(length.ceil() as i32);
			self.position.set_range(0, self.length.get().max(1));
		}
		let user_moved_it = self.last_user_seek.get().is_some_and(|t| t.elapsed() < Duration::from_millis(800));
		if !user_moved_it && self.position.get_value() != at as i32 {
			self.position.set_value(at as i32);
		}
		self.show_time();
		if changes.ended || (ended && self.play_btn.get_label() == "Pause") {
			self.play_btn.set_label("Play");
			if let Some(action) = self.on_ended.borrow().as_ref() {
				action();
			}
		}
	}

	fn time_label(&self) -> String {
		let at = self.engine.borrow().as_ref().map(|e| e.position()).unwrap_or(0.0);
		format!("{} of {}", clock(at as i32), clock(self.length.get()))
	}

	fn show_time(&self) {
		let label = self.time_label();
		if self.time_text.get_label() != label {
			self.time_text.set_label(&label);
		}
	}

	/// Whether the window `hwnd` is one of the player's sliders, which take
	/// the arrow keys for themselves.
	pub(crate) fn owns_arrows(&self, hwnd: *mut std::ffi::c_void) -> bool {
		self.is_loaded() && (hwnd == self.position.get_handle() || hwnd == self.volume.get_handle())
	}
}

/// An event handler running `f` on the player while it's still there.
fn handler<E>(weak: &Weak<MediaPlayer>, f: fn(&MediaPlayer)) -> impl FnMut(E) + 'static {
	let weak = weak.clone();
	move |_| {
		if let Some(player) = weak.upgrade() {
			f(&player);
		}
	}
}

impl Drop for MediaPlayer {
	fn drop(&mut self) {
		self.timer.stop();
	}
}

/// The window with keyboard focus.
pub(crate) fn focused_window() -> *mut std::ffi::c_void {
	unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetFocus().0 }
}

/// `seconds` as m:ss, or h:mm:ss from an hour up.
pub(crate) fn clock(seconds: i32) -> String {
	let s = seconds.max(0);
	if s >= 3600 {
		format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
	} else {
		format!("{}:{:02}", s / 60, s % 60)
	}
}

#[cfg(test)]
mod tests {
	use super::clock;

	#[test]
	fn clock_reads_like_a_player() {
		assert_eq!(clock(0), "0:00");
		assert_eq!(clock(65), "1:05");
		assert_eq!(clock(3725), "1:02:05");
		assert_eq!(clock(-3), "0:00");
	}
}
