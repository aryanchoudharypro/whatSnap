//! Text the Go backend and WhatsApp send in English: group notices, the
//! summaries of special messages, and error messages. Every English form is
//! a msgid in the po files, so each language translates it there; with no
//! translation the English text comes back unchanged.

use std::fmt::Display;

/// Notices said exactly like this.
static SYSTEM_TEXTS: [&str; 9] = [
	"You blocked this contact.",
	"You unblocked this contact.",
	"You received a view once message. For added privacy, you can only open it on your phone.",
	"You sent a view once message from another device.",
	"You joined the group",
	"You scheduled a message",
	"Group created",
	"Options were removed from a poll",
	"An option added to a poll was removed",
];

/// "{name} did something" notices, as (when WhatsApp names you, when it names
/// someone else). Checked in order, by the text after "{}".
static WHO_DID_TEXTS: [(&str, &str); 26] = [
	("You deleted this group's icon", "{} deleted this group's icon"),
	("You changed this group's icon", "{} changed this group's icon"),
	("You turned off disappearing messages.", "{} turned off disappearing messages."),
	("You pinned a message", "{} pinned a message"),
	("You turned on advanced chat privacy", "{} turned on advanced chat privacy"),
	("You turned off advanced chat privacy", "{} turned off advanced chat privacy"),
	("You created the group", "{} created the group"),
	("You created this group", "{} created this group"),
	("You deleted the group description", "{} deleted the group description"),
	("You changed the group description", "{} changed the group description"),
	(
		"You changed this group's settings to allow only admins to send messages to this group",
		"{} changed this group's settings to allow only admins to send messages to this group",
	),
	(
		"You changed this group's settings to allow all members to send messages to this group",
		"{} changed this group's settings to allow all members to send messages to this group",
	),
	(
		"You changed this group's settings to allow only admins to edit this group's info",
		"{} changed this group's settings to allow only admins to edit this group's info",
	),
	(
		"You changed this group's settings to allow all members to edit this group's info",
		"{} changed this group's settings to allow all members to edit this group's info",
	),
	("You turned on admin approval to join this group", "{} turned on admin approval to join this group"),
	("You turned off admin approval to join this group", "{} turned off admin approval to join this group"),
	(
		"You changed this group's settings to allow only admins to add others to this group",
		"{} changed this group's settings to allow only admins to add others to this group",
	),
	(
		"You changed this group's settings to allow all members to add others to this group",
		"{} changed this group's settings to allow all members to add others to this group",
	),
	("You deleted this group", "{} deleted this group"),
	("You reset this group's invite link", "{} reset this group's invite link"),
	("You joined using this group's invite link", "{} joined using this group's invite link"),
	("You joined", "{} joined"),
	("You left", "{} left"),
	("You removed options from a poll", "{} removed options from a poll"),
	("You removed an option that was added to a poll", "{} removed an option that was added to a poll"),
	("You shared recent message history", "{} shared recent message history"),
];

/// "{name} ..." notices that never start with "You".
static NAMED_TEXTS: [&str; 4] = [
	"{} added you",
	"{} is now a group admin",
	"{} changed their phone number",
	"{} is no longer a group admin",
];

/// Special messages said exactly like this.
static RICH_TEXTS: [&str; 20] = [
	"[Event]",
	"[Event cancelled]",
	"[Scheduled Call]",
	"[Product]",
	"[Order]",
	"[Poll results]",
	"[Sticker Pack]",
	"[Asked for your phone number]",
	"[Payment] Open WhatsApp on your phone to see this payment.",
	"[Mentioned you in a status update]",
	"[Channel Invite]",
	"[Group Invite]",
	"[Channel Admin Invite]",
	"[Reaction]",
	"[Protocol Message]",
	"[Unsupported Message]",
	"[Audio]",
	"[Voice Message]",
	"[Video Note]",
	"[View once] Opened",
];

/// "[Tag] {}" special messages, the rest kept as it is.
static RICH_PREFIX_TEXTS: [&str; 11] = [
	"[Event cancelled] {}",
	"[Scheduled Call] {}",
	"[Product] {}",
	"[Poll results] {}",
	"[Sticker Pack] {}",
	"[Channel Invite] {}",
	"[Group Invite] {}",
	"[Channel Admin Invite] {}",
	"[Audio] {}",
	"[Voice Message] {}",
	"[Video Note] {}",
];

