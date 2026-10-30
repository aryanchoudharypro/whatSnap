package main

import (
	"context"
	"sync"

	"go.mau.fi/whatsmeow/store"
	"go.mau.fi/whatsmeow/types"
)

// whatSnap stores people under their phone JID, but WhatsApp increasingly names
// them by LID: a group that hides numbers addresses every member by LID, and an
// account whose chats moved to LIDs (LIDMigrationTimestamp) indexes its synced
// chat settings by LID. Whatever goes back to WhatsApp about a message or chat
// - a read receipt, a reaction's or a quote's key, an app-state patch - has to
// use the address WhatsApp itself used, as WhatsApp Web does, not the phone JID
// it was stored under. These helpers keep that address and give it back.

// wireChat is the address a 1:1 message came in on, when that isn't the phone
// JID its chat is stored under (a LID chat), or "".
func wireChat(chat, stored types.JID) string {
	chat = chat.ToNonAD()
	if chat.Server != types.HiddenUserServer || chat == stored.ToNonAD() {
		return ""
	}
	return chat.String()
}

// wireSender is a group message's sender as the message named them, when that
// isn't the JID stored as its sender, or "".
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

// wireParticipantFor is the participant to name a stored message by in a
// message key or quote: its wire sender when the given participant is the one
// it was stored under (or not known).
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

// receiptChatJID is where read receipts for a 1:1 chat go: the address its
// messages came in on. When that's a LID the number has since moved away
// from (a history-sync chat of the contact's old account), it's the
// number's current LID, as the chat WhatsApp Web would have open.
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

// syncdChatJID is the JID an app-state patch about a 1:1 chat is indexed by.
// Once the phone has moved its chats to LIDs it indexes them by LID, and a
// patch indexed by the phone JID would be about a chat it doesn't have.
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

// recheckChatLIDs heals what older builds left behind: history sync could
// store a contact's old, replaced LID over the current one, and everything
// sent to that number then went to the old account's chat. A 1:1 chat whose
// messages came in on another LID than the one stored for its number is
// handed to whatsmeow, which asks the server which LID is current.
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
