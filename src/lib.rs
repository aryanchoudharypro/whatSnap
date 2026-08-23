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

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
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
	#[serde(default)]
	pub is_unread_divider: bool,
	#[serde(rename = "isDeleted", alias = "is_deleted", default)]
	pub is_deleted: bool,
	#[serde(rename = "isEdited", alias = "is_edited", default)]
	pub is_edited: bool,
	#[serde(default)]
	pub starred: bool,
	#[serde(default)]
	pub status: String,
	#[serde(rename = "editedAt", alias = "edited_at", default)]
	pub edited_at: i64,
	#[serde(rename = "sentAt", alias = "sent_at", default)]
	pub sent_at: i64,
	#[serde(rename = "deliveredAt", alias = "delivered_at", default)]
	pub delivered_at: i64,
	#[serde(rename = "readAt", alias = "read_at", default)]
	pub read_at: i64,
	#[serde(rename = "playedAt", alias = "played_at", default)]
	pub played_at: i64,
	#[serde(default)]
	pub reactions: Vec<ChatReaction>,
	#[serde(rename = "quotedId", alias = "quoted_id", default)]
	pub quoted_id: String,
	#[serde(rename = "quotedText", alias = "quoted_text", default)]
	pub quoted_text: String,
	#[serde(rename = "quotedSender", alias = "quoted_sender", default)]
	pub quoted_sender: String,
	#[serde(rename = "mediaType", alias = "media_type", default)]
	pub media_type: String,
	#[serde(rename = "mediaMimetype", alias = "media_mimetype", default)]
	pub media_mimetype: String,
	#[serde(rename = "mediaFilename", alias = "media_filename", default)]
	pub media_filename: String,
	#[serde(rename = "mediaPath", alias = "media_path", default)]
	pub media_path: String,
	/// ID of the album header the backend grouped this media under, empty for
	/// standalone media. Authoritative when present: it comes from WhatsApp's
	/// own album announcement rather than from guessing at timestamps.
	#[serde(rename = "albumId", alias = "album_id", default)]
	pub album_id: String,
}

/// Label shown on a date-divider row: "Today" / "Yesterday" for the two most
/// recent days, otherwise the full weekday/month/day/year string. Used both to
/// build the row text and (via `date_key`) to decide whether two messages fall
/// on the same day, so the same calendar comparison always governs both.
pub fn format_date_divider_label(local_dt: chrono::DateTime<chrono::Local>) -> String {
	let day = local_dt.date_naive();
	let today = chrono::Local::now().date_naive();
	if day == today {
		"Today".to_string()
	} else if day == today - chrono::Duration::days(1) {
		"Yesterday".to_string()
	} else {
		local_dt.format("%A, %B %e, %Y").to_string()
	}
}

/// Calendar-day key for a timestamp, used to decide whether two messages need
/// a date divider between them. Independent of `format_date_divider_label`'s
/// "Today"/"Yesterday" wording so that changes to the label never affect the
/// same-day comparison.
pub fn date_key(local_dt: chrono::DateTime<chrono::Local>) -> chrono::NaiveDate {
	local_dt.date_naive()
}

/// Converts a raw backend placeholder like `"[Image] caption"` or
/// `"[Document] file.pdf"` into the bracket-free text used in the chat list
/// preview (`"Photo: caption"`, `"Document: file.pdf"`, ...). Plain text
/// passes through unchanged.
pub fn friendly_preview_text(text: &str) -> String {
	if text.starts_with("[Voice Message]") {
		return "Voice message".to_string();
	}
	if text.starts_with("[Audio]") {
		return "Audio".to_string();
	}
	for tag in ["[Image]", "[Photo]"] {
		if let Some(rest) = text.strip_prefix(tag) {
			let rest = rest.trim();
			return if rest.is_empty() { "Photo".to_string() } else { format!("Photo: {}", rest) };
		}
	}
	if let Some(rest) = text.strip_prefix("[Video]") {
		let rest = rest.trim();
		return if rest.is_empty() { "Video".to_string() } else { format!("Video: {}", rest) };
	}
	if let Some(rest) = text.strip_prefix("[Document]") {
		let rest = rest.trim();
		return if rest.is_empty() { "Document".to_string() } else { format!("Document: {}", rest) };
	}
	if let Some(rest) = text.strip_prefix("[Live Location]") {
		let rest = rest.trim();
		return if rest.is_empty() { "Live location".to_string() } else { format!("Live location: {}", rest) };
	}
	if let Some(rest) = text.strip_prefix("[Contact]") {
		let rest = rest.trim();
		return if rest.is_empty() { "Contact".to_string() } else { format!("Contact: {}", rest) };
	}
	if let Some(rest) = text.strip_prefix("[Poll]") {
		let rest = rest.trim();
		return if rest.is_empty() { "Poll".to_string() } else { format!("Poll: {}", rest) };
	}
	match text {
		"[Location]" => "Location".to_string(),
		"[Sticker]" => "Sticker".to_string(),
		"[Message deleted]" => "This message was deleted".to_string(),
		"[Unsupported Message]" => "Message".to_string(),
		_ => text.to_string(),
	}
}

