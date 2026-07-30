package main

import (
	"context"
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
	db.Exec("UPDATE messages SET sent_at = timestamp WHERE from_me = 1 AND (sent_at IS NULL OR sent_at = 0)")
	// Rows synced before revoked-stub handling existed: no text, no media and
	// no raw protobuf means the message was revoked, so surface it as deleted
	// instead of "[Media]".
	db.Exec(`UPDATE messages SET is_deleted = 1 WHERE COALESCE(is_deleted, 0) = 0 AND COALESCE(text, '') = ''
		AND COALESCE(media_type, '') = '' AND COALESCE(media_path, '') = '' AND (raw IS NULL OR length(raw) = 0)`)

	if _, err := db.Exec(`CREATE INDEX IF NOT EXISTS idx_messages_jid_timestamp ON messages(jid, timestamp DESC);`); err != nil {
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
			(id, jid, from_me, text, timestamp, sender_jid, raw, status, media_type, media_mimetype, media_filename, media_path, quoted_id, quoted_text, quoted_sender, sent_at)
		VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
		ON CONFLICT(id) DO UPDATE SET
			text=excluded.text,
			timestamp=excluded.timestamp,
			sender_jid=excluded.sender_jid,
			raw=excluded.raw,
			media_type=excluded.media_type,
			media_mimetype=excluded.media_mimetype,
			media_filename=excluded.media_filename,
			media_path=CASE WHEN excluded.media_path != '' THEN excluded.media_path ELSE messages.media_path END,
			quoted_id=excluded.quoted_id,
			quoted_text=excluded.quoted_text,
			quoted_sender=excluded.quoted_sender,
			sent_at=CASE WHEN excluded.sent_at != 0 THEN excluded.sent_at ELSE messages.sent_at END`,
		sm.ID, sm.JID, sm.FromMe, sm.Text, sm.Timestamp, sm.SenderJID, sm.Raw, sm.Status,
		sm.MediaType, sm.MediaMimetype, sm.MediaFilename, sm.MediaPath,
		sm.QuotedID, sm.QuotedText, sm.QuotedSender, sentAt)
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

func markMessageEdited(id, jid, newText string, newRaw []byte, editTime int64) error {
	_, err := db.Exec(`UPDATE messages SET text = ?, raw = ?, is_edited = 1, edited_at = ? WHERE id = ? AND jid = ?`, newText, newRaw, editTime, id, jid)
	return err
}

func markMessageDeleted(id, jid string) error {
	_, err := db.Exec(`UPDATE messages SET is_deleted = 1, text = '' WHERE id = ? AND jid = ?`, id, jid)
	UpdateChatSnippet(jid)
	return err
}

func UpdateChatSnippet(jid string) {
	query := `
		UPDATE chats 
		SET preview_snippet = COALESCE(
			(SELECT 
				CASE 
					WHEN is_deleted = 1 THEN 
						CASE WHEN from_me = 1 THEN 'You deleted this message' ELSE 'This message was deleted' END 
					ELSE 
						NULLIF(text, '')
				END 
			 FROM messages 
			 WHERE messages.jid = chats.jid 
			 ORDER BY timestamp DESC 
			 LIMIT 1), 
			chats.preview_snippet)
		WHERE jid = ?
	`
	db.Exec(query, jid)
}

func UpdateAllChatSnippets() {
	query := `
		UPDATE chats 
		SET preview_snippet = COALESCE(
			(SELECT 
				CASE 
					WHEN is_deleted = 1 THEN 
						CASE WHEN from_me = 1 THEN 'You deleted this message' ELSE 'This message was deleted' END 
					ELSE 
						NULLIF(text, '')
				END 
			 FROM messages 
			 WHERE messages.jid = chats.jid 
			 ORDER BY timestamp DESC 
			 LIMIT 1), 
			chats.preview_snippet)
	`
	db.Exec(query)
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

// --- contacts -----------------------------------------------------------------

// upsertContactCacheRow keeps the local contacts_cache table (used for fast,
// offline-friendly get_contacts/check_contacts responses) in sync.
func upsertContactCacheRow(jid, name, phone string, isBusiness bool) error {
	_, err := db.Exec(`INSERT INTO contacts_cache (jid, name, phone, is_business, updated_at) VALUES (?, ?, ?, ?, ?)
		ON CONFLICT(jid) DO UPDATE SET name=excluded.name, phone=excluded.phone, is_business=excluded.is_business, updated_at=excluded.updated_at`,
		jid, name, phone, isBusiness, time.Now().Unix())
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
	var name string
	if db != nil {
		_ = db.QueryRow(`SELECT name FROM contacts WHERE (jid = ? OR jid = ?) AND name != '' AND name NOT LIKE '%@%' LIMIT 1`, jid.String(), contactJID.String()).Scan(&name)
		if name != "" {
			return name
		}
		_ = db.QueryRow(`SELECT sender_name FROM messages WHERE (sender_jid = ? OR sender_jid = ?) AND sender_name != '' AND sender_name NOT LIKE '%@%' ORDER BY timestamp DESC LIMIT 1`, jid.String(), contactJID.String()).Scan(&name)
		if name != "" {
			return name
		}
	}
	if jid.Server != "g.us" && jid.Server != "lid" && jid.Server != "" && jid.User != "" {
		return "+" + jid.User
	}
	return jid.String()
}
