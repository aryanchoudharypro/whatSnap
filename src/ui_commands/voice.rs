//! `handle_ui_command` arms for voice-message recording and playback (record/pause/
//! stop/send/discard, `VoiceRecordingReady`, playback seek/speed).

use std::{cell::RefCell, rc::Rc, thread};

use wxdragon::prelude::*;

use crate::{
	AppState, NotificationSound, PendingVoiceRecording, UiCommand,
	audio::{concat_wav_segments, convert_wav_to_opus_ogg, do_seek, ensure_audio_stream, play_notification_sound},
	ui::window::WindowParts,
	voice_recorder,
};

/// Reacts to a live "default audio device changed" notification (see
/// `audio_device_watch.rs`). Forces the cached playback stream to reopen on
/// its now-current device, reattaching any actively-playing voice message at
/// its same position/pause state so the switch is inaudible-gap-free rather
/// than silent until the next play, and nudges an active call's audio to do
/// the same on its own capture/render threads.
pub(crate) fn handle_audio_device_changed(state: &Rc<RefCell<AppState>>) {
	let mut s = state.borrow_mut();
	// Bypasses ensure_audio_stream's name-match/TTL shortcuts: this
	// notification is itself the authoritative "something changed" signal.
	s.audio_stream_opened_at = None;

	if s.playing_msg_id.is_some() {
		let was_paused = s.is_audio_paused;
		let elapsed = if was_paused {
			s.audio_elapsed_secs
		} else {
			s.audio_elapsed_secs + s.audio_start_instant.map(|i| i.elapsed().as_secs_f32()).unwrap_or(0.0)
		};
		if let Some(ref sink) = s.audio_sink {
			sink.stop();
		}
		s.audio_sink = None;
		let path = s.audio_path.clone().unwrap_or_default();
		ensure_audio_stream(&mut s);
		if let Some(ref stream_handle) = s.audio_stream {
			let new_sink = do_seek(stream_handle, &path, elapsed, s.audio_speed);
			if was_paused {
				if let Some(ref sink) = new_sink {
					sink.pause();
				}
			}
			s.audio_sink = new_sink;
		}
		s.audio_elapsed_secs = elapsed;
		s.audio_start_instant = if was_paused { None } else { Some(std::time::Instant::now()) };
	} else {
		// Nothing playing right now - just make sure the stream is fresh for
		// whatever plays next (notification sounds already open fresh every
		// time and don't need this).
		ensure_audio_stream(&mut s);
	}

	if let Some(audio) = &s.active_call_audio {
		audio.force_device_refresh();
	}
}

pub(crate) fn start_recording(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let mut s = state.borrow_mut();
	if s.selected_chat.is_some() && !s.is_voice_recording && !s.voice_recording_opening {
		let temp_dir = std::env::temp_dir();
		let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
		let wav_path = temp_dir.join(format!("voice_rec_{}_0.wav", ts));
		let wav_path_str = wav_path.to_string_lossy().to_string();

		s.voice_recording_opening = true;
		s.voice_recording_generation += 1;
		let generation = s.voice_recording_generation;
		let pending = s.voice_recording_pending.clone();
		let ui_tx = s.ui_tx.clone();
		let wav_path_for_thread = wav_path_str.clone();

		parts.conversations_parts.record_voice_btn.enable(false);

		voice_recorder::RecordingSession::start_async(
			&wav_path_str,
			s.settings.input_device_name.clone(),
			s.settings.voice_message_noise_suppression,
			move |result| {
				*pending.lock().unwrap() =
					Some(PendingVoiceRecording { generation, wav_path: wav_path_for_thread, is_resume: false, result });
				let _ = ui_tx.send(UiCommand::VoiceRecordingReady);
			},
		);
	}
}

pub(crate) fn stop_recording(_state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	live_region::announce(
		parts.conversations_parts.messages_announcer.clone(),
		"Voice recording stopped. Ready to send.",
	);
}