impl ChatMessage {
	pub fn is_document(&self) -> bool {
		self.media_type == "document" || self.text.starts_with("[Document]")
	}

	/// A message that reached us but carried nothing we could show. The content
	/// isn't recoverable locally - the only way back is asking the phone for
	/// its copy - so these are the ones worth offering "Request again" on.
	pub fn is_unreadable(&self) -> bool {
		!self.from_me && !self.is_date_divider && self.text == "[Unsupported Message]"
	}

	pub fn is_voice(&self) -> bool {
		self.is_audio || self.text.starts_with("[Voice Message]") || self.text.starts_with("[Audio]")
	}

	/// Photos and videos only - matches what official WhatsApp's Media tab
	/// shows (voice messages and documents get their own tabs).
	pub fn is_photo_or_video(&self) -> bool {
		!self.is_voice()
			&& (self.media_type == "image"
				|| self.media_type == "video"
				|| self.text.starts_with("[Photo]")
				|| self.text.starts_with("[Image]")
				|| self.text.starts_with("[Video]"))
	}

	/// The first URL-looking word in the message text, if any. Used to
	/// populate the Links tab; messages whose text is a backend placeholder
	/// (e.g. "[Document] file.pdf") never count as links.
	pub fn first_link(&self) -> Option<String> {
		if self.text.starts_with('[') {
			return None;
		}
		self.text.split_whitespace().find_map(|word| {
			let lower = word.to_lowercase();
			if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("www.") {
				Some(word.trim_matches(|c: char| ".,;:!?()'\"".contains(c)).to_string())
			} else {
				None
			}
		})
	}

	pub fn is_media_downloaded(&self) -> bool {
		!self.media_path.is_empty() && std::path::Path::new(&self.media_path).exists()
	}

	pub fn sanitize_filename(filename: &str) -> String {
		filename
			.replace(".x-python", ".py")
			.replace(".x-script.python", ".py")
			.replace(".x-shellscript", ".sh")
			.replace(".x-c", ".c")
			.replace(".x-c++", ".cpp")
			.replace(".x-ruby", ".rb")
			.replace(".x-perl", ".pl")
			.replace(".x-php", ".php")
	}

