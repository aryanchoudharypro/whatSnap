package main

import (
	"log"
	"net"
	"sort"
	"strings"

	"go.mau.fi/whatsmeow/proto/waSyncAction"
	"go.mau.fi/whatsmeow/types"
)

// call_records is what the Calls tab lists. The chat history still gets a
// "call:<id>" message row for each call (see call_log.go), but that row is only
// prose; this table keeps the fields the prose was made from, so the tab never
// has to parse a sentence to learn whether a call was video or how it ended.

type callRecord struct {
	CallID    string
	JID       string
	FromMe    bool
	IsVideo   bool
	IsGroup   bool
	Result    string
	Duration  int64
	Timestamp int64
	Text      string
	// Authoritative marks a record that came from the phone's call history. It
	// replaces a row this device wrote itself, never the other way round.
	Authoritative bool
	// Creator and KeyChat are the call log message's key the way WA Web builds
	// it (see callLogKeyChat), which Delete for me has to name. Empty when it
	// isn't known, and then a delete stays on this device.
	Creator string
	KeyChat string
}

func (acc *Account) ensureCallRecords() error {
	if _, err := acc.db.Exec(`CREATE TABLE IF NOT EXISTS call_records (
		call_id TEXT PRIMARY KEY,
		jid TEXT NOT NULL,
		from_me INTEGER DEFAULT 0,
		is_video INTEGER DEFAULT 0,
		is_group INTEGER DEFAULT 0,
		result TEXT DEFAULT '',
		duration INTEGER DEFAULT 0,
		timestamp INTEGER DEFAULT 0,
		text TEXT DEFAULT '',
		authoritative INTEGER DEFAULT 0
	)`); err != nil {
		return err
	}
	acc.db.Exec(`ALTER TABLE call_records ADD COLUMN creator TEXT DEFAULT ''`)
	acc.db.Exec(`ALTER TABLE call_records ADD COLUMN key_chat TEXT DEFAULT ''`)
	// Calls deleted here or on another device, so a call log record or history
	// chunk naming one later doesn't bring it back.
	acc.db.Exec(`CREATE TABLE IF NOT EXISTS deleted_calls (call_id TEXT PRIMARY KEY)`)
	acc.db.Exec(`CREATE INDEX IF NOT EXISTS idx_call_records_ts ON call_records (timestamp DESC)`)
	acc.db.Exec(`CREATE INDEX IF NOT EXISTS idx_call_records_jid ON call_records (jid)`)
	_, err := acc.db.Exec(`INSERT OR IGNORE INTO call_records (call_id, jid, from_me, is_video, is_group, text, timestamp)
		SELECT SUBSTR(id, 6), jid, COALESCE(from_me, 0), LOWER(COALESCE(text, '')) LIKE '%video%',
			jid LIKE '%@g.us', COALESCE(text, ''), COALESCE(timestamp, 0)
		FROM messages WHERE media_type = 'call' AND id LIKE 'call:%'`)
	return err
}

func callResultName(r waSyncAction.CallLogRecord_CallResult) string {
	switch r {
	case waSyncAction.CallLogRecord_CONNECTED, waSyncAction.CallLogRecord_ONGOING:
		return "connected"
	case waSyncAction.CallLogRecord_MISSED:
		return "missed"
	case waSyncAction.CallLogRecord_REJECTED:
		return "rejected"
	case waSyncAction.CallLogRecord_CANCELLED:
		return "cancelled"
	case waSyncAction.CallLogRecord_UNAVAILABLE:
		return "unavailable"
	case waSyncAction.CallLogRecord_FAILED, waSyncAction.CallLogRecord_INVALID:
		return "failed"
	case waSyncAction.CallLogRecord_ABANDONED:
		return "abandoned"
	case waSyncAction.CallLogRecord_ACCEPTEDELSEWHERE:
		return "accepted_elsewhere"
	case waSyncAction.CallLogRecord_UPCOMING:
		return "upcoming"
	}
	return ""
}

