use super::*;

pub(super) fn handle(t: &str, val: &serde_json::Value, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) -> bool {
	match t {
		"message" => {
			if let Some(data) = val.get("data") {
				let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
				let from = data.get("from").and_then(|f| f.as_str()).unwrap_or("");
				let text = data.get("text").and_then(|t| t.as_str()).unwrap_or("");
				// Group event lines ("X joined", "Y left", a new subject) never
				// alert or count as unread in the official apps, muted or not.
				let is_system = data.get("mediaType").and_then(|m| m.as_str()) == Some("system");
				let is_backlog = whatsnap::is_backlog_message(
					data.get("timestamp").and_then(|t| t.as_i64()).unwrap_or(0),
					std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
						as i64,
				);
				let is_call_line = data.get("mediaType").and_then(|m| m.as_str()) == Some("call");
				let is_missed_call = is_call_line && text.starts_with("Missed");

				let mut s = state.borrow_mut();
				if is_call_line && s.active_tab == 6 {
					crate::ui_commands::chat_list::request_call_tab_data(&s);
				}

				if s.selected_chat.as_deref() == Some(jid)
					&& !s.messages_at_latest
					&& !data.get("from_me").and_then(|f| f.as_bool()).unwrap_or(false)
				{
					s.newer_unloaded += 1;
					parts.conversations_parts.messages_list.set_numbering(s.older_unloaded, s.newer_unloaded);
				}
				if s.selected_chat.as_deref() == Some(jid) && s.messages_at_latest {
					let is_from_me =
						data.get("from_me").and_then(|f| f.as_bool()).unwrap_or_else(|| from == "me" || from == "You");
					let is_group = jid.ends_with("@g.us");
					let ts = data
						.get("timestamp")
						.and_then(|t| t.as_i64().or_else(|| t.as_f64().map(|f| f as i64)))
						.unwrap_or_else(|| {
							std::time::SystemTime::now()
								.duration_since(std::time::UNIX_EPOCH)
								.unwrap_or_default()
								.as_secs() as i64
						});
					let msg_id = data.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();

					let mut temp_msg_idx = None;
					for (idx, msg) in s.messages.iter().enumerate() {
						if (!msg_id.is_empty() && msg.id == msg_id)
							|| (is_from_me && msg.id.starts_with("temp_upload_"))
						{
							temp_msg_idx = Some(idx);
							break;
						}
					}

					if let Some(idx) = temp_msg_idx {
						let pick_id = s.messages[idx].id.clone();
						let was_pending = s.messages[idx].status == "pending" || pick_id.starts_with("temp_upload_");
						let was_voice = s.messages[idx].media_type == "voice";
						let status =
							data.get("status").and_then(|st| st.as_str()).filter(|st| !st.is_empty()).unwrap_or("sent");
						// WA Web plays the sent tone once, when the server accepts the
						// message (WAWebSendMsgChatAction -> playOutgoingMessageTone
						// after the send resolves with ack SENT), not on delivery.
						// Voice messages already have their own send sound.
						let sent_here = data.get("sentHere").and_then(|v| v.as_bool()).unwrap_or(false);
						if is_from_me && (was_pending || sent_here) && status == "sent" && !was_voice {
							play_notification_sound(
								NotificationSound::MessageSent,
								s.settings.sounds_enabled && s.settings.outgoing_sounds,
								s.settings.output_device_name.clone(),
							);
						}
						let field = |name: &str| data.get(name).and_then(|v| v.as_str()).unwrap_or("");
						let quote_field = |snake: &str, camel: &str| {
							data.get(snake).or_else(|| data.get(camel)).and_then(|v| v.as_str()).unwrap_or("")
						};
						let target_di = update_loaded_message(&mut s, &pick_id, |m| {
							m.id = msg_id.clone();
							m.text = text.to_string();
							m.timestamp = ts;
							if is_from_me {
								m.sent_at = ts;
								m.status = status.to_string();
							}
							m.media_type = field("mediaType").to_string();
							m.media_filename = field("mediaFilename").to_string();
							let path = field("mediaPath");
							if !path.is_empty() {
								m.media_path = path.to_string();
							}
							m.quoted_id = quote_field("quoted_id", "quotedId").to_string();
							m.quoted_text = quote_field("quoted_text", "quotedText").to_string();
							m.quoted_sender = quote_field("quoted_sender", "quotedSender").to_string();
						});
						let updated_display = target_di
							.and_then(|di| s.display_items.get(di))
							.map(|item| item.format_display(is_group))
							.unwrap_or_default();

						if let Some(di) = target_di {
							parts.conversations_parts.messages_list.set_string(di as u32, &updated_display);
							// What we send ourselves is never read back.
							if !is_from_me && s.settings.announces_incoming(jid) {
								crate::platform::announce::announce(
									parts.conversations_parts.messages_announcer,
									&updated_display,
								);
							}
							if parts.conversations_parts.messages_list.get_selection() == Some(di as u32) {
								refresh_voice_action_button(&s, &parts.conversations_parts);
							}
						}
					} else {
						let sender_name = if is_from_me {
							"You".to_string()
						} else if from.is_empty() && !is_group {
							// Rows with no sender (call log lines) belong to the
							// other person in a 1:1 chat, same as when loaded.
							["senderName", "chatName"]
								.iter()
								.filter_map(|k| data.get(*k).and_then(|n| n.as_str()))
								.find(|n| !n.is_empty())
								.map(|n| n.to_string())
								.or_else(|| s.contacts.get(jid).cloned())
								.unwrap_or_else(|| jid.to_string())
						} else {
							let mut clean_jid = from.to_string();
							if let Some(idx) = from.find(':') {
								clean_jid = format!("{}@s.whatsapp.net", &from[..idx]);
							}
							data.get("senderName")
								.and_then(|n| n.as_str())
								.filter(|n| !n.is_empty())
								.map(|n| n.to_string())
								.or_else(|| s.contacts.get(&clean_jid).cloned())
								.or_else(|| s.contacts.get(from).cloned())
								.unwrap_or(clean_jid)
						};

						let mut insert_date = false;
						let mut divider_label = String::new();
						if let Some(local_datetime) = whatsnap::LocalTime::from_unix(ts) {
							let day = whatsnap::date_key(local_datetime);
							if let Some(last_msg) = s.messages.last() {
								if !last_msg.is_date_divider
									&& let Some(last_local) = whatsnap::LocalTime::from_unix(last_msg.timestamp)
								{
									if day != whatsnap::date_key(last_local) {
										insert_date = true;
									}
								}
							} else {
								insert_date = true;
							}
							if insert_date {
								divider_label = whatsnap::format_date_divider_label(local_datetime);
							}
						}

						let quoted_id = data
							.get("quoted_id")
							.or_else(|| data.get("quotedId"))
							.and_then(|q| q.as_str())
							.unwrap_or("")
							.to_string();
						let mut quoted_text = data
							.get("quoted_text")
							.or_else(|| data.get("quotedText"))
							.and_then(|q| q.as_str())
							.unwrap_or("")
							.to_string();
						let mut quoted_sender = data
							.get("quoted_sender")
							.or_else(|| data.get("quotedSender"))
							.and_then(|q| q.as_str())
							.unwrap_or("")
							.to_string();

						if !quoted_id.is_empty()
							&& (quoted_text.is_empty() || quoted_sender.is_empty())
							&& let Some(original) = s.messages.iter().find(|m| m.id == quoted_id)
						{
							if quoted_text.is_empty() {
								quoted_text = if original.is_audio || original.text.starts_with("[Voice Message]") {
									"[Voice Message]".to_string()
								} else if original.text.is_empty() {
									"[Media]".to_string()
								} else {
									original.text.clone()
								};
							}
							if quoted_sender.is_empty() {
								quoted_sender = if original.from_me {
									"me".to_string()
								} else if !original.sender_name.is_empty() {
									original.sender_name.clone()
								} else {
									original.sender_jid.clone()
								};
							}
						}

						if insert_date && !divider_label.is_empty() {
							let divider = whatsnap::Msg::new(ChatMessage {
								id: String::new(),
								text: divider_label,
								timestamp: ts,
								is_date_divider: true,
								..Default::default()
							});
							s.messages.push(divider.clone());
							s.display_items.push(whatsnap::DisplayItem::Single(divider.clone()));
							let text = divider.format_display(is_group);
							let key = message_row_key(&whatsnap::DisplayItem::Single(divider.clone()), &text);
							parts.conversations_parts.messages_list.append_keyed(&key, &text);
						}

						let chat_msg = whatsnap::Msg::new(ChatMessage {
							id: msg_id.clone(),
							text: text.to_string(),
							timestamp: ts,
							from_me: is_from_me,
							sender_jid: from.to_string(),
							sender_name,
							is_audio: text.starts_with("[Voice Message]") || text.starts_with("[Audio]"),
							status: data
								.get("status")
								.and_then(|st| st.as_str())
								.filter(|st| !st.is_empty())
								.map(|st| st.to_string())
								.or_else(|| s.pending_receipts.remove(&msg_id))
								.unwrap_or_else(|| "sent".to_string()),
							sent_at: if is_from_me { ts } else { 0 },
							quoted_id,
							quoted_text,
							quoted_sender,
							media_type: data.get("mediaType").and_then(|m| m.as_str()).unwrap_or("").to_string(),
							media_filename: data
								.get("mediaFilename")
								.and_then(|m| m.as_str())
								.unwrap_or("")
								.to_string(),
							media_path: data.get("mediaPath").and_then(|m| m.as_str()).unwrap_or("").to_string(),
							media_size: data.get("mediaSize").and_then(|m| m.as_i64()).unwrap_or(0),
							album_id: data.get("albumId").and_then(|m| m.as_str()).unwrap_or("").to_string(),
							poll_results: data.get("pollResults").and_then(|m| m.as_str()).unwrap_or("").to_string(),
							..Default::default()
						});
						s.messages.push(chat_msg.clone());
						// Stickers, polls and other sends with no pending row:
						// the sent tone once the server has accepted them, as
						// in WA Web.
						if is_from_me
							&& chat_msg.status == "sent"
							&& data.get("sentHere").and_then(|v| v.as_bool()).unwrap_or(false)
						{
							play_notification_sound(
								NotificationSound::MessageSent,
								s.settings.sounds_enabled && s.settings.outgoing_sounds,
								s.settings.output_device_name.clone(),
							);
						}

						let is_deleted_msg = chat_msg.is_deleted;
						let is_media_msg = !is_deleted_msg
							&& (chat_msg.media_type == "image"
								|| chat_msg.media_type == "video"
								|| chat_msg.text.starts_with("[Photo]")
								|| chat_msg.text.starts_with("[Image]")
								|| chat_msg.text.starts_with("[Video]"));

						let should_merge = if is_media_msg {
							if let Some(last_item) = s.display_items.last() {
								match last_item {
									whatsnap::DisplayItem::Single(prev_msg) => {
										let prev_is_media = !prev_msg.is_deleted
											&& (prev_msg.media_type == "image"
												|| prev_msg.media_type == "video"
												|| prev_msg.text.starts_with("[Photo]")
												|| prev_msg.text.starts_with("[Image]")
												|| prev_msg.text.starts_with("[Video]"));
										prev_is_media && whatsnap::same_album(prev_msg, &chat_msg)
									}
									whatsnap::DisplayItem::Album { messages, .. } => {
										messages.last().is_some_and(|m| whatsnap::same_album(m, &chat_msg))
									}
								}
							} else {
								false
							}
						} else {
							false
						};

						if should_merge {
							let last_idx = s.display_items.len() - 1;
							let merged = match s.display_items.pop().unwrap() {
								whatsnap::DisplayItem::Single(prev_msg) => whatsnap::DisplayItem::Album {
									id: prev_msg.id.clone(),
									sender_jid: prev_msg.sender_jid.clone(),
									sender_name: prev_msg.sender_name.clone(),
									from_me: prev_msg.from_me,
									timestamp: prev_msg.timestamp,
									messages: vec![prev_msg, chat_msg.clone()],
								},
								whatsnap::DisplayItem::Album {
									id,
									sender_jid,
									sender_name,
									from_me,
									timestamp,
									mut messages,
								} => {
									messages.push(chat_msg.clone());
									whatsnap::DisplayItem::Album {
										id,
										sender_jid,
										sender_name,
										from_me,
										timestamp,
										messages,
									}
								}
							};
							parts
								.conversations_parts
								.messages_list
								.set_string(last_idx as u32, &merged.format_display(is_group));
							s.display_items.push(merged);
						} else {
							s.display_items.push(whatsnap::DisplayItem::Single(chat_msg.clone()));
							parts
								.conversations_parts
								.messages_list
								.append_keyed(&msg_id, &chat_msg.format_display(is_group));
						}
						refresh_selection_list(&mut s, parts);
						crate::ui_commands::calls::mark_joinable_rows(&s, parts);

						let count = parts.conversations_parts.messages_list.get_count();
						if count > 0 && is_from_me && !s.settings.keep_list_position {
							parts.conversations_parts.messages_list.set_selection(count - 1, true);
							refresh_voice_action_button(&s, &parts.conversations_parts);
						}
						if !is_from_me && s.settings.announces_incoming(jid) {
							let formatted = chat_msg.format_display(is_group);
							crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &formatted);
						}

						if !is_from_me {
							let sounds_enabled = s.settings.sounds_enabled && !s.muted_jids.contains(jid) && !is_system;
							if s.window_focused {
								let mark_jid = jid.to_string();
								let mark_msg_id = msg_id.clone();
								let tx = s.ws_tx.clone();
								let payload = serde_json::json!({
									"action": "mark_read",
									"data": { "jid": mark_jid, "message_ids": vec![mark_msg_id] }
								});
								let _ = tx.send(payload.to_string());
							} else if !is_system {
								// The window isn't focused, so the user hasn't actually
								// seen this yet — don't send a read receipt. Queue it
								// for a batched mark_read once focus returns, and show
								// (or grow) an "N unread messages" divider right now so
								// it's already there when they look back at the chat —
								// updated/inserted in place rather than rebuilding the
								// whole list, so messages just quietly stack up in the
								// background like the official WhatsApp client.
								s.background_unread_message_ids.push(msg_id.clone());
								let is_group = jid.ends_with("@g.us");
								let existing_di = s.display_items.iter().position(
									|item| matches!(item, whatsnap::DisplayItem::Single(m) if m.is_unread_divider),
								);
								if let Some(di) = existing_di {
									s.unread_divider_count += 1;
									let count = s.unread_divider_count;
									let label = if count == 1 {
										"1 unread message".to_string()
									} else {
										format!("{} unread messages", count)
									};
									update_loaded_messages(&mut s, |m| m.is_unread_divider, |m| m.text = label.clone());
									parts.conversations_parts.messages_list.set_string(di as u32, &label);
								} else {
									s.unread_divider_jid = Some(jid.to_string());
									s.unread_divider_count = 1;
									let divider = whatsnap::Msg::new(ChatMessage {
										text: "1 unread message".to_string(),
										is_unread_divider: true,
										..Default::default()
									});
									let insert_at = s.messages.len().saturating_sub(1);
									s.messages.insert(insert_at, divider.clone());
									let di_insert_at = s.display_items.len().saturating_sub(1);
									s.display_items
										.insert(di_insert_at, whatsnap::DisplayItem::Single(divider.clone()));
									parts
										.conversations_parts
										.messages_list
										.insert(&divider.format_display(is_group), di_insert_at);
								}
							}
							if !is_backlog {
								play_notification_sound(
									NotificationSound::MessageCurrent,
									sounds_enabled,
									s.settings.output_device_name.clone(),
								);
							} else if is_missed_call || !is_call_line {
								crate::ui::chat_ui::record_catch_up_alert(
									&mut s,
									jid,
									is_missed_call,
									whatsnap::IncomingAlert { sound: sounds_enabled, toast: false },
								);
							}
						}
					}
				} else {
					let is_ignored =
						jid == "status@broadcast" || jid.ends_with("@broadcast") || jid.ends_with("@newsletter");
					if !is_ignored {
						let is_from_me = data
							.get("from_me")
							.and_then(|f| f.as_bool())
							.unwrap_or_else(|| from == "me" || from == "You");
						if !is_from_me && !is_system {
							let addresses_me = data.get("addressesMe").and_then(|v| v.as_bool()).unwrap_or(false);
							let sender_chat = match (from.split_once(':'), from.split_once('@')) {
								(Some((user, _)), Some((_, server))) => format!("{user}@{server}"),
								_ => from.to_string(),
							};
							let reaches_me = addresses_me && !s.muted_jids.contains(&sender_chat);
							let alert = s.settings.message_alert(
								jid,
								s.muted_jids.contains(jid) && !reaches_me,
								s.archived_jids.contains(jid) && !reaches_me,
							);
							let alert = if !is_backlog && jid.ends_with("@g.us") && (alert.sound || alert.toast) {
								let now = std::time::Instant::now();
								let quiet = s.group_quiet_until.get(jid).is_some_and(|until| now < *until);
								if quiet && !addresses_me {
									whatsnap::IncomingAlert { sound: false, toast: false }
								} else {
									let members = data.get("groupSize").and_then(|v| v.as_u64()).unwrap_or(0);
									let secs = (30 + 5 * members).min(120);
									s.group_quiet_until
										.insert(jid.to_string(), now + std::time::Duration::from_secs(secs));
									alert
								}
							} else {
								alert
							};
							let alert = if is_backlog {
								if is_missed_call || !is_call_line {
									let allowed = whatsnap::IncomingAlert {
										sound: alert.sound
											&& s.settings.sounds_enabled
											&& !crate::platform::do_not_disturb::should_silence(&s.settings),
										toast: alert.toast && s.settings.notifications_enabled,
									};
									crate::ui::chat_ui::record_catch_up_alert(&mut s, jid, is_missed_call, allowed);
								}
								whatsnap::IncomingAlert { sound: false, toast: false }
							} else {
								alert
							};
							if alert.sound && !crate::platform::do_not_disturb::should_silence(&s.settings) {
								let (tone, file) = s.settings.message_tone_for(jid);
								crate::audio::play_message_tone(
									tone,
									&file,
									s.settings.sounds_enabled,
									s.settings.output_device_name.clone(),
								);
							}
							if alert.toast && s.locked_jids.contains(jid) {
								let unread = s.unread_counts.get(jid).copied().unwrap_or(0).max(1);
								let body = if unread == 1 {
									"1 new message".to_string()
								} else {
									format!("{unread} new messages")
								};
								crate::ui::chat_ui::show_chat_notification(jid, "WhatsApp", &body);
							} else if alert.toast {
								let previews =
									s.settings.notification_previews && !s.settings.chat_notify(jid).hide_content;
								let body = whatsnap::friendly_preview_text(text);
								let is_group = jid.ends_with("@g.us");
								let clean_from = match from.find(':') {
									Some(idx) => format!("{}@s.whatsapp.net", &from[..idx]),
									None => from.to_string(),
								};
								let provided_sender = data
									.get("senderName")
									.and_then(|n| n.as_str())
									.filter(|n| !n.is_empty())
									.map(|n| n.to_string());
								let provided_chat_name = data
									.get("chatName")
									.and_then(|n| n.as_str())
									.filter(|n| !n.is_empty())
									.map(|n| n.to_string());
								if is_group {
									let mut chat_name = provided_chat_name
										.or_else(|| s.contacts.get(jid).cloned())
										.unwrap_or_else(|| jid.to_string());
									if chat_name == *jid {
										chat_name = "New Group".to_string();
									}
									let sender_name = provided_sender
										.or_else(|| s.contacts.get(&clean_from).cloned())
										.or_else(|| s.contacts.get(from).cloned())
										.unwrap_or_else(|| display_name_for_jid(&clean_from));
									let toast_body = match (previews, sender_name.is_empty()) {
										(false, true) => "New message".to_string(),
										(false, false) => format!("Message from {}", sender_name),
										(true, true) => body.to_string(),
										(true, false) => format!("{}: {}", sender_name, body),
									};
									crate::ui::chat_ui::show_chat_notification(jid, &chat_name, &toast_body);
								} else {
									// Call log lines have no sender: name the chat.
									let fallback_jid = if from.is_empty() { jid } else { clean_from.as_str() };
									let sender_name = provided_sender
										.or_else(|| s.contacts.get(&clean_from).cloned())
										.or_else(|| s.contacts.get(from).cloned())
										.or_else(|| if from.is_empty() { provided_chat_name } else { None })
										.or_else(|| s.contacts.get(jid).cloned())
										.unwrap_or_else(|| display_name_for_jid(fallback_jid));
									let body = if previews { body.to_string() } else { "New message".to_string() };
									crate::ui::chat_ui::show_chat_notification(jid, &sender_name, &body);
								}
							}
						}
					}
				}
			}
		}
		"message_deleted" => {
			if let Some(data) = val.get("data") {
				let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
				let message_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("");
				let for_everyone = data.get("for_everyone").and_then(|f| f.as_bool()).unwrap_or(true);
				let upload_cancelled = data.get("upload_cancelled").and_then(|f| f.as_bool()).unwrap_or(false);

				let mut s = state.borrow_mut();
				// A bulk delete (selection mode) fires one "message_deleted"
				// event per message; announcing each individually would spam
				// a screen reader with "Message deleted" N times back to
				// back, on top of the one summary announcement
				// DeleteSelectedMessages already made - suppress the
				// per-message announce while draining that batch's set.
				// Keyed by (jid, message_id) so an unrelated delete arriving
				// mid-batch (e.g. synced from another linked device) isn't
				// mistaken for part of this batch.
				let suppress_announce = s.bulk_delete_pending.remove(&(jid.to_string(), message_id.to_string()));
				if Some(jid.to_string()) == s.selected_chat {
					let is_group = jid.ends_with("@g.us");

					if !for_everyone {
						let had_message = s.messages.iter().any(|m| m.id == message_id);
						if had_message {
							s.messages.retain(|m| m.id != message_id);
							// A day's divider goes with its last message.
							s.messages = whatsnap::rebuild_date_dividers(std::mem::take(&mut s.messages));
							s.display_items = whatsnap::group_messages_into_display_items(&s.messages);
							s.prev_msg_count = s.display_items.len();

							let previous_row = parts.conversations_parts.messages_list.get_selection();
							let rows: Vec<String> =
								s.display_items.iter().map(|item| item.format_display(is_group)).collect();
							parts.conversations_parts.messages_list.freeze();
							parts
								.conversations_parts
								.messages_list
								.set_items(&rows.iter().map(String::as_str).collect::<Vec<_>>());
							parts.conversations_parts.messages_list.thaw();
							refresh_selection_list(&mut s, parts);
							crate::ui_commands::calls::mark_joinable_rows(&s, parts);
							let count = parts.conversations_parts.messages_list.get_count();
							match previous_row {
								Some(row) if count > 0 && s.settings.keep_list_position => {
									parts.conversations_parts.messages_list.set_selection(row.min(count - 1), true);
								}
								_ if count > 0 => {
									parts.conversations_parts.messages_list.set_selection(count - 1, true);
								}
								_ => {}
							}
							refresh_voice_action_button(&s, &parts.conversations_parts);
							if !suppress_announce {
								crate::platform::announce::announce(
									parts.conversations_parts.messages_announcer,
									if upload_cancelled { "Upload cancelled." } else { "Message deleted." },
								);
							}
						}
						return true;
					}

					if let Some(di) = update_loaded_message(&mut s, message_id, |m| {
						m.is_deleted = true;
						m.text = String::new();
					}) {
						if let Some(item) = s.display_items.get(di) {
							parts
								.conversations_parts
								.messages_list
								.set_string(di as u32, &item.format_display(is_group));

							if let Some(sel_idx) = parts.conversations_parts.messages_list.get_selection()
								&& sel_idx == di as u32
							{
								let mut need_layout = crate::update_action_buttons(&s, &parts.conversations_parts);
								if parts.conversations_parts.play_voice_btn.is_shown() {
									parts.conversations_parts.play_voice_btn.show(false);
									need_layout = true;
								}
								if parts.conversations_parts.audio_slider.is_shown() {
									parts.conversations_parts.audio_slider.show(false);
									parts.conversations_parts.audio_progress_label.show(false);
									parts.conversations_parts.audio_speed_btn.show(false);
									need_layout = true;
								}
								if need_layout {
									parts.conversations_parts.action_panel.layout();
									parts.conversations_parts.conversation_panel.layout();
								}
							}
						}
						if !suppress_announce {
							crate::platform::announce::announce(
								parts.conversations_parts.messages_announcer,
								"Message deleted.",
							);
						}
					}
				}
			}
		}
		"history_start_reached" => {
			let Some(jid) = val.get("data").and_then(|d| d.get("jid")).and_then(|j| j.as_str()) else { return true };
			let open = {
				let mut s = state.borrow_mut();
				s.history_start_chats.insert(jid.to_string());
				if s.phone_history_pending_jid.as_deref() == Some(jid) {
					s.phone_history_pending_jid = None;
					s.phone_history_deadline = None;
					s.history_loading = false;
				}
				s.selected_chat.as_deref() == Some(jid)
			};
			if open {
				crate::platform::announce::announce(
					parts.conversations_parts.messages_announcer,
					"This is the start of the chat.",
				);
			}
		}
		"no_more_history" => {
			let mut s = state.borrow_mut();
			s.history_loading = false;
			if let Some(jid) = val.get("data").and_then(|d| d.get("jid")).and_then(|j| j.as_str())
				&& !jid.is_empty()
			{
				eprintln!("[LOG UI] Backend held back a repeat history request for chat: {}", jid);
				if s.phone_history_pending_jid.as_deref() == Some(jid) {
					s.phone_history_pending_jid = None;
					s.phone_history_deadline = None;
				}
			}
		}
		"chat_cleared" => {
			if let Some(jid) = val.get("data").and_then(|d| d.get("jid")).and_then(|j| j.as_str()) {
				let mut s = state.borrow_mut();
				if s.selected_chat.as_deref() == Some(jid) && !s.messages.is_empty() {
					s.messages.clear();
					s.display_items.clear();
					s.prev_msg_count = 0;
					drop(s);
					parts.conversations_parts.messages_list.clear();
					update_action_buttons(&state.borrow(), &parts.conversations_parts);
					crate::platform::announce::announce(parts.conversations_parts.messages_announcer, "Chat cleared.");
				}
			}
		}
		"chat_updated" => {
			let data = val.get("data");
			let jid = data.and_then(|d| d.get("jid")).and_then(|j| j.as_str()).unwrap_or_default();
			let unread = data.and_then(|d| d.get("unread_count")).and_then(|u| u.as_i64());
			if !jid.is_empty() && unread == Some(0) {
				crate::ui::chat_ui::clear_chat_notifications(jid);
				let mut s = state.borrow_mut();
				if s.notification_reads.remove(jid) {
					crate::ui::chat_ui::clear_unread_divider_for_open_chat(&mut s, parts, jid);
					// Acknowledged with the rest; don't send them again on refocus.
					if s.selected_chat.as_deref() == Some(jid) {
						s.background_unread_message_ids.clear();
					}
				}
				if s.unread_counts.insert(jid.to_string(), 0).is_some_and(|n| n != 0) {
					crate::platform::tray::refresh_tray_tooltip(&s, parts);
					let _ = s.ws_tx.send(serde_json::json!({ "action": "get_chats" }).to_string());
				}
			}
		}
		"mark_read_failed" => {
			let jid = val.get("data").and_then(|d| d.get("jid")).and_then(|j| j.as_str()).unwrap_or_default();
			let mut s = state.borrow_mut();
			let from_notification = s.notification_reads.remove(jid);
			let _ = s.ws_tx.send(serde_json::json!({ "action": "get_chats" }).to_string());
			if from_notification {
				let name = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.to_string());
				drop(s);
				crate::ui::chat_ui::show_toast_notification(
					whatsnap::TRAY_APP_NAME,
					&format!("Couldn't mark {} as read. Check your connection and try again.", name),
				);
			}
		}
		"play_audio_failed" => {
			eprintln!("[LOG UI] Voice message audio failed to load or download");
			crate::platform::announce::announce(
				parts.conversations_parts.messages_announcer,
				"Could not load voice message",
			);
		}
		"message_info" => {
			if let Some(data) = val.get("data") {
				let fmt_ts = |ts: i64| -> String {
					if ts == 0 {
						return String::new();
					}
					let Some(dt) = whatsnap::LocalTime::from_unix(ts) else {
						return String::new();
					};
					let time = dt.format("%l:%M %p").trim().to_lowercase();
					if dt.date() == whatsnap::LocalTime::now().date() { time } else { whatsnap::day_and_time(dt) }
				};
				let msg_id = data.get("id").and_then(|v| v.as_str()).unwrap_or("");
				let mut sent_s = data.get("sent_at").and_then(|v| v.as_i64()).unwrap_or(0);
				let mut delivered_s = data.get("delivered_at").and_then(|v| v.as_i64()).unwrap_or(0);
				let mut read_s = data.get("read_at").and_then(|v| v.as_i64()).unwrap_or(0);
				let mut played_s = data.get("played_at").and_then(|v| v.as_i64()).unwrap_or(0);
				let mut edited_s = data.get("edited_at").and_then(|v| v.as_i64()).unwrap_or(0);
				let media_type = data.get("media_type").and_then(|v| v.as_str()).unwrap_or("");
				let is_voice = media_type == "voice" || media_type == "audio";

				{
					let s = state.borrow();
					if let Some(msg) = s.messages.iter().find(|m| m.id == msg_id) {
						if sent_s == 0 {
							sent_s = if msg.sent_at > 0 { msg.sent_at } else { msg.timestamp };
						}
						if delivered_s == 0 && msg.delivered_at > 0 {
							delivered_s = msg.delivered_at;
						}
						if read_s == 0 && msg.read_at > 0 {
							read_s = msg.read_at;
						}
						if played_s == 0 && msg.played_at > 0 {
							played_s = msg.played_at;
						}
						if edited_s == 0 && msg.edited_at > 0 {
							edited_s = msg.edited_at;
						}
					}
				}

				let mut lines = Vec::new();
				let sent_fmt = fmt_ts(sent_s);
				if !sent_fmt.is_empty() {
					lines.push(format!("Sent at {}", sent_fmt));
				}
				let del_fmt = fmt_ts(delivered_s);
				if !del_fmt.is_empty() {
					lines.push(format!("Delivered at {}", del_fmt));
				}
				let read_fmt = fmt_ts(read_s);
				if !read_fmt.is_empty() {
					lines.push(format!("Read at {}", read_fmt));
				}
				if is_voice {
					let played_fmt = fmt_ts(played_s);
					if !played_fmt.is_empty() {
						lines.push(format!("Played at {}", played_fmt));
					}
				}
				let edited_fmt = fmt_ts(edited_s);
				if !edited_fmt.is_empty() {
					lines.push(format!("Edited at {}", edited_fmt));
				}

				if let Some(participants) = data.get("participants").and_then(|p| p.as_array())
					&& !participants.is_empty()
				{
					lines.retain(|l| l.starts_with("Sent at") || l.starts_with("Edited at"));

					let mut played = Vec::new();
					let mut read = Vec::new();
					let mut delivered = Vec::new();
					let mut sent = Vec::new();
					for p in participants {
						let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
						let pstatus = p.get("status").and_then(|v| v.as_str()).unwrap_or("sent");
						let pts = p.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0);
						let p_at = |key: &str| p.get(key).and_then(|v| v.as_i64()).unwrap_or(0);
						let mut times = Vec::new();
						if is_voice {
							let played = fmt_ts(p_at("played_at"));
							if !played.is_empty() {
								times.push(format!("played {}", played));
							}
						}
						let read_at = fmt_ts(p_at("read_at"));
						if !read_at.is_empty() {
							times.push(format!("read {}", read_at));
						}
						let delivered_at = fmt_ts(p_at("delivered_at"));
						if !delivered_at.is_empty() {
							times.push(format!("delivered {}", delivered_at));
						}
						if times.is_empty() {
							let ts_fmt = fmt_ts(pts);
							if !ts_fmt.is_empty() {
								times.push(ts_fmt);
							}
						}
						let entry = if times.is_empty() {
							name.to_string()
						} else {
							format!("{} — {}", name, times.join(", "))
						};
						match pstatus {
							"played" => played.push(entry),
							"read" => read.push(entry),
							"delivered" => delivered.push(entry),
							_ => sent.push(entry),
						}
					}

					let mut push_group = |label: &str, entries: &Vec<String>| {
						if entries.is_empty() {
							return;
						}
						lines.push(String::new());
						lines.push(label.to_string());
						for e in entries {
							lines.push(format!("  {}", e));
						}
					};
					push_group("Played by", &played);
					push_group("Read by", &read);
					push_group("Delivered to", &delivered);
					push_group("Sent to", &sent);
				}

				if let Some(poll) = data.get("poll") {
					let from_me = state.borrow().messages.iter().any(|m| m.id == msg_id && m.from_me);
					if !from_me {
						lines.clear();
					}
					let mut poll_lines = Vec::new();
					let question = poll.get("question").and_then(|v| v.as_str()).unwrap_or("");
					poll_lines.push(format!("Poll: {}", question));
					let results = poll.get("results").and_then(|v| v.as_str()).unwrap_or("");
					if !results.is_empty() {
						poll_lines.push(results.to_string());
					}
					let voters = poll.get("voters").and_then(|v| v.as_array()).cloned().unwrap_or_default();
					let is_channel = poll.get("channel").and_then(|v| v.as_bool()).unwrap_or(false);
					poll_lines.push(String::new());
					if voters.is_empty() {
						poll_lines.push("No votes yet.".to_string());
					} else {
						poll_lines.push("Votes".to_string());
						for v in &voters {
							let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("");
							let options: Vec<&str> = v
								.get("options")
								.and_then(|x| x.as_array())
								.map(|a| a.iter().filter_map(|o| o.as_str()).collect())
								.unwrap_or_default();
							let when = fmt_ts(v.get("timestamp").and_then(|x| x.as_i64()).unwrap_or(0));
							let entry = format!("  {} voted {}", name, options.join(" and "));
							poll_lines.push(if when.is_empty() { entry } else { format!("{}, {}", entry, when) });
						}
					}
					if is_channel {
						poll_lines.push(
								"Channels show vote totals, not who voted. Admins can see voters from the Channel post menu.".to_string(),
							);
					}
					if !lines.is_empty() {
						poll_lines.push(String::new());
					}
					poll_lines.append(&mut lines);
					lines = poll_lines;
				}

				if lines.is_empty() {
					lines.push("No receipt information available.".to_string());
				}
				let info_text = lines.join("\n");

				let dlg =
					Dialog::builder(&parts.frame, "Message Info").with_style(DialogStyle::DefaultDialogStyle).build();
				let sizer = BoxSizer::builder(Orientation::Vertical).build();
				let txt = TextCtrl::builder(&dlg)
					.with_value(&info_text)
					.with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly | TextCtrlStyle::DontWrap)
					.build();
				sizer.add(&txt, 1, SizerFlag::Expand | SizerFlag::All, 10);
				let ok_btn = Button::builder(&dlg).with_label("OK").build();
				sizer.add(&ok_btn, 0, SizerFlag::AlignCentre | SizerFlag::Bottom | SizerFlag::All, 8);
				dlg.set_sizer_and_fit(sizer, true);
				let dlg_close = dlg;
				ok_btn.on_click(move |_| {
					dlg_close.end_modal(wxdragon::id::ID_OK);
				});
				dlg.show_modal();
				dlg.destroy();
			}
		}
		"chat_media" => {
			#[derive(Deserialize)]
			struct ChatMedia {
				jid: String,
				#[serde(default)]
				media: Vec<ChatMessage>,
				#[serde(default)]
				documents: Vec<ChatMessage>,
				#[serde(default)]
				links: Vec<ChatMessage>,
			}
			let Some(data) = val.get("data").and_then(|d| ChatMedia::deserialize(d).ok()) else { return true };
			let mut s = state.borrow_mut();
			let jid = Some(data.jid.as_str());
			if s.selected_chat.as_deref() != jid && s.info_chat.as_deref() != jid {
				return true;
			}
			let found: Vec<whatsnap::Msg> =
				data.media.into_iter().chain(data.documents).chain(data.links).map(whatsnap::Msg::new).collect();
			let (gd, cd) = (&parts.group_data_dialog, &parts.conversation_data_dialog);
			if gd.dialog.is_shown() {
				let lists =
					InfoMediaLists { media: &gd.media_list, documents: &gd.documents_list, links: &gd.links_list };
				(s.group_data_media, s.group_data_documents, s.group_data_links) =
					fill_info_media(&s, lists, Some(&found));
			} else if cd.dialog.is_shown() {
				let lists =
					InfoMediaLists { media: &cd.media_list, documents: &cd.documents_list, links: &cd.links_list };
				(s.conversation_data_media, s.conversation_data_documents, s.conversation_data_links) =
					fill_info_media(&s, lists, Some(&found));
			}
		}
		"history_updated" => {
			eprintln!("[LOG UI] History sync completed on phone! Refreshing messages for active chat...");
			let s = state.borrow();
			if let Some(ref jid) = s.selected_chat.clone() {
				let payload = serde_json::json!({
					"action": "get_messages",
					"data": { "jid": jid, "limit": crate::MESSAGE_PAGE_SIZE, "offset": 0 }
				});
				let _ = s.ws_tx.send(payload.to_string());
			}
		}
		"export_progress" => {
			if let Some(data) = val.get("data") {
				let current = data.get("current").and_then(|c| c.as_i64()).unwrap_or(0);
				let total = data.get("total").and_then(|t| t.as_i64()).unwrap_or(0);
				parts.frame.set_status_text(&format!("Status: Exporting chat... {}/{}", current, total), 0);
			}
		}
		"chat_exported" => {
			if let Some(data) = val.get("data") {
				let path = data.get("path").and_then(|p| p.as_str()).unwrap_or("");
				let conn_status = state.borrow().conn_status_text.clone();
				parts.frame.set_status_text(&conn_status, 0);
				if data.get("text_only").and_then(|t| t.as_bool()).unwrap_or(false) {
					let count = data.get("message_count").and_then(|m| m.as_i64()).unwrap_or(0);
					let msg = format!("Chat text exported to {}. {} messages.", path, count);
					crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &msg);
					return true;
				}
				let media_exported = data.get("media_exported").and_then(|m| m.as_i64()).unwrap_or(0);
				let media_skipped = data.get("media_skipped").and_then(|m| m.as_i64()).unwrap_or(0);
				let msg = if media_skipped > 0 {
					format!(
						"Chat exported to {}. {} media files included, {} could not be downloaded and were skipped.",
						path, media_exported, media_skipped
					)
				} else {
					format!("Chat exported to {}. {} media files included.", path, media_exported)
				};
				crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &msg);
			}
		}
		"starred_messages" => {
			if let Some(data) = val.get("data")
				&& let Some(results) = data.get("results").and_then(|r| r.as_array())
			{
				let mut s = state.borrow_mut();
				let mut entries = Vec::new();
				for item in results {
					let jid = item.get("jid").and_then(|j| j.as_str()).unwrap_or("").to_string();
					let message_id = item.get("id").and_then(|m| m.as_str()).unwrap_or("").to_string();
					if jid.is_empty() || message_id.is_empty() {
						continue;
					}
					let from_me = item.get("fromMe").and_then(|f| f.as_bool()).unwrap_or(false);
					let sender_name = item.get("senderName").and_then(|n| n.as_str()).unwrap_or("");
					let sender_jid = item.get("senderJid").and_then(|n| n.as_str()).unwrap_or("").to_string();
					let text = item.get("text").and_then(|t| t.as_str()).unwrap_or("");
					let quoted_text = item.get("quotedText").and_then(|t| t.as_str()).unwrap_or("");
					let media_type = item.get("mediaType").and_then(|t| t.as_str()).unwrap_or("");
					let timestamp = item.get("timestamp").and_then(|t| t.as_i64()).unwrap_or(0);

					let chat_name = s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
					let sender = if from_me {
						"You".to_string()
					} else if !sender_name.is_empty() {
						sender_name.to_string()
					} else {
						chat_name.clone()
					};
					let body = if !text.is_empty() {
						whatsnap::friendly_preview_text(text)
					} else if !quoted_text.is_empty() {
						whatsnap::friendly_preview_text(quoted_text)
					} else if !media_type.is_empty() {
						"Media".to_string()
					} else {
						"Message".to_string()
					};
					let date_str = if timestamp > 0 {
						whatsnap::LocalTime::from_unix(timestamp)
							.map(|local_dt| whatsnap::day_name(local_dt, "%b %e, %Y"))
							.unwrap_or_default()
					} else {
						String::new()
					};
					let label = if date_str.is_empty() {
						format!("{} ({}): {}", chat_name, sender, body)
					} else {
						format!("{} ({}): {}, {}", chat_name, sender, body, date_str)
					};

					entries.push(StarredEntry { jid, message_id, sender_jid, from_me, label });
				}
				s.starred_entries = entries;
				if s.active_tab == 2 {
					apply_chat_filter(&mut s, parts);
				}
			}
		}
		"search_results" => {
			if let Some(data) = val.get("data") {
				let response_jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
				if state.borrow().selected_chat.as_deref() != Some(response_jid) {
					return true;
				}
				if let Some(results) = data.get("results").and_then(|r| r.as_array()) {
					let mut parsed_msgs: Vec<ChatMessage> = Vec::new();
					for msg_val in results {
						if let Ok(msg) = serde_json::from_value::<ChatMessage>(msg_val.clone()) {
							parsed_msgs.push(msg);
						}
					}

					let mut with_dates = Vec::new();
					let mut last_date: Option<whatsnap::Date> = None;
					for msg in parsed_msgs {
						if let Some(local_datetime) = whatsnap::LocalTime::from_unix(msg.timestamp) {
							let day = whatsnap::date_key(local_datetime);
							if last_date != Some(day) {
								last_date = Some(day);
								with_dates.push(whatsnap::Msg::new(ChatMessage {
									text: whatsnap::format_date_divider_label(local_datetime),
									timestamp: msg.timestamp,
									is_date_divider: true,
									..Default::default()
								}));
							}
						}
						with_dates.push(whatsnap::Msg::new(msg));
					}

					let mut s = state.borrow_mut();
					let is_group = s.selected_chat.as_ref().map(|j| j.ends_with("@g.us")).unwrap_or(false);
					s.messages = with_dates;
					s.display_items = whatsnap::group_messages_into_display_items(&s.messages);
					s.older_unloaded = 0;
					s.newer_unloaded = 0;

					parts.conversations_parts.messages_list.freeze();
					parts.conversations_parts.messages_list.clear();
					for item in &s.display_items {
						parts.conversations_parts.messages_list.append(&item.format_display(is_group));
					}
					parts.conversations_parts.messages_list.thaw();
					refresh_selection_list(&mut s, parts);
					crate::ui_commands::calls::mark_joinable_rows(&s, parts);
					let count = parts.conversations_parts.messages_list.get_count();
					if count > 0 {
						parts.conversations_parts.messages_list.set_selection(0, true);
					}
					refresh_voice_action_button(&s, &parts.conversations_parts);
				}
			}
		}
		"typing" => {
			if let Some(data) = val.get("data") {
				let chat_jid = data.get("chat_jid").or_else(|| data.get("jid")).and_then(|j| j.as_str()).unwrap_or("");
				let mut sender_jid =
					data.get("sender_jid").or_else(|| data.get("jid")).and_then(|j| j.as_str()).unwrap_or("");
				if sender_jid.is_empty() {
					sender_jid = chat_jid;
				}
				if chat_jid.is_empty() || sender_jid.is_empty() {
					return true;
				}
				let is_typing = data.get("typing").and_then(|t| t.as_bool()).unwrap_or(false);
				let is_recording = data.get("recording").and_then(|r| r.as_bool()).unwrap_or(false);

				let state_changed = {
					let mut s = state.borrow_mut();
					if is_typing || is_recording {
						s.typing_timestamps.insert(chat_jid.to_string(), std::time::Instant::now());
					} else {
						s.typing_timestamps.remove(chat_jid);
					}
					let new_kind: u8 = if is_recording {
						2
					} else if is_typing {
						1
					} else {
						0
					};
					let prev_kind = s.typing_announce_state.get(chat_jid).copied().unwrap_or(0);
					if new_kind == 0 {
						s.typing_announce_state.remove(chat_jid);
					} else {
						s.typing_announce_state.insert(chat_jid.to_string(), new_kind);
					}
					new_kind != prev_kind
				};
				if state_changed {
					crate::ui::chat_rows::refresh();
				}

				let s = state.borrow();
				if Some(chat_jid.to_string()) == s.selected_chat {
					let sender_name =
						s.contacts.get(sender_jid).cloned().unwrap_or_else(|| display_name_for_jid(sender_jid));
					if !sender_name.is_empty() {
						let announcement = if is_recording && s.settings.announce_recording {
							Some(format!("{} is recording audio...", sender_name))
						} else if is_typing && s.settings.announce_typing {
							Some(format!("{} is typing...", sender_name))
						} else {
							None
						};
						if let Some(ann) = announcement
							&& state_changed
						{
							crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &ann);
						}
						let norm_chat_jid = if let Some((user, server)) = chat_jid.split_once('@') {
							let base = user.split('.').next().unwrap_or(user).split(':').next().unwrap_or(user);
							format!("{}@{}", base, server)
						} else {
							chat_jid.to_string()
						};
						let chat_name = s
							.contacts
							.get(chat_jid)
							.or_else(|| s.contacts.get(&norm_chat_jid))
							.cloned()
							.unwrap_or_else(|| display_name_for_jid(chat_jid));
						let presence_label = s
							.presence_labels
							.get(chat_jid)
							.or_else(|| s.presence_labels.get(&norm_chat_jid))
							.map(|l| l.as_str());
						let btn_label = format_conv_data_label(
							chat_jid,
							&chat_name,
							&sender_name,
							is_typing,
							is_recording,
							presence_label,
						);
						parts.conversations_parts.conv_data_btn.set_label(&btn_label);
					}
				}
			}
		}
		"presence" => {
			if let Some(data) = val.get("data") {
				let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
				if jid.is_empty() || jid.ends_with("@g.us") {
					return true;
				}
				let clean_jid = if let Some((user, server)) = jid.split_once('@') {
					let base_user = user.split('.').next().unwrap_or(user).split(':').next().unwrap_or(user);
					format!("{}@{}", base_user, server)
				} else {
					jid.to_string()
				};
				let unavailable = data.get("unavailable").and_then(|u| u.as_bool()).unwrap_or(false);
				let last_seen = data.get("last_seen").and_then(|l| l.as_i64()).unwrap_or(0);
				let mut s = state.borrow_mut();

				let name = s
					.contacts
					.get(&clean_jid)
					.or_else(|| s.contacts.get(jid))
					.cloned()
					.unwrap_or_else(|| display_name_for_jid(&clean_jid));

				let btn_label = if unavailable {
					if last_seen > 0 {
						if let Some(local_dt) = whatsnap::LocalTime::from_unix(last_seen) {
							let date_str = whatsnap::day_and_time(local_dt);
							format!("{} (last seen {})", name, date_str)
						} else {
							name.clone()
						}
					} else {
						name.clone()
					}
				} else {
					format!("{} (online)", name)
				};

				s.presence_labels.insert(clean_jid.clone(), btn_label.clone());
				s.presence_labels.insert(jid.to_string(), btn_label.clone());
				let was_online = s.contact_online.insert(clean_jid.clone(), !unavailable);
				if !unavailable && was_online != Some(true) {
					crate::ui_commands::messaging::contact_came_online(&mut s, &clean_jid);
				}

				let is_currently_typing = s
					.typing_timestamps
					.get(&clean_jid)
					.or_else(|| s.typing_timestamps.get(jid))
					.or_else(|| s.selected_chat.as_ref().and_then(|sel| s.typing_timestamps.get(sel)))
					.is_some_and(|t| t.elapsed().as_secs() < 5);

				if unavailable {
					s.typing_timestamps.remove(&clean_jid);
					s.typing_timestamps.remove(jid);
					s.typing_announce_state.remove(&clean_jid);
					s.typing_announce_state.remove(jid);
				}

				if Some(clean_jid.clone()) == s.selected_chat || Some(jid.to_string()) == s.selected_chat {
					let mut should_announce = true;
					if !s.initial_presence_received {
						s.initial_presence_received = true;
						should_announce = false;
					}

					let announcement = if unavailable {
						if last_seen > 0 {
							if let Some(local_dt) = whatsnap::LocalTime::from_unix(last_seen) {
								let date_str = whatsnap::day_and_time(local_dt);
								format!("{}, last seen {}", name, date_str)
							} else {
								format!("{}, offline", name)
							}
						} else {
							format!("{}, offline", name)
						}
					} else {
						format!("{} is now online", name)
					};
					if should_announce {
						crate::platform::announce::announce(
							parts.conversations_parts.messages_announcer,
							&announcement,
						);
					}
					if unavailable || !is_currently_typing {
						parts.conversations_parts.conv_data_btn.set_label(&btn_label);
					}
				}
			}
		}
		"mention_candidates" => {
			if let Some(data) = val.get("data") {
				crate::ui_commands::mentions::on_candidates(state, &parts.conversations_parts, data);
			}
		}
		"message_pinned" | "message_kept" | "pinned_messages" | "chat_privacy" => {
			if let Some(data) = val.get("data") {
				use crate::ui_commands::message_extras as extras;
				match t {
					"message_pinned" => extras::message_pinned(state, parts, data),
					"message_kept" => extras::message_kept(state, parts, data),
					"pinned_messages" => extras::pinned_messages(state, parts, data),
					_ => extras::chat_privacy(state, parts, data),
				}
			}
		}
		"media_ready" | "media_failed" | "media_pending" => {
			if let Some(data) = val.get("data") {
				crate::ui::status_viewer::on_event(t, data);
			}
		}
		"stickers" | "sticker_store" | "sticker_pack" => {
			let panel = state.borrow().emoji_panel.clone();
			if let (Some(panel), Some(data)) = (panel, val.get("data")) {
				match t {
					"stickers" => panel.apply_stickers(data),
					"sticker_store" => panel.apply_store(data),
					_ => panel.apply_pack(data),
				}
			}
		}
		"sticker_favorite" => {
			if let Some(data) = val.get("data") {
				crate::ui_commands::stickers::sticker_favorite(state, parts, data);
			}
		}
		"chat_limit_sharing" => {
			if let Some(data) = val.get("data") {
				let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or_default();
				let enabled = data.get("enabled").and_then(|e| e.as_bool()).unwrap_or(false);
				crate::ui_commands::mentions::on_limit_sharing(state, &parts.conversations_parts, jid, enabled);
			}
		}
		"thumbnails" => {
			if let Some(data) = val.get("data") {
				crate::ui::thumbnails::on_thumbnails(data);
			}
		}
		"waveforms" => {
			if let Some(data) = val.get("data") {
				crate::ui::thumbnails::on_waveforms(data);
			}
		}
		"poll_details" => {
			if let Some(data) = val.get("data") {
				crate::ui_commands::polls::show_poll_vote_dialog(state, parts, data);
			}
		}
		"message_options" => {
			if let Some(data) = val.get("data") {
				crate::ui_commands::business::show_message_options(state, parts, data);
			}
		}
		"catalog" => {
			if let Some(data) = val.get("data") {
				crate::ui_commands::business::show_catalog(state, parts, data);
			}
		}
		"order_details" => {
			if let Some(data) = val.get("data") {
				crate::ui_commands::business::show_order_details(parts, data);
			}
		}
		"message_option_sent" => {
			let label = val.pointer("/data/label").and_then(|l| l.as_str()).unwrap_or("");
			crate::platform::announce::announce(
				parts.conversations_parts.messages_announcer,
				&format!("Sent: {label}"),
			);
		}
		"event_details" => {
			if let Some(data) = val.get("data") {
				crate::ui_commands::polls::show_event_rsvp_dialog(state, parts, data);
			}
		}
		"event_answered" => {
			if let Some(data) = val.get("data") {
				let results = data.get("pollResults").and_then(|m| m.as_str()).unwrap_or("");
				crate::platform::announce::announce(
					parts.conversations_parts.messages_announcer,
					&format!("Answer sent. {}", results),
				);
			}
		}
		"poll_updated" => {
			if let Some(data) = val.get("data") {
				let id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("");
				let results = data.get("pollResults").and_then(|m| m.as_str()).unwrap_or("");
				crate::ui_commands::polls::update_poll_results(state, parts, id, results);
			}
		}
		"poll_voted" => {
			if let Some(data) = val.get("data") {
				let results = data.get("pollResults").and_then(|m| m.as_str()).unwrap_or("");
				let retracted = data.get("retracted").and_then(|m| m.as_bool()).unwrap_or(false);
				let lead = if retracted { "Vote removed." } else { "Voted." };
				crate::platform::announce::announce(
					parts.conversations_parts.messages_announcer,
					&format!("{} {}", lead, results),
				);
			}
		}
		"message_edited" | "message_updated" => {
			if let Some(data) = val.get("data") {
				let chat_jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
				let target_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("");
				let new_text = data.get("text").and_then(|t| t.as_str()).unwrap_or("");
				let edited_at = data.get("edited_at").and_then(|e| e.as_i64()).unwrap_or(0);
				// Meta AI (and other bots) stream a reply as several
				// edits of the same bubble while "typing"; the backend
				// marks all but the finished one `final: false` so
				// NVDA doesn't read out every growing partial draft.
				// Absent (a real edit, not a bot stream) means final.
				let is_final_edit = data.get("final").and_then(|f| f.as_bool()).unwrap_or(true);
				let not_an_edit = data.get("not_edit").and_then(|f| f.as_bool()).unwrap_or(false);
				let relabelled = t == "message_updated";

				let norm_jid_edit = |jid: &str| -> String {
					if let Some((user, server)) = jid.split_once('@') {
						let base_user = user.split('.').next().unwrap_or(user).split(':').next().unwrap_or(user);
						format!("{}@{}", base_user, server)
					} else {
						jid.to_string()
					}
				};

				let mut s = state.borrow_mut();
				let is_match = if let Some(ref sel) = s.selected_chat {
					norm_jid_edit(chat_jid) == norm_jid_edit(sel)
				} else {
					false
				};

				if is_match {
					let is_group = chat_jid.ends_with("@g.us");
					let edited_row = update_loaded_message(&mut s, target_id, |m| {
						m.text = new_text.to_string();
						if not_an_edit && !relabelled {
							m.media_path.clear();
						} else if !not_an_edit && !relabelled {
							m.is_edited = true;
							m.edited_at = edited_at;
						}
					});
					if let Some(di) = edited_row {
						let quoting_rows = update_loaded_messages(
							&mut s,
							|m| m.quoted_id == target_id,
							|m| m.quoted_text = new_text.to_string(),
						);
						if let Some(item) = s.display_items.get(di) {
							let updated_display = item.format_display(is_group);
							parts.conversations_parts.messages_list.set_string(di as u32, &updated_display);
							if is_final_edit && !not_an_edit && !relabelled {
								crate::platform::announce::announce(
									parts.conversations_parts.messages_announcer,
									&format!("Message edited: {}", updated_display),
								);
							}
						}
						for row in quoting_rows.into_iter().filter(|&row| row != di) {
							if let Some(item) = s.display_items.get(row) {
								parts
									.conversations_parts
									.messages_list
									.set_string(row as u32, &item.format_display(is_group));
							}
						}
					}
				}
			}
		}
		"message_starred" => {
			if let Some(data) = val.get("data") {
				let chat_jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
				let target_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("");
				let starred = data.get("starred").and_then(|s| s.as_bool()).unwrap_or(false);

				let norm_jid_star = |jid: &str| -> String {
					if let Some((user, server)) = jid.split_once('@') {
						let base_user = user.split('.').next().unwrap_or(user).split(':').next().unwrap_or(user);
						format!("{}@{}", base_user, server)
					} else {
						jid.to_string()
					}
				};

				let mut s = state.borrow_mut();
				let is_match = if let Some(ref sel) = s.selected_chat {
					norm_jid_star(chat_jid) == norm_jid_star(sel)
				} else {
					false
				};

				if is_match {
					let is_group = chat_jid.ends_with("@g.us");
					if let Some(di) = update_loaded_message(&mut s, target_id, |m| m.starred = starred)
						&& let Some(item) = s.display_items.get(di)
					{
						let updated_display = item.format_display(is_group);
						parts.conversations_parts.messages_list.set_string(di as u32, &updated_display);
						crate::platform::announce::announce(
							parts.conversations_parts.messages_announcer,
							if starred { "Message starred." } else { "Message unstarred." },
						);
					}
				}
				if s.active_tab == 2 {
					let payload = serde_json::json!({ "action": "get_starred_messages" });
					let _ = s.ws_tx.send(payload.to_string());
				}
			}
		}
		"message_reaction" => {
			if let Some(data) = val.get("data") {
				let chat_jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
				let target_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("");
				let reactor = data.get("reactor").and_then(|r| r.as_str()).unwrap_or("");
				let emoji = data.get("emoji").and_then(|e| e.as_str()).unwrap_or("");
				let reactor_name_val = data.get("reactor_name").and_then(|n| n.as_str()).unwrap_or("");

				let norm_jid_react = |jid: &str| -> String {
					let no_device = jid.split(':').next().unwrap_or(jid);
					if no_device.contains('@') {
						no_device.to_string()
					} else {
						jid.split('@').next().unwrap_or(jid).to_string()
					}
				};

				let mut s = state.borrow_mut();
				let is_match = if let Some(ref sel) = s.selected_chat {
					norm_jid_react(chat_jid) == norm_jid_react(sel)
				} else {
					false
				};

				if is_match {
					let is_group = chat_jid.ends_with("@g.us");

					let is_own_reaction = reactor == "me" || reactor_name_val == "You" || reactor_name_val == "me";

					let resolved_name = if is_own_reaction {
						"You".to_string()
					} else if !reactor_name_val.is_empty() && !reactor_name_val.contains('@') {
						reactor_name_val.to_string()
					} else {
						s.contacts
							.get(norm_jid_react(reactor).as_str())
							.or_else(|| s.contacts.get(reactor))
							.cloned()
							.unwrap_or_else(|| "Contact".to_string())
					};

					let reacted_row = update_loaded_message(&mut s, target_id, |msg| {
						if emoji.is_empty() {
							msg.reactions.retain(|r| {
								if is_own_reaction {
									r.reactor_name != "You" && r.reactor_jid != "me" && r.reactor_name != "me"
								} else {
									r.reactor_jid != reactor && r.reactor_name != resolved_name
								}
							});
						} else {
							let mut found = false;
							for r in msg.reactions.iter_mut() {
								let r_is_own = r.reactor_name == "You" || r.reactor_jid == "me";
								if (is_own_reaction && r_is_own)
									|| (!is_own_reaction
										&& (r.reactor_jid == reactor || r.reactor_name == resolved_name))
								{
									r.emoji = emoji.to_string();
									r.reactor_name = resolved_name.clone();
									found = true;
									break;
								}
							}
							if !found {
								msg.reactions.push(whatsnap::ChatReaction {
									reactor_jid: if is_own_reaction { "me".to_string() } else { reactor.to_string() },
									reactor_name: resolved_name.clone(),
									emoji: emoji.to_string(),
									timestamp: std::time::SystemTime::now()
										.duration_since(std::time::UNIX_EPOCH)
										.unwrap_or_default()
										.as_secs() as i64,
								});
							}
						}
					});

					if let Some((di, item)) = reacted_row.and_then(|di| s.display_items.get(di).map(|item| (di, item)))
					{
						parts.conversations_parts.messages_list.set_string(di as u32, &item.format_display(is_group));
						let announce_msg = if emoji.is_empty() {
							format!("Reaction removed by {}", resolved_name)
						} else {
							format!("{} reacted with {}", resolved_name, emoji)
						};
						crate::platform::announce::announce(
							parts.conversations_parts.messages_announcer,
							&announce_msg,
						);
					}
				} else if !emoji.is_empty() {
					let is_own_reaction = reactor == "me" || reactor_name_val == "You" || reactor_name_val == "me";
					if !is_own_reaction {
						let text = data.get("text").and_then(|t| t.as_str()).unwrap_or("");
						if !text.is_empty() {
							let resolved_name = if !reactor_name_val.is_empty() && !reactor_name_val.contains('@') {
								reactor_name_val.to_string()
							} else {
								s.contacts
									.get(norm_jid_react(reactor).as_str())
									.or_else(|| s.contacts.get(reactor))
									.cloned()
									.unwrap_or_else(|| "Contact".to_string())
							};
							let alert = s.settings.message_alert(
								chat_jid,
								s.muted_jids.contains(chat_jid),
								s.archived_jids.contains(chat_jid),
							);
							let alert = if s.settings.reaction_notifications {
								alert
							} else {
								whatsnap::IncomingAlert { sound: false, toast: false }
							};
							if alert.sound && !crate::platform::do_not_disturb::should_silence(&s.settings) {
								let (tone, file) = s.settings.message_tone_for(chat_jid);
								crate::audio::play_message_tone(
									tone,
									&file,
									s.settings.sounds_enabled,
									s.settings.output_device_name.clone(),
								);
							}
							if alert.toast && s.locked_jids.contains(chat_jid) {
								crate::ui::chat_ui::show_chat_notification(chat_jid, "WhatsApp", "New reaction");
							} else if alert.toast {
								let previews =
									s.settings.notification_previews && !s.settings.chat_notify(chat_jid).hide_content;
								let body = if previews {
									format!("Reacted {} to \"{}\"", emoji, whatsnap::friendly_preview_text(text))
								} else {
									format!("Reacted {}", emoji)
								};
								crate::ui::chat_ui::show_chat_notification(chat_jid, &resolved_name, &body);
							}
						}
					}
				}
			}
		}
		"copied_album_to_clipboard" => {
			let ann_text = if let Some(data) = val.get("data") {
				let msg_ids: Vec<String> = data
					.get("message_ids")
					.and_then(|ids| ids.as_array())
					.map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
					.unwrap_or_default();

				let mut s = state.borrow_mut();
				let mut is_voice = false;
				for id in &msg_ids {
					s.pending_copy_announcements.remove(id);
					if let Some(m) = s.messages.iter().find(|msg| msg.id == *id)
						&& (m.is_audio
							|| m.media_type == "voice"
							|| m.media_type == "audio"
							|| m.text.starts_with("[Voice Message]")
							|| m.text.starts_with("[Audio]"))
					{
						is_voice = true;
					}
				}
				if is_voice {
					"Copied voice message to clipboard.".to_string()
				} else if msg_ids.len() <= 1 {
					"Copied file to clipboard.".to_string()
				} else {
					format!("Copied {} files to clipboard.", msg_ids.len())
				}
			} else {
				"Copied file to clipboard.".to_string()
			};

			crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &ann_text);
		}
		"media_downloaded" => {
			if let Some(data) = val.get("data") {
				let msg_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("").to_string();
				let path = data.get("path").and_then(|p| p.as_str()).unwrap_or("").to_string();
				let filename = data.get("filename").and_then(|f| f.as_str()).unwrap_or("").to_string();
				let clean_filename = ChatMessage::sanitize_filename(&filename);
				crate::ui::thumbnails::forget(&msg_id);
				parts.conversations_parts.messages_list.refresh(false, None);

				let mut s = state.borrow_mut();
				s.downloads_in_progress.remove(&msg_id);
				update_loaded_message(&mut s, &msg_id, |m| {
					m.media_path = path;
					if !clean_filename.is_empty() {
						m.media_filename = clean_filename;
					}
				});

				let download_had_focus = parts.conversations_parts.action_download_btn.has_focus();
				parts.conversations_parts.conversation_panel.freeze();
				if update_action_buttons(&s, &parts.conversations_parts) {
					parts.conversations_parts.action_panel.layout();
				}
				parts.conversations_parts.conversation_panel.thaw();

				if download_had_focus && !parts.conversations_parts.action_download_btn.is_shown() {
					if parts.conversations_parts.action_open_btn.is_shown() {
						parts.conversations_parts.action_open_btn.set_focus();
					} else if parts.conversations_parts.action_save_as_btn.is_shown() {
						parts.conversations_parts.action_save_as_btn.set_focus();
					}
				}

				let is_copy_pending = s.pending_copy_announcements.contains(&msg_id);
				let media_type = data.get("mediaType").and_then(|m| m.as_str()).unwrap_or("");
				let auto = data.get("auto").and_then(|a| a.as_bool()).unwrap_or(false);
				if media_type != "voice" && !auto && !is_copy_pending {
					let announce_name = if filename.is_empty() { "File".to_string() } else { filename };
					crate::platform::announce::announce(
						parts.conversations_parts.messages_announcer,
						&format!("{} downloaded", announce_name),
					);
				}
			}
		}
		"media_download_progress" => {
			if let Some(data) = val.get("data") {
				let msg_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("").to_string();
				let percent = data.get("percent").and_then(|p| p.as_i64()).unwrap_or(0);
				let auto = data.get("auto").and_then(|a| a.as_bool()).unwrap_or(false);
				let mut s = state.borrow_mut();
				let prev = s.downloads_in_progress.insert(msg_id.clone(), percent);
				parts.conversations_parts.conversation_panel.freeze();
				if update_action_buttons(&s, &parts.conversations_parts) {
					parts.conversations_parts.action_panel.layout();
				}
				parts.conversations_parts.conversation_panel.thaw();
				for milestone in [25, 50, 75] {
					if !auto && prev.unwrap_or(0) < milestone && percent >= milestone {
						crate::platform::announce::announce(
							parts.conversations_parts.messages_announcer,
							&format!("Download {}%", percent),
						);
						break;
					}
				}
			}
		}
		"media_download_cancelled" | "media_download_failed" => {
			if let Some(data) = val.get("data") {
				let msg_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("").to_string();
				let mut s = state.borrow_mut();
				s.downloads_in_progress.remove(&msg_id);
				parts.conversations_parts.conversation_panel.freeze();
				if update_action_buttons(&s, &parts.conversations_parts) {
					parts.conversations_parts.action_panel.layout();
				}
				parts.conversations_parts.conversation_panel.thaw();
				let quiet = data.get("auto").and_then(|a| a.as_bool()).unwrap_or(false)
					&& s.playing_msg_id.as_deref() != Some(msg_id.as_str());
				if !quiet {
					let announcement = if t == "media_download_cancelled" {
						"Download cancelled".to_string()
					} else {
						let err = data.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error");
						format!("Download failed: {}", err)
					};
					crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &announcement);
					parts.conversations_parts.play_voice_btn.set_label("Play voice message");
				}
			}
		}
		"media_saved" => {
			if let Some(data) = val.get("data") {
				let path = data.get("path").and_then(|p| p.as_str()).unwrap_or("");
				crate::platform::announce::announce(
					parts.conversations_parts.messages_announcer,
					&format!("File saved to {}", path),
				);
			}
		}
		"play_audio" => {
			if let Some(data) = val.get("data")
				&& let Some(path) = data.get("path").and_then(|p| p.as_str())
			{
				let msg_id =
					data.get("message_id").or_else(|| data.get("id")).and_then(|i| i.as_str()).map(|s| s.to_string());
				eprintln!("[LOG UI] Playing voice note: {} from path: {}", msg_id.as_deref().unwrap_or(""), path);
				let mut s = state.borrow_mut();
				if let Some(ref id) = msg_id
					&& !id.is_empty()
				{
					s.playing_msg_id = Some(id.clone());
				}
				let start_paused = s.is_audio_paused;
				s.audio_elapsed_secs = 0.0;
				s.audio_start_instant = None;
				s.audio_path = Some(path.to_string());
				if let Some(ref sink) = s.audio_sink {
					sink.stop();
				}
				s.audio_sink = None;

				let source = crate::audio::file::open(path, std::time::Duration::ZERO);
				s.audio_total_secs =
					source.as_ref().and_then(|src| src.total_duration()).map_or(0.0, |d| d.as_secs_f32());

				if s.audio_total_secs == 0.0 {
					let playing_id = s.playing_msg_id.clone();
					if let Some(ref pid) = playing_id
						&& let Some(msg) = s.messages.iter().find(|m| &m.id == pid)
					{
						let text = &msg.text;
						if let Some(open) = text.find('(')
							&& let Some(close) = text[open..].find(')')
						{
							let inner = &text[open + 1..open + close];
							let parts_dur: Vec<&str> = inner.split(':').collect();
							if parts_dur.len() == 2
								&& let (Ok(m), Ok(ss)) = (parts_dur[0].parse::<f32>(), parts_dur[1].parse::<f32>())
							{
								s.audio_total_secs = m * 60.0 + ss;
							}
						}
					}
				}

				eprintln!("[LOG UI] Audio total duration: {}s", s.audio_total_secs);

				ensure_audio_stream(&mut s);
				let mut played = false;
				if let Some(ref stream_handle) = s.audio_stream {
					if let Some(source) = source {
						let sink = Player::connect_new(stream_handle.mixer());
						sink.append(source.wsola(s.audio_speed));
						if start_paused {
							sink.pause();
						} else {
							sink.play();
						}
						s.audio_sink = Some(sink);
						played = true;
						s.audio_start_instant = if start_paused { None } else { Some(std::time::Instant::now()) };
						parts.conversations_parts.play_voice_btn.set_label(if start_paused {
							"Play voice message"
						} else {
							"Pause voice message"
						});
					}
				}
				if !played {
					// Every format WhatsApp actually sends is covered above
					// (Ogg Opus by libopus, everything else by Media
					// Foundation), so landing here means the output device
					// couldn't be opened or the file is truncated/corrupt.
					// Reset back to "not playing" rather than leaving the
					// button stuck on "Pause" for audio that never started.
					eprintln!("[LOG UI] No decoder could play {}", path);
					s.playing_msg_id = None;
					s.audio_path = None;
					s.audio_start_instant = None;
					s.audio_elapsed_secs = 0.0;
					s.is_audio_paused = false;
					drop(s);
					parts.conversations_parts.play_voice_btn.set_label("Play voice message");
					parts.conversations_parts.audio_slider.show(false);
					parts.conversations_parts.audio_speed_btn.show(false);
					parts.conversations_parts.audio_progress_label.show(false);
					parts.conversations_parts.conversation_panel.layout();
					crate::platform::announce::announce(
						parts.conversations_parts.messages_announcer,
						"Could not play voice message",
					);
					return true;
				}
				s.is_audio_paused = start_paused;
				parts.conversations_parts.audio_slider.show(true);
				parts.conversations_parts.audio_speed_btn.show(true);
				parts.conversations_parts.audio_speed_btn.set_label(&format!("{:.1}x", s.audio_speed));
				parts.conversations_parts.audio_progress_label.show(false);
				let total = s.audio_total_secs;
				let max_sec = (total as i32).max(1);
				parts.conversations_parts.audio_slider.set_range(0, max_sec);
				parts.conversations_parts.audio_slider.set_value(0);
				let fmt_dur = |v: f32| {
					if v > 0.0 { format!("{}:{:02}", v as u32 / 60, v as u32 % 60) } else { "?:??".to_string() }
				};
				parts.conversations_parts.audio_progress_label.set_label(&format!("0:00 / {}", fmt_dur(total)));
				parts.conversations_parts.conversation_panel.layout();
			}
		}
		"attachment_uploading" => {
			// A gauge instead of spoken updates: NVDA users choose whether progress
			// is beeped, spoken or ignored, and per-percent announcements drowned
			// out everything else.
			if let Some(data) = val.get("data") {
				let percent = data.get("percent").and_then(|p| p.as_i64()).unwrap_or(0).clamp(0, 100) as i32;
				let gauge = &parts.conversations_parts.upload_gauge;
				if !gauge.is_shown() {
					gauge.show(true);
					parts.conversations_parts.conversation_panel.layout();
				}
				gauge.set_value(percent);
				if let Some(id) = data.get("id").and_then(|i| i.as_str()) {
					let mut s = state.borrow_mut();
					s.uploads_in_progress.insert(id.to_string(), percent as i64);
					refresh_action_buttons_now(&s, parts);
				}
			}
		}
		"attachment_sent" | "attachment_upload_ended" => {
			parts.attachment_dialog.attachment_info_label.set_label("");
			if let Some(id) = val.get("data").and_then(|d| d.get("id")).and_then(|i| i.as_str()) {
				state.borrow_mut().uploads_in_progress.remove(id);
			}
			let gauge = &parts.conversations_parts.upload_gauge;
			if gauge.is_shown() {
				gauge.show(false);
				parts.conversations_parts.conversation_panel.layout();
			}
		}
		_ => return false,
	}
	true
}

fn refresh_action_buttons_now(s: &AppState, parts: &WindowParts) {
	// Also shows or hides the whole action row, which the Tab order needs.
	refresh_voice_action_button(s, &parts.conversations_parts);
	parts.conversations_parts.conversation_panel.layout();
}
