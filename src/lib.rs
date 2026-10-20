pub mod countries;
pub mod local_time;
pub use local_time::{Date, LocalTime};

/// WhatsApp sends `notify="[]"` (and the backend forwards it verbatim as the
/// sender/quoted-sender name) when a contact has no push name set. Treat that
/// literal placeholder the same as an empty name rather than rendering it.
fn is_placeholder_name(name: &str) -> bool {
	name.is_empty() || name == "[]"
}

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
	#[serde(rename = "mediaSize", alias = "media_size", default)]
	pub media_size: i64,
	/// ID of the album header the backend grouped this media under, empty for
	/// standalone media. Authoritative when present: it comes from WhatsApp's
	/// own album announcement rather than from guessing at timestamps.
	#[serde(rename = "albumId", alias = "album_id", default)]
	pub album_id: String,
	/// For a poll: each option's share of the votes ("Yes 50%, 1 vote; ..."),
	/// shown after the question.
	#[serde(rename = "pollResults", alias = "poll_results", default)]
	pub poll_results: String,
	/// For a channel post: its view count and reaction totals ("1,204 views,
	/// 👍 31, ❤️ 12"). Channels show counts, not who reacted.
	#[serde(rename = "channelStats", alias = "channel_stats", default)]
	pub channel_stats: String,
	/// A pinned channel post.
	#[serde(rename = "channelPinned", default)]
	pub channel_pinned: bool,
	/// Pinned in its chat.
	#[serde(default)]
	pub pinned: bool,
	/// Kept in a chat with disappearing messages, so it won't disappear.
	#[serde(default)]
	pub kept: bool,
	/// Will disappear (so can be kept).
	#[serde(default)]
	pub expiring: bool,
	/// A sticker that's one of our favorites.
	#[serde(default)]
	pub sticker_favorite: bool,
}

/// Renders a byte count the way file managers do: whole numbers under 1000
/// bytes, one decimal place from KB up. `0` (unknown size, e.g. an older
/// message synced before the backend recorded it) renders as `None`.
pub fn format_file_size(bytes: i64) -> Option<String> {
	if bytes <= 0 {
		return None;
	}
	const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
	let mut size = bytes as f64;
	let mut unit = 0;
	while size >= 1000.0 && unit < UNITS.len() - 1 {
		size /= 1000.0;
		unit += 1;
	}
	Some(if unit == 0 { format!("{} {}", bytes, UNITS[0]) } else { format!("{:.1} {}", size, UNITS[unit]) })
}

/// Label shown on a date-divider row: "Today" / "Yesterday" for the two most
/// recent days, otherwise the full weekday/month/day/year string. Used both to
/// build the row text and (via `date_key`) to decide whether two messages fall
/// on the same day, so the same calendar comparison always governs both.
pub fn format_date_divider_label(local_dt: LocalTime) -> String {
	let day = local_dt.date();
	let today = LocalTime::now().date();
	if day == today {
		"Today".to_string()
	} else if day == today.add_days(-1) {
		"Yesterday".to_string()
	} else {
		local_dt.format("%A, %B %e, %Y")
	}
}

/// Calendar-day key for a timestamp, used to decide whether two messages need
/// a date divider between them. Independent of `format_date_divider_label`'s
/// "Today"/"Yesterday" wording so that changes to the label never affect the
/// same-day comparison.
pub fn date_key(local_dt: LocalTime) -> Date {
	local_dt.date()
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
	if let Some(rest) = text.strip_prefix("[Sticker]") {
		let rest = rest.trim();
		return if rest.is_empty() { "Sticker".to_string() } else { format!("Sticker: {}", rest) };
	}
	if let Some(rest) = text.strip_prefix("[Quiz]") {
		let rest = rest.trim();
		return if rest.is_empty() { "Quiz".to_string() } else { format!("Quiz: {}", rest) };
	}
	match text {
		"[Location]" => "Location".to_string(),
		"[Message deleted]" => "This message was deleted".to_string(),
		"[Unsupported Message]" => "Message".to_string(),
		_ => text.to_string(),
	}
}

impl ChatMessage {
	pub fn is_document(&self) -> bool {
		self.media_type == "document" || self.text.starts_with("[Document]")
	}

	/// A synthesized group-change line ("Aryan added Rishit", "Rishit left"),
	/// not a real message - shown as plain text with no sender/time framing.
	pub fn is_system(&self) -> bool {
		self.media_type == "system"
	}

	/// A message that reached us but carried nothing we could show. The content
	/// isn't recoverable locally - the only way back is asking the phone for
	/// its copy - so these are the ones worth offering "Request again" on.
	pub fn is_unreadable(&self) -> bool {
		!self.from_me && !self.is_date_divider && self.text == "[Unsupported Message]"
	}

	pub fn is_poll(&self) -> bool {
		self.text.starts_with("[Poll]") || self.text.starts_with("[Quiz]")
	}

	/// An event (not a cancelled one) - `V` answers it.
	pub fn is_event(&self) -> bool {
		self.text.starts_with("[Event]")
	}

	pub fn is_voice(&self) -> bool {
		self.is_audio || self.text.starts_with("[Voice Message]") || self.text.starts_with("[Audio]")
	}

	/// How long after sending WhatsApp lets a message be edited. Sourced from
	/// whatsmeow's own `EditWindow = 20 * time.Minute` (send.go) - the value
	/// it documents as the real server-side policy. whatsmeow's own
	/// `BuildEdit`/`SendMessage` don't enforce it, so an edit past this point
	/// still sends and syncs to your own other devices without error, which
	/// first looked like the window wasn't real - but other recipients don't
	/// actually see it update, confirming the server does enforce it, just
	/// silently. This is a client-side-only guard on top of that.
	pub const EDIT_WINDOW_SECS: i64 = 20 * 60;

	/// Whether this message can still be edited: ours, not deleted, not a
	/// voice message (WhatsApp never allows editing those), and sent less
	/// than `EDIT_WINDOW_SECS` ago. `now` is the caller's own unix-seconds
	/// clock reading, so every call site in one pass agrees on "now" instead
	/// of each computing it separately.
	pub fn is_editable(&self, now: i64) -> bool {
		// Plain text only. Caption edits on media go out in a different wire
		// shape that hasn't been confirmed against the official apps yet, and
		// sending an unusual edit is exactly the kind of thing worth not
		// guessing at.
		self.from_me
			&& !self.is_deleted
			&& !self.is_voice()
			&& !self.is_system()
			&& !self.is_call()
			&& self.media_type.is_empty()
			&& !self.text.starts_with('[')
			&& (now - self.timestamp) < Self::EDIT_WINDOW_SECS
	}

	/// How long after sending WhatsApp lets a message be deleted for everyone.
	/// Unlike `EDIT_WINDOW_SECS`, whatsmeow documents no constant for this at
	/// all (`BuildRevoke` just builds a REVOKE unconditionally) - 60 hours is
	/// sourced from WhatsApp's own 2022 announcement of the extended window
	/// (reported by TechCrunch and corroborated elsewhere), the current
	/// settled figure as of this writing, not something whatsmeow verifies.
	pub const DELETE_FOR_EVERYONE_WINDOW_SECS: i64 = 60 * 60 * 60;

	/// Whether "Delete for everyone" is still offered for this message: ours,
	/// not already deleted, and sent less than
	/// `DELETE_FOR_EVERYONE_WINDOW_SECS` ago. `now` is the caller's own
	/// unix-seconds clock reading, matching `is_editable`'s convention.
	pub fn is_delete_for_everyone_eligible(&self, now: i64) -> bool {
		self.from_me && !self.is_deleted && (now - self.timestamp) < Self::DELETE_FOR_EVERYONE_WINDOW_SECS
	}

	/// A synced call-history row (a call placed or received on any of this
	/// account's devices), rather than a message somebody sent. Enter on one
	/// calls the chat back, the way Enter on a voice message plays it.
	pub fn is_call(&self) -> bool {
		self.media_type == "call"
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

	/// The first phone-number-looking run of text in the message, if any.
	/// Returns (matched substring as it appeared in the text, digits-only
	/// form suitable for building a `<digits>@s.whatsapp.net` JID). Runs are
	/// validated with `phonenumber` so things like order numbers or dates
	/// don't false-positive.
	pub fn first_phone_number(&self) -> Option<(String, String)> {
		if self.text.starts_with('[') {
			return None;
		}
		let chars: Vec<char> = self.text.chars().collect();
		let n = chars.len();
		let mut i = 0;
		while i < n {
			let c = chars[i];
			if (c == '+' || c.is_ascii_digit()) && (i == 0 || !chars[i - 1].is_alphanumeric()) {
				let start = i;
				let mut j = i;
				let mut digit_count = 0usize;
				let mut end = i;
				while j < n {
					let cj = chars[j];
					if cj.is_ascii_digit() {
						digit_count += 1;
						j += 1;
						end = j;
					} else if cj == '+' || cj == '-' || cj == '(' || cj == ')' || cj == '.' {
						j += 1;
					} else if cj == ' ' {
						let mut k = j + 1;
						while k < n && chars[k] == ' ' {
							k += 1;
						}
						if k < n && chars[k].is_ascii_digit() {
							j = k;
						} else {
							break;
						}
					} else {
						break;
					}
				}
				if (8..=15).contains(&digit_count) {
					let candidate: String = chars[start..end].iter().collect();
					let digits: String = candidate.chars().filter(|c| c.is_ascii_digit()).collect();
					let looks_valid = phonenumber::parse(None, candidate.clone())
						.or_else(|_| phonenumber::parse(None, format!("+{}", digits)))
						.map(|p| p.is_valid())
						.unwrap_or(false);
					if looks_valid {
						return Some((candidate, digits));
					}
				}
				i = j.max(i + 1);
				continue;
			}
			i += 1;
		}
		None
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
			if self.timestamp > 0
				&& let Some(local_datetime) = LocalTime::from_unix(self.timestamp)
			{
				time_str = local_datetime.format(" %Y-%m-%d at %H.%M.%S");
			}

			if time_str.is_empty() {
				format!("{} {}.{}", prefix, self.id, ext)
			} else {
				format!("{}{}.{}", prefix, time_str, ext)
			}
		};
		Self::sanitize_filename(&raw)
	}

