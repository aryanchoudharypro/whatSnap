package main

import "testing"

func TestViewOnceOpensOnceThenIsGone(t *testing.T) {
	openDisappearingTestDB(t)
	if err := saveMessage(StoredMessage{
		ID: "vo", JID: "1@s.whatsapp.net", Text: viewOnceText("[Image]"), Timestamp: 1,
		Raw: []byte{1, 2, 3}, MediaType: "image",
	}); err != nil {
		t.Fatal(err)
	}
	markViewOnce(db, "vo")
	if viewOnceState("vo") != viewOnceUnopened {
		t.Fatal("not marked view-once")
	}
	if viewOnceBlocked("vo") == "" {
		t.Fatal("unopened view-once media could be saved or forwarded")
	}

	consumeViewOnce("vo", "", "")

	var text string
	var raw []byte
	db.QueryRow(`SELECT text, raw FROM messages WHERE id = 'vo'`).Scan(&text, &raw)
	if text != viewOnceOpenedText || raw != nil {
		t.Fatalf("after opening: text %q, raw %v", text, raw)
	}
	if _, _, _, _, err := ensureMediaCached("vo"); err == nil {
		t.Fatal("opened view-once media could be fetched again")
	}
	// A redelivered copy can't make it openable again.
	markViewOnce(db, "vo")
	if viewOnceState("vo") != viewOnceOpened {
		t.Fatal("re-marking reset an opened view-once message")
	}
	if viewOnceBlocked("plain") != "" {
		t.Fatal("an ordinary message was treated as view-once")
	}
}
