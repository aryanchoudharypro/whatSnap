pub mod countries;

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
}

impl ChatMessage {
	pub fn format_display(&self, is_group: bool) -> String {
		if self.from_me {
			format!("You: {}", self.text)
		} else if is_group {
			if !self.sender_name.is_empty() {
				format!("{}: {}", self.sender_name, self.text)
			} else if !self.sender_jid.is_empty() {
				format!("{}: {}", self.sender_jid, self.text)
			} else {
				format!("Contact: {}", self.text)
			}
		} else {
			if !self.sender_name.is_empty() {
				format!("{}: {}", self.sender_name, self.text)
			} else if !self.sender_jid.is_empty() {
				format!("{}: {}", self.sender_jid, self.text)
			} else {
				format!("Contact: {}", self.text)
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
			text: "Hello".into(),
			timestamp: 1700000000,
			from_me: true,
			sender_jid: "user@s.whatsapp.net".into(),
			sender_name: "Me".into(),
		};
		assert_eq!(msg_from_me.format_display(true), "You: Hello");
		assert_eq!(msg_from_me.format_display(false), "You: Hello");

		let msg_named = ChatMessage {
			id: "2".into(),
			text: "Hi".into(),
			timestamp: 1700000001,
			from_me: false,
			sender_jid: "123@s.whatsapp.net".into(),
			sender_name: "Alice".into(),
		};
		assert_eq!(msg_named.format_display(true), "Alice: Hi");
		assert_eq!(msg_named.format_display(false), "Alice: Hi");

		let msg_jid_only = ChatMessage {
			id: "3".into(),
			text: "Hey".into(),
			timestamp: 1700000002,
			from_me: false,
			sender_jid: "123@s.whatsapp.net".into(),
			sender_name: "".into(),
		};
		assert_eq!(msg_jid_only.format_display(true), "123@s.whatsapp.net: Hey");
		assert_eq!(msg_jid_only.format_display(false), "123@s.whatsapp.net: Hey");

		let msg_empty = ChatMessage {
			id: "4".into(),
			text: "Test".into(),
			timestamp: 1700000003,
			from_me: false,
			sender_jid: "".into(),
			sender_name: "".into(),
		};
		assert_eq!(msg_empty.format_display(true), "Contact: Test");
		assert_eq!(msg_empty.format_display(false), "Contact: Test");
	}

	#[test]
	fn test_chat_message_json_deserialization() {
		let json = r#"{"id":"m1","text":"test","timestamp":1700000000,"fromMe":false,"senderJid":"sender@s.whatsapp.net","senderName":"Sender"}"#;
		let msg: ChatMessage = serde_json::from_str(json).unwrap();
		assert_eq!(msg.id, "m1");
		assert_eq!(msg.text, "test");
		assert_eq!(msg.timestamp, 1700000000);
		assert!(!msg.from_me);
		assert_eq!(msg.sender_jid, "sender@s.whatsapp.net");
		assert_eq!(msg.sender_name, "Sender");
	}
}

