package main

import "testing"

func TestHistoryAnchorSkipsAStuckOne(t *testing.T) {
	openDisappearingTestDB(t)
	const chat = "15550001111@s.whatsapp.net"
	for _, m := range []struct {
		id, media, album string
		ts               int64
		deleted          int
	}{
		{"photo1", "image", "album1", 100, 0},
		{"photo2", "image", "album1", 100, 0},
		{"gone", "", "", 101, 1},
		{"text1", "", "", 102, 0},
		{"joined", "system", "", 50, 0},
	} {
		if _, err := acc.db.Exec(`INSERT INTO messages (id, jid, from_me, text, timestamp, media_type, album_id, is_deleted)
			VALUES (?, ?, 0, 'x', ?, ?, ?, ?)`, m.id, chat, m.ts, m.media, m.album, m.deleted); err != nil {
			t.Fatal(err)
		}
	}
	oldest, err := acc.oldestHistoryAnchor(chat, nil)
	if err != nil || oldest.id != "photo1" {
		t.Fatalf("oldest = %+v, %v; want photo1 (made-up rows never anchor)", oldest, err)
	}
	alt, err := acc.oldestHistoryAnchor(chat, []string{"photo1", "photo1"})
	if err != nil || alt.id != "text1" {
		t.Fatalf("alternative = %+v, %v; want text1", alt, err)
	}
	alt, err = acc.oldestHistoryAnchor(chat, []string{"photo1", "text1"})
	if err != nil || alt.id != "photo2" {
		t.Fatalf("fallback = %+v, %v; want photo2", alt, err)
	}
}
