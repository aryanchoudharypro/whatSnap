use super::*;

pub(super) fn handle(t: &str, val: &serde_json::Value, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) -> bool {
	match t {
		"statuses" => {
			if let Some(data) = val.get("data")
				&& let Some(groups) = data.get("groups").and_then(|g| g.as_array())
			{
				let mut parsed = Vec::new();
				for g in groups {
					let poster_jid = g.get("posterJid").and_then(|j| j.as_str()).unwrap_or("").to_string();
					let poster_name = g.get("posterName").and_then(|n| n.as_str()).unwrap_or("").to_string();
					let all_viewed = g.get("allViewed").and_then(|v| v.as_bool()).unwrap_or(false);
					let mut items = Vec::new();
					if let Some(statuses) = g.get("statuses").and_then(|s| s.as_array()) {
						for it in statuses {
							items.push(StatusItem {
								id: it.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
								text: it.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string(),
								media_type: it.get("mediaType").and_then(|v| v.as_str()).unwrap_or("").to_string(),
								timestamp: it.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0),
								viewed: it.get("viewed").and_then(|v| v.as_bool()).unwrap_or(false),
								viewers: it.get("viewers").and_then(|v| v.as_i64()).unwrap_or(0),
								my_reaction: it.get("myReaction").and_then(|v| v.as_str()).unwrap_or("").to_string(),
								background: it.get("background").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
								text_color: it.get("textColor").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
								font: it.get("font").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
							});
						}
					}
					let muted = g.get("muted").and_then(|v| v.as_bool()).unwrap_or(false);
					let from_me = g.get("fromMe").and_then(|v| v.as_bool()).unwrap_or(false);
					parsed.push(StatusGroup { poster_jid, poster_name, all_viewed, from_me, muted, items });
				}
				let mut s = state.borrow_mut();
				s.status_groups = parsed;
				if s.active_tab == 1 {
					apply_chat_filter(&mut s, parts);
				}
			}
		}
		"status_update" => {
			if let Some(data) = val.get("data") {
				let id = data.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
				let poster_jid = data.get("posterJid").and_then(|v| v.as_str()).unwrap_or("").to_string();
				if id.is_empty() || poster_jid.is_empty() {
					return true;
				}
				let from_me = data.get("fromMe").and_then(|v| v.as_bool()).unwrap_or(false);
				let text = data.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
				let media_type = data.get("mediaType").and_then(|v| v.as_str()).unwrap_or("").to_string();
				let timestamp = data.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0);

				let mut s = state.borrow_mut();
				let poster_name = s
					.contacts
					.get(&poster_jid)
					.cloned()
					.unwrap_or_else(|| if from_me { "You".to_string() } else { poster_jid.clone() });
				let item = StatusItem {
					id,
					text,
					media_type,
					timestamp,
					viewed: from_me,
					viewers: 0,
					my_reaction: String::new(),
					background: data.get("background").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
					text_color: data.get("textColor").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
					font: data.get("font").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
				};
				if let Some(group) = s.status_groups.iter_mut().find(|g| g.poster_jid == poster_jid) {
					group.items.push(item);
					group.all_viewed = group.items.iter().all(|it| it.viewed);
				} else {
					s.status_groups.push(StatusGroup {
						poster_jid,
						poster_name,
						all_viewed: from_me,
						from_me,
						muted: false,
						items: vec![item],
					});
				}
				if s.active_tab == 1 {
					apply_chat_filter(&mut s, parts);
				}
			}
		}
		"status_viewers"
		| "status_deleted"
		| "status_muted"
		| "status_viewed"
		| "status_reaction"
		| "status_reacted"
		| "status_privacy"
		| "status_privacy_saved" => {
			if let Some(data) = val.get("data") {
				crate::ui_commands::status_extras::on_event(state, parts, t, data);
			}
		}
		_ => return false,
	}
	true
}
