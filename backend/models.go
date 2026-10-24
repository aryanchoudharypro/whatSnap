package main

import "encoding/json"

// Event is a message pushed from the backend to the frontend over the WS connection.
type Event struct {
	Type string          `json:"type"`
	Data json.RawMessage `json:"data,omitempty"`
}

// Command is a message received from the frontend over the WS connection.
type Command struct {
	Action string          `json:"action"`
	Data   json.RawMessage `json:"data,omitempty"`
}

// --- Session / pairing -------------------------------------------------

type PairCommandData struct {
	Phone string `json:"phone"`
}

type NewContactCommandData struct {
	Phone     string `json:"phone"` // as typed: may lack the country code, or start with + or 00
	Name      string `json:"name"`  // full name
	FirstName string `json:"first_name"`
	Sync      bool   `json:"sync"` // also push to WhatsApp's own contact-sync (visible on the phone), not just whatSnap's local cache
}

// --- Messaging -----------------------------------------------------------

type SendCommandData struct {
	To          string `json:"to"`
	Text        string `json:"text"`
	ReplyToID   string `json:"reply_to_id"`     // optional: message ID being quoted
	ReplySender string `json:"reply_to_sender"` // optional: JID of the quoted message's original sender (needed in groups)
	// @mentions picked in the composer; Text holds them as "@Name".
	Mentions []MentionRef `json:"mentions"`
}

type ReactMessageData struct {
	JID       string `json:"jid"`        // chat JID
	MessageID string `json:"message_id"` // target message being reacted to
	SenderJID string `json:"sender_jid"` // original sender of the target message ("" or own JID if it was your own message)
	Emoji     string `json:"emoji"`      // reaction emoji; empty string removes the reaction
}

type EditMessageData struct {
	JID       string `json:"jid"`
	MessageID string `json:"message_id"`
	NewText   string `json:"new_text"`
	// @mentions picked while editing (the message's own are kept too).
	Mentions []MentionRef `json:"mentions"`
}

type DeleteMessageData struct {
	JID         string `json:"jid"`
	MessageID   string `json:"message_id"`
	SenderJID   string `json:"sender_jid"`   // original sender ("" / own JID if it was your own message)
	ForEveryone bool   `json:"for_everyone"` // true = revoke for everyone, false = delete locally only
}

type StarMessageData struct {
	JID       string `json:"jid"`        // chat JID
	MessageID string `json:"message_id"` // target message being starred/unstarred
	SenderJID string `json:"sender_jid"` // original sender ("" or "me" if it was your own message)
	FromMe    bool   `json:"from_me"`
	Starred   bool   `json:"starred"`
}

type SearchMessagesData struct {
	Query string `json:"query"`
	JID   string `json:"jid"` // optional: scope search to a single chat
}

type MessageRefData struct {
	JID       string `json:"jid"`
	MessageID string `json:"message_id"`
}

type SaveMediaAsData struct {
	JID       string `json:"jid"`
	MessageID string `json:"message_id"`
	DestPath  string `json:"dest_path"`
}

// --- Attachments / media ---------------------------------------------------

type SendAttachmentData struct {
	JID         string `json:"jid"`
	Type        string `json:"type"` // image | video | audio | document | sticker
	Path        string `json:"path"` // local file path (preferred - frontend already has native file access)
	Data        string `json:"data"` // base64 file bytes (fallback, e.g. clipboard paste)
	Mimetype    string `json:"mimetype"`
	Filename    string `json:"filename"`
	Caption     string `json:"caption"`
	Duration    int    `json:"duration"`     // seconds, audio only
	Thumbnail   string `json:"thumbnail"`    // base64 PNG of a video's first frame, decoded by the frontend; video only
	GifPlayback bool   `json:"gif_playback"` // a GIF the frontend turned into this (temporary) MP4; loops, silent
	ViewOnce    bool   `json:"view_once"`    // photos and videos: can be opened once
	ReplyToID   string `json:"reply_to_id"`
	ReplySender string `json:"reply_to_sender"`
}

// SendPollData creates a poll. MultipleAnswers is WhatsApp's "Allow multiple
// answers" switch.
type SendPollData struct {
	JID             string   `json:"jid"`
	Question        string   `json:"question"`
	Options         []string `json:"options"`
	MultipleAnswers bool     `json:"multiple_answers"`

	// CorrectAnswer makes it a quiz: the option that is the right answer.
	CorrectAnswer string `json:"correct_answer"`
	ReplyToID     string `json:"reply_to_id"`
	ReplySender   string `json:"reply_to_sender"`
}

type SendContactsData struct {
	JID         string   `json:"jid"`
	ContactJIDs []string `json:"contact_jids"`
	ReplyToID   string   `json:"reply_to_id"`
	ReplySender string   `json:"reply_to_sender"`
}

type SendVoiceMessageData struct {
	JID         string `json:"jid"`
	Path        string `json:"path"` // local file path to the recorded clip
	Data        string `json:"data"` // base64 fallback
	Mimetype    string `json:"mimetype"`
	Duration    int    `json:"duration"` // seconds
	Waveform    []int  `json:"waveform"` // 64 loudness bars, 0-100
	ReplyToID   string `json:"reply_to_id"`
	ReplySender string `json:"reply_to_sender"`
}

// --- Chats -----------------------------------------------------------------

type JIDData struct {
	JID string `json:"jid"`
}

type ChatBoolData struct {
	JID   string `json:"jid"`
	Value bool   `json:"value"`
}

