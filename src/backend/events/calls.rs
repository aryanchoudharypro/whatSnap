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
							parts.call_parts.mute_btn.set_label("Unmute microphone");
							if let Some(call_id) = &s.active_call_id {
								let payload = serde_json::json!({
									"action": "mute_call",
									"data": { "call_id": call_id, "muted": true }
								});
								let _ = s.ws_tx.send(payload.to_string());
							}
						} else {
							parts.call_parts.mute_btn.set_label("Mute microphone");
						}
						s.active_call_audio = Some(streams);
						let connected_msg = if s.call_video.is_video {
							"Video call connected. Your camera is off."
						} else {
							"Call connected."
						};
						announce_call(parts, connected_msg);

						if s.call_participants.is_empty() {
							parts.call_parts.status_label.set_label(connected_msg);
						} else {
							parts.call_parts.status_label.set_label(&group_call_status_text(&s.call_participants));
						}
						let running_info = crate::calls::helpers::call_running_info_text(
							s.call_is_incoming,
							s.call_video.is_video,
							&s.call_peer_name,
							0,
							false,
						);
						parts.call_parts.caller_info.set_value(&running_info);
						parts.call_parts.caller_info.show(true);
						let title = if s.call_video.is_video {
							format!("Video call with {}", s.call_peer_name)
						} else {
							format!("Call with {}", s.call_peer_name)
						};
						parts.call_parts.frame.set_title(&title);
						parts.call_parts.accept_btn.show(false);
						parts.call_parts.reject_btn.show(false);
						parts.call_parts.mute_btn.show(true);
						parts.call_parts.camera_btn.show(true);
						parts.call_parts.camera_btn.set_label(if s.call_video.local_camera_on {
							"Turn camera off"
						} else {
							"Turn camera on"
						});
						crate::ui_commands::calls::show_screen_share_controls(&s, parts);
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
						announce_call(parts, "Failed to start audio stream.");
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
					s.call_participants.clear();
					s.active_call_video_send = None;
					s.call_video_port = None;
					if data.get("link").and_then(|l| l.as_bool()) != Some(true) {
						let self_phone = data.get("self_phone").and_then(|p| p.as_str()).unwrap_or("");
						start_call_ringback(&mut s, self_phone);
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
					"No sound is reaching you from the call. Check your internet connection."
				} else {
					"Sound from the call isn't getting through."
				};
				announce_call(parts, message);
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
						s.call_is_incoming,
						s.call_video.is_video,
						&s.call_peer_name,
						secs,
						true,
					);
					parts.call_parts.caller_info.set_value(&info);
					parts.call_parts.caller_info.show(true);
					crate::audio::start_call_reconnect_tone(&mut s);
					announce_call(parts, "Reconnecting...");
				} else {
					s.call_is_reconnecting = false;
					s.call_connected_segment_start = Some(std::time::Instant::now());
					let secs = s.call_connected_duration.as_secs();
					let info = crate::calls::helpers::call_running_info_text(
						s.call_is_incoming,
						s.call_video.is_video,
						&s.call_peer_name,
						secs,
						false,
					);
					parts.call_parts.caller_info.set_value(&info);
					parts.call_parts.caller_info.show(true);
					crate::audio::stop_call_reconnect_tone(&mut s);
					announce_call(parts, "Reconnected.");
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
					let name = data.get("name").and_then(|n| n.as_str()).unwrap_or("Contact");
					format!("Ringing {}.", name)
				} else {
					data.get("message")
						.and_then(|m| m.as_str())
						.unwrap_or("Couldn't add that person to the call.")
						.to_string()
				};
				announce_call(parts, &message);
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
					announce_call(parts, &changes.join(" "));
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
					"on" | "upgrade_requested" => {
						s.call_video.peer_camera_on = Some(true);
						if !s.call_video.video_call {
							s.call_video.video_call = true;
							if s.active_call_audio.is_some() {
								crate::ui_commands::calls::show_screen_share_controls(&s, parts);
							}
						}
					}
					"off" => s.call_video.peer_camera_on = Some(false),
					_ => {}
				}
				drop(s);
				if let Some(text) = announcement {
					announce_call(parts, &text);
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
				let reason = data.get("message").and_then(|m| m.as_str()).unwrap_or("").to_string();
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
				parts.call_parts.camera_btn.set_label("Turn camera on");
				parts.call_parts.screen_btn.set_label("Share screen");
				drop(s);
				announce_call(
					parts,
					if was_sharing {
						"Couldn't share the screen on this call."
					} else {
						"Couldn't turn the camera on for this call."
					},
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
				announce_call(parts, &crate::calls::helpers::peer_screen_share_announcement(name, active));
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
				parts.call_parts.frame.show(false);
				stop_call_ringing_sound(&mut s);
				play_call_fail_sound(&mut s);
				let message = val
					.get("data")
					.and_then(|d| d.get("message"))
					.and_then(|m| m.as_str())
					.unwrap_or("Failed to place call.");
				announce_call(parts, message);
			}
		}
		"call_ended" => {
			if let Some(data) = val.get("data")
				&& let Some(call_id) = data.get("call_id").and_then(|c| c.as_str())
			{
				let mut s = state.borrow_mut();
				call_log(parts, &s, "ended", call_id);
				if s.active_call_id.as_deref() == Some(call_id) {
					crate::audio::stop_call_reconnect_tone(&mut s);
					s.active_call_id = None;
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
					parts.call_parts.frame.show(false);
					stop_call_ringing_sound(&mut s);
					let reason = data.get("reason").and_then(|r| r.as_str()).unwrap_or("");
					if reason == "busy" && s.incoming_call_caller.is_none() {
						crate::audio::play_busy_tone(&mut s);
					} else {
						play_call_fail_sound(&mut s);
					}
					let summary = data.get("summary").and_then(|s| s.as_str()).unwrap_or("");
					if s.incoming_call_caller.take().is_none() {
						announce_call(
							parts,
							match reason {
								"busy" => "Busy, on another call.",
								"connection_lost" => "Call ended. Connection lost.",
								_ => "Call ended.",
							},
						);
					} else if summary.contains("on another device") {
						announce_call(parts, &format!("{}.", summary));
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
					parts.call_parts.frame.show(false);
					stop_call_ringing_sound(&mut s);
					play_call_fail_sound(&mut s);
					let message = data.get("message").and_then(|m| m.as_str()).unwrap_or("Failed to answer call.");
					announce_call(parts, message);
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
					s.call_answer_deadline = None;
					let chat_jid = data.get("chat_jid").and_then(|j| j.as_str());
					let raw_name = data.get("caller_name").and_then(|n| n.as_str());
					let raw_phone = data.get("caller_phone").and_then(|p| p.as_str());
					let (name, phone_digits) = crate::calls::helpers::resolve_call_peer_name_and_phone(
						&s.contacts,
						&s.all_contacts,
						caller,
						chat_jid,
						raw_name,
						raw_phone,
					);
					s.incoming_call_caller = Some(name.clone());
					s.call_is_incoming = true;
					s.call_peer_name = name.clone();
					s.call_start_muted = false;
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
					let win_title = if is_video {
						format!("Incoming video call from {name}")
					} else if is_group {
						format!("Incoming group call from {name}")
					} else {
						format!("Incoming call from {name}")
					};
					parts.call_parts.frame.set_title(&win_title);
					parts.call_parts.status_label.set_label(&win_title);
					parts.call_parts.accept_btn.show(true);
					parts.call_parts.reject_btn.show(true);
					crate::ui_commands::call_extras::hide_call_buttons(parts);
					parts.call_parts.caller_info.set_value(&crate::calls::helpers::call_incoming_info_text(
						&name,
						phone_digits.as_deref(),
						group_name.as_deref(),
						is_video,
					));
					parts.call_parts.caller_info.show(true);
					parts.call_parts.mute_btn.set_label("Mute microphone");
					parts.call_parts.mute_btn.show(true);
					parts.call_parts.camera_btn.set_label("Turn camera on");
					parts.call_parts.camera_btn.show(true);
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
						announce_call(
							parts,
							&format!("Could not register the call key {chord}; another program is using it"),
						);
					}
					if s.settings.notifications_enabled && s.settings.call_notifications {
						let mut toast_body = description.clone();
						toast_body[..1].make_ascii_uppercase();
						show_toast_notification("Incoming call", &toast_body);
					}
					announce_call(parts, &win_title);
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
