package main

import (
	"path/filepath"
	"testing"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/store"
	"go.mau.fi/whatsmeow/types"
)

var (
	testPN     = types.NewJID("919800000001", types.DefaultUserServer)
	testLID    = types.NewJID("100000012345678", types.HiddenUserServer)
	testOldLID = types.NewJID("10000000000002", types.HiddenUserServer)
	mappedLID  = types.NewJID("100000055555555", types.HiddenUserServer)
	mappedPN   = types.NewJID("919800000002", types.DefaultUserServer)
	loneLID    = types.NewJID("100000099999999", types.HiddenUserServer)
)

func testTwin(jid types.JID) types.JID {
	switch jid {
	case mappedPN:
		return mappedLID
	case mappedLID:
		return mappedPN
	}
	return types.EmptyJID
}

func openBlocklistTestAccount(t *testing.T) (*Account, *whatsmeow.Client) {
	t.Helper()
	a := newAccount("default", filepath.Join(t.TempDir(), "blocklist.db"))
	var err error
	a.db, err = openSQLite(a.dbPath)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { a.cancel(); a.db.Close() })
	if err := a.migrateDB(); err != nil {
		t.Fatal(err)
	}
	c := &whatsmeow.Client{Store: &store.Device{}}
	a.waClient.Store(c)
	return a, c
}

func TestBlocklistSnapshotFailurePreservesRowsAndHash(t *testing.T) {
	a, c := openBlocklistTestAccount(t)
	old := &types.Blocklist{DHash: "old", Entries: []types.BlocklistEntry{{JID: testLID, PN: testPN}}}
	if err := a.applyBlocklistLocked(c, old); err != nil {
		t.Fatal(err)
	}
	if _, err := a.db.Exec(`CREATE TRIGGER reject_blocklist BEFORE INSERT ON blocked_contacts
		BEGIN SELECT RAISE(ABORT, 'test write failure'); END`); err != nil {
		t.Fatal(err)
	}
	newList := &types.Blocklist{DHash: "new", Entries: []types.BlocklistEntry{{JID: loneLID}}}
	if err := a.applyBlocklistLocked(c, newList); err == nil {
		t.Fatal("failed cache write was reported as successful")
	}
	rows, err := a.loadBlockedRows()
	if err != nil || len(rows) != 1 || rows[0].JID != testLID || a.blocklistDHash != "old" {
		t.Fatalf("failed snapshot replaced committed state: rows=%v, hash=%s, err=%v", rows, a.blocklistDHash, err)
	}
}

func TestBlocklistNoticesFollowPeopleAndLeaveChatListAlone(t *testing.T) {
	a, c := openBlocklistTestAccount(t)
	chat := testPN.String()
	if _, err := a.db.Exec(`INSERT INTO chats (jid, timestamp, unread_count, preview_snippet) VALUES (?, 100, 2, 'hi')`, chat); err != nil {
		t.Fatal(err)
	}
	if err := a.saveMessage(StoredMessage{ID: "m1", JID: chat, SenderJID: chat, Text: "hi", Timestamp: 100}); err != nil {
		t.Fatal(err)
	}
	apply := func(hash string, entries ...types.BlocklistEntry) {
		t.Helper()
		if err := a.applyBlocklistLocked(c, &types.Blocklist{DHash: hash, Entries: entries}); err != nil {
			t.Fatal(err)
		}
	}
	count := func(want int) {
		t.Helper()
		var n int
		if err := a.db.QueryRow(`SELECT COUNT(*) FROM messages WHERE media_type = 'system'`).Scan(&n); err != nil || n != want {
			t.Fatalf("want %d notices, got %d (%v)", want, n, err)
		}
	}
	first := types.BlocklistEntry{JID: testLID, PN: testPN}
	old := types.BlocklistEntry{JID: testOldLID, PN: testPN}
	apply("initial", first)
	count(0) // Initial sync must not invent a new block action.
	apply("same-person", first, old)
	count(0)
	apply("partial-unblock", old)
	count(0) // Still blocked under the other LID.
	apply("unblocked")
	count(1)
	apply("unblocked")
	count(1) // Repeated snapshots must not duplicate notices.
	apply("blocked", first, old, types.BlocklistEntry{JID: loneLID})
	count(2) // The unknown contact must not create a chat or a notice.
	var blocked, unblocked int
	a.db.QueryRow(`SELECT COUNT(*) FROM messages WHERE text = 'You blocked this contact.'`).Scan(&blocked)
	a.db.QueryRow(`SELECT COUNT(*) FROM messages WHERE text = 'You unblocked this contact.'`).Scan(&unblocked)
	if blocked != 1 || unblocked != 1 {
		t.Fatalf("wrong transitions: blocked=%d unblocked=%d", blocked, unblocked)
	}
	a.UpdateChatSnippet(chat)
	var ts, unread int
	var preview string
	if err := a.db.QueryRow(`SELECT timestamp, unread_count, preview_snippet FROM chats WHERE jid = ?`, chat).Scan(&ts, &unread, &preview); err != nil {
		t.Fatal(err)
	}
	if ts != 100 || unread != 2 || preview != "hi" {
		t.Fatalf("notices changed chat list: timestamp=%d unread=%d preview=%q", ts, unread, preview)
	}
}

