package main

import (
	"database/sql"
	"testing"

	_ "modernc.org/sqlite"
)

func TestUpsertContactCacheRowKeepsRealName(t *testing.T) {
	var err error
	acc.db, err = sql.Open("sqlite", "file::memory:")
	if err != nil {
		t.Fatal(err)
	}
	acc.db.SetMaxOpenConns(1)
	defer acc.db.Close()
	if err := acc.migrateDB(); err != nil {
		t.Fatal(err)
	}

	name := func() string {
		var n string
		if err := acc.db.QueryRow("SELECT name FROM contacts_cache WHERE jid = 'x@s.whatsapp.net'").Scan(&n); err != nil {
			t.Fatal(err)
		}
		return n
	}

	acc.upsertContactCacheRow("x@s.whatsapp.net", "+911234", "911234", false)
	if got := name(); got != "+911234" {
		t.Fatalf("placeholder on insert: got %q", got)
	}
	acc.upsertContactCacheRow("x@s.whatsapp.net", "Asha", "911234", false)
	if got := name(); got != "Asha" {
		t.Fatalf("real name should replace placeholder: got %q", got)
	}
	acc.upsertContactCacheRow("x@s.whatsapp.net", "+911234", "911234", false)
	acc.upsertContactCacheRow("x@s.whatsapp.net", "", "911234", false)
	if got := name(); got != "Asha" {
		t.Fatalf("placeholder/empty must not overwrite a real name: got %q", got)
	}
	acc.upsertContactCacheRow("x@s.whatsapp.net", "Asha Sharma", "911234", false)
	if got := name(); got != "Asha Sharma" {
		t.Fatalf("a real name should still replace another real name: got %q", got)
	}
}
