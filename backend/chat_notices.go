package main

import (
	"fmt"
	"log"
	"time"

	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
)

// Texts shown in place of a message this device couldn't decrypt, worded the
// way the official apps word them.
const (
	undecryptableText = "Waiting for this message. This may take a while."
	viewOnceOnPhone   = "View once message. Open WhatsApp on your phone to view it."
)

// storeUndecryptablePlaceholder records a message that arrived but couldn't be
// decrypted, so it shows in the chat instead of silently not existing.
// whatsmeow has already asked the sender (and the phone) to send it again;
// when that copy decrypts it arrives as an ordinary events.Message under the
// same ID and saveMessage replaces this row with the real content.
func storeUndecryptablePlaceholder(v *events.UndecryptableMessage) {
	// The sender marked this one as not worth showing when it fails (a
	// reaction, poll vote or edit, typically).
	if v.DecryptFailMode == events.DecryptFailHide || v.Info.ID == "" {
		return
	}
	chatJID, senderJID := resolveLIDChat(&events.Message{Info: v.Info})
	jid := chatJID.String()
	if isIgnoredChatJID(jid) {
		return
	}
	// A retry that also failed raises this event again; and a message that
	// has since arrived intact must never be downgraded to a placeholder.
	var exists int
	if err := db.QueryRow(`SELECT COUNT(*) FROM messages WHERE id = ?`, v.Info.ID).Scan(&exists); err != nil || exists > 0 {
		return
	}

	text := undecryptableText
	if v.UnavailableType == events.UnavailableTypeViewOnce {
		text = viewOnceOnPhone
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
	if err := saveMessage(StoredMessage{
		ID: v.Info.ID, JID: jid, SenderJID: senderJID.String(), FromMe: v.Info.IsFromMe,
		Text: text, Timestamp: ts, Status: status,
	}); err != nil {
		return
	}
	if _, err := db.Exec(`INSERT INTO chats (jid, timestamp, unread_count, preview_snippet, preview_sender, preview_from_me) VALUES (?, ?, ?, ?, ?, ?)
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
		senderName = resolveDisplayName(senderJID)
	}
	broadcastChats()
	broadcastEvent("message", map[string]interface{}{
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
func noteSecurityCodeChange(v *events.IdentityChange) {
	contact := preferPN(v.JID)
	if contact.Server != types.DefaultUserServer || isOwnJID(contact) {
		return
	}
	jid := contact.String()
	var exists int
	if err := db.QueryRow(`SELECT COUNT(*) FROM chats WHERE jid = ?`, jid).Scan(&exists); err != nil || exists == 0 {
		return
	}
	ts := v.Timestamp
	if ts.IsZero() {
		ts = time.Now()
	}
	// One line per second at most: an implicit change (an untrusted-identity
	// error while decrypting) can repeat for every message in a burst.
	id := fmt.Sprintf("system_%s_%d_identity", contact.User, ts.Unix())
	insertGroupSystemMessage(jid, id, fmt.Sprintf("Your security code with %s changed.", nameToken(contact)), ts)
}

// groupPictureChangeText is the chat line for a group icon change by author.
func groupPictureChangeText(author types.JID, removed bool) string {
	who := "Someone"
	if isOwnJID(author) {
		who = "You"
	} else if !author.IsEmpty() {
		who = nameToken(preferPN(author))
	}
	if removed {
		return who + " deleted this group's icon"
	}
	return who + " changed this group's icon"
}
