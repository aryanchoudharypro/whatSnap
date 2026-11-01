package main

import (
	"encoding/json"
	"reflect"
	"testing"
)

func TestMessageJSONWireFormatIsUnchanged(t *testing.T) {
	want := []string{
		"id", "text", "timestamp", "fromMe", "senderJid", "senderName", "status",
		"mediaType", "mediaMimetype", "mediaFilename", "mediaPath", "mediaSize",
		"quotedId", "quotedText", "quotedSender", "isEdited", "isDeleted",
		"sentAt", "deliveredAt", "readAt", "playedAt", "editedAt", "starred",
		"albumId", "reactions",
	}

	blob, err := json.Marshal(MessageJSON{Reactions: emptyReactions})
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	var got map[string]json.RawMessage
	if err := json.Unmarshal(blob, &got); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}

	for _, key := range want {
		if _, ok := got[key]; !ok {
			t.Errorf("missing key %q - the UI reads this field by name", key)
		}
	}
	if len(got) != len(want) {
		var extra []string
		for k := range got {
			if !contains(want, k) {
				extra = append(extra, k)
			}
		}
		t.Errorf("key count = %d, want %d; unexpected: %v", len(got), len(want), extra)
	}
}

// A message with no reactions must serialize as [] and not null: the UI's
// field is a Vec with #[serde(default)], which covers an absent key but not an
// explicit null, so null would fail the whole payload.
func TestReactionsSerializeAsEmptyListNotNull(t *testing.T) {
	blob, err := json.Marshal(MessageJSON{Reactions: emptyReactions})
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	var got struct {
		Reactions json.RawMessage `json:"reactions"`
	}
	if err := json.Unmarshal(blob, &got); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if string(got.Reactions) != "[]" {
		t.Errorf("reactions = %s, want []", got.Reactions)
	}
}

// Values must survive the struct round trip with the same types the map form
// produced - notably the three int-to-bool conversions.
func TestMessageJSONValuesRoundTrip(t *testing.T) {
	msg := MessageJSON{
		ID: "m1", Text: "hi", Timestamp: 17, FromMe: true,
		SenderJID: "a@s.whatsapp.net", SenderName: "A", Status: "read",
		MediaType: "image", MediaMimetype: "image/jpeg", MediaFilename: "p.jpg",
		MediaPath: `C:\tmp\p.jpg`, QuotedID: "q1", QuotedText: "quoted",
		QuotedSender: "You", IsEdited: true, IsDeleted: false,
		SentAt: 1, DeliveredAt: 2, ReadAt: 3, PlayedAt: 4, EditedAt: 5,
		Starred: true, AlbumID: "alb1",
		Reactions: []ReactionInfo{{ReactorJID: "b@s.whatsapp.net", Emoji: "x", Timestamp: 9}},
	}
	blob, err := json.Marshal(msg)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	var back MessageJSON
	if err := json.Unmarshal(blob, &back); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if !reflect.DeepEqual(msg, back) {
		t.Errorf("round trip changed the message:\n got %+v\nwant %+v", back, msg)
	}
}

func contains(xs []string, s string) bool {
	for _, x := range xs {
		if x == s {
			return true
		}
	}
	return false
}
