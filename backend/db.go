package main

import (
	"cmp"
	"context"
	"database/sql"
	"fmt"
	"log"
	"strings"
	"time"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
)

// migrateDB creates/extends all of this application's own tables. It is
// called once at startup, after sql.Open and before the whatsmeow sqlstore
// container is attached to the same *sql.DB. Every ALTER TABLE follows the
// existing pattern in this codebase: errors are ignored because SQLite has
// no "ADD COLUMN IF NOT EXISTS", so re-running this on a DB that already has
// the column is expected to "fail" harmlessly.
func migrateDB() error {
	if _, err := db.Exec(`CREATE TABLE IF NOT EXISTS chats (
		jid TEXT PRIMARY KEY,
		name TEXT,
		timestamp INTEGER
	)`); err != nil {
		return err
	}
	db.Exec("ALTER TABLE chats ADD COLUMN unread_count INTEGER DEFAULT 0")
	db.Exec("ALTER TABLE chats ADD COLUMN pinned INTEGER DEFAULT 0")
	db.Exec("ALTER TABLE chats ADD COLUMN muted INTEGER DEFAULT 0")
	db.Exec("ALTER TABLE chats ADD COLUMN preview_snippet TEXT DEFAULT ''")
	db.Exec("ALTER TABLE chats ADD COLUMN archived INTEGER DEFAULT 0")
	db.Exec("ALTER TABLE chats ADD COLUMN preview_sender TEXT DEFAULT ''")
	db.Exec("ALTER TABLE chats ADD COLUMN preview_from_me INTEGER DEFAULT 0")
	db.Exec("ALTER TABLE chats ADD COLUMN preview_is_reaction INTEGER DEFAULT 0")
	db.Exec("ALTER TABLE chats ADD COLUMN preview_reaction_emoji TEXT DEFAULT ''")
	db.Exec("ALTER TABLE chats ADD COLUMN is_community INTEGER DEFAULT 0")
	db.Exec("ALTER TABLE chats ADD COLUMN is_admin INTEGER DEFAULT 0")
	db.Exec("ALTER TABLE chats ADD COLUMN is_locked INTEGER DEFAULT 0")
	// Member count for groups (from the joined-groups list), so a group
	// message's own status only reaches "delivered"/"read" once every other
	// member got there - the way official WhatsApp shows ticks in groups.
	db.Exec("ALTER TABLE chats ADD COLUMN participant_count INTEGER DEFAULT 0")
	// Community structure, from the joined-groups list and link/unlink
	// events: which community a group belongs to, and whether it is that
	// community's announcement group.
	db.Exec("ALTER TABLE chats ADD COLUMN parent_jid TEXT DEFAULT ''")
	db.Exec("ALTER TABLE chats ADD COLUMN is_announce_group INTEGER DEFAULT 0")
	// Channels we follow, and our role in each (owner, admin, subscriber).
	db.Exec("ALTER TABLE chats ADD COLUMN is_channel INTEGER DEFAULT 0")
	db.Exec("ALTER TABLE chats ADD COLUMN channel_role TEXT DEFAULT ''")
	// Advanced chat privacy ("limit sharing"), and for a 1:1 chat the
	// setting's own timestamp, so an older stamp can't undo a newer toggle.
	db.Exec("ALTER TABLE chats ADD COLUMN limit_sharing INTEGER DEFAULT 0")
	db.Exec("ALTER TABLE chats ADD COLUMN limit_sharing_ts INTEGER DEFAULT 0")
	// Disappearing messages: the chat's timer in seconds (0 = off) and when
	// it was set, so an older setting can't undo a newer one.
	db.Exec("ALTER TABLE chats ADD COLUMN ephemeral_expiration INTEGER DEFAULT 0")
	db.Exec("ALTER TABLE chats ADD COLUMN ephemeral_setting_ts INTEGER DEFAULT 0")
	// Set while a chat is marked unread on purpose, so healStaleUnreadCounts
	// leaves it alone. Reading the chat (its count going to 0) clears it.
	db.Exec("ALTER TABLE chats ADD COLUMN marked_unread INTEGER DEFAULT 0")
	db.Exec(`CREATE TRIGGER IF NOT EXISTS chats_clear_marked_unread AFTER UPDATE OF unread_count ON chats
		WHEN NEW.unread_count = 0 AND COALESCE(OLD.marked_unread, 0) = 1
		BEGIN UPDATE chats SET marked_unread = 0 WHERE jid = NEW.jid; END`)

	if _, err := db.Exec(`CREATE TABLE IF NOT EXISTS app_settings (
		key TEXT PRIMARY KEY,
		value BLOB
	)`); err != nil {
		return err
	}

	// The last on-demand history request per chat (see
	// handleRequestHistorySync): which message it asked for history before,
	// and when.
	if _, err := db.Exec(`CREATE TABLE IF NOT EXISTS history_requests (
		jid TEXT PRIMARY KEY,
		anchor_id TEXT NOT NULL,
		requested_at INTEGER NOT NULL
	)`); err != nil {
		return err
	}
	// The chat's oldest message when that request went out: if it's still
	// the oldest next time, the phone sent nothing older for that anchor.
	db.Exec("ALTER TABLE history_requests ADD COLUMN oldest_id TEXT DEFAULT ''")

	if _, err := db.Exec(`CREATE TABLE IF NOT EXISTS messages (
		id TEXT PRIMARY KEY,
		jid TEXT,
		from_me BOOLEAN,
		text TEXT,
		timestamp INTEGER,
		sender_jid TEXT
	)`); err != nil {
		return err
	}
	db.Exec("ALTER TABLE messages ADD COLUMN sender_jid TEXT DEFAULT ''")
	db.Exec("ALTER TABLE messages ADD COLUMN raw BLOB")
	db.Exec("ALTER TABLE messages ADD COLUMN voice_waveform BLOB")
	// New columns added for the enhancement pass:
	db.Exec("ALTER TABLE messages ADD COLUMN status TEXT DEFAULT 'sent'")
	db.Exec("ALTER TABLE messages ADD COLUMN media_type TEXT DEFAULT ''")
	db.Exec("ALTER TABLE messages ADD COLUMN media_mimetype TEXT DEFAULT ''")
	db.Exec("ALTER TABLE messages ADD COLUMN media_filename TEXT DEFAULT ''")
	db.Exec("ALTER TABLE messages ADD COLUMN media_path TEXT DEFAULT ''")
	db.Exec("ALTER TABLE messages ADD COLUMN media_size INTEGER DEFAULT 0")
	db.Exec("ALTER TABLE messages ADD COLUMN quoted_id TEXT DEFAULT ''")
	db.Exec("ALTER TABLE messages ADD COLUMN quoted_text TEXT DEFAULT ''")
	db.Exec("ALTER TABLE messages ADD COLUMN quoted_sender TEXT DEFAULT ''")
	db.Exec("ALTER TABLE messages ADD COLUMN is_edited INTEGER DEFAULT 0")
	db.Exec("ALTER TABLE messages ADD COLUMN is_deleted INTEGER DEFAULT 0")
	db.Exec("ALTER TABLE messages ADD COLUMN sent_at INTEGER DEFAULT 0")
	db.Exec("ALTER TABLE messages ADD COLUMN delivered_at INTEGER DEFAULT 0")
	db.Exec("ALTER TABLE messages ADD COLUMN read_at INTEGER DEFAULT 0")
	db.Exec("ALTER TABLE messages ADD COLUMN played_at INTEGER DEFAULT 0")
	db.Exec("ALTER TABLE messages ADD COLUMN edited_at INTEGER DEFAULT 0")
	db.Exec("ALTER TABLE messages ADD COLUMN starred INTEGER DEFAULT 0")
	// "viewed" only has meaning for jid = 'status@broadcast' rows (Status/Stories).
	db.Exec("ALTER TABLE messages ADD COLUMN viewed INTEGER DEFAULT 0")
	// Media that arrived as one WhatsApp album share the album header's
	// message ID here, so the frontend can group them without guessing from
	// timestamps. Empty for standalone media.
	db.Exec("ALTER TABLE messages ADD COLUMN album_id TEXT DEFAULT ''")
	// Channel posts: the server's ID for the post (what reactions and view
	// receipts name), its view count and reaction totals as JSON.
	db.Exec("ALTER TABLE messages ADD COLUMN server_id INTEGER DEFAULT 0")
	db.Exec("ALTER TABLE messages ADD COLUMN channel_views INTEGER DEFAULT 0")
	db.Exec("ALTER TABLE messages ADD COLUMN channel_reactions TEXT DEFAULT ''")
	// A channel poll's vote counts as JSON, by option hash, and whether a
	// channel post is pinned.
	db.Exec("ALTER TABLE messages ADD COLUMN channel_poll_votes TEXT DEFAULT ''")
	db.Exec("ALTER TABLE messages ADD COLUMN channel_pinned INTEGER DEFAULT 0")
	// When a disappearing message is due to vanish (unix seconds, 0 = never).
	db.Exec("ALTER TABLE messages ADD COLUMN expires_at INTEGER DEFAULT 0")
	// View-once media: 1 = not opened yet, 2 = opened (see viewonce.go).
	db.Exec("ALTER TABLE messages ADD COLUMN view_once INTEGER DEFAULT 0")
	// An edit is delivered as its own stanza with its own message ID, so every
	// receipt for it names an ID no message row has. This maps those IDs back
	// to the message being edited.
	db.Exec(`CREATE TABLE IF NOT EXISTS message_edits (
		edit_id TEXT PRIMARY KEY,
		target_id TEXT NOT NULL,
		jid TEXT NOT NULL,
		timestamp INTEGER DEFAULT 0
	)`)
	db.Exec("UPDATE messages SET sent_at = timestamp WHERE from_me = 1 AND (sent_at IS NULL OR sent_at = 0)")
	// Excludes is_deleted=1 rows: those are deliberate "this message was
	// deleted" tombstones (a revoke stub synced from history, or a live
	// revoke of a message whose content we never had) - empty text and no
	// raw is their normal, intended state, matching official WhatsApp
	// keeping the tombstone after a "delete for everyone". Only "delete for
	// me" removes the row outright, and that already happens immediately via
	// deleteMessageForMe, not through this startup sweep.
	db.Exec("DELETE FROM messages WHERE COALESCE(is_deleted, 0) = 0 AND (text = '' OR text IS NULL OR text = '[Unsupported Message]' OR text = '[Protocol Message]') AND (media_type IS NULL OR media_type = '') AND (raw IS NULL OR length(raw) = 0)")
	db.Exec("DELETE FROM chats WHERE jid NOT IN (SELECT DISTINCT jid FROM messages WHERE (text != '' AND text IS NOT NULL AND text != '[Unsupported Message]' AND text != '[Protocol Message]') OR (media_type != '' AND media_type IS NOT NULL) OR (raw IS NOT NULL AND length(raw) > 0) OR COALESCE(is_deleted, 0) = 1) AND pinned = 0 AND muted = 0 AND archived = 0 AND COALESCE(is_community, 0) = 0 AND COALESCE(is_channel, 0) = 0")
	// These re-derive rows stored by older builds. Each one only ever has
	// work to do once, but used to rescan and re-decode every matching row's
	// raw protobuf on every single launch.
	runOnce("backfill_template_messages_v1", backfillTemplateMessages)
	// v2: messages withheld from linked devices (placeholders) now have text.
	runOnce("backfill_template_messages_v2", backfillTemplateMessages)
	// v3: video notes, events, products and the other kinds extractText now
	// knows, plus pins/RSVPs that were stored as bubbles.
	runOnce("backfill_template_messages_v3", backfillTemplateMessages)
	runOnce("backfill_album_messages_v1", backfillAlbumMessages)
	runOnce("backfill_media_sizes_v1", backfillMediaSizes)
	runOnce("remove_key_distribution_stubs_v1", removeKeyDistributionStubs)
	runOnce("tokenize_system_line_names_v1", tokenizeBakedSystemLines)
	// An upload that was still running when the app last quit can never
	// finish now - and retry only accepts pending/failed rows, so leaving it
	// as "uploading" stranded it on screen forever with no way to resend.
	db.Exec(`UPDATE messages SET status = 'failed' WHERE from_me = 1 AND status = 'uploading'`)

	if _, err := db.Exec(`CREATE INDEX IF NOT EXISTS idx_messages_jid_timestamp ON messages(jid, timestamp DESC);`); err != nil {
		return err
	}
	// History sync's LID->phone remap rewrites sender_jid by value; without
	// this it scanned the whole messages table on nearly every chunk.
	if _, err := db.Exec(`CREATE INDEX IF NOT EXISTS idx_messages_sender_jid ON messages(sender_jid)`); err != nil {
		return err
	}
	// The retry sweep (every 3s while connected) and read-receipt lookups
	// filter on status; without these each one scanned the whole table.
	db.Exec(`CREATE INDEX IF NOT EXISTS idx_messages_outgoing_status ON messages(status) WHERE from_me = 1`)
	db.Exec(`CREATE INDEX IF NOT EXISTS idx_messages_incoming_status ON messages(jid, status) WHERE from_me = 0`)
	// The disappearing-message sweep runs every minute; only the few rows
	// with a timer belong in its index.
	db.Exec(`CREATE INDEX IF NOT EXISTS idx_messages_expires_at ON messages(expires_at) WHERE expires_at > 0`)
	// purgeMessages drops a deleted message's edit mappings by target; without
	// this, clearing a big chat scanned the whole table once per message.
	db.Exec(`CREATE INDEX IF NOT EXISTS idx_message_edits_target ON message_edits(target_id)`)
	// Starred Messages and the starred-message sync look these up directly.
	db.Exec(`CREATE INDEX IF NOT EXISTS idx_messages_starred ON messages(timestamp DESC) WHERE starred = 1`)

	// One row per poll and voter: their current selection as comma-separated
	// hex option hashes (see poll_votes.go).
	if _, err := db.Exec(`CREATE TABLE IF NOT EXISTS poll_votes (
		poll_id TEXT NOT NULL,
		voter TEXT NOT NULL,
		selected TEXT NOT NULL DEFAULT '',
		timestamp INTEGER NOT NULL DEFAULT 0,
		PRIMARY KEY (poll_id, voter)
	)`); err != nil {
		return err
	}
	// Group member tags, one row per group and member (the newest wins).
	if _, err := db.Exec(`CREATE TABLE IF NOT EXISTS group_member_labels (
		group_jid TEXT NOT NULL,
		member TEXT NOT NULL,
		label TEXT NOT NULL DEFAULT '',
		set_at INTEGER NOT NULL DEFAULT 0,
		PRIMARY KEY (group_jid, member)
	)`); err != nil {
		return err
	}
	// RSVPs to events, one row per event and responder (the latest answer
	// wins). response is waE2E.EventResponseMessage_EventResponseType.
	if _, err := db.Exec(`CREATE TABLE IF NOT EXISTS event_responses (
		event_id TEXT NOT NULL,
		responder TEXT NOT NULL,
		response INTEGER NOT NULL DEFAULT 0,
		extra_guests INTEGER NOT NULL DEFAULT 0,
		timestamp INTEGER NOT NULL DEFAULT 0,
		PRIMARY KEY (event_id, responder)
	)`); err != nil {
		return err
	}
	if _, err := db.Exec(`CREATE TABLE IF NOT EXISTS reactions (
		message_id TEXT NOT NULL,
		chat_jid TEXT NOT NULL,
		reactor_jid TEXT NOT NULL,
		emoji TEXT NOT NULL DEFAULT '',
		timestamp INTEGER,
		PRIMARY KEY (message_id, reactor_jid)
	)`); err != nil {
		return err
	}
	// UpdateChatSnippet looks up a chat's latest reaction for every chat it
	// refreshes; without this each lookup scanned every reaction stored.
	if _, err := db.Exec(`CREATE INDEX IF NOT EXISTS idx_reactions_chat_timestamp ON reactions(chat_jid, timestamp DESC)`); err != nil {
		return err
	}

	// Per-participant delivery/read status for group messages, so Message Info
	// can show a WhatsApp-style "Read by ..." breakdown. Populated from live
	// receipts and backfilled from history sync, whose group messages carry a
	// per-member receipt list. status/timestamp are the furthest state reached;
	// delivered_at/read_at/played_at keep each state's own time so both "read
	// at" and "delivered at" can be shown.
	if _, err := db.Exec(`CREATE TABLE IF NOT EXISTS message_receipts (
		message_id TEXT NOT NULL,
		participant_jid TEXT NOT NULL,
		status TEXT NOT NULL DEFAULT 'delivered',
		timestamp INTEGER DEFAULT 0,
		PRIMARY KEY (message_id, participant_jid)
	)`); err != nil {
		return err
	}
	db.Exec("ALTER TABLE message_receipts ADD COLUMN delivered_at INTEGER DEFAULT 0")
	db.Exec("ALTER TABLE message_receipts ADD COLUMN read_at INTEGER DEFAULT 0")
	db.Exec("ALTER TABLE message_receipts ADD COLUMN played_at INTEGER DEFAULT 0")

	if _, err := db.Exec(`CREATE TABLE IF NOT EXISTS contacts_cache (
		jid TEXT PRIMARY KEY,
		name TEXT DEFAULT '',
		phone TEXT DEFAULT '',
		is_business INTEGER DEFAULT 0,
		about TEXT DEFAULT '',
		avatar_path TEXT DEFAULT '',
		avatar_id TEXT DEFAULT '',
		is_blocked INTEGER DEFAULT 0,
		updated_at INTEGER DEFAULT 0
	)`); err != nil {
		return err
	}

	// The blocked list as the server gives it (mostly LIDs); see blocklist.go.
	if _, err := db.Exec(`CREATE TABLE IF NOT EXISTS blocked_contacts (jid TEXT PRIMARY KEY)`); err != nil {
		return err
	}

	purgeExpiredStatuses()
	cleanupScratchTempDirs(24 * time.Hour)
	return nil
}

