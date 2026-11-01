use super::*;

pub(super) fn handle(t: &str, val: &serde_json::Value, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) -> bool {
	match t {
		"connected" => {
			if let Some(jid) = val.get("data").and_then(|d| d.get("jid")).and_then(|j| j.as_str()) {
				crate::ui::owner_draw::set_own_jid(jid);
			}
			// A reconnect (keepalive blip, brief network drop) also
			// fires this event, not just the first connect - don't
			// clobber an in-progress "Syncing history..." status
			// with plain "Connected" just because the socket
			// happened to bounce mid-sync.
			let mid_sync = state
				.borrow()
				.history_sync_last_update
				.map(|t| t.elapsed() < std::time::Duration::from_secs(5))
				.unwrap_or(false);
			if !mid_sync {
				parts.frame.set_status_text("Status: Connected", 0);
			}

			let first_show = !state.borrow().main_ui_shown;
			if first_show {
				parts.pairing_dialog.show(false);
				parts.qrcode_dialog.show(false);
				parts.login_panel.show(false);
				parts.nav_panel.show(true);
				parts.content_panel.show(true);
				parts.main_panel.layout();
				if state.borrow().settings.start_maximized {
					parts.frame.maximize(true);
				}
				parts.frame.show(true);
				ship_shape::ui::bring_to_front_after_update(&parts.frame);
				parts.conversations_parts.search_field.set_focus();
				state.borrow_mut().main_ui_shown = true;
				let (check_updates, channel, ui_tx) = {
					let s = state.borrow();
					(s.settings.check_for_updates_on_startup, s.settings.update_channel.clone(), s.ui_tx.clone())
				};
				if check_updates {
					crate::platform::update_check::run(&parts.frame, &ui_tx, &channel, true);
				}

				// Now that we're logged in, install the tray icon if the
				// user wants it - it's gated on login so an unpaired app
				// never parks an icon in the tray (window.rs).
				install_tray_icon(&state.borrow(), parts);
			}

			let payload = serde_json::json!({
				"action": "get_chats"
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
			let payload = serde_json::json!({ "action": "get_chat_lists" });
			let _ = state.borrow().ws_tx.send(payload.to_string());
			let payload = serde_json::json!({ "action": "get_joinable_calls" });
			let _ = state.borrow().ws_tx.send(payload.to_string());
			crate::ui_commands::calls::send_call_options(&state.borrow());

			{
				let mut s = state.borrow_mut();
				let in_use = crate::ui_commands::misc::desired_presence(&s);
				s.presence_sent = None;
				crate::ui_commands::misc::send_presence(&mut s, in_use);
			}
		}
		"needs_login" => {
			parts.frame.set_status_text("Status: Not logged in", 0);
			parts.nav_panel.show(false);
			parts.content_panel.show(false);
			parts.main_panel.layout();
			// The login panel is a Dialog owned by the frame (see
			// set_dialog_owner). Windows won't activate an owned window
			// whose owner is hidden, so with the frame no longer shown at
			// startup we reveal it here first - otherwise the login dialog
			// shows but never takes focus and NVDA lands on nothing. A
			// visible owner is what lets the owned dialog activate.
			// Keep the frame hidden so the login dialog is the only top-level
			// window while logged out. That way alt-tabbing to whatSnap activates
			// the dialog itself - a screen reader announces the window and its
			// focused field in one step - instead of landing on an empty frame
			// behind it and needing focus force-shifted into the dialog.
			parts.frame.show(false);
			parts.pairing_dialog.show(false);
			parts.qrcode_dialog.show(false);
			parts.login_panel.show(true);
			parts.login_panel.set_focus();
			state.borrow_mut().main_ui_shown = false;
			parts.tray_icon.remove_icon();

			if let Some(reason) = val.get("data").and_then(|d| d.get("reason")).and_then(|r| r.as_str()) {
				MessageDialog::builder(
					&parts.frame,
					&format!("whatSnap was logged out of WhatsApp: {reason}"),
					"Logged Out",
				)
				.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconWarning)
				.build()
				.show_modal();
			}
		}
		"pairing_code" => {
			if let Some(code) = val.get("data").and_then(|d| d.get("code")).and_then(|c| c.as_str()) {
				parts.pairing_code_display.set_value(code);
				println!("{{'data': {{'qrcode': {{'pairingCode': '{}'}}}}}}", code);
				parts.login_panel.show(false);
				parts.pairing_dialog.show(true);
				parts.qrcode_dialog.show(false);
				parts.pairing_code_display.select_all();
				parts.pairing_code_display.set_focus();
				unsafe {
					let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
						windows::Win32::UI::WindowsAndMessaging::MB_OK,
					);
				}
				crate::platform::announce::announce(
					parts.pairing_announcer,
					&format!("Pairing code updated: {}", code),
				);
			}
		}
		"qr" => {
			if state.borrow().is_pairing_active {
				return true;
			}
			if let Some(code) = val.get("data").and_then(|d| d.get("code")).and_then(|c| c.as_str()) {
				if let Ok(qr) = qrcode::QrCode::new(code) {
					let (width, height, data) = qr_rgba(&qr);
					let png_bytes = crate::media::png::encode_rgba(width, height, &data);
					println!(
						"{{'data': {{'qrcode': {{'base64': 'data:image/png;base64,{}'}}}}}}",
						crate::util::base64(&png_bytes)
					);
					if let Some(bitmap) = wxdragon::bitmap::Bitmap::from_rgba(&data, width, height) {
						parts.qrcode_display.set_min_size(Size::new(width as i32, height as i32));
						parts.qrcode_display.set_bitmap(&bitmap);
						parts.qrcode_dialog.fit();
						parts.qrcode_dialog.layout();
					}
				}
				parts.login_panel.show(false);
				parts.pairing_dialog.show(false);
				parts.qrcode_dialog.show(true);
				parts.qrcode_cancel_btn.set_focus();
				unsafe {
					let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
						windows::Win32::UI::WindowsAndMessaging::MB_OK,
					);
				}
				crate::platform::announce::announce(parts.qrcode_announcer, "QR code updated");
			}
		}
		"error" => {
			if let Some(msg) = val.get("data").and_then(|d| d.get("message")).and_then(|m| m.as_str()) {
				parts.conversations_parts.play_voice_btn.set_label("Play voice message");
				MessageDialog::builder(&parts.frame, msg, "Error")
					.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
					.build()
					.show_modal();
			}
		}
		"connection_state" => {
			let data = val.get("data");
			let conn_state = data.and_then(|d| d.get("state")).and_then(|v| v.as_str()).unwrap_or("");
			let message = data.and_then(|d| d.get("message")).and_then(|v| v.as_str()).unwrap_or("");
			let retry_in = data.and_then(|d| d.get("retry_in")).and_then(|v| v.as_u64());

			let mut s = state.borrow_mut();
			s.conn_online = conn_state == "online";
			// Only speak on the edges of an outage (and for a manual
			// Reconnect Now): a quick keepalive blip that reconnects on
			// the first try says nothing, and retries during a long
			// outage don't repeat the same announcement every time.
			let announcement = match conn_state {
				"online" => {
					let mid_sync = s
						.history_sync_last_update
						.map(|t| t.elapsed() < std::time::Duration::from_secs(5))
						.unwrap_or(false);
					if !mid_sync {
						parts.frame.set_status_text("Status: Connected", 0);
					}
					s.conn_status_text = "Status: Connected".to_string();
					let speak = s.conn_outage_announced || s.conn_manual_reconnect;
					s.conn_outage_announced = false;
					s.conn_manual_reconnect = false;
					speak.then(|| "Connected to WhatsApp.".to_string())
				}
				"connecting" => {
					let label = if message.is_empty() {
						"Status: Connecting...".to_string()
					} else {
						format!("Status: {message} Reconnecting...")
					};
					parts.frame.set_status_text(&label, 0);
					s.conn_status_text = label;
					None
				}
				"offline" => {
					let reason = if message.is_empty() { "Can't connect to WhatsApp." } else { message };
					let label = match retry_in {
						Some(secs @ 1..=119) => {
							format!("Status: Offline. {reason} Retrying in {secs} seconds.")
						}
						Some(secs @ 120..=7199) => {
							format!("Status: Offline. {reason} Retrying in {} minutes.", secs.div_ceil(60))
						}
						Some(secs @ 7200..) => {
							format!("Status: Offline. {reason} Retrying in {} hours.", secs.div_ceil(3600))
						}
						_ => format!("Status: Offline. {reason} Retrying automatically."),
					};
					parts.frame.set_status_text(&label, 0);
					s.conn_status_text = label;
					let speak = !s.conn_outage_announced || s.conn_manual_reconnect;
					s.conn_outage_announced = true;
					s.conn_manual_reconnect = false;
					speak.then(|| format!("{reason} whatSnap will keep trying to reconnect."))
				}
				"stopped" => {
					let label = format!("Status: {message}");
					parts.frame.set_status_text(&label, 0);
					s.conn_status_text = label;
					s.conn_outage_announced = true;
					s.conn_manual_reconnect = false;
					None
				}
				_ => None,
			};
			drop(s);
			if let Some(text) = announcement {
				crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &text);
			}
		}
		"history_sync" => {
			if let Some(data) = val.get("data") {
				let chats_done = data.get("chatsDone").and_then(|v| v.as_i64()).unwrap_or(0);
				let msgs_done = data.get("msgsDone").and_then(|v| v.as_i64()).unwrap_or(0);
				let label =
					format!("Status: Connected (Syncing history... {msgs_done} messages, {chats_done} chats so far)");
				{
					let mut s = state.borrow_mut();
					// Batches already received keep being saved after the
					// connection drops; don't paint "Connected" over that.
					if s.conn_online {
						parts.frame.set_status_text(&label, 0);
					}
					let now = std::time::Instant::now();
					s.history_sync_last_update = Some(now);
					s.history_sync_armed_until = Some(now + std::time::Duration::from_secs(60));
				}
			}
		}
		"history_access" => {
			let none = val.get("data").and_then(|d| d.get("none")).and_then(|v| v.as_bool()).unwrap_or(false);
			let msg = if none {
				"Your phone didn't share any message history with whatSnap. New messages will still arrive."
			} else {
				"Your phone shared only your recent messages with whatSnap, not your full chat history. Older messages load from your phone when you scroll to the top of a chat."
			};
			MessageDialog::builder(&parts.frame, msg, "Chat history")
				.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
				.build()
				.show_modal();
		}
		"history_chats_synced" => {
			let jids: Vec<&str> = val
				.get("data")
				.and_then(|d| d.get("jids"))
				.and_then(|j| j.as_array())
				.map(|a| a.iter().filter_map(|j| j.as_str()).collect())
				.unwrap_or_default();
			let mut s = state.borrow_mut();
			let Some(open_jid) = s.selected_chat.clone() else { return true };
			if !jids.contains(&open_jid.as_str()) {
				return true;
			}
			if s.phone_history_pending_jid.as_deref() == Some(open_jid.as_str()) {
				let payload = crate::ui_commands::chat_list::older_page_request(&s.messages, &open_jid);
				let _ = s.ws_tx.send(payload.to_string());
			} else if !s.history_loading && s.quiet_reload_jid.is_none() {
				s.no_more_messages = false;
				if whatsnap::real_message_count(&s.messages) == 0 && s.messages_at_latest {
					s.quiet_reload_jid = Some(open_jid.clone());
					let payload = serde_json::json!({
						"action": "get_messages",
						"data": { "jid": open_jid, "limit": crate::MESSAGE_PAGE_SIZE, "offset": 0 }
					});
					let _ = s.ws_tx.send(payload.to_string());
				}
			}
		}
		"chat_lock_verify_result" => {
			if let Some(data) = val.get("data") {
				let success = data.get("success").and_then(|s| s.as_bool()).unwrap_or(false);
				if success {
					let mut s = state.borrow_mut();
					s.chats_unlocked = true;
					s.search_query.clear();
					parts.conversations_parts.search_field.set_value("");
					apply_chat_filter(&mut s, parts);
					crate::platform::announce::announce(
						parts.conversations_parts.messages_announcer,
						"Locked chats shown.",
					);
				}
			}
		}
		"server_flags" => {
			if let Some(data) = val.get("data") {
				state.borrow_mut().channel_pinning =
					data.get("channelPinning").and_then(|v| v.as_bool()).unwrap_or(false);
			}
		}
		"synced_setting" => {
			let Some(data) = val.get("data") else { return true };
			let name = data.get("name").and_then(|n| n.as_str()).unwrap_or("");
			let value = data.get("value").cloned().unwrap_or_default();
			if name == "startAtLogin" {
				if let Some(on) = value.as_bool()
					&& crate::platform::autostart::is_enabled() != on
				{
					crate::platform::autostart::set_enabled(on);
				}
				return true;
			}
			let mut s = state.borrow_mut();
			let old_theme = s.settings.theme;
			if !s.settings.apply_synced(name, &value) {
				return true;
			}
			s.settings.save();
			let theme = s.settings.theme;
			crate::ui::spell_check::configure(s.settings.spell_check, &s.settings.spell_language);
			let settings = s.settings.clone();
			drop(s);
			if parts.settings_dialog.dialog.is_shown() {
				parts.settings_dialog.show_chat_settings(&settings);
				parts.settings_dialog.sounds_check.set_value(settings.sounds_enabled);
				parts.settings_dialog.notifications_check.set_value(settings.notifications_enabled);
			}
			if theme != old_theme {
				wxdragon::call_after(Box::new(move || crate::ui::theme::change_theme(theme)));
			}
		}
		"app_prefs" => {
			if let Some(data) = val.get("data") {
				state.borrow_mut().app_prefs = Some(data.clone());
				if parts.settings_dialog.dialog.is_shown() {
					parts.settings_dialog.show_app_prefs(Some(data));
				}
			}
		}
		_ => return false,
	}
	true
}
