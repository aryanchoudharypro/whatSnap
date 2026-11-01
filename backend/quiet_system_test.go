package main

import (
	"testing"
	"time"
)

func TestQuietSystemLineLeavesChatListAlone(t *testing.T) {
	openDisappearingTestDB(t)
	const chat = "1@s.whatsapp.net"
	if err := saveMessage(StoredMessage{ID: "m1", JID: chat, SenderJID: chat, Text: "hi", Timestamp: 100}); err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec(`INSERT INTO chats (jid, timestamp, unread_count, preview_snippet, preview_sender) VALUES (?, 100, 2, 'hi', ?)`, chat, chat); err != nil {
		t.Fatal(err)
	}
	insertQuietSystemMessage(chat, "system_1_500"+quietSystemSuffix, "Your security code with 1 changed.", time.Unix(500, 0))
	UpdateChatSnippet(chat)

	var ts, unread int64
	var preview string
	if err := db.QueryRow(`SELECT timestamp, unread_count, preview_snippet FROM chats WHERE jid = ?`, chat).Scan(&ts, &unread, &preview); err != nil {
		t.Fatal(err)
	}
	if ts != 100 || unread != 2 || preview != "hi" {
		t.Fatalf("chat changed: timestamp %d, unread %d, preview %q", ts, unread, preview)
	}
	var stored int
	db.QueryRow(`SELECT COUNT(*) FROM messages WHERE jid = ? AND media_type = 'system'`, chat).Scan(&stored)
	if stored != 1 {
		t.Fatalf("system line not stored")
	}
}
