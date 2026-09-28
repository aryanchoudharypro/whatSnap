package main

import (
	"context"
	"sync"
	"time"

	"go.mau.fi/whatsmeow/store"
	"go.mau.fi/whatsmeow/types"
)

// The chat list is rebuilt and re-sent after nearly every event, so the work
// per chat has to be close to nothing. Resolving each person's name (address
// book, push name, LID twin, contacts cache) and reading each chat's synced
// settings one query at a time made every rebuild thousands of queries.

// displayNameTTL bounds how stale a cached name can get if some path that
// changes a name forgets to invalidate the cache.
const displayNameTTL = 30 * time.Second

var displayNames struct {
	sync.Mutex
	names   map[string]string
	expires time.Time
}

// cachedDisplayName is resolveDisplayName, remembered for a while.
func cachedDisplayName(jid types.JID) string {
	key := jid.String()
	now := time.Now()
	displayNames.Lock()
	if displayNames.names == nil || now.After(displayNames.expires) {
		displayNames.names = make(map[string]string)
		displayNames.expires = now.Add(displayNameTTL)
	}
	name, ok := displayNames.names[key]
	displayNames.Unlock()
	if ok {
		return name
	}
	name = resolveDisplayName(jid)
	displayNames.Lock()
	if displayNames.names != nil {
		displayNames.names[key] = name
	}
	displayNames.Unlock()
	return name
}

// invalidateDisplayNames drops every cached name. Called wherever a name can
// change: push names, address-book edits, business names, LID mappings,
// contact refreshes.
func invalidateDisplayNames() {
	displayNames.Lock()
	displayNames.names = nil
	displayNames.Unlock()
}

// chatSettingsRow is one chat's synced mute/pin/archive state.
type chatSettingsRow struct {
	mutedUntil int64
	pinned     bool
	archived   bool
}

// allChatSettings reads every chat's synced settings in one query, keyed by
// chat JID, instead of one lookup (plus a LID-twin retry) per chat.
func allChatSettings() map[string]chatSettingsRow {
	out := map[string]chatSettingsRow{}
	if client == nil || client.Store == nil || client.Store.ID == nil {
		return out
	}
	rows, err := db.Query(`SELECT chat_jid, muted_until, pinned, archived FROM whatsmeow_chat_settings WHERE our_jid = ?`,
		client.Store.ID.String())
	if err != nil {
		return out
	}
	defer rows.Close()
	for rows.Next() {
		var jid string
		var row chatSettingsRow
		if rows.Scan(&jid, &row.mutedUntil, &row.pinned, &row.archived) == nil {
			out[jid] = row
		}
	}
	return out
}

// chatSettingsFromMap is chatSettingsFor against a map from allChatSettings.
// A 1:1 chat's settings can be stored under its LID rather than its phone
// JID, so the twin is tried too (from whatsmeow's in-memory LID cache).
func chatSettingsFromMap(all map[string]chatSettingsRow, jid types.JID) (types.LocalChatSettings, bool) {
	row, ok := all[jid.String()]
	if !ok && client != nil && client.Store != nil && client.Store.LIDs != nil {
		var twin types.JID
		switch jid.Server {
		case types.DefaultUserServer:
			twin, _ = client.Store.LIDs.GetLIDForPN(context.Background(), jid)
		case types.HiddenUserServer:
			twin, _ = client.Store.LIDs.GetPNForLID(context.Background(), jid)
		}
		if !twin.IsEmpty() {
			row, ok = all[twin.String()]
		}
	}
	if !ok {
		return types.LocalChatSettings{}, false
	}
	settings := types.LocalChatSettings{Found: true, Pinned: row.pinned, Archived: row.archived}
	switch {
	case row.mutedUntil < 0:
		settings.MutedUntil = store.MutedForever
	case row.mutedUntil > 0:
		settings.MutedUntil = time.Unix(row.mutedUntil, 0)
	}
	return settings, true
}

// healStaleUnreadCounts zeroes the unread count of chats whose every incoming
// message has been read - counts history sync carried over for chats read
// since. A chat the user marked unread on purpose keeps its count; this used
// to run for each chat on every chat-list rebuild, and undid "Mark as unread"
// within a fraction of a second.
func healStaleUnreadCounts() {
	db.Exec(`UPDATE chats SET unread_count = 0
		WHERE unread_count > 0 AND COALESCE(marked_unread, 0) = 0
			AND EXISTS (SELECT 1 FROM messages m WHERE m.jid = chats.jid AND m.from_me = 0)
			AND NOT EXISTS (SELECT 1 FROM messages m WHERE m.jid = chats.jid AND m.from_me = 0
				AND m.status NOT IN ('read', 'played') AND COALESCE(m.read_at, 0) = 0)`)
}
