package main

import (
	"context"
	"time"

	"go.mau.fi/whatsmeow/proto/waHistorySync"
	"go.mau.fi/whatsmeow/store"
	"go.mau.fi/whatsmeow/types"
)

const displayNameTTL = 30 * time.Second

func (acc *Account) cachedDisplayName(jid types.JID) string {
	key := jid.String()
	now := time.Now()
	acc.displayNames.Lock()
	acc.resetExpiredDisplayNames(now)
	name, ok := acc.displayNames.names[key]
	acc.displayNames.Unlock()
	if ok {
		return name
	}
	name = acc.resolveDisplayName(jid)
	acc.displayNames.Lock()
	if acc.displayNames.names != nil {
		acc.displayNames.names[key] = name
	}
	acc.displayNames.Unlock()
	return name
}

// resetExpiredDisplayNames starts both name caches afresh once they're stale.
// The caller holds acc.displayNames.
func (acc *Account) resetExpiredDisplayNames(now time.Time) {
	if acc.displayNames.names == nil || now.After(acc.displayNames.expires) {
		acc.displayNames.names = make(map[string]string)
		acc.displayNames.search = make(map[string]chatSearchKeys)
		acc.displayNames.expires = now.Add(displayNameTTL)
	}
}

func (acc *Account) invalidateDisplayNames() {
	acc.displayNames.Lock()
	acc.displayNames.names = nil
	acc.displayNames.search = nil
	acc.displayNames.Unlock()
}

// chatSearchKeys is what else a person's chat can be found by in the chat
// list's search, besides the name it shows: WA Web's contact search also
// matches the push name, the verified business name, "Me" for yourself and
// the phone number, including the one behind a LID chat.
type chatSearchKeys struct {
	names []string
	phone string
}

func (acc *Account) cachedSearchKeys(jid types.JID) chatSearchKeys {
	key := jid.String()
	acc.displayNames.Lock()
	acc.resetExpiredDisplayNames(time.Now())
	keys, ok := acc.displayNames.search[key]
	acc.displayNames.Unlock()
	if ok {
		return keys
	}
	keys = acc.resolveSearchKeys(jid)
	acc.displayNames.Lock()
	if acc.displayNames.search != nil {
		acc.displayNames.search[key] = keys
	}
	acc.displayNames.Unlock()
	return keys
}

func (acc *Account) resolveSearchKeys(jid types.JID) chatSearchKeys {
	var keys chatSearchKeys
	jid = jid.ToNonAD()
	if jid.Server != types.DefaultUserServer && jid.Server != types.HiddenUserServer {
		return keys
	}
	cli := acc.wa()
	if cli == nil || cli.Store == nil || cli.Store.LIDs == nil || cli.Store.Contacts == nil {
		return keys
	}
	pn, lid := acc.contactForms(jid)
	// A phone-number chat's number is in its JID already; a LID chat's is not.
	if !pn.IsEmpty() && jid.Server == types.HiddenUserServer {
		keys.phone = pn.User
	}
	if cli.Store.ID != nil && pn.User == cli.Store.ID.User {
		keys.names = append(keys.names, "Me")
	}
	seen := map[string]bool{}
	for _, form := range []types.JID{pn, lid} {
		if form.IsEmpty() {
			continue
		}
		info, err := cli.Store.Contacts.GetContact(context.Background(), form)
		if err != nil || !info.Found {
			continue
		}
		for _, name := range []string{info.PushName, info.BusinessName} {
			if name != "" && !seen[name] {
				seen[name] = true
				keys.names = append(keys.names, name)
			}
		}
	}
	return keys
}

type chatSettingsRow struct {
	mutedUntil int64
	pinned     bool
	archived   bool
}

func (acc *Account) allChatSettings() map[string]chatSettingsRow {
	out := map[string]chatSettingsRow{}
	if acc.wa() == nil || acc.wa().Store == nil || acc.wa().Store.ID == nil {
		return out
	}
	rows, err := acc.db.Query(`SELECT chat_jid, muted_until, pinned, archived FROM whatsmeow_chat_settings WHERE our_jid = ?`,
		acc.wa().Store.ID.String())
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

func (acc *Account) chatSettingsFromMap(all map[string]chatSettingsRow, jid types.JID) (types.LocalChatSettings, bool) {
	row, ok := all[jid.String()]
	if !ok && acc.wa() != nil && acc.wa().Store != nil && acc.wa().Store.LIDs != nil {
		var twin types.JID
		switch jid.Server {
		case types.DefaultUserServer:
			twin, _ = acc.wa().Store.LIDs.GetLIDForPN(context.Background(), jid)
		case types.HiddenUserServer:
			twin, _ = acc.wa().Store.LIDs.GetPNForLID(context.Background(), jid)
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

func (acc *Account) healStaleUnreadCounts() {
	acc.db.Exec(`UPDATE chats SET unread_count = 0
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

func (acc *Account) businessContactJIDs() map[string]bool {
	jids := map[string]bool{}
	rows, err := acc.db.Query(`SELECT jid FROM contacts_cache WHERE COALESCE(is_business, 0) = 1`)
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

func (acc *Account) isBusinessChat(jid types.JID, business map[string]bool) bool {
	if jid.Server != types.DefaultUserServer && jid.Server != types.HiddenUserServer {
		return false
	}
	if business[jid.ToNonAD().String()] {
		return true
	}
	if jid.Server != types.HiddenUserServer {
		return false
	}
	cli := acc.wa()
	if cli == nil || cli.Store == nil || cli.Store.LIDs == nil {
		return false
	}
	pn, err := cli.Store.LIDs.GetPNForLID(context.Background(), jid.ToNonAD())
	return err == nil && !pn.IsEmpty() && business[pn.ToNonAD().String()]
}

// markBusinessContact records a person as a business account when one of
// their messages carries a verified business name, which a business's
// messages do even when the contact list never said so.
func (acc *Account) markBusinessContact(jid types.JID) {
	key := jid.ToNonAD().String()
	if _, seen := acc.knownBusinesses.LoadOrStore(key, true); seen {
		return
	}
	res, err := acc.db.Exec(`INSERT INTO contacts_cache (jid, name, phone, is_business, updated_at) VALUES (?, '', '', 1, ?)
		ON CONFLICT(jid) DO UPDATE SET is_business = 1 WHERE COALESCE(contacts_cache.is_business, 0) = 0`,
		key, time.Now().Unix())
	if err != nil {
		acc.knownBusinesses.Delete(key)
		return
	}
	if n, _ := res.RowsAffected(); n > 0 {
		acc.broadcastChats()
	}
}
