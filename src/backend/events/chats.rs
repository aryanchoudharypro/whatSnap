use super::*;

#[derive(Deserialize)]
pub(super) struct ChatRow<'a> {
	#[serde(rename = "JID", borrow, default)]
	pub(super) jid: Cow<'a, str>,
	#[serde(rename = "FullName", borrow, default)]
	pub(super) full_name: Option<Cow<'a, str>>,
	#[serde(rename = "UnreadCount", default)]
	pub(super) unread_count: u64,
	#[serde(rename = "Pinned", default)]
	pub(super) pinned: bool,
	#[serde(rename = "Muted", default)]
	pub(super) muted: bool,
	#[serde(rename = "Archived", default)]
	pub(super) archived: bool,
	#[serde(rename = "IsLocked", default)]
	pub(super) is_locked: bool,
	#[serde(rename = "IsCommunity", default)]
	pub(super) is_community: bool,
	#[serde(rename = "PreviewSnippet", borrow, default)]
	pub(super) preview_snippet: Cow<'a, str>,
	#[serde(rename = "Timestamp", default)]
	pub(super) timestamp: i64,
	#[serde(rename = "PreviewSender", borrow, default)]
	pub(super) preview_sender: Cow<'a, str>,
	#[serde(rename = "PreviewFromMe", default)]
	pub(super) preview_from_me: bool,
	#[serde(rename = "PreviewStatus", borrow, default)]
	pub(super) preview_status: Cow<'a, str>,
	#[serde(rename = "PreviewIsReaction", default)]
	pub(super) preview_is_reaction: bool,
	#[serde(rename = "PreviewReactionEmoji", borrow, default)]
	pub(super) preview_reaction_emoji: Cow<'a, str>,
	#[serde(rename = "IsChannel", default)]
	pub(super) is_channel: bool,
	#[serde(rename = "ChannelRole", borrow, default)]
	pub(super) channel_role: Cow<'a, str>,
	#[serde(rename = "ParentJID", borrow, default)]
	pub(super) parent_jid: Cow<'a, str>,
	#[serde(rename = "CommunityName", borrow, default)]
	pub(super) community_name: Cow<'a, str>,
	#[serde(rename = "IsAnnouncements", default)]
	pub(super) is_announcements: bool,
	#[serde(rename = "IsAdmin", default)]
	pub(super) is_admin: bool,
	#[serde(rename = "AdminsOnly", default)]
	pub(super) admins_only: bool,
	#[serde(rename = "NotMember", default)]
	pub(super) not_member: bool,
	#[serde(rename = "IsBusiness", default)]
	pub(super) is_business: bool,
}

#[derive(Deserialize)]
pub(super) struct ChatsEnvelope<'a> {
	#[serde(borrow, default)]
	pub(super) data: Option<Vec<ChatRow<'a>>>,
}