	/// Just the message's own content — no sender name, timestamp, status,
	/// reply-quote, or reaction suffixes. `format_display` builds its `body`
	/// from this; it's also what a Shift+Enter "view/copy this message"
	/// dialog should show, since a screen reader review field is for reading
	/// the message that was sent, not repeating everything the row already
	/// announced around it.
	pub fn body_text(&self) -> String {
		let is_deleted = self.is_deleted
			|| self.text == "[You deleted this message]"
			|| self.text == "[This message was deleted]"
			|| self.text == "You deleted this message"
			|| self.text == "This message was deleted";
		if is_deleted {
			return "This message was deleted".to_string();
		}

		let is_voice = self.is_audio || self.text.starts_with("[Voice Message]") || self.text.starts_with("[Audio]");
		if is_voice {
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
		} else if self.is_poll() && !self.poll_results.is_empty() {
			let question = friendly_preview_text(&self.text);
			let sep = if question.ends_with(['?', '!', '.']) { " " } else { ". " };
			format!("{question}{sep}{}", self.poll_results)
		} else if self.is_event() && !self.poll_results.is_empty() {
			// The RSVP tally, after the event's details.
			let details = self.text.trim_end();
			let sep = if details.ends_with(['?', '!', '.']) { " " } else { ". " };
			format!("{details}{sep}{}", self.poll_results)
		} else if self.text.starts_with('[') {
			// text is the backend's "[Document] file.docx"-style placeholder
			// (used for the chat-list preview and DB persistence), not a real
			// caption — translate it the same way the chat list does instead
			// of showing the bracket tag as if the user had typed it.
			friendly_preview_text(&self.text)
		} else {
			self.text.clone()
		}
	}