	pub fn display_filename(&self) -> String {
		let raw = if !self.media_filename.is_empty() {
			self.media_filename.clone()
		} else if let Some(rest) = self.text.strip_prefix("[Document]") {
			let rest = rest.trim();
			let name = rest.split(": ").next().unwrap_or(rest).trim();
			if !name.is_empty() { name.to_string() } else { format!("{}.bin", self.id) }
		} else {
			let mut ext = if !self.media_path.is_empty() {
				std::path::Path::new(&self.media_path).extension().and_then(|e| e.to_str()).unwrap_or("").to_string()
			} else {
				"".to_string()
			};

			if ext.is_empty() || ext == "bin" {
				if self.media_mimetype.contains("jpeg") || self.media_mimetype.contains("jpg") {
					ext = "jpg".to_string();
				} else if self.media_mimetype.contains("png") {
					ext = "png".to_string();
				} else if self.media_mimetype.contains("webp") {
					ext = "webp".to_string();
				} else if self.media_mimetype.contains("mp4") {
					ext = "mp4".to_string();
				} else if self.media_mimetype.contains("ogg") {
					ext = "ogg".to_string();
				} else if self.media_mimetype.contains("mp3") || self.media_mimetype.contains("mpeg") {
					ext = "mp3".to_string();
				} else if self.media_mimetype.contains("pdf") {
					ext = "pdf".to_string();
				} else {
					ext = match self.media_type.as_str() {
						"image" => "jpg",
						"video" => "mp4",
						"voice" => "ogg",
						"audio" => "mp3",
						"sticker" => "webp",
						_ => "bin",
					}
					.to_string();
				}
			}

			let prefix = match self.media_type.as_str() {
				"image" => "WhatsApp Image",
				"video" => "WhatsApp Video",
				"voice" => "WhatsApp PTT",
				"audio" => "WhatsApp Audio",
				_ => "WhatsApp File",
			};

			let mut time_str = String::new();
			if self.timestamp > 0 {
				if let Some(datetime) = chrono::DateTime::from_timestamp(self.timestamp, 0) {
					let local_datetime: chrono::DateTime<chrono::Local> = datetime.into();
					time_str = local_datetime.format(" %Y-%m-%d at %H.%M.%S").to_string();
				}
			}

			if time_str.is_empty() {
				format!("{} {}.{}", prefix, self.id, ext)
			} else {
				format!("{}{}.{}", prefix, time_str, ext)
			}
		};
		Self::sanitize_filename(&raw)
	}
	pub fn format_display(&self, _is_group: bool) -> String {
		if self.is_date_divider || self.is_unread_divider {
			return self.text.clone();
		}

		let mut formatted_time = String::new();
		if self.timestamp > 0 {
			if let Some(datetime) = chrono::DateTime::from_timestamp(self.timestamp, 0) {
				let local_datetime: chrono::DateTime<chrono::Local> = datetime.into();
				formatted_time = local_datetime.format("%I:%M %p").to_string();
			}
		}

		let is_deleted = self.is_deleted
			|| self.text == "[You deleted this message]"
			|| self.text == "[This message was deleted]"
			|| self.text == "You deleted this message"
			|| self.text == "This message was deleted";

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
			"failed" => "Failed to send",
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
					format!("{} Duration: {}", prefix, &rest[1..rest.len() - 1])
				} else {
					format!("{} {}", prefix, rest)
				}
			} else {
				self.text.clone()
			}
		} else if self.text.is_empty() {
			if self.is_document() || self.media_type == "document" {
				format!("Document: {}", self.display_filename())
			} else {
				"[Media]".to_string()
			}
		} else if self.text.starts_with('[') {
			// text is the backend's "[Document] file.docx"-style placeholder
			// (used for the chat-list preview and DB persistence), not a real
			// caption — translate it the same way the chat list does instead
			// of showing the bracket tag as if the user had typed it.
			friendly_preview_text(&self.text)
		} else {
			self.text.clone()
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
					format!("Voice message Duration: {}", &rest[1..rest.len() - 1])
				} else {
					format!("Voice message {}", rest)
				}
			} else if self.quoted_text.starts_with("[Audio]") {
				let rest = self.quoted_text["[Audio]".len()..].trim();
				if rest.is_empty() {
					"Audio".to_string()
				} else if rest.starts_with('(') && rest.ends_with(')') {
					format!("Audio Duration: {}", &rest[1..rest.len() - 1])
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
			let active: Vec<String> = self
				.reactions
				.iter()
				.filter(|r| !r.emoji.is_empty())
				.map(|r| {
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
				})
				.collect();

			if !active.is_empty() { format!(", {}", active.join(", ")) } else { String::new() }
		};

		let edit_suffix = if self.is_edited { ", edited" } else { "" };
		let star_suffix = if self.starred { ", starred" } else { "" };

		if self.from_me {
			if !formatted_time.is_empty() {
				format!(
					"You: {}{}, sent at {}, {}{}{}{}",
					body, reply_suffix, formatted_time, status_title, reactions_suffix, edit_suffix, star_suffix
				)
			} else {
				format!(
					"You: {}{}, {}{}{}{}",
					body, reply_suffix, status_title, reactions_suffix, edit_suffix, star_suffix
				)
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
				format!(
					"{}: {}{}, received at {}{}{}{}",
					name, body, reply_suffix, formatted_time, reactions_suffix, edit_suffix, star_suffix
				)
			} else {
				format!("{}: {}{}{}{}{}", name, body, reply_suffix, reactions_suffix, edit_suffix, star_suffix)
			}
		}
	}

	pub fn format_display_with_progress(&self, is_group: bool, progress: Option<&str>) -> String {
		let orig = self.format_display(is_group);
		if let Some(prog) = progress {
			if orig.contains("Voice message") {
				return orig.replace("Voice message", &format!("Voice message ({})", prog));
			} else if orig.contains("Audio") {
				return orig.replace("Audio", &format!("Audio ({})", prog));
			}
		}
		orig
	}
}

#[derive(Clone, Debug)]
pub enum DisplayItem {
	Single(ChatMessage),
	Album {
		id: String,
		sender_jid: String,
		sender_name: String,
		from_me: bool,
		timestamp: i64,
		messages: Vec<ChatMessage>,
	},
}