// runOnce runs fn the first time a database sees key, then records it in
// app_settings so later launches skip it.
func runOnce(key string, fn func()) {
	var done int
	if db.QueryRow(`SELECT COUNT(*) FROM app_settings WHERE key = ?`, "migration:"+key).Scan(&done) == nil && done > 0 {
		return
	}
	fn()
	db.Exec(`INSERT OR REPLACE INTO app_settings (key, value) VALUES (?, '1')`, "migration:"+key)
}

// --- messages ---------------------------------------------------------------

// statusRank lets us avoid a late/out-of-order receipt regressing a message's
// status backwards (e.g. a delayed "delivered" arriving after we already know
// it was "read").
func statusRank(status string) int {
	switch status {
	case "pending":
		return 0
	case "sent":
		return 1
	case "delivered":
		return 2
	case "read":
		return 3
	case "played":
		return 4
	default:
		return 1
	}
}

// saveMessage is the single write path for inserting/updating a message row,
// used by history sync, live incoming messages, and everything we send
// ourselves. Using ON CONFLICT throughout means re-delivery / re-sync never
// creates duplicate rows.
func saveMessage(sm StoredMessage) error {
	if sm.Status == "" {
		sm.Status = "sent"
	}
	sentAt := int64(0)
	if sm.FromMe {
		sentAt = sm.Timestamp
	}
	_, err := db.Exec(`INSERT INTO messages
			(id, jid, from_me, text, timestamp, sender_jid, raw, status, media_type, media_mimetype, media_filename, media_path, media_size, quoted_id, quoted_text, quoted_sender, sent_at, album_id, server_id)
		VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
		ON CONFLICT(id) DO UPDATE SET
			-- A message can be delivered more than once (reconnect resync, or a
			-- group message whose two encrypted halves each raise an event), and
			-- the copies can land in either order. Never let a copy we couldn't
			-- make sense of overwrite one we could - keep the whole content set
			-- together so text, raw and media stay consistent.
			text=CASE WHEN excluded.text = '[Unsupported Message]' AND messages.text NOT IN ('', '[Unsupported Message]') THEN messages.text ELSE excluded.text END,
			timestamp=excluded.timestamp,
			sender_jid=excluded.sender_jid,
			raw=CASE WHEN excluded.text = '[Unsupported Message]' AND messages.text NOT IN ('', '[Unsupported Message]') THEN messages.raw ELSE excluded.raw END,
			media_type=CASE WHEN excluded.text = '[Unsupported Message]' AND messages.text NOT IN ('', '[Unsupported Message]') THEN messages.media_type ELSE excluded.media_type END,
			media_mimetype=CASE WHEN excluded.text = '[Unsupported Message]' AND messages.text NOT IN ('', '[Unsupported Message]') THEN messages.media_mimetype ELSE excluded.media_mimetype END,
			media_filename=CASE WHEN excluded.text = '[Unsupported Message]' AND messages.text NOT IN ('', '[Unsupported Message]') THEN messages.media_filename ELSE excluded.media_filename END,
			media_path=CASE WHEN excluded.media_path != '' THEN excluded.media_path ELSE messages.media_path END,
			media_size=CASE WHEN excluded.text = '[Unsupported Message]' AND messages.text NOT IN ('', '[Unsupported Message]') THEN messages.media_size ELSE excluded.media_size END,
			quoted_id=excluded.quoted_id,
			quoted_text=excluded.quoted_text,
			quoted_sender=excluded.quoted_sender,
			sent_at=CASE WHEN excluded.sent_at != 0 THEN excluded.sent_at ELSE messages.sent_at END,
			album_id=CASE WHEN excluded.album_id != '' THEN excluded.album_id ELSE messages.album_id END,
			server_id=CASE WHEN excluded.server_id != 0 THEN excluded.server_id ELSE messages.server_id END`,
		sm.ID, sm.JID, sm.FromMe, sm.Text, sm.Timestamp, sm.SenderJID, sm.Raw, sm.Status,
		sm.MediaType, sm.MediaMimetype, sm.MediaFilename, sm.MediaPath, sm.MediaSize,
		sm.QuotedID, sm.QuotedText, sm.QuotedSender, sentAt, sm.AlbumID, sm.ServerID)
	// Several call sites (local echo of a send, media/status messages) don't
	// check this error themselves, so a write failure here would otherwise
	// vanish silently instead of just losing the message on screen.
	if err != nil {
		log.Println("saveMessage failed for", sm.ID, "in", sm.JID, ":", err)
	}
	return err
}