	pub fn format_display(&self, _is_group: bool) -> String {
		if self.is_date_divider || self.is_unread_divider || self.is_system() {
			return self.text.clone();
		}

		let mut formatted_time = String::new();
		if self.timestamp > 0
			&& let Some(local_datetime) = LocalTime::from_unix(self.timestamp)
		{
			formatted_time = local_datetime.format("%I:%M %p");
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
				let name = if !is_placeholder_name(&self.sender_name) {
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

		let body = self.body_text();

		let reply_suffix = if !self.quoted_text.is_empty() || !self.quoted_id.is_empty() {
			let q_sender = if self.quoted_sender == "me" || self.quoted_sender == "You" {
				"You".to_string()
			} else if !is_placeholder_name(&self.quoted_sender) && !self.quoted_sender.contains('@') {
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
					} else if !is_placeholder_name(&r.reactor_name) && !r.reactor_name.contains('@') {
						r.reactor_name.clone()
					} else if !is_placeholder_name(&self.sender_name)
						&& !self.sender_name.contains('@')
						&& !self.from_me
					{
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
		let star_suffix = format!(
			"{}{}{}",
			if self.starred { ", starred" } else { "" },
			if self.pinned { ", pinned" } else { "" },
			if self.kept { ", kept" } else { "" }
		);
		let reactions_suffix = if self.channel_stats.is_empty() {
			reactions_suffix
		} else {
			format!("{}, {}", reactions_suffix, self.channel_stats)
		};

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
			let name = if !is_placeholder_name(&self.sender_name) {
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

/// Test helper: wraps plain messages as the shared `Msg`s the list code takes.
#[cfg(test)]
fn rc(msgs: &[ChatMessage]) -> Vec<Msg> {
	msgs.iter().cloned().map(Msg::new).collect()
}

/// A loaded message, shared between the open chat's message list and the
/// display items grouped from it rather than copied into both: a whole chat
/// can be tens of thousands of messages, and the second copy was tens of MB.
/// Code that changes a loaded message goes through
/// `message_display::update_loaded_messages`, which keeps it shared.
pub type Msg = std::rc::Rc<ChatMessage>;

#[derive(Clone, Debug)]
pub enum DisplayItem {
	Single(Msg),
	Album { id: String, sender_jid: String, sender_name: String, from_me: bool, timestamp: i64, messages: Vec<Msg> },
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

	/// The messages this row shows - shared, not copied.
	pub fn messages(&self) -> &[Msg] {
		match self {
			DisplayItem::Single(msg) => std::slice::from_ref(msg),
			DisplayItem::Album { messages, .. } => messages,
		}
	}

	pub fn messages_mut(&mut self) -> &mut [Msg] {
		match self {
			DisplayItem::Single(msg) => std::slice::from_mut(msg),
			DisplayItem::Album { messages, .. } => messages,
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
			}
			DisplayItem::Album { messages, .. } => messages.is_empty() || messages.iter().all(|m| m.is_deleted),
		}
	}

	/// Just this item's own content, with no sender/time/status wrapper — see
	/// `ChatMessage::body_text`.
	pub fn body_text(&self) -> String {
		match self {
			DisplayItem::Single(msg) => msg.body_text(),
			DisplayItem::Album { messages, .. } => {
				let photo_count = messages
					.iter()
					.filter(|m| {
						m.media_type == "image" || m.text.starts_with("[Photo]") || m.text.starts_with("[Image]")
					})
					.count();
				let video_count =
					messages.iter().filter(|m| m.media_type == "video" || m.text.starts_with("[Video]")).count();
				let total = messages.len();
				if photo_count > 0 && video_count == 0 {
					format!("Album: {} photos", photo_count)
				} else if video_count > 0 && photo_count == 0 {
					format!("Album: {} videos", video_count)
				} else {
					format!("Album: {} media items", total)
				}
			}
		}
	}

	pub fn format_display(&self, is_group: bool) -> String {
		match self {
			DisplayItem::Single(msg) => msg.format_display(is_group),
			DisplayItem::Album { from_me, sender_name, sender_jid, timestamp, messages, .. } => {
				let formatted_time = if *timestamp > 0 {
					if let Some(local_dt) = LocalTime::from_unix(*timestamp) {
						local_dt.format("%l:%M %p").trim().to_string()
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
					let name = if !is_placeholder_name(sender_name) {
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

/// Changes every message in `messages` that `pick` selects, each one exactly
/// once, and returns the rows of `display` holding a changed message, in
/// order. `display` and `other` are lists grouped from `messages` (the shown
/// rows, and the selection-mode rows).
///
/// A message is one `Rc` shared by all three lists. `Rc::make_mut` on a
/// shared one copies the whole message and leaves the other lists on the
/// stale original, so changing it through each list in turn used to leave two
/// or three copies of every edited, receipted or reacted-to message behind.
/// Here the other lists let go of it first, it changes in place, and they all
/// get it back.
pub fn update_shared_messages(
	messages: &mut [Msg],
	display: &mut [DisplayItem],
	other: &mut [DisplayItem],
	mut pick: impl FnMut(&ChatMessage) -> bool,
	mut change: impl FnMut(&mut ChatMessage),
) -> Vec<usize> {
	use std::{collections::HashMap, rc::Rc};

	let hits: Vec<usize> = messages.iter().enumerate().filter(|(_, m)| pick(m)).map(|(i, _)| i).collect();
	if hits.is_empty() {
		return Vec::new();
	}
	// Keyed by the message's address, which is what every list shares.
	let mut changed: HashMap<*const ChatMessage, Option<Msg>> =
		hits.iter().map(|&i| (Rc::as_ptr(&messages[i]), None)).collect();
	let placeholder = Msg::default();

	let mut rows = Vec::new();
	let mut held: Vec<(*const ChatMessage, &mut Msg)> = Vec::new();
	for (row, item) in display.iter_mut().enumerate() {
		let mut hit = false;
		for slot in item.messages_mut() {
			let ptr = Rc::as_ptr(slot);
			if changed.contains_key(&ptr) {
				*slot = placeholder.clone();
				held.push((ptr, slot));
				hit = true;
			}
		}
		if hit {
			rows.push(row);
		}
	}
	for slot in other.iter_mut().flat_map(DisplayItem::messages_mut) {
		let ptr = Rc::as_ptr(slot);
		if changed.contains_key(&ptr) {
			*slot = placeholder.clone();
			held.push((ptr, slot));
		}
	}

	for &i in &hits {
		let ptr = Rc::as_ptr(&messages[i]);
		change(Rc::make_mut(&mut messages[i]));
		changed.insert(ptr, Some(messages[i].clone()));
	}
	for (ptr, slot) in held {
		if let Some(Some(msg)) = changed.get(&ptr) {
			*slot = msg.clone();
		}
	}
	rows
}

#[cfg(test)]
mod update_shared_messages_tests {
	use super::*;

	fn msg(id: &str, ts: i64, media: &str) -> ChatMessage {
		ChatMessage { id: id.into(), timestamp: ts, media_type: media.into(), ..Default::default() }
	}

	#[test]
	fn a_change_keeps_every_list_on_one_shared_copy() {
		let mut messages = rc(&[msg("a", 1, ""), msg("p1", 100, "image"), msg("p2", 101, "image"), msg("b", 200, "")]);
		let mut display = group_messages_into_display_items(&messages);
		let mut selection = display.clone();
		assert_eq!(display.len(), 3, "the two photos group into an album");

		let rows = update_shared_messages(
			&mut messages,
			&mut display,
			&mut selection,
			|m| m.id == "p2" || m.id == "b",
			|m| m.status = "read".into(),
		);

		assert_eq!(rows, vec![1, 2]);
		for m in &messages {
			assert_eq!(std::rc::Rc::strong_count(m), 3, "{} is shared by all three lists, not copied", m.id);
		}
		assert!(std::rc::Rc::ptr_eq(&messages[2], &display[1].messages()[1]));
		assert!(std::rc::Rc::ptr_eq(&messages[3], &selection[2].messages()[0]));
		assert_eq!(display[1].messages()[1].status, "read");
		assert_eq!(display[2].messages()[0].status, "read");
		assert_eq!(messages[0].status, "", "unpicked messages are untouched");
	}

	#[test]
	fn a_message_held_elsewhere_is_copied_once_and_the_lists_follow_it() {
		let mut messages = rc(&[msg("a", 1, "")]);
		let mut display = group_messages_into_display_items(&messages);
		let snapshot = messages[0].clone();

		update_shared_messages(&mut messages, &mut display, &mut [], |_| true, |m| m.text = "edited".into());

		assert_eq!(snapshot.text, "");
		assert!(std::rc::Rc::ptr_eq(&messages[0], &display[0].messages()[0]));
		assert_eq!(display[0].messages()[0].text, "edited");
	}

	#[test]
	fn nothing_picked_changes_nothing() {
		let mut messages = rc(&[msg("a", 1, "")]);
		let mut display = group_messages_into_display_items(&messages);
		assert!(update_shared_messages(&mut messages, &mut display, &mut [], |_| false, |_| unreachable!()).is_empty());
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

/// Which attach-menu option a batch of files came from. Like WhatsApp's,
/// each one sends differently: Document sends any file as a document (a photo
/// too, at full quality), Photos & videos sends media, Audio sends playable
/// audio files. Files pasted from the clipboard didn't come from the menu, so
/// they're sorted by type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AttachKind {
	#[default]
	Pasted,
	Document,
	Media,
	Audio,
}

/// What WhatsApp Web sends as photos (its image check takes JPEG, PNG, WebP
/// and GIF; a BMP isn't one, so it goes as a document).
const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp"];
/// What WhatsApp Web's Photos & videos picker takes without transcoding.
const VIDEO_EXTS: &[&str] = &["mp4", "3gp", "mov"];
/// WhatsApp Web's Audio picker (WAWebMimeTypes.AUDIO_MIMES), plus the other
/// audio types it sends as audio messages (m4a, opus, amr).
const AUDIO_EXTS: &[&str] = &["mp3", "wav", "ogg", "opus", "aac", "m4a", "amr"];

fn ext_of(path: &str) -> String {
	std::path::Path::new(path).extension().unwrap_or_default().to_string_lossy().to_lowercase()
}

impl AttachKind {
	/// The file-dialog filter for this option.
	pub fn file_filter(self) -> String {
		let pattern = |exts: &[&str]| exts.iter().map(|e| format!("*.{e}")).collect::<Vec<_>>().join(";");
		match self {
			AttachKind::Media => {
				let media: Vec<&str> = IMAGE_EXTS.iter().chain(VIDEO_EXTS).copied().collect();
				format!("Photos and videos|{}", pattern(&media))
			}
			AttachKind::Audio => format!("Audio|{}", pattern(AUDIO_EXTS)),
			AttachKind::Document | AttachKind::Pasted => "All files|*.*".to_string(),
		}
	}

	/// The backend's message type for one file sent this way.
	pub fn send_type(self, path: &str) -> &'static str {
		let ext = ext_of(path);
		let is = |exts: &[&str]| exts.contains(&ext.as_str());
		match self {
			AttachKind::Document => "document",
			AttachKind::Audio => "audio",
			AttachKind::Media | AttachKind::Pasted if is(IMAGE_EXTS) => "image",
			AttachKind::Media | AttachKind::Pasted if is(VIDEO_EXTS) => "video",
			// A pasted file of another kind goes as a document, as before;
			// the Media picker's filter keeps anything else out.
			AttachKind::Media | AttachKind::Pasted => "document",
		}
	}

	/// Audio messages carry no caption in WhatsApp.
	pub fn has_caption(self) -> bool {
		self != AttachKind::Audio
	}
}

/// WhatsApp Web's poll limits (WAWebPollsGatingUtils defaults).
pub const POLL_MAX_QUESTION_CHARS: usize = 255;
pub const POLL_MAX_OPTION_CHARS: usize = 100;
pub const POLL_MAX_OPTIONS: usize = 12;

/// Checks a poll the way WhatsApp's composer does and returns its options
/// (one per non-blank line, trimmed), or what's wrong with it.
pub fn validate_poll(question: &str, options_text: &str) -> Result<Vec<String>, &'static str> {
	let question = question.trim();
	if question.is_empty() {
		return Err("Type a question for the poll.");
	}
	if question.chars().count() > POLL_MAX_QUESTION_CHARS {
		return Err("The question is too long. Polls allow up to 255 characters.");
	}
	let options: Vec<String> =
		options_text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
	if options.len() < 2 {
		return Err("A poll needs at least two options, one per line.");
	}
	if options.len() > POLL_MAX_OPTIONS {
		return Err("A poll can have at most 12 options.");
	}
	if options.iter().any(|o| o.chars().count() > POLL_MAX_OPTION_CHARS) {
		return Err("An option is too long. Each option allows up to 100 characters.");
	}
	for (i, o) in options.iter().enumerate() {
		if options[..i].contains(o) {
			return Err("Two options are the same. Each option must be different.");
		}
	}
	Ok(options)
}

/// The voice message to play automatically once `current_id` finishes, if
/// any - WhatsApp's sequential playback (WAWebPttFindSequentialMsg): only the
/// message straight after it counts, and only if that is itself a voice note
/// (not an audio file). Anything else in between - text, a photo, a call row -
/// ends the run. Date and unread dividers are ours, not messages, so they're
/// looked past.
pub fn next_sequential_voice<'a>(messages: &'a [Msg], current_id: &str) -> Option<&'a ChatMessage> {
	let pos = messages.iter().position(|m| m.id == current_id)?;
	let next: &ChatMessage = messages[pos + 1..].iter().find(|m| !m.is_date_divider && !m.is_unread_divider)?;
	let is_voice_note = next.media_type == "voice" || next.text.starts_with("[Voice Message]");
	(is_voice_note && !next.is_deleted).then_some(next)
}

pub fn group_messages_into_display_items(messages: &[Msg]) -> Vec<DisplayItem> {
	let mut items = Vec::with_capacity(messages.len());
	let mut current_album: Vec<Msg> = Vec::new();

	// Turns the album accumulated so far into an item and hands the buffer back
	// empty. Takes ownership of the run rather than cloning it: the old code
	// cloned `current_album` into the Album and then immediately overwrote or
	// cleared it, so every album's messages were allocated twice.
	fn flush(current_album: &mut Vec<Msg>, items: &mut Vec<DisplayItem>) {
		match current_album.len() {
			0 => {}
			1 => items.push(DisplayItem::Single(current_album.remove(0))),
			_ => {
				let album = std::mem::take(current_album);
				let first = &album[0];
				items.push(DisplayItem::Album {
					id: first.id.clone(),
					sender_jid: first.sender_jid.clone(),
					sender_name: first.sender_name.clone(),
					from_me: first.from_me,
					timestamp: first.timestamp,
					messages: album,
				});
			}
		}
		current_album.clear();
	}

	for msg in messages {
		let is_deleted = msg.is_deleted;
		let is_media = !is_deleted
			&& (msg.media_type == "image"
				|| msg.media_type == "video"
				|| msg.text.starts_with("[Photo]")
				|| msg.text.starts_with("[Image]")
				|| msg.text.starts_with("[Video]"));

		if is_media {
			// A media run continues only while consecutive media belong to the
			// same album; anything else closes the run before starting a new one.
			if current_album.last().is_some_and(|prev| same_album(prev, msg)) {
				current_album.push(msg.clone());
			} else {
				flush(&mut current_album, &mut items);
				current_album.push(msg.clone());
			}
		} else {
			flush(&mut current_album, &mut items);
			items.push(DisplayItem::Single(msg.clone()));
		}
	}
	flush(&mut current_album, &mut items);

	items
}

/// The smallest change that turns the displayed rows `old` into `new` after an
/// older page was prepended: drop `remove_leading` rows from the top, then
/// insert `new[..insert]` there. The row `old[remove_leading]` ends up at
/// index `insert`.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct PrependPlan {
	pub remove_leading: usize,
	pub insert: usize,
}

fn is_date_divider_item(item: &DisplayItem) -> bool {
	matches!(item, DisplayItem::Single(m) if m.is_date_divider)
}

/// Works out a [`PrependPlan`], or `None` when `new` isn't just `old` with older
/// rows added on top (an album spanning the seam regrouped, say) and the list
/// has to be rebuilt.
///
/// The seam is the reason this can't just compare tails: `old` always starts
/// with the date divider for its oldest day, and when the older page ends on
/// that same day, `rebuild_date_dividers` moves the divider up above the new
/// page's messages. Matching whole tails failed on that for almost every page,
/// so each one cleared and re-added every row loaded so far.
pub fn prepend_plan(old: &[DisplayItem], new: &[DisplayItem]) -> Option<PrependPlan> {
	// Only date dividers may sit above the first real row being kept.
	let old_first_real = old.iter().position(|d| !is_date_divider_item(d))?;
	if old[..old_first_real].iter().any(|d| !is_date_divider_item(d)) {
		return None;
	}
	let anchor = old[old_first_real].id();
	if anchor.is_empty() {
		return None;
	}
	let k = new.iter().position(|d| d.id() == anchor)?;
	let tail_matches = new.len() - k == old.len() - old_first_real
		&& new[k..].iter().zip(&old[old_first_real..]).all(|(a, b)| a.id() == b.id());
	if !tail_matches {
		return None;
	}
	// The divider right above the anchor is the same day's either way - keep
	// the one already on screen rather than removing and re-adding it.
	let plan = if old_first_real > 0 && k > 0 && is_date_divider_item(&new[k - 1]) {
		PrependPlan { remove_leading: old_first_real - 1, insert: k - 1 }
	} else {
		PrependPlan { remove_leading: old_first_real, insert: k }
	};
	(plan.insert > 0).then_some(plan)
}

/// Strips every existing date-divider row from `messages` and reinserts fresh
/// ones on each calendar-day transition, in one pass over the whole list.
///
/// Building a page's date dividers only ever looked at that one page (see
/// the divider-insertion loop in `websocket_events::apply_conversation_messages`),
/// which is correct for a full chat load but not for prepending an older
/// page onto messages already loaded: the older page's own trailing divider
/// and the existing batch's leading divider can both name the same day (two
/// "Yesterday" rows with nothing but real messages between them). Rebuilding
/// across the merged list instead of patching at the seam sidesteps that
/// case entirely - the divider set only ever depends on where the days
/// actually change, never on which page a message came from.
///
/// `is_unread_divider` rows are left in place and untouched, and don't count
/// as a "message" for day-transition purposes (their timestamp is 0, not a
/// real send time).
pub fn rebuild_date_dividers(messages: Vec<Msg>) -> Vec<Msg> {
	let mut out = Vec::with_capacity(messages.len());
	let mut last_date: Option<Date> = None;
	for msg in messages.into_iter().filter(|m| !m.is_date_divider) {
		if !msg.is_unread_divider
			&& let Some(local_datetime) = LocalTime::from_unix(msg.timestamp)
		{
			let day = date_key(local_datetime);
			if last_date != Some(day) {
				last_date = Some(day);
				out.push(Msg::new(ChatMessage {
					text: format_date_divider_label(local_datetime),
					timestamp: msg.timestamp,
					is_date_divider: true,
					..Default::default()
				}));
			}
		}
		out.push(msg);
	}
	out
}

/// Counts the real messages in `messages` - i.e. excluding date and unread
/// dividers, which are synthetic display rows the backend's `messages` table
/// has no concept of. This is what `get_messages`'s `offset` must be counted
/// against: using `messages.len()` there previously (which does include the
/// divider rows) overcounted the position by one for every divider already
/// inserted, silently skipping that many real messages from every page after
/// the first.
/// The oldest real (non-divider, persisted) message loaded, as the
/// `(before_timestamp, before_id)` cursor the next older page is requested
/// from. Paging by position instead of by offset means a message arriving in
/// between two pages can't shift what the next one returns, and it still works
/// for a window opened around a search hit in the middle of a chat.
pub fn oldest_message_cursor<M: std::ops::Deref<Target = ChatMessage>>(messages: &[M]) -> Option<(i64, String)> {
	messages
		.iter()
		.find(|m| !m.is_date_divider && !m.is_unread_divider && !m.id.is_empty())
		.map(|m| (m.timestamp, m.id.clone()))
}

/// The newest real message loaded, as the `(after_timestamp, after_id)` cursor
/// the next newer page is requested from - the mirror of
/// [`oldest_message_cursor`] for a window that stops short of the latest
/// message.
pub fn newest_message_cursor<M: std::ops::Deref<Target = ChatMessage>>(messages: &[M]) -> Option<(i64, String)> {
	messages
		.iter()
		.rev()
		.find(|m| !m.is_date_divider && !m.is_unread_divider && !m.id.is_empty())
		.map(|m| (m.timestamp, m.id.clone()))
}

pub fn real_message_count<M: std::ops::Deref<Target = ChatMessage>>(messages: &[M]) -> usize {
	messages.iter().filter(|m| !m.is_date_divider && !m.is_unread_divider).count()
}

/// Inserts an "N unread messages" divider before the last `count` non-divider
/// (real) messages in `messages`, e.g. to mark where messages that arrived
/// while the chat was open but unread pick up. No-op if `count` is 0 or
/// exceeds the number of real messages present.
pub fn insert_unread_divider(messages: &mut Vec<Msg>, count: u64) {
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
	messages.insert(insert_at, Msg::new(ChatMessage { text: label, is_unread_divider: true, ..Default::default() }));
}

/// How much noise an archived chat may make when a message arrives in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchivedAlert {
	/// No sound and nothing on screen - what archiving means by default here,
	/// matching WhatsApp's own "keep chats archived" behaviour.
	#[default]
	Silent,
	/// The usual message sound, but no notification.
	SoundOnly,
	/// Alerts exactly like an unarchived chat.
	Full,
}

/// What an incoming message is allowed to do about itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IncomingAlert {
	pub sound: bool,
	pub toast: bool,
}

/// Decides whether a new message may play the background sound and raise a
/// toast. Muting silences a chat outright, archived or not. An unmuted archived
/// chat falls to its own per-chat level, which defaults to silent: archiving is
/// how you say "stop bothering me about this", so an archived chat alerting like
/// any other is the surprising behaviour, not the quiet one.
///
/// The global `sounds_enabled`/`notifications_enabled` settings still apply on
/// top of this - it answers "is this chat allowed to", not "is the app".
pub fn incoming_alert(muted: bool, archived: bool, level: ArchivedAlert) -> IncomingAlert {
	if muted {
		return IncomingAlert { sound: false, toast: false };
	}
	if !archived {
		return IncomingAlert { sound: true, toast: true };
	}
	match level {
		ArchivedAlert::Silent => IncomingAlert { sound: false, toast: false },
		ArchivedAlert::SoundOnly => IncomingAlert { sound: true, toast: false },
		ArchivedAlert::Full => IncomingAlert { sound: true, toast: true },
	}
}

/// Whether Windows is holding its own notifications back, from the two
/// readings `do_not_disturb` takes: `focus_assist` is the live Do Not Disturb
/// / Focus Assist state, and `notification_state` is
/// `SHQueryUserNotificationState`'s answer. `None` means that reading failed.
///
/// Full-screen games and presentation mode count. The legacy quiet-time state
/// counts only when Focus Assist couldn't be read, because the old API
/// predates Focus Assist and goes stale; a Focus Assist reading of off
/// overrides it. `QUNS_BUSY` (2) never counts: it's reported in situations
/// where Windows itself still sounds its notifications. Every doubt answers
/// false, since a wrong false costs one sound and a wrong true silently hides
/// every message.
pub fn windows_is_quieting(focus_assist: Option<bool>, notification_state: Option<i32>) -> bool {
	const QUNS_RUNNING_D3D_FULL_SCREEN: i32 = 3;
	const QUNS_PRESENTATION_MODE: i32 = 4;
	const QUNS_QUIET_TIME: i32 = 6;
	if focus_assist == Some(true) {
		return true;
	}
	match notification_state {
		Some(QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE) => true,
		Some(QUNS_QUIET_TIME) => focus_assist.is_none(),
		_ => false,
	}
}

/// Total unread across every chat that would actually have alerted about it, by
/// the same [`incoming_alert`] rules the "message" event applies. Muted chats
/// and silent archived ones are left out deliberately: this is a summary of what
/// is waiting for attention, not a raw count of everything unopened, and a
/// tooltip advertising chats that are set never to make a sound would contradict
/// the setting that silenced them.
pub fn unread_notification_total(
	unread_counts: &std::collections::HashMap<String, u64>,
	muted_jids: &std::collections::HashSet<String>,
	archived_jids: &std::collections::HashSet<String>,
	archived_levels: &std::collections::HashMap<String, ArchivedAlert>,
) -> u64 {
	unread_counts
		.iter()
		.filter(|(jid, _)| {
			let level = archived_levels.get(*jid).copied().unwrap_or_default();
			let alert = incoming_alert(muted_jids.contains(*jid), archived_jids.contains(*jid), level);
			alert.sound || alert.toast
		})
		.map(|(_, n)| *n)
		.sum()
}

/// The tray icon's accessible name, supplied as the tooltip when the icon is
/// first added to the notification area.
///
/// The shell fixes a notification-area item's name from the tooltip it was added
/// with, and routes every later tooltip change to the item's description
/// instead; a screen reader reads both, in that order. So the name has to be the
/// app's identity and nothing else - install the icon carrying a count and that
/// count is stuck in the announcement forever, however stale it goes
/// ("No unread messages, 7 unread messages").
pub const TRAY_APP_NAME: &str = "whaNTV";

/// Tooltip text for the tray icon, written over it as the unread count changes.
/// A screen reader reads this out after [`TRAY_APP_NAME`] when the icon is
/// focused in the notification area (Tab to it, or Win+B), which makes it the
/// only way to check for new messages without restoring the window - so the
/// count is spelled out in words rather than left to a badge overlay, which is
/// purely visual.
///
/// Carries no app name of its own: it is read as the second half of
/// "whaNTV, 7 unread messages", and naming whaNTV again here is what made the
/// icon stutter its own name.
pub fn tray_tooltip_text(unread: u64) -> String {
	match unread {
		0 => "No unread messages".to_string(),
		1 => "1 unread message".to_string(),
		n => format!("{} unread messages", n),
	}
}

/// How old a message has to be on arrival to count as part of a backlog rather
/// than as live traffic. A live message lands within a second or two of being
/// sent; anything this stale was queued on the server while whaNTV was closed or
/// offline and is now being replayed in a burst.
pub const BACKLOG_MESSAGE_AGE_SECS: i64 = 60;

/// Whether a message stamped `timestamp` (Unix seconds) and arriving at `now`
/// is being replayed from a backlog. A missing timestamp (0) reads as live:
/// alerting once too often beats swallowing a real message.
pub fn is_backlog_message(timestamp: i64, now: i64) -> bool {
	timestamp > 0 && now - timestamp > BACKLOG_MESSAGE_AGE_SECS
}

/// What piled up while whaNTV was away, tallied as the backlog is replayed so
/// it can be answered with one sound and one notification once the replay goes
/// quiet, instead of one of each per message.
///
/// Only things that would have alerted on their own are recorded - a muted
/// chat adds nothing here, same as it adds nothing live.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CatchUpAlerts {
	messages: u32,
	missed_calls: u32,
	chats: std::collections::HashSet<String>,
	sound: bool,
	toast: bool,
}

impl CatchUpAlerts {
	/// Adds one replayed message from `jid`, which was allowed the given alert.
	pub fn record_message(&mut self, jid: &str, alert: IncomingAlert) {
		if !alert.sound && !alert.toast {
			return;
		}
		self.messages += 1;
		self.note(jid, alert);
	}

	/// Adds one replayed missed call from `jid`, which was allowed the given alert.
	pub fn record_missed_call(&mut self, jid: &str, alert: IncomingAlert) {
		if !alert.sound && !alert.toast {
			return;
		}
		self.missed_calls += 1;
		self.note(jid, alert);
	}

	fn note(&mut self, jid: &str, alert: IncomingAlert) {
		self.chats.insert(jid.to_string());
		self.sound |= alert.sound;
		self.toast |= alert.toast;
	}

	pub fn is_empty(&self) -> bool {
		self.messages == 0 && self.missed_calls == 0
	}

	/// Whether anything recorded was allowed to make a sound.
	pub fn sound(&self) -> bool {
		self.sound
	}

	/// Whether anything recorded was allowed to raise a notification.
	pub fn toast(&self) -> bool {
		self.toast
	}

	/// One line for the single notification, e.g.
	/// "12 new messages in 3 chats and 2 missed calls".
	pub fn summary(&self) -> String {
		let mut segments = Vec::new();
		if self.messages > 0 {
			let messages = match self.messages {
				1 => "1 new message".to_string(),
				n => format!("{} new messages", n),
			};
			// With calls in the tally the chat count covers both, so it would
			// misdescribe the messages alone.
			if self.missed_calls == 0 && self.chats.len() > 1 {
				segments.push(format!("{} in {} chats", messages, self.chats.len()));
			} else {
				segments.push(messages);
			}
		}
		match self.missed_calls {
			0 => {}
			1 => segments.push("1 missed call".to_string()),
			n => segments.push(format!("{} missed calls", n)),
		}
		segments.join(" and ")
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

		let voice_reply_no_pushname = ChatMessage {
			id: "4".into(),
			text: "Yes you are right".into(),
			timestamp: 0,
			from_me: false,
			sender_jid: "919800000003@s.whatsapp.net".into(),
			sender_name: "Sam Rivers".into(),
			is_date_divider: false,
			is_deleted: false,
			status: "".into(),
			sent_at: 0,
			delivered_at: 0,
			read_at: 0,
			played_at: 0,
			reactions: Vec::new(),
			quoted_id: "q2".into(),
			quoted_text: "[Voice Message] (0:49)".into(),
			quoted_sender: "[]".into(),
			..Default::default()
		};
		assert_eq!(
			voice_reply_no_pushname.format_display(true),
			"Sam Rivers: Yes you are right, replied to quoted message: \"Voice message Duration: 0:49\""
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

	/// Covers the field mapping itself - the camelCase renames and the
	/// snake_case aliases the backend may send either of - independently of
	/// which deserializer the caller happens to use.
	#[test]
	fn test_chat_message_field_aliases_and_renames() {
		use serde::Deserialize;

		let json = r#"{"messages":[
			{"id":"m1","text":"hi","timestamp":1700000000,"fromMe":true,"senderJid":"a@s.whatsapp.net","isAudio":true,"mediaType":"audio","albumId":"alb1","sentAt":17,"reactions":[{"reactorJid":"b@s.whatsapp.net","emoji":"x"}]},
			{"id":"m2","text":"yo","timestamp":1700000001,"from_me":false,"sender_jid":"b@s.whatsapp.net","media_type":"image","album_id":"alb2"}
		]}"#;
		let val: serde_json::Value = serde_json::from_str(json).unwrap();
		let arr = val.get("messages").and_then(|m| m.as_array()).unwrap();
		let parsed: Vec<ChatMessage> = arr.iter().map(|m| ChatMessage::deserialize(m).unwrap()).collect();

		assert_eq!(parsed.len(), 2);
		assert_eq!(parsed[0].id, "m1");
		assert!(parsed[0].from_me);
		assert!(parsed[0].is_audio);
		assert_eq!(parsed[0].sender_jid, "a@s.whatsapp.net");
		assert_eq!(parsed[0].media_type, "audio");
		assert_eq!(parsed[0].album_id, "alb1");
		assert_eq!(parsed[0].sent_at, 17);
		assert_eq!(parsed[0].reactions.len(), 1);
		assert_eq!(parsed[0].reactions[0].emoji, "x");
		// Fields absent from the payload fall back to Default, not an error.
		assert_eq!(parsed[0].quoted_id, "");
		assert_eq!(parsed[0].read_at, 0);

		// snake_case aliases resolve the same way.
		assert_eq!(parsed[1].id, "m2");
		assert!(!parsed[1].from_me);
		assert_eq!(parsed[1].sender_jid, "b@s.whatsapp.net");
		assert_eq!(parsed[1].media_type, "image");
		assert_eq!(parsed[1].album_id, "alb2");
	}

	#[test]
	fn test_unread_notification_total_excludes_muted() {
		let mut counts = std::collections::HashMap::new();
		counts.insert("a@s.whatsapp.net".to_string(), 3u64);
		counts.insert("b@s.whatsapp.net".to_string(), 5u64);
		counts.insert("c@g.us".to_string(), 7u64);
		let mut muted = std::collections::HashSet::new();
		muted.insert("c@g.us".to_string());
		let none = std::collections::HashSet::new();
		let no_levels = std::collections::HashMap::new();
		assert_eq!(unread_notification_total(&counts, &muted, &none, &no_levels), 8);

		// Every chat muted, and the read chats a chats broadcast leaves sitting
		// at zero, both come out as "nothing waiting" rather than a stray count.
		muted.insert("a@s.whatsapp.net".to_string());
		muted.insert("b@s.whatsapp.net".to_string());
		assert_eq!(unread_notification_total(&counts, &muted, &none, &no_levels), 0);
		let zeroed: std::collections::HashMap<String, u64> =
			[("a@s.whatsapp.net".to_string(), 0u64)].into_iter().collect();
		assert_eq!(unread_notification_total(&zeroed, &none, &none, &no_levels), 0);
	}

	#[test]
	fn test_unread_notification_total_respects_archived_levels() {
		let counts: std::collections::HashMap<String, u64> = [
			("silent@s.whatsapp.net".to_string(), 2u64),
			("sound@s.whatsapp.net".to_string(), 3u64),
			("full@s.whatsapp.net".to_string(), 4u64),
		]
		.into_iter()
		.collect();
		let archived: std::collections::HashSet<String> = counts.keys().cloned().collect();
		let muted = std::collections::HashSet::new();

		// Archived defaults to silent, so an archived chat with no stored level
		// contributes nothing at all.
		assert_eq!(unread_notification_total(&counts, &muted, &archived, &std::collections::HashMap::new()), 0);

		// Sound-only still alerts, so it still counts; only Silent drops out.
		let levels: std::collections::HashMap<String, ArchivedAlert> = [
			("sound@s.whatsapp.net".to_string(), ArchivedAlert::SoundOnly),
			("full@s.whatsapp.net".to_string(), ArchivedAlert::Full),
		]
		.into_iter()
		.collect();
		assert_eq!(unread_notification_total(&counts, &muted, &archived, &levels), 7);

		// Muting beats a per-chat level that would otherwise have alerted.
		let muted_full: std::collections::HashSet<String> = ["full@s.whatsapp.net".to_string()].into_iter().collect();
		assert_eq!(unread_notification_total(&counts, &muted_full, &archived, &levels), 3);
	}

	#[test]
	fn quieting_follows_focus_assist_first() {
		assert!(windows_is_quieting(Some(true), None));
		assert!(windows_is_quieting(Some(true), Some(5)));
		assert!(!windows_is_quieting(Some(false), None));
		assert!(!windows_is_quieting(None, None));
	}

	#[test]
	fn quieting_counts_full_screen_and_presentations() {
		assert!(windows_is_quieting(Some(false), Some(3)));
		assert!(windows_is_quieting(Some(false), Some(4)));
		assert!(windows_is_quieting(None, Some(4)));
	}

	#[test]
	fn legacy_quiet_time_yields_to_a_live_focus_assist_reading() {
		assert!(windows_is_quieting(None, Some(6)));
		assert!(!windows_is_quieting(Some(false), Some(6)));
	}

	#[test]
	fn busy_and_normal_states_never_quiet() {
		for state in [1, 2, 5, 7] {
			assert!(!windows_is_quieting(None, Some(state)));
			assert!(!windows_is_quieting(Some(false), Some(state)));
		}
	}

	#[test]
	fn test_incoming_alert() {
		// Unarchived chats are unaffected by the archived level entirely.
		for level in [ArchivedAlert::Silent, ArchivedAlert::SoundOnly, ArchivedAlert::Full] {
			assert_eq!(incoming_alert(false, false, level), IncomingAlert { sound: true, toast: true });
		}
		// Muting wins over every archived level, including Full.
		for level in [ArchivedAlert::Silent, ArchivedAlert::SoundOnly, ArchivedAlert::Full] {
			assert_eq!(incoming_alert(true, true, level), IncomingAlert { sound: false, toast: false });
			assert_eq!(incoming_alert(true, false, level), IncomingAlert { sound: false, toast: false });
		}
		assert_eq!(incoming_alert(false, true, ArchivedAlert::default()), IncomingAlert { sound: false, toast: false });
		assert_eq!(incoming_alert(false, true, ArchivedAlert::SoundOnly), IncomingAlert { sound: true, toast: false });
		assert_eq!(incoming_alert(false, true, ArchivedAlert::Full), IncomingAlert { sound: true, toast: true });
	}

	#[test]
	fn test_archived_alert_serde_roundtrip() {
		// settings.json holds these by name - renaming a variant would silently
		// reset every archived chat to the default on next load.
		assert_eq!(serde_json::to_string(&ArchivedAlert::Silent).unwrap(), "\"silent\"");
		assert_eq!(serde_json::to_string(&ArchivedAlert::SoundOnly).unwrap(), "\"sound_only\"");
		assert_eq!(serde_json::to_string(&ArchivedAlert::Full).unwrap(), "\"full\"");
		assert_eq!(serde_json::from_str::<ArchivedAlert>("\"sound_only\"").unwrap(), ArchivedAlert::SoundOnly);
		assert_eq!(ArchivedAlert::default(), ArchivedAlert::Silent);
	}

	#[test]
	fn test_is_backlog_message() {
		assert!(!is_backlog_message(1_000, 1_000));
		assert!(!is_backlog_message(1_000, 1_000 + BACKLOG_MESSAGE_AGE_SECS));
		assert!(is_backlog_message(1_000, 1_001 + BACKLOG_MESSAGE_AGE_SECS));
		// A clock running behind the sender's, or no timestamp at all, is live.
		assert!(!is_backlog_message(2_000, 1_000));
		assert!(!is_backlog_message(0, 1_000_000));
	}

	#[test]
	fn test_catch_up_alerts_summary() {
		let full = IncomingAlert { sound: true, toast: true };
		let silent = IncomingAlert { sound: false, toast: false };
		let mut tally = CatchUpAlerts::default();
		assert!(tally.is_empty());

		tally.record_message("muted@g.us", silent);
		tally.record_missed_call("muted@g.us", silent);
		assert!(tally.is_empty());

		tally.record_message("a@s.whatsapp.net", IncomingAlert { sound: true, toast: false });
		assert_eq!(tally.summary(), "1 new message");
		assert!(tally.sound() && !tally.toast());

		tally.record_message("a@s.whatsapp.net", full);
		assert_eq!(tally.summary(), "2 new messages");
		tally.record_message("b@g.us", full);
		assert_eq!(tally.summary(), "3 new messages in 2 chats");
		assert!(tally.toast());

		tally.record_missed_call("c@s.whatsapp.net", full);
		assert_eq!(tally.summary(), "3 new messages and 1 missed call");

		let mut calls = CatchUpAlerts::default();
		calls.record_missed_call("c@s.whatsapp.net", full);
		calls.record_missed_call("c@s.whatsapp.net", full);
		assert_eq!(calls.summary(), "2 missed calls");
	}

	#[test]
	fn test_tray_tooltip_text() {
		assert_eq!(tray_tooltip_text(0), "No unread messages");
		assert_eq!(tray_tooltip_text(1), "1 unread message");
		assert_eq!(tray_tooltip_text(12), "12 unread messages");
		// TRAY_APP_NAME is announced ahead of this string, so repeating the name
		// here is what made the icon stutter it.
		for n in [0, 1, 12] {
			assert!(!tray_tooltip_text(n).contains(TRAY_APP_NAME));
		}
	}

	#[test]
	fn test_failed_message_display_format() {
		let failed_msg = ChatMessage {
			id: "fail1".into(),
			text: "[Photo] photo.jpg".into(),
			timestamp: 1700000000,
			from_me: true,
			sender_name: "You".into(),
			status: "failed".into(),
			media_type: "image".into(),
			media_filename: "photo.jpg".into(),
			media_path: "C:\\some\\file.jpg".into(),
			..Default::default()
		};
		let display = failed_msg.format_display(false);
		assert!(display.contains("Failed to send"), "Expected 'Failed to send' in '{}'", display);
	}

	fn media(id: &str, album_id: &str, sender: &str, ts: i64) -> ChatMessage {
		ChatMessage {
			id: id.into(),
			text: "[Photo]".into(),
			timestamp: ts,
			sender_jid: sender.into(),
			media_type: "image".into(),
			album_id: album_id.into(),
			..Default::default()
		}
	}

	fn text(id: &str, body: &str) -> ChatMessage {
		ChatMessage { id: id.into(), text: body.into(), ..Default::default() }
	}

	fn ids(items: &[DisplayItem]) -> Vec<Vec<String>> {
		items
			.iter()
			.map(|item| match item {
				DisplayItem::Single(m) => vec![m.id.clone()],
				DisplayItem::Album { messages, .. } => messages.iter().map(|m| m.id.clone()).collect(),
			})
			.collect()
	}

	#[test]
	fn grouping_keeps_lone_media_and_plain_text_as_singles() {
		let msgs = vec![text("a", "hi"), media("b", "", "x@s.whatsapp.net", 100), text("c", "bye")];
		assert_eq!(ids(&group_messages_into_display_items(&rc(&msgs))), vec![vec!["a"], vec!["b"], vec!["c"]]);
	}

	#[test]
	fn grouping_collects_a_run_of_same_album_media() {
		let msgs = vec![
			media("a", "alb1", "x@s.whatsapp.net", 100),
			media("b", "alb1", "x@s.whatsapp.net", 101),
			media("c", "alb1", "x@s.whatsapp.net", 102),
		];
		let items = group_messages_into_display_items(&rc(&msgs));
		assert_eq!(ids(&items), vec![vec!["a", "b", "c"]]);
		// The album's identity comes from its first message.
		match &items[0] {
			DisplayItem::Album { id, sender_jid, timestamp, .. } => {
				assert_eq!(id, "a");
				assert_eq!(sender_jid, "x@s.whatsapp.net");
				assert_eq!(*timestamp, 100);
			}
			DisplayItem::Single(_) => panic!("expected an album"),
		}
	}

	#[test]
	fn grouping_splits_adjacent_albums_and_flushes_before_text() {
		let msgs = vec![
			media("a", "alb1", "x@s.whatsapp.net", 100),
			media("b", "alb1", "x@s.whatsapp.net", 101),
			media("c", "alb2", "x@s.whatsapp.net", 102),
			media("d", "alb2", "x@s.whatsapp.net", 103),
			text("e", "look"),
			media("f", "alb3", "x@s.whatsapp.net", 200),
		];
		assert_eq!(
			ids(&group_messages_into_display_items(&rc(&msgs))),
			vec![vec!["a", "b"], vec!["c", "d"], vec!["e"], vec!["f"]]
		);
	}

	#[test]
	fn grouping_falls_back_to_the_ten_second_window_without_album_ids() {
		// No album_id on either side: same sender within 10s groups, beyond it splits.
		let msgs = vec![
			media("a", "", "x@s.whatsapp.net", 100),
			media("b", "", "x@s.whatsapp.net", 105),
			media("c", "", "x@s.whatsapp.net", 200),
		];
		assert_eq!(ids(&group_messages_into_display_items(&rc(&msgs))), vec![vec!["a", "b"], vec!["c"]]);
	}

	#[test]
	fn grouping_never_merges_across_senders() {
		let msgs = vec![media("a", "", "x@s.whatsapp.net", 100), media("b", "", "y@s.whatsapp.net", 101)];
		assert_eq!(ids(&group_messages_into_display_items(&rc(&msgs))), vec![vec!["a"], vec!["b"]]);
	}

	#[test]
	fn grouping_of_nothing_yields_nothing() {
		assert!(group_messages_into_display_items(&[]).is_empty());
	}
}

#[cfg(test)]
mod attach_and_poll_tests {
	use super::*;

	#[test]
	fn each_attach_option_sends_its_own_way() {
		assert_eq!(AttachKind::Document.send_type("C:/a/photo.JPG"), "document");
		assert_eq!(AttachKind::Media.send_type("C:/a/photo.JPG"), "image");
		assert_eq!(AttachKind::Media.send_type("clip.mp4"), "video");
		assert_eq!(AttachKind::Audio.send_type("song.mp3"), "audio");
		assert_eq!(AttachKind::Pasted.send_type("notes.txt"), "document");
		assert_eq!(AttachKind::Pasted.send_type("shot.png"), "image");
		assert!(!AttachKind::Audio.has_caption());
		assert!(AttachKind::Media.file_filter().contains("*.mov"));
		assert!(AttachKind::Audio.file_filter().starts_with("Audio|*.mp3"));
	}

	#[test]
	fn poll_results_follow_the_question() {
		let poll = ChatMessage {
			text: "[Poll] Lunch?".into(),
			poll_results: "Yes 50%, 1 vote; No 50%, 1 vote. 2 people voted.".into(),
			..Default::default()
		};
		let body = poll.body_text();
		assert_eq!(body, "Poll: Lunch? Yes 50%, 1 vote; No 50%, 1 vote. 2 people voted.");
		assert!(poll.is_poll());
	}

	#[test]
	fn quiz_and_event_results_follow_them() {
		let quiz = ChatMessage {
			text: "[Quiz] Capital of France?".into(),
			poll_results: "Options: Lyon, Paris. No votes yet.".into(),
			..Default::default()
		};
		assert!(quiz.is_poll());
		assert_eq!(quiz.body_text(), "Quiz: Capital of France? Options: Lyon, Paris. No votes yet.");
		let event = ChatMessage {
			text: "[Event] Picnic\nSat 3 Oct 2026, 12:00".into(),
			poll_results: "2 going, 0 maybe, 1 can't go.".into(),
			..Default::default()
		};
		assert!(event.is_event() && !event.is_poll());
		assert_eq!(event.body_text(), "[Event] Picnic\nSat 3 Oct 2026, 12:00. 2 going, 0 maybe, 1 can't go.");
	}

	#[test]
	fn poll_options_come_one_per_line() {
		assert_eq!(validate_poll(" Lunch? ", "Pizza\r\n\n  Pasta  \n"), Ok(vec!["Pizza".into(), "Pasta".into()]));
	}

	#[test]
	fn polls_are_checked_like_whatsapp() {
		let lines = |n: usize| (1..=n).map(|n| n.to_string()).collect::<Vec<_>>().join("\n");
		assert!(validate_poll("", "a\nb").is_err());
		assert!(validate_poll("Q", "a").is_err());
		assert!(validate_poll("Q", "a\na").is_err());
		assert!(validate_poll("Q", &lines(13)).is_err());
		assert!(validate_poll(&"q".repeat(256), "a\nb").is_err());
		assert!(validate_poll("Q", &format!("a\n{}", "o".repeat(101))).is_err());
		assert!(validate_poll(&"q".repeat(255), &lines(12)).is_ok());
	}
}

#[cfg(test)]
mod sequential_voice_tests {
	use super::*;

	fn msg(id: &str, text: &str, media_type: &str) -> ChatMessage {
		ChatMessage { id: id.into(), text: text.into(), media_type: media_type.into(), ..Default::default() }
	}

	#[test]
	fn plays_the_voice_note_straight_after() {
		let m = vec![msg("a", "[Voice Message] (0:05)", "voice"), msg("b", "[Voice Message] (0:03)", "voice")];
		assert_eq!(next_sequential_voice(&rc(&m), "a").map(|m| m.id.as_str()), Some("b"));
		assert!(next_sequential_voice(&rc(&m), "b").is_none());
	}

	#[test]
	fn anything_in_between_ends_the_run() {
		let m = vec![
			msg("a", "[Voice Message] (0:05)", "voice"),
			msg("t", "ok", ""),
			msg("b", "[Voice Message] (0:03)", "voice"),
		];
		assert!(next_sequential_voice(&rc(&m), "a").is_none());
	}

	#[test]
	fn audio_files_and_deleted_notes_are_not_played() {
		let mut deleted = msg("d", "[Voice Message] (0:03)", "voice");
		deleted.is_deleted = true;
		let m = vec![msg("a", "[Voice Message] (0:05)", "voice"), msg("f", "[Audio] (3:10)", "audio")];
		assert!(next_sequential_voice(&rc(&m), "a").is_none());
		let m = vec![msg("a", "[Voice Message] (0:05)", "voice"), deleted];
		assert!(next_sequential_voice(&rc(&m), "a").is_none());
	}

	#[test]
	fn looks_past_dividers() {
		let divider = ChatMessage { text: "Today".into(), is_date_divider: true, ..Default::default() };
		let unread = ChatMessage { text: "1 unread".into(), is_unread_divider: true, ..Default::default() };
		let m = vec![
			msg("a", "[Voice Message] (0:05)", "voice"),
			divider,
			unread,
			msg("b", "[Voice Message] (0:03)", "voice"),
		];
		assert_eq!(next_sequential_voice(&rc(&m), "a").map(|m| m.id.as_str()), Some("b"));
	}
}

#[cfg(test)]
mod call_row_tests {
	use super::ChatMessage;

	/// A call-history row is recognised by its media_type alone, so Enter on it
	/// calls back instead of falling through to the list's default.
	#[test]
	fn call_rows_are_recognised_and_nothing_else_is() {
		let call = ChatMessage { media_type: "call".to_string(), ..Default::default() };
		assert!(call.is_call());
		assert!(!call.is_voice(), "a call row must not be mistaken for a voice message");

		for other in ["", "system", "image", "audio", "document"] {
			let msg = ChatMessage { media_type: other.to_string(), ..Default::default() };
			assert!(!msg.is_call(), "media_type {other:?} must not read as a call row");
		}
	}

	/// The backend sends media_type; the alias keeps the camelCase form working.
	#[test]
	fn call_rows_parse_from_either_spelling() {
		let bodies = [
			r#"{"id":"call:abc","text":"Missed voice call","timestamp":1,"media_type":"call"}"#,
			r#"{"id":"call:abc","text":"Missed voice call","timestamp":1,"mediaType":"call"}"#,
		];
		for body in bodies {
			let msg: ChatMessage = serde_json::from_str(body).expect("parse");
			assert!(msg.is_call(), "{body} should parse as a call row");
			assert_eq!(msg.text, "Missed voice call");
		}
	}
}

#[cfg(test)]
mod prepend_plan_tests {
	use super::*;

	fn msg(id: &str) -> DisplayItem {
		DisplayItem::Single(Msg::new(ChatMessage { id: id.to_string(), text: id.to_string(), ..Default::default() }))
	}

	fn day(label: &str) -> DisplayItem {
		DisplayItem::Single(Msg::new(ChatMessage {
			text: label.to_string(),
			is_date_divider: true,
			..Default::default()
		}))
	}

	#[test]
	fn older_page_ending_on_an_earlier_day_keeps_the_old_divider() {
		let old = vec![day("Tue"), msg("c"), msg("d")];
		let new = vec![day("Mon"), msg("a"), msg("b"), day("Tue"), msg("c"), msg("d")];
		assert_eq!(prepend_plan(&old, &new), Some(PrependPlan { remove_leading: 0, insert: 3 }));
	}

	#[test]
	fn older_page_ending_on_the_same_day_moves_the_divider_up() {
		// The case the old whole-tail comparison missed on nearly every page.
		let old = vec![day("Tue"), msg("c"), msg("d")];
		let new = vec![day("Mon"), msg("a"), day("Tue"), msg("b"), msg("c"), msg("d")];
		assert_eq!(prepend_plan(&old, &new), Some(PrependPlan { remove_leading: 1, insert: 4 }));
	}

	#[test]
	fn applying_the_plan_reproduces_the_new_rows() {
		let old = vec![day("Tue"), msg("c"), msg("d")];
		let new = vec![day("Mon"), msg("a"), day("Tue"), msg("b"), msg("c"), msg("d")];
		let plan = prepend_plan(&old, &new).unwrap();
		let mut rows: Vec<String> = old.iter().map(|d| d.format_display(false)).collect();
		rows.drain(..plan.remove_leading);
		for (i, item) in new[..plan.insert].iter().enumerate() {
			rows.insert(i, item.format_display(false));
		}
		let expected: Vec<String> = new.iter().map(|d| d.format_display(false)).collect();
		assert_eq!(rows, expected);
	}

	#[test]
	fn regrouped_seam_falls_back_to_a_rebuild() {
		let old = vec![day("Tue"), msg("c"), msg("d")];
		let new = vec![day("Tue"), msg("b"), msg("x"), msg("d")];
		assert_eq!(prepend_plan(&old, &new), None);
	}

	#[test]
	fn nothing_new_is_not_a_prepend() {
		let old = vec![day("Tue"), msg("c")];
		assert_eq!(prepend_plan(&old, &old.clone()), None);
	}
}

/// Orders the Communities tab the way the official apps lay it out: each
/// community, followed by the groups in it that we're in, its announcements
/// group first. `chats` is the chat list in its usual order, which the
/// communities and the groups under each keep. Groups whose community isn't
/// in the list are left out.
pub fn community_tab_order<'a>(
	chats: &'a [String],
	communities: &std::collections::HashSet<String>,
	parents: &std::collections::HashMap<String, String>,
	announcements: &std::collections::HashSet<String>,
) -> Vec<&'a String> {
	let mut children: std::collections::HashMap<&str, Vec<&String>> = std::collections::HashMap::new();
	for jid in chats {
		if let Some(parent) = parents.get(jid)
			&& communities.contains(parent)
		{
			children.entry(parent.as_str()).or_default().push(jid);
		}
	}
	let mut order = Vec::new();
	for jid in chats.iter().filter(|j| communities.contains(*j)) {
		order.push(jid);
		if let Some(groups) = children.get_mut(jid.as_str()) {
			groups.sort_by_key(|g| !announcements.contains(*g));
			order.extend(groups.iter().copied());
		}
	}
	order
}

#[cfg(test)]
mod community_tab_order_tests {
	use super::community_tab_order;

	fn strings(v: &[&str]) -> Vec<String> {
		v.iter().map(|s| s.to_string()).collect()
	}

	fn order(
		chats: &[String],
		communities: &std::collections::HashSet<String>,
		parents: &std::collections::HashMap<String, String>,
		announcements: &std::collections::HashSet<String>,
	) -> Vec<String> {
		community_tab_order(chats, communities, parents, announcements).into_iter().cloned().collect()
	}

	#[test]
	fn groups_follow_their_community_with_announcements_first() {
		let chats = strings(&["g1", "c2", "g2", "ann1", "c1", "dm", "ann2"]);
		let communities = ["c1", "c2"].iter().map(|s| s.to_string()).collect();
		let parents = [("g1", "c1"), ("ann1", "c1"), ("g2", "c2"), ("ann2", "c2")]
			.iter()
			.map(|(a, b)| (a.to_string(), b.to_string()))
			.collect();
		let announcements = ["ann1", "ann2"].iter().map(|s| s.to_string()).collect();
		assert_eq!(
			order(&chats, &communities, &parents, &announcements),
			strings(&["c2", "ann2", "g2", "c1", "ann1", "g1"])
		);
	}

	#[test]
	fn groups_of_an_unknown_community_are_left_out() {
		let chats = strings(&["g1", "c1"]);
		let communities = ["c1"].iter().map(|s| s.to_string()).collect();
		let parents = [("g1", "elsewhere")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
		let announcements = Default::default();
		assert_eq!(order(&chats, &communities, &parents, &announcements), strings(&["c1"]));
	}
}

/// One row of a community's Groups list: the group's name and what it is to
/// us, so a screen reader hears whether Enter will open it or join it.
pub fn community_group_label(
	name: &str,
	announcements: bool,
	general_chat: bool,
	joined: bool,
	participant_count: u64,
) -> String {
	let mut parts = vec![if name.is_empty() { "Unnamed group".to_string() } else { name.to_string() }];
	if announcements {
		parts.push("announcements".to_string());
	} else if general_chat {
		parts.push("general chat".to_string());
	}
	parts.push(if joined { "joined" } else { "not joined" }.to_string());
	match participant_count {
		0 => {}
		1 => parts.push("1 member".to_string()),
		n => parts.push(format!("{} members", n)),
	}
	parts.join(", ")
}

#[cfg(test)]
mod community_group_label_tests {
	use super::community_group_label;

	#[test]
	fn says_what_the_group_is_and_whether_we_are_in_it() {
		assert_eq!(community_group_label("NAB", true, false, true, 0), "NAB, announcements, joined");
		assert_eq!(community_group_label("Class 11", false, false, false, 42), "Class 11, not joined, 42 members");
		assert_eq!(community_group_label("", false, true, true, 1), "Unnamed group, general chat, joined, 1 member");
	}
}

/// A WhatsApp link we can act on: the whatsapp:// links that "Share on
/// WhatsApp" buttons and wa.me pages open, and group invite links.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WhatsAppLink {
	/// Send a message. With a phone number, the chat with that number opens
	/// with the text ready to send; without one, the user picks who to send
	/// it to.
	Send { phone: Option<String>, text: String },
	/// Join a group from its invite code.
	Invite { code: String },
	/// Look at (and maybe follow) a channel from its invite code.
	Channel { code: String },
	/// Join a call from its call link: call.whatsapp.com/voice/<token>, or
	/// /audio/ as whatsapp-rust and meowcaller write it.
	Call { token: String, video: bool },
}

/// Understands whatsapp://send, whatsapp://chat?code=, wa.me/<number>,
/// api.whatsapp.com/send, web.whatsapp.com/send, chat.whatsapp.com/<code>
/// and call.whatsapp.com/<voice|video>/<token> links. Returns None for anything else.
pub fn parse_whatsapp_link(link: &str) -> Option<WhatsAppLink> {
	let link = link.trim().trim_matches('"');
	let (scheme, rest) = link.split_once("://")?;
	let scheme = scheme.to_ascii_lowercase();
	let (location, query) = rest.split_once('?').unwrap_or((rest, ""));
	let location = location.split('#').next().unwrap_or("").trim_end_matches('/');
	let query = query.split('#').next().unwrap_or("");
	let param = |name: &str| {
		query.split('&').find_map(|pair| {
			let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
			(key.eq_ignore_ascii_case(name)).then(|| percent_decode_query(value))
		})
	};
	let (host, path) = location.split_once('/').unwrap_or((location, ""));
	let host = host.to_ascii_lowercase();
	let host = host.strip_prefix("www.").unwrap_or(&host);
	let path = path.trim_matches('/');

	let send = |phone: Option<String>| {
		let phone = phone.map(|p| p.chars().filter(char::is_ascii_digit).collect::<String>()).filter(|p| !p.is_empty());
		let text = param("text").unwrap_or_default();
		(phone.is_some() || !text.is_empty()).then_some(WhatsAppLink::Send { phone, text })
	};
	let invite = |code: String| (!code.is_empty()).then_some(WhatsAppLink::Invite { code });

	match (scheme.as_str(), host) {
		("whatsapp", "send") => send(param("phone")),
		("whatsapp", "chat") => invite(param("code")?),
		("whatsapp", "channel") => {
			let code = param("code").unwrap_or_else(|| path.split('/').next().unwrap_or("").to_string());
			(!code.is_empty()).then_some(WhatsAppLink::Channel { code })
		}
		("http" | "https", "wa.me") if path.is_empty() => send(param("phone")),
		("http" | "https", "wa.me") => send(Some(path.to_string())),
		("http" | "https", "api.whatsapp.com" | "web.whatsapp.com") if path == "send" => send(param("phone")),
		("http" | "https", "whatsapp.com") if path.starts_with("channel/") => {
			let code = path["channel/".len()..].split('/').next().unwrap_or("").to_string();
			(!code.is_empty()).then_some(WhatsAppLink::Channel { code })
		}
		("https", "call.whatsapp.com") => {
			let (media, token) = path.split_once('/')?;
			let video = match media.to_ascii_lowercase().as_str() {
				"voice" | "audio" => false,
				"video" => true,
				_ => return None,
			};
			(!token.is_empty() && !token.contains('/')).then(|| WhatsAppLink::Call { token: token.to_string(), video })
		}
		("http" | "https", "chat.whatsapp.com") => {
			invite(param("code").unwrap_or_else(|| path.strip_prefix("invite/").unwrap_or(path).to_string()))
		}
		_ => None,
	}
}

/// Decodes a URL query value: %XX escapes (as UTF-8) and + for a space.
fn percent_decode_query(value: &str) -> String {
	let bytes = value.as_bytes();
	let mut out = Vec::with_capacity(bytes.len());
	let mut i = 0;
	while i < bytes.len() {
		match bytes[i] {
			b'+' => out.push(b' '),
			b'%' => {
				let escape = bytes.get(i + 1..i + 3).and_then(|hex| std::str::from_utf8(hex).ok());
				match escape.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
					Some(byte) => {
						out.push(byte);
						i += 3;
						continue;
					}
					None => out.push(b'%'),
				}
			}
			byte => out.push(byte),
		}
		i += 1;
	}
	String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod whatsapp_link_tests {
	use super::{WhatsAppLink, parse_whatsapp_link};

	fn send(phone: Option<&str>, text: &str) -> Option<WhatsAppLink> {
		Some(WhatsAppLink::Send { phone: phone.map(str::to_string), text: text.to_string() })
	}

	#[test]
	fn share_links() {
		assert_eq!(parse_whatsapp_link("whatsapp://send?text=Hello%20there%21"), send(None, "Hello there!"));
		assert_eq!(parse_whatsapp_link("whatsapp://send/?text=a+b&phone=+91%2098100"), send(Some("9198100"), "a b"));
		assert_eq!(
			parse_whatsapp_link("https://api.whatsapp.com/send?phone=919810012345&text=Hi"),
			send(Some("919810012345"), "Hi")
		);
		assert_eq!(parse_whatsapp_link("https://wa.me/919810012345?text=%E0%A4%A8"), send(Some("919810012345"), "न"));
		assert_eq!(parse_whatsapp_link("https://wa.me/?text=news"), send(None, "news"));
		assert_eq!(parse_whatsapp_link("\"whatsapp://send?text=100%\""), send(None, "100%"));
	}

	#[test]
	fn invite_links() {
		let invite = |c: &str| Some(WhatsAppLink::Invite { code: c.to_string() });
		assert_eq!(parse_whatsapp_link("https://chat.whatsapp.com/AbC123"), invite("AbC123"));
		assert_eq!(parse_whatsapp_link("https://chat.whatsapp.com/invite/AbC123/"), invite("AbC123"));
		assert_eq!(parse_whatsapp_link("whatsapp://chat/?code=AbC123"), invite("AbC123"));
	}

	#[test]
	fn channel_links() {
		let channel = |c: &str| Some(WhatsAppLink::Channel { code: c.to_string() });
		assert_eq!(parse_whatsapp_link("https://whatsapp.com/channel/0029VaAbC"), channel("0029VaAbC"));
		assert_eq!(parse_whatsapp_link("whatsapp://channel/0029VaAbC"), channel("0029VaAbC"));
		assert_eq!(parse_whatsapp_link("whatsapp://channel?code=0029VaAbC"), channel("0029VaAbC"));
		assert_eq!(
			parse_whatsapp_link("https://whatsapp.com/channel/0029VbEF0hk0wajnFXjT4g2b"),
			channel("0029VbEF0hk0wajnFXjT4g2b")
		);
		assert_eq!(parse_whatsapp_link("https://www.whatsapp.com/channel/0029VaAbC/123"), channel("0029VaAbC"));
		assert_eq!(parse_whatsapp_link("https://whatsapp.com/download"), None);
	}

	#[test]
	fn call_links() {
		let call = |t: &str, video| Some(WhatsAppLink::Call { token: t.to_string(), video });
		assert_eq!(parse_whatsapp_link("https://call.whatsapp.com/voice/AbC123"), call("AbC123", false));
		assert_eq!(parse_whatsapp_link("https://call.whatsapp.com/audio/AbC123?x=1"), call("AbC123", false));
		assert_eq!(parse_whatsapp_link("https://call.whatsapp.com/video/AbC123/"), call("AbC123", true));
		assert_eq!(parse_whatsapp_link("https://call.whatsapp.com/AbC123"), None);
		assert_eq!(parse_whatsapp_link("https://call.whatsapp.com/fax/AbC123"), None);
	}

	#[test]
	fn not_ours() {
		assert_eq!(parse_whatsapp_link("https://example.com/send?text=x"), None);
		assert_eq!(parse_whatsapp_link("whatsapp://send"), None);
		assert_eq!(parse_whatsapp_link("hello"), None);
	}
}