pub(crate) fn toggle_pause_recording(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let mut s = state.borrow_mut();
	if s.is_voice_recording && !s.voice_recording_opening {
		if s.is_voice_paused {
			// Resume by starting a brand new segment rather than a
			// pause/resume on the same capture session - segments are
			// spliced together on send.
			let temp_dir = std::env::temp_dir();
			let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
			let seg_idx = s.voice_recording_segments.len();
			let wav_path = temp_dir.join(format!("voice_rec_{}_{}.wav", ts, seg_idx));
			let wav_path_str = wav_path.to_string_lossy().to_string();

			s.voice_recording_opening = true;
			s.voice_recording_generation += 1;
			let generation = s.voice_recording_generation;
			let pending = s.voice_recording_pending.clone();
			let ui_tx = s.ui_tx.clone();
			let wav_path_for_thread = wav_path_str.clone();

			parts.conversations_parts.pause_resume_btn.enable(false);

			voice_recorder::RecordingSession::start_async(
				&wav_path_str,
				s.settings.input_device_name.clone(),
				s.settings.voice_message_noise_suppression,
				move |result| {
					*pending.lock().unwrap() = Some(PendingVoiceRecording {
						generation,
						wav_path: wav_path_for_thread,
						is_resume: true,
						result,
					});
					let _ = ui_tx.send(UiCommand::VoiceRecordingReady);
				},
			);
		} else {
			// Pause by closing off the current segment as a
			// complete, valid WAV file.
			if let Some(path) = s.voice_recording_path.take() {
				if let Some(session) = s.voice_recording_session.take() {
					session.finish();
				}
				s.voice_recording_segments.push(path);
			}
			s.is_voice_paused = true;
			s.voice_pause_instant = Some(std::time::Instant::now());
			// Unlike Discard/Send, pausing never told the backend the
			// composing state had stopped - the recipient kept seeing
			// "recording audio..." for the whole pause, not just until
			// WhatsApp's own protocol-level timeout cleared it.
			if let Some(ref jid) = s.selected_chat {
				let payload = serde_json::json!({
					"action": "send_typing",
					"data": { "jid": jid, "state": "paused", "media": "" }
				});
				let _ = s.ws_tx.send(payload.to_string());
			}
			parts.conversations_parts.pause_resume_btn.set_label("Resume recording");
			live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Voice recording paused.");
		}
	}
}

pub(crate) fn voice_recording_ready(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let pending = {
		let s = state.borrow();
		s.voice_recording_pending.lock().unwrap().take()
	};
	let Some(pending) = pending else {
		return;
	};

	let mut s = state.borrow_mut();
	if pending.generation != s.voice_recording_generation {
		// Stale: the recording was discarded/sent (or another open
		// was kicked off) before this device-open finished. Clean up
		// quietly instead of resurrecting recording state.
		drop(s);
		if let Ok(session) = pending.result {
			session.finish();
		}
		let _ = std::fs::remove_file(&pending.wav_path);
		return;
	}
	s.voice_recording_opening = false;

	if pending.is_resume {
		parts.conversations_parts.pause_resume_btn.enable(true);
		match pending.result {
			Ok(session) => {
				s.voice_recording_path = Some(pending.wav_path);
				s.voice_recording_session = Some(session);
				s.is_voice_paused = false;
				if let Some(inst) = s.voice_pause_instant.take() {
					s.voice_paused_duration += inst.elapsed();
				}
				if let Some(ref jid) = s.selected_chat {
					let payload = serde_json::json!({
						"action": "send_typing",
						"data": { "jid": jid, "state": "composing", "media": "audio" }
					});
					let _ = s.ws_tx.send(payload.to_string());
				}
				parts.conversations_parts.pause_resume_btn.set_label("Pause recording");
				live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Voice recording resumed.");
			}
			Err(e) => {
				eprintln!("[LOG UI] Failed to resume voice recording: {:?}", e);
				live_region::announce(
					parts.conversations_parts.messages_announcer.clone(),
					"Failed to resume recording.",
				);
			}
		}
	} else {
		parts.conversations_parts.record_voice_btn.enable(true);
		match pending.result {
			Ok(session) => {
				s.is_voice_recording = true;
				s.is_voice_paused = false;
				s.voice_recording_path = Some(pending.wav_path);
				s.voice_recording_session = Some(session);
				s.voice_recording_segments.clear();
				s.voice_recording_start = Some(std::time::Instant::now());
				s.voice_paused_duration = std::time::Duration::ZERO;
				s.voice_pause_instant = None;

				if let Some(ref jid) = s.selected_chat {
					let payload = serde_json::json!({
						"action": "send_typing",
						"data": { "jid": jid, "state": "composing", "media": "audio" }
					});
					let _ = s.ws_tx.send(payload.to_string());
				}

				parts.conversations_parts.message_field.show(false);
				parts.conversations_parts.add_attachment_btn.show(false);
				parts.conversations_parts.send_message_btn.show(false);
				parts.conversations_parts.record_voice_btn.show(false);
				parts.conversations_parts.discard_voice_btn.show(true);
				parts.conversations_parts.pause_resume_btn.show(true);
				parts.conversations_parts.send_voice_btn.show(true);
				parts.conversations_parts.pause_resume_btn.set_label("Pause recording");
				parts.conversations_parts.conversation_panel.layout();
				parts.conversations_parts.send_voice_btn.set_focus();
				live_region::announce(
					parts.conversations_parts.messages_announcer.clone(),
					"Recording voice message...",
				);
			}
			Err(e) => {
				eprintln!("[LOG UI] Failed to start voice recording: {:?}", e);
				live_region::announce(
					parts.conversations_parts.messages_announcer.clone(),
					"Failed to start microphone recording.",
				);
			}
		}
	}
}

