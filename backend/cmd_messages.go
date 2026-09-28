package main

import (
	"context"
	"encoding/json"
	"log"
	"strings"
	"sync"
	"time"

	"net"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
)

func handleSend(conn net.Conn, raw json.RawMessage) {
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
	if jid.Server == types.NewsletterServer && !canPostInChannel(jid.String()) {
		broadcastEvent("error", map[string]string{"message": "Only this channel's admins can post in it."})
		return
	}

	// Pick the message ID up front (instead of taking whatever whatsmeow
	// generates on success) so the pending row we persist right now and the
	// eventual sent/failed row are the same message throughout.
	id := client.GenerateMessageID()

	quotedID, quotedText, quotedSender := "", "", ""
	if data.ReplyToID != "" {
		if _, storedSenderJID, preview, qErr := getMessageForQuote(data.ReplyToID); qErr == nil {
			quotedID = data.ReplyToID
			quotedText = preview
			quotedSender = data.ReplySender
			if quotedSender == "" {
				quotedSender = storedSenderJID
			}
		}
	}

	// The text keeps its "@Name" mentions for display; the wire text and the
	// mentioned JIDs go with the pending row, so a retry sends them too.
	var pendingRaw []byte
	if wireText, mentionJIDs := applyMentions(data.Text, data.Mentions); len(mentionJIDs) > 0 {
		if !invokedBot(mentionJIDs).IsEmpty() && chatLimitSharing(jid.String()) {
			broadcastEvent("error", map[string]string{"message": "Advanced chat privacy is on in this chat, so Meta AI can't be mentioned here."})
			return
		}
		pendingRaw, _ = proto.Marshal(&waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{
			Text:        proto.String(wireText),
			ContextInfo: &waE2E.ContextInfo{MentionedJID: jidStrings(mentionJIDs)},
		}})
	}

	senderJID := ""
	if client.Store.ID != nil {
		senderJID = client.Store.ID.ToNonAD().String()
	}
	ts := time.Now().Unix()
	quotedSenderDisplay := resolveQuotedSenderName(quotedSender)

	// Persist and broadcast as "pending" immediately, before the network
	// attempt: this is the local echo, and it's what makes a failed/offline
	// send recoverable instead of just vanishing.
	db.Exec(`INSERT INTO chats (jid, timestamp, preview_snippet, preview_sender, preview_from_me) VALUES (?, ?, ?, ?, 1)
			ON CONFLICT(jid) DO UPDATE SET
			timestamp=MAX(timestamp, excluded.timestamp),
			preview_snippet=COALESCE(NULLIF(excluded.preview_snippet, ''), chats.preview_snippet),
			preview_sender=excluded.preview_sender,
			preview_from_me=1,
			preview_is_reaction=0,
			preview_reaction_emoji=''`,
		jid.String(), ts, data.Text, senderJID)
	saveMessage(StoredMessage{
		ID: id, JID: jid.String(), SenderJID: senderJID, FromMe: true,
		Text: data.Text, Timestamp: ts, Status: "pending", Raw: pendingRaw,
		QuotedID: quotedID, QuotedText: quotedText, QuotedSender: quotedSenderDisplay,
	})
	broadcastEvent("message", map[string]interface{}{
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
	broadcastChats()

	sendPendingTextMessage(conn, id, jid.String(), data.Text, quotedID, quotedSender)
}

// sendPendingTextMessage attempts to actually deliver a text message that has
// already been persisted (as "pending" or "failed"). It's the single place
// that turns a stored row into a real WhatsApp send, shared by the initial
// send above, the reconnect auto-retry sweep, and a user-triggered manual
// retry, so all three behave identically.
func sendPendingTextMessage(conn net.Conn, id, jidStr, text, quotedID, quotedSender string) {
	if !beginSendAttempt(id) {
		// Already in flight - the reconnect sweep, the periodic auto-retry
		// sweep, and a manual "Retry send" can all reach this function for
		// the same message, and only one should ever hit the network.
		return
	}
	defer endSendAttempt(id)

	jid, err := types.ParseJID(jidStr)
	if err != nil {
		return
	}

	// text is what's shown ("@Name"); a message with mentions was stored
	// with its wire text ("@<JID user>") and mentioned JIDs.
	wireText, mentionJIDs := storedMentionMessage(id)
	if wireText == "" {
		wireText = text
	}

	var msg *waE2E.Message
	finalQuotedID, quotedText, finalQuotedSender := "", "", ""
	if quotedID != "" {
		quotedMsg, storedSenderJID, preview, qErr := getMessageForQuote(quotedID)
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
			if client.Store.ID != nil {
				participant = client.Store.ID.ToNonAD().String()
			} else {
				participant = ""
			}
		}
		if qErr == nil {
			finalQuotedID, quotedText, finalQuotedSender = quotedID, preview, resolveQuotedSenderName(displayParticipant)
			ctxInfo := &waE2E.ContextInfo{StanzaID: proto.String(quotedID), QuotedMessage: quotedMsg}
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

	msg = applyDisappearingTimer(jid, msg)
	// Mentioning Meta AI sends the message to it as well (whatsmeow's inline
	// bot mode), which is what makes it answer in this chat.
	resp, err := client.SendMessage(context.Background(), jid, msg, whatsmeow.SendRequestExtra{ID: id, InlineBotJID: invokedBot(mentionJIDs)})
	if err != nil {
		log.Printf("Error sending message %s: %v", id, err)
		if conn != nil {
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
		}
		// Stays "pending" in the DB while auto-retries remain; the periodic
		// sweep (backend/cmd_retry.go) will pick it up again with backoff,
		// or a manual retry from the context menu. Once retries are
		// exhausted, recordSendFailure marks it "failed" instead.
		recordSendFailure(id, jidStr, text, quotedID, quotedSender)
		return
	}
	log.Printf("Message %s sent successfully", id)
	clearRetryState(id)
	ts := resp.Timestamp.Unix()
	rawBytes, _ := proto.Marshal(msg)

	db.Exec(`UPDATE messages SET status='sent', timestamp=?, sent_at=?, raw=? WHERE id=?`, ts, ts, rawBytes, id)
	noteMessageExpiry(db, id, ts, msg)
	noteChannelPostSent(jid, id, resp)
	senderJID := ""
	if client.Store.ID != nil {
		senderJID = client.Store.ID.ToNonAD().String()
	}
	db.Exec(`INSERT INTO chats (jid, timestamp, preview_snippet, preview_sender, preview_from_me) VALUES (?, ?, ?, ?, 1)
			ON CONFLICT(jid) DO UPDATE SET
			timestamp=MAX(timestamp, excluded.timestamp),
			preview_snippet=COALESCE(NULLIF(excluded.preview_snippet, ''), chats.preview_snippet),
			preview_sender=excluded.preview_sender,
			preview_from_me=1,
			preview_is_reaction=0,
			preview_reaction_emoji=''`,
		jid.String(), ts, text, senderJID)

	broadcastEvent("message", map[string]interface{}{
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
	broadcastChats()
}

func handleReactMessage(raw json.RawMessage) {
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
		go handleChannelReaction(data)
		return
	}
	go func() {
		msg := client.BuildReaction(chatJID, senderJID, data.MessageID, data.Emoji)
		resp, err := client.SendMessage(context.Background(), chatJID, msg, whatsmeow.SendRequestExtra{ID: client.GenerateMessageID()})
		if err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		upsertReaction(data.MessageID, data.JID, "me", data.Emoji, resp.Timestamp.Unix())
		updateChatReactionPreview(data.JID, data.MessageID, "me", data.Emoji, true)
		broadcastChats()
		broadcastEvent("message_reaction", map[string]interface{}{
			"jid": data.JID, "message_id": data.MessageID, "reactor": "me", "reactor_name": "You", "emoji": data.Emoji,
		})
	}()
}

func handleEditMessage(raw json.RawMessage) {
	var data EditMessageData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	chatJID, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		// An edit can't invoke Meta AI, so a bot mention stays plain text.
		var mentions []MentionRef
		for _, m := range editMentions(data.MessageID, data.Mentions) {
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
		msg := client.BuildEdit(chatJID, data.MessageID, newContent)

		// The edit travels as its own message with its own ID, and every
		// receipt for it comes back under that ID. Note the pairing before
		// sending - the first receipt can arrive before SendMessage returns.
		// (A channel post's edit goes out under the post's own ID instead.)
		editStanzaID := client.GenerateMessageID()
		if chatJID.Server != types.NewsletterServer {
			recordEditStanza(editStanzaID, data.MessageID, data.JID, time.Now().Unix())
		}

		_, err := client.SendMessage(context.Background(), chatJID, msg, whatsmeow.SendRequestExtra{ID: editStanzaID})
		if err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		editTime := time.Now().Unix()
		shown, _ := markMessageEdited(data.MessageID, data.JID, newContent, data.NewText, editTime)
		broadcastEvent("message_edited", map[string]interface{}{
			"jid": data.JID, "message_id": data.MessageID, "text": shown, "edited_at": editTime,
		})
	}()
}

func handleDeleteMessage(raw json.RawMessage) {
	var data DeleteMessageData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	chatJID, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		targetChatJID := chatJID
		var storedJIDStr, storedSenderStr string
		var storedFromMe bool
		var storedTimestamp int64
		if dbErr := db.QueryRow("SELECT jid, COALESCE(sender_jid, ''), from_me, timestamp FROM messages WHERE id = ?", data.MessageID).
			Scan(&storedJIDStr, &storedSenderStr, &storedFromMe, &storedTimestamp); dbErr == nil {
			if parsed, perr := types.ParseJID(storedJIDStr); perr == nil && !parsed.IsEmpty() {
				targetChatJID = parsed
			}
		}

		if data.ForEveryone {
			senderJID := types.EmptyJID
			if targetChatJID.Server == types.GroupServer {
				if storedSenderStr != "" {
					if parsed, perr := types.ParseJID(storedSenderStr); perr == nil {
						senderJID = parsed
					}
				}
				if senderJID.IsEmpty() && data.SenderJID != "" && data.SenderJID != "me" {
					if parsed, perr := types.ParseJID(data.SenderJID); perr == nil {
						senderJID = parsed
					}
				}
				if senderJID.IsEmpty() && client.Store.ID != nil {
					senderJID = client.Store.ID.ToNonAD()
				}
			}

			msg := client.BuildRevoke(targetChatJID, senderJID, data.MessageID)
			if _, err := client.SendMessage(context.Background(), targetChatJID, msg, whatsmeow.SendRequestExtra{ID: client.GenerateMessageID()}); err != nil {
				errMsg := err.Error()
				if strings.Contains(errMsg, "479") {
					errMsg = "WhatsApp error 479: Failed to delete for everyone. The message key mismatch or the revoke time limit expired."
				}
				broadcastEvent("error", map[string]string{"message": errMsg})
				return
			}
		} else if targetChatJID.Server != types.NewsletterServer {
			// Channel posts aren't in app state; hiding one is local only.
			// "Delete for me": syncs the local hide via app-state so it also
			// applies on the user's other linked devices, not just this one
			// (see appstate.BuildDeleteMessageForMe).
			originalSender := types.EmptyJID
			if storedFromMe {
				if client.Store.ID != nil {
					originalSender = client.Store.ID.ToNonAD()
				}
			} else if storedSenderStr != "" {
				if parsed, perr := types.ParseJID(storedSenderStr); perr == nil {
					originalSender = parsed
				}
			}
			msgTimestamp := time.Now()
			if storedTimestamp > 0 {
				msgTimestamp = time.Unix(storedTimestamp, 0)
			}
			patch := appstate.BuildDeleteMessageForMe(targetChatJID, originalSender, data.MessageID, storedFromMe, true, msgTimestamp)
			if err := sendAppStateWithRetry(context.Background(), patch); err != nil {
				log.Printf("Delete-for-me app-state sync failed for %s: %v", data.MessageID, err)
				// Fall through to the local delete below regardless - at
				// minimum this device drops it even if the sync to other
				// linked devices didn't go through.
			}
		}
		if data.ForEveryone {
			markMessageDeleted(data.MessageID, data.JID)
		} else {
			deleteMessageForMe(data.MessageID, data.JID)
		}
		broadcastEvent("message_deleted", map[string]interface{}{
			"jid": data.JID, "message_id": data.MessageID, "by": "me", "for_everyone": data.ForEveryone,
		})
	}()
}

func handleStarMessage(raw json.RawMessage) {
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
		patch := appstate.BuildStar(chatJID, senderJID, types.MessageID(data.MessageID), data.FromMe, data.Starred)
		if err := sendAppStateWithRetry(context.Background(), patch); err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		markMessageStarred(data.MessageID, data.JID, data.Starred)
		broadcastEvent("message_starred", map[string]interface{}{
			"jid": data.JID, "message_id": data.MessageID, "starred": data.Starred,
		})
	}()
}

func handleSendTyping(raw json.RawMessage) {
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
	go client.SendChatPresence(context.Background(), jid, pState, types.ChatPresenceMedia(data.Media))
}

// desiredPresence is what the frontend last asked for: available only while
// its window is actually in use. It's replayed after every (re)connect instead
// of always announcing "available" - which left the account showing online
// around the clock while whaNTV sat in the tray, something no official client
// does.
var (
	desiredPresenceMu sync.Mutex
	desiredPresence   = types.PresenceUnavailable
	sentPresence      types.Presence
)

func handleSetPresence(raw json.RawMessage) {
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
	desiredPresenceMu.Lock()
	desiredPresence = state
	desiredPresenceMu.Unlock()
	go sendDesiredPresence(false)
}

// sendDesiredPresence sends the frontend's desired presence. Repeats of the
// state already sent on this connection are skipped unless force is set (a
// fresh connection has no presence yet, and whatsmeow needs one sent after
// connecting so the server has our push name).
func sendDesiredPresence(force bool) {
	c := client
	if c == nil || !c.IsLoggedIn() {
		return
	}
	desiredPresenceMu.Lock()
	state := desiredPresence
	if !force && state == sentPresence {
		desiredPresenceMu.Unlock()
		return
	}
	sentPresence = state
	desiredPresenceMu.Unlock()
	if err := c.SendPresence(context.Background(), state); err != nil {
		log.Printf("SendPresence(%s) failed: %v", state, err)
		desiredPresenceMu.Lock()
		sentPresence = ""
		desiredPresenceMu.Unlock()
	}
}

func handleSubscribePresence(raw json.RawMessage) {
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
	err = client.SubscribePresence(context.Background(), jid)
	if err != nil {
		log.Printf("Error subscribing to presence: %v", err)
	} else {
		debugf("Subscribed to presence for %s", jid.String())
	}
}

func handleMarkRead(raw json.RawMessage) {
	var data struct {
		JID        string   `json:"jid"`
		MessageIDs []string `json:"message_ids"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	chatJID, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	// Channel posts get view receipts when they're fetched (refreshChannel),
	// not read receipts.
	if chatJID.Server == types.NewsletterServer {
		return
	}
	if chatJID.Server == "lid" {
		if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), chatJID); err == nil && !pn.IsEmpty() {
			chatJID = pn
		}
	}

	type msgSender struct {
		id        types.MessageID
		senderJID types.JID
	}
	var items []msgSender

	if len(data.MessageIDs) > 0 {
		for _, id := range data.MessageIDs {
			var senderStr, mediaType string
			_ = db.QueryRow("SELECT COALESCE(sender_jid, ''), COALESCE(media_type, '') FROM messages WHERE id = ?", id).Scan(&senderStr, &mediaType)
			// Event lines and call rows are ours, with no real message behind
			// them to acknowledge.
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
			if sJID.Server == "lid" {
				if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), sJID); err == nil && !pn.IsEmpty() {
					sJID = pn
				}
			}
			items = append(items, msgSender{id: types.MessageID(id), senderJID: sJID})
		}
	} else {
		// Opening a chat: read receipts for what's actually still unread, the
		// way the official apps do - not a blanket re-send for the newest 100
		// incoming messages every single time a chat is opened, most of them
		// long since read. System lines and call rows have no real message
		// behind them to acknowledge.
		rows, err := db.Query(`SELECT id, sender_jid FROM messages
			WHERE jid = ? AND from_me = 0 AND COALESCE(read_at, 0) = 0 AND status NOT IN ('read', 'played')
				AND COALESCE(media_type, '') NOT IN ('system', 'call') AND id NOT LIKE 'system\_%' ESCAPE '\'
			ORDER BY timestamp DESC LIMIT 100`, data.JID)
		if err == nil {
			for rows.Next() {
				var id, senderStr string
				if err := rows.Scan(&id, &senderStr); err == nil {
					sJID, _ := types.ParseJID(senderStr)
					if sJID.IsEmpty() {
						if chatJID.Server == types.GroupServer {
							continue
						}
						sJID = chatJID
					}
					if sJID.Server == "lid" {
						if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), sJID); err == nil && !pn.IsEmpty() {
							sJID = pn
						}
					}
					items = append(items, msgSender{id: types.MessageID(id), senderJID: sJID})
				}
			}
			rows.Close()
		}
	}

	bySender := map[string][]types.MessageID{}
	senderJIDMap := map[string]types.JID{}

	for _, item := range items {
		key := item.senderJID.ToNonAD().String()
		bySender[key] = append(bySender[key], item.id)
		senderJIDMap[key] = item.senderJID
	}

	now := time.Now()
	// Remember they're read, so the next open doesn't acknowledge them again.
	for _, item := range items {
		db.Exec(`UPDATE messages SET read_at = ? WHERE id = ? AND from_me = 0 AND COALESCE(read_at, 0) = 0`, now.Unix(), string(item.id))
	}
	for key, ids := range bySender {
		sJID := senderJIDMap[key]
		go func(mIDs []types.MessageID, targetSender types.JID) {
			err := client.MarkRead(context.Background(), mIDs, now, chatJID, targetSender)
			if err != nil {
				log.Printf("MarkRead error for chat %s sender %s: %v", chatJID, targetSender, err)
			}
		}(ids, sJID)
	}

	// Always clear unread_count when chat is marked read
	db.Exec("UPDATE chats SET unread_count = 0 WHERE jid = ?", data.JID)
	broadcastEvent("chat_updated", map[string]interface{}{
		"jid":          data.JID,
		"unread_count": 0,
	})
}

func handleGetMessageInfo(conn net.Conn, raw json.RawMessage) {
	var data struct {
		ID string `json:"id"`
	}
	if err := json.Unmarshal(raw, &data); err != nil || data.ID == "" {
		return
	}
	var sentAt, deliveredAt, readAt, playedAt, editedAt, msgTS int64
	var mediaType, jid string
	var fromMe bool
	err := db.QueryRow(
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

	// Official WhatsApp only breaks receipts down per-member for messages you
	// sent to a group - it has no visibility into who read someone else's
	// message either, so we match that scope.
	if fromMe && strings.HasSuffix(jid, "@g.us") {
		if participants := groupMessageReceiptParticipants(data.ID, jid); len(participants) > 0 {
			result["participants"] = participants
		}
	}

	if poll := pollInfoForMessage(data.ID); poll != nil {
		result["poll"] = poll
	}

	sendEvent(conn, "message_info", result)
}

// groupMessageReceiptParticipants lists every other member of groupJID with
// whatever delivery/read status has been observed for messageID, for the
// Message Info "Read by" breakdown. A member with no message_receipts row
// just hasn't had a receipt recorded since that table existed - it does not
// mean the message wasn't delivered.
func groupMessageReceiptParticipants(messageID, groupJID string) []map[string]interface{} {
	jid, err := types.ParseJID(groupJID)
	if err != nil {
		return nil
	}
	info, err := client.GetGroupInfo(context.Background(), jid)
	if err != nil {
		return nil
	}
	myJID := client.Store.ID.ToNonAD()

	rows, err := db.Query(`SELECT participant_jid, status, timestamp,
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
			// Rows written before the per-state columns existed only have
			// the latest state's time.
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
			if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), displayJID); err == nil && !pn.IsEmpty() {
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
			"name":         resolveDisplayName(displayJID),
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
	MessageID string   `json:"message_id"`
	ToJIDs    []string `json:"to_jids"`
}

func handleForwardMessage(conn net.Conn, raw json.RawMessage) {
	var data ForwardMessageData
	if err := json.Unmarshal(raw, &data); err != nil || data.MessageID == "" || len(data.ToJIDs) == 0 {
		return
	}
	go func() {
		rawBytes, sourceJID, _, _, err := getMessageRaw(data.MessageID)
		if err != nil || len(rawBytes) == 0 {
			sendEvent(conn, "error", map[string]string{"message": "Message not found for forwarding"})
			return
		}
		var origMsg waE2E.Message
		if unmarshalErr := proto.Unmarshal(rawBytes, &origMsg); unmarshalErr != nil {
			sendEvent(conn, "error", map[string]string{"message": "Failed to parse message protobuf for forwarding"})
			return
		}

		// View-once media can't be forwarded in the official apps.
		if origMsg.GetViewOnceMessage() != nil || origMsg.GetViewOnceMessageV2() != nil || origMsg.GetViewOnceMessageV2Extension() != nil {
			sendEvent(conn, "error", map[string]string{"message": "View once messages can't be forwarded."})
			return
		}
		// Forward the content itself, not the envelope it arrived in
		// (disappearing-message wrapper, an edit, ...), with a fresh
		// context: forwarded, forwarding score bumped the way official
		// clients do, and none of the original's reply quote - which
		// otherwise showed up for the recipient as a reply to a message
		// from some other chat entirely.
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
			// Like the official apps, a post forwarded out of a channel says
			// which channel, so it links back there.
			forwardCtx.ForwardedNewsletterMessageInfo = forwardedChannelInfo(sourceJID, data.MessageID)
		}
		if !setContextInfo(content, forwardCtx) {
			sendEvent(conn, "error", map[string]string{"message": "This kind of message can't be forwarded."})
			return
		}
		for _, toJIDStr := range data.ToJIDs {
			targetJID, parseErr := types.ParseJID(toJIDStr)
			if parseErr != nil {
				continue
			}
			toChannel := targetJID.Server == types.NewsletterServer
			if toChannel && !canPostInChannel(targetJID.String()) {
				sendEvent(conn, "error", map[string]string{"message": "You can only forward to channels you run."})
				continue
			}
			outgoing := content
			var extra whatsmeow.SendRequestExtra
			if toChannel != fromChannel && mediaPart(content) != nil {
				// Chat media is encrypted and channel media isn't, so media
				// crossing between them has to go up again the other way.
				outgoing = proto.Clone(content).(*waE2E.Message)
				handle, reErr := reuploadMedia(context.Background(), outgoing, toChannel)
				if reErr != nil {
					log.Printf("Couldn't re-upload forwarded media for %s: %v", toJIDStr, reErr)
					sendEvent(conn, "error", map[string]string{"message": "Couldn't forward the media: " + reErr.Error()})
					continue
				}
				extra.MediaHandle = handle
			}
			if !toChannel {
				// Each target chat has its own disappearing timer, so the
				// shared content can't carry one.
				outgoing = applyDisappearingTimer(targetJID, proto.Clone(outgoing).(*waE2E.Message))
			}
			resp, sendErr := client.SendMessage(context.Background(), targetJID, outgoing, extra)
			if sendErr != nil {
				log.Printf("Error forwarding message to %s: %v", toJIDStr, sendErr)
				continue
			}
			ts := resp.Timestamp.Unix()
			text := extractText(content)
			senderJID := client.Store.ID.ToNonAD().String()
			fwdRawBytes, _ := proto.Marshal(outgoing)
			mediaType, mimetype, filename, mediaSize := mediaInfoFromMessage(outgoing)
			quotedID, quotedText, quotedSender := quotedPreview(outgoing)

			db.Exec(`INSERT INTO chats (jid, timestamp, preview_snippet, preview_sender, preview_from_me) VALUES (?, ?, ?, ?, 1)
				ON CONFLICT(jid) DO UPDATE SET
				timestamp=MAX(timestamp, excluded.timestamp),
				preview_snippet=COALESCE(NULLIF(excluded.preview_snippet, ''), chats.preview_snippet),
				preview_sender=excluded.preview_sender,
				preview_from_me=1,
				preview_is_reaction=0,
				preview_reaction_emoji=''`,
				targetJID.String(), ts, text, senderJID)

			saveMessage(StoredMessage{
				ID: resp.ID, JID: targetJID.String(), SenderJID: senderJID, FromMe: true,
				Text: text, Timestamp: ts, Raw: fwdRawBytes, Status: "sent",
				MediaType: mediaType, MediaMimetype: mimetype, MediaFilename: filename, MediaSize: mediaSize,
				QuotedID: quotedID, QuotedText: quotedText, QuotedSender: quotedSender,
				ServerID: int64(resp.ServerID),
			})
			noteMessageExpiry(db, resp.ID, ts, outgoing)

			broadcastEvent("message", map[string]interface{}{
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
		broadcastChats()
	}()
}