// updateMessageStatusIfNewer applies a status transition only if it moves the
// message forward (sent -> delivered -> read -> played), for one chat's worth
// of message IDs at once (receipts can cover several messages read together).
func updateMessageStatusIfNewer(ids []string, newStatus string, receiptTS int64) []string {
	if len(ids) == 0 {
		return nil
	}
	newRank := statusRank(newStatus)
	var tsCol string
	switch newStatus {
	case "delivered":
		tsCol = "delivered_at"
	case "read":
		tsCol = "read_at"
	case "played":
		tsCol = "played_at"
	}
	var updated []string
	for _, id := range ids {
		// Receipts for an edit name the edit stanza, not the message it
		// changed - the status belongs to the message the reader actually sees.
		if target := editTargetID(id); target != "" {
			id = target
		}
		if tsCol != "" && receiptTS > 0 {
			db.Exec("UPDATE messages SET "+tsCol+" = ? WHERE id = ? AND "+tsCol+" = 0", receiptTS, id)
		}
		var current string
		err := db.QueryRow("SELECT status FROM messages WHERE id = ?", id).Scan(&current)
		if err != nil {
			continue
		}
		if statusRank(current) >= newRank {
			continue
		}
		if _, err := db.Exec("UPDATE messages SET status = ? WHERE id = ?", newStatus, id); err == nil {
			updated = append(updated, id)
		}
	}
	return updated
}

