use super::*;

static FIRST_CONNECT_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

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
				let label = crate::connected_status_text(&state.borrow());
				parts.frame.set_status_text(&label, 0);
			}

			let first_show = !state.borrow().main_ui_shown;
			// Switching accounts shows the window again, but maximizing and the
			// update check only belong to the app's start.
			let app_start = first_show && !FIRST_CONNECT_DONE.swap(true, std::sync::atomic::Ordering::SeqCst);
			if first_show {
				parts.pairing_dialog.show(false);
				parts.qrcode_dialog.show(false);
				parts.login_panel.show(false);
				parts.nav_panel.show(true);
				parts.content_panel.show(true);
				parts.main_panel.layout();
				// Started by Windows at sign-in: stay out of the way. Nothing is
				// shown or focused; the tray icon (or, without one, a minimized
				// taskbar button) is the way in.
				let background = app_start && crate::platform::autostart::started_in_background();
				let in_tray = background && state.borrow().settings.show_tray_icon;
				if app_start && state.borrow().settings.start_maximized {
					if in_tray {
						crate::platform::tray::maximize_on_restore();
					} else {
						parts.frame.maximize(true);
					}
				}
				state.borrow_mut().main_ui_shown = true;
				if background {
					if in_tray {
						crate::platform::tray::hide_main_window(&parts.frame);
					} else {
						parts.frame.iconize(true);
						parts.frame.show(true);
					}
					crate::ui_commands::misc::window_hidden(state, parts);
				} else {
					parts.frame.show(true);
					ship_shape::ui::bring_to_front_after_update(&parts.frame);
					parts.conversations_parts.search_field.set_focus();
				}
				let (check_updates, channel, ui_tx) = {
					let s = state.borrow();
					(s.settings.check_for_updates_on_startup, s.settings.update_channel.clone(), s.ui_tx.clone())
				};
				if check_updates && app_start {
					if background {
						state.borrow_mut().pending_update_check = true;
					} else {
						crate::platform::update_check::run(&parts.frame, &ui_tx, &channel, true);
					}
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
			parts.frame.set_status_text(crate::i18n::tr("Status: Not logged in"), 0);
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
			crate::ui::window::switch_login_view(parts, crate::ui::window::LoginView::Phone);
			parts.login_panel.show(true);
			parts.login_panel.set_focus();
			state.borrow_mut().main_ui_shown = false;
			parts.tray_icon.remove_icon();

			if let Some(reason) = val.get("data").and_then(|d| d.get("reason")).and_then(|r| r.as_str()) {
				let reason_text = crate::i18n::translate_backend_message(reason);
				let msg = crate::i18n::tr_fmt("whatSnap was logged out of WhatsApp: {}", &[&reason_text]);
				MessageDialog::builder(&parts.frame, &msg, crate::i18n::tr("Logged Out"))
					.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconWarning)
					.build()
					.show_modal();
			}
		}
		"pairing_code" => {
			if let Some(code) = val.get("data").and_then(|d| d.get("code")).and_then(|c| c.as_str()) {
				parts.pairing_code_display.set_value(code);
				println!("{{'data': {{'qrcode': {{'pairingCode': '{}'}}}}}}", code);
				crate::ui::window::switch_login_view(parts, crate::ui::window::LoginView::Pairing);
				unsafe {
					let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
						windows::Win32::UI::WindowsAndMessaging::MB_OK,
					);
				}
				let msg = crate::i18n::tr_fmt("Pairing code updated: {}", &[&code]);
				crate::platform::announce::announce(parts.pairing_announcer, &msg);
			}
		}
		"qr" => {
			if state.borrow().is_pairing_active {
				return true;
			}
			parts.qrcode_instructions.set_label(crate::i18n::tr("Scan this QR Code with WhatsApp:"));
			parts.qrcode_reload_btn.show(false);
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
					}
				}
				// Each new code only swaps the picture: switching views again
				// would re-centre the window and pull focus every 20 seconds.
				if !parts.qr_panel.is_shown() {
					crate::ui::window::switch_login_view(parts, crate::ui::window::LoginView::Qr);
				} else {
					parts.login_panel.layout();
				}
				unsafe {
					let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
						windows::Win32::UI::WindowsAndMessaging::MB_OK,
					);
				}
				crate::platform::announce::announce(parts.qrcode_announcer, crate::i18n::tr("QR code updated"));
			}
		}
		"qr_timeout" => {
			if state.borrow().is_pairing_active || !parts.qr_panel.is_shown() {
				return true;
			}
			parts.qrcode_instructions.set_label(crate::i18n::tr("QR Code expired. Click Reload to get a new code:"));
			parts.qrcode_reload_btn.show(true);
			parts.qrcode_reload_btn.set_focus();
			parts.login_panel.layout();
			crate::platform::announce::announce(
				parts.qrcode_announcer,
				crate::i18n::tr("QR code expired. Click Reload QR Code to try again."),
			);
		}
		"error" => {
			if let Some(msg) = val.get("data").and_then(|d| d.get("message")).and_then(|m| m.as_str()) {
				parts.conversations_parts.play_voice_btn.set_label(crate::i18n::tr("Play voice message"));
				let translated = crate::i18n::translate_backend_message(msg);
				// This one arrives on its own, so whatSnap is brought forward
				// for it, and the keyboard goes back to the window afterwards.
				let frame = windows::Win32::Foundation::HWND(parts.frame.get_handle());
				crate::platform::focus::show_modal_and_refocus(frame, true, || {
					MessageDialog::builder(&parts.frame, &translated, crate::i18n::tr("Error"))
						.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
						.build()
						.show_modal()
				});
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
						parts.frame.set_status_text(&crate::connected_status_text(&s), 0);
					}
					s.conn_status_text = crate::i18n::tr("Status: Connected").to_string();
					let speak = s.conn_outage_announced || s.conn_manual_reconnect;
					s.conn_outage_announced = false;
					s.conn_manual_reconnect = false;
					speak.then(|| crate::i18n::tr("Connected to WhatsApp.").to_string())
				}
				"connecting" => {
					let label = if message.is_empty() {
						crate::i18n::tr("Status: Connecting...").to_string()
					} else {
						let msg_tr = crate::i18n::translate_backend_message(message);
						crate::i18n::tr_fmt("Status: {} Reconnecting...", &[&msg_tr])
					};
					parts.frame.set_status_text(&label, 0);
					s.conn_status_text = label;
					None
				}
				"offline" => {
					let reason = if message.is_empty() {
						crate::i18n::tr("Can't connect to WhatsApp.").to_string()
					} else {
						crate::i18n::translate_backend_message(message)
					};
					let label = match retry_in {
						Some(secs @ 1..=119) => {
							crate::i18n::tr_fmt("Status: Offline. {} Retrying in {} seconds.", &[&reason, &secs])
						}
						Some(secs @ 120..=7199) => {
							let mins = secs.div_ceil(60);
							crate::i18n::tr_fmt("Status: Offline. {} Retrying in {} minutes.", &[&reason, &mins])
						}
						Some(secs @ 7200..) => {
							let hours = secs.div_ceil(3600);
							crate::i18n::tr_fmt("Status: Offline. {} Retrying in {} hours.", &[&reason, &hours])
						}
						_ => crate::i18n::tr_fmt("Status: Offline. {} Retrying automatically.", &[&reason]),
					};
					parts.frame.set_status_text(&label, 0);
					s.conn_status_text = label;
					let speak = !s.conn_outage_announced || s.conn_manual_reconnect;
					s.conn_outage_announced = true;
					s.conn_manual_reconnect = false;
					speak.then(|| crate::i18n::tr_fmt("{} whatSnap will keep trying to reconnect.", &[&reason]))
				}
				"stopped" => {
					let msg_tr = crate::i18n::translate_backend_message(message);
					let label = format!("Status: {msg_tr}");
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
				{
					let mut s = state.borrow_mut();
					let label = if s.recent_sync_active && s.recent_sync_percent >= 0 {
						let pct = s.recent_sync_percent;
						crate::i18n::tr_fmt(
							"Status: Connected (Syncing older messages, {}% complete: {} messages, {} chats so far)",
							&[&pct, &msgs_done, &chats_done],
						)
					} else {
						crate::i18n::tr_fmt(
							"Status: Connected (Syncing history... {} messages, {} chats so far)",
							&[&msgs_done, &chats_done],
						)
					};
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
		"history_sync_progress" | "offline_sync" => {
			let Some(data) = val.get("data") else { return true };
			let active = data.get("active").and_then(|v| v.as_bool()).unwrap_or(false);
			let percent = data.get("percent").and_then(|v| v.as_i64()).unwrap_or(-1);
			let mut s = state.borrow_mut();
			if t == "offline_sync" {
				s.offline_sync_percent = active.then_some(percent.clamp(0, 100));
			} else {
				s.recent_sync_active = active;
				s.recent_sync_percent = percent.min(100);
				s.recent_sync_paused = data.get("paused").and_then(|v| v.as_bool()).unwrap_or(false);
			}
			// A detailed "N messages, N chats" line that's still fresh keeps
			// showing; the next one carries the new percent.
			let detailed_fresh =
				s.history_sync_last_update.map(|t| t.elapsed() < std::time::Duration::from_secs(3)).unwrap_or(false);
			if s.conn_online && !detailed_fresh {
				parts.frame.set_status_text(&crate::connected_status_text(&s), 0);
			}
		}
		"history_access" => {
			let none = val.get("data").and_then(|d| d.get("none")).and_then(|v| v.as_bool()).unwrap_or(false);
			let msg = if none {
				crate::i18n::tr(
					"Your phone didn't share any message history with whatSnap. New messages will still arrive.",
				)
			} else {
				crate::i18n::tr(
					"Your phone shared only your recent messages with whatSnap, not your full chat history. Older messages load from your phone when you scroll to the top of a chat.",
				)
			};
			MessageDialog::builder(&parts.frame, msg, crate::i18n::tr("Chat history"))
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
					crate::ui_commands::chat_list::show_locked_chats(state, parts);
					crate::ui_commands::chat_list::offer_chat_lock_hello(state, parts);
				} else {
					state.borrow_mut().pending_locked_chat = None;
					let err = data.get("error").and_then(|e| e.as_str()).unwrap_or("");
					let (title, message) = if err == "no secret code configured" {
						(
							crate::i18n::tr("Secret Code Required"),
							crate::i18n::tr(
								"No Secret Code configured.\n\nTo lock or unlock chats on linked devices, you must first create a Secret Code in WhatsApp on your primary phone:\n\n1. Open WhatsApp on your phone\n2. Go to Settings > Privacy > Chat Lock\n3. Tap 'Secret Code' and set a code.\n\nOnce set, it will sync to this device automatically.",
							),
						)
					} else {
						(
							crate::i18n::tr("Unlock Failed"),
							crate::i18n::tr("Incorrect Secret Code.\n\nPlease check your secret code and try again."),
						)
					};
					let alert = MessageDialog::builder(&parts.frame, message, title)
						.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconWarning)
						.build();
					alert.show_modal();
				}
			}
		}
		"server_flags" => {
			if let Some(data) = val.get("data") {
				let mut s = state.borrow_mut();
				s.channel_pinning = data.get("channelPinning").and_then(|v| v.as_bool()).unwrap_or(false);
				s.scheduled_messages_enabled = data.get("scheduledMessages").and_then(|v| v.as_bool()).unwrap_or(true);
				crate::ui::window::refresh_menu_bar(&s, &parts.frame);
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
		"chat_tone" => {
			let Some(data) = val.get("data") else { return true };
			let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
			let tone = data.get("tone").and_then(whatsnap::synced_tone);
			if let Some(tone) = tone.filter(|_| !jid.is_empty()) {
				let mut s = state.borrow_mut();
				if s.settings.set_chat_tone(jid, tone) {
					s.settings.save();
				}
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
