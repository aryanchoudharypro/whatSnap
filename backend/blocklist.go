package main

import (
	"context"
	"sort"
	"strings"
	"sync"

	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
)

// The blocked-contacts list. WhatsApp keeps it keyed by LID, while chats here
// are mostly keyed by phone JID, so the list is stored exactly as the server
// gave it and every entry is sent to the frontend with both of the person's
// addresses (whatsapp-rust hit the same mismatch: a phone-number lookup never
// matched a LID-keyed block).

// jidTwin returns the same person's other address (their LID for a phone
// JID, or the reverse), or an empty JID when it isn't known.
type jidTwin func(types.JID) types.JID

func storeTwin(jid types.JID) types.JID {
	c := wa()
	if c == nil || c.Store == nil || c.Store.LIDs == nil {
		return types.EmptyJID
	}
	switch jid.Server {
	case types.HiddenUserServer:
		if pn, err := c.Store.LIDs.GetPNForLID(context.Background(), jid); err == nil {
			return pn
		}
	case types.DefaultUserServer:
		if lid, err := c.Store.LIDs.GetLIDForPN(context.Background(), jid); err == nil {
			return lid
		}
	}
	return types.EmptyJID
}

type blockedEntry struct {
	// JID is the address chats use: the phone JID when it's known.
	JID string `json:"jid"`
	// Alt is their other address, empty when unknown.
	Alt   string `json:"alt"`
	Name  string `json:"name"`
	Phone string `json:"phone"`
}

// buildBlockedEntries turns the stored list into one entry per person, named
// and sorted for display.
func buildBlockedEntries(stored []types.JID, twin jidTwin, nameFor func(types.JID) string) []blockedEntry {
	entries := []blockedEntry{}
	seen := map[string]bool{}
	for _, jid := range stored {
		jid = jid.ToNonAD()
		other := twin(jid).ToNonAD()
		if jid.Server == types.HiddenUserServer && !other.IsEmpty() {
			jid, other = other, jid
		}
		if seen[jid.String()] {
			continue
		}
		seen[jid.String()] = true
		entry := blockedEntry{JID: jid.String(), Name: nameFor(jid)}
		if !other.IsEmpty() {
			entry.Alt = other.String()
			seen[entry.Alt] = true
		}
		if jid.Server == types.DefaultUserServer {
			entry.Phone = "+" + jid.User
		}
		entries = append(entries, entry)
	}
	sort.SliceStable(entries, func(i, j int) bool {
		return strings.ToLower(entries[i].Name) < strings.ToLower(entries[j].Name)
	})
	return entries
}

// withBlockChange applies one block or unblock to the stored list. An unblock
// removes both of the person's addresses, whichever one the list holds.
func withBlockChange(stored []types.JID, jid types.JID, block bool, twin jidTwin) []types.JID {
	jid = jid.ToNonAD()
	other := twin(jid).ToNonAD()
	out := make([]types.JID, 0, len(stored)+1)
	for _, s := range stored {
		if s.ToNonAD() == jid || (!other.IsEmpty() && s.ToNonAD() == other) {
			continue
		}
		out = append(out, s)
	}
	if block {
		out = append(out, jid)
	}
	return out
}

var blocklistMu sync.Mutex

func loadBlockedJIDs() []types.JID {
	rows, err := db.Query(`SELECT jid FROM blocked_contacts`)
	if err != nil {
		return nil
	}
	defer rows.Close()
	var jids []types.JID
	for rows.Next() {
		var s string
		if rows.Scan(&s) != nil {
			continue
		}
		if jid, err := types.ParseJID(s); err == nil {
			jids = append(jids, jid)
		}
	}
	return jids
}

func storeBlockedJIDs(jids []types.JID) {
	tx, err := db.Begin()
	if err != nil {
		return
	}
	tx.Exec(`DELETE FROM blocked_contacts`)
	for _, jid := range jids {
		tx.Exec(`INSERT OR IGNORE INTO blocked_contacts (jid) VALUES (?)`, jid.ToNonAD().String())
	}
	tx.Commit()
}

func blocklistPayload() map[string]interface{} {
	return map[string]interface{}{
		"entries": buildBlockedEntries(loadBlockedJIDs(), storeTwin, resolveDisplayName),
	}
}

// replaceBlocklist stores the server's whole list and tells the frontend.
func replaceBlocklist(jids []types.JID) {
	blocklistMu.Lock()
	storeBlockedJIDs(jids)
	blocklistMu.Unlock()
	broadcastEvent("blocklist", blocklistPayload())
}

// refreshBlocklist asks the server for the list. Blocks made on the phone
// before this device was linked only ever arrive this way.
func refreshBlocklist() {
	c := wa()
	if c == nil || !c.IsLoggedIn() {
		return
	}
	bl, err := c.GetBlocklist(context.Background())
	if err != nil {
		return
	}
	replaceBlocklist(bl.JIDs)
}

// handleBlocklistEvent applies a change another device made.
func handleBlocklistEvent(evt *events.Blocklist) {
	// "modify" carries no changes: the whole list has to be asked for again.
	if evt.Action == events.BlocklistActionModify || len(evt.Changes) == 0 {
		refreshBlocklist()
		return
	}
	blocklistMu.Lock()
	stored := loadBlockedJIDs()
	for _, change := range evt.Changes {
		stored = withBlockChange(stored, change.JID, change.Action == events.BlocklistChangeActionBlock, storeTwin)
	}
	storeBlockedJIDs(stored)
	blocklistMu.Unlock()
	broadcastEvent("blocklist", blocklistPayload())
}

// setBlocked blocks or unblocks someone on the server, then updates the list.
func setBlocked(jid types.JID, block bool) error {
	action := events.BlocklistChangeActionUnblock
	if block {
		action = events.BlocklistChangeActionBlock
	}
	if _, err := wa().UpdateBlocklist(context.Background(), jid.ToNonAD(), action); err != nil {
		return err
	}
	// Applied here first so the frontend hears at once, then checked against
	// the server's own list rather than trusting the shape of its answer.
	blocklistMu.Lock()
	storeBlockedJIDs(withBlockChange(loadBlockedJIDs(), jid, block, storeTwin))
	blocklistMu.Unlock()
	broadcastEvent("blocklist", blocklistPayload())
	go refreshBlocklist()
	return nil
}
