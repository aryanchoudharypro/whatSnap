package main

import (
	"database/sql"
	"testing"
	"time"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
	_ "modernc.org/sqlite"
)

func openDisappearingTestDB(t *testing.T) {
	t.Helper()
	var err error
	db, err = sql.Open("sqlite", "file::memory:")
	if err != nil {
		t.Fatal(err)
	}
	db.SetMaxOpenConns(1)
	if err := migrateDB(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		// The sweep schedules a chat-list broadcast, which needs a live
		// WhatsApp client; it must not fire after the test.
		wsMutex.Lock()
		if chatBroadcastTimer != nil {
			chatBroadcastTimer.Stop()
		}
		wsMutex.Unlock()
		db.Close()
	})
}

func TestChatDisappearingKeepsTheNewestSetting(t *testing.T) {
	openDisappearingTestDB(t)
	const chat = "1@s.whatsapp.net"
	if !setChatDisappearing(chat, 86400, 100) {
		t.Fatal("first setting not stored")
	}
	if setChatDisappearing(chat, 0, 50) {
		t.Fatal("an older setting replaced a newer one")
	}
	if !setChatDisappearing(chat, 604800, 200) {
		t.Fatal("a newer setting was ignored")
	}
	if seconds, ts := chatDisappearing(chat); seconds != 604800 || ts != 200 {
		t.Fatalf("got %d at %d", seconds, ts)
	}
}

func TestOutgoingMessagesCarryTheChatTimer(t *testing.T) {
	openDisappearingTestDB(t)
	chat := types.NewJID("1", types.DefaultUserServer)
	setChatDisappearing(chat.String(), 86400, 100)

	msg := applyDisappearingTimer(chat, &waE2E.Message{Conversation: proto.String("hi")})
	if msg.Conversation != nil || msg.GetExtendedTextMessage().GetText() != "hi" {
		t.Fatalf("text wasn't moved to an extended text: %v", msg)
	}
	if ctx := msg.GetExtendedTextMessage().GetContextInfo(); ctx.GetExpiration() != 86400 || ctx.GetEphemeralSettingTimestamp() != 100 {
		t.Fatalf("context = %v", ctx)
	}

	image := applyDisappearingTimer(chat, &waE2E.Message{ImageMessage: &waE2E.ImageMessage{}})
	if image.GetImageMessage().GetContextInfo().GetExpiration() != 86400 {
		t.Fatal("media didn't get the timer")
	}

	other := types.NewJID("2", types.DefaultUserServer)
	plain := applyDisappearingTimer(other, &waE2E.Message{Conversation: proto.String("hi")})
	if plain.GetConversation() != "hi" {
		t.Fatal("a chat without a timer had its message changed")
	}
}

func TestIncomingTimerIsAdoptedOnlyWhenNewer(t *testing.T) {
	openDisappearingTestDB(t)
	chat := types.NewJID("1", types.DefaultUserServer)
	withTimer := func(seconds uint32, settingTS int64) *waE2E.Message {
		ctx := &waE2E.ContextInfo{Expiration: proto.Uint32(seconds)}
		if settingTS > 0 {
			ctx.EphemeralSettingTimestamp = proto.Int64(settingTS)
		}
		return &waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{Text: proto.String("x"), ContextInfo: ctx}}
	}
	adoptChatDisappearing(chat, withTimer(86400, 0))
	if seconds, _ := chatDisappearing(chat.String()); seconds != 86400 {
		t.Fatalf("unknown timer not learned: %d", seconds)
	}
	// Unstamped, it may only fill in a timer we never knew.
	adoptChatDisappearing(chat, withTimer(604800, 0))
	if seconds, _ := chatDisappearing(chat.String()); seconds != 86400 {
		t.Fatalf("unstamped timer overrode a known one: %d", seconds)
	}
	adoptChatDisappearing(chat, withTimer(604800, 500))
	if seconds, _ := chatDisappearing(chat.String()); seconds != 604800 {
		t.Fatalf("newer stamped timer ignored: %d", seconds)
	}
	group := types.NewJID("123", types.GroupServer)
	adoptChatDisappearing(group, withTimer(86400, 900))
	if seconds, _ := chatDisappearing(group.String()); seconds != 0 {
		t.Fatal("a group's timer was taken from a message")
	}
}

func TestExpiredMessagesAreSwept(t *testing.T) {
	openDisappearingTestDB(t)
	now := time.Now().Unix()
	withTimer := &waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{
		Text: proto.String("x"), ContextInfo: &waE2E.ContextInfo{Expiration: proto.Uint32(60)},
	}}
	for _, id := range []string{"old", "starred", "fresh"} {
		ts := now - 3600
		if id == "fresh" {
			ts = now
		}
		if err := saveMessage(StoredMessage{ID: id, JID: "1@s.whatsapp.net", Text: "x", Timestamp: ts}); err != nil {
			t.Fatal(err)
		}
		noteMessageExpiry(db, id, ts, withTimer)
	}
	db.Exec(`UPDATE messages SET starred = 1 WHERE id = 'starred'`)
	db.Exec(`INSERT INTO poll_votes (poll_id, voter, selected, timestamp) VALUES ('old', 'v', '', 0)`)
	db.Exec(`INSERT INTO messages (id, jid, text, timestamp) VALUES ('plain', '1@s.whatsapp.net', 'x', ?)`, now-7200)

	expireDisappearingMessages()

	left := map[string]bool{}
	rows, err := db.Query(`SELECT id FROM messages`)
	if err != nil {
		t.Fatal(err)
	}
	for rows.Next() {
		var id string
		rows.Scan(&id)
		left[id] = true
	}
	rows.Close()
	if left["old"] || !left["starred"] || !left["fresh"] || !left["plain"] {
		t.Fatalf("left after sweep: %v", left)
	}
	var votes int
	db.QueryRow(`SELECT COUNT(*) FROM poll_votes WHERE poll_id = 'old'`).Scan(&votes)
	if votes != 0 {
		t.Fatal("the expired message's poll votes were left behind")
	}
}
