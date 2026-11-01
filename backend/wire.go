package main

import (
	"context"
	"sync"

	"go.mau.fi/whatsmeow/store"
	"go.mau.fi/whatsmeow/types"
)

func wireChat(chat, stored types.JID) string {
	chat = chat.ToNonAD()
	if chat.Server != types.HiddenUserServer || chat == stored.ToNonAD() {
		return ""
	}
	return chat.String()
}

func wireSender(chat, sender, stored types.JID) string {
	if chat.Server != types.GroupServer {
		return ""
	}
	sender = sender.ToNonAD()
	if sender.IsEmpty() || sender == stored.ToNonAD() {
		return ""
	}
	return sender.String()
}

func wireParticipantFor(messageID, participant string) string {
	var sender, wire string
	err := db.QueryRow(`SELECT COALESCE(sender_jid, ''), COALESCE(wire_sender, '') FROM messages WHERE id = ?`, messageID).Scan(&sender, &wire)
	if err != nil || wire == "" {
		return participant
	}
	if participant == "" || participant == sender {
		return wire
	}
	return participant
}

func receiptChatJID(chat types.JID) types.JID {
	if chat.Server != types.DefaultUserServer {
		return chat
	}
	var wire string
	db.QueryRow(`SELECT COALESCE(wire_jid, '') FROM chats WHERE jid = ?`, chat.String()).Scan(&wire)
	parsed, err := types.ParseJID(wire)
	if err != nil || parsed.Server != types.HiddenUserServer {
		return chat
	}
	if wa() != nil && wa().Store != nil {
		if current, err := wa().Store.LIDs.GetLIDForPN(context.Background(), chat); err == nil && !current.IsEmpty() {
			return current.ToNonAD()
		}
	}
	return parsed
}

func syncdChatJID(chat types.JID) types.JID {
	chat = chat.ToNonAD()
	if chat.Server != types.DefaultUserServer || wa() == nil || wa().Store == nil || wa().Store.LIDMigrationTimestamp == 0 {
		return chat
	}
	if lid, err := wa().Store.LIDs.GetLIDForPN(context.Background(), chat); err == nil && !lid.IsEmpty() {
		return lid.ToNonAD()
	}
	return chat
}

var recheckChatLIDsOnce sync.Once

func recheckChatLIDs() {
	rows, err := db.Query(`SELECT jid, wire_jid FROM chats WHERE jid LIKE '%@s.whatsapp.net' AND wire_jid LIKE '%@lid'`)
	if err != nil {
		return
	}
	var pairs []store.LIDMapping
	for rows.Next() {
		var jid, wire string
		if rows.Scan(&jid, &wire) != nil {
			continue
		}
		pn, err1 := types.ParseJID(jid)
		lid, err2 := types.ParseJID(wire)
		if err1 == nil && err2 == nil {
			pairs = append(pairs, store.LIDMapping{LID: lid.ToNonAD(), PN: pn.ToNonAD()})
		}
	}
	rows.Close()
	if len(pairs) > 0 && wa() != nil {
		wa().StoreObservedLIDMappings(context.Background(), pairs)
	}
}