// updateParticipantReceiptIfNewer records one group member's delivery/read
// status for a set of messages, ignoring a receipt that would regress a
// status already recorded for that member (mirrors updateMessageStatusIfNewer,
// but per-participant instead of per-message).
func updateParticipantReceiptIfNewer(ids []string, participantJID, newStatus string, receiptTS int64) {
	for _, id := range ids {
		if target := editTargetID(id); target != "" {
			id = target
		}
		if err := recordParticipantReceipt(db, id, participantJID, newStatus, receiptTS); err != nil {
			log.Println("Error recording participant receipt:", err)
		}
	}
}

// updateGroupMessageStatuses advances our own group messages' overall status
// from their per-member receipts, the way official WhatsApp draws group ticks:
// "delivered" once every other member has it, "read"/"played" once every other
// member has read/played it. Moving the whole message to "read" on the first
// member's receipt (as for a 1:1 chat) showed blue ticks while most of the
// group hadn't even opened it. Returns the IDs whose status changed and the
// status they now have.
func updateGroupMessageStatuses(ids []string, groupJID string, receiptTS int64) map[string][]string {
	var members int
	db.QueryRow(`SELECT COALESCE(participant_count, 0) FROM chats WHERE jid = ?`, groupJID).Scan(&members)
	others := members - 1
	changed := map[string][]string{}
	for _, id := range ids {
		if target := editTargetID(id); target != "" {
			id = target
		}
		var delivered, read, played int
		if db.QueryRow(`SELECT
				COALESCE(SUM(CASE WHEN status IN ('delivered', 'read', 'played') THEN 1 ELSE 0 END), 0),
				COALESCE(SUM(CASE WHEN status IN ('read', 'played') THEN 1 ELSE 0 END), 0),
				COALESCE(SUM(CASE WHEN status = 'played' THEN 1 ELSE 0 END), 0)
			FROM message_receipts WHERE message_id = ?`, id).Scan(&delivered, &read, &played) != nil {
			continue
		}
		status := ""
		switch {
		case others <= 0:
			// Group size not known yet (the joined-groups list hasn't been
			// fetched this session): never claim more than "delivered".
			if delivered > 0 {
				status = "delivered"
			}
		case played >= others:
			status = "played"
		case read >= others:
			status = "read"
		case delivered >= others:
			status = "delivered"
		}
		if status == "" {
			continue
		}
		for _, updatedID := range updateMessageStatusIfNewer([]string{id}, status, receiptTS) {
			changed[status] = append(changed[status], updatedID)
		}
	}
	return changed
}

type sqlExecer interface {
	Exec(query string, args ...interface{}) (sql.Result, error)
}

// stmtTx is a transaction that parses each distinct statement once and reuses
// it. Bulk writes (history sync, contact refresh) run the same few long
// statements hundreds of times per transaction; a plain tx.Exec re-parses the
// SQL on every call. The prepared statements close with the transaction.
type stmtTx struct {
	tx    *sql.Tx
	stmts map[string]*sql.Stmt
}

func beginStmtTx() (*stmtTx, error) {
	tx, err := db.Begin()
	if err != nil {
		return nil, err
	}
	return &stmtTx{tx: tx, stmts: map[string]*sql.Stmt{}}, nil
}

func (t *stmtTx) Exec(query string, args ...interface{}) (sql.Result, error) {
	stmt, ok := t.stmts[query]
	if !ok {
		var err error
		if stmt, err = t.tx.Prepare(query); err != nil {
			return nil, err
		}
		t.stmts[query] = stmt
	}
	return stmt.Exec(args...)
}

// batchSize caps how many statements one batched write transaction runs, so
// live traffic (and whatsmeow's own store writes) get the write lock back
// regularly during a big history-sync chat or contact refresh.
const batchSize = 500

// writeBatch runs pre-built writes in short transactions. The closures must
// only run SQL on the executor they're given - never call into whatsmeow's
// stores, which hold their own in-memory locks while waiting for SQLite's
// write lock and so deadlock against an open write transaction until the busy
// timeout.
func writeBatch(writes []func(tx sqlExecer)) {
	for start := 0; start < len(writes); start += batchSize {
		end := min(start+batchSize, len(writes))
		tx, err := beginStmtTx()
		if err != nil {
			log.Println("Error starting batched write transaction:", err)
			return
		}
		for _, w := range writes[start:end] {
			w(tx)
		}
		if err := tx.tx.Commit(); err != nil {
			log.Println("Error committing batched write transaction:", err)
		}
	}
}

// recordParticipantReceipt stores one member's delivered/read/played receipt
// for one message. Each state's own time is kept the first time it's seen,
// even when it arrives out of order (a delivery receipt landing after the
// read one); status/timestamp only ever advance.
func recordParticipantReceipt(ex sqlExecer, messageID, participantJID, status string, ts int64) error {
	var col string
	switch status {
	case "delivered":
		col = "delivered_at"
	case "read":
		col = "read_at"
	case "played":
		col = "played_at"
	default:
		return nil
	}
	const rank = `CASE %s WHEN 'played' THEN 4 WHEN 'read' THEN 3 WHEN 'delivered' THEN 2 ELSE 1 END`
	advance := fmt.Sprintf(rank, "excluded.status") + " > " + fmt.Sprintf(rank, "message_receipts.status")
	_, err := ex.Exec(`INSERT INTO message_receipts (message_id, participant_jid, status, timestamp, `+col+`) VALUES (?, ?, ?, ?, ?)
		ON CONFLICT(message_id, participant_jid) DO UPDATE SET
			status = CASE WHEN `+advance+` THEN excluded.status ELSE message_receipts.status END,
			timestamp = CASE WHEN `+advance+` THEN excluded.timestamp ELSE message_receipts.timestamp END,
			`+col+` = CASE WHEN COALESCE(message_receipts.`+col+`, 0) = 0 THEN excluded.`+col+` ELSE message_receipts.`+col+` END`,
		messageID, participantJID, status, ts, ts)
	return err
}

// recordEditStanza remembers that editID carries an edit of targetID. Called
// before the edit goes out, because the delivery receipt can beat the send call
// returning.
func recordEditStanza(editID, targetID, jid string, ts int64) {
	if editID == "" || targetID == "" || editID == targetID {
		return
	}
	db.Exec(`INSERT INTO message_edits (edit_id, target_id, jid, timestamp) VALUES (?, ?, ?, ?)
		ON CONFLICT(edit_id) DO UPDATE SET target_id = excluded.target_id`, editID, targetID, jid, ts)
}

// editTargetID resolves an edit stanza's ID to the message it edited, following
// the chain when a message has been edited more than once. Returns "" for IDs
// that aren't edits.
func editTargetID(editID string) string {
	seen := 0
	current := editID
	for seen < 8 {
		var target string
		if db.QueryRow(`SELECT target_id FROM message_edits WHERE edit_id = ?`, current).Scan(&target) != nil {
			break
		}
		current = target
		seen++
	}
	if current == editID {
		return ""
	}
	return current
}

