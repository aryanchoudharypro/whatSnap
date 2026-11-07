package main

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"net"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
)

func (acc *Account) handleSend(conn net.Conn, raw json.RawMessage) {
	var data SendCommandData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	// Never the text itself: backend.log is a plain file on disk.
	debugf("Sending message to %s (%d chars)", data.To, len(data.Text))
	jid, err := types.ParseJID(data.To)
	if err != nil {
		return
	}
	if jid.Server == types.NewsletterServer && !acc.canPostInChannel(jid.String()) {
		acc.broadcastEvent("error", map[string]string{"message": "Only this channel's admins can post in it."})
		return
	}

	id := acc.wa().GenerateMessageID()

	quotedID, quotedText, quotedSender := "", "", ""
	if data.ReplyToID != "" {
		if _, storedSenderJID, preview, qErr := acc.getMessageForQuote(data.ReplyToID); qErr == nil {
			quotedID = data.ReplyToID
			quotedText = preview
			quotedSender = data.ReplySender
			if quotedSender == "" {
				quotedSender = storedSenderJID
			}
		}
	}

	var pendingRaw []byte
	if wireText, mentionJIDs := applyMentions(data.Text, data.Mentions); len(mentionJIDs) > 0 {
		if !invokedBot(mentionJIDs).IsEmpty() && acc.chatSharingLimited(jid.String()) {
			acc.broadcastEvent("error", map[string]string{"message": acc.chatPrivacyName(jid.String()) + " is on in this chat, so Meta AI can't be mentioned here."})
			return
		}
		pendingRaw, _ = proto.Marshal(&waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{
			Text:        proto.String(wireText),
			ContextInfo: &waE2E.ContextInfo{MentionedJID: jidStrings(mentionJIDs)},
		}})
	}

	senderJID := ""
	if acc.wa().Store.ID != nil {
		senderJID = acc.wa().Store.ID.ToNonAD().String()
	}
	ts := time.Now().Unix()
	quotedSenderDisplay := acc.resolveQuotedSenderName(quotedSender)

	acc.db.Exec(`INSERT INTO chats (jid, timestamp, preview_snippet, preview_sender, preview_from_me) VALUES (?, ?, ?, ?, 1)
			ON CONFLICT(jid) DO UPDATE SET
			timestamp=MAX(timestamp, excluded.timestamp),
			preview_snippet=COALESCE(NULLIF(excluded.preview_snippet, ''), chats.preview_snippet),
			preview_sender=excluded.preview_sender,
			preview_from_me=1,
			preview_is_reaction=0,
			preview_reaction_emoji=''`,
		jid.String(), ts, data.Text, senderJID)
	acc.saveMessage(StoredMessage{
		ID: id, JID: jid.String(), SenderJID: senderJID, FromMe: true,
		Text: data.Text, Timestamp: ts, Status: "pending", Raw: pendingRaw,
		QuotedID: quotedID, QuotedText: quotedText, QuotedSender: quotedSenderDisplay,
	})
	acc.broadcastEvent("message", map[string]interface{}{
		"jid":          jid.String(),
		"from":         "me",
		"id":           id,
		"text":         data.Text,
		"from_me":      true,
		"timestamp":    ts,
		"has_media":    false,
		"status":       "pending",
		"quotedId":     quotedID,
		"quotedText":   quotedText,
		"quotedSender": quotedSenderDisplay,
	})
	acc.broadcastChats()

	acc.sendPendingTextMessage(conn, id, jid.String(), data.Text, quotedID, quotedSender)
}