pub(super) fn apply_chats(chats: Vec<ChatRow>, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let mut chat_list = Vec::new();
	let mut archived_jids = std::collections::HashSet::new();
	let mut community_jids = std::collections::HashSet::new();
	let mut chat_parents = std::collections::HashMap::new();
	let mut announcement_jids = std::collections::HashSet::new();
	let mut channel_jids = std::collections::HashSet::new();
	let mut channel_roles = std::collections::HashMap::new();
	let mut pinned_jids = std::collections::HashSet::new();
	let mut muted_jids = std::collections::HashSet::new();
	let mut admin_group_jids = std::collections::HashSet::new();
	let mut admins_only_jids = std::collections::HashSet::new();
	let mut not_member_jids = std::collections::HashSet::new();
	let mut business_jids = std::collections::HashSet::new();
	let mut locked_jids = std::collections::HashSet::new();
	let mut unread_counts = std::collections::HashMap::new();
	let mut display_names = std::collections::HashMap::new();
	let mut contacts_map = std::collections::HashMap::new();
	let mut row_art = std::collections::HashMap::new();
	for chat in &chats {
		let jid: &str = &chat.jid;
		if !jid.is_empty() {
			let name = chat.full_name.as_deref().unwrap_or(jid).to_string();
			chat_list.push(jid.to_string());

			let unread = chat.unread_count;
			let pinned = chat.pinned;
			let muted = chat.muted;
			let archived = chat.archived;
			let is_locked = chat.is_locked;
			let is_community = chat.is_community;
			let preview = &*chat.preview_snippet;

			let ts = chat.timestamp;
			let preview_sender = &*chat.preview_sender;
			let preview_from_me = chat.preview_from_me;
			let preview_is_reaction = chat.preview_is_reaction;
			let preview_reaction_emoji = &*chat.preview_reaction_emoji;

			let time_str = if ts > 0 {
				if let Some(local_dt) = whatsnap::LocalTime::from_unix(ts) {
					let now_dt = whatsnap::LocalTime::now();
					let msg_date = local_dt.date();
					let today = now_dt.date();
					let yesterday = today.add_days(-1);
					let num_days_ago = today.days_since(msg_date);
					if msg_date == today {
						local_dt.format("%I:%M %p")
					} else if msg_date == yesterday {
						"Yesterday".to_string()
					} else if num_days_ago > 1 && num_days_ago < 7 {
						local_dt.format("%A")
					} else {
						local_dt.format("%d/%m/%Y")
					}
				} else {
					String::new()
				}
			} else {
				String::new()
			};

			let mut tags: Vec<Cow<str>> = Vec::new();
			if is_locked {
				tags.push("Locked".into());
				locked_jids.insert(jid.to_string());
			}
			if archived {
				tags.push("Archived".into());
				archived_jids.insert(jid.to_string());
			}
			if pinned {
				tags.push("Pinned".into());
				pinned_jids.insert(jid.to_string());
			}
			if muted {
				tags.push("Muted".into());
				muted_jids.insert(jid.to_string());
			}
			if is_community {
				tags.push("Community".into());
				community_jids.insert(jid.to_string());
			}
			if chat.is_channel {
				tags.push("Channel".into());
				channel_jids.insert(jid.to_string());
				let role = &*chat.channel_role;
				channel_roles.insert(jid.to_string(), role.to_string());
			}
			let parent = &*chat.parent_jid;
			if !parent.is_empty() {
				chat_parents.insert(jid.to_string(), parent.to_string());
				let community_name = &*chat.community_name;
				if chat.is_announcements {
					announcement_jids.insert(jid.to_string());
					tags.push("Announcements".into());
				}
				if !community_name.is_empty() && community_name != name {
					tags.push(format!("In {}", community_name).into());
				}
			}
			if chat.is_admin {
				admin_group_jids.insert(jid.to_string());
			}
			if chat.admins_only {
				admins_only_jids.insert(jid.to_string());
			}
			if chat.not_member {
				not_member_jids.insert(jid.to_string());
			}
			if chat.is_business {
				business_jids.insert(jid.to_string());
			}
			unread_counts.insert(jid.to_string(), unread);

			let name_line = if tags.is_empty() { name.clone() } else { format!("{} [{}]", name, tags.join("] [")) };

			let unread_line = if unread == 1 {
				"1 unread message".to_string()
			} else if unread > 1 {
				format!("{} unread messages", unread)
			} else {
				String::new()
			};

			let preview_line = if preview_is_reaction && !preview_reaction_emoji.is_empty() && !preview.is_empty() {
				let who = if preview_from_me {
					"You".to_string()
				} else if !preview_sender.is_empty() {
					preview_sender.to_string()
				} else {
					"Someone".to_string()
				};
				format!(
					"{} reacted with {} to: {}",
					who,
					preview_reaction_emoji,
					whatsnap::friendly_preview_text(preview)
				)
			} else if !preview.is_empty() {
				let body = whatsnap::friendly_preview_text(preview);
				if preview_from_me {
					format!("You: {}", body)
				} else if !preview_sender.is_empty() && jid.ends_with("@g.us") {
					format!("{}: {}", preview_sender, body)
				} else {
					body
				}
			} else {
				String::new()
			};

			let (preview_prefix, icon, preview_text) = if preview.is_empty() {
				(String::new(), None, String::new())
			} else if preview_is_reaction && !preview_reaction_emoji.is_empty() {
				(String::new(), None, preview_line.clone())
			} else {
				let (icon, text) = crate::ui::chat_rows::preview_icon(preview);
				let prefix = if preview_from_me {
					""
				} else {
					preview_line.strip_suffix(&whatsnap::friendly_preview_text(preview)).unwrap_or("")
				};
				(prefix.to_string(), icon, text)
			};
			let extra: Vec<&str> =
				tags.iter().map(|t| t.as_ref()).filter(|t| !matches!(*t, "Pinned" | "Muted" | "Locked")).collect();
			row_art.insert(
				jid.to_string(),
				crate::ui::chat_rows::RowArt {
					jid: jid.to_string(),
					title: name.clone(),
					extra: extra.join(" · "),
					time: time_str.clone(),
					preview_prefix,
					icon,
					preview: preview_text,
					ticks: (preview_from_me && !preview_is_reaction && !preview.is_empty())
						.then(|| crate::ui::message_art::ticks_for(&chat.preview_status)),
					unread: unread.min(u32::MAX as u64) as u32,
					pinned,
					muted,
					locked: is_locked,
					..Default::default()
				},
			);

			let mut parts_vec: Vec<&str> = Vec::new();
			parts_vec.push(&name_line);
			if !unread_line.is_empty() {
				parts_vec.push(&unread_line);
			}
			if !time_str.is_empty() {
				parts_vec.push(&time_str);
			}
			if !preview_line.is_empty() {
				parts_vec.push(&preview_line);
			}
			let display_name = parts_vec.join(" | ");
			display_names.insert(jid.to_string(), display_name);
			contacts_map.insert(jid.to_string(), name);
		}
	}
	let mut s = state.borrow_mut();
	s.chats = chat_list;
	s.contacts = contacts_map;
	s.archived_jids = archived_jids;
	s.locked_jids = locked_jids;
	s.community_jids = community_jids;
	s.chat_parents = chat_parents;
	s.announcement_jids = announcement_jids;
	s.channel_jids = channel_jids;
	s.channel_roles = channel_roles;
	s.pinned_jids = pinned_jids;
	s.muted_jids = muted_jids;
	s.admin_group_jids = admin_group_jids;
	s.admins_only_jids = admins_only_jids;
	s.not_member_jids = not_member_jids;
	if s.business_jids != business_jids {
		s.business_jids = business_jids;
		crate::ui::window::refresh_menu_bar(&s, &parts.frame);
	}
	if let Some(jid) = s.selected_chat.clone() {
		crate::ui::chat_ui::apply_composer_access(&s, parts, &jid);
	}
	for (jid, old) in &s.unread_counts {
		if *old > 0 && unread_counts.get(jid).is_none_or(|new| *new == 0) {
			crate::ui::chat_ui::clear_chat_notifications(jid);
		}
	}
	s.unread_counts = unread_counts;
	s.display_names = display_names;
	if s.chat_row_art != row_art {
		crate::ui::chat_rows::refresh();
	}
	s.chat_row_art = row_art;
	refresh_tray_tooltip(&s, parts);

	let lower_query = s.search_query.to_lowercase();
	let mut new_filtered = Vec::new();
	let mut new_display = Vec::new();
	if s.active_tab != 1 && s.active_tab != 2 {
		for jid in chats_in_tab_order(&s) {
			if !chat_visible_in_tab(
				s.active_tab,
				jid,
				&s.archived_jids,
				&s.community_jids,
				&s.chat_parents,
				&s.channel_jids,
				&s.locked_jids,
				s.chats_unlocked,
			) || !crate::ui::message_display::chat_passes_filter(&s, jid)
			{
				continue;
			}
			let name = s.contacts.get(jid).unwrap_or(jid);
			if lower_query.is_empty() || name.to_lowercase().contains(&lower_query) {
				new_filtered.push(jid.clone());
				new_display.push(s.display_names.get(jid).unwrap_or(name).clone());
			}
		}
	}

	let structure_changed = s.active_tab != 1
		&& s.active_tab != 2
		&& (new_filtered.len() != s.filtered_chats.len()
			|| new_filtered.iter().zip(s.filtered_chats.iter()).any(|(a, b)| a != b));

	// The Status and Starred tabs keep their own rows (and
	// their own filtered_chats) - a chats refresh must leave
	// them alone, or Enter on a row stops doing anything.
	let own_rows_tab = s.active_tab == 1 || s.active_tab == 2 || s.active_tab == 6;
	if !own_rows_tab && structure_changed {
		drop(s);
		apply_chat_filter(&mut state.borrow_mut(), parts);
	} else if !own_rows_tab {
		// Same rows in the same order: rewrite only the ones
		// whose text actually changed. A chats broadcast lands
		// every ~200ms during a sync, and rewriting every row
		// each time was pure churn (and a screen reader can
		// re-read a row whose text is set, even to itself).
		let changed: Vec<usize> = new_display
			.iter()
			.enumerate()
			.filter(|(i, dn)| s.displayed_chat_labels.get(*i) != Some(*dn))
			.map(|(i, _)| i)
			.collect();
		if !changed.is_empty() {
			parts.conversations_parts.conversations_list.freeze();
			for &i in &changed {
				parts.conversations_parts.conversations_list.set_string(i as u32, &new_display[i]);
			}
			parts.conversations_parts.conversations_list.thaw();
		}
		s.filtered_chats = new_filtered;
		s.displayed_chat_labels = new_display;
		drop(s);
	} else {
		drop(s);
	}
	crate::ui_commands::links::open_pending_whatsapp_link(state);
}
