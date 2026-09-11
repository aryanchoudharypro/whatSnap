package main

import (
	"database/sql"
	"testing"

	_ "modernc.org/sqlite"
)

func TestUpsertContactCacheRowKeepsRealName(t *testing.T) {
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

	name := func() string {
		var n string
		if err := db.QueryRow("SELECT name FROM contacts_cache WHERE jid = 'x@s.whatsapp.net'").Scan(&n); err != nil {
			t.Fatal(err)
		}
		return n
	}

	upsertContactCacheRow("x@s.whatsapp.net", "+911234", "911234", false)
	if got := name(); got != "+911234" {
		t.Fatalf("placeholder on insert: got %q", got)
	}
	upsertContactCacheRow("x@s.whatsapp.net", "Asha", "911234", false)
	if got := name(); got != "Asha" {
		t.Fatalf("real name should replace placeholder: got %q", got)
	}
	upsertContactCacheRow("x@s.whatsapp.net", "+911234", "911234", false)
	upsertContactCacheRow("x@s.whatsapp.net", "", "911234", false)
	if got := name(); got != "Asha" {
		t.Fatalf("placeholder/empty must not overwrite a real name: got %q", got)
	}
	upsertContactCacheRow("x@s.whatsapp.net", "Asha Sharma", "911234", false)
	if got := name(); got != "Asha Sharma" {
		t.Fatalf("a real name should still replace another real name: got %q", got)
	}
}