func (acc *Account) sendPendingTextMessage(conn net.Conn, id, jidStr, text, quotedID, quotedSender string) {
	if !acc.beginSendAttempt(id) {
		return
	}
	defer acc.endSendAttempt(id)

	jid, err := types.ParseJID(jidStr)
	if err != nil {
		return
	}

	wireText, mentionJIDs := acc.storedMentionMessage(id)
	if wireText == "" {
		wireText = text
	}

	var msg *waE2E.Message
	finalQuotedID, quotedText, finalQuotedSender := "", "", ""
	if quotedID != "" {
		quotedMsg, storedSenderJID, preview, qErr := acc.getMessageForQuote(quotedID)
		participant := quotedSender
		if participant == "" {
			participant = storedSenderJID
		}
		displayParticipant := participant
		// The frontend uses the literal string "me" as its own-message sender
		// sentinel everywhere (see resolveQuotedSenderName, which also expects
		// it) - fine for local display, but ContextInfo.Participant goes out
		// on the wire and has to be a real JID. Sending "me" there produced a
		// malformed quoted-message context, which is why replying to your own
		// message never rendered as a reply on the recipient's end.
		if participant == "me" {
			if acc.wa().Store.ID != nil {
				participant = acc.wa().Store.ID.ToNonAD().String()
			} else {
				participant = ""
			}
		}
		participant = acc.wireParticipantFor(quotedID, participant)
		if qErr == nil {
			finalQuotedID, quotedText, finalQuotedSender = quotedID, preview, acc.resolveQuotedSenderName(displayParticipant)
			ctxInfo := &waE2E.ContextInfo{StanzaID: proto.String(quotedID), QuotedMessage: quotedMsg}
			acc.quoteStatus(ctxInfo, quotedID)
			if participant != "" {
				ctxInfo.Participant = proto.String(participant)
			}
			if len(mentionJIDs) > 0 {
				ctxInfo.MentionedJID = jidStrings(mentionJIDs)
			}
			msg = &waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{Text: &wireText, ContextInfo: ctxInfo}}
		}
	}
	if msg == nil && len(mentionJIDs) > 0 {
		msg = &waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{
			Text:        &wireText,
			ContextInfo: &waE2E.ContextInfo{MentionedJID: jidStrings(mentionJIDs)},
		}}
	}
	if msg == nil {
		msg = &waE2E.Message{Conversation: &text}
	}

	if !jid.IsBot() {
		msg = acc.withLinkPreview(msg)
	}
	msg = acc.applyDisappearingTimer(jid, msg)
	resp, err := acc.wa().SendMessage(context.Background(), jid, msg, whatsmeow.SendRequestExtra{ID: id, InlineBotJID: invokedBot(mentionJIDs), CheckPermissions: true})
	if err != nil {
		log.Printf("Error sending message %s: %v", id, err)
		if conn != nil {
			acc.sendEvent(conn, "error", map[string]string{"message": sendErrorText(err)})
		}
		if sendRefusedForGood(err) {
			acc.markSendFailed(id, jidStr, text, quotedID, quotedSender)
			return
		}
		acc.recordSendFailure(id, jidStr, text, quotedID, quotedSender, err)
		return
	}
	log.Printf("Message %s sent successfully", id)
	acc.clearRetryState(id)
	ts := resp.Timestamp.Unix()
	rawBytes, _ := proto.Marshal(msg)

	acc.db.Exec(`UPDATE messages SET status='sent', timestamp=?, sent_at=?, raw=? WHERE id=?`, ts, ts, rawBytes, id)
	noteMessageExpiry(acc.db, id, ts, msg)
	acc.noteChannelPostSent(jid, id, resp)
	senderJID := ""
	if acc.wa().Store.ID != nil {
		senderJID = acc.wa().Store.ID.ToNonAD().String()
	}
	acc.db.Exec(`INSERT INTO chats (jid, timestamp, preview_snippet, preview_sender, preview_from_me) VALUES (?, ?, ?, ?, 1)
			ON CONFLICT(jid) DO UPDATE SET
			timestamp=MAX(timestamp, excluded.timestamp),
			preview_snippet=COALESCE(NULLIF(excluded.preview_snippet, ''), chats.preview_snippet),
			preview_sender=excluded.preview_sender,
			preview_from_me=1,
			preview_is_reaction=0,
			preview_reaction_emoji=''`,
		jid.String(), ts, text, senderJID)

	acc.broadcastEvent("message", map[string]interface{}{
		"jid":          jid.String(),
		"from":         "me",
		"id":           id,
		"text":         text,
		"from_me":      true,
		"timestamp":    ts,
		"has_media":    false,
		"status":       "sent",
		"quotedId":     finalQuotedID,
		"quotedText":   quotedText,
		"quotedSender": finalQuotedSender,
	})
	acc.broadcastChats()
}

func (acc *Account) handleReactMessage(raw json.RawMessage) {
	var data ReactMessageData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	chatJID, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	// The frontend names our own messages' sender "me". Parsed as a JID that
	// became {Server: "me"}, which BuildMessageKey took for someone else - so
	// a reaction to our own message went out with fromMe=false (and a bogus
	// "@me" participant in groups) and never showed up for anyone else.
	senderJID := types.EmptyJID
	if data.SenderJID != "" && data.SenderJID != "me" {
		if parsed, perr := types.ParseJID(data.SenderJID); perr == nil {
			senderJID = parsed
		}
	}
	if isChannelJID(data.JID) {
		go acc.handleChannelReaction(data)
		return
	}
	go func() {
		if chatJID.Server == types.GroupServer && !senderJID.IsEmpty() {
			if wire, perr := types.ParseJID(acc.wireParticipantFor(data.MessageID, senderJID.String())); perr == nil && !wire.IsEmpty() {
				senderJID = wire
			}
		}
		msg := acc.wa().BuildReaction(chatJID, senderJID, data.MessageID, data.Emoji)
		resp, err := acc.wa().SendMessage(context.Background(), chatJID, msg, whatsmeow.SendRequestExtra{ID: acc.wa().GenerateMessageID()})
		if err != nil {
			acc.broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		acc.upsertReaction(data.MessageID, data.JID, "me", data.Emoji, resp.Timestamp.Unix())
		acc.updateChatReactionPreview(data.JID, data.MessageID, "me", data.Emoji, true)
		acc.broadcastChats()
		acc.broadcastEvent("message_reaction", map[string]interface{}{
			"jid": data.JID, "message_id": data.MessageID, "reactor": "me", "reactor_name": "You", "emoji": data.Emoji,
		})
	}()
}

func (acc *Account) handleEditMessage(raw json.RawMessage) {
	var data EditMessageData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	chatJID, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		var mentions []MentionRef
		for _, m := range acc.editMentions(data.MessageID, data.Mentions) {
			if jid, err := types.ParseJID(m.JID); err == nil && !jid.IsBot() {
				mentions = append(mentions, m)
			}
		}
		newContent := &waE2E.Message{Conversation: proto.String(data.NewText)}
		if wireText, mentionJIDs := applyMentions(data.NewText, mentions); len(mentionJIDs) > 0 {
			newContent = &waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{
				Text:        proto.String(wireText),
				ContextInfo: &waE2E.ContextInfo{MentionedJID: jidStrings(mentionJIDs)},
			}}
		}
		msg := acc.wa().BuildEdit(chatJID, data.MessageID, newContent)

		editStanzaID := acc.wa().GenerateMessageID()
		if chatJID.Server != types.NewsletterServer {
			acc.recordEditStanza(editStanzaID, data.MessageID, data.JID, time.Now().Unix())
		}

		_, err := acc.wa().SendMessage(context.Background(), chatJID, msg, whatsmeow.SendRequestExtra{ID: editStanzaID})
		if err != nil {
			acc.broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		editTime := time.Now().Unix()
		shown, _ := acc.markMessageEdited(data.MessageID, data.JID, newContent, data.NewText, editTime)
		acc.broadcastEvent("message_edited", map[string]interface{}{
			"jid": data.JID, "message_id": data.MessageID, "text": shown, "edited_at": editTime,
		})
	}()
}

