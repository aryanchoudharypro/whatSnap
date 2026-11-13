use live_region::Priority;

use super::*;

pub(super) fn handle(t: &str, val: &serde_json::Value, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) -> bool {
	match t {
		"call_connected" => {
			if let Some(data) = val.get("data") {
				let call_id = data.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
				if let Some(audio_port) = data.get("audio_port").and_then(|p| p.as_u64()) {
					let mut s = state.borrow_mut();
					call_log(parts, &s, "connected", call_id);
					if s.active_call_id.as_deref() != Some(call_id) {
						return true;
					}
					s.call_answer_deadline = None;
					s.incoming_call_caller = None;
					s.call_connected_duration = std::time::Duration::ZERO;
					s.call_connected_segment_start = Some(std::time::Instant::now());
					s.call_is_reconnecting = false;
					s.call_video_port =
						data.get("video_port").and_then(|p| p.as_u64()).map(|p| p as u16).filter(|&p| p != 0);
					s.call_video.screen_port =
						data.get("screen_port").and_then(|p| p.as_u64()).map(|p| p as u16).filter(|&p| p != 0);
					stop_call_ringing_sound(&mut s);
					if let Ok(streams) = crate::calls::audio::start_audio_streams(
						audio_port as u16,
						s.settings.input_device_name.clone(),
						s.settings.call_output_device_name.clone(),
					) {
						if s.call_start_muted {
							streams.is_muted.store(true, std::sync::atomic::Ordering::Relaxed);
							parts.call_parts.mute_btn.set_label(crate::i18n::tr("Unmute microphone"));
							if let Some(call_id) = &s.active_call_id {
								let payload = serde_json::json!({
									"action": "mute_call",
									"data": { "call_id": call_id, "muted": true }
								});
								let _ = s.ws_tx.send(payload.to_string());
							}
						} else {
							parts.call_parts.mute_btn.set_label(crate::i18n::tr("Mute microphone"));
						}
						s.active_call_audio = Some(streams);
						crate::ui_commands::calls::refresh_active_call_row(&mut s, parts);
						let connected_msg = if s.call_video.is_video {
							crate::i18n::tr("Video call connected. Your camera is off.")
						} else {
							crate::i18n::tr("Call connected.")
						};
						announce_call(parts, connected_msg, Priority::High);

						if s.call_participants.is_empty() {
							parts.call_parts.status_label.set_label(connected_msg);
						} else {
							parts.call_parts.status_label.set_label(&group_call_status_text(&s.call_participants));
						}
						let running_info = crate::calls::helpers::call_running_info_text(
							s.call_info_direction(),
							s.call_video.is_video,
							&s.call_peer_name,
							0,
							false,
						);
						parts.call_parts.caller_info.set_value(&running_info);
						parts.call_parts.caller_info.show(true);
						let title = crate::calls::helpers::format_call_window_title(
							s.call_shows_incoming(),
							s.call_participants.len() > 1 || s.call_video.peer_name.is_none(),
							s.call_video.is_video,
							&s.call_peer_name,
							s.call_peer_is_saved,
							None,
						);
						parts.call_parts.frame.set_title(&title);
						if let Some(target) = s.selected_chat.as_deref() {
							crate::calls::helpers::update_call_avatar(parts, target);
						}
						parts.call_parts.accept_btn.show(false);
						parts.call_parts.reject_btn.show(false);
						parts.call_parts.dismiss_btn.show(false);
						parts.call_parts.callback_btn.show(false);
						parts.call_parts.close_btn.show(false);
						parts.conversations_parts.incoming_call_banner.show(false);
						parts.conversations_parts.panel.layout();
						parts.frame.layout();
						parts.call_parts.mute_btn.show(true);
						parts.call_parts.device_settings_btn.show(true);
						parts.call_parts.camera_btn.show(true);
						parts.call_parts.camera_btn.set_label(if s.call_video.local_camera_on {
							crate::i18n::tr("Turn camera off")
						} else {
							crate::i18n::tr("Turn camera on")
						});
						crate::ui_commands::calls::show_screen_share_controls(&s, parts);
						parts.call_parts.leave_btn.set_label(crate::i18n::tr("End call"));
						parts.call_parts.leave_btn.show(true);
						parts.call_parts.add_participant_btn.show(true);
						crate::ui_commands::call_extras::show_call_buttons(&s, parts);
						show_call_device_combos(&s, parts);
						parts.call_parts.frame.show(true);
						let autostart_camera = s.call_video.autostart_camera;
						s.call_video.autostart_camera = false;
						drop(s);
						if autostart_camera {
							crate::ui_commands::calls::toggle_camera(state, parts);
						}
					} else {
						announce_call(parts, crate::i18n::tr("Failed to start audio stream."), Priority::High);
					}
				}
			}
		}
		"call_joinable" => {
			if let Some(data) = val.get("data") {
				crate::ui_commands::calls::on_joinable_calls(state, parts, data);
			}
		}
		"call_progress" => {
			if let Some(data) = val.get("data") {
				crate::ui_commands::calls::on_call_progress(state, parts, data);
			}
		}
		"call_placed" => {
			if let Some(data) = val.get("data")
				&& let Some(call_id) = data.get("call_id").and_then(|c| c.as_str())
			{
				let mut s = state.borrow_mut();
				call_log(parts, &s, "placed", call_id);
				s.call_placing = false;
				if s.call_pending_cancel {
					s.call_pending_cancel = false;
					let msg = serde_json::json!({
						"action": "leave_call",
						"data": { "call_id": call_id }
					});
					let _ = s.ws_tx.send(msg.to_string());
				} else {
					s.active_call_id = Some(call_id.to_string());
					crate::ui_commands::calls::refresh_active_call_row(&mut s, parts);
					s.call_participants.clear();
					s.active_call_video_send = None;
					s.call_video_port = None;
					if let Some(pn) = data.get("peer_name").and_then(|p| p.as_str()) {
						if !pn.is_empty()
							&& !pn.starts_with('+')
							&& (s.call_peer_name.is_empty()
								|| s.call_peer_name.starts_with('+')
								|| s.call_peer_name
									.chars()
									.all(|c| c.is_ascii_digit() || c.is_whitespace() || c == '-'))
						{
							s.call_peer_name = pn.to_string();
							let win_title = crate::calls::helpers::format_call_window_title(
								false,
								false,
								s.call_video.is_video,
								&s.call_peer_name,
								s.call_peer_is_saved,
								None,
							);
							parts.call_parts.frame.set_title(&win_title);
						}
					}
					if let Some(self_phone) = data.get("self_phone").and_then(|p| p.as_str()) {
						s.call_self_phone = self_phone.to_string();
					}
					// The ringback plays from the moment the call is placed, as it
					// always did: the peer's "ringing" notice is not guaranteed to
					// reach us, and a silent outgoing call is worse than an early tone.
					if data.get("link").and_then(|l| l.as_bool()) != Some(true) && !s.call_ringback_on {
						let self_phone = s.call_self_phone.clone();
						start_call_ringback(&mut s, &self_phone);
					}
				}
			}
		}
		"call_audio_health" => {
			if let Some(data) = val.get("data") {
				let call_id = data.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
				if state.borrow().active_call_id.as_deref() != Some(call_id) {
					return true;
				}
				if state.borrow().call_reconnect_sink.is_some() {
					return true;
				}
				let stalled = data.get("stalled").and_then(|s| s.as_bool()).unwrap_or(false);
				let message = if stalled {
					crate::i18n::tr("No sound is reaching you from the call. Check your internet connection.")
				} else {
					crate::i18n::tr("Sound from the call isn't getting through.")
				};
				announce_call(parts, message, Priority::High);
			}
		}
		"call_peer_interrupted" | "call_peer_battery_low" => {
			if let Some(data) = val.get("data") {
				let call_id = data.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
				let s = state.borrow();
				if s.active_call_id.as_deref() == Some(call_id) {
					let name = if s.call_peer_name.is_empty() {
						crate::i18n::tr("The other person").to_string()
					} else {
						s.call_peer_name.clone()
					};
					let message = if t == "call_peer_battery_low" {
						crate::i18n::tr_fmt("{}'s battery is low.", &[&name])
					} else if data.get("interrupted").and_then(|i| i.as_bool()).unwrap_or(true) {
						crate::i18n::tr_fmt("{} is on a phone call and can't hear you right now.", &[&name])
					} else {
						crate::i18n::tr_fmt("{} is back on the call.", &[&name])
					};
					drop(s);
					announce_call(parts, &message, Priority::High);
				}
			}
		}
		"call_loss" => {
			if let Some(data) = val.get("data") {
				let call_id = data.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
				if state.borrow().active_call_id.as_deref() == Some(call_id) {
					let percent = data.get("percent").and_then(|p| p.as_i64()).unwrap_or(0) as i32;
					crate::calls::audio::set_call_packet_loss_percent(percent);
				}
			}
		}
		"call_reconnecting" => {
			if let Some(data) = val.get("data") {
				let call_id = data.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
				let mut s = state.borrow_mut();
				if s.active_call_id.as_deref() != Some(call_id) {
					return true;
				}
				let is_reconnecting = data.get("reconnecting").and_then(|r| r.as_bool()).unwrap_or(false);
				if is_reconnecting {
					if let Some(start) = s.call_connected_segment_start.take() {
						s.call_connected_duration += start.elapsed();
					}
					s.call_is_reconnecting = true;
					let secs = s.call_connected_duration.as_secs();
					let info = crate::calls::helpers::call_running_info_text(
						s.call_info_direction(),
						s.call_video.is_video,
						&s.call_peer_name,
						secs,
						true,
					);
					parts.call_parts.caller_info.set_value(&info);
					parts.call_parts.caller_info.show(true);
					parts.call_parts.status_label.set_label(crate::i18n::tr("Reconnecting..."));
					crate::audio::start_call_reconnect_tone(&mut s);
					announce_call(parts, crate::i18n::tr("Reconnecting..."), Priority::High);
				} else {
					s.call_is_reconnecting = false;
					s.call_connected_segment_start = Some(std::time::Instant::now());
					let secs = s.call_connected_duration.as_secs();
					let info = crate::calls::helpers::call_running_info_text(
						s.call_info_direction(),
						s.call_video.is_video,
						&s.call_peer_name,
						secs,
						false,
					);
					parts.call_parts.caller_info.set_value(&info);
					parts.call_parts.caller_info.show(true);
					parts.call_parts.status_label.set_label(crate::i18n::tr("Connected"));
					crate::audio::stop_call_reconnect_tone(&mut s);
					announce_call(parts, crate::i18n::tr("Reconnected."), Priority::High);
				}
			}
		}
		"call_participant_added" | "call_participant_add_failed" => {
			if let Some(data) = val.get("data") {
				let call_id = data.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
				if state.borrow().active_call_id.as_deref() != Some(call_id) {
					return true;
				}
				let message = if t == "call_participant_added" {
					let name = data.get("name").and_then(|n| n.as_str()).unwrap_or_else(|| crate::i18n::tr("Contact"));
					crate::i18n::tr_fmt("Ringing {}.", &[&name])
				} else {
					data.get("message")
						.and_then(|m| m.as_str())
						.map(crate::i18n::translate_backend_message)
						.unwrap_or_else(|| crate::i18n::tr("Couldn't add that person to the call.").to_string())
				};
				announce_call(parts, &message, Priority::High);
			}
		}
		"call_participants" => {
			if let Some(data) = val.get("data") {
				let call_id = data.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
				let mut s = state.borrow_mut();
				if s.active_call_id.as_deref() != Some(call_id) {
					return true;
				}
				let participants = parse_call_participants(data);
				let changes = if s.call_video.roster_seen {
					call_participant_changes(&s.call_participants, &participants)
				} else {
					Vec::new()
				};
				s.call_video.roster_seen = true;
				s.call_video.can_remove = data.get("can_remove").and_then(|v| v.as_bool()).unwrap_or(false);
				s.call_participants = participants;
				crate::ui_commands::call_extras::refresh_people(&s, parts);
				if s.incoming_call_caller.is_none() && s.call_answer_deadline.is_none() {
					parts.call_parts.status_label.set_label(&group_call_status_text(&s.call_participants));
				}
				drop(s);
				if !changes.is_empty() {
					announce_call(parts, &changes.join(" "), Priority::High);
				}
			}
		}
		"call_video_state" => {
			if let Some(data) = val.get("data") {
				let call_id = data.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
				let event = data.get("peer_video").and_then(|e| e.as_str()).unwrap_or("");
				let mut s = state.borrow_mut();
				if s.active_call_id.as_deref() != Some(call_id) {
					return true;
				}
				let announcement =
					peer_video_announcement(event, s.call_video.peer_camera_on, s.call_video.peer_name.as_deref());
				match event {
					"on" => {
						s.call_video.peer_camera_on = Some(true);
						if !s.call_video.video_call {
							s.call_video.video_call = true;
							s.call_video.is_video = true;
							if s.active_call_audio.is_some() {
								crate::ui_commands::calls::show_screen_share_controls(&s, parts);
							}
						}
						let is_group = s.call_participants.len() > 1 || s.call_video.peer_name.is_none();
						let name = s.call_peer_name.clone();
						let is_saved = s.call_peer_is_saved;
						let is_incoming = s.call_shows_incoming();
						let direction = s.call_info_direction();
						let elapsed = s.call_connected_duration
							+ s.call_connected_segment_start.map(|t| t.elapsed()).unwrap_or_default();
						let secs = elapsed.as_secs();
						let clock =
							if secs > 0 { Some(crate::calls::helpers::format_call_duration(secs)) } else { None };
						let win_title = crate::calls::helpers::format_call_window_title(
							is_incoming,
							is_group,
							true,
							&name,
							is_saved,
							clock.as_deref(),
						);
						parts.call_parts.frame.set_title(&win_title);
						let running_info = crate::calls::helpers::call_running_info_text(
							direction,
							true,
							&name,
							secs,
							s.call_is_reconnecting,
						);
						parts.call_parts.caller_info.set_value(&running_info);
						crate::calls::helpers::show_call_device_combos(&s, parts);
					}
					"off" => s.call_video.peer_camera_on = Some(false),
					_ => {}
				}
				drop(s);
				if let Some(text) = announcement {
					announce_call(parts, &text, Priority::High);
				}
			}
		}
		"call_video_keyframe_request" => {
			if let Some(data) = val.get("data") {
				let call_id = data.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
				let s = state.borrow();
				if s.active_call_id.as_deref() == Some(call_id)
					&& let Some(send) = &s.active_call_video_send
				{
					send.request_keyframe();
				}
			}
		}
		"call_screen_keyframe_request" => {
			if let Some(data) = val.get("data") {
				let call_id = data.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
				let s = state.borrow();
				if s.active_call_id.as_deref() == Some(call_id)
					&& let Some(send) = &s.call_video.screen_send
				{
					send.request_keyframe();
				}
			}
		}
		"call_screen_share_failed" => {
			if let Some(data) = val.get("data") {
				let call_id = data.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
				if state.borrow().active_call_id.as_deref() != Some(call_id) {
					return true;
				}
				let reason =
					crate::i18n::translate_backend_message(data.get("message").and_then(|m| m.as_str()).unwrap_or(""));
				crate::ui_commands::calls::screen_share_failed(state, parts, reason);
			}
		}
		"call_set_video_failed" => {
			if let Some(data) = val.get("data") {
				let call_id = data.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
				let mut s = state.borrow_mut();
				if s.active_call_id.as_deref() != Some(call_id) {
					return true;
				}
				let was_sharing = s.call_video.local_screen_on;
				s.active_call_video_send = None;
				s.call_video.screen_send = None;
				s.call_video.local_camera_on = false;
				s.call_video.local_screen_on = false;
				s.call_video.camera_before_share = false;
				crate::ui_commands::calls::set_system_audio(&mut s, false);
				if was_sharing {
					let msg = serde_json::json!({
						"action": "set_screen_share",
						"data": { "call_id": call_id, "enabled": false }
					});
					let _ = s.ws_tx.send(msg.to_string());
				}
				parts.call_parts.camera_btn.set_label(crate::i18n::tr("Turn camera on"));
				parts.call_parts.screen_btn.set_label(crate::i18n::tr("Share screen"));
				drop(s);
				announce_call(
					parts,
					if was_sharing {
						crate::i18n::tr("Couldn't share the screen on this call.")
					} else {
						crate::i18n::tr("Couldn't turn the camera on for this call.")
					},
					Priority::High,
				);
			}
		}
		"call_screen_share" => {
			if let Some(data) = val.get("data") {
				let call_id = data.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
				let name = data.get("name").and_then(|n| n.as_str()).unwrap_or("");
				let active = data.get("active").and_then(|a| a.as_bool()).unwrap_or(false);
				if state.borrow().active_call_id.as_deref() != Some(call_id) {
					return true;
				}
				announce_call(
					parts,
					&crate::calls::helpers::peer_screen_share_announcement(name, active),
					Priority::High,
				);
			}
		}
		"call_failed" => {
			let mut s = state.borrow_mut();
			if s.call_placing {
				s.call_placing = false;
				s.call_pending_cancel = false;
				s.call_connected_segment_start = None;
				s.call_connected_duration = std::time::Duration::ZERO;
				s.call_is_reconnecting = false;
				s.call_is_incoming = false;
				s.call_start_muted = false;
				s.call_peer_name.clear();
				s.call_video = CallVideo::default();
				s.active_call_video_send = None;
				s.call_video_port = None;
				parts.conversations_parts.incoming_call_banner.show(false);
				parts.conversations_parts.panel.layout();
				parts.frame.layout();
				parts.call_parts.frame.show(false);
				stop_call_ringing_sound(&mut s);
				play_call_fail_sound(&mut s);
				let message = val
					.get("data")
					.and_then(|d| d.get("message"))
					.and_then(|m| m.as_str())
					.map(crate::i18n::translate_backend_message)
					.unwrap_or_else(|| crate::i18n::tr("Failed to place call.").to_string());
				announce_call(parts, &message, Priority::High);
			}
		}
		"call_ended" => {
			if let Some(data) = val.get("data")
				&& let Some(call_id) = data.get("call_id").and_then(|c| c.as_str())
			{
				let mut s = state.borrow_mut();
				call_log(parts, &s, "ended", call_id);
				if s.active_call_id.as_deref() == Some(call_id) {
					let total_duration = if let Some(start) = s.call_connected_segment_start.take() {
						s.call_connected_duration + start.elapsed()
					} else {
						s.call_connected_duration
					};
					let peer_name_saved = s.call_peer_name.clone();
					let was_incoming_ringing = s.incoming_call_caller.is_some() && s.active_call_audio.is_none();
					let was_frame_shown = parts.call_parts.frame.is_shown();
					let was_connected = total_duration > std::time::Duration::ZERO || s.active_call_audio.is_some();
					parts.conversations_parts.incoming_call_banner.show(false);
					parts.conversations_parts.panel.layout();
					parts.frame.layout();

					crate::audio::stop_call_reconnect_tone(&mut s);
					s.active_call_id = None;
					crate::ui_commands::calls::refresh_active_call_row(&mut s, parts);
					s.active_call_audio = None;
					s.call_connected_segment_start = None;
					s.call_connected_duration = std::time::Duration::ZERO;
					s.call_is_reconnecting = false;
					s.call_is_incoming = false;
					s.call_start_muted = false;
					s.call_participants.clear();
					s.call_video = CallVideo::default();
					s.active_call_video_send = None;
					s.call_video_port = None;
					s.call_answer_deadline = None;
					s.call_self_phone.clear();
					stop_call_ringing_sound(&mut s);
					let reason = data.get("reason").and_then(|r| r.as_str()).unwrap_or("");
					if !was_connected && reason == "busy" && s.incoming_call_caller.is_none() {
						crate::audio::play_busy_tone(&mut s);
					} else {
						play_call_fail_sound(&mut s);
					}
					let summary = data.get("summary").and_then(|s| s.as_str()).unwrap_or("");
					let had_caller = s.incoming_call_caller.take().is_some();
					if !had_caller {
						announce_call(
							parts,
							if was_connected {
								if reason == "connection_lost" {
									crate::i18n::tr("Call ended. Connection lost.")
								} else {
									crate::i18n::tr("Call ended.")
								}
							} else {
								match reason {
									"busy" => crate::i18n::tr("Busy, on another call."),
									"declined" | "rejected" => crate::i18n::tr("Call declined."),
									"cancelled" => crate::i18n::tr("Call cancelled."),
									"timeout" | "no_answer" => crate::i18n::tr("No answer."),
									"connection_lost" => crate::i18n::tr("Call ended. Connection lost."),
									_ => crate::i18n::tr("Call ended."),
								}
							},
							Priority::High,
						);
					} else if summary.contains("on another device") {
						let translated_summary = crate::i18n::translate_call_kind_text(summary);
						announce_call(parts, &format!("{translated_summary}."), Priority::High);
					} else if was_incoming_ringing && !was_frame_shown {
						let msg = crate::i18n::tr_fmt("Missed call from {}.", &[&peer_name_saved]);
						announce_call(parts, &msg, Priority::High);
					}

					// Only a missed call (it rang and was never answered) keeps the
					// ended screen with a Call back button. Declined, connected and
					// outgoing calls close the window as before.
					let was_missed = was_incoming_ringing
						&& !was_connected
						&& matches!(reason, "timeout" | "no_answer" | "cancelled");
					if was_frame_shown && was_missed {
						let end_text = if was_connected {
							let clock = crate::calls::helpers::format_call_duration(total_duration.as_secs());
							crate::i18n::tr_fmt("Call ended with {}\r\nDuration: {}", &[&peer_name_saved, &clock])
						} else {
							crate::calls::helpers::call_ended_info_text(
								&peer_name_saved,
								total_duration.as_secs(),
								reason,
							)
						};
						let title_name =
							crate::calls::helpers::clean_call_title_peer_name(&peer_name_saved, s.call_peer_is_saved);
						let end_title = crate::i18n::tr_fmt("Call ended with {}", &[&title_name]);
						parts.call_parts.frame.set_title(&end_title);
						parts.call_parts.status_label.set_label(crate::i18n::tr("Call ended"));
						parts.call_parts.caller_info.set_value(&end_text);
						parts.call_parts.caller_info.show(true);
						parts.call_parts.accept_btn.show(false);
						parts.call_parts.reject_btn.show(false);
						parts.call_parts.dismiss_btn.show(false);
						parts.call_parts.mute_btn.show(false);
						parts.call_parts.camera_btn.show(false);
						parts.call_parts.screen_btn.show(false);
						parts.call_parts.device_settings_btn.show(false);
						parts.call_parts.device_settings_btn.set_label(crate::i18n::tr("Device settings"));
						parts.call_parts.leave_btn.show(false);
						parts.call_parts.add_participant_btn.show(false);
						parts.call_parts.raise_hand_btn.show(false);
						parts.call_parts.react_btn.show(false);
						parts.call_parts.people_panel.show(false);
						parts.call_parts.input_device_group.show(false);
						parts.call_parts.output_device_group.show(false);
						parts.call_parts.camera_device_group.show(false);
						parts.call_parts.callback_btn.show(true);
						parts.call_parts.close_btn.show(true);
						parts.call_parts.close_btn.set_focus();
						parts.call_parts.frame.layout();
					} else {
						parts.call_parts.frame.show(false);
					}

					if s.active_tab == 6 {
						crate::ui_commands::chat_list::request_call_tab_data(&s);
					}
				}
			}
		}
		"call_answer_failed" => {
			if let Some(data) = val.get("data")
				&& let Some(call_id) = data.get("call_id").and_then(|c| c.as_str())
			{
				let mut s = state.borrow_mut();
				if s.active_call_id.as_deref() == Some(call_id) {
					s.active_call_id = None;
					crate::ui_commands::calls::refresh_active_call_row(&mut s, parts);
					s.active_call_audio = None;
					s.call_connected_segment_start = None;
					s.call_connected_duration = std::time::Duration::ZERO;
					s.call_is_reconnecting = false;
					s.call_is_incoming = false;
					s.call_start_muted = false;
					s.call_peer_name.clear();
					s.call_participants.clear();
					s.call_video = CallVideo::default();
					s.active_call_video_send = None;
					s.call_video_port = None;
					s.call_answer_deadline = None;
					s.incoming_call_caller = None;
					parts.conversations_parts.incoming_call_banner.show(false);
					parts.conversations_parts.panel.layout();
					parts.frame.layout();
					parts.call_parts.frame.show(false);
					stop_call_ringing_sound(&mut s);
					play_call_fail_sound(&mut s);
					let message = data
						.get("message")
						.and_then(|m| m.as_str())
						.map(crate::i18n::translate_backend_message)
						.unwrap_or_else(|| crate::i18n::tr("Failed to answer call.").to_string());
					announce_call(parts, &message, Priority::High);
				}
			}
		}
		"incoming_call" => {
			if let Some(data) = val.get("data") {
				let caller = data.get("caller").and_then(|c| c.as_str()).unwrap_or("Unknown");
				if let Some(call_id) = data.get("call_id").and_then(|c| c.as_str()) {
					let mut s = state.borrow_mut();
					call_log(parts, &s, "incoming", call_id);
					s.active_call_id = Some(call_id.to_string());
					crate::ui_commands::calls::refresh_active_call_row(&mut s, parts);
					s.call_answer_deadline = None;
					let chat_jid = data.get("chat_jid").and_then(|j| j.as_str());
					let raw_name = data.get("caller_name").and_then(|n| n.as_str());
					let raw_phone = data.get("caller_phone").and_then(|p| p.as_str());
					let (resolved_name, phone_digits) = crate::calls::helpers::resolve_call_peer_name_and_phone(
						&s.contacts,
						&s.all_contacts,
						caller,
						chat_jid,
						raw_name,
						raw_phone,
					);
					let name =
						if resolved_name == "Unknown" { crate::i18n::tr("Unknown").to_string() } else { resolved_name };
					let is_saved = s.contacts.contains_key(caller)
						|| chat_jid.map(|j| s.contacts.contains_key(j)).unwrap_or(false)
						|| s.all_contacts
							.iter()
							.any(|c| (c.jid == caller || chat_jid == Some(&c.jid)) && !c.name.is_empty());
					s.call_peer_is_saved = is_saved;
					s.incoming_call_caller = Some(name.clone());
					s.call_is_incoming = true;
					s.call_peer_name = name.clone();
					s.call_start_muted = s.settings.call_start_muted;
					parts.call_parts.mute_btn.set_label(if s.call_start_muted {
						crate::i18n::tr("Unmute microphone")
					} else {
						crate::i18n::tr("Mute microphone")
					});
					s.call_connected_segment_start = None;
					s.call_connected_duration = std::time::Duration::ZERO;
					s.call_is_reconnecting = false;
					s.call_participants.clear();
					s.call_video = CallVideo::default();
					s.active_call_video_send = None;
					s.call_video_port = None;
					let is_group = data.get("is_group").and_then(|g| g.as_bool()).unwrap_or(false);
					let group_name = data
						.get("group_name")
						.and_then(|n| n.as_str())
						.filter(|n| !n.is_empty())
						.map(|n| n.to_string())
						.or_else(|| {
							let group_jid = data.get("group_jid").and_then(|j| j.as_str())?;
							s.display_names.get(group_jid).or_else(|| s.contacts.get(group_jid)).cloned()
						});
					let is_video = data.get("is_video").and_then(|v| v.as_bool()).unwrap_or(false);
					s.call_last_target = Some((chat_jid.unwrap_or(caller).to_string(), is_video));
					let description = incoming_call_description(is_group, is_video, &name, group_name.as_deref());
					s.call_video = CallVideo {
						is_video,
						peer_name: if is_group { None } else { Some(name.clone()) },
						peer_camera_on: Some(is_video),
						local_camera_on: false,
						local_screen_on: false,
						camera_before_share: false,
						autostart_camera: false,
						video_call: is_video,
						..Default::default()
					};
					let win_title = crate::calls::helpers::format_call_window_title(
						true,
						is_group,
						is_video,
						group_name.as_deref().unwrap_or(&name),
						is_saved,
						None,
					);
					let avatar_target = chat_jid.unwrap_or(caller);
					crate::calls::helpers::update_call_avatar(parts, avatar_target);
					parts.call_parts.frame.set_title(&win_title);
					parts.call_parts.status_label.set_label(&win_title);
					parts.call_parts.accept_btn.show(true);
					parts.call_parts.reject_btn.show(true);
					parts.call_parts.dismiss_btn.show(true);
					parts.call_parts.callback_btn.show(false);
					parts.call_parts.close_btn.show(false);
					crate::ui_commands::call_extras::hide_call_buttons(parts);
					parts.call_parts.caller_info.set_value(&crate::calls::helpers::call_incoming_info_text(
						&name,
						phone_digits.as_deref(),
						group_name.as_deref(),
						is_video,
					));
					parts.call_parts.caller_info.show(true);
					parts.call_parts.mute_btn.set_label(crate::i18n::tr("Mute microphone"));
					parts.call_parts.mute_btn.show(true);
					parts.call_parts.camera_btn.set_label(crate::i18n::tr("Turn camera on"));
					parts.call_parts.camera_btn.show(true);
					parts.call_parts.device_settings_btn.set_label(crate::i18n::tr("Device settings"));
					parts.call_parts.device_settings_btn.show(true);
					parts.call_parts.input_device_group.show(false);
					parts.call_parts.output_device_group.show(false);
					parts.call_parts.camera_device_group.show(false);
					crate::ui_commands::calls::hide_screen_share_controls(parts);
					parts.call_parts.leave_btn.show(false);
					parts.call_parts.add_participant_btn.show(false);
					show_call_device_combos(&s, parts);
					parts.call_parts.frame.show(true);
					parts.call_parts.frame.iconize(false);
					parts.call_parts.frame.raise();
					parts.call_parts.accept_btn.set_focus();
					if s.settings.call_ringtone {
						let chat_jid = data.get("chat_jid").and_then(|j| j.as_str()).unwrap_or(caller);
						let ringtone = s.settings.ringtone_for(chat_jid, is_group);
						crate::audio::start_call_ringtone(&mut s, &ringtone);
					}
					let (call_hotkeys, unavailable) = crate::calls::hotkeys::start(&s.ui_tx, &s.keymap);
					s.call_hotkeys = call_hotkeys;
					for chord in unavailable {
						let msg = crate::i18n::tr_fmt(
							"Could not register the call key {}; another program is using it",
							&[&chord],
						);
						announce_call(parts, &msg, Priority::High);
					}
					if s.settings.notifications_enabled && s.settings.call_notifications {
						let toast_body = if crate::i18n::is_indonesian()
							&& (description.starts_with("group ")
								|| description.starts_with("video call ")
								|| description.starts_with("voice call "))
						{
							let kind = if is_video { "Panggilan video" } else { "Panggilan suara" };
							match (is_group, group_name.as_deref()) {
								(true, Some(group)) => format!("{kind} grup dari {name} di {group}"),
								(true, None) => format!("{kind} grup dari {name}"),
								(false, _) => format!("{kind} dari {name}"),
							}
						} else {
							let mut body = description.clone();
							if let Some(first) = body.get_mut(..1) {
								first.make_ascii_uppercase();
							}
							body
						};
						show_toast_notification(crate::i18n::tr("Incoming call"), &toast_body);
					}
					announce_call(parts, &win_title, Priority::High);
					call_log(parts, &s, "incoming shown", call_id);
				}
			}
		}
		"call_logs" => {
			if let Some(data) = val.get("data").and_then(|d| d.as_array()) {
				let mut s = state.borrow_mut();
				let mut entries = Vec::new();
				for item in data {
					let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
					let jid = item.get("jid").and_then(|v| v.as_str()).unwrap_or("").to_string();
					let peer_name = item.get("peer_name").and_then(|v| v.as_str()).unwrap_or("").to_string();
					let from_me = item.get("from_me").and_then(|v| v.as_bool()).unwrap_or(false);
					let text = item.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
					let timestamp = item.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0);
					let is_video = item.get("is_video").and_then(|v| v.as_bool()).unwrap_or(false);
					let is_group = item.get("is_group").and_then(|v| v.as_bool()).unwrap_or(false);
					let result = item.get("result").and_then(|v| v.as_str()).unwrap_or("").to_string();
					let duration = item.get("duration").and_then(|v| v.as_i64()).unwrap_or(0);
					entries.push(CallLogEntry {
						id,
						jid,
						peer_name,
						from_me,
						text,
						timestamp,
						is_video,
						is_group,
						result,
						duration,
					});
				}
				s.call_logs = entries;
				if s.active_tab == 6 {
					apply_chat_filter(&mut s, parts);
				}
			}
		}
		"call_targets" => {
			if let Some(data) = val.get("data") {
				let parse = |key: &str| -> Vec<CallTarget> {
					data.get(key)
						.and_then(|v| v.as_array())
						.map(|rows| {
							rows.iter()
								.filter_map(|r| {
									let jid = r.get("jid").and_then(|v| v.as_str())?.to_string();
									Some(CallTarget {
										name: r.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
										phone: r.get("phone").and_then(|v| v.as_str()).unwrap_or("").to_string(),
										jid,
									})
								})
								.collect()
						})
						.unwrap_or_default()
				};
				let mut s = state.borrow_mut();
				s.call_frequent = parse("frequent");
				s.call_contacts = parse("contacts");
				if s.active_tab == 6 {
					apply_chat_filter(&mut s, parts);
				}
			}
		}
		"call_logs_changed" => {
			let s = state.borrow();
			if s.active_tab == 6 {
				crate::ui_commands::chat_list::request_call_tab_data(&s);
			}
		}
		"call_link_created" | "call_hand" | "call_reaction" | "call_waiting_room" | "call_action_done"
		| "call_action_failed" | "call_muted_by" => {
			if let Some(data) = val.get("data") {
				crate::ui_commands::call_extras::on_event(state, parts, t, data);
			}
		}
		_ => return false,
	}
	true
}
