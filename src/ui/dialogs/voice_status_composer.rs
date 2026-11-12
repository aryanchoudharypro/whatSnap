use std::{
	cell::RefCell,
	rc::Rc,
	sync::{Arc, Mutex},
	time::{Duration, Instant},
};

use wxdragon::prelude::*;

use crate::{
	AppState,
	audio::{compute_voice_waveform, convert_wav_to_opus_ogg, voice_recorder::RecordingSession},
	ui::{dialog_look, keys, theme, window::WindowParts},
};

use super::status_composer::STATUS_PALETTE;

const MAX_VOICE_STATUS: Duration = Duration::from_secs(30);

type PendingStart = Arc<Mutex<Option<(u32, crate::error::Result<RecordingSession>)>>>;

#[derive(Default)]
struct Recorder {
	session: Option<RecordingSession>,
	started: Option<Instant>,
	path: Option<String>,
	seconds: u64,
	opening: bool,
	generation: u32,
}

impl Recorder {
	fn stop(&mut self) {
		if let Some(session) = self.session.take() {
			session.finish();
		}
		if let Some(started) = self.started.take() {
			self.seconds = started.elapsed().min(MAX_VOICE_STATUS).as_secs().max(1);
		}
	}

	fn discard(&mut self) {
		self.stop();
		self.generation += 1;
		self.opening = false;
		if let Some(path) = self.path.take() {
			let _ = std::fs::remove_file(path);
		}
		self.seconds = 0;
	}
}

fn clock(secs: u64) -> String {
	format!("{}:{:02}", secs / 60, secs % 60)
}

