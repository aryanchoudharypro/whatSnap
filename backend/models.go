package main

import "encoding/json"

type Event struct {
	Type string          `json:"type"`
	Data json.RawMessage `json:"data,omitempty"`
}

type Command struct {
	Action  string          `json:"action"`
	Account string          `json:"account,omitempty"`
	Data    json.RawMessage `json:"data,omitempty"`
}

type PairCommandData struct {
	Phone string `json:"phone"`
}

type NewContactCommandData struct {
	Phone     string `json:"phone"`
	Name      string `json:"name"`
	FirstName string `json:"first_name"`
	Sync      bool   `json:"sync"`
}

type SendCommandData struct {
	To          string       `json:"to"`
	Text        string       `json:"text"`
	ReplyToID   string       `json:"reply_to_id"`
	ReplySender string       `json:"reply_to_sender"`
	Mentions    []MentionRef `json:"mentions"`
}

type ReactMessageData struct {
	JID       string `json:"jid"`
	MessageID string `json:"message_id"`
	SenderJID string `json:"sender_jid"`
	Emoji     string `json:"emoji"`
}

type EditMessageData struct {
	JID       string       `json:"jid"`
	MessageID string       `json:"message_id"`
	NewText   string       `json:"new_text"`
	Mentions  []MentionRef `json:"mentions"`
}

type DeleteMessageData struct {
	JID         string `json:"jid"`
	MessageID   string `json:"message_id"`
	SenderJID   string `json:"sender_jid"`
	ForEveryone bool   `json:"for_everyone"`
}

type StarMessageData struct {
	JID       string `json:"jid"`
	MessageID string `json:"message_id"`
	SenderJID string `json:"sender_jid"`
	FromMe    bool   `json:"from_me"`
	Starred   bool   `json:"starred"`
}

type SearchMessagesData struct {
	Query string `json:"query"`
	JID   string `json:"jid"`
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

type SendAttachmentData struct {
	JID         string `json:"jid"`
	Type        string `json:"type"`
	Path        string `json:"path"`
	Data        string `json:"data"`
	Mimetype    string `json:"mimetype"`
	Filename    string `json:"filename"`
	Caption     string `json:"caption"`
	Duration    int    `json:"duration"`
	Waveform    []int  `json:"waveform"`
	Thumbnail   string `json:"thumbnail"`
	GifPlayback bool   `json:"gif_playback"`
	ViewOnce    bool   `json:"view_once"`
	ReplyToID   string `json:"reply_to_id"`
	ReplySender string `json:"reply_to_sender"`
}

type SendPollData struct {
	JID             string   `json:"jid"`
	Question        string   `json:"question"`
	Options         []string `json:"options"`
	MultipleAnswers bool     `json:"multiple_answers"`

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
	Path        string `json:"path"`
	Data        string `json:"data"`
	Mimetype    string `json:"mimetype"`
	Duration    int    `json:"duration"`
	Waveform    []int  `json:"waveform"`
	ViewOnce    bool   `json:"view_once"`
	ReplyToID   string `json:"reply_to_id"`
	ReplySender string `json:"reply_to_sender"`
}

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
	DurationSeconds int    `json:"duration_seconds"`
}

type CheckContactsData struct {
	Phones []string `json:"phones"`
}

type SetAboutData struct {
	Text     string `json:"text"`
	Duration *int   `json:"duration,omitempty"`
}

type SetPushNameData struct {
	Name string `json:"name"`
}

type SetPrivacySettingData struct {
	Name  string `json:"name"`
	Value string `json:"value"`
}

type CreateGroupCommandData struct {
	Name         string   `json:"name"`
	Participants []string `json:"participants"`
}

type UpdateGroupParticipantsData struct {
	JID          string   `json:"jid"`
	Participants []string `json:"participants"`
	Action       string   `json:"action"`
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
	Data string `json:"data"`
}

type GetGroupInviteLinkData struct {
	JID   string `json:"jid"`
	Reset bool   `json:"reset"`
}

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
	PollResults   string `json:"pollResults,omitempty"`
	ChannelStats  string `json:"channelStats,omitempty"`
	ChannelPinned bool   `json:"channelPinned,omitempty"`
	Pinned        bool   `json:"pinned,omitempty"`
	Kept          bool   `json:"kept,omitempty"`
	Expiring      bool   `json:"expiring,omitempty"`
	StickerFav    bool   `json:"sticker_favorite,omitempty"`
	// Always a list, never null: the Rust side's `#[serde(default)]` covers a
	// missing field but not an explicit null, which would fail to deserialize.
	Reactions []ReactionInfo `json:"reactions"`
}

type StoredMessage struct {
	ID            string
	JID           string
	SenderJID     string
	WireSender    string
	FromMe        bool
	Text          string
	Timestamp     int64
	Raw           []byte
	Status        string
	MediaType     string
	MediaMimetype string
	MediaFilename string
	MediaPath     string
	MediaSize     int64
	QuotedID      string
	QuotedText    string
	QuotedSender  string
	AlbumID       string
	ServerID      int64
}