func (acc *Account) handleDeleteMessage(raw json.RawMessage) {
	var data DeleteMessageData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go acc.deleteMessages(data.JID, []DeleteMessageItem{{MessageID: data.MessageID, SenderJID: data.SenderJID}}, data.ForEveryone)
}

// handleDeleteMessages deletes several messages of one chat at once, as
// WhatsApp Web's select bar does (WAWebChatSendDeleteMsgsBridge,
// WAWebRevokeMsgAction).
func (acc *Account) handleDeleteMessages(raw json.RawMessage) {
	var data DeleteMessagesData
	if err := json.Unmarshal(raw, &data); err != nil || len(data.Messages) == 0 {
		return
	}
	go acc.deleteMessages(data.JID, data.Messages, data.ForEveryone)
}

type storedDeleteTarget struct {
	chat      types.JID
	sender    string
	fromMe    bool
	timestamp int64
}

func (acc *Account) storedDeleteTarget(chatJID types.JID, messageID string) storedDeleteTarget {
	target := storedDeleteTarget{chat: chatJID}
	var storedJIDStr string
	if dbErr := acc.db.QueryRow("SELECT jid, COALESCE(sender_jid, ''), from_me, timestamp FROM messages WHERE id = ?", messageID).
		Scan(&storedJIDStr, &target.sender, &target.fromMe, &target.timestamp); dbErr == nil {
		if parsed, perr := types.ParseJID(storedJIDStr); perr == nil && !parsed.IsEmpty() {
			target.chat = parsed
		}
	}
	return target
}

// deleteMessages deletes the messages for everyone, one revoke message each
// (sender or admin revoke, by whose message it is), or for this account only.
// Deleting for me puts every message into a single app-state patch, one
// deleteMessageForMe mutation per message under one timestamp - what WA
// Web's getDeleteForMeMutations + lockForSync send - instead of one patch
// per message racing each other for the collection's next version.
func (acc *Account) deleteMessages(jid string, items []DeleteMessageItem, forEveryone bool) {
	chatJID, err := types.ParseJID(jid)
	if err != nil {
		return
	}
	var forMe appstate.PatchInfo
	done := make([]string, 0, len(items))
	for _, item := range items {
		if item.MessageID == "" {
			continue
		}
		target := acc.storedDeleteTarget(chatJID, item.MessageID)
		if forEveryone {
			if errMsg := acc.revokeMessage(target, item); errMsg != "" {
				acc.broadcastEvent("error", map[string]string{"message": errMsg})
				continue
			}
		} else if target.chat.Server != types.NewsletterServer {
			patch := acc.deleteForMePatch(target, item.MessageID)
			forMe.Type = patch.Type
			forMe.Mutations = append(forMe.Mutations, patch.Mutations...)
		}
		done = append(done, item.MessageID)
	}
	if len(forMe.Mutations) > 0 {
		if err := acc.sendAppStateWithRetry(context.Background(), forMe); err != nil {
			log.Printf("Delete-for-me app-state sync failed for %d message(s): %v", len(forMe.Mutations), err)
		}
	}
	for _, messageID := range done {
		if forEveryone {
			acc.markMessageDeleted(messageID, jid)
		} else {
			acc.deleteMessageForMe(messageID, jid)
		}
		acc.broadcastEvent("message_deleted", map[string]interface{}{
			"jid": jid, "message_id": messageID, "by": "me", "for_everyone": forEveryone,
		})
	}
}

// revokeMessage sends the revoke for one message; it returns an error to
// show, or "" once sent.
func (acc *Account) revokeMessage(target storedDeleteTarget, item DeleteMessageItem) string {
	senderJID := types.EmptyJID
	if target.chat.Server == types.GroupServer {
		if target.sender != "" {
			if parsed, perr := types.ParseJID(target.sender); perr == nil {
				senderJID = parsed
			}
		}
		if senderJID.IsEmpty() && item.SenderJID != "" && item.SenderJID != "me" {
			if parsed, perr := types.ParseJID(item.SenderJID); perr == nil {
				senderJID = parsed
			}
		}
		if senderJID.IsEmpty() && acc.wa().Store.ID != nil {
			senderJID = acc.wa().Store.ID.ToNonAD()
		}
		if wire, perr := types.ParseJID(acc.wireParticipantFor(item.MessageID, senderJID.String())); perr == nil && !wire.IsEmpty() {
			senderJID = wire
		}
	}
	msg := acc.wa().BuildRevoke(target.chat, senderJID, item.MessageID)
	if _, err := acc.wa().SendMessage(context.Background(), target.chat, msg, whatsmeow.SendRequestExtra{ID: acc.wa().GenerateMessageID()}); err != nil {
		errMsg := err.Error()
		if strings.Contains(errMsg, "479") {
			errMsg = "WhatsApp error 479: Failed to delete for everyone. The message key mismatch or the revoke time limit expired."
		}
		return errMsg
	}
	return ""
}

