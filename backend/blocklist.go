package main

import (
	"context"
	"sort"
	"strings"

	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
)

// The blocked-contacts list. WhatsApp keeps it keyed by LID, while chats here
// are mostly keyed by phone JID, and one person can be in it under several
// LIDs (ones their number used to go by). So the list is stored as the server
// gave it, phone numbers included, and sent to the frontend as one entry per
// person carrying every address they're blocked under (whatsapp-rust hit the
// same mismatch: a phone-number lookup never matched a LID-keyed block).

type jidTwin func(types.JID) types.JID

func (acc *Account) storeTwin(jid types.JID) types.JID {
	c := acc.wa()
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

type blockedRow struct {
	JID types.JID
	PN  types.JID
}

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
	JID   string   `json:"jid"`
	Alts  []string `json:"alts"`
	Name  string   `json:"name"`
	Phone string   `json:"phone"`
}

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

func blockedAddressesOf(rows []blockedRow, jid types.JID, twin jidTwin) []types.JID {
	target := blockedRow{JID: jid}.person(twin)
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

func (acc *Account) loadBlockedRows() []blockedRow {
	rows, err := acc.db.Query(`SELECT jid, pn FROM blocked_contacts`)
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

func (acc *Account) storeBlockedRows(rows []blockedRow) {
	tx, err := acc.db.Begin()
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

func (acc *Account) blocklistPayload() map[string]interface{} {
	return map[string]interface{}{
		"entries": buildBlockedEntries(acc.loadBlockedRows(), acc.storeTwin, acc.resolveDisplayName),
	}
}

func (acc *Account) refreshBlocklistOnConnect() {
	acc.blocklistSyncMu.Lock()
	synced := acc.blocklistSyncedFor != nil && acc.blocklistSyncedFor == acc.wa()
	acc.blocklistSyncMu.Unlock()
	if !synced {
		acc.refreshBlocklist()
	}
}

func (acc *Account) refreshBlocklist() {
	c := acc.wa()
	if c == nil || !c.IsLoggedIn() {
		return
	}
	acc.blocklistSyncMu.Lock()
	dhash := ""
	if acc.blocklistSyncedFor == c {
		dhash = acc.blocklistDHash
	}
	acc.blocklistSyncMu.Unlock()
	bl, err := c.GetBlocklist(context.Background(), dhash)
	if err != nil {
		return
	}
	if bl == nil {
		return
	}
	acc.blocklistSyncMu.Lock()
	acc.blocklistSyncedFor, acc.blocklistDHash = c, bl.DHash
	acc.blocklistSyncMu.Unlock()
	rows := make([]blockedRow, 0, len(bl.Entries))
	for _, entry := range bl.Entries {
		rows = append(rows, blockedRow{JID: entry.JID, PN: entry.PN})
	}
	acc.blocklistMu.Lock()
	acc.storeBlockedRows(rows)
	acc.blocklistMu.Unlock()
	acc.broadcastEvent("blocklist", acc.blocklistPayload())
}

func (acc *Account) setBlocked(jid types.JID, block bool) error {
	jid = jid.ToNonAD()
	targets := []types.JID{jid}
	action := events.BlocklistChangeActionBlock
	if !block {
		action = events.BlocklistChangeActionUnblock
		if listed := blockedAddressesOf(acc.loadBlockedRows(), jid, acc.storeTwin); len(listed) > 0 {
			targets = listed
		}
	}
	for _, target := range targets {
		if _, err := acc.wa().UpdateBlocklist(context.Background(), target, action); err != nil {
			return err
		}
	}
	acc.refreshBlocklist()
	return nil
}
