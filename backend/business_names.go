package main

import (
	"context"
	"log"
	"strings"

	"go.mau.fi/whatsmeow/types"
)

// lookUpBusinessNames asks WhatsApp for the verified business name of every
// 1:1 chat still shown as a bare number, such as an offers or shopping bot
// whose chat came in through history sync with no name. The phone shows
// those names without the number being saved; they come from the usync
// "business" field, which GetUserInfo asks for. whatsmeow then raises a
// BusinessName event for each one it learns, and that names the chat.
// Each number is asked about once per run.
func (acc *Account) lookUpBusinessNames() {
	cli := acc.wa()
	if cli == nil || cli.Store == nil || cli.Store.ID == nil {
		return
	}
	rows, err := acc.db.Query(`SELECT jid FROM chats WHERE jid LIKE '%@s.whatsapp.net' AND (name IS NULL OR name = '' OR name LIKE '+%')`)
	if err != nil {
		return
	}
	var jids []types.JID
	for rows.Next() {
		var s string
		if rows.Scan(&s) != nil {
			continue
		}
		if _, asked := acc.businessNameLookups.LoadOrStore(s, true); asked {
			continue
		}
		j, err := types.ParseJID(s)
		if err != nil || j.User == cli.Store.ID.User {
			continue
		}
		jids = append(jids, j)
	}
	rows.Close()
	// Known names are checked with the rows closed, as that runs queries of
	// its own.
	unnamed := jids[:0]
	for _, j := range jids {
		if name := acc.resolveDisplayName(j); name == "" || strings.HasPrefix(name, "+") || name == j.String() {
			unnamed = append(unnamed, j)
		}
	}
	jids = unnamed
	found := 0
	for start := 0; start < len(jids); start += 50 {
		batch := jids[start:min(start+50, len(jids))]
		info, err := cli.GetUserInfo(context.Background(), batch)
		if err != nil {
			log.Printf("Business names: lookup of %d numbers failed: %v", len(batch), err)
			for _, j := range batch {
				acc.businessNameLookups.Delete(j.String())
			}
			continue
		}
		for _, ui := range info {
			if ui.VerifiedName != nil && ui.VerifiedName.Details.GetVerifiedName() != "" {
				found++
			}
		}
	}
	if len(jids) > 0 {
		log.Printf("Business names: asked about %d unnamed chats, %d are businesses", len(jids), found)
	}
}
