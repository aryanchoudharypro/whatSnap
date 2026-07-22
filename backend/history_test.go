package main

import (
	"database/sql"
	"fmt"
	"os"
	"path/filepath"
	"testing"
	"time"

	_ "modernc.org/sqlite"
)

func TestHistorySyncStress(t *testing.T) {
	tempDir, err := os.MkdirTemp("", "stress_test_*")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	dbPath := filepath.Join(tempDir, "stress.db")
	db, err := sql.Open("sqlite", "file:"+dbPath)
	if err != nil {
		t.Fatalf("Failed to open DB: %v", err)
	}
	defer db.Close()

	if _, err := db.Exec(`CREATE TABLE messages (
		id TEXT PRIMARY KEY,
		jid TEXT,
		from_me BOOLEAN,
		text TEXT,
		timestamp INTEGER,
		sender_jid TEXT
	)`); err != nil {
		t.Fatalf("Failed to create messages table: %v", err)
	}

	if _, err := db.Exec(`CREATE INDEX idx_messages_jid_timestamp ON messages(jid, timestamp DESC);`); err != nil {
		t.Fatalf("Failed to create index: %v", err)
	}

	tx, err := db.Begin()
	if err != nil {
		t.Fatalf("Failed to begin tx: %v", err)
	}

	stmt, err := tx.Prepare("INSERT INTO messages (id, jid, from_me, text, timestamp, sender_jid) VALUES (?, ?, ?, ?, ?, ?)")
	if err != nil {
		t.Fatalf("Failed to prepare statement: %v", err)
	}
	defer stmt.Close()

	targetJID := "stress_test_chat@g.us"
	totalCount := 50000
	baseTS := int64(1700000000)

	for i := 1; i <= totalCount; i++ {
		msgID := fmt.Sprintf("msg_%d", i)
		fromMe := i%2 == 0
		text := fmt.Sprintf("Synthetic stress message #%d with payload content", i)
		ts := baseTS + int64(i)
		senderJID := fmt.Sprintf("sender_%d@s.whatsapp.net", i%50)

		_, err = stmt.Exec(msgID, targetJID, fromMe, text, ts, senderJID)
		if err != nil {
			t.Fatalf("Failed to insert message %d: %v", i, err)
		}
	}

	if err := tx.Commit(); err != nil {
		t.Fatalf("Failed to commit transaction: %v", err)
	}

	var totalFetched int
	var beforeTS int64 = 0
	var maxLatency time.Duration
	var totalLatency time.Duration
	pageCount := 0

	for {
		start := time.Now()
		var rows *sql.Rows
		var queryErr error

		if beforeTS > 0 {
			rows, queryErr = db.Query("SELECT id, text, timestamp, from_me, COALESCE(sender_jid, '') FROM messages WHERE jid = ? AND timestamp < ? ORDER BY timestamp DESC LIMIT 50", targetJID, beforeTS)
		} else {
			rows, queryErr = db.Query("SELECT id, text, timestamp, from_me, COALESCE(sender_jid, '') FROM messages WHERE jid = ? ORDER BY timestamp DESC LIMIT 50", targetJID)
		}

		latency := time.Since(start)
		if queryErr != nil {
			t.Fatalf("Query failed on page %d: %v", pageCount+1, queryErr)
		}

		pageCount++
		totalLatency += latency
		if latency > maxLatency {
			maxLatency = latency
		}

		if latency >= 10*time.Millisecond {
			t.Errorf("Page %d query latency %v exceeded 10ms threshold", pageCount, latency)
		}

		count := 0
		var oldestInPage int64 = 0

		for rows.Next() {
			var id, text, senderJID string
			var ts int64
			var fromMe bool
			if err := rows.Scan(&id, &text, &ts, &fromMe, &senderJID); err != nil {
				t.Fatalf("Scan error on page %d: %v", pageCount, err)
			}
			count++
			totalFetched++
			oldestInPage = ts
		}
		rows.Close()

		if count == 0 {
			break
		}
		beforeTS = oldestInPage
	}

	if totalFetched != totalCount {
		t.Fatalf("Data truncation! Expected %d messages, fetched %d", totalCount, totalFetched)
	}

	avgLatency := totalLatency / time.Duration(pageCount)
	t.Logf("Stress test passed: Fetched %d messages across %d pages. Avg latency: %v, Max latency: %v", totalFetched, pageCount, avgLatency, maxLatency)
}

