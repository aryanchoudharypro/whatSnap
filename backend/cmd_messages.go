package main

import (
	"context"
	"encoding/json"
	"log"
	"strings"
	"time"

	"github.com/gorilla/websocket"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
)

func handleSend(conn *websocket.Conn, raw json.RawMessage) {
	var data SendCommandData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	log.Printf("Sending message to %s: %s", data.To, data.Text)
	jid, err := types.ParseJID(data.To)
	if err != nil {
		return
	}

	var msg *waE2E.Message
	quotedID, quotedText, quotedSender := "", "", ""
	if data.ReplyToID != "" {
		quotedMsg, storedSenderJID, preview, qErr := getMessageForQuote(data.ReplyToID)
		participant := data.ReplySender
		if participant == "" {
			participant = storedSenderJID
		}
		if qErr == nil {
			quotedID, quotedText, quotedSender = data.ReplyToID, preview, participant
			ctxInfo := &waE2E.ContextInfo{StanzaID: proto.String(data.ReplyToID), QuotedMessage: quotedMsg}
			if participant != "" {
				ctxInfo.Participant = proto.String(participant)
			}
			msg = &waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{Text: &data.Text, ContextInfo: ctxInfo}}
		}
	}
	if msg == nil {
		msg = &waE2E.Message{Conversation: &data.Text}
	}

	resp, err := client.SendMessage(context.Background(), jid, msg)
	if err != nil {
		log.Printf("Error sending message: %v", err)
		sendEvent(conn, "error", map[string]string{"message": err.Error()})
		return
	}
	log.Printf("Message sent successfully with ID %s", resp.ID)
	ts := resp.Timestamp.Unix()
	senderJID := client.Store.ID.ToNonAD().String()
	rawBytes, _ := proto.Marshal(msg)

	if _, dbErr := db.Exec(`INSERT INTO chats (jid, timestamp, preview_snippet) VALUES (?, ?, ?)
			ON CONFLICT(jid) DO UPDATE SET 
			timestamp=MAX(timestamp, excluded.timestamp),
			preview_snippet=COALESCE(NULLIF(excluded.preview_snippet, ''), chats.preview_snippet)`,
		jid.String(), ts, data.Text); dbErr != nil {
		log.Println("Error updating chat on send:", dbErr)
	}
	if dbErr := saveMessage(StoredMessage{
		ID: resp.ID, JID: jid.String(), SenderJID: senderJID, FromMe: true,
		Text: data.Text, Timestamp: ts, Raw: rawBytes, Status: "sent",
		QuotedID: quotedID, QuotedText: quotedText, QuotedSender: quotedSender,
	}); dbErr != nil {
		log.Println("Error inserting sent message:", dbErr)
	}

	broadcastEvent("message", map[string]interface{}{
		"jid":          jid.String(),
		"from":         "me",
		"id":           resp.ID,
		"text":         data.Text,
		"from_me":      true,
		"timestamp":    ts,
		"has_media":    false,
		"quotedId":     quotedID,
		"quotedText":   quotedText,
		"quotedSender": quotedSender,
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
		myJID := client.Store.ID.ToNonAD().String()
		upsertReaction(data.MessageID, data.JID, myJID, data.Emoji, resp.Timestamp.Unix())
		broadcastEvent("message_reaction", map[string]interface{}{
			"jid": data.JID, "message_id": data.MessageID, "reactor": myJID, "emoji": data.Emoji,
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
		_, err := client.SendMessage(context.Background(), chatJID, msg, whatsmeow.SendRequestExtra{ID: whatsmeow.GenerateMessageID()})
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
		if data.ForEveryone {
			targetChatJID := chatJID
			var storedJIDStr, storedSenderStr string
			var storedFromMe bool
			if dbErr := db.QueryRow("SELECT jid, COALESCE(sender_jid, ''), from_me FROM messages WHERE id = ?", data.MessageID).Scan(&storedJIDStr, &storedSenderStr, &storedFromMe); dbErr == nil {
				if parsed, perr := types.ParseJID(storedJIDStr); perr == nil && !parsed.IsEmpty() {
					targetChatJID = parsed
				}
			}

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
		}
		markMessageDeleted(data.MessageID, data.JID)
		broadcastEvent("message_deleted", map[string]interface{}{
			"jid": data.JID, "message_id": data.MessageID, "by": "me",
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

func handleSubscribePresence(raw json.RawMessage) {
	var data struct {
		JID string `json:"jid"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go client.SubscribePresence(context.Background(), jid)
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

func handleGetMessageInfo(conn *websocket.Conn, raw json.RawMessage) {
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
