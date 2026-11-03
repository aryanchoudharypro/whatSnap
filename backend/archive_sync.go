package main

import (
	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waCommon"
	"go.mau.fi/whatsmeow/proto/waSyncAction"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

// Archived chats, the way WA Web keeps them (WAWebArchiveChatSync,
// WAWebMsgRangeContainmentHandlers, WAWebArchiveSettingSync).
//
// An archive mutation carries the message range the chat had when it was
// archived. When a newer message comes in and "Keep chats archived" is off,
// every device takes the chat out of the archive by itself; nobody sends an
// unarchive mutation for it. So the phone's last archive mutation can still
// say archived for a chat the phone shows unarchived, and whatsmeow's own
// chat settings store (which just keeps that last mutation) can't be used for
// the archive state. archive_actions keeps the last mutation with its range
// instead, and refreshArchivedChats works the state out from it.

const keepChatsArchivedKey = "keep_chats_archived"

func initArchiveActions() {
	db.Exec(`CREATE TABLE IF NOT EXISTS archive_actions (
		jid TEXT PRIMARY KEY,
		archived INTEGER NOT NULL,
		range_ts INTEGER NOT NULL DEFAULT 0,
		changed_at INTEGER NOT NULL DEFAULT 0
	)`)
	// Chats archived before this table existed: nothing says when they were
	// archived, so they count as archived up to their newest message now and
	// come out with the next new one.
	runOnce("seed_archive_actions_v1", func() {
		db.Exec(`INSERT OR IGNORE INTO archive_actions (jid, archived, range_ts, changed_at)
			SELECT jid, 1, COALESCE(timestamp, 0), 0 FROM chats WHERE archived = 1`)
		rows, err := db.Query(`SELECT chat_jid FROM whatsmeow_chat_settings WHERE archived = 1`)
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
		for _, raw := range jids {
			parsed, err := types.ParseJID(raw)
			if err != nil {
				continue
			}
			jid := preferPN(parsed).String()
			db.Exec(`INSERT OR IGNORE INTO archive_actions (jid, archived, range_ts, changed_at)
				SELECT ?, 1, COALESCE((SELECT timestamp FROM chats WHERE jid = ?), 0), 0`, jid, jid)
		}
	})
}

// keepChatsArchived is the phone's "Keep chats archived" (setting_unarchiveChats
// holds the opposite). Like WA Web, whatSnap only follows the phone's choice
// and never changes it, and a setting never heard about counts as off.
func keepChatsArchived() bool {
	var keep int
	db.QueryRow(`SELECT CAST(value AS INTEGER) FROM app_settings WHERE key = ?`, keepChatsArchivedKey).Scan(&keep)
	return keep == 1
}

func noteUnarchiveChatsSetting(v *events.UnarchiveChatsSetting) {
	if v.Action == nil || v.Action.UnarchiveChats == nil {
		return
	}
	db.Exec(`INSERT OR REPLACE INTO app_settings (key, value) VALUES (?, ?)`, keepChatsArchivedKey, boolToInt(!v.Action.GetUnarchiveChats()))
	broadcastChats()
}

// noteArchiveAction stores an archive or unarchive of jid made at changedAt
// (unix seconds), unless a newer one is already stored.
func noteArchiveAction(jid string, archived bool, msgRange *waSyncAction.SyncActionMessageRange, changedAt int64) {
	// Our own messages after the newest incoming one are listed by name;
	// they belong to the range too.
	rangeTS := msgRange.GetLastMessageTimestamp()
	for _, m := range msgRange.GetMessages() {
		rangeTS = max(rangeTS, m.GetTimestamp())
	}
	db.Exec(`INSERT INTO archive_actions (jid, archived, range_ts, changed_at) VALUES (?, ?, ?, ?)
		ON CONFLICT(jid) DO UPDATE SET archived = excluded.archived, range_ts = excluded.range_ts, changed_at = excluded.changed_at
		WHERE excluded.changed_at >= archive_actions.changed_at`,
		jid, boolToInt(archived), rangeTS, changedAt)
}

// refreshArchivedChats sets chats.archived from the last archive mutation of
// each chat: an archived chat stays archived while "Keep chats archived" is
// on or nothing newer than its archive range has come in. Chats with no
// mutation keep what history sync said.
func refreshArchivedChats() {
	db.Exec(`UPDATE chats SET archived = CASE
			WHEN a.archived = 1 AND (? = 1 OR COALESCE((SELECT MAX(m.timestamp) FROM messages m
				WHERE m.jid = chats.jid AND COALESCE(m.media_type, '') NOT IN ('system', 'call')
					AND m.id NOT LIKE 'system\_%' ESCAPE '\' AND m.id NOT LIKE 'call:%'), 0) <= a.range_ts) THEN 1
			ELSE 0 END
		FROM archive_actions a WHERE a.jid = chats.jid`, boolToInt(keepChatsArchived()))
}

// chatMessageRange builds the message range of a chat-wide mutation (archive,
// mark read, clear, delete) like WA Web's constructMessageRange: the time of
// the newest message from someone else, every message of ours since then and
// the other messages at that same time, and the newest system message time
// when that is later. The phone only applies the mutation to the whole chat
// when this range covers every message it has, so naming just the newest
// message, as whatsmeow does, isn't enough when several share its second.
func chatMessageRange(jid string) *waSyncAction.SyncActionMessageRange {
	const realMessage = `COALESCE(media_type, '') NOT IN ('system', 'call') AND id NOT LIKE 'system\_%' ESCAPE '\' AND id NOT LIKE 'call:%'`
	remote := jid
	if parsed, err := types.ParseJID(jid); err == nil {
		remote = syncdChatJID(parsed).String()
	}
	isGroup := false
	if parsed, err := types.ParseJID(jid); err == nil {
		isGroup = parsed.Server == types.GroupServer
	}
	var lastIncoming int64
	hasIncoming := db.QueryRow(`SELECT MAX(timestamp) FROM messages WHERE jid = ? AND from_me = 0 AND `+realMessage, jid).Scan(&lastIncoming) == nil && lastIncoming > 0
	rows, err := db.Query(`SELECT id, timestamp, from_me, COALESCE(NULLIF(wire_sender, ''), sender_jid, '') FROM messages
		WHERE jid = ? AND `+realMessage+` AND timestamp >= ? AND (from_me = 1 OR timestamp = ?)
		ORDER BY timestamp DESC LIMIT 1000`, jid, lastIncoming, lastIncoming)
	msgRange := &waSyncAction.SyncActionMessageRange{}
	var newest int64
	if err == nil {
		for rows.Next() {
			var id, sender string
			var ts int64
			var fromMe bool
			if rows.Scan(&id, &ts, &fromMe, &sender) != nil {
				continue
			}
			newest = max(newest, ts)
			key := &waCommon.MessageKey{RemoteJID: proto.String(remote), ID: proto.String(id), FromMe: proto.Bool(fromMe)}
			if isGroup && !fromMe && sender != "" {
				key.Participant = proto.String(sender)
			}
			msgRange.Messages = append(msgRange.Messages, &waSyncAction.SyncActionMessage{Key: key, Timestamp: proto.Int64(ts)})
		}
		rows.Close()
	}
	switch {
	case hasIncoming:
		msgRange.LastMessageTimestamp = proto.Int64(lastIncoming)
	case newest > 0:
		msgRange.LastMessageTimestamp = proto.Int64(newest)
	}
	var lastSystem int64
	db.QueryRow(`SELECT COALESCE(MAX(timestamp), 0) FROM messages WHERE jid = ? AND COALESCE(media_type, '') = 'system'`, jid).Scan(&lastSystem)
	if lastSystem > 0 && lastSystem > msgRange.GetLastMessageTimestamp() {
		if msgRange.LastMessageTimestamp == nil {
			msgRange.LastMessageTimestamp = proto.Int64(lastSystem)
		} else {
			msgRange.LastSystemMessageTimestamp = proto.Int64(lastSystem)
		}
	}
	return msgRange
}

// withChatRange puts jid's message range into every chat-wide action of patch.
func withChatRange(patch appstate.PatchInfo, jid string) appstate.PatchInfo {
	msgRange := chatMessageRange(jid)
	if msgRange.LastMessageTimestamp == nil && len(msgRange.Messages) == 0 {
		return patch
	}
	for _, m := range patch.Mutations {
		v := m.Value
		switch {
		case v.GetArchiveChatAction() != nil:
			v.ArchiveChatAction.MessageRange = msgRange
		case v.GetMarkChatAsReadAction() != nil:
			v.MarkChatAsReadAction.MessageRange = msgRange
		case v.GetDeleteChatAction() != nil:
			v.DeleteChatAction.MessageRange = msgRange
		case v.GetClearChatAction() != nil:
			v.ClearChatAction.MessageRange = msgRange
		}
	}
	return patch
}

// rangeCoversChat says whether a range from another device names every
// message jid has from its start on (compareMessageRanges: the incoming range
// encloses ours), so nothing came in here that the other device hadn't seen.
// A missing range covers everything.
func rangeCoversChat(jid string, msgRange *waSyncAction.SyncActionMessageRange) bool {
	if msgRange == nil || (msgRange.LastMessageTimestamp == nil && len(msgRange.Messages) == 0) {
		return true
	}
	named := map[string]bool{}
	for _, m := range msgRange.GetMessages() {
		named[m.GetKey().GetID()] = true
	}
	rows, err := db.Query(`SELECT id FROM messages WHERE jid = ? AND timestamp >= ?
		AND COALESCE(media_type, '') NOT IN ('system', 'call') AND id NOT LIKE 'system\_%' ESCAPE '\' AND id NOT LIKE 'call:%'`,
		jid, msgRange.GetLastMessageTimestamp())
	if err != nil {
		return true
	}
	defer rows.Close()
	for rows.Next() {
		var id string
		if rows.Scan(&id) == nil && !named[id] {
			return false
		}
	}
	return true
}

// purgeChatRange removes what a clear or delete chat mutation from another
// device covers (WAWebMsgRangeContainmentHandlers): the messages up to its
// range and the ones it names. Anything newer stays, as it does on the phone,
// so a delete that was made before the chat carried on doesn't wipe the rest.
// It says whether the chat has no messages left.
func purgeChatRange(jid string, msgRange *waSyncAction.SyncActionMessageRange) bool {
	if msgRange == nil || (msgRange.LastMessageTimestamp == nil && len(msgRange.Messages) == 0) {
		purgeMessages("jid = ?", jid)
	} else {
		limit := max(msgRange.GetLastMessageTimestamp(), msgRange.GetLastSystemMessageTimestamp())
		purgeMessages("jid = ? AND timestamp <= ?", jid, limit)
		for _, m := range msgRange.GetMessages() {
			if id := m.GetKey().GetID(); id != "" {
				purgeMessages("jid = ? AND id = ?", jid, id)
			}
		}
	}
	var left int
	db.QueryRow(`SELECT COUNT(*) FROM messages WHERE jid = ?`, jid).Scan(&left)
	return left == 0
}
