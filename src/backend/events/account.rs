use super::*;

pub(super) fn handle(t: &str, val: &serde_json::Value, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) -> bool {
	match t {
		"chat_lists" | "favorite_saved" | "list_saved" | "list_deleted" | "list_member_saved" | "common_groups" => {
			if let Some(data) = val.get("data") {
				crate::ui_commands::chat_lists::on_event(state, parts, t, data);
			}
		}
		"own_profile" => {
			if let Some(data) = val.get("data") {
				let mut s = state.borrow_mut();
				if let Some(push_name) = data.get("pushName").and_then(|p| p.as_str()) {
					s.own_push_name = push_name.to_string();
				}
				let about_arrived = if let Some(about) = data.get("about").and_then(|a| a.as_str()) {
					s.own_about = about.to_string();
					s.own_about_duration = data.get("aboutDuration").and_then(|d| d.as_i64()).unwrap_or(-1);
					s.own_about_loaded = true;
					true
				} else {
					false
				};
				let push_name = s.own_push_name.clone();
				let about = (s.own_about.clone(), s.own_about_duration);
				drop(s);
				if parts.settings_dialog.dialog.is_shown() {
					parts.settings_dialog.name_field.set_value(&push_name);
					if about_arrived {
						parts.settings_dialog.show_about(Some((&about.0, about.1)));
					}
				}
			}
		}
		"default_timer" => {
			if let Some(data) = val.get("data")
				&& let Some(seconds) = data.get("seconds").and_then(|s| s.as_i64())
			{
				state.borrow_mut().default_timer = Some(seconds);
				let combo = &parts.settings_dialog.default_timer_combo;
				crate::ui::dialogs::settings_dialog::set_combo_from_value(
					combo,
					crate::ui::dialogs::settings_dialog::DEFAULT_TIMER_CHOICES,
					&seconds.to_string(),
				);
				combo.enable(true);
			}
		}
		"own_photo_changed" => {
			let removed = val.get("data").and_then(|d| d.get("removed")).and_then(|r| r.as_bool()) == Some(true);
			let parent = &parts.settings_dialog.dialog;
			let message = if removed { "Your profile photo was removed." } else { "Your profile photo was changed." };
			if parent.is_shown() {
				MessageDialog::builder(parent, message, "Profile Photo")
					.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
					.build()
					.show_modal();
			} else {
				crate::platform::announce::announce(parts.conversations_parts.messages_announcer, message);
			}
		}
		"default_timer_saved" => {}
		"privacy_settings" => {
			if let Some(data) = val.get("data") {
				let mut s = state.borrow_mut();
				for key in ["groupadd", "last", "status", "profile", "readreceipts", "online", "calladd"] {
					if let Some(v) = data.get(key).and_then(|v| v.as_str()) {
						s.privacy_settings.insert(key.to_string(), v.to_string());
					}
				}
				s.privacy_settings_loaded = true;
				let privacy_settings = s.privacy_settings.clone();
				drop(s);
				if parts.settings_dialog.dialog.is_shown() {
					parts.settings_dialog.show_privacy(Some(&privacy_settings));
				}
			}
		}
		"privacy_list" => {
			if let Some(data) = val.get("data") {
				let Some(name) = data.get("name").and_then(|n| n.as_str()) else { return true };
				let list: Result<Vec<Contact>, String> = match data.get("error").and_then(|e| e.as_str()) {
					Some(err) => Err(err.to_string()),
					None => Ok(data
						.get("entries")
						.and_then(|e| e.as_array())
						.map(|entries| {
							entries
								.iter()
								.filter_map(|entry| {
									let field =
										|k: &str| entry.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
									let jid = field("jid");
									(!jid.is_empty()).then(|| Contact {
										jid,
										name: field("name"),
										phone: field("phone"),
									})
								})
								.collect()
						})
						.unwrap_or_default()),
				};
				let mut s = state.borrow_mut();
				if list.is_ok() {
					s.privacy_list_edits.remove(name);
				}
				s.privacy_lists.insert(name.to_string(), list);
			}
		}
		kind @ ("message_status" | "receipt") => {
			if let Some(data) = val.get("data") {
				let chat = data.get("jid").or_else(|| data.get("chat_jid")).and_then(|j| j.as_str());
				if kind == "receipt" && chat.is_some_and(|j| j.ends_with("@g.us")) {
					return true;
				}
				let chat_jid = data.get("jid").or_else(|| data.get("chat_jid")).and_then(|j| j.as_str()).unwrap_or("");
				let raw_status = data.get("status").or_else(|| data.get("type")).and_then(|s| s.as_str()).unwrap_or("");

				let mut msg_ids = Vec::new();
				if let Some(ids) = data.get("message_ids").and_then(|i| i.as_array()) {
					for id_val in ids {
						if let Some(id_str) = id_val.as_str() {
							msg_ids.push(id_str.to_string());
						}
					}
				} else if let Some(id_str) = data.get("message_id").and_then(|i| i.as_str()) {
					msg_ids.push(id_str.to_string());
				}

				let norm_jid_str = |jid: &str| -> String {
					let no_device = jid.split(':').next().unwrap_or(jid);
					if no_device.contains('@') {
						no_device.to_string()
					} else {
						jid.split('@').next().unwrap_or(jid).to_string()
					}
				};
				let norm_incoming = norm_jid_str(chat_jid);

				let mut s = state.borrow_mut();
				let is_match =
					if let Some(ref sel) = s.selected_chat { norm_incoming == norm_jid_str(sel) } else { false };

				if is_match {
					let is_group = chat_jid.ends_with("@g.us");
					let norm_status = match raw_status {
						"read-self" => "read",
						"sender" => "sent",
						"inactive" => "delivered",
						"" => "delivered",
						other => other,
					};
					let receipt_ts = data.get("timestamp").and_then(|t| t.as_i64()).unwrap_or(0);

					let ids: std::collections::HashSet<&str> = msg_ids.iter().map(String::as_str).collect();
					let mut missing = ids.clone();
					for msg in &s.messages {
						missing.remove(msg.id.as_str());
					}
					let any_updated = missing.len() < ids.len();
					for id in missing {
						s.pending_receipts.insert(id.to_string(), norm_status.to_string());
					}
					let rows = update_loaded_messages(
						&mut s,
						|m| ids.contains(m.id.as_str()) && get_status_rank(norm_status) >= get_status_rank(&m.status),
						|m| {
							m.status = norm_status.to_string();
							if receipt_ts > 0 {
								let at = match norm_status {
									"delivered" => &mut m.delivered_at,
									"read" => &mut m.read_at,
									"played" => &mut m.played_at,
									_ => return,
								};
								if *at == 0 {
									*at = receipt_ts;
								}
							}
						},
					);
					for di in rows {
						if let Some(item) = s.display_items.get(di) {
							parts
								.conversations_parts
								.messages_list
								.set_string(di as u32, &item.format_display(is_group));
						}
					}
					if any_updated && (norm_status == "sent" || norm_status == "delivered") {
						let now_ts = std::time::SystemTime::now()
							.duration_since(std::time::UNIX_EPOCH)
							.unwrap_or_default()
							.as_secs() as i64;
						// Only IDs that could still ring are worth remembering.
						// The recency gate means anything older than the window
						// can never produce a sound again, so admitting it to
						// the set was pure unbounded growth - and a reconnect
						// admits the entire backlog at once. Drop what has aged
						// out, and never take a backlogged ID in the first place.
						s.played_sent_sounds.retain(|_, acked_at| now_ts - *acked_at < SENT_SOUND_WINDOW_SECS);
						let mut should_play = false;
						let recent: Vec<String> = s
							.messages
							.iter()
							.filter(|m| ids.contains(m.id.as_str()) && now_ts - m.sent_at < SENT_SOUND_WINDOW_SECS)
							.map(|m| m.id.clone())
							.collect();
						for target_id in recent {
							if let std::collections::hash_map::Entry::Vacant(e) = s.played_sent_sounds.entry(target_id)
							{
								e.insert(now_ts);
								should_play = true;
							}
						}
						if should_play {
							play_notification_sound(
								NotificationSound::MessageSent,
								s.settings.sounds_enabled && s.settings.outgoing_sounds,
								s.settings.output_device_name.clone(),
							);
						}
					}
				}
			}
		}
		"reported_messages" => {
			if let Some(data) = val.get("data") {
				crate::ui_commands::report::show_reported_messages(state, parts, data);
			}
		}
		"reported" => {
			if let Some(data) = val.get("data") {
				crate::ui_commands::report::reported(parts, data);
			}
		}
		"blocklist" => {
			if let Some(data) = val.get("data") {
				crate::ui_commands::blocking::blocklist_received(state, data);
			}
		}
		"block_result" => {
			if let Some(data) = val.get("data") {
				crate::ui_commands::blocking::block_result(state, parts, data);
			}
		}
		_ => return false,
	}
	true
}