impl DisplayItem {
	pub fn id(&self) -> &str {
		match self {
			DisplayItem::Single(msg) => &msg.id,
			DisplayItem::Album { id, .. } => id,
		}
	}

	pub fn is_album(&self) -> bool {
		matches!(self, DisplayItem::Album { .. })
	}

	pub fn messages(&self) -> Vec<ChatMessage> {
		match self {
			DisplayItem::Single(msg) => vec![msg.clone()],
			DisplayItem::Album { messages, .. } => messages.clone(),
		}
	}

	pub fn all_downloaded(&self) -> bool {
		match self {
			DisplayItem::Single(msg) => msg.is_media_downloaded(),
			DisplayItem::Album { messages, .. } => {
				!messages.is_empty() && messages.iter().all(|m| m.is_media_downloaded())
			}
		}
	}

	pub fn is_deleted(&self) -> bool {
		match self {
			DisplayItem::Single(msg) => {
				msg.is_deleted
					|| msg.text == "[You deleted this message]"
					|| msg.text == "[This message was deleted]"
					|| msg.text == "You deleted this message"
					|| msg.text == "This message was deleted"
					|| msg.text.contains("deleted")
			}
			DisplayItem::Album { messages, .. } => messages.is_empty() || messages.iter().all(|m| m.is_deleted),
		}
	}

	pub fn format_display(&self, is_group: bool) -> String {
		match self {
			DisplayItem::Single(msg) => msg.format_display(is_group),
			DisplayItem::Album { from_me, sender_name, sender_jid, timestamp, messages, .. } => {
				let formatted_time = if *timestamp > 0 {
					if let Some(datetime) = chrono::DateTime::from_timestamp(*timestamp, 0) {
						let local_dt: chrono::DateTime<chrono::Local> = datetime.into();
						local_dt.format("%l:%M %p").to_string().trim().to_string()
					} else {
						String::new()
					}
				} else {
					String::new()
				};

				let photo_count = messages
					.iter()
					.filter(|m| {
						m.media_type == "image" || m.text.starts_with("[Photo]") || m.text.starts_with("[Image]")
					})
					.count();
				let video_count =
					messages.iter().filter(|m| m.media_type == "video" || m.text.starts_with("[Video]")).count();
				let total = messages.len();

				let desc = if photo_count > 0 && video_count == 0 {
					format!("{} photos", photo_count)
				} else if video_count > 0 && photo_count == 0 {
					format!("{} videos", video_count)
				} else {
					format!("{} media items", total)
				};

				if *from_me {
					if !formatted_time.is_empty() {
						format!("You: [Album: {}], sent at {}", desc, formatted_time)
					} else {
						format!("You: [Album: {}]", desc)
					}
				} else {
					let name = if !sender_name.is_empty() {
						sender_name.as_str()
					} else if !sender_jid.is_empty() {
						sender_jid.as_str()
					} else {
						"Contact"
					};
					if !formatted_time.is_empty() {
						format!("{}: [Album: {}], received at {}", name, desc, formatted_time)
					} else {
						format!("{}: [Album: {}]", name, desc)
					}
				}
			}
		}
	}

	pub fn format_display_with_progress(&self, is_group: bool, progress: Option<&str>) -> String {
		match self {
			DisplayItem::Single(msg) => msg.format_display_with_progress(is_group, progress),
			DisplayItem::Album { .. } => self.format_display(is_group),
		}
	}
}

/// Whether `msg` belongs to the same album as the media in `prev`. The backend
/// stamps media that arrived under one WhatsApp album header with a shared
/// `album_id`, and when that's present it decides on its own - it is what the
/// sender actually grouped, rather than an inference. Media stored before that
/// existed, or synced from history, carries no stamp, so it falls back to the
/// original "same sender, within ten seconds of each other" guess.
pub fn same_album(prev: &ChatMessage, msg: &ChatMessage) -> bool {
	if prev.from_me != msg.from_me || prev.sender_jid != msg.sender_jid {
		return false;
	}
	if !prev.album_id.is_empty() || !msg.album_id.is_empty() {
		return prev.album_id == msg.album_id;
	}
	(msg.timestamp - prev.timestamp).abs() <= 10
}

