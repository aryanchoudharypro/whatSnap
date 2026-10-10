package main

import (
	"context"
	"sort"
	"strings"
	"sync"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
)

// The blocked-contacts list. WhatsApp keeps it keyed by LID, while chats here
// are mostly keyed by phone JID, and one person can be in it under several
// LIDs (ones their number used to go by). So the list is stored as the server
// gave it, phone numbers included, and sent to the frontend as one entry per
// person carrying every address they're blocked under (whatsapp-rust hit the
// same mismatch: a phone-number lookup never matched a LID-keyed block).

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

// blockedRow is one item of the server's list: the address blocked, and the
// phone JID the server gave with it (empty if it gave none).
type blockedRow struct {
	JID types.JID
	PN  types.JID
}

// person is the address a blocked row is shown and matched under: the phone
// JID whenever one is known.
func (r blockedRow) person(twin jidTwin) types.JID {
	if !r.PN.IsEmpty() {
		return r.PN.ToNonAD()
	}
	if r.JID.Server == types.HiddenUserServer {
		if pn := twin(r.JID.ToNonAD()); !pn.IsEmpty() {
			return pn.ToNonAD()
		}
	}
	return r.JID.ToNonAD()
}

type blockedEntry struct {
	// JID is the address chats use: the phone JID when it's known.
	JID string `json:"jid"`
	// Alts are the other addresses they're blocked under.
	Alts  []string `json:"alts"`
	Name  string   `json:"name"`
	Phone string   `json:"phone"`
}

// buildBlockedEntries turns the stored list into one entry per person, named
// and sorted for display.
func buildBlockedEntries(rows []blockedRow, twin jidTwin, nameFor func(types.JID) string) []blockedEntry {
	entries := []blockedEntry{}
	index := map[string]int{}
	for _, row := range rows {
		person := row.person(twin)
		key := person.String()
		i, ok := index[key]
		if !ok {
			i = len(entries)
			index[key] = i
			entry := blockedEntry{JID: key, Alts: []string{}, Name: nameFor(person)}
			if person.Server == types.DefaultUserServer {
				entry.Phone = "+" + person.User
			}
			entries = append(entries, entry)
		}
		if alt := row.JID.ToNonAD().String(); alt != key {
			entries[i].Alts = append(entries[i].Alts, alt)
		}
	}
	sort.SliceStable(entries, func(i, j int) bool {
		return strings.ToLower(entries[i].Name) < strings.ToLower(entries[j].Name)
	})
	return entries
}

// blockedAddressesOf returns every address the list holds for the person
// behind jid: unblocking someone means unblocking all of them, or they stay
// listed under an old LID.
func blockedAddressesOf(rows []blockedRow, jid types.JID, twin jidTwin) []types.JID {
	target := blockedRow{JID: jid}.person(twin)
	// The list itself may be the only place that pairs this LID with a number.
	for _, row := range rows {
		if row.JID.ToNonAD() == jid.ToNonAD() {
			target = row.person(twin)
			break
		}
	}
	var out []types.JID
	for _, row := range rows {
		if row.person(twin) == target {
			out = append(out, row.JID.ToNonAD())
		}
	}
	return out
}

var blocklistMu sync.Mutex

func loadBlockedRows() []blockedRow {
	rows, err := db.Query(`SELECT jid, pn FROM blocked_contacts`)
	if err != nil {
		return nil
	}
	defer rows.Close()
	var out []blockedRow
	for rows.Next() {
		var jid, pn string
		if rows.Scan(&jid, &pn) != nil {
			continue
		}
		row := blockedRow{}
		if row.JID, err = types.ParseJID(jid); err != nil {
			continue
		}
		if pn != "" {
			row.PN, _ = types.ParseJID(pn)
		}
		out = append(out, row)
	}
	return out
}

func storeBlockedRows(rows []blockedRow) {
	tx, err := db.Begin()
	if err != nil {
		return
	}
	tx.Exec(`DELETE FROM blocked_contacts`)
	for _, row := range rows {
		pn := ""
		if !row.PN.IsEmpty() {
			pn = row.PN.ToNonAD().String()
		}
		tx.Exec(`INSERT OR REPLACE INTO blocked_contacts (jid, pn) VALUES (?, ?)`, row.JID.ToNonAD().String(), pn)
	}
	tx.Commit()
}

func blocklistPayload() map[string]interface{} {
	return map[string]interface{}{
		"entries": buildBlockedEntries(loadBlockedRows(), storeTwin, resolveDisplayName),
	}
}

// The hash of the list as last fetched over this session, which the server
// compares against its own so an unchanged list isn't sent again (WA Web's
// WAWebGetBlocklistJob does the same).
var (
	blocklistSyncMu    sync.Mutex
	blocklistSyncedFor *whatsmeow.Client
	blocklistDHash     string
)

// refreshBlocklistOnConnect fetches the list the first time a session
// connects. Blocks made on the phone before this device was linked only ever
// arrive this way. After that the server announces every change (a blocklist
// notification, or an account_sync dirty bit for ones missed while offline),
// so a reconnect asks for nothing, as in the official apps.
func refreshBlocklistOnConnect() {
	blocklistSyncMu.Lock()
	synced := blocklistSyncedFor != nil && blocklistSyncedFor == wa()
	blocklistSyncMu.Unlock()
	if !synced {
		refreshBlocklist()
	}
}

// refreshBlocklist asks the server for the list, stores it and tells the
// frontend.
func refreshBlocklist() {
	c := wa()
	if c == nil || !c.IsLoggedIn() {
		return
	}
	blocklistSyncMu.Lock()
	dhash := ""
	if blocklistSyncedFor == c {
		dhash = blocklistDHash
	}
	blocklistSyncMu.Unlock()
	bl, err := c.GetBlocklist(context.Background(), dhash)
	if err != nil {
		return
	}
	if bl == nil {
		// The server's list has the hash we sent: nothing changed.
		return
	}
	blocklistSyncMu.Lock()
	blocklistSyncedFor, blocklistDHash = c, bl.DHash
	blocklistSyncMu.Unlock()
	rows := make([]blockedRow, 0, len(bl.Entries))
	for _, entry := range bl.Entries {
		rows = append(rows, blockedRow{JID: entry.JID, PN: entry.PN})
	}
	blocklistMu.Lock()
	storeBlockedRows(rows)
	blocklistMu.Unlock()
	broadcastEvent("blocklist", blocklistPayload())
}

// setBlocked blocks or unblocks someone on the server, then fetches the list
// again rather than trusting the shape of the server's answer.
func setBlocked(jid types.JID, block bool) error {
	jid = jid.ToNonAD()
	targets := []types.JID{jid}
	action := events.BlocklistChangeActionBlock
	if !block {
		action = events.BlocklistChangeActionUnblock
		if listed := blockedAddressesOf(loadBlockedRows(), jid, storeTwin); len(listed) > 0 {
			targets = listed
		}
	}
	for _, target := range targets {
		if _, err := wa().UpdateBlocklist(context.Background(), target, action); err != nil {
			return err
		}
	}
	refreshBlocklist()
	return nil
}