func TestBlocklistRejectsOldClientAndClosedAccount(t *testing.T) {
	a, c := openBlocklistTestAccount(t)
	list := &types.Blocklist{DHash: "stale", Entries: []types.BlocklistEntry{{JID: testLID, PN: testPN}}}
	if err := a.applyBlocklistLocked(&whatsmeow.Client{}, list); err == nil {
		t.Fatal("snapshot from a replaced client was accepted")
	}
	a.cancel()
	if err := a.applyBlocklistLocked(c, list); err == nil {
		t.Fatal("snapshot was applied after the account closed")
	}
	rows, err := a.loadBlockedRows()
	if err != nil || len(rows) != 0 || a.blocklistDHash != "" {
		t.Fatalf("stale snapshot changed state: %v, %q, %v", rows, a.blocklistDHash, err)
	}
}

func testRows() []blockedRow {
	return []blockedRow{
		{JID: testLID, PN: testPN},
		{JID: loneLID},
		{JID: testOldLID, PN: testPN},
		{JID: mappedLID},
	}
}

func TestBuildBlockedEntriesOnePerPerson(t *testing.T) {
	names := map[types.JID]string{testPN: "Zoya", loneLID: "asha", mappedPN: "Meera"}
	got := buildBlockedEntries(testRows(), testTwin, func(j types.JID) string { return names[j] })
	if len(got) != 3 {
		t.Fatalf("want 3 people, got %+v", got)
	}
	if got[0].Name != "asha" || got[0].JID != loneLID.String() || len(got[0].Alts) != 0 || got[0].Phone != "" {
		t.Fatalf("LID with no known number: %+v", got[0])
	}
	if got[1].Name != "Meera" || got[1].JID != mappedPN.String() || got[1].Phone != "+919800000002" {
		t.Fatalf("LID resolved through the store: %+v", got[1])
	}
	z := got[2]
	if z.Name != "Zoya" || z.JID != testPN.String() || z.Phone != "+919800000001" ||
		len(z.Alts) != 2 || z.Alts[0] != testLID.String() || z.Alts[1] != testOldLID.String() {
		t.Fatalf("person under two LIDs: %+v", z)
	}
}

func TestBlockedEntriesIncludeLIDForPhoneNumberRows(t *testing.T) {
	entries := buildBlockedEntries([]blockedRow{{JID: mappedPN}}, testTwin, func(types.JID) string { return "A" })
	if len(entries) != 1 || entries[0].JID != mappedPN.String() || len(entries[0].Alts) != 1 || entries[0].Alts[0] != mappedLID.String() {
		t.Fatalf("phone-number snapshot didn't block its LID chat: %+v", entries)
	}
}

func TestBlockedAddressesOf(t *testing.T) {
	for _, jid := range []types.JID{testPN, testLID, testOldLID} {
		got := blockedAddressesOf(testRows(), jid, testTwin)
		if len(got) != 2 || got[0] != testLID || got[1] != testOldLID {
			t.Fatalf("addresses of %s: %v", jid, got)
		}
	}
	if got := blockedAddressesOf(testRows(), mappedPN, testTwin); len(got) != 1 || got[0] != mappedLID {
		t.Fatalf("addresses through the store's mapping: %v", got)
	}
	if got := blockedAddressesOf(testRows(), types.NewJID("911", types.DefaultUserServer), testTwin); len(got) != 0 {
		t.Fatalf("someone not blocked: %v", got)
	}
}