func TestPaginationBoundaryZero(t *testing.T) {
	tempDir, err := os.MkdirTemp("", "boundary_test_*")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	dbPath := filepath.Join(tempDir, "boundary.db")
	db, err := sql.Open("sqlite", "file:"+dbPath)
	if err != nil {
		t.Fatalf("Failed to open DB: %v", err)
	}
	defer db.Close()

	if _, err := db.Exec(`CREATE TABLE messages (id TEXT PRIMARY KEY, jid TEXT, from_me BOOLEAN, text TEXT, timestamp INTEGER, sender_jid TEXT)`); err != nil {
		t.Fatalf("Failed to create table: %v", err)
	}

	for i := 1; i <= 10; i++ {
		_, err = db.Exec("INSERT INTO messages (id, jid, from_me, text, timestamp, sender_jid) VALUES (?, ?, ?, ?, ?, ?)",
			fmt.Sprintf("m_%d", i), "chat@s.whatsapp.net", false, fmt.Sprintf("msg %d", i), int64(1000+i), "sender@s.whatsapp.net")
		if err != nil {
			t.Fatalf("Insert failed: %v", err)
		}
	}

	rows, err := db.Query("SELECT id, text, timestamp FROM messages WHERE jid = ? ORDER BY timestamp DESC LIMIT 50", "chat@s.whatsapp.net")
	if err != nil {
		t.Fatalf("Query failed: %v", err)
	}
	defer rows.Close()

	count := 0
	for rows.Next() {
		count++
	}
	if count != 10 {
		t.Fatalf("Expected 10 messages, got %d", count)
	}
}

func TestPaginationBeyondOldest(t *testing.T) {
	tempDir, err := os.MkdirTemp("", "oldest_test_*")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	dbPath := filepath.Join(tempDir, "oldest.db")
	db, err := sql.Open("sqlite", "file:"+dbPath)
	if err != nil {
		t.Fatalf("Failed to open DB: %v", err)
	}
	defer db.Close()

	if _, err := db.Exec(`CREATE TABLE messages (id TEXT PRIMARY KEY, jid TEXT, from_me BOOLEAN, text TEXT, timestamp INTEGER, sender_jid TEXT)`); err != nil {
		t.Fatalf("Failed to create table: %v", err)
	}

	_, err = db.Exec("INSERT INTO messages (id, jid, from_me, text, timestamp, sender_jid) VALUES (?, ?, ?, ?, ?, ?)",
		"m1", "chat@s.whatsapp.net", false, "msg 1", int64(1000), "sender@s.whatsapp.net")
	if err != nil {
		t.Fatalf("Insert failed: %v", err)
	}

	rows, err := db.Query("SELECT id FROM messages WHERE jid = ? AND timestamp < ? ORDER BY timestamp DESC LIMIT 50", "chat@s.whatsapp.net", int64(500))
	if err != nil {
		t.Fatalf("Query failed: %v", err)
	}
	defer rows.Close()

	count := 0
	for rows.Next() {
		count++
	}
	if count != 0 {
		t.Fatalf("Expected 0 messages beyond oldest, got %d", count)
	}
}

func TestPaginationDuplicateTimestamps(t *testing.T) {
	tempDir, err := os.MkdirTemp("", "dup_ts_test_*")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	dbPath := filepath.Join(tempDir, "dup_ts.db")
	db, err := sql.Open("sqlite", "file:"+dbPath)
	if err != nil {
		t.Fatalf("Failed to open DB: %v", err)
	}
	defer db.Close()

	if _, err := db.Exec(`CREATE TABLE messages (id TEXT PRIMARY KEY, jid TEXT, from_me BOOLEAN, text TEXT, timestamp INTEGER, sender_jid TEXT)`); err != nil {
		t.Fatalf("Failed to create table: %v", err)
	}

	for i := 1; i <= 60; i++ {
		_, err = db.Exec("INSERT INTO messages (id, jid, from_me, text, timestamp, sender_jid) VALUES (?, ?, ?, ?, ?, ?)",
			fmt.Sprintf("dup_%d", i), "chat@s.whatsapp.net", false, fmt.Sprintf("dup msg %d", i), int64(1000), "sender@s.whatsapp.net")
		if err != nil {
			t.Fatalf("Insert failed: %v", err)
		}
	}

	rows1, err := db.Query("SELECT id, timestamp FROM messages WHERE jid = ? ORDER BY timestamp DESC LIMIT 50", "chat@s.whatsapp.net")
	if err != nil {
		t.Fatalf("Query 1 failed: %v", err)
	}
	var count1 int
	var oldestTS int64
	for rows1.Next() {
		var id string
		var ts int64
		rows1.Scan(&id, &ts)
		count1++
		oldestTS = ts
	}
	rows1.Close()

	rows2, err := db.Query("SELECT id FROM messages WHERE jid = ? AND timestamp < ? ORDER BY timestamp DESC LIMIT 50", "chat@s.whatsapp.net", oldestTS)
	if err != nil {
		t.Fatalf("Query 2 failed: %v", err)
	}
	var count2 int
	for rows2.Next() {
		count2++
	}
	rows2.Close()

	if count1+count2 < 60 {
		t.Logf("Empirical finding: Strict timestamp < %d cursor missed %d messages with duplicate timestamps!", oldestTS, 60-(count1+count2))
	}
}

