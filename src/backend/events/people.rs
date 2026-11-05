use super::*;

pub(super) fn handle(t: &str, val: &serde_json::Value, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) -> bool {
	match t {
		"new_contact_status" => {
			if let Some(data) = val.get("data")
				&& let Some(status) = data.get("status").and_then(|s| s.as_str())
			{
				if status == "success" {
					let payload = serde_json::json!({
						"action": "get_chats"
					});
					let _ = state.borrow().ws_tx.send(payload.to_string());
					refresh_open_contact_info(state, parts);
				} else if status == "error" {
					let err_msg = data.get("error").and_then(|e| e.as_str()).unwrap_or("Unknown error");
					unsafe {
						let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
							windows::Win32::UI::WindowsAndMessaging::MB_OK,
						);
					}
					let owner = windows::Win32::Foundation::HWND(parts.new_contact_dialog.dialog.get_handle());
					crate::platform::focus::show_modal_and_refocus(owner, false, || {
						MessageDialog::builder(&parts.new_contact_dialog.dialog, err_msg, "Error")
							.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
							.build()
							.show_modal()
					});
				}
			}
		}
		"contacts" => {
			if let Ok(rows) = serde_json::from_value::<Vec<serde_json::Value>>(
				val.get("data").cloned().unwrap_or(serde_json::Value::Null),
			) {
				let mut contacts: Vec<Contact> = rows
					.iter()
					.filter_map(|row| {
						let jid = row.get("jid").and_then(|j| j.as_str())?.to_string();
						if jid.is_empty() {
							return None;
						}
						let name = row.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
						let phone = row.get("phone").and_then(|p| p.as_str()).unwrap_or("").to_string();
						Some(Contact { jid, name, phone })
					})
					.collect();
				contacts.sort_by(|a, b| {
					let ka = if a.name.is_empty() { &a.phone } else { &a.name };
					let kb = if b.name.is_empty() { &b.phone } else { &b.name };
					ka.to_lowercase().cmp(&kb.to_lowercase())
				});
				let mut s = state.borrow_mut();
				s.all_contacts = contacts;
				let new_conv_shown = parts.new_conv_dialog.dialog.is_shown();
				let new_group_shown = parts.new_group_dialog.dialog.is_shown();
				let add_members_shown = parts.add_members_dialog.dialog.is_shown();
				let attach_contact_shown = parts.attach_contact_dialog.dialog.is_shown();
				drop(s);
				if new_conv_shown {
					let query = parts.new_conv_dialog.search_field.get_value();
					apply_contact_picker_filter(&query, state, parts);
				}
				// Route a fresh `contacts` response through the same debounce
				// timer as keystrokes (rather than rebuilding the checklist
				// widget here immediately) - back-to-back rebuilds (this one
				// landing right after the dialog-open rebuild) race NVDA's
				// async accessibility queries and trip wx's "bad
				// wxCheckListBox index" assert. See the timer's own comment
				// in window.rs's bind_input_handlers.
				if new_group_shown {
					sync_new_group_contact_labels(state);
					parts.new_group_contacts_timer.stop();
					parts.new_group_contacts_timer.start(250, true);
				}
				if add_members_shown {
					sync_add_members_contact_labels(state);
					parts.add_members_contacts_timer.stop();
					parts.add_members_contacts_timer.start(250, true);
				}
				if attach_contact_shown {
					sync_attach_contact_labels(state);
					parts.attach_contact_timer.stop();
					parts.attach_contact_timer.start(250, true);
				}
			}
		}
		"create_group_status" | "new_group_status" => {
			if let Some(data) = val.get("data")
				&& let Some(status) = data.get("status").and_then(|s| s.as_str())
			{
				if status == "success" {
					let payload = serde_json::json!({
						"action": "get_chats"
					});
					let _ = state.borrow().ws_tx.send(payload.to_string());
				} else if status == "error" {
					parts.new_group_dialog.ok_btn.enable(true);
					let err_msg = data.get("error").and_then(|e| e.as_str()).unwrap_or("Unknown error");
					unsafe {
						let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
							windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_STYLE(0x00000030),
						);
					}
					let owner = windows::Win32::Foundation::HWND(parts.new_group_dialog.dialog.get_handle());
					crate::platform::focus::show_modal_and_refocus(owner, false, || {
						MessageDialog::builder(&parts.new_group_dialog.dialog, err_msg, "Error")
							.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
							.build()
							.show_modal()
					});
				}
			}
		}
		"join_requests"
		| "join_requests_answered"
		| "linkable_groups"
		| "community_groups_changed"
		| "community_created"
		| "community_deactivated" => {
			if let Some(data) = val.get("data") {
				crate::ui_commands::group_extras::on_event(state, parts, t, data);
			}
		}
		"group_info" => {
			if let Some(payload) = val.get("data") {
				let info = payload.get("Info").unwrap_or(payload);
				let viewer_is_admin = payload.get("ViewerIsAdmin").and_then(|v| v.as_bool()).unwrap_or(false);

				if let Some(group_jid) = info.get("JID").and_then(|j| j.get("User")).and_then(|u| u.as_str()) {
					let full_jid = format!("{}@g.us", group_jid);
					let mut s = state.borrow_mut();
					if viewer_is_admin {
						s.admin_group_jids.insert(full_jid);
					} else {
						s.admin_group_jids.remove(&full_jid);
					}
				}

				parts.group_data_dialog.add_member_btn.show(viewer_is_admin);
				parts.group_data_dialog.invite_link_btn.show(viewer_is_admin);
				parts.group_data_dialog.save_settings_btn.show(viewer_is_admin);
				parts.group_data_dialog.announce_chk.enable(viewer_is_admin);
				parts.group_data_dialog.locked_chk.enable(viewer_is_admin);
				parts.group_data_dialog.forwarded_chk.enable(viewer_is_admin);
				parts.group_data_dialog.admin_reports_chk.enable(viewer_is_admin);
				parts.group_data_dialog.history_chk.enable(viewer_is_admin);
				parts.group_data_dialog.reported_btn.show(viewer_is_admin);
				let flag = |k: &str| info.get(k).and_then(|t| t.as_bool()).unwrap_or(false);
				parts.group_data_dialog.forwarded_chk.set_value(flag("NoFrequentlyForwarded"));
				parts.group_data_dialog.admin_reports_chk.set_value(flag("AllowAdminReports"));
				parts.group_data_dialog.history_chk.set_value(flag("HasGroupHistory"));

				let my_tag = payload.get("MyLabel").and_then(|t| t.as_str()).unwrap_or("");
				if !parts.group_data_dialog.my_tag_field.has_focus() {
					parts.group_data_dialog.my_tag_field.set_value(my_tag);
				}

				if let Some(topic) = info.get("Topic").and_then(|t| t.as_str()) {
					parts.group_data_dialog.group_desc_field.set_value(topic);
				}
				if let Some(is_announce) = info.get("IsAnnounce").and_then(|t| t.as_bool()) {
					parts.group_data_dialog.announce_chk.set_value(is_announce);
				}
				if let Some(is_locked) = info.get("IsLocked").and_then(|t| t.as_bool()) {
					parts.group_data_dialog.locked_chk.set_value(is_locked);
				}
				let is_community = info.get("IsParent").and_then(|t| t.as_bool()).unwrap_or(false);
				let in_community = info.get("LinkedParentJID").and_then(|t| t.as_str()).is_some_and(|p| !p.is_empty());
				let gd = &parts.group_data_dialog;
				gd.join_requests_btn.show(viewer_is_admin && !is_community);
				gd.link_group_btn.show(viewer_is_admin && is_community);
				gd.unlink_group_btn.show(viewer_is_admin && is_community);
				gd.deactivate_btn.show(viewer_is_admin && is_community);
				gd.admins_add_chk.enable(viewer_is_admin);
				gd.approve_members_chk.enable(viewer_is_admin);
				gd.admins_add_chk.set_value(info.get("MemberAddMode").and_then(|m| m.as_str()) == Some("admin_add"));
				gd.approve_members_chk.set_value(flag("IsJoinApprovalRequired"));
				gd.members_panel.layout();
				gd.groups_panel.layout();
				gd.settings_panel.layout();
				parts.group_data_dialog.groups_list.clear();
				if is_community || in_community {
					let s = state.borrow();
					if let Some(ref jid) = s.selected_chat {
						let payload = serde_json::json!({
							"action": "get_community_subgroups",
							"data": { "jid": jid }
						});
						let _ = s.ws_tx.send(payload.to_string());
					}
				}

				let s = state.borrow();
				let mut member_strings = Vec::new();
				let mut member_jids = Vec::new();
				let mut member_info = Vec::new();
				if let Some(participants) = payload.get("Participants").and_then(|p| p.as_array()) {
					for p in participants {
						if let Some(jid) = p.get("JID").and_then(|j| j.as_str()) {
							// PhoneJID is the participant's phone-number JID
							// (backend-resolved from a @lid address when the
							// group hides phone numbers), or empty for someone
							// who hides their number from us - never format the
							// raw action JID as a number, a LID isn't one.
							let phone_jid = p.get("PhoneJID").and_then(|j| j.as_str()).unwrap_or("");
							let is_admin = p.get("IsAdmin").and_then(|a| a.as_bool()).unwrap_or(false);
							let is_super = p.get("IsSuperAdmin").and_then(|a| a.as_bool()).unwrap_or(false);

							let backend_name = p.get("Name").and_then(|n| n.as_str()).unwrap_or("");
							let tag = p.get("Label").and_then(|n| n.as_str()).unwrap_or("").trim();

							let phone = if phone_jid.is_empty() {
								String::new()
							} else {
								let raw_phone = phone_jid.split('@').next().unwrap_or(phone_jid);
								whatsnap::phone::format_international(raw_phone)
									.unwrap_or_else(|| raw_phone.to_string())
							};

							let local_name = if !backend_name.is_empty() {
								backend_name.to_string()
							} else if let Some(contact) = s.contacts.get(phone_jid).filter(|_| !phone_jid.is_empty()) {
								contact.clone()
							} else if !phone.is_empty() {
								phone.clone()
							} else {
								"Unknown member".to_string()
							};

							let mut role = "";
							if is_super {
								role = " (Super Admin)";
							} else if is_admin {
								role = " (Admin)";
							}

							let tagged = if tag.is_empty() {
								local_name.clone()
							} else {
								format!("{}, tag: {}", local_name, tag)
							};
							let display = if phone.is_empty() || local_name == phone {
								format!("{}{}", tagged, role)
							} else {
								format!("{}{} - {}", tagged, role, phone)
							};
							member_strings.push(display);
							member_jids.push(jid.to_string());
							member_info.push(crate::GroupMember {
								jid: jid.to_string(),
								phone_jid: phone_jid.to_string(),
								name: local_name.clone(),
								phone: phone.clone(),
								admin: is_admin || is_super,
								super_admin: is_super,
								me: p.get("IsMe").and_then(|v| v.as_bool()).unwrap_or(false),
								saved: p.get("Saved").and_then(|v| v.as_bool()).unwrap_or(false),
							});
						}
					}
				}
				drop(s);
				let mut s_mut = state.borrow_mut();
				s_mut.group_data_members = member_strings;
				s_mut.group_data_member_jids = member_jids;
				s_mut.group_data_member_info = member_info;

				let gd = &parts.group_data_dialog;
				let lists =
					InfoMediaLists { media: &gd.media_list, documents: &gd.documents_list, links: &gd.links_list };
				(s_mut.group_data_media, s_mut.group_data_documents, s_mut.group_data_links) =
					fill_info_media(&s_mut, lists, None);

				let query_members = parts.group_data_dialog.members_search.get_value().to_lowercase();
				parts.group_data_dialog.members_list.clear();
				let mut visible_jids = Vec::new();
				for (i, member) in s_mut.group_data_members.iter().enumerate() {
					if query_members.is_empty() || member.to_lowercase().contains(&query_members) {
						parts.group_data_dialog.members_list.append(member);
						visible_jids.push(s_mut.group_data_member_jids[i].clone());
					}
				}
				s_mut.group_data_visible_member_jids = visible_jids;
				drop(s_mut);
			}
		}
		"community_subgroups" => {
			if let Some(data) = val.get("data") {
				let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
				let s = state.borrow();
				let is_relevant = s.selected_chat.as_deref() == Some(jid) && parts.group_data_dialog.dialog.is_shown();
				drop(s);
				if is_relevant {
					let community = data.get("community").and_then(|c| c.as_str()).unwrap_or(jid).to_string();
					let community_name = data.get("communityName").and_then(|n| n.as_str()).unwrap_or("");
					parts.group_data_dialog.groups_label.set_label(&if community_name.is_empty() {
						"Groups in this community".to_string()
					} else {
						format!("Groups in {}", community_name)
					});
					let mut subgroup_jids = Vec::new();
					let mut subgroup_joined = Vec::new();
					let mut labels = Vec::new();
					if let Some(groups) = data.get("groups").and_then(|g| g.as_array()) {
						for g in groups {
							let flag = |key: &str| g.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
							let joined = flag("Joined");
							let label = whatsnap::community_group_label(
								g.get("Name").and_then(|n| n.as_str()).unwrap_or(""),
								flag("IsDefaultSubGroup"),
								flag("IsGeneralChat"),
								joined,
								g.get("ParticipantCount").and_then(|c| c.as_u64()).unwrap_or(0),
							);
							labels.push(label);
							subgroup_jids.push(g.get("JID").and_then(|j| j.as_str()).unwrap_or("").to_string());
							subgroup_joined.push(joined);
						}
					}
					let groups_list = &parts.group_data_dialog.groups_list;
					let previous_jid = groups_list
						.get_selection()
						.and_then(|i| state.borrow().group_data_subgroup_jids.get(i as usize).cloned());
					groups_list.set_items(&labels.iter().map(String::as_str).collect::<Vec<_>>());
					if let Some(idx) = previous_jid.and_then(|prev| subgroup_jids.iter().position(|j| *j == prev)) {
						groups_list.set_selection(idx as u32, true);
					}
					let mut s = state.borrow_mut();
					s.group_data_subgroup_jids = subgroup_jids;
					s.group_data_subgroup_joined = subgroup_joined;
					s.group_data_community = Some(community);
					drop(s);
					crate::ui_commands::contacts_groups::community_subgroup_selection_changed(state, parts);
				}
			}
		}
		"member_label" => {
			if let Some(data) = val.get("data") {
				let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
				let s = state.borrow();
				if data.get("mine").and_then(|m| m.as_bool()).unwrap_or(false) {
					let label = data.get("label").and_then(|l| l.as_str()).unwrap_or("");
					let text = if label.is_empty() {
						"Your tag was removed.".to_string()
					} else {
						format!("Your tag is now {}.", label)
					};
					crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &text);
				}
				if s.selected_chat.as_deref() == Some(jid) && parts.group_data_dialog.dialog.is_shown() {
					let payload = serde_json::json!({
						"action": "get_group_info",
						"data": { "jid": jid }
					});
					let _ = s.ws_tx.send(payload.to_string());
				}
			}
		}
		"group_updated" => {
			if let Some(jid) = val.get("data").and_then(|d| d.get("jid")).and_then(|j| j.as_str()) {
				let s = state.borrow();
				let is_relevant = s.selected_chat.as_deref() == Some(jid) && parts.group_data_dialog.dialog.is_shown();
				if is_relevant {
					let payload = serde_json::json!({
						"action": "get_group_info",
						"data": { "jid": jid }
					});
					let _ = s.ws_tx.send(payload.to_string());
				}
			}
		}
		"community_subgroup_joined" => {
			if let Some(jid) = val.get("data").and_then(|d| d.get("jid")).and_then(|j| j.as_str()) {
				{
					let s = state.borrow();
					if parts.group_data_dialog.dialog.is_shown()
						&& let Some(ref selected) = s.selected_chat
					{
						let payload = serde_json::json!({
							"action": "get_community_subgroups",
							"data": { "jid": selected }
						});
						let _ = s.ws_tx.send(payload.to_string());
					}
				}
				let name = state.borrow().contacts.get(jid).cloned().unwrap_or_else(|| jid.to_string());
				crate::platform::announce::announce(
					parts.conversations_parts.messages_announcer,
					&format!("Joined {}.", name),
				);
			}
		}
		"group_invite_info" => {
			if let Some(data) = val.get("data") {
				let message_id = data.get("messageId").and_then(|v| v.as_str()).unwrap_or("").to_string();
				let code = data.get("code").and_then(|v| v.as_str()).unwrap_or("").to_string();
				let name = data.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
				let topic = data.get("topic").and_then(|v| v.as_str()).unwrap_or("");
				let size = data.get("size").and_then(|v| v.as_i64()).unwrap_or(0);
				let display_name = if name.is_empty() { "This group".to_string() } else { name };
				let mut body = format!("{display_name}\n{size} member(s)");
				if !topic.is_empty() {
					body.push_str(&format!("\n\n{topic}"));
				}
				let dlg = MessageDialog::builder(&parts.frame, &body, "Group Invite")
					.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
					.build();
				if dlg.show_modal() == wxdragon::id::ID_YES {
					let command = if code.is_empty() {
						UiCommand::JoinGroupInvite(message_id)
					} else {
						UiCommand::JoinGroupLink(code)
					};
					let _ = state.borrow().ui_tx.send(command);
				}
			}
		}
		"channel_directory" => {
			let directory = state.borrow().channel_directory.clone();
			if let (Some(directory), Some(data)) = (directory, val.get("data")) {
				directory.apply(data);
			}
		}
		"channel_info" => {
			if let Some(data) = val.get("data") {
				crate::ui_commands::links::show_channel_info(state, parts, data);
			}
		}
		"channel_admin_invite" => {
			if let Some(data) = val.get("data") {
				crate::ui_commands::channel_admin::channel_admin_invite(state, parts, data);
			}
		}
		"channel_notice" => {
			if let Some(data) = val.get("data") {
				crate::ui_commands::channel_admin::channel_notice(parts, data);
			}
		}
		"channel_people" => {
			if let Some(data) = val.get("data") {
				crate::ui_commands::channel_admin::channel_people(parts, data);
			}
		}
		"channel_pending_invites" => {
			if let Some(data) = val.get("data") {
				crate::ui_commands::channel_admin::channel_pending_invites(state, parts, data);
			}
		}
		"similar_channels" => {
			let directory = state.borrow().channel_directory.clone();
			if let (Some(directory), Some(data)) = (directory, val.get("data")) {
				directory.apply(data);
			}
		}
		"channel_created" | "channel_updated" | "channel_deleted" | "channel_followers" => {
			if let Some(data) = val.get("data") {
				use crate::ui_commands::channel_admin as admin;
				match t {
					"channel_created" => admin::channel_created(state, parts, data),
					"channel_updated" => admin::channel_updated(state, parts, data),
					"channel_deleted" => admin::channel_deleted(state, parts, data),
					_ => admin::channel_followers(state, parts, data),
				}
			}
		}
		"channel_messages_updated" => {
			let jid = val.get("data").and_then(|d| d.get("jid")).and_then(|j| j.as_str()).unwrap_or("");
			let mut s = state.borrow_mut();
			if s.selected_chat.as_deref() == Some(jid) && s.quiet_reload_jid.is_none() && !s.history_loading {
				s.quiet_reload_jid = Some(jid.to_string());
				let payload = serde_json::json!({
					"action": "get_messages",
					"data": { "jid": jid, "limit": crate::MESSAGE_PAGE_SIZE, "offset": 0, "no_server_fetch": true }
				});
				let _ = s.ws_tx.send(payload.to_string());
			}
		}
		"channel_follow_changed" => {
			if let Some(data) = val.get("data") {
				let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
				let following = data.get("following").and_then(|f| f.as_bool()).unwrap_or(false);
				let directory = state.borrow().channel_directory.clone();
				if let Some(directory) = directory {
					directory.set_following(jid, following);
				}
				let name = state.borrow().contacts.get(jid).cloned().unwrap_or_else(|| "the channel".to_string());
				let message = if following {
					format!("Following {}. It's in the Channels tab, Alt+6.", name)
				} else {
					format!("Unfollowed {}.", name)
				};
				crate::platform::announce::announce(parts.conversations_parts.messages_announcer, &message);
			}
		}
		"group_invite_link" => {
			if let Some(data) = val.get("data") {
				let jid = data.get("jid").and_then(|v| v.as_str()).unwrap_or("").to_string();
				let link = data.get("link").and_then(|v| v.as_str()).unwrap_or("").to_string();
				if !link.is_empty() {
					crate::ui_commands::links::show_group_invite_link(state, parts, jid, link);
				}
			}
		}
		"group_invite_joined" => {
			let name = val
				.get("data")
				.and_then(|d| d.get("jid"))
				.and_then(|j| j.as_str())
				.and_then(|jid| state.borrow().contacts.get(jid).cloned())
				.unwrap_or_else(|| "the group".to_string());
			crate::platform::announce::announce(
				parts.conversations_parts.messages_announcer,
				&format!("Joined {}.", name),
			);
		}
		"avatar_previews" => {
			if let Some(data) = val.get("data") {
				crate::ui::avatars::on_previews(data);
			}
		}
		"avatar" => {
			let Some(data) = val.get("data") else { return true };
			let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
			let path = data.get("path").and_then(|p| p.as_str()).unwrap_or("");
			let s = state.borrow();
			if s.info_chat.as_deref() == Some(jid) {
				let dlg = &parts.conversation_data_dialog;
				crate::ui::conversation_data::show_photo(&dlg.photo, &dlg.info_panel, path);
			}
			if jid.ends_with("@g.us") && s.selected_chat.as_deref() == Some(jid) {
				let dlg = &parts.group_data_dialog;
				crate::ui::conversation_data::show_photo(&dlg.photo, &dlg.members_panel, path);
			}
		}
		"avatar_updated" => {
			if let Some(jid) = val.get("data").and_then(|d| d.get("jid")).and_then(|j| j.as_str()) {
				crate::ui::avatars::on_updated(jid);
			}
		}
		"about_updated" => {
			if let Some(jid) = val.get("data").and_then(|d| d.get("jid")).and_then(|j| j.as_str()) {
				let s = state.borrow();
				let is_relevant =
					s.info_chat.as_deref() == Some(jid) && parts.conversation_data_dialog.dialog.is_shown();
				if is_relevant {
					let payload = serde_json::json!({
						"action": "get_user_info",
						"data": { "jid": jid }
					});
					let _ = s.ws_tx.send(payload.to_string());
				}
			}
		}
		"contact_deleted" => {
			refresh_open_contact_info(state, parts);
			crate::platform::announce::announce(parts.conversations_parts.messages_announcer, "Contact deleted.");
			let s = state.borrow();
			if s.active_tab == 6 {
				crate::ui_commands::chat_list::request_call_tab_data(&s);
			}
		}
		"user_info" => {
			if let Some(data) = val.get("data") {
				let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
				let status = data.get("status").and_then(|s| s.as_str()).unwrap_or("");
				let push_name = data.get("pushName").and_then(|p| p.as_str()).unwrap_or("");
				let full_name = data.get("fullName").and_then(|f| f.as_str()).unwrap_or("");

				let s = state.borrow();
				if s.info_chat.as_deref() == Some(jid) {
					let local_name = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.to_string());
					let raw_phone = data
						.get("phone")
						.and_then(|p| p.as_str())
						.filter(|p| !p.is_empty())
						.unwrap_or_else(|| jid.split('@').next().unwrap_or(jid));
					let mut phone = raw_phone.to_string();
					if let Some(formatted) = whatsnap::phone::format_international(raw_phone) {
						phone = formatted;
					}

					let mut text = format!("Name: {}\nPhone: {}\n", local_name, phone);
					text.push_str(crate::ui_commands::blocking::info_line(&s, jid));
					if !push_name.is_empty() {
						text.push_str(&format!("WhatsApp Name: {}\n", push_name));
					} else if !full_name.is_empty() {
						text.push_str(&format!("WhatsApp Name: {}\n", full_name));
					} else {
						text.push_str("WhatsApp Name: Not available\n");
					}

					if !status.is_empty() {
						text.push_str(&format!("About/Status: {}\n", status));
					} else {
						text.push_str("About/Status: Not set\n");
					}

					parts.conversation_data_dialog.info_field.set_value(&text);
					drop(s);
					let card = crate::ui_commands::contacts_groups::ContactCard::from_user_info(data);
					crate::ui_commands::contacts_groups::show_contact_card(parts, Some(&card));
					state.borrow_mut().conversation_data_contact = Some(card);
					crate::platform::announce::announce(
						parts.conversations_parts.messages_announcer,
						&format!("Conversation data loaded for {}", local_name),
					);
				}
			}
		}
		_ => return false,
	}
	true
}