/// Error and status messages said exactly like this.
static BACKEND_TEXTS: [&str; 91] = [
	"Disconnected: Logged out from primary device.",
	"Logged out from primary device.",
	"Disconnected: Connection lost. Attempting to reconnect...",
	"Failed to send QR image",
	"Can't reach WhatsApp. Check your internet connection.",
	"Couldn't complete the secure connection to WhatsApp.",
	"Couldn't connect to WhatsApp.",
	"WhatsApp has rejected this version of whatSnap as outdated. Please update whatSnap to reconnect.",
	"This version of whatSnap is outdated. Please update it.",
	"Couldn't refresh WhatsApp's connection token.",
	"Not responding.",
	"Calls aren't available yet.",
	"That call is no longer available.",
	"That call has already ended.",
	"Wait for the call to connect before adding someone.",
	"You can't add yourself to the call.",
	"You must configure a Secret Code for Chat Lock on your primary phone before you can lock or unlock chats from this device.",
	"Only this channel's admins can post in it.",
	"View once messages can't be forwarded.",
	"This kind of message can't be forwarded.",
	"You can only forward to channels you run.",
	"Message not found for forwarding",
	"Failed to parse message protobuf for forwarding",
	"Message not found for retry",
	"The original file is no longer available to retry.",
	"You've already opened this view once message.",
	"You've already played this view once voice message.",
	"View once media opens outside the viewer.",
	"Not on WhatsApp or query failed",
	"Couldn't load the catalogue. This business may not have one.",
	"Couldn't load the order's details.",
	"This message's buttons can't be opened here.",
	"This message has no buttons.",
	"Couldn't load the pinned messages.",
	"Channel updated.",
	"Channel picture removed.",
	"Channel picture changed.",
	"You're now an admin of this channel.",
	"Admin invite sent.",
	"Admin invite cancelled.",
	"Admin removed.",
	"Ownership handed over. You're now an admin.",
	"Post pinned.",
	"Post unpinned.",
	"There's no one else in this group to call.",
	"Choose a date and a time for the message.",
	"Messages can't be scheduled in this chat.",
	"Phone could not re-upload the expired media",
	"Phone could not re-upload the media",
	"Timeout waiting for phone to re-upload media",
	"a poll can have at most twelve options",
	"a poll needs at least two options",
	"a poll option is too long",
	"poll options must all be different",
	"the poll needs a question",
	"the poll question is too long",
	"the quiz's correct answer isn't one of its options",
	"WhatsApp hasn't turned on pinning channel posts for your account yet.",
	"Can't pin this post: it hasn't finished loading from the channel.",
	"This post hasn't finished loading from the channel.",
	"This poll hasn't finished loading from the channel.",
	"Can't react to this post: it hasn't finished loading from the channel.",
	"Couldn't find this channel.",
	"Couldn't join this group: its community isn't known. Open the community's groups list and try again.",
	"This group needs an admin to approve new members. Your request to join was sent.",
	"Only group admins can see this group's invite link.",
	"Some participants have privacy settings that prevent being added directly. An invite link was sent to them instead.",
	"This group invite has expired.",
	"This invite link was reset by a group admin and no longer works.",
	"This invite link isn't valid.",
	"The pairing code expired. Ask for a new one to try again.",
	"This event can't be opened here.",
	"This contact card can't be opened here.",
	"This message isn't a contact card.",
	"This event was cancelled.",
	"Events can't be posted in channels.",
	"This poll can't be opened here.",
	"You've already answered this quiz.",
	"Can't vote in this poll: it hasn't finished loading from the channel.",
	"You have reached the maximum number of messages you can schedule.",
	"That scheduled message is already gone.",
	"Couldn't delete scheduled message.",
	"Couldn't delete scheduled message. Connect to the internet and try again.",
	"Couldn't recover this message's text to send it now.",
	"That status update isn't here any more.",
	"That isn't a sticker that can be added to favorites.",
	"Couldn't load this sticker pack.",
	"WhatsApp error 479: Failed to delete for everyone. The message key mismatch or the revoke time limit expired.",
	"not logged in",
	"invalid jid",
	"user info not found",
];

