package main

import (
	"context"
	"sync"
	"time"

	"go.mau.fi/whatsmeow/proto/waHistorySync"
	"go.mau.fi/whatsmeow/store"
	"go.mau.fi/whatsmeow/types"
)

const displayNameTTL = 30 * time.Second

var displayNames struct {
	sync.Mutex
	names   map[string]string
	expires time.Time
}

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

func invalidateDisplayNames() {
	displayNames.Lock()
	displayNames.names = nil
	displayNames.Unlock()
}

type chatSettingsRow struct {
	mutedUntil int64
	pinned     bool
	archived   bool
}

func allChatSettings() map[string]chatSettingsRow {
	out := map[string]chatSettingsRow{}
	if wa() == nil || wa().Store == nil || wa().Store.ID == nil {
		return out
	}
	rows, err := db.Query(`SELECT chat_jid, muted_until, pinned, archived FROM whatsmeow_chat_settings WHERE our_jid = ?`,
		wa().Store.ID.String())
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

func chatSettingsFromMap(all map[string]chatSettingsRow, jid types.JID) (types.LocalChatSettings, bool) {
	row, ok := all[jid.String()]
	if !ok && wa() != nil && wa().Store != nil && wa().Store.LIDs != nil {
		var twin types.JID
		switch jid.Server {
		case types.DefaultUserServer:
			twin, _ = wa().Store.LIDs.GetLIDForPN(context.Background(), jid)
		case types.HiddenUserServer:
			twin, _ = wa().Store.LIDs.GetPNForLID(context.Background(), jid)
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

func healStaleUnreadCounts() {
	db.Exec(`UPDATE chats SET unread_count = 0
		WHERE unread_count > 0 AND COALESCE(marked_unread, 0) = 0
			AND EXISTS (SELECT 1 FROM messages m WHERE m.jid = chats.jid AND m.from_me = 0)
			AND NOT EXISTS (SELECT 1 FROM messages m WHERE m.jid = chats.jid AND m.from_me = 0
				AND m.status NOT IN ('read', 'played') AND COALESCE(m.read_at, 0) = 0)`)
}

// historyConversationIsChat reports whether a history-sync conversation is a
// chat to list. The phone sends more than its chat list: entries that carry
// only a contact's privacy token, identity key or name, for people never
// chatted with. A chat has a timestamp (however old, and whether or not any
// of its messages came along), or messages, or state of its own. Groups are
// always taken; which ones we're still in is settled against the server's
// group list (populateChatsFromStoreOnce).
func historyConversationIsChat(conv *waHistorySync.Conversation, jid types.JID) bool {
	if jid.Server == types.GroupServer {
		return true
	}
	return conv.GetConversationTimestamp() > 0 || conv.GetLastMsgTimestamp() > 0 || len(conv.GetMessages()) > 0 ||
		conv.GetPinned() > 0 || conv.GetArchived() || conv.GetMarkedAsUnread() || conv.GetUnreadCount() > 0 ||
		historyMuteActive(conv.GetMuteEndTime())
}

func historyMuteActive(end uint64) bool {
	e := int64(end)
	if e < 0 {
		return true
	}
	if e > 1e12 {
		e /= 1000
	}
	return e > time.Now().Unix()
}

func businessContactJIDs() map[string]bool {
	jids := map[string]bool{}
	rows, err := db.Query(`SELECT jid FROM contacts_cache WHERE COALESCE(is_business, 0) = 1`)
	if err != nil {
		return jids
	}
	defer rows.Close()
	for rows.Next() {
		var jid string
		if rows.Scan(&jid) == nil {
			jids[jid] = true
		}
	}
	return jids
}

func isBusinessChat(jid types.JID, business map[string]bool) bool {
	if jid.Server != types.DefaultUserServer && jid.Server != types.HiddenUserServer {
		return false
	}
	if business[jid.ToNonAD().String()] {
		return true
	}
	if jid.Server != types.HiddenUserServer {
		return false
	}
	cli := wa()
	if cli == nil || cli.Store == nil || cli.Store.LIDs == nil {
		return false
	}
	pn, err := cli.Store.LIDs.GetPNForLID(context.Background(), jid.ToNonAD())
	return err == nil && !pn.IsEmpty() && business[pn.ToNonAD().String()]
}

var knownBusinesses sync.Map

// markBusinessContact records a person as a business account when one of
// their messages carries a verified business name, which a business's
// messages do even when the contact list never said so.
func markBusinessContact(jid types.JID) {
	key := jid.ToNonAD().String()
	if _, seen := knownBusinesses.LoadOrStore(key, true); seen {
		return
	}
	res, err := db.Exec(`INSERT INTO contacts_cache (jid, name, phone, is_business, updated_at) VALUES (?, '', '', 1, ?)
		ON CONFLICT(jid) DO UPDATE SET is_business = 1 WHERE COALESCE(contacts_cache.is_business, 0) = 0`,
		key, time.Now().Unix())
	if err != nil {
		knownBusinesses.Delete(key)
		return
	}
	if n, _ := res.RowsAffected(); n > 0 {
		broadcastChats()
	}
}
