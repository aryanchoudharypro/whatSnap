pub mod countries;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct ChatReaction {
	#[serde(rename = "reactorJid", alias = "reactor_jid", alias = "reactor", default)]
	pub reactor_jid: String,
	#[serde(rename = "reactorName", alias = "reactor_name", default)]
	pub reactor_name: String,
	#[serde(default)]
	pub emoji: String,
	#[serde(default)]
	pub timestamp: i64,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct ChatMessage {
	pub id: String,
	pub text: String,
	pub timestamp: i64,
	#[serde(rename = "fromMe", alias = "from_me", default)]
	pub from_me: bool,
	#[serde(rename = "senderJid", alias = "sender_jid", default)]
	pub sender_jid: String,
	#[serde(rename = "senderName", alias = "sender_name", default)]
	pub sender_name: String,
	#[serde(rename = "isAudio", alias = "is_audio", default)]
	pub is_audio: bool,
	#[serde(default)]
	pub is_date_divider: bool,
	#[serde(rename = "isDeleted", alias = "is_deleted", default)]
	pub is_deleted: bool,
	#[serde(default)]
	pub status: String,
	#[serde(default)]
	pub reactions: Vec<ChatReaction>,
	#[serde(rename = "quotedId", alias = "quoted_id", default)]
	pub quoted_id: String,
	#[serde(rename = "quotedText", alias = "quoted_text", default)]
	pub quoted_text: String,
	#[serde(rename = "quotedSender", alias = "quoted_sender", default)]
	pub quoted_sender: String,
}

impl ChatMessage {
	pub fn format_display(&self, _is_group: bool) -> String {
		if self.is_date_divider {
			return self.text.clone();
		}

		let formatted_time = if self.timestamp > 0 {
			if let Some(datetime) = chrono::DateTime::from_timestamp(self.timestamp, 0) {
				let local_dt: chrono::DateTime<chrono::Local> = datetime.into();
				local_dt.format("%l:%M %p").to_string().trim().to_string()
			} else {
				String::new()
			}
		} else {
			String::new()
		};

		let is_deleted = self.is_deleted || 
		                 self.text == "[You deleted this message]" || 
		                 self.text == "[This message was deleted]" || 
		                 self.text == "You deleted this message" || 
		                 self.text == "This message was deleted";

		if is_deleted {
			return if self.from_me {
				if !formatted_time.is_empty() {
					format!("You: deleted this message, sent at {}", formatted_time)
				} else {
					"You: deleted this message".to_string()
				}
			} else {
				let name = if !self.sender_name.is_empty() {
					&self.sender_name
				} else if !self.sender_jid.is_empty() {
					&self.sender_jid
				} else {
					"Contact"
				};
				if !formatted_time.is_empty() {
					format!("{}: deleted this message, received at {}", name, formatted_time)
				} else {
					format!("{}: deleted this message", name)
				}
			};
		}

		let is_voice = self.is_audio || self.text.starts_with("[Voice Message]") || self.text.starts_with("[Audio]");
		let status_title = match self.status.as_str() {
			"pending" => "Pending",
			"uploading" => "Uploading",
			"sent" => "Sent",
			"delivered" => "Delivered",
			"read" => "Read",
			"played" => "Played",
			"" => "Sent",
			other => other,
		};

		let body = if is_voice {
			let prefix = if self.text.starts_with("[Audio]") { "Audio" } else { "Voice message" };
			let tag = if self.text.starts_with("[Audio]") { "[Audio]" } else { "[Voice Message]" };
			
			if self.text == tag || self.text.is_empty() {
				prefix.to_string()
			} else if self.text.starts_with(tag) {
				let rest = self.text[tag.len()..].trim();
				if rest.is_empty() {
					prefix.to_string()
				} else if rest.starts_with('(') && rest.ends_with(')') {
					format!("{} Duration: {}", prefix, &rest[1..rest.len()-1])
				} else {
					format!("{} {}", prefix, rest)
				}
			} else {
				self.text.clone()
			}
		} else {
			if self.text.is_empty() {
				"[Media]".to_string()
			} else {
				self.text.clone()
			}
		};

		let reply_suffix = if !self.quoted_text.is_empty() || !self.quoted_id.is_empty() {
			let q_sender = if self.quoted_sender == "me" || self.quoted_sender == "You" {
				"You".to_string()
			} else if !self.quoted_sender.is_empty() && !self.quoted_sender.contains('@') {
				if self.quoted_sender.starts_with('@') {
					self.quoted_sender.clone()
				} else {
					format!("@{}", self.quoted_sender)
				}
			} else {
				"quoted message".to_string()
			};

			let q_body = if self.quoted_text.starts_with("[Voice Message]") {
				let rest = self.quoted_text["[Voice Message]".len()..].trim();
				if rest.is_empty() {
					"Voice message".to_string()
				} else if rest.starts_with('(') && rest.ends_with(')') {
					format!("Voice message Duration: {}", &rest[1..rest.len()-1])
				} else {
					format!("Voice message {}", rest)
				}
			} else if self.quoted_text.starts_with("[Audio]") {
				let rest = self.quoted_text["[Audio]".len()..].trim();
				if rest.is_empty() {
					"Audio".to_string()
				} else if rest.starts_with('(') && rest.ends_with(')') {
					format!("Audio Duration: {}", &rest[1..rest.len()-1])
				} else {
					format!("Audio {}", rest)
				}
			} else if self.quoted_text.is_empty() {
				"[Media]".to_string()
			} else {
				self.quoted_text.clone()
			};

			if q_sender == "quoted message" {
				format!(", replied to quoted message: \"{}\"", q_body)
			} else {
				format!(", replied to quoted message from {}: \"{}\"", q_sender, q_body)
			}
		} else {
			String::new()
		};

		let reactions_suffix = {
			let active: Vec<String> = self.reactions.iter().filter(|r| !r.emoji.is_empty()).map(|r| {
				let name = if r.reactor_name == "me" || r.reactor_name == "You" || r.reactor_jid == "me" {
					"You".to_string()
				} else if !r.reactor_name.is_empty() && !r.reactor_name.contains('@') {
					r.reactor_name.clone()
				} else if !self.sender_name.is_empty() && !self.sender_name.contains('@') && !self.from_me {
					self.sender_name.clone()
				} else {
					"Contact".to_string()
				};
				format!("{} reacted with {}", name, r.emoji)
			}).collect();

			if !active.is_empty() {
				format!(", {}", active.join(", "))
			} else {
				String::new()
			}
		};

		if self.from_me {
			if !formatted_time.is_empty() {
				format!("You: {}{}, sent at {}, {}{}", body, reply_suffix, formatted_time, status_title, reactions_suffix)
			} else {
				format!("You: {}{}, {}{}", body, reply_suffix, status_title, reactions_suffix)
			}
		} else {
			let name = if !self.sender_name.is_empty() {
				&self.sender_name
			} else if !self.sender_jid.is_empty() {
				&self.sender_jid
			} else {
				"Contact"
			};

			if !formatted_time.is_empty() {
				format!("{}: {}{}, received at {}{}", name, body, reply_suffix, formatted_time, reactions_suffix)
			} else {
				format!("{}: {}{}{}", name, body, reply_suffix, reactions_suffix)
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_chat_message_format_group_vs_direct() {
		let msg_from_me = ChatMessage {
			id: "1".into(),
			text: "kya kar raha hai?".into(),
			timestamp: 0,
			from_me: true,
			sender_jid: "user@s.whatsapp.net".into(),
			sender_name: "Me".into(),
			is_audio: false,
			is_date_divider: false,
			is_deleted: false,
			status: "delivered".into(),
			reactions: Vec::new(),
			quoted_id: "".into(),
			quoted_text: "".into(),
			quoted_sender: "".into(),
		};
		assert_eq!(msg_from_me.format_display(true), "You: kya kar raha hai?, Delivered");
		assert_eq!(msg_from_me.format_display(false), "You: kya kar raha hai?, Delivered");

		let msg_named = ChatMessage {
			id: "2".into(),
			text: "Hi".into(),
			timestamp: 0,
			from_me: false,
			sender_jid: "123@s.whatsapp.net".into(),
			sender_name: "Alice".into(),
			is_audio: false,
			is_date_divider: false,
			is_deleted: false,
			status: "".into(),
			reactions: Vec::new(),
			quoted_id: "".into(),
			quoted_text: "".into(),
			quoted_sender: "".into(),
		};
		assert_eq!(msg_named.format_display(true), "Alice: Hi");
		assert_eq!(msg_named.format_display(false), "Alice: Hi");

		let voice_received = ChatMessage {
			id: "3".into(),
			text: "[Voice Message] (0:10)".into(),
			timestamp: 0,
			from_me: false,
			sender_jid: "123@s.whatsapp.net".into(),
			sender_name: "Pooja".into(),
			is_audio: true,
			is_date_divider: false,
			is_deleted: false,
			status: "".into(),
			reactions: Vec::new(),
			quoted_id: "q1".into(),
			quoted_text: "[Voice Message] (0:23)".into(),
			quoted_sender: "alex".into(),
		};
		assert_eq!(voice_received.format_display(true), "Pooja: Voice message Duration: 0:10, replied to quoted message from @alex: \"Voice message Duration: 0:23\"");
	}

	#[test]
	fn test_chat_message_json_deserialization() {
		let json = r#"{"id":"m1","text":"test","timestamp":1700000000,"fromMe":false,"senderJid":"sender@s.whatsapp.net","senderName":"Sender","status":"read"}"#;
		let msg: ChatMessage = serde_json::from_str(json).unwrap();
		assert_eq!(msg.id, "m1");
		assert_eq!(msg.text, "test");
		assert_eq!(msg.timestamp, 1700000000);
		assert!(!msg.from_me);
		assert_eq!(msg.sender_jid, "sender@s.whatsapp.net");
		assert_eq!(msg.sender_name, "Sender");
		assert_eq!(msg.status, "read");
	}
}