func (acc *Account) storeCallRecord(r callRecord) {
	if r.CallID == "" || r.JID == "" {
		return
	}
	if acc.callDeleted(r.CallID) {
		return
	}
	var q string
	if r.Authoritative {
		q = `INSERT INTO call_records (call_id, jid, from_me, is_video, is_group, result, duration, timestamp, text, creator, key_chat, authoritative)
			VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1)
			ON CONFLICT(call_id) DO UPDATE SET jid=excluded.jid, from_me=excluded.from_me, is_video=excluded.is_video,
				is_group=excluded.is_group, result=excluded.result, duration=excluded.duration,
				timestamp=excluded.timestamp, text=excluded.text,
				creator=CASE WHEN excluded.creator != '' THEN excluded.creator ELSE call_records.creator END,
				key_chat=CASE WHEN excluded.key_chat != '' THEN excluded.key_chat ELSE call_records.key_chat END,
				authoritative=1`
	} else {
		q = `INSERT INTO call_records (call_id, jid, from_me, is_video, is_group, result, duration, timestamp, text, creator, key_chat, authoritative)
			VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0)
			ON CONFLICT(call_id) DO NOTHING`
	}
	if _, err := acc.db.Exec(q, r.CallID, r.JID, boolToInt(r.FromMe), boolToInt(r.IsVideo), boolToInt(r.IsGroup),
		r.Result, r.Duration, r.Timestamp, r.Text, r.Creator, r.KeyChat); err != nil {
		log.Printf("call records: storing %s failed: %v", r.CallID, err)
	}
}

func (acc *Account) callDeleted(callID string) bool {
	var one int
	return acc.db.QueryRow(`SELECT 1 FROM deleted_calls WHERE call_id = ?`, callID).Scan(&one) == nil
}

func (acc *Account) deleteNewestCallRecord(jid string, fromMe bool, until int64) (string, bool) {
	var id string
	err := acc.db.QueryRow(`SELECT call_id FROM call_records
		WHERE jid = ? AND from_me = ? AND is_group = 0 AND timestamp <= ?
		ORDER BY timestamp DESC LIMIT 1`, jid, boolToInt(fromMe), until).Scan(&id)
	if err != nil {
		return "", false
	}
	if _, err := acc.db.Exec(`DELETE FROM call_records WHERE call_id = ?`, id); err != nil {
		return "", false
	}
	acc.db.Exec(`INSERT OR IGNORE INTO deleted_calls (call_id) VALUES (?)`, id)
	return id, true
}

type CallLogJSON struct {
	ID        string `json:"id"`
	JID       string `json:"jid"`
	PeerName  string `json:"peer_name"`
	FromMe    bool   `json:"from_me"`
	Text      string `json:"text"`
	Timestamp int64  `json:"timestamp"`
	IsVideo   bool   `json:"is_video"`
	IsGroup   bool   `json:"is_group"`
	Result    string `json:"result"`
	Duration  int64  `json:"duration"`
}

func (acc *Account) nameResolver() func(string) string {
	names := map[string]string{}
	return func(jidStr string) string {
		if n, ok := names[jidStr]; ok {
			return n
		}
		n := jidStr
		if parsed, err := types.ParseJID(jidStr); err == nil {
			n = acc.resolveDisplayName(parsed)
		}
		names[jidStr] = n
		return n
	}
}