pub(crate) fn send_voice_message(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let (
		segments,
		rec_start,
		paused_dur,
		pause_inst,
		is_paused,
		jid_opt,
		sounds_enabled,
		output_device_name,
		ws_tx,
		reply_id,
		reply_sender,
	) = {
		let mut s = state.borrow_mut();
		s.is_voice_recording = false;
		// Invalidate a resume's device-open still in flight, if any -
		// see the matching comment in DiscardRecording.
		s.voice_recording_generation += 1;
		s.voice_recording_opening = false;
		// If still actively recording (not paused), close off the
		// final segment as a complete, valid WAV file.
		if let Some(path) = s.voice_recording_path.take() {
			if let Some(session) = s.voice_recording_session.take() {
				session.finish();
			}
			s.voice_recording_segments.push(path);
		}
		let reply_id = s.pending_reply_id.take().unwrap_or_default();
		let reply_sender = s.pending_reply_sender.take().unwrap_or_default();
		s.pending_reply_preview = None;
		(
			std::mem::take(&mut s.voice_recording_segments),
			s.voice_recording_start.take(),
			s.voice_paused_duration,
			s.voice_pause_instant.take(),
			s.is_voice_paused,
			s.selected_chat.clone(),
			s.settings.sounds_enabled,
			s.settings.output_device_name.clone(),
			s.ws_tx.clone(),
			reply_id,
			reply_sender,
		)
	};
	parts.conversations_parts.remove_quote_btn.show(false);

	if let Some(ref jid) = jid_opt {
		let payload = serde_json::json!({
			"action": "send_typing",
			"data": { "jid": jid, "state": "paused", "media": "" }
		});
		let _ = ws_tx.send(payload.to_string());
	}

	if !segments.is_empty() {
		if let (Some(start_inst), Some(jid)) = (rec_start, jid_opt) {
			let total_elapsed = start_inst.elapsed();
			let total_paused = paused_dur
				+ if is_paused {
					pause_inst.map(|i| i.elapsed()).unwrap_or_default()
				} else {
					std::time::Duration::ZERO
				};
			let active_secs = total_elapsed.saturating_sub(total_paused).as_secs().max(1);

			thread::spawn(move || {
				// Splice pause/resume segments into one continuous
				// WAV before encoding (a single-segment recording,
				// the common case, needs no splicing).
				let is_multi_segment = segments.len() > 1;
				let wav_path = if !is_multi_segment {
					segments[0].clone()
				} else {
					let merged_path = segments[0].replace(".wav", "_merged.wav");
					let result = concat_wav_segments(&segments, &merged_path);
					// The originals are fully absorbed into
					// `merged_path` now (or, on failure, no longer
					// useful) - clean them up either way.
					for seg in &segments {
						let _ = std::fs::remove_file(seg);
					}
					match result {
						Ok(()) => merged_path,
						Err(e) => {
							eprintln!("[LOG UI] Failed to splice voice recording segments: {}", e);
							String::new()
						}
					}
				};
				if wav_path.is_empty() {
					eprintln!("[LOG UI] No usable audio to send for voice message");
					return;
				}

				let ogg_path = wav_path.replace(".wav", ".ogg");
				let (final_path, mimetype) = match convert_wav_to_opus_ogg(&wav_path, &ogg_path) {
					Ok(_) => {
						// wav_path is only an intermediate once
						// converted; if it's the sole segment itself
						// leave it (matches the single-segment path,
						// which never deletes the source recording).
						if is_multi_segment {
							let _ = std::fs::remove_file(&wav_path);
						}
						(ogg_path, "audio/ogg; codecs=opus")
					}
					Err(e) => {
						eprintln!("[LOG UI] Error converting wav to opus natively: {}", e);
						(wav_path.clone(), "audio/wav")
					}
				};

				let payload = serde_json::json!({
					"action": "send_voice_message",
					"data": {
						"jid": jid,
						"path": final_path,
						"mimetype": mimetype,
						"duration": active_secs,
						"reply_to_id": reply_id,
						"reply_to_sender": reply_sender
					}
				});
				let _ = ws_tx.send(payload.to_string());
			});

			play_notification_sound(NotificationSound::VoiceMessageSent, sounds_enabled, output_device_name);
			live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Voice message sent.");
		}
	}

	parts.conversations_parts.message_field.show(true);
	parts.conversations_parts.add_attachment_btn.show(true);
	parts.conversations_parts.record_voice_btn.show(true);
	parts.conversations_parts.record_voice_btn.enable(true);
	parts.conversations_parts.pause_resume_btn.enable(true);
	parts
		.conversations_parts
		.send_message_btn
		.show(!parts.conversations_parts.message_field.get_value().trim().is_empty());
	parts.conversations_parts.message_field.set_focus();

	parts.conversations_parts.discard_voice_btn.show(false);
	parts.conversations_parts.pause_resume_btn.show(false);
	parts.conversations_parts.send_voice_btn.show(false);

	parts.conversations_parts.conversation_panel.layout();
}