// editPlainText is the bare new text an edit carries, whichever field it
// arrived in (a text edit, or a caption on a media edit).
func editPlainText(msg *waE2E.Message) string {
	msg = unwrapMessage(msg)
	switch {
	case msg == nil:
		return ""
	case msg.GetConversation() != "":
		return msg.GetConversation()
	case msg.GetExtendedTextMessage() != nil:
		return msg.GetExtendedTextMessage().GetText()
	case msg.GetImageMessage() != nil:
		return msg.GetImageMessage().GetCaption()
	case msg.GetVideoMessage() != nil:
		return msg.GetVideoMessage().GetCaption()
	case msg.GetDocumentMessage() != nil:
		return msg.GetDocumentMessage().GetCaption()
	}
	return ""
}

// mergeEdit applies an edit to the original message instead of replacing it.
// An edit only carries the new text: storing it as the whole message (as this
// used to) threw away a photo's media keys - so it could never be downloaded
// again - and a reply's quote. Returns the raw protobuf to keep and the
// display text for it, or ok=false if there's no usable original to merge
// into (the edit's own content is then all there is).
func mergeEdit(id string, newContent *waE2E.Message) (raw []byte, text string, ok bool) {
	var origRaw []byte
	if db.QueryRow(`SELECT raw FROM messages WHERE id = ?`, id).Scan(&origRaw) != nil || len(origRaw) == 0 {
		return nil, "", false
	}
	orig := &waE2E.Message{}
	if proto.Unmarshal(origRaw, orig) != nil {
		return nil, "", false
	}
	newText := editPlainText(newContent)
	carrier := mediaCarrier(orig)
	switch {
	case carrier.GetImageMessage() != nil:
		carrier.GetImageMessage().Caption = proto.String(newText)
	case carrier.GetVideoMessage() != nil:
		carrier.GetVideoMessage().Caption = proto.String(newText)
	case carrier.GetDocumentMessage() != nil:
		carrier.GetDocumentMessage().Caption = proto.String(newText)
	default:
		inner := unwrapMessage(orig)
		ctx := getContextInfo(inner)
		if inner.GetExtendedTextMessage() != nil {
			inner.GetExtendedTextMessage().Text = proto.String(newText)
		} else if inner.Conversation != nil {
			inner.Conversation = proto.String(newText)
		} else {
			// Something that isn't text at all (a poll, a bot's rich
			// response...): the edit's content replaces it, keeping any quote.
			orig = proto.Clone(newContent).(*waE2E.Message)
			if ctx != nil && orig.GetConversation() != "" {
				orig = &waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{
					Text: proto.String(orig.GetConversation()), ContextInfo: ctx,
				}}
			}
		}
	}
	merged, err := proto.Marshal(orig)
	if err != nil {
		return nil, "", false
	}
	return merged, extractText(orig), true
}

// markMessageEdited records an edit of message id. newContent is the edit's
// payload (may be nil if it couldn't be decrypted, in which case only the text
// is updated); fallbackText is used when there's nothing to merge into.
// Returns the text the message now displays.
func markMessageEdited(id, jid string, newContent *waE2E.Message, fallbackText string, editTime int64) (string, error) {
	text := fallbackText
	var err error
	if newContent == nil {
		_, err = db.Exec(`UPDATE messages SET text = ?, is_edited = 1, edited_at = ? WHERE id = ? AND jid = ?`, text, editTime, id, jid)
	} else if raw, merged, ok := mergeEdit(id, newContent); ok {
		text = merged
		_, err = db.Exec(`UPDATE messages SET text = ?, raw = ?, is_edited = 1, edited_at = ? WHERE id = ? AND jid = ?`, text, raw, editTime, id, jid)
	} else {
		raw, _ := proto.Marshal(newContent)
		_, err = db.Exec(`UPDATE messages SET text = ?, raw = ?, is_edited = 1, edited_at = ? WHERE id = ? AND jid = ?`, text, raw, editTime, id, jid)
	}
	db.Exec(`UPDATE messages SET quoted_text = ? WHERE quoted_id = ?`, text, id)
	UpdateChatSnippet(jid)
	return text, err
}

func markMessageDeleted(id, jid string) error {
	_, err := db.Exec(`UPDATE messages SET is_deleted = 1, text = '' WHERE id = ? AND jid = ?`, id, jid)
	UpdateChatSnippet(jid)
	return err
}

// deleteMessageForMe removes a message entirely from the local cache, for
// "delete for me" (both our own and synced from another linked device via
// events.DeleteForMe). Unlike markMessageDeleted this leaves no tombstone -
// the row just disappears, matching official WhatsApp: a message you (or
// another of your devices) deleted for yourself was never a "this message
// was deleted" placeholder, it's simply gone from your view.
func deleteMessageForMe(id, jid string) error {
	// Through purgeMessages so the row's receipts, edit mappings, poll votes
	// and downloaded media file go with it - deleting just the row and its
	// reactions left the rest (the media file on disk included) behind.
	purgeMessages(`id = ? AND jid = ?`, id, jid)
	UpdateChatSnippet(jid)
	return nil
}

func markMessageStarred(id, jid string, starred bool) error {
	starredInt := 0
	if starred {
		starredInt = 1
	}
	_, err := db.Exec(`UPDATE messages SET starred = ? WHERE id = ? AND jid = ?`, starredInt, id, jid)
	return err
}

// backfillMediaSizes fills in media_size for messages stored before that
// column existed. Their raw protobuf was already kept for other reasons
// (quoting, forwarding, downloading), so the size can be re-derived from it
// without a history resync; the query's own filter means this is a no-op
// after the first run, once every row it can resolve has been resolved.
func backfillMediaSizes() {
	rows, err := db.Query(`SELECT id, raw FROM messages
		WHERE media_type != '' AND COALESCE(media_size, 0) = 0 AND raw IS NOT NULL AND length(raw) > 0`)
	if err != nil {
		return
	}
	type sized struct {
		id   string
		size int64
	}
	var fixed []sized
	for rows.Next() {
		var id string
		var raw []byte
		if rows.Scan(&id, &raw) != nil {
			continue
		}
		msg := &waE2E.Message{}
		if proto.Unmarshal(raw, msg) != nil {
			continue
		}
		if _, _, _, fileSize := mediaInfoFromMessage(msg); fileSize > 0 {
			fixed = append(fixed, sized{id, fileSize})
		}
	}
	rows.Close()
	for _, m := range fixed {
		db.Exec(`UPDATE messages SET media_size = ? WHERE id = ?`, m.size, m.id)
	}
}

// backfillTemplateMessages re-derives the text and media info of messages that
// were stored back when business template / interactive / buttons messages had
// no handling, so they landed as "[Unsupported Message]" with their header
// image unreachable. Their raw protobuf was kept, so re-running the (now
// smarter) extractors over it recovers them in place - no history resync, and
// nothing to do on a DB that has none of them.
func backfillTemplateMessages() {
	rows, err := db.Query(`SELECT id, jid, raw FROM messages
		WHERE text = '[Unsupported Message]' AND raw IS NOT NULL AND length(raw) > 0`)
	if err != nil {
		return
	}
	type recovered struct {
		id, jid, text, mediaType, mimetype, filename string
		fileSize                                     int64
	}
	var fixed []recovered
	// Bookkeeping messages (pins, event RSVPs, ...) that older builds stored
	// as bubbles and that are now known to show nothing.
	var bookkeeping []string
	for rows.Next() {
		var id, jid string
		var raw []byte
		if rows.Scan(&id, &jid, &raw) != nil {
			continue
		}
		msg := &waE2E.Message{}
		if proto.Unmarshal(raw, msg) != nil {
			continue
		}
		text := extractText(msg)
		if text == "[Protocol Message]" {
			bookkeeping = append(bookkeeping, id)
			continue
		}
		if text == "" || text == "[Unsupported Message]" {
			continue
		}
		mediaType, mimetype, filename, fileSize := mediaInfoFromMessage(msg)
		fixed = append(fixed, recovered{id, jid, text, mediaType, mimetype, filename, fileSize})
	}
	rows.Close()

	chats := make(map[string]struct{}, len(fixed))
	for _, msg := range fixed {
		db.Exec(`UPDATE messages SET text = ?, media_type = ?, media_mimetype = ?, media_filename = ?, media_size = ?
			WHERE id = ?`, msg.text, msg.mediaType, msg.mimetype, msg.filename, msg.fileSize, msg.id)
		chats[msg.jid] = struct{}{}
	}
	for _, id := range bookkeeping {
		var jid string
		db.QueryRow(`SELECT jid FROM messages WHERE id = ?`, id).Scan(&jid)
		purgeMessages(`id = ?`, id)
		chats[jid] = struct{}{}
	}
	for jid := range chats {
		UpdateChatSnippet(jid)
	}
}

