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
	Phone string `json:"phone"`
	Name  string `json:"name"`
}

// --- Messaging -----------------------------------------------------------

type SendCommandData struct {
	To          string `json:"to"`
	Text        string `json:"text"`
	ReplyToID   string `json:"reply_to_id"`     // optional: message ID being quoted
	ReplySender string `json:"reply_to_sender"` // optional: JID of the quoted message's original sender (needed in groups)
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
	Type        string `json:"type"` // image | video | document | sticker
	Path        string `json:"path"` // local file path (preferred - frontend already has native file access)
	Data        string `json:"data"` // base64 file bytes (fallback, e.g. clipboard paste)
	Mimetype    string `json:"mimetype"`
	Filename    string `json:"filename"`
	Caption     string `json:"caption"`
	ReplyToID   string `json:"reply_to_id"`
	ReplySender string `json:"reply_to_sender"`
}

type SendVoiceMessageData struct {
	JID         string `json:"jid"`
	Path        string `json:"path"` // local file path to the recorded clip
	Data        string `json:"data"` // base64 fallback
	Mimetype    string `json:"mimetype"`
	Duration    int    `json:"duration"` // seconds
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

// StoredMessage is the normalized shape used when writing a message row,
// whether it came from history sync, a live event, or something we just sent.
type StoredMessage struct {
	ID            string
	JID           string
	SenderJID     string
	FromMe        bool
	Text          string
	Timestamp     int64
	Raw           []byte
	Status        string // pending|sent|delivered|read|played
	MediaType     string // image|video|audio|document|sticker|"" for plain text
	MediaMimetype string
	MediaFilename string
	MediaPath     string // local cache path, if already downloaded
	QuotedID      string
	QuotedText    string
	QuotedSender  string
	AlbumID       string // header message ID of the album this belongs to, "" if standalone
}
