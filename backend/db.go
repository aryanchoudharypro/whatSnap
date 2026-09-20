package main

import (
	"context"
	"database/sql"
	"fmt"
	"log"
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

	if _, err := db.Exec(`CREATE TABLE IF NOT EXISTS app_settings (
		key TEXT PRIMARY KEY,
		value BLOB
	)`); err != nil {
		return err
	}

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
	// An edit is delivered as its own stanza with its own message ID, so every
	// receipt for it names an ID no message row has. This maps those IDs back
	// to the message being edited.
	db.Exec(`CREATE TABLE IF NOT EXISTS message_edits (
		edit_id TEXT PRIMARY KEY,
		target_id TEXT NOT NULL,
		jid TEXT NOT NULL,
		timestamp INTEGER DEFAULT 0
	)`)
	db.Exec("DELETE FROM messages WHERE jid = 'status@broadcast' AND timestamp <= ?", time.Now().Add(-24*time.Hour).Unix())
	db.Exec("UPDATE messages SET sent_at = timestamp WHERE from_me = 1 AND (sent_at IS NULL OR sent_at = 0)")
	// Excludes is_deleted=1 rows: those are deliberate "this message was
	// deleted" tombstones (a revoke stub synced from history, or a live
	// revoke of a message whose content we never had) - empty text and no
	// raw is their normal, intended state, matching official WhatsApp
	// keeping the tombstone after a "delete for everyone". Only "delete for
	// me" removes the row outright, and that already happens immediately via
	// deleteMessageForMe, not through this startup sweep.
	db.Exec("DELETE FROM messages WHERE COALESCE(is_deleted, 0) = 0 AND (text = '' OR text IS NULL OR text = '[Unsupported Message]' OR text = '[Protocol Message]') AND (media_type IS NULL OR media_type = '') AND (raw IS NULL OR length(raw) = 0)")
	db.Exec("DELETE FROM chats WHERE jid NOT IN (SELECT DISTINCT jid FROM messages WHERE (text != '' AND text IS NOT NULL AND text != '[Unsupported Message]' AND text != '[Protocol Message]') OR (media_type != '' AND media_type IS NOT NULL) OR (raw IS NOT NULL AND length(raw) > 0) OR COALESCE(is_deleted, 0) = 1) AND pinned = 0 AND muted = 0 AND archived = 0 AND COALESCE(is_community, 0) = 0")
	backfillTemplateMessages()
	backfillAlbumMessages()
	removeKeyDistributionStubs()

	if _, err := db.Exec(`CREATE INDEX IF NOT EXISTS idx_messages_jid_timestamp ON messages(jid, timestamp DESC);`); err != nil {
		return err
	}
	// History sync's LID->phone remap rewrites sender_jid by value; without
	// this it scanned the whole messages table on nearly every chunk.
	if _, err := db.Exec(`CREATE INDEX IF NOT EXISTS idx_messages_sender_jid ON messages(sender_jid)`); err != nil {
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

	return nil
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
			(id, jid, from_me, text, timestamp, sender_jid, raw, status, media_type, media_mimetype, media_filename, media_path, quoted_id, quoted_text, quoted_sender, sent_at, album_id)
		VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
			quoted_id=excluded.quoted_id,
			quoted_text=excluded.quoted_text,
			quoted_sender=excluded.quoted_sender,
			sent_at=CASE WHEN excluded.sent_at != 0 THEN excluded.sent_at ELSE messages.sent_at END,
			album_id=CASE WHEN excluded.album_id != '' THEN excluded.album_id ELSE messages.album_id END`,
		sm.ID, sm.JID, sm.FromMe, sm.Text, sm.Timestamp, sm.SenderJID, sm.Raw, sm.Status,
		sm.MediaType, sm.MediaMimetype, sm.MediaFilename, sm.MediaPath,
		sm.QuotedID, sm.QuotedText, sm.QuotedSender, sentAt, sm.AlbumID)
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

func markMessageEdited(id, jid, newText string, newRaw []byte, editTime int64) error {
	_, err := db.Exec(`UPDATE messages SET text = ?, raw = ?, is_edited = 1, edited_at = ? WHERE id = ? AND jid = ?`, newText, newRaw, editTime, id, jid)
	db.Exec(`UPDATE messages SET quoted_text = ? WHERE quoted_id = ?`, newText, id)
	UpdateChatSnippet(jid)
	return err
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
	db.Exec(`DELETE FROM reactions WHERE message_id = ?`, id)
	_, err := db.Exec(`DELETE FROM messages WHERE id = ? AND jid = ?`, id, jid)
	UpdateChatSnippet(jid)
	return err
}

func markMessageStarred(id, jid string, starred bool) error {
	starredInt := 0
	if starred {
		starredInt = 1
	}
	_, err := db.Exec(`UPDATE messages SET starred = ? WHERE id = ? AND jid = ?`, starredInt, id, jid)
	return err
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
	}
	var fixed []recovered
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
		if text == "" || text == "[Unsupported Message]" {
			continue
		}
		mediaType, mimetype, filename := mediaInfoFromMessage(msg)
		fixed = append(fixed, recovered{id, jid, text, mediaType, mimetype, filename})
	}
	rows.Close()

	chats := make(map[string]struct{}, len(fixed))
	for _, msg := range fixed {
		db.Exec(`UPDATE messages SET text = ?, media_type = ?, media_mimetype = ?, media_filename = ?
			WHERE id = ?`, msg.text, msg.mediaType, msg.mimetype, msg.filename, msg.id)
		chats[msg.jid] = struct{}{}
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
	if len(ids) == 0 {
		return result
	}
	placeholders := ""
	args := make([]interface{}, len(ids))
	for i, id := range ids {
		if i > 0 {
			placeholders += ","
		}
		placeholders += "?"
		args[i] = id
	}
	rows, err := db.Query(`SELECT message_id, reactor_jid, emoji, timestamp FROM reactions WHERE message_id IN (`+placeholders+`) AND emoji != ''`, args...)
	if err != nil {
		return result
	}
	defer rows.Close()
	for rows.Next() {
		var messageID, reactorJID, emoji string
		var ts int64
		if err := rows.Scan(&messageID, &reactorJID, &emoji, &ts); err == nil {
			name := "You"
			if reactorJID != "me" {
				if parsed, perr := types.ParseJID(reactorJID); perr == nil {
					name = resolveDisplayName(parsed)
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
	return result
}

// upsertReaction records (or, for an empty emoji, effectively clears) a
// single reactor's reaction to a message. WhatsApp only allows one active
// reaction per person per message, so reactor_jid+message_id is the key.
func upsertReaction(messageID, chatJID, reactorJID, emoji string, ts int64) error {
	if emoji == "" {
		_, err := db.Exec(`DELETE FROM reactions WHERE message_id = ? AND (reactor_jid = ? OR reactor_jid = 'me')`, messageID, reactorJID)
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

func setContactBlocked(jid string, blocked bool) error {
	_, err := db.Exec(`INSERT INTO contacts_cache (jid, is_blocked, updated_at) VALUES (?, ?, ?)
		ON CONFLICT(jid) DO UPDATE SET is_blocked=excluded.is_blocked, updated_at=excluded.updated_at`,
		jid, blocked, time.Now().Unix())
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
	if client != nil && client.Store != nil && client.Store.ID != nil {
		if jid.User == client.Store.ID.User {
			return "You"
		}
	}
	contactJID := jid
	if jid.Server == "lid" {
		if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), jid); err == nil && !pn.IsEmpty() {
			contactJID = pn
		}
	}
	if client != nil && client.Store != nil && client.Store.Contacts != nil {
		if info, err := client.Store.Contacts.GetContact(context.Background(), contactJID); err == nil && info.Found {
			if info.FullName != "" {
				return info.FullName
			} else if info.PushName != "" {
				return info.PushName
			} else if info.BusinessName != "" {
				return info.BusinessName
			}
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