pub fn show_voice_status_composer(parent: &impl WxWidget, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let dialog = Dialog::builder(parent, crate::i18n::tr("Post Voice Status")).with_size(460, 260).build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let announcer = StaticText::builder(&panel).with_label("").build();
	live_region::set_live_region(&announcer);
	sizer.add(&announcer, 0, SizerFlag::All, 0);

	let info = StaticText::builder(&panel)
		.with_label(crate::i18n::tr("Press Record and speak. A voice status can be up to 30 seconds."))
		.build();
	theme::style_heading(&info);
	sizer.add(&info, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Top | SizerFlag::Right, 12);

	let record_btn = Button::builder(&panel).with_label(crate::i18n::tr("&Record")).build();
	dialog_look::pill_button(&record_btn, true, theme::Palette::CONTENT_BG());
	sizer.add(&record_btn, 0, SizerFlag::Left | SizerFlag::Top, 12);

	let privacy_row = BoxSizer::builder(Orientation::Horizontal).build();
	let privacy_hint =
		StaticText::builder(&panel).with_label(crate::i18n::tr("Visible to your configured status audience.")).build();
	theme::style_muted_label(&privacy_hint);
	privacy_row.add(&privacy_hint, 1, SizerFlag::AlignCenterVertical, 0);
	let privacy_btn = Button::builder(&panel).with_label(crate::i18n::tr("Status &Privacy...")).build();
	dialog_look::pill_button(&privacy_btn, false, theme::Palette::CONTENT_BG());
	privacy_row.add(&privacy_btn, 0, SizerFlag::AlignCenterVertical | SizerFlag::Left, 8);
	sizer.add_sizer(&privacy_row, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);

	let btn_row = BoxSizer::builder(Orientation::Horizontal).build();
	btn_row.add_stretch_spacer(1);
	let cancel_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label(crate::i18n::tr("Cancel")).build();
	dialog_look::pill_button(&cancel_btn, false, theme::Palette::CONTENT_BG());
	btn_row.add(&cancel_btn, 0, SizerFlag::Right, 8);
	let post_btn = Button::builder(&panel).with_id(ID_OK).with_label(crate::i18n::tr("Post &Status")).build();
	dialog_look::pill_button(&post_btn, false, theme::Palette::CONTENT_BG());
	btn_row.add(&post_btn, 0, SizerFlag::Right, 0);
	sizer.add_sizer(&btn_row, 0, SizerFlag::Expand | SizerFlag::All, 12);

	panel.set_sizer(sizer, true);
	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();

	let recorder = Rc::new(RefCell::new(Recorder::default()));
	let pending: PendingStart = Arc::new(Mutex::new(None));
	let timer = Rc::new(Timer::new(&dialog));
	let weak_timer = || {
		let weak = Rc::downgrade(&timer);
		move || weak.upgrade()
	};

	let stop_recording = {
		let recorder = recorder.clone();
		let timer = weak_timer();
		Rc::new(move |text: String| {
			recorder.borrow_mut().stop();
			if let Some(t) = timer() {
				t.stop();
			}
			record_btn.set_label(crate::i18n::tr("&Record again"));
			info.set_label(&crate::i18n::tr_fmt("Recorded {}.", &[&clock(recorder.borrow().seconds)]));
			crate::platform::announce::announce(announcer, &text);
		})
	};

	{
		let recorder = recorder.clone();
		let pending = pending.clone();
		let stop_recording = stop_recording.clone();
		let weak = weak_timer();
		timer.on_tick(move |_| {
			if let Some((generation, result)) = pending.lock().unwrap().take() {
				let mut r = recorder.borrow_mut();
				if generation != r.generation {
					if let Ok(session) = result {
						session.finish();
					}
					return;
				}
				r.opening = false;
				record_btn.enable(true);
				match result {
					Ok(session) => {
						r.session = Some(session);
						r.started = Some(Instant::now());
						record_btn.set_label(crate::i18n::tr("&Stop"));
						crate::platform::announce::announce(announcer, crate::i18n::tr("Recording..."));
					}
					Err(e) => {
						eprintln!("[LOG UI] Failed to start voice status recording: {e:?}");
						r.path = None;
						if let Some(t) = weak() {
							t.stop();
						}
						crate::platform::announce::announce(
							announcer,
							crate::i18n::tr("Failed to start microphone recording."),
						);
					}
				}
				return;
			}
			let elapsed = recorder.borrow().started.map(|s| s.elapsed());
			if let Some(elapsed) = elapsed {
				if elapsed >= MAX_VOICE_STATUS {
					stop_recording(crate::i18n::tr("30 seconds reached. Recording stopped.").to_string());
				} else {
					info.set_label(&crate::i18n::tr_fmt("Recording {}", &[&clock(elapsed.as_secs())]));
				}
			}
		});
	}

	{
		let recorder = recorder.clone();
		let state = state.clone();
		let pending = pending.clone();
		let stop_recording = stop_recording.clone();
		let timer = weak_timer();
		record_btn.on_click(move |_| {
			if recorder.borrow().session.is_some() {
				let secs = recorder.borrow().started.map(|s| s.elapsed().as_secs()).unwrap_or(0).max(1);
				stop_recording(crate::i18n::tr_fmt("Recorded {}. Press Post Status to post it.", &[&clock(secs)]));
				post_btn.set_focus();
				return;
			}
			if recorder.borrow().opening {
				return;
			}
			let mut r = recorder.borrow_mut();
			r.discard();
			let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
			let path = std::env::temp_dir().join(format!("voice_status_{ts}.wav")).to_string_lossy().to_string();
			r.path = Some(path.clone());
			r.opening = true;
			let generation = r.generation;
			drop(r);
			record_btn.enable(false);
			if let Some(t) = timer() {
				t.start(200, false);
			}
			let (device, clean_up) = {
				let s = state.borrow();
				(s.settings.input_device_name.clone(), s.settings.voice_message_noise_suppression)
			};
			let pending = pending.clone();
			RecordingSession::start_async(&path, device, clean_up, move |result| {
				*pending.lock().unwrap() = Some((generation, result));
			});
		});
	}

	{
		let state_p = state.clone();
		let parts_p = parts.clone();
		privacy_btn.on_click(move |_| {
			let payload = serde_json::json!({ "action": "get_status_privacy", "data": {} });
			let _ = state_p.borrow().ws_tx.send(payload.to_string());
			crate::platform::announce::announce(
				parts_p.conversations_parts.messages_announcer,
				crate::i18n::tr("Loading status privacy settings..."),
			);
		});
	}

	cancel_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));

	let do_post = {
		let recorder = recorder.clone();
		let state = state.clone();
		let parts = parts.clone();
		let timer = weak_timer();
		Rc::new(move || {
			let (path, seconds) = {
				let mut r = recorder.borrow_mut();
				if r.opening {
					return;
				}
				r.stop();
				if let Some(t) = timer() {
					t.stop();
				}
				match r.path.take() {
					Some(path) => (path, r.seconds),
					None => {
						crate::platform::announce::announce(announcer, crate::i18n::tr("Record something first."));
						return;
					}
				}
			};
			// The phone picks a random background for each voice status.
			let nanos =
				std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().subsec_nanos();
			let (_, background) = STATUS_PALETTE[nanos as usize % STATUS_PALETTE.len()];
			let ws_tx = state.borrow().ws_tx.clone();
			std::thread::spawn(move || {
				let waveform = compute_voice_waveform(&path);
				let ogg_path = path.replace(".wav", ".ogg");
				let converted = convert_wav_to_opus_ogg(&path, &ogg_path);
				let _ = std::fs::remove_file(&path);
				if let Err(e) = converted {
					eprintln!("[LOG UI] Couldn't encode the voice status: {e}");
					return;
				}
				let payload = serde_json::json!({
					"action": "post_voice_status",
					"data": {
						"path": ogg_path,
						"duration": seconds,
						"waveform": waveform,
						"background_argb": background,
					}
				});
				let _ = ws_tx.send(payload.to_string());
			});
			crate::platform::announce::announce(
				parts.conversations_parts.messages_announcer,
				crate::i18n::tr("Posting voice status..."),
			);
			dialog.end_modal(ID_OK);
		})
	};

	{
		let do_post = do_post.clone();
		post_btn.on_click(move |_| do_post());
	}

	dialog.bind_internal(EventType::CHAR_HOOK, move |event| {
		let data = WindowEventData::new(event);
		let WindowEventData::Keyboard(ref key) = data else { return };
		let code = key.get_key_code();
		if keys::is_escape(code) {
			dialog.end_modal(ID_CANCEL);
			data.skip(false);
			return;
		}
		if keys::is_enter(code) && key.control_down() {
			do_post();
			data.skip(false);
			return;
		}
		data.skip(true);
	});

	crate::platform::announce::announce(
		announcer,
		crate::i18n::tr("Post voice status. Press Record to start, then Stop, then Post Status or Ctrl+Enter."),
	);
	record_btn.set_focus();
	dialog.show_modal();
	timer.stop();
	recorder.borrow_mut().discard();
	dialog.destroy();
}