func (acc *Account) deleteForMePatch(target storedDeleteTarget, messageID string) appstate.PatchInfo {
	originalSender := types.EmptyJID
	if target.fromMe {
		if acc.wa().Store.ID != nil {
			originalSender = acc.wa().Store.ID.ToNonAD()
		}
	} else if target.sender != "" {
		if parsed, perr := types.ParseJID(target.sender); perr == nil {
			originalSender = parsed
		}
	}
	msgTimestamp := time.Now()
	if target.timestamp > 0 {
		msgTimestamp = time.Unix(target.timestamp, 0)
	}
	if !target.fromMe && target.chat.Server == types.GroupServer && !originalSender.IsEmpty() {
		if wire, perr := types.ParseJID(acc.wireParticipantFor(messageID, originalSender.String())); perr == nil && !wire.IsEmpty() {
			originalSender = wire
		}
	}
	return appstate.BuildDeleteMessageForMe(acc.syncdChatJID(target.chat), originalSender, messageID, target.fromMe, true, msgTimestamp)
}

func (acc *Account) handleStarMessage(raw json.RawMessage) {
	var data StarMessageData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	chatJID, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	senderJID := types.EmptyJID
	if data.SenderJID != "" && data.SenderJID != "me" {
		if parsed, perr := types.ParseJID(data.SenderJID); perr == nil {
			senderJID = parsed
		}
	}
	go func() {
		if chatJID.Server == types.GroupServer && !data.FromMe && !senderJID.IsEmpty() {
			if wire, perr := types.ParseJID(acc.wireParticipantFor(data.MessageID, senderJID.String())); perr == nil && !wire.IsEmpty() {
				senderJID = wire
			}
		}
		patch := appstate.BuildStar(acc.syncdChatJID(chatJID), senderJID, types.MessageID(data.MessageID), data.FromMe, data.Starred)
		if err := acc.sendAppStateWithRetry(context.Background(), patch); err != nil {
			acc.broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		acc.markMessageStarred(data.MessageID, data.JID, data.Starred)
		acc.broadcastEvent("message_starred", map[string]interface{}{
			"jid": data.JID, "message_id": data.MessageID, "starred": data.Starred,
		})
	}()
}

func (acc *Account) handleSendTyping(raw json.RawMessage) {
	var data struct {
		JID   string `json:"jid"`
		State string `json:"state"`
		Media string `json:"media"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	pState := types.ChatPresenceComposing
	if data.State == "paused" {
		pState = types.ChatPresencePaused
	}
	go acc.wa().SendChatPresence(context.Background(), jid, pState, types.ChatPresenceMedia(data.Media))
}

func (acc *Account) handleSetPresence(raw json.RawMessage) {
	var data struct {
		Available bool `json:"available"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		log.Printf("Failed to unmarshal set_presence: %v", err)
		return
	}
	state := types.PresenceUnavailable
	if data.Available {
		state = types.PresenceAvailable
	}
	acc.desiredPresenceMu.Lock()
	acc.desiredPresence = state
	acc.desiredPresenceMu.Unlock()
	go acc.sendDesiredPresence(false)
}

func (acc *Account) sendDesiredPresence(force bool) {
	c := acc.wa()
	if c == nil || !c.IsLoggedIn() {
		return
	}
	acc.desiredPresenceMu.Lock()
	state := acc.desiredPresence
	if !force && state == acc.sentPresence {
		acc.desiredPresenceMu.Unlock()
		return
	}
	acc.sentPresence = state
	acc.desiredPresenceMu.Unlock()
	if state == types.PresenceUnavailable {
		acc.forgetPresenceSubscriptions()
	}
	if err := c.SendPresence(context.Background(), state); err != nil {
		log.Printf("SendPresence(%s) failed: %v", state, err)
		acc.desiredPresenceMu.Lock()
		acc.sentPresence = ""
		acc.desiredPresenceMu.Unlock()
	}
}

// WA Web never sends a read receipt while it shows as offline: its sendSeen
// waits for presence "available" before anything goes out, and the server
// treats an available device as the one the user is reading on. A read sent
// while we're unavailable (just after the window comes back, or after the
// idle timeout while new messages keep arriving) may never reach the phone.
// So go available first, then fall back to what the app wants once the
// reads are out.
const readPresenceHold = 10 * time.Second

func (acc *Account) ensureAvailableForRead() {
	c := acc.wa()
	if c == nil || !c.IsLoggedIn() {
		return
	}
	acc.desiredPresenceMu.Lock()
	if acc.readPresenceTimer != nil {
		acc.readPresenceTimer.Stop()
	}
	acc.readPresenceTimer = time.AfterFunc(readPresenceHold, func() { acc.sendDesiredPresence(false) })
	if acc.sentPresence == types.PresenceAvailable {
		acc.desiredPresenceMu.Unlock()
		return
	}
	acc.sentPresence = types.PresenceAvailable
	acc.desiredPresenceMu.Unlock()
	if err := c.SendPresence(context.Background(), types.PresenceAvailable); err != nil {
		log.Printf("SendPresence(available) before a read receipt failed: %v", err)
		acc.desiredPresenceMu.Lock()
		acc.sentPresence = ""
		acc.desiredPresenceMu.Unlock()
	}
}

// WA Web marks the chat read whenever you send something in it (sendSeen in
// WAWebSendMsgRecordAction), so replying from here clears it on the phone too.
func (acc *Account) markReadOnSend(raw json.RawMessage) {
	var data struct {
		JID string `json:"jid"`
		To  string `json:"to"`
	}
	if json.Unmarshal(raw, &data) != nil {
		return
	}
	jid := data.JID
	if jid == "" {
		jid = data.To
	}
	if jid == "" {
		return
	}
	if !acc.hasUnacknowledgedMessages(jid) {
		return
	}
	payload, _ := json.Marshal(map[string]interface{}{"jid": jid, "message_ids": []string{}})
	acc.handleMarkRead(payload)
}

// hasUnacknowledgedMessages reports whether the chat has an incoming message
// we haven't sent a read receipt for.
func (acc *Account) hasUnacknowledgedMessages(jid string) bool {
	var found int
	err := acc.db.QueryRow(`SELECT 1 FROM messages WHERE jid = ? AND `+unacknowledgedIncoming+` LIMIT 1`, jid).Scan(&found)
	return err == nil
}

func (acc *Account) forgetPresenceSubscriptions() {
	acc.presenceSubscribed.Clear()
}

func (acc *Account) handleSubscribePresence(raw json.RawMessage) {
	var data struct {
		JID string `json:"jid"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		log.Printf("Failed to unmarshal subscribe_presence: %v", err)
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		log.Printf("Failed to parse JID for subscribe_presence: %v", err)
		return
	}
	if jid.Server != types.DefaultUserServer && jid.Server != types.HiddenUserServer {
		return
	}
	jid = jid.ToNonAD()
	if own := acc.wa().Store.ID; own != nil && jid.User == own.User || jid.User == acc.wa().Store.GetLID().User {
		return
	}
	if _, already := acc.presenceSubscribed.LoadOrStore(jid, struct{}{}); already {
		return
	}
	err = acc.wa().SubscribePresence(context.Background(), jid)
	if err != nil {
		acc.presenceSubscribed.Delete(jid)
		log.Printf("Error subscribing to presence: %v", err)
	} else {
		debugf("Subscribed to presence for %s", jid.String())
	}
}

// WA Web (sendAggregateReceipts) puts at most 256 message ids in one receipt.
const readReceiptChunk = 256

// unacknowledgedIncoming picks a chat's incoming messages we haven't sent a
// read for. Like WA Web's pending-read rows it leaves out calls, system
// notices and messages still waiting to be decrypted (WA Web skips
// CIPHERTEXT): there's nothing on the server to acknowledge for those yet.
const unacknowledgedIncoming = `COALESCE(read_at, 0) = 0 AND status NOT IN ('read', 'played') AND ` + receivedMessage

// receivedMessage is an incoming message a read receipt can name.
const receivedMessage = `from_me = 0
	AND COALESCE(media_type, '') NOT IN ('system', 'call') AND id NOT LIKE 'system\_%' ESCAPE '\'
	AND NOT (COALESCE(length(raw), 0) = 0 AND text IN ('` + undecryptableText + `', '` + viewOnceReceived + `', '` + viewOnceSentElsewhere + `', '` + viewOnceOnPhoneOld + `'))`

func (acc *Account) handleMarkRead(raw json.RawMessage) {
	var data struct {
		JID        string   `json:"jid"`
		MessageIDs []string `json:"message_ids"`
		// Latest makes a whole-chat read name the newest received message
		// even when it was acknowledged before, the way WA Web's Mark as
		// read sends a receipt for chat.lastReceivedKey
		// (WAWebUpdateUnreadChatAction.sendSeen -> markChatRead).
		Latest bool `json:"latest"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	chatJID, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	if chatJID.Server == types.NewsletterServer {
		return
	}
	// Receipts go to the address the messages came in on, naming each
	// sender as their message did - in a group that hides numbers, by LID.
	// Swapping in the phone JID (as this used to) addressed a receipt to
	// someone the message never named.
	syncJID := acc.syncdChatJID(chatJID)
	chatJID = acc.receiptChatJID(chatJID)

	type msgSender struct {
		id        types.MessageID
		senderJID types.JID
		sts       int64
	}
	var items []msgSender

	if len(data.MessageIDs) > 0 {
		for _, id := range data.MessageIDs {
			var senderStr, mediaType string
			var sts int64
			_ = acc.db.QueryRow("SELECT COALESCE(NULLIF(wire_sender, ''), sender_jid, ''), COALESCE(media_type, ''), COALESCE(server_sts, 0) FROM messages WHERE id = ?", id).Scan(&senderStr, &mediaType, &sts)
			if mediaType == "system" || mediaType == "call" {
				continue
			}
			sJID, _ := types.ParseJID(senderStr)
			if sJID.IsEmpty() {
				if chatJID.Server == types.GroupServer {
					// A group receipt must name the member who sent it.
					continue
				}
				sJID = chatJID
			}
			items = append(items, msgSender{id: types.MessageID(id), senderJID: sJID, sts: sts})
		}
	}
	var markedUnread bool
	if len(data.MessageIDs) == 0 {
		_ = acc.db.QueryRow("SELECT COALESCE(marked_unread, 0) = 1 FROM chats WHERE jid = ?", data.JID).Scan(&markedUnread)

		// The chat's unread count can't decide this: opening a chat loads its
		// messages first, and that already sets the count to 0, so this saw
		// nothing to acknowledge and the phone was never told. Like WA Web's
		// pending-read-receipt rows, acknowledge every incoming message we
		// haven't sent a read for yet (newest first), however many there are.
		rows, err := acc.db.Query(`SELECT id, COALESCE(NULLIF(wire_sender, ''), sender_jid, ''), COALESCE(server_sts, 0) FROM messages
			WHERE jid = ? AND `+unacknowledgedIncoming+` ORDER BY timestamp DESC`, data.JID)
		if err == nil {
			for rows.Next() {
				var id, senderStr string
				var sts int64
				if err := rows.Scan(&id, &senderStr, &sts); err == nil {
					sJID, _ := types.ParseJID(senderStr)
					if sJID.IsEmpty() {
						if chatJID.Server == types.GroupServer {
							continue
						}
						sJID = chatJID
					}
					items = append(items, msgSender{id: types.MessageID(id), senderJID: sJID, sts: sts})
				}
			}
			rows.Close()
		}
		if len(items) == 0 && data.Latest {
			var id, senderStr string
			var sts int64
			// In a group the receipt must name the sender, so skip rows that
			// don't know theirs (deleted-message stubs from history sync).
			senderKnown := ""
			if chatJID.Server == types.GroupServer {
				senderKnown = ` AND COALESCE(NULLIF(wire_sender, ''), sender_jid, '') != ''`
			}
			err := acc.db.QueryRow(`SELECT id, COALESCE(NULLIF(wire_sender, ''), sender_jid, ''), COALESCE(server_sts, 0) FROM messages
				WHERE jid = ? AND `+receivedMessage+senderKnown+` ORDER BY timestamp DESC LIMIT 1`, data.JID).Scan(&id, &senderStr, &sts)
			if err != nil {
				log.Printf("Mark as read %s: no received message to name in a read receipt: %v", data.JID, err)
			} else {
				sJID, _ := types.ParseJID(senderStr)
				if sJID.IsEmpty() {
					sJID = chatJID
				}
				log.Printf("Mark as read %s: sending a read receipt for the newest received message %s from %s", data.JID, id, sJID)
				items = append(items, msgSender{id: types.MessageID(id), senderJID: sJID, sts: sts})
			}
		}
	}

	bySender := map[string][]types.MessageID{}
	senderJIDMap := map[string]types.JID{}
	// WA Web's maxStsByAuthor: the newest server store time per sender, put
	// on every receipt for that sender.
	maxSTS := map[string]int64{}

	for _, item := range items {
		key := item.senderJID.ToNonAD().String()
		bySender[key] = append(bySender[key], item.id)
		senderJIDMap[key] = item.senderJID
		maxSTS[key] = max(maxSTS[key], item.sts)
	}

	// Like WA Web's sendAggregateReceipts: one receipt per sender, split into
	// chunks of 256 ids, all sent at once, and the chat only counts as read
	// once every one of them is acknowledged; otherwise the app is told it
	// failed, so it doesn't show the chat read when the server never heard.
	go func() {
		if len(bySender) > 0 {
			acc.ensureAvailableForRead()
		}
		now := time.Now()
		var wg sync.WaitGroup
		var failed atomic.Bool
		for key, ids := range bySender {
			sender, sts := senderJIDMap[key], maxSTS[key]
			for start := 0; start < len(ids); start += readReceiptChunk {
				chunk := ids[start:min(start+readReceiptChunk, len(ids))]
				wg.Go(func() {
					if err := acc.markReadAndWaitForAck(chunk, now, chatJID, sender, sts); err != nil {
						log.Printf("MarkRead error for chat %s sender %s: %v", chatJID, sender, err)
						failed.Store(true)
						return
					}
					for _, id := range chunk {
						acc.db.Exec(`UPDATE messages SET read_at = ? WHERE id = ? AND from_me = 0 AND COALESCE(read_at, 0) = 0`, now.Unix(), string(id))
					}
				})
			}
		}
		wg.Wait()
		ok := !failed.Load()
		if !ok {
			acc.broadcastEvent("mark_read_failed", map[string]interface{}{"jid": data.JID})
			return
		}
		if markedUnread {
			lastTS, lastKey := acc.getLastMessageKeyForChat(data.JID)
			if err := acc.sendAppStateWithRetry(context.Background(), acc.withChatRange(appstate.BuildMarkChatAsRead(syncJID, true, lastTS, lastKey), data.JID)); err != nil {
				log.Printf("Clearing the unread mark of %s on other devices failed: %v", data.JID, err)
			}
		}
		if len(data.MessageIDs) == 0 {
			acc.db.Exec(`UPDATE messages SET read_at = ? WHERE jid = ? AND from_me = 0 AND COALESCE(read_at, 0) = 0`, now.Unix(), data.JID)
		}
		acc.db.Exec("UPDATE chats SET unread_count = 0, marked_unread = 0 WHERE jid = ?", data.JID)
		acc.broadcastEvent("chat_updated", map[string]interface{}{
			"jid":          data.JID,
			"unread_count": 0,
		})
	}()
}

const readReceiptAckTimeout = 15 * time.Second

func (acc *Account) markReadAndWaitForAck(ids []types.MessageID, now time.Time, chat, sender types.JID, maxSTS int64) error {
	if len(ids) == 0 {
		return nil
	}
	// Registered before sending so a quick ack isn't missed. It's never
	// cancelled: CancelResponse closes the channel, which would panic if the
	// ack arrived at that moment. A waiter left behind holds one buffered
	// channel and is cleared on disconnect.
	ackCh := acc.wa().DangerousInternals().WaitResponse(string(ids[0]))
	receipt := whatsmeow.ReadReceipt{IDs: ids, Timestamp: now, Chat: chat, Sender: sender, MaxServerStoreTime: maxSTS}
	if err := acc.wa().SendReadReceipt(context.Background(), receipt); err != nil {
		return err
	}
	select {
	case ack, open := <-ackCh:
		if !open || ack == nil {
			return fmt.Errorf("disconnected before the server acknowledged the receipt")
		}
		if ack.Tag != "ack" {
			return fmt.Errorf("unexpected <%s> in reply to the receipt", ack.Tag)
		}
		if code, has := ack.Attrs["error"]; has {
			return fmt.Errorf("server rejected the receipt with error %v", code)
		}
		// The server says on the ack whether our read receipts are on; WA Web
		// takes its setting from there, so a change on the phone applies to
		// the next receipt without waiting for a privacy fetch.
		if setting, ok := ack.Attrs["readreceipts"].(string); ok && (setting == "all" || setting == "none") {
			acc.wa().SetReadReceiptsPrivacy(types.PrivacySetting(setting))
		}
		return nil
	case <-time.After(readReceiptAckTimeout):
		return fmt.Errorf("no ack from the server within %s", readReceiptAckTimeout)
	}
}

func (acc *Account) handleGetMessageInfo(conn net.Conn, raw json.RawMessage) {
	var data struct {
		ID string `json:"id"`
	}
	if err := json.Unmarshal(raw, &data); err != nil || data.ID == "" {
		return
	}
	var sentAt, deliveredAt, readAt, playedAt, editedAt, msgTS int64
	var mediaType, jid string
	var fromMe bool
	err := acc.db.QueryRow(
		`SELECT COALESCE(sent_at,0), COALESCE(delivered_at,0), COALESCE(read_at,0), COALESCE(played_at,0), COALESCE(edited_at,0), COALESCE(media_type,''), COALESCE(timestamp,0), COALESCE(jid,''), COALESCE(from_me,0) FROM messages WHERE id = ?`,
		data.ID,
	).Scan(&sentAt, &deliveredAt, &readAt, &playedAt, &editedAt, &mediaType, &msgTS, &jid, &fromMe)
	if err != nil {
		return
	}
	if sentAt == 0 && msgTS > 0 {
		sentAt = msgTS
	}

	result := map[string]interface{}{
		"id":           data.ID,
		"sent_at":      sentAt,
		"delivered_at": deliveredAt,
		"read_at":      readAt,
		"played_at":    playedAt,
		"edited_at":    editedAt,
		"media_type":   mediaType,
	}

	if fromMe && strings.HasSuffix(jid, "@g.us") {
		if participants := acc.groupMessageReceiptParticipants(data.ID, jid); len(participants) > 0 {
			result["participants"] = participants
		}
	}

	if poll := acc.pollInfoForMessage(data.ID); poll != nil {
		result["poll"] = poll
	}

	acc.sendEvent(conn, "message_info", result)
}

func (acc *Account) groupMessageReceiptParticipants(messageID, groupJID string) []map[string]interface{} {
	jid, err := types.ParseJID(groupJID)
	if err != nil {
		return nil
	}
	info, err := acc.wa().GetGroupInfo(context.Background(), jid)
	if err != nil {
		return nil
	}
	myJID := acc.wa().Store.ID.ToNonAD()

	rows, err := acc.db.Query(`SELECT participant_jid, status, timestamp,
			COALESCE(delivered_at, 0), COALESCE(read_at, 0), COALESCE(played_at, 0)
		FROM message_receipts WHERE message_id = ?`, messageID)
	if err != nil {
		return nil
	}
	type receipt struct {
		status                      string
		ts, delivered, read, played int64
	}
	receipts := make(map[string]receipt)
	for rows.Next() {
		var r receipt
		var pJID string
		if rows.Scan(&pJID, &r.status, &r.ts, &r.delivered, &r.read, &r.played) == nil {
			switch {
			case r.status == "delivered" && r.delivered == 0:
				r.delivered = r.ts
			case r.status == "read" && r.read == 0:
				r.read = r.ts
			case r.status == "played" && r.played == 0:
				r.played = r.ts
			}
			receipts[pJID] = r
		}
	}
	rows.Close()

	participants := make([]map[string]interface{}, 0, len(info.Participants))
	for _, p := range info.Participants {
		displayJID := p.JID
		if !p.PhoneNumber.IsEmpty() {
			displayJID = p.PhoneNumber
		} else if displayJID.Server == "lid" {
			if pn, err := acc.wa().Store.LIDs.GetPNForLID(context.Background(), displayJID); err == nil && !pn.IsEmpty() {
				displayJID = pn
			}
		}
		if displayJID.User == myJID.User {
			continue
		}
		r, ok := receipts[displayJID.ToNonAD().String()]
		if !ok {
			r.status = "sent"
		}
		participants = append(participants, map[string]interface{}{
			"name":         acc.resolveDisplayName(displayJID),
			"status":       r.status,
			"timestamp":    r.ts,
			"delivered_at": r.delivered,
			"read_at":      r.read,
			"played_at":    r.played,
		})
	}
	return participants
}

type ForwardMessageData struct {
	MessageID  string   `json:"message_id"`
	MessageIDs []string `json:"message_ids"`
	ToJIDs     []string `json:"to_jids"`
}

const (
	maxForwardTargets        = 5
	frequentlyForwardedScore = 5
)

// handleForwardMessage forwards one message (message_id) or several
// (message_ids, oldest first) to every chat in to_jids. Several are sent one
// after another on a single goroutine so they arrive in the order they were
// written, as WhatsApp Web does when forwarding a selection.
func (acc *Account) handleForwardMessage(conn net.Conn, raw json.RawMessage) {
	var data ForwardMessageData
	if err := json.Unmarshal(raw, &data); err != nil || len(data.ToJIDs) == 0 {
		return
	}
	ids := data.MessageIDs
	if data.MessageID != "" {
		ids = append([]string{data.MessageID}, ids...)
	}
	if len(ids) == 0 {
		return
	}
	go func() {
		for _, id := range ids {
			acc.forwardOneMessage(conn, id, data.ToJIDs)
		}
		acc.broadcastChats()
	}()
}

func (acc *Account) forwardOneMessage(conn net.Conn, messageID string, toJIDs []string) {
	data := ForwardMessageData{MessageID: messageID, ToJIDs: toJIDs}
	{
		rawBytes, sourceJID, _, _, err := acc.getMessageRaw(data.MessageID)
		if err != nil || len(rawBytes) == 0 {
			acc.sendEvent(conn, "error", map[string]string{"message": "Message not found for forwarding"})
			return
		}
		var origMsg waE2E.Message
		if unmarshalErr := proto.Unmarshal(rawBytes, &origMsg); unmarshalErr != nil {
			acc.sendEvent(conn, "error", map[string]string{"message": "Failed to parse message protobuf for forwarding"})
			return
		}

		if acc.viewOnceState(data.MessageID) != 0 || origMsg.GetViewOnceMessage() != nil || origMsg.GetViewOnceMessageV2() != nil || origMsg.GetViewOnceMessageV2Extension() != nil {
			acc.sendEvent(conn, "error", map[string]string{"message": "View once messages can't be forwarded."})
			return
		}
		content := proto.Clone(unwrapMessage(&origMsg)).(*waE2E.Message)
		if content.Conversation != nil {
			txt := content.GetConversation()
			content.Conversation = nil
			content.ExtendedTextMessage = &waE2E.ExtendedTextMessage{Text: &txt}
		}
		score := uint32(0)
		if old := getContextInfo(content); old != nil {
			score = old.GetForwardingScore()
		}
		forwardCtx := &waE2E.ContextInfo{
			IsForwarded:     proto.Bool(true),
			ForwardingScore: proto.Uint32(score + 1),
		}
		fromChannel := isChannelJID(sourceJID)
		if fromChannel {
			forwardCtx.ForwardedNewsletterMessageInfo = acc.forwardedChannelInfo(sourceJID, data.MessageID)
		}
		if !setContextInfo(content, forwardCtx) {
			acc.sendEvent(conn, "error", map[string]string{"message": "This kind of message can't be forwarded."})
			return
		}
		limit := maxForwardTargets
		if score >= frequentlyForwardedScore {
			limit = 1
		}
		if len(data.ToJIDs) > limit {
			acc.sendEvent(conn, "error", map[string]string{"message": fmt.Sprintf("This message can only be forwarded to %d chat(s) at a time.", limit)})
			return
		}
		for _, toJIDStr := range data.ToJIDs {
			targetJID, parseErr := types.ParseJID(toJIDStr)
			if parseErr != nil {
				continue
			}
			toChannel := targetJID.Server == types.NewsletterServer
			if toChannel && !acc.canPostInChannel(targetJID.String()) {
				acc.sendEvent(conn, "error", map[string]string{"message": "You can only forward to channels you run."})
				continue
			}
			outgoing := content
			var extra whatsmeow.SendRequestExtra
			if toChannel != fromChannel && mediaPart(content) != nil {
				outgoing = proto.Clone(content).(*waE2E.Message)
				handle, reErr := acc.reuploadMedia(context.Background(), outgoing, toChannel)
				if reErr != nil {
					log.Printf("Couldn't re-upload forwarded media for %s: %v", toJIDStr, reErr)
					acc.sendEvent(conn, "error", map[string]string{"message": "Couldn't forward the media: " + reErr.Error()})
					continue
				}
				extra.MediaHandle = handle
			}
			if !toChannel {
				outgoing = acc.applyDisappearingTimer(targetJID, proto.Clone(outgoing).(*waE2E.Message))
			}
			resp, sendErr := acc.wa().SendMessage(context.Background(), targetJID, outgoing, extra)
			if sendErr != nil {
				log.Printf("Error forwarding message to %s: %v", toJIDStr, sendErr)
				continue
			}
			ts := resp.Timestamp.Unix()
			text := acc.extractText(content)
			senderJID := acc.wa().Store.ID.ToNonAD().String()
			fwdRawBytes, _ := proto.Marshal(outgoing)
			mediaType, mimetype, filename, mediaSize := mediaInfoFromMessage(outgoing)
			quotedID, quotedText, quotedSender := acc.quotedPreview(outgoing)

			acc.db.Exec(`INSERT INTO chats (jid, timestamp, preview_snippet, preview_sender, preview_from_me) VALUES (?, ?, ?, ?, 1)
				ON CONFLICT(jid) DO UPDATE SET
				timestamp=MAX(timestamp, excluded.timestamp),
				preview_snippet=COALESCE(NULLIF(excluded.preview_snippet, ''), chats.preview_snippet),
				preview_sender=excluded.preview_sender,
				preview_from_me=1,
				preview_is_reaction=0,
				preview_reaction_emoji=''`,
				targetJID.String(), ts, text, senderJID)

			acc.saveMessage(StoredMessage{
				ID: resp.ID, JID: targetJID.String(), SenderJID: senderJID, FromMe: true,
				Text: text, Timestamp: ts, Raw: fwdRawBytes, Status: "sent",
				MediaType: mediaType, MediaMimetype: mimetype, MediaFilename: filename, MediaSize: mediaSize,
				QuotedID: quotedID, QuotedText: quotedText, QuotedSender: quotedSender,
				ServerID: int64(resp.ServerID),
			})
			noteMessageExpiry(acc.db, resp.ID, ts, outgoing)

			acc.broadcastEvent("message", map[string]interface{}{
				"jid":           targetJID.String(),
				"from":          "me",
				"id":            resp.ID,
				"text":          text,
				"from_me":       true,
				"timestamp":     ts,
				"has_media":     mediaType != "",
				"mediaType":     mediaType,
				"mediaFilename": filename,
				"mediaSize":     mediaSize,
				"quotedId":      quotedID,
				"quotedText":    quotedText,
				"quotedSender":  quotedSender,
			})
		}
	}
}
