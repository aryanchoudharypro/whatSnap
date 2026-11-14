package main

import (
	"context"
	"fmt"
	"log"
	"net"
	"sort"
	"strings"
	"time"

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
		addresses := []types.JID{row.JID.ToNonAD()}
		// Legacy PN-keyed lists still need to match an open LID chat. For
		// migrated lists, include only LIDs actually returned by the server:
		// an old blocked LID may reference a number now owned by another LID.
		if row.JID.Server == types.DefaultUserServer {
			addresses = append(addresses, twin(person))
		}
		for _, address := range addresses {
			if address.IsEmpty() || address.String() == key {
				continue
			}
			alt := address.ToNonAD().String()
			found := false
			for _, existing := range entries[i].Alts {
				found = found || existing == alt
			}
			if !found {
				entries[i].Alts = append(entries[i].Alts, alt)
			}
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

func (acc *Account) loadBlockedRows() ([]blockedRow, error) {
	rows, err := acc.db.Query(`SELECT jid, pn FROM blocked_contacts`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []blockedRow
	for rows.Next() {
		var jid, pn string
		if err := rows.Scan(&jid, &pn); err != nil {
			return nil, err
		}
		row := blockedRow{}
		if row.JID, err = types.ParseJID(jid); err != nil {
			return nil, err
		}
		if pn != "" {
			row.PN, _ = types.ParseJID(pn)
		}
		out = append(out, row)
	}
	return out, rows.Err()
}

func (acc *Account) storeBlockedRows(rows []blockedRow) error {
	tx, err := acc.db.Begin()
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if _, err := tx.Exec(`DELETE FROM blocked_contacts`); err != nil {
		return err
	}
	for _, row := range rows {
		pn := ""
		if !row.PN.IsEmpty() {
			pn = row.PN.ToNonAD().String()
		}
		if _, err := tx.Exec(`INSERT OR REPLACE INTO blocked_contacts (jid, pn) VALUES (?, ?)`, row.JID.ToNonAD().String(), pn); err != nil {
			return err
		}
	}
	return tx.Commit()
}

func (acc *Account) blocklistPayload() (map[string]interface{}, error) {
	rows, err := acc.loadBlockedRows()
	if err != nil {
		return nil, err
	}
	return map[string]interface{}{
		"entries": buildBlockedEntries(rows, acc.storeTwin, acc.resolveDisplayName),
	}, nil
}

func (acc *Account) sendBlocklist(conn net.Conn) {
	acc.blocklistSyncMu.Lock()
	defer acc.blocklistSyncMu.Unlock()
	payload, err := acc.blocklistPayload()
	if err != nil {
		log.Printf("Couldn't load blocked contacts: %v", err)
		return // A failed read must not replace the UI's list with an empty one.
	}
	acc.sendEvent(conn, "blocklist", payload)
}

func (acc *Account) refreshBlocklistOnConnect() {
	acc.blocklistSyncMu.Lock()
	defer acc.blocklistSyncMu.Unlock()
	c := acc.wa()
	if c != nil && c.IsLoggedIn() && acc.blocklistSyncedFor != c {
		if err := acc.refreshBlocklistLocked(c); err != nil {
			log.Printf("Couldn't refresh blocked contacts: %v", err)
		}
	}
}

func (acc *Account) refreshBlocklist() {
	// Keep the request, persistence and broadcast in one critical section.
	// A server notification and a local action can otherwise fetch different
	// snapshots concurrently and apply them in the opposite order.
	acc.blocklistSyncMu.Lock()
	defer acc.blocklistSyncMu.Unlock()
	c := acc.wa()
	if c == nil || !c.IsLoggedIn() {
		return
	}
	if err := acc.refreshBlocklistLocked(c); err != nil {
		log.Printf("Couldn't refresh blocked contacts: %v", err)
	}
}

// The caller holds blocklistSyncMu, including for mutations below.
func (acc *Account) refreshBlocklistLocked(c *whatsmeow.Client) error {
	dhash := ""
	if acc.blocklistSyncedFor == c {
		dhash = acc.blocklistDHash
	}
	ctx, cancel := context.WithTimeout(acc.ctx, 30*time.Second)
	defer cancel()
	bl, err := c.GetBlocklist(ctx, dhash)
	if err != nil {
		return err
	}
	if bl == nil {
		return nil // A matching dhash means unchanged, not an empty blocklist.
	}
	return acc.applyBlocklistLocked(c, bl)
}

func (acc *Account) applyBlocklistLocked(c *whatsmeow.Client, bl *types.Blocklist) error {
	if c != acc.wa() || acc.ctx.Err() != nil {
		return fmt.Errorf("account changed while updating blocked contacts")
	}
	previous, err := acc.loadBlockedRows()
	if err != nil {
		return err
	}
	rows := make([]blockedRow, 0, len(bl.Entries))
	for _, entry := range bl.Entries {
		rows = append(rows, blockedRow{JID: entry.JID, PN: entry.PN})
	}
	if err := acc.storeBlockedRows(rows); err != nil {
		return fmt.Errorf("couldn't save WhatsApp's blocked contacts: %w", err)
	}
	wasSynced := acc.blocklistSyncedFor == c
	// Advance the hash only after commit, so a failed save can be fetched again.
	acc.blocklistSyncedFor, acc.blocklistDHash = c, bl.DHash
	acc.broadcastEvent("blocklist", map[string]interface{}{
		"entries": buildBlockedEntries(rows, acc.storeTwin, acc.resolveDisplayName),
	})
	if wasSynced {
		acc.noteBlocklistChanges(previous, rows)
	}
	return nil
}

func (acc *Account) setBlocked(jid types.JID, block bool) error {
	acc.blocklistSyncMu.Lock()
	defer acc.blocklistSyncMu.Unlock()
	c := acc.wa()
	if c == nil || !c.IsLoggedIn() {
		return fmt.Errorf("WhatsApp is not connected")
	}
	if acc.blocklistSyncedFor != c {
		if err := acc.refreshBlocklistLocked(c); err != nil {
			return err
		}
	}
	rows, err := acc.loadBlockedRows()
	if err != nil {
		return err
	}
	jid = jid.ToNonAD()
	targets := []types.JID{jid}
	action := events.BlocklistChangeActionBlock
	if !block {
		action = events.BlocklistChangeActionUnblock
		if listed := blockedAddressesOf(rows, jid, acc.storeTwin); len(listed) > 0 {
			targets = listed
		}
	}
	ctx, cancel := context.WithTimeout(acc.ctx, 30*time.Second)
	defer cancel()
	for _, target := range targets {
		if c != acc.wa() || ctx.Err() != nil {
			return fmt.Errorf("account changed while updating blocked contacts")
		}
		bl, err := c.UpdateBlocklist(ctx, target, action)
		if err != nil {
			return err
		}
		if bl == nil {
			bl, err = c.GetBlocklist(ctx, "")
			if err != nil {
				return err
			}
		}
		if bl == nil {
			return fmt.Errorf("WhatsApp didn't return the updated blocked contacts")
		}
		// Use the server's mutation response rather than discard it and race
		// another GET. Persist partial successes too when unblocking old LIDs.
		if err := acc.applyBlocklistLocked(c, bl); err != nil {
			return err
		}
	}
	confirmed, err := acc.loadBlockedRows()
	if err != nil {
		return err
	}
	if (len(blockedAddressesOf(confirmed, jid, acc.storeTwin)) > 0) != block {
		return fmt.Errorf("WhatsApp hasn't confirmed the requested block status")
	}
	return nil
}

func (acc *Account) noteBlocklistChanges(previous, current []blockedRow) {
	people := func(rows []blockedRow) map[types.JID][]types.JID {
		out := make(map[types.JID][]types.JID)
		for _, row := range rows {
			person := row.person(acc.storeTwin)
			out[person] = append(out[person], row.JID.ToNonAD())
		}
		return out
	}
	before, after := people(previous), people(current)
	note := func(person types.JID, addresses []types.JID, blocked bool) {
		text := "You unblocked this contact."
		if blocked {
			text = "You blocked this contact."
		}
		// Only existing conversations get a notice. Prefer the canonical chat
		// but retain LID-only chats when the phone number hasn't been learned.
		for _, chat := range append([]types.JID{person}, addresses...) {
			var exists int
			if err := acc.db.QueryRow(`SELECT COUNT(*) FROM chats WHERE jid = ?`, chat.String()).Scan(&exists); err != nil || exists == 0 {
				continue
			}
			now := time.Now()
			id := fmt.Sprintf("system_%s_%d_block%s", chat.User, now.UnixNano(), quietSystemSuffix)
			acc.insertQuietSystemMessage(chat.String(), id, text, now)
			return
		}
	}
	for person, addresses := range after {
		if _, existed := before[person]; !existed {
			note(person, addresses, true)
		}
	}
	for person, addresses := range before {
		if _, exists := after[person]; !exists {
			note(person, addresses, false)
		}
	}
}