/// "Something failed: {}" errors, the reason kept as it is.
static ERROR_PREFIX_TEXTS: [&str; 80] = [
	"Failed to answer call: {}",
	"Failed to delete chat: {}",
	"Failed to clear chat: {}",
	"Failed to leave group: {}",
	"Left the group, but failed to delete the chat: {}",
	"Failed to post status: {}",
	"Logout failed: {}",
	"Could not start pairing: {}",
	"Pairing failed: {}",
	"Export failed while finalizing archive: {}",
	"Export failed: {}",
	"Couldn't change link previews: {}",
	"Couldn't join the call: {}",
	"Couldn't create the call link: {}",
	"Couldn't load the groups in common: {}",
	"Couldn't use the picture: {}",
	"Couldn't create the channel: {}",
	"Couldn't change the channel: {}",
	"Couldn't delete the channel: {}",
	"Couldn't load the channel's followers: {}",
	"Couldn't change the channel's admins: {}",
	"Couldn't send an admin invite to {}",
	"Couldn't accept the invite: {}",
	"Couldn't cancel the invite: {}",
	"Couldn't load who reacted: {}",
	"Couldn't load who voted: {}",
	"Couldn't load the pending admin invites: {}",
	"Couldn't load similar channels: {}",
	"Couldn't load this channel's posts: {}",
	"Couldn't load channels: {}",
	"Couldn't react: {}",
	"Couldn't change this channel's mute setting: {}",
	"Couldn't load this channel: {}",
	"Couldn't load the community's groups: {}",
	"Couldn't join this group: {}",
	"Couldn't delete the contact: {}",
	"Couldn't read the group's settings: {}",
	"Couldn't change a group setting: {}",
	"Couldn't report to the admins: {}",
	"Couldn't load reported messages: {}",
	"Couldn't load member changes: {}",
	"Couldn't load the group: {}",
	"Couldn't send the message history: {}",
	"Couldn't get the invite link: {}",
	"Couldn't update participant(s): {}",
	"This invite is no longer valid: {}",
	"Couldn't open this invite link: {}",
	"Couldn't forward the media: {}",
	"Couldn't send the report: {}",
	"Couldn't change disappearing messages: {}",
	"Couldn't answer the event: {}",
	"Couldn't send your answer: {}",
	"Can't create the event: {}",
	"Couldn't load the join requests: {}",
	"Couldn't answer the join requests: {}",
	"Couldn't create the community: {}",
	"The community was created, but its description couldn't be set: {}",
	"Couldn't load your groups: {}",
	"Couldn't deactivate the community: {}",
	"Couldn't change your tag: {}",
	"Couldn't load the group's members to mention: {}",
	"Couldn't pin: {}",
	"Couldn't keep the message: {}",
	"Couldn't change advanced chat privacy: {}",
	"Couldn't vote: {}",
	"Couldn't send the vote: {}",
	"Couldn't save the excluded contacts: {}",
	"Couldn't use that photo: {}",
	"Couldn't change your profile photo: {}",
	"Couldn't change the default message timer: {}",
	"Couldn't react to the status: {}",
	"Couldn't like the status: {}",
	"Couldn't delete the status: {}",
	"Couldn't change muted status updates: {}",
	"Couldn't change who sees your status: {}",
	"Couldn't send the sticker: {}",
	"Couldn't change your favorite stickers: {}",
	"Couldn't load WhatsApp's sticker packs: {}",
	"unknown participant action: {}",
	"invalid participant number(s), skipped: {}",
];

/// `you` when WhatsApp names the user ("You left"), else `other` with the name
/// in its first "{}" ("{} left"). `rest` fills the placeholders after that.
fn by_who(who: &str, you: &'static str, other: &'static str, rest: &[&dyn Display]) -> String {
	if who == "You" {
		return crate::i18n::tr_fmt(you, rest);
	}
	let mut args: Vec<&dyn Display> = Vec::with_capacity(rest.len() + 1);
	args.push(&who);
	args.extend_from_slice(rest);
	crate::i18n::tr_fmt(other, &args)
}

