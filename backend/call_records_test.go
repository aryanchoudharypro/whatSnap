package main

import (
	"database/sql"
	"testing"

	"go.mau.fi/whatsmeow/proto/waSyncAction"
	_ "modernc.org/sqlite"
)

func openCallRecordsDB(t *testing.T) {
	t.Helper()
	var err error
	db, err = sql.Open("sqlite", "file::memory:")
	if err != nil {
		t.Fatal(err)
	}
	db.SetMaxOpenConns(1)
	t.Cleanup(func() { db.Close() })
	if err := migrateDB(); err != nil {
		t.Fatal(err)
	}
}

func TestStoreCallRecordPhoneRecordWinsOverLiveRow(t *testing.T) {
	openCallRecordsDB(t)
	peer := "5511999990000@s.whatsapp.net"
	storeCallRecord(callRecord{CallID: "c1", JID: peer, Result: "missed", Timestamp: 100, Text: "Missed voice call"})
	storeCallRecord(callRecord{CallID: "c1", JID: peer, FromMe: true, IsVideo: true, Result: "connected", Duration: 30,
		Timestamp: 90, Text: "Outgoing video call, 30 seconds", Authoritative: true})
	// A live row arriving later must not undo the phone's record.
	storeCallRecord(callRecord{CallID: "c1", JID: peer, Result: "missed", Timestamp: 100, Text: "Missed voice call"})

	var result string
	var video, fromMe int
	var dur int64
	if err := db.QueryRow(`SELECT result, is_video, from_me, duration FROM call_records WHERE call_id='c1'`).
		Scan(&result, &video, &fromMe, &dur); err != nil {
		t.Fatal(err)
	}
	if result != "connected" || video != 1 || fromMe != 1 || dur != 30 {
		t.Errorf("got %s video=%d fromMe=%d dur=%d", result, video, fromMe, dur)
	}
}

func TestDeleteNewestCallRecord(t *testing.T) {
	openCallRecordsDB(t)
	peer := "5511999990000@s.whatsapp.net"
	storeCallRecord(callRecord{CallID: "old", JID: peer, Timestamp: 10, Authoritative: true})
	storeCallRecord(callRecord{CallID: "new", JID: peer, Timestamp: 20, Authoritative: true})
	storeCallRecord(callRecord{CallID: "out", JID: peer, FromMe: true, Timestamp: 30, Authoritative: true})
	storeCallRecord(callRecord{CallID: "later", JID: peer, Timestamp: 500, Authoritative: true})

	if id, ok := deleteNewestCallRecord(peer, false, 100); !ok || id != "new" {
		t.Fatalf("got %q %v, want new", id, ok)
	}
	if id, ok := deleteNewestCallRecord(peer, false, 100); !ok || id != "old" {
		t.Fatalf("got %q %v, want old", id, ok)
	}
	if _, ok := deleteNewestCallRecord(peer, false, 100); ok {
		t.Fatal("a call after the deletion time, or in the other direction, must be kept")
	}
}

func TestMigrationCarriesOverCallMessages(t *testing.T) {
	openCallRecordsDB(t)
	db.Exec(`INSERT INTO messages (id, jid, from_me, text, timestamp, media_type)
		VALUES ('call:x', 'a@s.whatsapp.net', 1, 'Outgoing video call', 5, 'call')`)
	if err := ensureCallRecords(); err != nil {
		t.Fatal(err)
	}
	var video, fromMe int
	if err := db.QueryRow(`SELECT is_video, from_me FROM call_records WHERE call_id='x'`).Scan(&video, &fromMe); err != nil {
		t.Fatal(err)
	}
	if video != 1 || fromMe != 1 {
		t.Errorf("video=%d fromMe=%d", video, fromMe)
	}
}

func TestCallResultName(t *testing.T) {
	if got := callResultName(waSyncAction.CallLogRecord_ONGOING); got != "connected" {
		t.Errorf("ongoing = %q", got)
	}
	if got := callResultName(waSyncAction.CallLogRecord_ACCEPTEDELSEWHERE); got != "accepted_elsewhere" {
		t.Errorf("elsewhere = %q", got)
	}
}
