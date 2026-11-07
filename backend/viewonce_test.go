package main

import "testing"

func TestViewOnceOpensOnceThenIsGone(t *testing.T) {
	openDisappearingTestDB(t)
	if err := acc.saveMessage(StoredMessage{
		ID: "vo", JID: "1@s.whatsapp.net", Text: viewOnceText("[Image]"), Timestamp: 1,
		Raw: []byte{1, 2, 3}, MediaType: "image",
	}); err != nil {
		t.Fatal(err)
	}
	markViewOnce(acc.db, "vo")
	if acc.viewOnceState("vo") != viewOnceUnopened {
		t.Fatal("not marked view-once")
	}
	if acc.viewOnceBlocked("vo") == "" {
		t.Fatal("unopened view-once media could be saved or forwarded")
	}

	acc.consumeViewOnce("vo", "", "")

	var text string
	var raw []byte
	acc.db.QueryRow(`SELECT text, raw FROM messages WHERE id = 'vo'`).Scan(&text, &raw)
	if text != viewOnceOpenedText || raw != nil {
		t.Fatalf("after opening: text %q, raw %v", text, raw)
	}
	if _, _, _, _, err := acc.ensureMediaCached("vo"); err == nil {
		t.Fatal("opened view-once media could be fetched again")
	}
	markViewOnce(acc.db, "vo")
	if acc.viewOnceState("vo") != viewOnceOpened {
		t.Fatal("re-marking reset an opened view-once message")
	}
	if acc.viewOnceBlocked("plain") != "" {
		t.Fatal("an ordinary message was treated as view-once")
	}
}