/// A name in a notice, with WhatsApp's "You" translated.
fn participant(who: &str) -> &str {
	if who == "You" { crate::i18n::tr("You") } else { who }
}

/// Who did it and the quoted name, for "Anna changed the group name to
/// \"Family\"" split at `marker` " changed the group name to \"".
fn quoted_after<'a>(text: &'a str, marker: &str) -> Option<(&'a str, &'a str)> {
	let (left, right) = text.split_once(marker)?;
	Some((left, right.strip_suffix('"')?))
}

fn quoted<'a>(text: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
	text.strip_prefix(prefix)?.strip_suffix(suffix)
}

fn duration_phrase(phrase: &str) -> String {
	if let Some(n) = phrase.strip_suffix(" days") {
		return crate::i18n::tr_fmt("{} days", &[&n]);
	}
	if let Some(n) = phrase.strip_suffix(" hours") {
		return crate::i18n::tr_fmt("{} hours", &[&n]);
	}
	if let Some(n) = phrase.strip_suffix(" seconds") {
		return crate::i18n::tr_fmt("{} seconds", &[&n]);
	}
	phrase.to_string()
}

pub fn translate_system_message(text: &str) -> String {
	if let Some(exact) = SYSTEM_TEXTS.iter().copied().find(|s| *s == text) {
		return crate::i18n::tr(exact).to_string();
	}
	if let Some(who) = quoted(text, "Your security code with ", " changed.") {
		return crate::i18n::tr_fmt("Your security code with {} changed.", &[&who]);
	}
	if let Some((who, tail)) =
		text.split_once(" turned on disappearing messages. New messages will disappear from this chat ")
		&& let Some(dur) = tail.strip_suffix(" after they're sent.")
	{
		return by_who(
			who,
			"You turned on disappearing messages. New messages will disappear from this chat {} after they're sent.",
			"{} turned on disappearing messages. New messages will disappear from this chat {} after they're sent.",
			&[&duration_phrase(dur)],
		);
	}
	if let Some((who, name)) = quoted_after(text, " added this group to the community \"") {
		return by_who(
			who,
			"You added this group to the community \"{}\"",
			"{} added this group to the community \"{}\"",
			&[&name],
		);
	}
	if let Some(name) = quoted(text, "This group was added to the community \"", "\"") {
		return crate::i18n::tr_fmt("This group was added to the community \"{}\"", &[&name]);
	}
	if let Some(name) = quoted(text, "The community \"", "\" was deactivated") {
		return crate::i18n::tr_fmt("The community \"{}\" was deactivated", &[&name]);
	}
	if let Some((who, name)) = quoted_after(text, " removed this group from the community \"") {
		return by_who(
			who,
			"You removed this group from the community \"{}\"",
			"{} removed this group from the community \"{}\"",
			&[&name],
		);
	}
	if let Some(name) = quoted(text, "This group was removed from the community \"", "\"") {
		return crate::i18n::tr_fmt("This group was removed from the community \"{}\"", &[&name]);
	}
	if let Some((who, name)) = quoted_after(text, " changed the group name to \"") {
		return by_who(who, "You changed the group name to \"{}\"", "{} changed the group name to \"{}\"", &[&name]);
	}
	if let Some(name) = quoted(text, "Group name changed to \"", "\"") {
		return crate::i18n::tr_fmt("Group name changed to \"{}\"", &[&name]);
	}
	for &(you, other) in WHO_DID_TEXTS.iter() {
		if let Some(who) = text.strip_suffix(&other[2..]) {
			return by_who(who, you, other, &[]);
		}
	}
	for &other in NAMED_TEXTS.iter() {
		if let Some(who) = text.strip_suffix(&other[2..]) {
			return crate::i18n::tr_fmt(other, &[&participant(who)]);
		}
	}
	if let Some((who, target)) = text.split_once(" shared recent message history with ") {
		return by_who(
			who,
			"You shared recent message history with {}",
			"{} shared recent message history with {}",
			&[&target],
		);
	}
	if let Some(target) = text.strip_prefix("Recent message history was shared with ") {
		return crate::i18n::tr_fmt("Recent message history was shared with {}", &[&target]);
	}
	if let Some((who, target)) = text.split_once(" added ") {
		return by_who(who, "You added {}", "{} added {}", &[&participant(target)]);
	}
	if let Some((who, rest)) = text.split_once(" removed ") {
		if let Some(target) = rest.strip_suffix(" as group admin") {
			return by_who(
				who,
				"You removed {} as group admin",
				"{} removed {} as group admin",
				&[&participant(target)],
			);
		}
		return by_who(who, "You removed {}", "{} removed {}", &[&participant(rest)]);
	}
	if let Some((who, rest)) = text.split_once(" made ")
		&& let Some(target) = rest.strip_suffix(" a group admin")
	{
		return by_who(who, "You made {} a group admin", "{} made {} a group admin", &[&participant(target)]);
	}
	text.to_string()
}

