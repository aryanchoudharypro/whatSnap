package main

import (
	"context"
	"database/sql"
	"encoding/json"
	"log"
	"strings"
	"time"

	"github.com/gorilla/websocket"
	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
)

func handleGetChats(conn *websocket.Conn) {
	go func() {
		sendEvent(conn, "chats", buildChatsList())
	}()
}

func handleGetMessages(conn *websocket.Conn, raw json.RawMessage) {
	var data struct {
		JID    string `json:"jid"`
		Limit  int    `json:"limit"`
		Offset int    `json:"offset"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		db.Exec("UPDATE chats SET unread_count = 0 WHERE jid = ?", data.JID)
		broadcastChats()

		limit := data.Limit
		if limit <= 0 {
			limit = 50
		}
		offset := data.Offset
		if offset < 0 {
			offset = 0
		}

		log.Printf("Getting messages for %s (Limit: %d, Offset: %d)", data.JID, limit, offset)

		rows, queryErr := db.Query(`SELECT id, COALESCE(text, ''), timestamp, from_me, COALESCE(sender_jid, ''),
				COALESCE(status, ''), COALESCE(media_type, ''), COALESCE(media_mimetype, ''), COALESCE(media_filename, ''), COALESCE(media_path, ''),
				COALESCE(quoted_id, ''), COALESCE(quoted_text, ''), COALESCE(quoted_sender, ''), COALESCE(is_edited, 0), COALESCE(is_deleted, 0), raw
			FROM messages WHERE jid = ? ORDER BY timestamp DESC, id DESC LIMIT ? OFFSET ?`, data.JID, limit, offset)

		msgs := []map[string]interface{}{}
		var ids []string
		if queryErr == nil {
			defer rows.Close()
			for rows.Next() {
				var id, text, senderJID, status, mediaType, mediaMimetype, mediaFilename, mediaPath string
				var quotedID, quotedText, quotedSender string
				var ts int64
				var fromMe bool
				var isEdited, isDeleted int
				var rawBytes []byte
				if err := rows.Scan(&id, &text, &ts, &fromMe, &senderJID, &status, &mediaType, &mediaMimetype, &mediaFilename, &mediaPath,
					&quotedID, &quotedText, &quotedSender, &isEdited, &isDeleted, &rawBytes); err != nil {
					log.Println("Error scanning message row:", err)
					continue
				}
				if isDeleted == 1 {
					if fromMe {
						text = "[You deleted this message]"
					} else {
						text = "[This message was deleted]"
					}
				}

				if mediaType == "voice" || mediaType == "audio" || strings.HasPrefix(text, "[Voice Message]") || strings.HasPrefix(text, "[Audio]") {
					if !strings.Contains(text, "(") && len(rawBytes) > 0 {
						var pbMsg waE2E.Message
						if err := proto.Unmarshal(rawBytes, &pbMsg); err == nil {
							extracted := extractText(&pbMsg)
							if extracted != "" {
								text = extracted
							}
						}
					}
				}

				if strings.HasPrefix(quotedText, "[Voice Message]") || strings.HasPrefix(quotedText, "[Audio]") {
					if !strings.Contains(quotedText, "(") && quotedID != "" {
						if _, _, qPreview, qErr := getMessageForQuote(quotedID); qErr == nil && qPreview != "" {
							quotedText = qPreview
						}
					}
				}

				senderName := ""
				if senderJID != "" {
					if parsedSender, errP := types.ParseJID(senderJID); errP == nil {
						senderName = resolveDisplayName(parsedSender)
					}
				}
				qSenderName := quotedSender
				if quotedSender != "" {
					if quotedSender == "me" || (client != nil && client.Store != nil && client.Store.ID != nil && strings.Contains(quotedSender, client.Store.ID.User)) {
						qSenderName = "You"
					} else if parsed, err := types.ParseJID(quotedSender); err == nil {
						qSenderName = resolveDisplayName(parsed)
					}
				}
				ids = append(ids, id)
				msgs = append(msgs, map[string]interface{}{
					"id":            id,
					"text":          text,
					"timestamp":     ts,
					"fromMe":        fromMe,
					"senderJid":     senderJID,
					"senderName":    senderName,
					"status":        status,
					"mediaType":     mediaType,
					"mediaMimetype": mediaMimetype,
					"mediaFilename": mediaFilename,
					"mediaPath":     mediaPath,
					"quotedId":      quotedID,
					"quotedText":    quotedText,
					"quotedSender":  qSenderName,
					"isEdited":      isEdited == 1,
					"isDeleted":     isDeleted == 1,
				})
			}
		} else {
			log.Println("Error querying messages in get_messages:", queryErr)
		}

		for i, j := 0, len(msgs)-1; i < j; i, j = i+1, j-1 {
			msgs[i], msgs[j] = msgs[j], msgs[i]
		}
		if ids != nil {
			reactions := getReactionsForMessageIDs(ids)
			for _, m := range msgs {
				if rs, ok := reactions[m["id"].(string)]; ok {
					m["reactions"] = rs
				} else {
					m["reactions"] = []ReactionInfo{}
				}
			}
		}

		log.Printf("Got %d messages for %s", len(msgs), data.JID)
		sendEvent(conn, "conversation_messages", map[string]interface{}{
			"jid":      data.JID,
			"messages": msgs,
		})
	}()
}

// handleSearchMessages implements real full-text search over the local
// message cache (previously a stub). Search is scoped to one chat when jid
// is provided, otherwise it searches everything.
func handleSearchMessages(conn *websocket.Conn, raw json.RawMessage) {
	var data SearchMessagesData
	if err := json.Unmarshal(raw, &data); err != nil || data.Query == "" {
		return
	}
	go func() {
		like := "%" + data.Query + "%"
		var rows *sql.Rows
		var err error
		if data.JID != "" {
			rows, err = db.Query(`SELECT id, jid, COALESCE(text,''), timestamp, from_me, COALESCE(sender_jid,''),
				COALESCE(status,''), COALESCE(media_type,''), COALESCE(media_mimetype,''), COALESCE(media_filename,''), COALESCE(media_path,''),
				COALESCE(quoted_id,''), COALESCE(quoted_text,''), COALESCE(quoted_sender,''), COALESCE(is_edited,0), COALESCE(is_deleted,0),
				COALESCE(sent_at,0), COALESCE(delivered_at,0), COALESCE(read_at,0), COALESCE(played_at,0) FROM messages
				WHERE jid = ? AND text LIKE ? AND COALESCE(is_deleted, 0) = 0 ORDER BY timestamp ASC LIMIT 200`, data.JID, like)
		} else {
			rows, err = db.Query(`SELECT id, jid, COALESCE(text,''), timestamp, from_me, COALESCE(sender_jid,''),
				COALESCE(status,''), COALESCE(media_type,''), COALESCE(media_mimetype,''), COALESCE(media_filename,''), COALESCE(media_path,''),
				COALESCE(quoted_id,''), COALESCE(quoted_text,''), COALESCE(quoted_sender,''), COALESCE(is_edited,0), COALESCE(is_deleted,0),
				COALESCE(sent_at,0), COALESCE(delivered_at,0), COALESCE(read_at,0), COALESCE(played_at,0) FROM messages
				WHERE text LIKE ? AND COALESCE(is_deleted, 0) = 0 ORDER BY timestamp ASC LIMIT 200`, like)
		}
		results := []map[string]interface{}{}
		if err == nil {
			defer rows.Close()
			for rows.Next() {
				var id, jid, text, senderJID, status, mediaType, mediaMimetype, mediaFilename, mediaPath string
				var quotedID, quotedText, quotedSender string
				var ts, sentAt, deliveredAt, readAt, playedAt int64
				var fromMe bool
				var isEdited, isDeleted int
				if scanErr := rows.Scan(&id, &jid, &text, &ts, &fromMe, &senderJID, &status, &mediaType, &mediaMimetype, &mediaFilename, &mediaPath,
					&quotedID, &quotedText, &quotedSender, &isEdited, &isDeleted, &sentAt, &deliveredAt, &readAt, &playedAt); scanErr == nil {
					senderName := ""
					if senderJID != "" {
						if parsed, perr := types.ParseJID(senderJID); perr == nil {
							senderName = resolveDisplayName(parsed)
						}
					}
					results = append(results, map[string]interface{}{
						"id":            id,
						"jid":           jid,
						"fromMe":        fromMe,
						"text":          text,
						"timestamp":     ts,
						"senderJid":     senderJID,
						"senderName":    senderName,
						"status":        status,
						"mediaType":     mediaType,
						"mediaMimetype": mediaMimetype,
						"mediaFilename": mediaFilename,
						"mediaPath":     mediaPath,
						"quotedId":      quotedID,
						"quotedText":    quotedText,
						"quotedSender":  quotedSender,
						"isEdited":      isEdited == 1,
						"isDeleted":     isDeleted == 1,
						"sentAt":        sentAt,
						"deliveredAt":   deliveredAt,
						"readAt":        readAt,
						"playedAt":      playedAt,
					})
				}
			}
		} else {
			log.Println("Error searching messages:", err)
		}
		sendEvent(conn, "search_results", map[string]interface{}{
			"query":   data.Query,
			"results": results,
		})
	}()
}

// --- chat-level state: pin / archive / mute / mark read / delete -----------
// These write through to WhatsApp's app-state (SendAppState) rather than
// just the local cache, so pin/archive/mute state matches what you'd see on
// your phone or any other linked device, not just this backend's chat list.

func handleSetChatPinned(raw json.RawMessage) {
	var data ChatBoolData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		if err := client.SendAppState(context.Background(), appstate.BuildPin(jid, data.Value)); err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		pinnedInt := 0
		if data.Value {
			pinnedInt = 1
		}
		db.Exec("UPDATE chats SET pinned = ? WHERE jid = ?", pinnedInt, data.JID)
		broadcastChats()
	}()
}

func handleSetChatArchived(raw json.RawMessage) {
	var data ChatBoolData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		if err := client.SendAppState(context.Background(), appstate.BuildArchive(jid, data.Value, time.Time{}, nil)); err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		archivedInt := 0
		if data.Value {
			archivedInt = 1
		}
		db.Exec("UPDATE chats SET archived = ? WHERE jid = ?", archivedInt, data.JID)
		broadcastChats()
	}()
}

func handleSetChatMuted(raw json.RawMessage) {
	var data SetChatMutedData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		duration := time.Duration(data.DurationSeconds) * time.Second
		if err := client.SendAppState(context.Background(), appstate.BuildMute(jid, data.Muted, duration)); err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		mutedInt := 0
		if data.Muted {
			mutedInt = 1
		}
		db.Exec("UPDATE chats SET muted = ? WHERE jid = ?", mutedInt, data.JID)
		broadcastChats()
	}()
}

func handleSetChatRead(raw json.RawMessage) {
	var data ChatBoolData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		if err := client.SendAppState(context.Background(), appstate.BuildMarkChatAsRead(jid, data.Value, time.Time{}, nil)); err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		if data.Value {
			db.Exec("UPDATE chats SET unread_count = 0 WHERE jid = ?", data.JID)
		}
		broadcastChats()
	}()
}

func handleDeleteChat(raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		db.Exec("DELETE FROM messages WHERE jid = ?", data.JID)
		db.Exec("DELETE FROM chats WHERE jid = ?", data.JID)
		broadcastChats()
	}()
}

func handleRequestHistorySync(raw json.RawMessage) {
	var data struct {
		JID       string `json:"jid"`
		MessageID string `json:"message_id"`
		Count     int    `json:"count"`
	}
	
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}

	chatJID, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}

	var senderJID string
	var fromMe bool
	err = db.QueryRow("SELECT COALESCE(sender_jid, ''), from_me FROM messages WHERE id = ?", data.MessageID).Scan(&senderJID, &fromMe)
	if err != nil {
		return
	}

	sender, _ := types.ParseJID(senderJID)
	if sender.IsEmpty() {
		sender = chatJID
	}

	msgInfo := &types.MessageInfo{
		MessageSource: types.MessageSource{
			Chat:     chatJID,
			Sender:   sender,
			IsFromMe: fromMe,
		},
		ID: data.MessageID,
	}

	req := client.BuildHistorySyncRequest(msgInfo, data.Count)
	
	client.SendPeerMessage(context.Background(), req)
}
