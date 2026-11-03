use std::{
	cell::{Cell, RefCell},
	rc::{Rc, Weak},
	time::{Duration, Instant},
};

use wxdragon::{event::KeyboardEvent, prelude::*};

use crate::{
	media::engine::MediaEngine,
	ui::{keys, theme},
};

const SPEEDS: [f64; 4] = [1.0, 1.5, 2.0, 0.5];
const TICK_MS: i32 = 250;

pub(crate) struct MediaPlayer {
	pub(crate) panel: Panel,
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
	length: Cell<i32>,
	speed: Cell<usize>,
	last_user_seek: Cell<Option<Instant>>,
	autoplay: Cell<bool>,
	on_ended: RefCell<Option<Box<dyn Fn()>>>,
}

impl MediaPlayer {
	/// WhatsApp's player: the picture on black, and one dark bar under it
	/// with play, the position and time, sound and speed. Built in the same
	/// order as before, which is the order Tab and a screen reader go
	/// through them; the labels naming the two sliders are kept but take no
	/// room, as WhatsApp shows none.
	pub(crate) fn build(parent: &Panel, sizer: &BoxSizer, announcer: StaticText) -> Rc<Self> {
		use crate::ui::status_viewer::{STAGE_BG, STAGE_MUTED, STAGE_TEXT};
		let custom = theme::use_custom_colours();
		let panel = Panel::builder(parent).build();
		theme::style_content(&panel);
		if custom {
			panel.set_background_color(STAGE_BG);
			panel.set_foreground_color(STAGE_TEXT);
		}
		let column = BoxSizer::builder(Orientation::Vertical).build();

		let video = Panel::builder(&panel).build();
		video.set_background_color(Colour::rgb(0, 0, 0));
		video.set_min_size(Size::new(480, 270));
		column.add(&video, 1, SizerFlag::Expand | SizerFlag::Bottom, 8);

		let controls = BoxSizer::builder(Orientation::Horizontal).build();
		let position_label = StaticText::builder(&panel).with_label("&Position").build();
		theme::style_muted_label(&position_label);
		let position = Slider::builder(&panel).with_min_value(0).with_max_value(1).build();
		let time_text = StaticText::builder(&panel).with_label("0:00 of 0:00").build();
		time_text.set_font(&theme::Fonts::body());

		let play_btn = Button::builder(&panel).with_label("Play").build();
		let mute_btn = Button::builder(&panel).with_label("Mute").build();
		let speed_btn = Button::builder(&panel).with_label("Speed 1x").build();
		let volume_label = StaticText::builder(&panel).with_label("&Volume").build();
		theme::style_muted_label(&volume_label);
		let volume = Slider::builder(&panel).with_min_value(0).with_max_value(100).with_value(100).build();

		if custom {
			for label in [&position_label, &volume_label] {
				label.set_foreground_color(STAGE_MUTED);
				label.set_min_size(Size::new(1, 1));
				label.set_size(Size::new(1, 1));
			}
			time_text.set_foreground_color(STAGE_TEXT);
			for slider in [&position, &volume] {
				slider.set_background_color(STAGE_BG);
			}
			for btn in [&play_btn, &mute_btn, &speed_btn] {
				crate::ui::owner_draw::install_icon_button(btn, STAGE_BG);
			}
			let side = speed_btn.get_min_size().height;
			speed_btn.set_min_size(Size::new(side * 2, side));
			volume.set_min_size(Size::new(90, -1));
			controls.add(&play_btn, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 6);
			controls.add(&position_label, 0, SizerFlag::AlignCenterVertical, 0);
			controls.add(&position, 1, SizerFlag::AlignCenterVertical | SizerFlag::Right, 8);
			controls.add(&time_text, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 12);
			controls.add(&mute_btn, 0, SizerFlag::AlignCenterVertical, 0);
			controls.add(&volume_label, 0, SizerFlag::AlignCenterVertical, 0);
			controls.add(&volume, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 8);
			controls.add(&speed_btn, 0, SizerFlag::AlignCenterVertical, 0);
			column.add_sizer(
				&controls,
				0,
				SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom,
				6,
			);
		} else {
			// High contrast keeps the plain two-row layout in Windows' colours.
			theme::style_primary_button(&play_btn);
			controls.add(&position_label, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 8);
			controls.add(&position, 1, SizerFlag::Expand | SizerFlag::Right, 8);
			controls.add(&time_text, 0, SizerFlag::AlignCenterVertical, 0);
			column.add_sizer(&controls, 0, SizerFlag::Expand | SizerFlag::Bottom, 8);
			let button_row = BoxSizer::builder(Orientation::Horizontal).build();
			button_row.add(&play_btn, 0, SizerFlag::Right, 8);
			button_row.add(&mute_btn, 0, SizerFlag::Right, 8);
			button_row.add(&speed_btn, 0, SizerFlag::Right, 16);
			button_row.add(&volume_label, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 8);
			button_row.add(&volume, 1, SizerFlag::Expand, 0);
			column.add_sizer(&button_row, 0, SizerFlag::Expand, 0);
		}

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

	pub(crate) fn on_ended(&self, action: impl Fn() + 'static) {
		*self.on_ended.borrow_mut() = Some(Box::new(action));
	}

	pub(crate) fn load(&self, path: &str, autoplay: bool) {
		self.stop();
		self.autoplay.set(autoplay);
		match MediaEngine::open(self.video.get_handle(), path) {
			Ok(engine) => {
				engine.set_volume(self.volume.get_value() as f64 / 100.0);
				engine.set_rate(SPEEDS[self.speed.get()]);
				engine.set_muted(self.mute_btn.get_label() == "Unmute");
				*self.engine.borrow_mut() = Some(engine);
				if autoplay {
					self.play_btn.set_label("Pause");
				}
				self.timer.start(TICK_MS, false);
			}
			Err(err) => {
				crate::platform::announce::announce(self.announcer, "The video couldn't be played.");
				eprintln!("Media engine failed to open {path}: {err}");
			}
		}
	}

	pub(crate) fn stop(&self) {
		self.timer.stop();
		let engine = self.engine.borrow_mut().take();
		drop(engine);
		self.length.set(0);
		self.position.set_range(0, 1);
		self.position.set_value(0);
		self.time_text.set_label("0:00 of 0:00");
		self.play_btn.set_label("Play");
	}

	/// How far the video has played, 0 to 1, while one is loaded.
	pub(crate) fn progress(&self) -> Option<f64> {
		let engine = self.engine.borrow();
		let engine = engine.as_ref()?;
		let length = engine.duration();
		Some(if length > 0.0 { (engine.position() / length).clamp(0.0, 1.0) } else { 0.0 })
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
			crate::platform::announce::announce(self.announcer, "Paused");
		} else {
			self.play();
			crate::platform::announce::announce(self.announcer, "Playing");
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
		crate::platform::announce::announce(self.announcer, if muted { "Muted" } else { "Sound on" });
	}

	fn next_speed(&self) {
		let next = (self.speed.get() + 1) % SPEEDS.len();
		self.speed.set(next);
		let label = format!("Speed {}x", SPEEDS[next]);
		self.speed_btn.set_label(&label);
		if let Some(engine) = self.engine.borrow().as_ref() {
			engine.set_rate(SPEEDS[next]);
		}
		crate::platform::announce::announce(self.announcer, &label);
	}

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
		crate::platform::announce::announce(self.announcer, &clock(at as i32));
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
		crate::platform::announce::announce(self.announcer, &format!("Volume {value}%"));
	}

	pub(crate) fn speak_time(&self) {
		crate::platform::announce::announce(self.announcer, &self.time_label());
	}

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
			crate::platform::announce::announce(self.announcer, error);
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

	pub(crate) fn owns_arrows(&self, hwnd: *mut std::ffi::c_void) -> bool {
		self.is_loaded() && (hwnd == self.position.get_handle() || hwnd == self.volume.get_handle())
	}
}

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

pub(crate) fn focused_window() -> *mut std::ffi::c_void {
	unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetFocus().0 }
}

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