type SetChatMutedData struct {
	JID             string `json:"jid"`
	Muted           bool   `json:"muted"`
	DurationSeconds int    `json:"duration_seconds"` // 0 = forever (when muted=true)
}

// --- Contacts ----------------------------------------------------------------

type CheckContactsData struct {
	Phones []string `json:"phones"`
}

type SetAboutData struct {
	Text string `json:"text"`
	// Duration is how long the About shows before it clears, in seconds;
	// -1 (or leaving it out) keeps it until it's changed.
	Duration *int `json:"duration,omitempty"`
}

type SetPushNameData struct {
	Name string `json:"name"`
}

type SetPrivacySettingData struct {
	Name  string `json:"name"`  // groupadd | last | status | profile | readreceipts | online | calladd
	Value string `json:"value"` // all | contacts | none | known | match_last_seen (contact_blacklist goes through set_privacy_list)
}

// --- Groups ------------------------------------------------------------------

type CreateGroupCommandData struct {
	Name         string   `json:"name"`
	Participants []string `json:"participants"`
}

type UpdateGroupParticipantsData struct {
	JID          string   `json:"jid"`
	Participants []string `json:"participants"`
	Action       string   `json:"action"` // add | remove | promote | demote
}

type SetGroupNameData struct {
	JID  string `json:"jid"`
	Name string `json:"name"`
}

type SetGroupTopicData struct {
	JID   string `json:"jid"`
	Topic string `json:"topic"`
}

type SetGroupPhotoData struct {
	JID  string `json:"jid"`
	Path string `json:"path"`
	Data string `json:"data"` // base64 fallback
}

type GetGroupInviteLinkData struct {
	JID   string `json:"jid"`
	Reset bool   `json:"reset"`
}

// ReactionInfo describes a single reaction attached to a message, used when
// embedding reaction summaries into get_messages / conversation_messages responses.
type ReactionInfo struct {
	ReactorJID  string `json:"reactorJid"`
	ReactorName string `json:"reactorName"`
	Emoji       string `json:"emoji"`
	Timestamp   int64  `json:"timestamp"`
}

// MessageJSON is one message as sent to the UI in a conversation_messages /
// get_messages response. Field names and types must stay in sync with the Rust
// side's ChatMessage.
//
// This used to be built as a map[string]interface{} per message. That cost a
// Go map plus a boxed interface{} per field - for a chat with tens of
// thousands of messages, hundreds of thousands of map entries and allocations
// whose only purpose was to be marshalled once and thrown away. A flat struct
// is one allocation per message and marshals without reflection over a map.
type MessageJSON struct {
	ID            string `json:"id"`
	Text          string `json:"text"`
	Timestamp     int64  `json:"timestamp"`
	FromMe        bool   `json:"fromMe"`
	SenderJID     string `json:"senderJid"`
	SenderName    string `json:"senderName"`
	Status        string `json:"status"`
	MediaType     string `json:"mediaType"`
	MediaMimetype string `json:"mediaMimetype"`
	MediaFilename string `json:"mediaFilename"`
	MediaPath     string `json:"mediaPath"`
	MediaSize     int64  `json:"mediaSize"`
	QuotedID      string `json:"quotedId"`
	QuotedText    string `json:"quotedText"`
	QuotedSender  string `json:"quotedSender"`
	IsEdited      bool   `json:"isEdited"`
	IsDeleted     bool   `json:"isDeleted"`
	SentAt        int64  `json:"sentAt"`
	DeliveredAt   int64  `json:"deliveredAt"`
	ReadAt        int64  `json:"readAt"`
	PlayedAt      int64  `json:"playedAt"`
	EditedAt      int64  `json:"editedAt"`
	Starred       bool   `json:"starred"`
	AlbumID       string `json:"albumId"`
	// For a poll: each option's share of the votes, shown after the question.
	PollResults string `json:"pollResults,omitempty"`
	// For a channel post: its views and reaction totals.
	ChannelStats string `json:"channelStats,omitempty"`
	// A pinned channel post.
	ChannelPinned bool `json:"channelPinned,omitempty"`
	// Pinned in the chat, kept in a chat with disappearing messages, and
	// whether it disappears at all (so can be kept).
	Pinned     bool `json:"pinned,omitempty"`
	Kept       bool `json:"kept,omitempty"`
	Expiring   bool `json:"expiring,omitempty"`
	StickerFav bool `json:"sticker_favorite,omitempty"`
	// Always a list, never null: the Rust side's `#[serde(default)]` covers a
	// missing field but not an explicit null, which would fail to deserialize.
	Reactions []ReactionInfo `json:"reactions"`
}

// StoredMessage is the normalized shape used when writing a message row,
// whether it came from history sync, a live event, or something we just sent.
type StoredMessage struct {
	ID            string
	JID           string
	SenderJID     string
	WireSender    string // the sender as WhatsApp named them, when not SenderJID (see wire.go)
	FromMe        bool
	Text          string
	Timestamp     int64
	Raw           []byte
	Status        string // pending|sent|delivered|read|played
	MediaType     string // image|video|audio|document|sticker|"" for plain text
	MediaMimetype string
	MediaFilename string
	MediaPath     string // local cache path, if already downloaded
	MediaSize     int64  // sender-reported byte size, 0 if unknown
	QuotedID      string
	QuotedText    string
	QuotedSender  string
	AlbumID       string // header message ID of the album this belongs to, "" if standalone
	ServerID      int64  // channel posts: the server's ID for the post, 0 otherwise
}