pub fn group_messages_into_display_items(messages: &[ChatMessage]) -> Vec<DisplayItem> {
	let mut items = Vec::new();
	let mut current_album: Vec<ChatMessage> = Vec::new();

	for msg in messages {
		let is_deleted = msg.is_deleted || msg.text.contains("deleted") || msg.text.contains("Deleted");
		let is_media = !is_deleted
			&& (msg.media_type == "image"
				|| msg.media_type == "video"
				|| msg.text.starts_with("[Photo]")
				|| msg.text.starts_with("[Image]")
				|| msg.text.starts_with("[Video]"));

		if is_media {
			if let Some(prev) = current_album.last() {
				if same_album(prev, msg) {
					current_album.push(msg.clone());
					continue;
				} else {
					if current_album.len() > 1 {
						let first = current_album[0].clone();
						items.push(DisplayItem::Album {
							id: first.id.clone(),
							sender_jid: first.sender_jid.clone(),
							sender_name: first.sender_name.clone(),
							from_me: first.from_me,
							timestamp: first.timestamp,
							messages: current_album.clone(),
						});
					} else if current_album.len() == 1 {
						items.push(DisplayItem::Single(current_album[0].clone()));
					}
					current_album = vec![msg.clone()];
				}
			} else {
				current_album = vec![msg.clone()];
			}
		} else {
			if current_album.len() > 1 {
				let first = current_album[0].clone();
				items.push(DisplayItem::Album {
					id: first.id.clone(),
					sender_jid: first.sender_jid.clone(),
					sender_name: first.sender_name.clone(),
					from_me: first.from_me,
					timestamp: first.timestamp,
					messages: current_album.clone(),
				});
			} else if current_album.len() == 1 {
				items.push(DisplayItem::Single(current_album[0].clone()));
			}
			current_album.clear();
			items.push(DisplayItem::Single(msg.clone()));
		}
	}

	if current_album.len() > 1 {
		let first = current_album[0].clone();
		items.push(DisplayItem::Album {
			id: first.id.clone(),
			sender_jid: first.sender_jid.clone(),
			sender_name: first.sender_name.clone(),
			from_me: first.from_me,
			timestamp: first.timestamp,
			messages: current_album,
		});
	} else if current_album.len() == 1 {
		items.push(DisplayItem::Single(current_album[0].clone()));
	}

	items
}

/// Inserts an "N unread messages" divider before the last `count` non-divider
/// (real) messages in `messages`, e.g. to mark where messages that arrived
/// while the chat was open but unread pick up. No-op if `count` is 0 or
/// exceeds the number of real messages present.
pub fn insert_unread_divider(messages: &mut Vec<ChatMessage>, count: u64) {
	let n = count as usize;
	if n == 0 {
		return;
	}
	let real_count = messages.iter().filter(|m| !m.is_date_divider && !m.is_unread_divider).count();
	if n > real_count {
		return;
	}
	let target_real_index = real_count - n;
	let mut seen_real = 0usize;
	let insert_at = messages
		.iter()
		.position(|m| {
			if m.is_date_divider || m.is_unread_divider {
				return false;
			}
			if seen_real == target_real_index {
				return true;
			}
			seen_real += 1;
			false
		})
		.unwrap_or(messages.len());
	let label = if n == 1 { "1 unread message".to_string() } else { format!("{} unread messages", n) };
	messages.insert(insert_at, ChatMessage { text: label, is_unread_divider: true, ..Default::default() });
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
			sent_at: 0,
			delivered_at: 0,
			read_at: 0,
			played_at: 0,
			reactions: Vec::new(),
			quoted_id: "".into(),
			quoted_text: "".into(),
			quoted_sender: "".into(),
			..Default::default()
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
			sent_at: 0,
			delivered_at: 0,
			read_at: 0,
			played_at: 0,
			reactions: Vec::new(),
			quoted_id: "".into(),
			quoted_text: "".into(),
			quoted_sender: "".into(),
			..Default::default()
		};
		assert_eq!(msg_named.format_display(true), "Alice: Hi");
		assert_eq!(msg_named.format_display(false), "Alice: Hi");

		let voice_received = ChatMessage {
			id: "3".into(),
			text: "[Voice Message] (0:10)".into(),
			timestamp: 0,
			from_me: false,
			sender_jid: "123@s.whatsapp.net".into(),
			sender_name: "Ravi".into(),
			is_audio: true,
			is_date_divider: false,
			is_deleted: false,
			status: "".into(),
			sent_at: 0,
			delivered_at: 0,
			read_at: 0,
			played_at: 0,
			reactions: Vec::new(),
			quoted_id: "q1".into(),
			quoted_text: "[Voice Message] (0:23)".into(),
			quoted_sender: "alex".into(),
			..Default::default()
		};
		assert_eq!(
			voice_received.format_display(true),
			"Ravi: Voice message Duration: 0:10, replied to quoted message from @alex: \"Voice message Duration: 0:23\""
		);
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