pub(crate) fn discard_recording(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	{
		let mut s = state.borrow_mut();
		s.is_voice_recording = false;
		s.is_voice_paused = false;
		// Invalidate any device-open still in flight (a resume, or the
		// initial start) - its result will arrive later via
		// VoiceRecordingReady and be cleaned up there instead of
		// resurrecting the recording state we're tearing down here.
		s.voice_recording_generation += 1;
		s.voice_recording_opening = false;
		// The in-progress segment's capture session (if any) is still
		// writing to disk - stop it, then clean up its file along with
		// any earlier segments from prior pause cycles.
		if let Some(session) = s.voice_recording_session.take() {
			session.finish();
		}
		if let Some(path) = s.voice_recording_path.take() {
			let _ = std::fs::remove_file(path);
		}
		for seg in s.voice_recording_segments.drain(..) {
			let _ = std::fs::remove_file(seg);
		}
		if let Some(ref jid) = s.selected_chat {
			let payload = serde_json::json!({
				"action": "send_typing",
				"data": { "jid": jid, "state": "paused", "media": "" }
			});
			let _ = s.ws_tx.send(payload.to_string());
		}
	}

	parts.conversations_parts.message_field.show(true);
	parts.conversations_parts.add_attachment_btn.show(true);
	parts.conversations_parts.record_voice_btn.show(true);
	parts.conversations_parts.record_voice_btn.enable(true);
	parts.conversations_parts.pause_resume_btn.enable(true);
	parts
		.conversations_parts
		.send_message_btn
		.show(!parts.conversations_parts.message_field.get_value().trim().is_empty());
	parts.conversations_parts.message_field.set_focus();

	parts.conversations_parts.discard_voice_btn.show(false);
	parts.conversations_parts.pause_resume_btn.show(false);
	parts.conversations_parts.send_voice_btn.show(false);

	parts.conversations_parts.conversation_panel.layout();
	live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Voice recording discarded.");
}

pub(crate) fn play_voice_message(state: &Rc<RefCell<AppState>>, _parts: &Rc<WindowParts>, msg_id: String) {
	let payload = serde_json::json!({
		"action": "play_voice_message",
		"data": { "id": msg_id }
	});
	let _ = state.borrow().ws_tx.send(payload.to_string());
}