// backfillAlbumMessages converts album headers that were stored back when they
// had no handling - each one showed up as a stray "[Unsupported Message]"
// bubble next to the photos it announced - into the album_id grouping the
// frontend now uses, then removes the header rows themselves.
// removeKeyDistributionStubs deletes rows that only ever held a group sender
// key, never any content. Older builds stored these as "[Unsupported Message]"
// bubbles, and one of them could even overwrite the real message it shared an
// ID with - the content is unrecoverable from what's left, so drop the stub
// rather than keep showing it.
func removeKeyDistributionStubs() {
	rows, err := db.Query(`SELECT id, jid, COALESCE(sender_jid, ''), raw FROM messages
		WHERE text = '[Unsupported Message]' AND raw IS NOT NULL AND length(raw) > 0`)
	if err != nil {
		return
	}
	type stub struct{ id, jid, senderJID string }
	var stubs []stub
	for rows.Next() {
		var id, jid, senderJID string
		var raw []byte
		if rows.Scan(&id, &jid, &senderJID, &raw) != nil {
			continue
		}
		msg := &waE2E.Message{}
		if proto.Unmarshal(raw, msg) != nil {
			continue
		}
		if isKeyDistributionOnly(msg) {
			stubs = append(stubs, stub{id, jid, senderJID})
		}
	}
	rows.Close()

	chats := make(map[string]struct{}, len(stubs))
	for _, st := range stubs {
		// Ask for the real message once we're connected. If it comes back it
		// lands as a normal message and repopulates the row we're deleting.
		requestMessageResend(st.id, st.jid, st.senderJID)
		db.Exec("DELETE FROM messages WHERE id = ?", st.id)
		chats[st.jid] = struct{}{}
	}
	for jid := range chats {
		UpdateChatSnippet(jid)
	}
}

func backfillAlbumMessages() {
	rows, err := db.Query(`SELECT id, jid, COALESCE(sender_jid, ''), timestamp, raw FROM messages
		WHERE text = '[Unsupported Message]' AND raw IS NOT NULL AND length(raw) > 0`)
	if err != nil {
		return
	}
	type header struct {
		id, jid, senderJID string
		timestamp          int64
		expected           int
	}
	var headers []header
	for rows.Next() {
		var id, jid, senderJID string
		var ts int64
		var raw []byte
		if rows.Scan(&id, &jid, &senderJID, &ts, &raw) != nil {
			continue
		}
		msg := &waE2E.Message{}
		if proto.Unmarshal(raw, msg) != nil {
			continue
		}
		album := unwrapMessage(msg).GetAlbumMessage()
		if album == nil {
			continue
		}
		headers = append(headers, header{id, jid, senderJID, ts,
			int(album.GetExpectedImageCount() + album.GetExpectedVideoCount())})
	}
	rows.Close()

	chats := make(map[string]struct{}, len(headers))
	for _, h := range headers {
		stampAlbumMembers(h.id, h.jid, h.senderJID, h.timestamp, h.expected)
		db.Exec("DELETE FROM messages WHERE id = ?", h.id)
		chats[h.jid] = struct{}{}
	}
	for jid := range chats {
		UpdateChatSnippet(jid)
	}
}

// UpdateChatSnippet recomputes a chat's list preview from its latest real
// message. If a reaction landed after that message (the normal "reacted to
// the last message and nothing followed" case), the reaction preview wins
// instead - otherwise a bulk resync (e.g. UpdateAllChatSnippets on app
// restart/reconnect) would blindly overwrite a still-current reaction
// preview with the plain last message, even though no new message actually
// arrived to justify that. A real new message already clears the reaction
// fields itself via the ON CONFLICT UPDATE in the live-message insert path.
func UpdateChatSnippet(jid string) {
	var msgText, msgSender string
	var msgFromMe, msgDeleted int
	var msgTS int64
	hasMsg := db.QueryRow(`SELECT COALESCE(text, ''), COALESCE(sender_jid, ''), from_me, is_deleted, timestamp
		FROM messages WHERE jid = ? AND (text != '' OR media_type != '' OR (raw IS NOT NULL AND length(raw) > 0))
		ORDER BY timestamp DESC LIMIT 1`, jid).Scan(&msgText, &msgSender, &msgFromMe, &msgDeleted, &msgTS) == nil

	var reactorJID, emoji, reactedText string
	var reactionTS int64
	hasReaction := db.QueryRow(`SELECT r.reactor_jid, r.emoji, r.timestamp, m.text
		FROM reactions r JOIN messages m ON m.id = r.message_id
		WHERE r.chat_jid = ? AND COALESCE(m.is_deleted, 0) = 0 AND COALESCE(m.text, '') != '' AND r.emoji != ''
		ORDER BY r.timestamp DESC LIMIT 1`, jid).Scan(&reactorJID, &emoji, &reactionTS, &reactedText) == nil

	if hasReaction && (!hasMsg || reactionTS > msgTS) {
		fromMe := 0
		if reactorJID == "me" {
			fromMe = 1
		}
		db.Exec(`UPDATE chats SET
				preview_snippet = ?, preview_sender = ?, preview_from_me = ?,
				preview_is_reaction = 1, preview_reaction_emoji = ?
			WHERE jid = ?`, reactedText, reactorJID, fromMe, emoji, jid)
		return
	}

	snippet := ""
	if hasMsg {
		if msgDeleted == 1 {
			if msgFromMe == 1 {
				snippet = "You deleted this message"
			} else {
				snippet = "This message was deleted"
			}
		} else {
			snippet = msgText
		}
	}
	db.Exec(`UPDATE chats SET
			preview_snippet = COALESCE(NULLIF(?, ''), preview_snippet),
			preview_sender = ?,
			preview_from_me = ?,
			preview_is_reaction = 0,
			preview_reaction_emoji = ''
		WHERE jid = ?`, snippet, msgSender, msgFromMe, jid)
}

func UpdateAllChatSnippets() {
	rows, err := db.Query(`SELECT jid FROM chats`)
	if err != nil {
		return
	}
	var jids []string
	for rows.Next() {
		var jid string
		if rows.Scan(&jid) == nil {
			jids = append(jids, jid)
		}
	}
	rows.Close()
	for _, jid := range jids {
		UpdateChatSnippet(jid)
	}
}

func setMessageMediaPath(id, path string) error {
	_, err := db.Exec(`UPDATE messages SET media_path = ? WHERE id = ?`, path, id)
	return err
}

// getMessageRaw loads the stored raw protobuf bytes for a message, along with
// its chat JID and original sender, so it can be re-parsed for downloads,
// quoting, or media re-fetching.
func getMessageRaw(id string) (raw []byte, jid string, senderJID string, fromMe bool, err error) {
	err = db.QueryRow(`SELECT raw, jid, COALESCE(sender_jid, ''), from_me FROM messages WHERE id = ?`, id).
		Scan(&raw, &jid, &senderJID, &fromMe)
	return
}

