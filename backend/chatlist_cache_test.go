package main

import (
	"testing"
	"time"

	"go.mau.fi/whatsmeow/types"
)

func TestStaleUnreadHealsButMarkedUnreadStays(t *testing.T) {
	openDisappearingTestDB(t)
	acc.db.Exec(`INSERT INTO chats (jid, timestamp, unread_count) VALUES ('stale@s.whatsapp.net', 1, 3), ('marked@s.whatsapp.net', 1, 1)`)
	acc.db.Exec(`UPDATE chats SET marked_unread = 1 WHERE jid = 'marked@s.whatsapp.net'`)
	for _, jid := range []string{"stale@s.whatsapp.net", "marked@s.whatsapp.net"} {
		acc.db.Exec(`INSERT INTO messages (id, jid, from_me, text, timestamp, status, read_at) VALUES (?, ?, 0, 'x', 1, 'read', 5)`, "m-"+jid, jid)
	}
	acc.healStaleUnreadCounts()
	unread := func(jid string) (n, marked int) {
		acc.db.QueryRow(`SELECT unread_count, marked_unread FROM chats WHERE jid = ?`, jid).Scan(&n, &marked)
		return
	}
	if n, _ := unread("stale@s.whatsapp.net"); n != 0 {
		t.Fatalf("stale count not healed: %d", n)
	}
	if n, marked := unread("marked@s.whatsapp.net"); n != 1 || marked != 1 {
		t.Fatalf("marked-unread chat lost its mark: %d %d", n, marked)
	}
	acc.db.Exec(`UPDATE chats SET unread_count = 0 WHERE jid = 'marked@s.whatsapp.net'`)
	if _, marked := unread("marked@s.whatsapp.net"); marked != 0 {
		t.Fatal("reading the chat didn't clear its unread mark")
	}
}

func TestChatSettingsFromMap(t *testing.T) {
	chat := types.NewJID("1", types.DefaultUserServer)
	all := map[string]chatSettingsRow{
		chat.String():      {mutedUntil: -1, pinned: true},
		"2@s.whatsapp.net": {mutedUntil: time.Now().Add(time.Hour).Unix(), archived: true},
	}
	settings, ok := acc.chatSettingsFromMap(all, chat)
	if !ok || !settings.Pinned || !settings.MutedUntil.After(time.Now().AddDate(100, 0, 0)) {
		t.Fatalf("muted-forever pinned chat read as %+v", settings)
	}
	if _, ok := acc.chatSettingsFromMap(all, types.NewJID("3", types.DefaultUserServer)); ok {
		t.Fatal("a chat without settings reported some")
	}
}
