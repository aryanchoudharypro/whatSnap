package main

import (
	"database/sql"
	"testing"

	_ "modernc.org/sqlite"
)

func TestRecordParticipantReceiptKeepsEachStateTime(t *testing.T) {
	var err error
	db, err = sql.Open("sqlite", "file::memory:")
	if err != nil {
		t.Fatal(err)
	}
	db.SetMaxOpenConns(1)
	defer db.Close()
	if err := migrateDB(); err != nil {
		t.Fatal(err)
	}

	for _, r := range []struct {
		status string
		ts     int64
	}{{"read", 200}, {"delivered", 100}, {"read", 300}} {
		if err := recordParticipantReceipt(db, "m1", "p@s.whatsapp.net", r.status, r.ts); err != nil {
			t.Fatal(err)
		}
	}

	var status string
	var ts, delivered, read int64
	if err := db.QueryRow(`SELECT status, timestamp, delivered_at, read_at FROM message_receipts
		WHERE message_id = 'm1' AND participant_jid = 'p@s.whatsapp.net'`).Scan(&status, &ts, &delivered, &read); err != nil {
		t.Fatal(err)
	}
	if status != "read" || ts != 200 {
		t.Errorf("status/timestamp = %s/%d, want read/200 (a late delivered must not regress it)", status, ts)
	}
	if delivered != 100 || read != 200 {
		t.Errorf("delivered_at/read_at = %d/%d, want 100/200 (first time each state was seen)", delivered, read)
	}
}