pub(crate) fn seek_audio(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, delta: f32) {
	let mut s = state.borrow_mut();
	if s.playing_msg_id.is_some() {
		let elapsed = if s.is_audio_paused {
			s.audio_elapsed_secs
		} else {
			s.audio_elapsed_secs + s.audio_start_instant.map(|i| i.elapsed().as_secs_f32()).unwrap_or(0.0)
		};
		let total = s.audio_total_secs;
		let target = if total > 0.0 { (elapsed + delta).max(0.0).min(total) } else { (elapsed + delta).max(0.0) };
		if let Some(ref sink) = s.audio_sink {
			sink.stop();
		}
		s.audio_sink = None;
		let path = s.audio_path.clone().unwrap_or_default();
		if let Some(ref stream_handle) = s.audio_stream {
			let new_sink = do_seek(stream_handle, &path, target, s.audio_speed);
			s.audio_sink = new_sink;
		}
		s.audio_elapsed_secs = target;
		s.audio_start_instant = Some(std::time::Instant::now());
		s.is_audio_paused = false;
		parts.conversations_parts.play_voice_btn.set_label("Pause voice message");
		let max_sec = (total as i32).max(1);
		let pos = (target as i32).max(0).min(max_sec);
		parts.conversations_parts.audio_slider.set_range(0, max_sec);
		parts.conversations_parts.audio_slider.set_value(pos);
	}
}

pub(crate) fn seek_audio_absolute(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>, slider_val: i32) {
	let mut s = state.borrow_mut();
	if s.playing_msg_id.is_some() {
		let total = s.audio_total_secs;
		let target = (slider_val as f32).max(0.0).min(total);
		if let Some(ref sink) = s.audio_sink {
			sink.stop();
		}
		s.audio_sink = None;
		let path = s.audio_path.clone().unwrap_or_default();
		if let Some(ref stream_handle) = s.audio_stream {
			let new_sink = do_seek(stream_handle, &path, target, s.audio_speed);
			s.audio_sink = new_sink;
		}
		s.audio_elapsed_secs = target;
		s.audio_start_instant = Some(std::time::Instant::now());
		s.is_audio_paused = false;
		parts.conversations_parts.play_voice_btn.set_label("Pause voice message");
		let max_sec = (total as i32).max(1);
		let pos = (target as i32).max(0).min(max_sec);
		parts.conversations_parts.audio_slider.set_range(0, max_sec);
		parts.conversations_parts.audio_slider.set_value(pos);
	}
}

pub(crate) fn toggle_audio_speed(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let mut s = state.borrow_mut();
	let old_speed = s.audio_speed;
	let new_speed = if s.audio_speed >= 2.0 {
		1.0
	} else if s.audio_speed >= 1.5 {
		2.0
	} else {
		1.5
	};
	s.audio_speed = new_speed;
	parts.conversations_parts.audio_speed_btn.set_label(&format!("{:.1}x", new_speed));

	// Trigger soft-seek to apply new wsola speed
	if s.playing_msg_id.is_some() && s.audio_sink.is_some() {
		let was_paused = s.is_audio_paused;
		// Use the *old* speed: the wall-clock time elapsed since the
		// last seek/start was played back at the previous speed, not
		// the one we're switching to. Using the new speed here made
		// every speed change jump the position, i.e. "plays from a
		// bit before" where it actually was.
		let elapsed =
			s.audio_elapsed_secs + s.audio_start_instant.map(|i| i.elapsed().as_secs_f32() * old_speed).unwrap_or(0.0);
		let target = elapsed.max(0.0).min(s.audio_total_secs);
		if let Some(ref sink) = s.audio_sink {
			sink.stop();
		}
		s.audio_sink = None;
		let path = s.audio_path.clone().unwrap_or_default();
		if let Some(ref stream_handle) = s.audio_stream {
			let new_sink = do_seek(stream_handle, &path, target, new_speed);
			if was_paused {
				if let Some(ref sink) = new_sink {
					sink.pause();
				}
			}
			s.audio_sink = new_sink;
		}
		s.audio_elapsed_secs = target;
		s.audio_start_instant = if was_paused { None } else { Some(std::time::Instant::now()) };
		s.is_audio_paused = was_paused;
		parts.conversations_parts.play_voice_btn.set_label(if was_paused {
			"Play voice message"
		} else {
			"Pause voice message"
		});
	}
}
