package main

import (
	"bytes"
	"testing"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

func storeTestScheduled(t *testing.T, id, chat, text string, at time.Time) *whatsmeow.PreparedScheduledMessage {
	t.Helper()
	prepared, err := whatsmeow.PrepareScheduledMessage(&waE2E.Message{Conversation: proto.String(text)}, at)
	if err != nil {
		t.Fatal(err)
	}
	crm := prepared.Wrapped.GetConditionalRevealMessage()
	if err := acc.storeScheduled(scheduledRecord{
		MsgID: id, ChatJID: chat, FromMe: true, RevealKeyID: prepared.RevealKeyID, RevealKey: prepared.RevealKey,
		EncPayload: crm.GetEncPayload(), EncIV: crm.GetEncIV(), ScheduledAt: at.Unix(),
	}); err != nil {
		t.Fatal(err)
	}
	return prepared
}

func TestScheduledMessagesListAndLimit(t *testing.T) {
	openCallRecordsDB(t)
	chat := "5511999990000@s.whatsapp.net"
	later := time.Now().Add(2 * time.Hour)
	storeTestScheduled(t, "B", chat, "second", later.Add(time.Hour))
	storeTestScheduled(t, "A", chat, "first", later)
	// Someone else's, waiting for its key: counted, not listed.
	if err := acc.storeScheduled(scheduledRecord{MsgID: "C", ChatJID: chat, SenderJID: chat, RevealKeyID: "x", ScheduledAt: later.Unix()}); err != nil {
		t.Fatal(err)
	}
	if n := acc.pendingScheduledCount(chat); n != 3 {
		t.Fatalf("pending = %d", n)
	}
	payload := acc.scheduledListPayload(chat)
	msgs := payload["messages"].([]map[string]interface{})
	if len(msgs) != 2 || msgs[0]["id"] != "A" || msgs[0]["text"] != "first" || msgs[1]["text"] != "second" {
		t.Fatalf("list %v", msgs)
	}
	acc.handleScheduledPost(&events.ScheduledMessagePost{RevealKeyID: acc.scheduledBy("msg_id", "A").RevealKeyID, Status: "FAILURE"})
	if rec := acc.scheduledBy("msg_id", "A"); rec == nil || rec.Status != scheduledFailed {
		t.Fatalf("after a failed post: %+v", rec)
	}
	if n := acc.pendingScheduledCount(chat); n != 2 {
		t.Fatalf("pending after failure = %d", n)
	}
}

func TestScheduledRevealBeforeMessageIsKeptAsOrphan(t *testing.T) {
	openCallRecordsDB(t)
	key := bytes.Repeat([]byte{3}, 32)
	acc.handleScheduledReveal(&events.ScheduledMessageReveal{RevealKeyID: "R1", RevealKey: key})
	var got []byte
	if err := acc.db.QueryRow(`SELECT reveal_key FROM scheduled_orphan_keys WHERE reveal_key_id = 'R1'`).Scan(&got); err != nil || !bytes.Equal(got, key) {
		t.Fatalf("orphan key %x, %v", got, err)
	}
}

func TestScheduledWhen(t *testing.T) {
	now := time.Date(2026, 11, 9, 10, 0, 0, 0, time.Local)
	cases := map[time.Time]string{
		time.Date(2026, 11, 9, 18, 30, 0, 0, time.Local): "today at 18:30",
		time.Date(2026, 11, 10, 9, 5, 0, 0, time.Local):  "tomorrow at 09:05",
		time.Date(2026, 11, 11, 9, 0, 0, 0, time.Local):  "Wed, Nov 11 at 09:00",
	}
	for at, want := range cases {
		if got := scheduledWhen(at, now); got != want {
			t.Errorf("%v: got %q, want %q", at, got, want)
		}
	}
}

func TestHistoryScheduledInner(t *testing.T) {
	got := historyScheduledInner(&waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{
		Text:        proto.String("hello"),
		ContextInfo: &waE2E.ContextInfo{Expiration: proto.Uint32(86400), StanzaID: proto.String("q")},
	}})
	if got.GetExtendedTextMessage().GetText() != "hello" || got.GetExtendedTextMessage().GetContextInfo().GetExpiration() != 86400 ||
		got.GetExtendedTextMessage().GetContextInfo().GetStanzaID() != "" {
		t.Fatalf("text with timer: %v", got)
	}
	if got := historyScheduledInner(&waE2E.Message{Conversation: proto.String("hi")}); got.GetConversation() != "hi" {
		t.Fatalf("plain text: %v", got)
	}
	if got := historyScheduledInner(&waE2E.Message{ImageMessage: &waE2E.ImageMessage{Caption: proto.String("c")}}); got.GetImageMessage().GetCaption() != "c" {
		t.Fatalf("image: %v", got)
	}
}