pub fn translate_rich_message_text(text: &str) -> String {
	if let Some(exact) = RICH_TEXTS.iter().copied().find(|s| *s == text) {
		return crate::i18n::tr(exact).to_string();
	}
	if let Some(rest) = text.strip_prefix("[Event] ") {
		return crate::i18n::tr_fmt("[Event] {}", &[&event_text(rest)]);
	}
	if let Some(rest) = text.strip_prefix("[Order] ") {
		let rest = rest.replace(", out of stock", crate::i18n::tr(", out of stock"));
		return crate::i18n::tr_fmt("[Order] {}", &[&rest]);
	}
	if let Some(rest) = text.strip_prefix("[View once] ") {
		return crate::i18n::tr_fmt("[View once] {}", &[&translate_rich_message_text(rest)]);
	}
	for &template in RICH_PREFIX_TEXTS.iter() {
		if let Some(rest) = text.strip_prefix(&template[..template.len() - 2]) {
			return crate::i18n::tr_fmt(template, &[&rest]);
		}
	}
	if text.starts_with("[Poll] ") || text.starts_with("[Quiz] ") {
		return match text.find("Options: ") {
			Some(pos) => format!("{}{}", &text[..pos], poll_results_text(&text[pos..])),
			None => text.to_string(),
		};
	}
	text.to_string()
}

/// A poll's or quiz's options and votes ("Options: A, 2 votes; B, 1 vote.
/// 3 votes in all, you voted A"), the option names kept as they are.
pub fn poll_results_text(text: &str) -> String {
	match text {
		"No votes yet." => return crate::i18n::tr("No votes yet.").to_string(),
		"No votes yet" => return crate::i18n::tr("No votes yet").to_string(),
		_ => {}
	}
	if let Some((before, chosen)) = text.split_once(", you voted ") {
		let chosen = chosen.replace(" and ", crate::i18n::tr(" and "));
		return crate::i18n::tr_fmt("{}, you voted {}", &[&poll_results_text(before), &chosen]);
	}
	let mut out = text.to_string();
	for part in [
		"You got it right: the answer is ",
		"The answer is ",
		"Options: ",
		". No votes yet.",
		". No votes yet",
		"1 person voted",
		" people voted",
		"1 vote in all",
		" votes in all",
		" 1 vote",
		" votes",
	] {
		out = out.replace(part, crate::i18n::tr(part));
	}
	out
}

/// An event's replies ("3 going, 1 maybe, 2 can't go. You're going.").
pub fn event_text(text: &str) -> String {
	let mut out = text.to_string();
	for part in [
		"No responses yet.",
		" You're going.",
		" You said maybe.",
		" You can't go.",
		" going",
		" maybe, ",
		" can't go.",
		" (including 1 guest)",
		" (including ",
		" guests)",
	] {
		out = out.replace(part, crate::i18n::tr(part));
	}
	out
}