// messageOrigin is the authoritative from_me/sender_jid of a message we still
// have locally, so quoted-reply display can trust the actual original row
// instead of a denormalized quoted_sender snapshot that may have been
// captured wrong or gone stale.
type messageOrigin struct {
	FromMe    bool
	SenderJID string
}

// lookupMessageOrigins is lookupMessageOrigin for a batch of messages, keyed
// by id; ids we don't have are simply absent.
func lookupMessageOrigins(ids []string) map[string]messageOrigin {
	result := map[string]messageOrigin{}
	// Chunked like getReactionsForMessageIDs, under SQLite's bound-parameter
	// limit.
	const chunk = 900
	for start := 0; start < len(ids); start += chunk {
		part := ids[start:min(start+chunk, len(ids))]
		args := make([]interface{}, len(part))
		for i, id := range part {
			args[i] = id
		}
		placeholders := strings.TrimSuffix(strings.Repeat("?,", len(part)), ",")
		rows, err := db.Query(`SELECT id, from_me, COALESCE(sender_jid, '') FROM messages WHERE id IN (`+placeholders+`)`, args...)
		if err != nil {
			log.Println("Error looking up quoted messages:", err)
			continue
		}
		for rows.Next() {
			var id string
			var origin messageOrigin
			if rows.Scan(&id, &origin.FromMe, &origin.SenderJID) == nil {
				result[id] = origin
			}
		}
		rows.Close()
	}
	return result
}

// getMessageForQuote loads and parses a stored message so it can be used as
// the QuotedMessage in a reply's ContextInfo, along with a short text preview
// for locally caching alongside the new message.
func getMessageForQuote(id string) (msg *waE2E.Message, senderJID string, preview string, err error) {
	var raw []byte
	var text string
	rawErr := db.QueryRow(`SELECT raw, COALESCE(sender_jid, ''), COALESCE(text, '') FROM messages WHERE id = ?`, id).
		Scan(&raw, &senderJID, &text)
	if rawErr != nil {
		err = rawErr
		return
	}
	preview = text
	msg = &waE2E.Message{}
	if len(raw) > 0 {
		if unmarshalErr := proto.Unmarshal(raw, msg); unmarshalErr == nil {
			extracted := extractText(msg)
			if extracted != "" {
				preview = extracted
			}
		} else {
			msg = &waE2E.Message{Conversation: proto.String(text)}
		}
	} else {
		msg = &waE2E.Message{Conversation: proto.String(text)}
	}
	return
}

// getReactionsForMessageIDs returns all current reactions for a batch of
// message IDs, keyed by message ID, for embedding into get_messages results.
func getReactionsForMessageIDs(ids []string) map[string][]ReactionInfo {
	result := map[string][]ReactionInfo{}
	names := map[string]string{}
	// In chunks: a whole chat can hold more IDs than SQLite allows bound
	// parameters in one statement, and the query then failed outright -
	// every reaction in the chat silently went missing.
	const chunk = 900
	for start := 0; start < len(ids); start += chunk {
		part := ids[start:min(start+chunk, len(ids))]
		args := make([]interface{}, len(part))
		for i, id := range part {
			args[i] = id
		}
		placeholders := strings.TrimSuffix(strings.Repeat("?,", len(part)), ",")
		rows, err := db.Query(`SELECT message_id, reactor_jid, emoji, timestamp FROM reactions WHERE message_id IN (`+placeholders+`) AND emoji != ''`, args...)
		if err != nil {
			log.Println("Error loading reactions:", err)
			continue
		}
		for rows.Next() {
			var messageID, reactorJID, emoji string
			var ts int64
			if err := rows.Scan(&messageID, &reactorJID, &emoji, &ts); err == nil {
				name := "You"
				if reactorJID != "me" {
					if cached, ok := names[reactorJID]; ok {
						name = cached
					} else if parsed, perr := types.ParseJID(reactorJID); perr == nil {
						name = resolveDisplayName(parsed)
						names[reactorJID] = name
					} else {
						name = reactorJID
					}
				}
				result[messageID] = append(result[messageID], ReactionInfo{
					ReactorJID:  reactorJID,
					ReactorName: name,
					Emoji:       emoji,
					Timestamp:   ts,
				})
			}
		}
		rows.Close()
	}
	return result
}

// upsertReaction records (or, for an empty emoji, effectively clears) a
// single reactor's reaction to a message. WhatsApp only allows one active
// reaction per person per message, so reactor_jid+message_id is the key.
func upsertReaction(messageID, chatJID, reactorJID, emoji string, ts int64) error {
	if emoji == "" {
		// Only this reactor's row. Our own reactions (from any of our
		// devices) are always stored under "me", so a removal by us already
		// arrives as reactorJID "me" - also matching 'me' here deleted *our*
		// reaction whenever anyone else removed theirs.
		_, err := db.Exec(`DELETE FROM reactions WHERE message_id = ? AND reactor_jid = ?`, messageID, reactorJID)
		return err
	}
	_, err := db.Exec(`INSERT INTO reactions (message_id, chat_jid, reactor_jid, emoji, timestamp) VALUES (?, ?, ?, ?, ?)
		ON CONFLICT(message_id, reactor_jid) DO UPDATE SET emoji = excluded.emoji, timestamp = excluded.timestamp`,
		messageID, chatJID, reactorJID, emoji, ts)
	return err
}

// updateChatReactionPreview mirrors official WhatsApp's chat-list behaviour:
// reacting to a message (even an old one) swaps the list row's preview to
// "<reactor> reacted with <emoji> to: <message>" without bumping the chat's
// position or displayed time - WhatsApp doesn't hand us a reaction
// timestamp for that, so the row keeps showing the last real message's time.
// Clearing a reaction (empty emoji) reverts the row to the actual last
// message via UpdateChatSnippet.
func updateChatReactionPreview(chatJID, reactedMessageID, reactorJID, emoji string, reactorIsFromMe bool) {
	if emoji == "" {
		db.Exec(`UPDATE chats SET preview_is_reaction = 0, preview_reaction_emoji = '' WHERE jid = ?`, chatJID)
		UpdateChatSnippet(chatJID)
		return
	}
	var text string
	var isDeleted int
	err := db.QueryRow(`SELECT COALESCE(text, ''), is_deleted FROM messages WHERE id = ?`, reactedMessageID).Scan(&text, &isDeleted)
	if err != nil || isDeleted == 1 || text == "" {
		return
	}
	fromMe := 0
	if reactorIsFromMe {
		fromMe = 1
	}
	db.Exec(`UPDATE chats SET
			preview_snippet = ?,
			preview_sender = ?,
			preview_from_me = ?,
			preview_is_reaction = 1,
			preview_reaction_emoji = ?
		WHERE jid = ?`, text, reactorJID, fromMe, emoji, chatJID)
}

// --- contacts -----------------------------------------------------------------

// upsertContactCacheRow keeps the local contacts_cache table (used for fast,
// offline-friendly get_contacts/check_contacts responses) in sync. A real
// name is never downgraded to an empty one or to the "+<phone>" placeholder
// callers use when they don't know the name.
func upsertContactCacheRow(jid, name, phone string, isBusiness bool) error {
	return upsertContactCacheRowWith(db, jid, name, phone, isBusiness)
}

func upsertContactCacheRowWith(ex sqlExecer, jid, name, phone string, isBusiness bool) error {
	_, err := ex.Exec(`INSERT INTO contacts_cache (jid, name, phone, is_business, updated_at) VALUES (?, ?, ?, ?, ?)
		ON CONFLICT(jid) DO UPDATE SET
			name = CASE
				WHEN (excluded.name = '' OR excluded.name LIKE '+%')
					AND COALESCE(contacts_cache.name, '') != '' AND contacts_cache.name NOT LIKE '+%'
				THEN contacts_cache.name
				ELSE excluded.name
			END,
			phone = COALESCE(NULLIF(excluded.phone, ''), contacts_cache.phone),
			is_business = excluded.is_business,
			updated_at = excluded.updated_at`,
		jid, name, phone, isBusiness, time.Now().Unix())
	return err
}

