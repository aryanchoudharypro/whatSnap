package main

import (
	"context"
	"encoding/json"
	"log"
	"strings"
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
	log.Printf("Sending message to %s: %s", data.To, data.Text)
	jid, err := types.ParseJID(data.To)
	if err != nil {
		return
	}

	// Pick the message ID up front (instead of taking whatever whatsmeow
	// generates on success) so the pending row we persist right now and the
	// eventual sent/failed row are the same message throughout.
	id := whatsmeow.GenerateMessageID()

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

	senderJID := ""
	if client.Store.ID != nil {
		senderJID = client.Store.ID.ToNonAD().String()
	}
	ts := time.Now().Unix()

	// Persist and broadcast as "pending" immediately, before the network
	// attempt: this is the local echo, and it's what makes a failed/offline
	// send recoverable instead of just vanishing.
	db.Exec(`INSERT INTO chats (jid, timestamp, preview_snippet) VALUES (?, ?, ?)
			ON CONFLICT(jid) DO UPDATE SET
			timestamp=MAX(timestamp, excluded.timestamp),
			preview_snippet=COALESCE(NULLIF(excluded.preview_snippet, ''), chats.preview_snippet),
			preview_is_reaction=0,
			preview_reaction_emoji=''`,
		jid.String(), ts, data.Text)
	saveMessage(StoredMessage{
		ID: id, JID: jid.String(), SenderJID: senderJID, FromMe: true,
		Text: data.Text, Timestamp: ts, Status: "pending",
		QuotedID: quotedID, QuotedText: quotedText, QuotedSender: quotedSender,
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
		"quotedSender": quotedSender,
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

	var msg *waE2E.Message
	finalQuotedID, quotedText, finalQuotedSender := "", "", ""
	if quotedID != "" {
		quotedMsg, storedSenderJID, preview, qErr := getMessageForQuote(quotedID)
		participant := quotedSender
		if participant == "" {
			participant = storedSenderJID
		}
		if qErr == nil {
			finalQuotedID, quotedText, finalQuotedSender = quotedID, preview, participant
			ctxInfo := &waE2E.ContextInfo{StanzaID: proto.String(quotedID), QuotedMessage: quotedMsg}
			if participant != "" {
				ctxInfo.Participant = proto.String(participant)
			}
			msg = &waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{Text: &text, ContextInfo: ctxInfo}}
		}
	}
	if msg == nil {
		msg = &waE2E.Message{Conversation: &text}
	}

	resp, err := client.SendMessage(context.Background(), jid, msg, whatsmeow.SendRequestExtra{ID: id})
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
	db.Exec(`INSERT INTO chats (jid, timestamp, preview_snippet) VALUES (?, ?, ?)
			ON CONFLICT(jid) DO UPDATE SET
			timestamp=MAX(timestamp, excluded.timestamp),
			preview_snippet=COALESCE(NULLIF(excluded.preview_snippet, ''), chats.preview_snippet),
			preview_is_reaction=0,
			preview_reaction_emoji=''`,
		jid.String(), ts, text)

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
	senderJID := types.EmptyJID
	if data.SenderJID != "" {
		if parsed, perr := types.ParseJID(data.SenderJID); perr == nil {
			senderJID = parsed
		}
	}
	go func() {
		msg := client.BuildReaction(chatJID, senderJID, data.MessageID, data.Emoji)
		resp, err := client.SendMessage(context.Background(), chatJID, msg, whatsmeow.SendRequestExtra{ID: whatsmeow.GenerateMessageID()})
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
		newContent := &waE2E.Message{Conversation: proto.String(data.NewText)}
		msg := client.BuildEdit(chatJID, data.MessageID, newContent)

		// The edit travels as its own message with its own ID, and every
		// receipt for it comes back under that ID. Note the pairing before
		// sending - the first receipt can arrive before SendMessage returns.
		editStanzaID := whatsmeow.GenerateMessageID()
		recordEditStanza(editStanzaID, data.MessageID, data.JID, time.Now().Unix())

		_, err := client.SendMessage(context.Background(), chatJID, msg, whatsmeow.SendRequestExtra{ID: editStanzaID})
		if err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		rawBytes, _ := proto.Marshal(newContent)
		editTime := time.Now().Unix()
		markMessageEdited(data.MessageID, data.JID, data.NewText, rawBytes, editTime)
		broadcastEvent("message_edited", map[string]interface{}{
			"jid": data.JID, "message_id": data.MessageID, "text": data.NewText, "edited_at": editTime,
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
				if senderJID.IsEmpty() && client.Store.ID != nil {
					senderJID = client.Store.ID.ToNonAD()
				}
			}

			msg := client.BuildRevoke(targetChatJID, senderJID, data.MessageID)
			if _, err := client.SendMessage(context.Background(), targetChatJID, msg, whatsmeow.SendRequestExtra{ID: whatsmeow.GenerateMessageID()}); err != nil {
				errMsg := err.Error()
				if strings.Contains(errMsg, "479") {
					errMsg = "WhatsApp error 479: Failed to delete for everyone. The message key mismatch or the revoke time limit expired."
				}
				broadcastEvent("error", map[string]string{"message": errMsg})
				return
			}
		} else {
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
	go client.SendPresence(context.Background(), state)
}

func handleSubscribePresence(raw json.RawMessage) {
	var data struct {
		JID string `json:"jid"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		log.Printf("Failed to unmarshal subscribe_presence: %v", err)
		return
	}
	log.Printf("Received subscribe_presence for JID: %s", data.JID)
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		log.Printf("Failed to parse JID for subscribe_presence: %v", err)
		return
	}
	err = client.SubscribePresence(context.Background(), jid)
	if err != nil {
		log.Printf("Error subscribing to presence: %v", err)
	} else {
		log.Printf("Successfully subscribed to presence for JID: %s", jid.String())
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
			var senderStr string
			_ = db.QueryRow("SELECT sender_jid FROM messages WHERE id = ?", id).Scan(&senderStr)
			sJID, _ := types.ParseJID(senderStr)
			if sJID.IsEmpty() {
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
		// Auto-detect incoming unread messages from DB
		rows, err := db.Query("SELECT id, sender_jid FROM messages WHERE jid = ? AND from_me = 0 ORDER BY timestamp DESC LIMIT 100", data.JID)
		if err == nil {
			for rows.Next() {
				var id, senderStr string
				if err := rows.Scan(&id, &senderStr); err == nil {
					sJID, _ := types.ParseJID(senderStr)
					if sJID.IsEmpty() {
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
	var mediaType string
	err := db.QueryRow(
		`SELECT COALESCE(sent_at,0), COALESCE(delivered_at,0), COALESCE(read_at,0), COALESCE(played_at,0), COALESCE(edited_at,0), COALESCE(media_type,''), COALESCE(timestamp,0) FROM messages WHERE id = ?`,
		data.ID,
	).Scan(&sentAt, &deliveredAt, &readAt, &playedAt, &editedAt, &mediaType, &msgTS)
	if err != nil {
		return
	}
	if sentAt == 0 && msgTS > 0 {
		sentAt = msgTS
	}
	sendEvent(conn, "message_info", map[string]interface{}{
		"id":           data.ID,
		"sent_at":      sentAt,
		"delivered_at": deliveredAt,
		"read_at":      readAt,
		"played_at":    playedAt,
		"edited_at":    editedAt,
		"media_type":   mediaType,
	})
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
		rawBytes, _, _, _, err := getMessageRaw(data.MessageID)
		if err != nil || len(rawBytes) == 0 {
			sendEvent(conn, "error", map[string]string{"message": "Message not found for forwarding"})
			return
		}
		var origMsg waE2E.Message
		if unmarshalErr := proto.Unmarshal(rawBytes, &origMsg); unmarshalErr != nil {
			sendEvent(conn, "error", map[string]string{"message": "Failed to parse message protobuf for forwarding"})
			return
		}

		if ctx := getContextInfo(&origMsg); ctx != nil {
			ctx.IsForwarded = proto.Bool(true)
		} else {
			if origMsg.ExtendedTextMessage != nil {
				origMsg.ExtendedTextMessage.ContextInfo = &waE2E.ContextInfo{IsForwarded: proto.Bool(true)}
			} else if origMsg.Conversation != nil {
				txt := origMsg.GetConversation()
				origMsg.Conversation = nil
				origMsg.ExtendedTextMessage = &waE2E.ExtendedTextMessage{
					Text:        &txt,
					ContextInfo: &waE2E.ContextInfo{IsForwarded: proto.Bool(true)},
				}
			} else if origMsg.ImageMessage != nil {
				origMsg.ImageMessage.ContextInfo = &waE2E.ContextInfo{IsForwarded: proto.Bool(true)}
			} else if origMsg.VideoMessage != nil {
				origMsg.VideoMessage.ContextInfo = &waE2E.ContextInfo{IsForwarded: proto.Bool(true)}
			} else if origMsg.DocumentMessage != nil {
				origMsg.DocumentMessage.ContextInfo = &waE2E.ContextInfo{IsForwarded: proto.Bool(true)}
			} else if origMsg.AudioMessage != nil {
				origMsg.AudioMessage.ContextInfo = &waE2E.ContextInfo{IsForwarded: proto.Bool(true)}
			} else if origMsg.StickerMessage != nil {
				origMsg.StickerMessage.ContextInfo = &waE2E.ContextInfo{IsForwarded: proto.Bool(true)}
			} else if origMsg.ContactMessage != nil {
				origMsg.ContactMessage.ContextInfo = &waE2E.ContextInfo{IsForwarded: proto.Bool(true)}
			} else if origMsg.ContactsArrayMessage != nil {
				origMsg.ContactsArrayMessage.ContextInfo = &waE2E.ContextInfo{IsForwarded: proto.Bool(true)}
			} else if origMsg.LocationMessage != nil {
				origMsg.LocationMessage.ContextInfo = &waE2E.ContextInfo{IsForwarded: proto.Bool(true)}
			} else if origMsg.LiveLocationMessage != nil {
				origMsg.LiveLocationMessage.ContextInfo = &waE2E.ContextInfo{IsForwarded: proto.Bool(true)}
			} else if origMsg.GroupInviteMessage != nil {
				origMsg.GroupInviteMessage.ContextInfo = &waE2E.ContextInfo{IsForwarded: proto.Bool(true)}
			} else if origMsg.PollCreationMessage != nil {
				origMsg.PollCreationMessage.ContextInfo = &waE2E.ContextInfo{IsForwarded: proto.Bool(true)}
			}
		}

		for _, toJIDStr := range data.ToJIDs {
			targetJID, parseErr := types.ParseJID(toJIDStr)
			if parseErr != nil {
				continue
			}
			resp, sendErr := client.SendMessage(context.Background(), targetJID, &origMsg)
			if sendErr != nil {
				log.Printf("Error forwarding message to %s: %v", toJIDStr, sendErr)
				continue
			}
			ts := resp.Timestamp.Unix()
			text := extractText(&origMsg)
			senderJID := client.Store.ID.ToNonAD().String()
			fwdRawBytes, _ := proto.Marshal(&origMsg)
			mediaType, mimetype, filename := mediaInfoFromMessage(&origMsg)
			quotedID, quotedText, quotedSender := quotedPreview(&origMsg)

			db.Exec(`INSERT INTO chats (jid, timestamp, preview_snippet) VALUES (?, ?, ?)
				ON CONFLICT(jid) DO UPDATE SET
				timestamp=MAX(timestamp, excluded.timestamp),
				preview_snippet=COALESCE(NULLIF(excluded.preview_snippet, ''), chats.preview_snippet),
				preview_is_reaction=0,
				preview_reaction_emoji=''`,
				targetJID.String(), ts, text)

			saveMessage(StoredMessage{
				ID: resp.ID, JID: targetJID.String(), SenderJID: senderJID, FromMe: true,
				Text: text, Timestamp: ts, Raw: fwdRawBytes, Status: "sent",
				MediaType: mediaType, MediaMimetype: mimetype, MediaFilename: filename,
				QuotedID: quotedID, QuotedText: quotedText, QuotedSender: quotedSender,
			})

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
				"quotedId":      quotedID,
				"quotedText":    quotedText,
				"quotedSender":  quotedSender,
			})
		}
		broadcastChats()
	}()
}
