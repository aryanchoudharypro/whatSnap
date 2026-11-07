package main

import (
	"testing"
	"time"
)

func TestQuietSystemLineLeavesChatListAlone(t *testing.T) {
	openDisappearingTestDB(t)
	const chat = "1@s.whatsapp.net"
	if err := acc.saveMessage(StoredMessage{ID: "m1", JID: chat, SenderJID: chat, Text: "hi", Timestamp: 100}); err != nil {
		t.Fatal(err)
	}
	if _, err := acc.db.Exec(`INSERT INTO chats (jid, timestamp, unread_count, preview_snippet, preview_sender) VALUES (?, 100, 2, 'hi', ?)`, chat, chat); err != nil {
		t.Fatal(err)
	}
	acc.insertQuietSystemMessage(chat, "system_1_500"+quietSystemSuffix, "Your security code with 1 changed.", time.Unix(500, 0))
	acc.UpdateChatSnippet(chat)

	var ts, unread int64
	var preview string
	if err := acc.db.QueryRow(`SELECT timestamp, unread_count, preview_snippet FROM chats WHERE jid = ?`, chat).Scan(&ts, &unread, &preview); err != nil {
		t.Fatal(err)
	}
	if ts != 100 || unread != 2 || preview != "hi" {
		t.Fatalf("chat changed: timestamp %d, unread %d, preview %q", ts, unread, preview)
	}
	var stored int
	acc.db.QueryRow(`SELECT COUNT(*) FROM messages WHERE jid = ? AND media_type = 'system'`, chat).Scan(&stored)
	if stored != 1 {
		t.Fatalf("system line not stored")
	}
}

func TestGroupLineMovesChatOnlyWhenWAWebWould(t *testing.T) {
	openDisappearingTestDB(t)
	const group = "1@g.us"
	if _, err := acc.db.Exec(`INSERT INTO chats (jid, timestamp, unread_count, preview_snippet) VALUES (?, 100, 0, 'hi')`, group); err != nil {
		t.Fatal(err)
	}
	read := func() (ts int64, preview string) {
		acc.db.QueryRow(`SELECT timestamp, preview_snippet FROM chats WHERE jid = ?`, group).Scan(&ts, &preview)
		return
	}

	acc.insertGroupSystemMessage(group, "system_1_500_0", "A removed B", time.Unix(500, 0), false)
	if ts, preview := read(); ts != 100 || preview != "A removed B" {
		t.Fatalf("quiet line: timestamp %d, preview %q; want 100 and the line", ts, preview)
	}

	acc.insertGroupSystemMessage(group, "system_1_600_0", `A changed the group name to "Trip"`, time.Unix(600, 0), true)
	if ts, _ := read(); ts != 600 {
		t.Fatalf("moving line: timestamp %d, want 600", ts)
	}
}