func (acc *Account) handleGetCallLogs(conn net.Conn) {
	go func() {
		logs := []CallLogJSON{}
		if acc.db == nil {
			acc.sendEvent(conn, "call_logs", logs)
			return
		}
		rows, err := acc.db.Query(`SELECT call_id, jid, from_me, is_video, is_group, COALESCE(result, ''), COALESCE(duration, 0),
				COALESCE(text, ''), COALESCE(timestamp, 0)
			FROM call_records ORDER BY timestamp DESC LIMIT 500`)
		if err != nil {
			log.Printf("get_call_logs query failed: %v", err)
			acc.sendEvent(conn, "call_logs", logs)
			return
		}
		defer rows.Close()
		nameFor := acc.nameResolver()
		for rows.Next() {
			var l CallLogJSON
			var fromMe, video, group int
			if err := rows.Scan(&l.ID, &l.JID, &fromMe, &video, &group, &l.Result, &l.Duration, &l.Text, &l.Timestamp); err != nil {
				continue
			}
			l.FromMe, l.IsVideo, l.IsGroup = fromMe == 1, video == 1, group == 1
			l.PeerName = nameFor(l.JID)
			logs = append(logs, l)
		}
		acc.sendEvent(conn, "call_logs", logs)
	}()
}

type CallTargetJSON struct {
	JID   string `json:"jid"`
	Name  string `json:"name"`
	Phone string `json:"phone"`
}

const frequentCallTargets = 8

func (acc *Account) callableJID(jid string) bool {
	if !strings.HasSuffix(jid, "@s.whatsapp.net") && !strings.HasSuffix(jid, "@lid") {
		return false
	}
	p, err := types.ParseJID(jid)
	return err == nil && !acc.isOwnJID(p)
}

func (acc *Account) handleGetCallTargets(conn net.Conn) {
	go func() {
		out := map[string]interface{}{"frequent": []CallTargetJSON{}, "contacts": []CallTargetJSON{}}
		if acc.db == nil {
			acc.sendEvent(conn, "call_targets", out)
			return
		}
		nameFor := acc.nameResolver()
		phones := map[string]string{}
		var contacts []CallTargetJSON
		saved := acc.savedContactJIDs()
		if rows, err := acc.db.Query(`SELECT jid, COALESCE(name, ''), COALESCE(phone, '') FROM contacts_cache
				WHERE name != '' OR phone != ''`); err == nil {
			for rows.Next() {
				var c CallTargetJSON
				if rows.Scan(&c.JID, &c.Name, &c.Phone) != nil || !acc.callableJID(c.JID) {
					continue
				}
				phones[c.JID] = c.Phone
				if saved != nil && !saved[c.JID] {
					continue
				}
				if c.Name == "" {
					c.Name = c.Phone
				}
				contacts = append(contacts, c)
			}
			rows.Close()
		}
		sort.SliceStable(contacts, func(i, j int) bool {
			return strings.ToLower(contacts[i].Name) < strings.ToLower(contacts[j].Name)
		})

		frequent := []CallTargetJSON{}
		seen := map[string]bool{}
		add := func(jid string) {
			if len(frequent) >= frequentCallTargets || seen[jid] || !acc.callableJID(jid) {
				return
			}
			seen[jid] = true
			frequent = append(frequent, CallTargetJSON{JID: jid, Name: nameFor(jid), Phone: phones[jid]})
		}
		if rows, err := acc.db.Query(`SELECT jid FROM call_records WHERE is_group = 0
				GROUP BY jid ORDER BY COUNT(*) DESC, MAX(timestamp) DESC LIMIT 20`); err == nil {
			for rows.Next() {
				var jid string
				if rows.Scan(&jid) == nil {
					add(jid)
				}
			}
			rows.Close()
		}
		if len(frequent) < frequentCallTargets {
			if rows, err := acc.db.Query(`SELECT jid FROM chats WHERE jid LIKE '%@s.whatsapp.net' OR jid LIKE '%@lid'
					ORDER BY COALESCE(timestamp, 0) DESC LIMIT 20`); err == nil {
				for rows.Next() {
					var jid string
					if rows.Scan(&jid) == nil {
						add(jid)
					}
				}
				rows.Close()
			}
		}
		if contacts == nil {
			contacts = []CallTargetJSON{}
		}
		out["frequent"], out["contacts"] = frequent, contacts
		acc.sendEvent(conn, "call_targets", out)
	}()
}