// deleteContactCacheRow removes a contact entirely from the local cache
// (e.g. after it's deleted from WhatsApp's own contact list on another
// device). Unlike upsertContactCacheRowWith, this doesn't go through the
// "keep the existing name" guard meant to protect a real name from being
// blanked by a lower-quality update - a removal is an explicit, intentional
// deletion, not a stale sync.
func deleteContactCacheRow(jid string) error {
	_, err := db.Exec(`DELETE FROM contacts_cache WHERE jid = ?`, jid)
	return err
}

func updateContactAvatarCache(jid, avatarPath, avatarID string) error {
	_, err := db.Exec(`INSERT INTO contacts_cache (jid, avatar_path, avatar_id, updated_at) VALUES (?, ?, ?, ?)
		ON CONFLICT(jid) DO UPDATE SET avatar_path=excluded.avatar_path, avatar_id=excluded.avatar_id, updated_at=excluded.updated_at`,
		jid, avatarPath, avatarID, time.Now().Unix())
	return err
}

func updateContactAboutCache(jid, about string) error {
	_, err := db.Exec(`INSERT INTO contacts_cache (jid, about, updated_at) VALUES (?, ?, ?)
		ON CONFLICT(jid) DO UPDATE SET about=excluded.about, updated_at=excluded.updated_at`,
		jid, about, time.Now().Unix())
	return err
}

func getCachedAvatarPath(jid string) string {
	var path string
	db.QueryRow(`SELECT COALESCE(avatar_path, '') FROM contacts_cache WHERE jid = ?`, jid).Scan(&path)
	return path
}

// resolveDisplayName centralizes the "what name do we show for this JID"
// logic that used to be duplicated between broadcastChats and get_chats. It
// resolves LIDs to phone-number JIDs first, then falls back through contact
// name -> push name -> business name -> "+<number>".
func resolveDisplayName(jid types.JID) string {
	if wa() != nil && wa().Store != nil && wa().Store.ID != nil {
		if jid.User == wa().Store.ID.User || (jid.Server == types.HiddenUserServer && jid.User == wa().Store.GetLID().User) {
			return "You"
		}
	}
	// Meta AI's bot JID has no phone number and whatsmeow only learns its
	// pushname ("Meta AI") from the notify attribute on its first reply -
	// until then this fell through to the "+<user>" phone-number fallback
	// below, showing a bogus number for a JID that was never a phone number.
	if jid.User == types.NewMetaAIJID.User && jid.Server == types.NewMetaAIJID.Server {
		return "Meta AI"
	}
	// A channel is named by its own title, which we keep in the chat list.
	if jid.Server == types.NewsletterServer {
		var name string
		if db != nil {
			db.QueryRow(`SELECT COALESCE(name, '') FROM chats WHERE jid = ?`, jid.String()).Scan(&name)
		}
		return cmp.Or(name, "Channel")
	}
	contactJID := jid
	// twin is the same person's other address (their LID for a phone JID,
	// or the reverse). Group members who hide their number are only ever
	// seen as a LID, so their push name is often stored under that alone,
	// while the address-book name is under the phone JID.
	var twin types.JID
	redactedPhone := ""
	if wa() != nil && wa().Store != nil && wa().Store.LIDs != nil {
		switch jid.Server {
		case types.HiddenUserServer:
			if pn, err := wa().Store.LIDs.GetPNForLID(context.Background(), jid); err == nil && !pn.IsEmpty() {
				contactJID = pn
				twin = jid
			}
		case types.DefaultUserServer:
			if lid, err := wa().Store.LIDs.GetLIDForPN(context.Background(), jid); err == nil && !lid.IsEmpty() {
				twin = lid
			}
		}
	}
	if wa() != nil && wa().Store != nil && wa().Store.Contacts != nil {
		pushName, businessName := "", ""
		for _, candidate := range []types.JID{contactJID, twin} {
			if candidate.IsEmpty() {
				continue
			}
			info, err := wa().Store.Contacts.GetContact(context.Background(), candidate)
			if err != nil || !info.Found {
				continue
			}
			if info.FullName != "" {
				return info.FullName
			}
			pushName = cmp.Or(pushName, info.PushName)
			businessName = cmp.Or(businessName, info.BusinessName)
			redactedPhone = cmp.Or(redactedPhone, info.RedactedPhone)
		}
		if name := cmp.Or(pushName, businessName); name != "" {
			return name
		}
	}
	var name, phone string
	if db != nil {
		_ = db.QueryRow(`SELECT name, phone FROM contacts_cache WHERE (jid = ? OR jid = ?) AND (name != '' OR phone != '') LIMIT 1`, jid.String(), contactJID.String()).Scan(&name, &phone)
		if name != "" {
			return name
		}
		if phone != "" {
			return "+" + phone
		}
	}
	// Fall back on contactJID, not jid: for a LID we already resolved to a
	// phone JID above, and "+<number>" beats showing a raw "<n>@lid".
	if contactJID.Server != "g.us" && contactJID.Server != "lid" && contactJID.Server != "" && contactJID.User != "" {
		return "+" + contactJID.User
	}
	// A member who hides their number in a community: WhatsApp gives us the
	// number with most digits masked ("+91∙∙∙∙∙∙∙∙62"), which is what the
	// official apps show too - far better than a raw "<n>@lid".
	if redactedPhone != "" {
		return redactedPhone
	}
	return jid.String()
}

// removeOrphanedDeviceRows deletes whatsmeow rows belonging to devices that no
// longer exist - what ON DELETE CASCADE would have done when each device was
// logged out, had foreign keys been on for that connection (see main.go).
// Each re-pair otherwise left the previous device's message secrets,
// contacts, sessions, etc. behind. Only foreign keys that whatsmeow's own
// schema declares against whatsmeow_device are followed.
func removeOrphanedDeviceRows() {
	rows, err := db.Query(`SELECT m.name, f."from"
		FROM sqlite_master m, pragma_foreign_key_list(m.name) f
		WHERE m.type = 'table' AND f."table" = 'whatsmeow_device'`)
	if err != nil {
		log.Println("Orphan cleanup: listing foreign keys failed:", err)
		return
	}
	type ref struct{ table, column string }
	var refs []ref
	for rows.Next() {
		var r ref
		if rows.Scan(&r.table, &r.column) == nil {
			refs = append(refs, r)
		}
	}
	rows.Close()
	for _, r := range refs {
		res, err := db.Exec(fmt.Sprintf(`DELETE FROM %q WHERE %q NOT IN (SELECT jid FROM whatsmeow_device)`, r.table, r.column))
		if err != nil {
			log.Printf("Orphan cleanup of %s failed: %v", r.table, err)
			continue
		}
		if n, _ := res.RowsAffected(); n > 0 {
			log.Printf("Orphan cleanup: removed %d rows from %s left by old device pairings", n, r.table)
		}
	}
}

func saveChatLockSecretCode(b []byte) error {
	_, err := db.Exec(`INSERT INTO app_settings (key, value) VALUES ('chat_lock_secret_code', ?) ON CONFLICT(key) DO UPDATE SET value=excluded.value`, b)
	return err
}

func getChatLockSecretCode() ([]byte, error) {
	var b []byte
	err := db.QueryRow(`SELECT value FROM app_settings WHERE key = 'chat_lock_secret_code'`).Scan(&b)
	if err == sql.ErrNoRows {
		return nil, nil
	}
	return b, err
}
