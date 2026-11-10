package main

import (
	"fmt"
	"log"
	"time"

	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
)

const (
	undecryptableText = "Waiting for this message. This may take a while."
	// WA Web's texts for <unavailable type="view_once"/>: the server never
	// fans view once media out to linked devices, and WA Web doesn't ask the
	// phone to resend it either, so only the phone can open it.
	viewOnceReceived      = "You received a view once message. For added privacy, you can only open it on your phone."
	viewOnceSentElsewhere = "You sent a view once message from another device."
	// Older builds stored this text; kept so those rows still count as placeholders.
	viewOnceOnPhoneOld = "View once message. Open WhatsApp on your phone to view it."
)

func (acc *Account) storeUndecryptablePlaceholder(v *events.UndecryptableMessage) {
	if v.DecryptFailMode == events.DecryptFailHide || v.Info.ID == "" {
		return
	}
	chatJID, senderJID := acc.resolveLIDChat(&events.Message{Info: v.Info})
	jid := chatJID.String()
	if isIgnoredChatJID(jid) {
		return
	}
	// A retry that also failed raises this event again; and a message that
	// has since arrived intact must never be downgraded to a placeholder.
	var exists int
	if err := acc.db.QueryRow(`SELECT COUNT(*) FROM messages WHERE id = ?`, v.Info.ID).Scan(&exists); err != nil || exists > 0 {
		return
	}

	text := undecryptableText
	if v.UnavailableType == events.UnavailableTypeViewOnce {
		text = viewOnceReceived
		if v.Info.IsFromMe {
			text = viewOnceSentElsewhere
		}
	}
	ts := v.Info.Timestamp.Unix()
	unread := 1
	if v.Info.IsFromMe {
		unread = 0
	}
	status := ""
	if v.Info.IsFromMe {
		status = "sent"
	}
	if err := acc.saveMessage(StoredMessage{
		ID: v.Info.ID, JID: jid, SenderJID: senderJID.String(), FromMe: v.Info.IsFromMe,
		Text: text, Timestamp: ts, Status: status,
	}); err != nil {
		return
	}
	if _, err := acc.db.Exec(`INSERT INTO chats (jid, timestamp, unread_count, preview_snippet, preview_sender, preview_from_me) VALUES (?, ?, ?, ?, ?, ?)
		ON CONFLICT(jid) DO UPDATE SET
			timestamp=MAX(timestamp, excluded.timestamp),
			unread_count=chats.unread_count + excluded.unread_count,
			preview_snippet=excluded.preview_snippet,
			preview_sender=excluded.preview_sender,
			preview_from_me=excluded.preview_from_me,
			preview_is_reaction=0,
			preview_reaction_emoji=''`,
		jid, ts, unread, text, senderJID.String(), v.Info.IsFromMe); err != nil {
		log.Println("Error updating chat for undecryptable message:", err)
	}
	log.Printf("Stored a placeholder for undecryptable message %s in %s (unavailable=%v)", v.Info.ID, jid, v.IsUnavailable)

	senderName := ""
	if !v.Info.IsFromMe {
		senderName = acc.hidePhoneInGroup(jid, acc.resolveDisplayName(senderJID))
	}
	acc.broadcastChats()
	acc.broadcastEvent("message", map[string]interface{}{
		"jid":        jid,
		"from":       senderJID.String(),
		"senderName": senderName,
		"id":         v.Info.ID,
		"text":       text,
		"from_me":    v.Info.IsFromMe,
		"timestamp":  ts,
		"has_media":  false,
	})
}

// noteSecurityCodeChange adds the "security code changed" line the official
// apps show when a contact's identity key changes (a reinstall or a new
// phone), to the 1:1 chat with them. Only for chats that already exist: an
// identity change for someone we've never talked to has nowhere to go.
// Like WhatsApp Web it's a quiet line: the chat keeps its place, preview and
// unread count, and nothing alerts (see insertQuietSystemMessage).
func (acc *Account) noteSecurityCodeChange(v *events.IdentityChange) {
	if !acc.appPref("security_notifications") {
		return
	}
	contact := acc.preferPN(v.JID)
	if contact.Server != types.DefaultUserServer || acc.isOwnJID(contact) {
		return
	}
	jid := contact.String()
	var exists int
	if err := acc.db.QueryRow(`SELECT COUNT(*) FROM chats WHERE jid = ?`, jid).Scan(&exists); err != nil || exists == 0 {
		return
	}
	ts := v.Timestamp
	if ts.IsZero() {
		ts = time.Now()
	}
	id := fmt.Sprintf("system_%s_%d_identity", contact.User, ts.Unix())
	acc.insertQuietSystemMessage(jid, id, fmt.Sprintf("Your security code with %s changed.", acc.nameToken(contact)), ts)
}

// quietSystemSuffix ends the id of every system line that only goes into the
// chat itself. UpdateChatSnippet skips these, so a rebuilt preview never
// becomes one of them.
const quietSystemSuffix = "_identity"

// insertQuietSystemMessage stores a system line in a chat without touching
// the chat list: no move to the top, no new preview, no unread. The UI still
// gets the "message" event so an open chat shows it, and system lines never
// play a sound or show a notification there.
func (acc *Account) insertQuietSystemMessage(jid, id, text string, ts time.Time) {
	if text == "" || isIgnoredChatJID(jid) {
		return
	}
	tsUnix := ts.Unix()
	if err := acc.saveMessage(StoredMessage{
		ID: id, JID: jid, Text: text, Timestamp: tsUnix, Status: "sent", MediaType: "system",
	}); err != nil {
		log.Println("Error inserting quiet system message:", err)
		return
	}
	acc.broadcastEvent("message", map[string]interface{}{
		"jid":       jid,
		"from":      "",
		"id":        id,
		"text":      acc.renderNames(text, nil),
		"from_me":   false,
		"timestamp": tsUnix,
		"has_media": false,
		"mediaType": "system",
	})
}

func (acc *Account) groupPictureChangeText(author types.JID, removed bool) string {
	who := "Someone"
	if acc.isOwnJID(author) {
		who = "You"
	} else if !author.IsEmpty() {
		who = acc.nameToken(acc.preferPN(author))
	}
	if removed {
		return who + " deleted this group's icon"
	}
	return who + " changed this group's icon"
}