pub fn translate_backend_message(msg: &str) -> String {
	if let Some(exact) = BACKEND_TEXTS.iter().copied().find(|s| *s == msg) {
		return crate::i18n::tr(exact).to_string();
	}
	if let Some(num) = quoted(msg, "Participant ", " is not on WhatsApp") {
		return crate::i18n::tr_fmt("Participant {} is not on WhatsApp", &[&num]);
	}
	if let Some(count) = quoted(msg, "Admin invites sent to ", " people.") {
		return crate::i18n::tr_fmt("Admin invites sent to {} people.", &[&count]);
	}
	if let Some(name) = msg.strip_suffix(" is already in the call.") {
		return crate::i18n::tr_fmt("{} is already in the call.", &[&name]);
	}
	if let Some(rest) = msg.strip_prefix("Couldn't add ") {
		if let Some(name) = rest.strip_suffix(": the call is full.") {
			return crate::i18n::tr_fmt("Couldn't add {}: the call is full.", &[&name]);
		}
		if let Some(name) = rest.strip_suffix(": they have no device that can take a call.") {
			return crate::i18n::tr_fmt("Couldn't add {}: they have no device that can take a call.", &[&name]);
		}
		if let Some(name) = rest.strip_suffix(
			": WhatsApp hasn't sent the call details needed to invite someone yet. Try again in a moment.",
		) {
			return crate::i18n::tr_fmt(
				"Couldn't add {}: WhatsApp hasn't sent the call details needed to invite someone yet. Try again in a moment.",
				&[&name],
			);
		}
		return crate::i18n::tr_fmt("Couldn't add {}", &[&rest]);
	}
	if let Some(limit) = quoted(msg, "This message can only be forwarded to ", " chat(s) at a time.") {
		return crate::i18n::tr_fmt("This message can only be forwarded to {} chat(s) at a time.", &[&limit]);
	}
	if let Some((feature, _)) = msg.split_once(" is on in this chat, so it can't be exported.") {
		return crate::i18n::tr_fmt("{} is on in this chat, so it can't be exported.", &[&feature]);
	}
	if let Some((feature, _)) = msg.split_once(" is on in this chat, so Meta AI can't be mentioned here.") {
		return crate::i18n::tr_fmt("{} is on in this chat, so Meta AI can't be mentioned here.", &[&feature]);
	}
	for &template in ERROR_PREFIX_TEXTS.iter() {
		if let Some(rest) = msg.strip_prefix(&template[..template.len() - 2]) {
			return crate::i18n::tr_fmt(template, &[&rest]);
		}
	}
	if let Some(rest) = msg.strip_prefix("Couldn't ") {
		return crate::i18n::tr_fmt("Couldn't {}", &[&rest]);
	}
	msg.to_string()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn templates_end_with_their_placeholder() {
		for (you, other) in WHO_DID_TEXTS {
			assert!(other.starts_with("{} ") && !other[2..].contains("{}"), "{other}");
			assert_eq!(you, format!("You{}", &other[2..]));
		}
		for other in NAMED_TEXTS {
			assert!(other.starts_with("{} ") && !other[2..].contains("{}"), "{other}");
		}
		for template in RICH_PREFIX_TEXTS.iter().chain(&ERROR_PREFIX_TEXTS) {
			assert!(template.ends_with("{}") && template.matches("{}").count() == 1, "{template}");
		}
	}

	// With no catalog loaded every text comes back as WhatsApp sent it.
	#[test]
	fn english_text_is_unchanged() {
		for text in [
			"You left",
			"Anna left",
			"Anna added you",
			"You added Anna",
			"Anna removed Marco as group admin",
			"You made Anna a group admin",
			"Anna changed the group name to \"Family\"",
			"Anna turned on disappearing messages. New messages will disappear from this chat 7 days after they're sent.",
			"Your security code with Anna changed.",
		] {
			assert_eq!(translate_system_message(text), text);
		}
		for text in [
			"[Event] Party. 3 going, 1 maybe, 2 can't go. You're going.",
			"[View once] [Voice Message] 0:05",
			"[Order] 2 items, out of stock",
			"[Poll] Lunch? Options: Pizza, 2 votes; Pasta, 1 vote. 3 votes in all, you voted Pizza and Pasta",
		] {
			assert_eq!(translate_rich_message_text(text), text);
		}
		for text in ["Couldn't add Anna: the call is full.", "Export failed: disk full", "Couldn't do this"] {
			assert_eq!(translate_backend_message(text), text);
		}
	}
}
