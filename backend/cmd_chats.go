package main

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"log"
	"strings"
	"time"

	"net"
	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waCommon"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
)

func handleGetChats(conn net.Conn) {
	go func() {
		sendEvent(conn, "chats", buildChatsList())
	}()
}

func handleGetMessages(conn net.Conn, raw json.RawMessage) {
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
				COALESCE(quoted_id, ''), COALESCE(quoted_text, ''), COALESCE(quoted_sender, ''), COALESCE(is_edited, 0), COALESCE(is_deleted, 0),
				COALESCE(sent_at, 0), COALESCE(delivered_at, 0), COALESCE(read_at, 0), COALESCE(played_at, 0), COALESCE(edited_at, 0), COALESCE(starred, 0), COALESCE(album_id, ''), raw
			FROM messages WHERE jid = ? ORDER BY timestamp DESC, id DESC LIMIT ? OFFSET ?`, data.JID, limit, offset)

		msgs := []map[string]interface{}{}
		var ids []string
		if queryErr == nil {
			defer rows.Close()
			for rows.Next() {
				var id, text, senderJID, status, mediaType, mediaMimetype, mediaFilename, mediaPath string
				var quotedID, quotedText, quotedSender, albumID string
				var ts, sentAt, deliveredAt, readAt, playedAt, editedAt int64
				var fromMe bool
				var isEdited, isDeleted, starred int
				var rawBytes []byte
				if err := rows.Scan(&id, &text, &ts, &fromMe, &senderJID, &status, &mediaType, &mediaMimetype, &mediaFilename, &mediaPath,
					&quotedID, &quotedText, &quotedSender, &isEdited, &isDeleted, &sentAt, &deliveredAt, &readAt, &playedAt, &editedAt, &starred, &albumID, &rawBytes); err != nil {
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
					"sentAt":        sentAt,
					"deliveredAt":   deliveredAt,
					"readAt":        readAt,
					"playedAt":      playedAt,
					"editedAt":      editedAt,
					"starred":       starred == 1,
					"albumId":       albumID,
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

// handleGetStarredMessages returns every starred message across all chats,
// newest first, for the Starred Messages nav view.
func handleGetStarredMessages(conn net.Conn) {
	go func() {
		rows, err := db.Query(`SELECT id, jid, COALESCE(text,''), timestamp, from_me, COALESCE(sender_jid,''),
				COALESCE(media_type,''), COALESCE(quoted_text,'')
			FROM messages WHERE starred = 1 AND COALESCE(is_deleted, 0) = 0 ORDER BY timestamp DESC LIMIT 500`)
		results := []map[string]interface{}{}
		if err == nil {
			defer rows.Close()
			for rows.Next() {
				var id, jid, text, senderJID, mediaType, quotedText string
				var ts int64
				var fromMe bool
				if scanErr := rows.Scan(&id, &jid, &text, &ts, &fromMe, &senderJID, &mediaType, &quotedText); scanErr == nil {
					senderName := ""
					if senderJID != "" {
						if parsed, perr := types.ParseJID(senderJID); perr == nil {
							senderName = resolveDisplayName(parsed)
						}
					}
					results = append(results, map[string]interface{}{
						"id":         id,
						"jid":        jid,
						"fromMe":     fromMe,
						"text":       text,
						"timestamp":  ts,
						"senderJid":  senderJID,
						"senderName": senderName,
						"mediaType":  mediaType,
						"quotedText": quotedText,
						"starred":    true,
					})
				}
			}
		} else {
			log.Println("Error querying starred messages:", err)
		}
		sendEvent(conn, "starred_messages", map[string]interface{}{
			"results": results,
		})
	}()
}

// handleSearchMessages implements real full-text search over the local
// message cache (previously a stub). Search is scoped to one chat when jid
// is provided, otherwise it searches everything.
func handleSearchMessages(conn net.Conn, raw json.RawMessage) {
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
				COALESCE(sent_at,0), COALESCE(delivered_at,0), COALESCE(read_at,0), COALESCE(played_at,0), COALESCE(edited_at,0), COALESCE(starred,0) FROM messages
				WHERE jid = ? AND text LIKE ? AND COALESCE(is_deleted, 0) = 0 ORDER BY timestamp ASC LIMIT 200`, data.JID, like)
		} else {
			rows, err = db.Query(`SELECT id, jid, COALESCE(text,''), timestamp, from_me, COALESCE(sender_jid,''),
				COALESCE(status,''), COALESCE(media_type,''), COALESCE(media_mimetype,''), COALESCE(media_filename,''), COALESCE(media_path,''),
				COALESCE(quoted_id,''), COALESCE(quoted_text,''), COALESCE(quoted_sender,''), COALESCE(is_edited,0), COALESCE(is_deleted,0),
				COALESCE(sent_at,0), COALESCE(delivered_at,0), COALESCE(read_at,0), COALESCE(played_at,0), COALESCE(edited_at,0), COALESCE(starred,0) FROM messages
				WHERE text LIKE ? AND COALESCE(is_deleted, 0) = 0 ORDER BY timestamp ASC LIMIT 200`, like)
		}
		results := []map[string]interface{}{}
		if err == nil {
			defer rows.Close()
			for rows.Next() {
				var id, jid, text, senderJID, status, mediaType, mediaMimetype, mediaFilename, mediaPath string
				var quotedID, quotedText, quotedSender string
				var ts, sentAt, deliveredAt, readAt, playedAt, editedAt int64
				var fromMe bool
				var isEdited, isDeleted, starred int
				if scanErr := rows.Scan(&id, &jid, &text, &ts, &fromMe, &senderJID, &status, &mediaType, &mediaMimetype, &mediaFilename, &mediaPath,
					&quotedID, &quotedText, &quotedSender, &isEdited, &isDeleted, &sentAt, &deliveredAt, &readAt, &playedAt, &editedAt, &starred); scanErr == nil {
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
						"editedAt":      editedAt,
						"starred":       starred == 1,
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

func getLastMessageKeyForChat(jid_str string) (time.Time, *waCommon.MessageKey) {
	var msg_id string
	var msg_ts int64
	var from_me bool
	var sender_jid string
	err := db.QueryRow(
		`SELECT id, timestamp, from_me, COALESCE(sender_jid, '') FROM messages WHERE jid = ? ORDER BY timestamp DESC LIMIT 1`,
		jid_str,
	).Scan(&msg_id, &msg_ts, &from_me, &sender_jid)
	if err != nil {
		return time.Time{}, nil
	}
	key := &waCommon.MessageKey{
		RemoteJID: proto.String(jid_str),
		ID:        proto.String(msg_id),
		FromMe:    proto.Bool(from_me),
	}
	if sender_jid != "" && sender_jid != jid_str {
		key.Participant = proto.String(sender_jid)
	}
	return time.Unix(msg_ts, 0), key
}

func sendAppStateWithRetry(ctx context.Context, patch appstate.PatchInfo) error {
	err := client.SendAppState(ctx, patch)
	if err == nil {
		return nil
	}
	log.Printf("SendAppState %s failed, attempting resync: %v", patch.Type, err)

	appStateResyncMu.Lock()
	lastAppStateResync[patch.Type] = time.Now()
	appStateResyncMu.Unlock()

	if sync_err := client.FetchAppState(ctx, patch.Type, false, false); sync_err != nil {
		log.Printf("Resync of %s failed: %v", patch.Type, sync_err)
		return fmt.Errorf("app state sync failed for %s: %w", patch.Type, sync_err)
	}
	if retry_err := client.SendAppState(ctx, patch); retry_err != nil {
		log.Printf("SendAppState %s retry also failed: %v", patch.Type, retry_err)
		return fmt.Errorf("app state retry failed for %s: %w", patch.Type, retry_err)
	}
	return nil
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
		if err := sendAppStateWithRetry(context.Background(), appstate.BuildPin(jid, data.Value)); err != nil {
			broadcastChats()
			return
		}
		pinned_int := 0
		if data.Value {
			pinned_int = 1
		}
		db.Exec("UPDATE chats SET pinned = ? WHERE jid = ?", pinned_int, data.JID)
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
		last_ts, last_key := getLastMessageKeyForChat(data.JID)
		if err := sendAppStateWithRetry(context.Background(), appstate.BuildArchive(jid, data.Value, last_ts, last_key)); err != nil {
			broadcastChats()
			return
		}
		archived_int := 0
		if data.Value {
			archived_int = 1
		}
		db.Exec("UPDATE chats SET archived = ? WHERE jid = ?", archived_int, data.JID)
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
		if err := sendAppStateWithRetry(context.Background(), appstate.BuildMute(jid, data.Muted, duration)); err != nil {
			broadcastChats()
			return
		}
		muted_int := 0
		if data.Muted {
			muted_int = 1
		}
		db.Exec("UPDATE chats SET muted = ? WHERE jid = ?", muted_int, data.JID)
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
		last_ts, last_key := getLastMessageKeyForChat(data.JID)
		if err := sendAppStateWithRetry(context.Background(), appstate.BuildMarkChatAsRead(jid, data.Value, last_ts, last_key)); err != nil {
			broadcastChats()
			return
		}
		if data.Value {
			db.Exec("UPDATE chats SET unread_count = 0 WHERE jid = ?", data.JID)
		} else {
			db.Exec("UPDATE chats SET unread_count = 1 WHERE jid = ?", data.JID)
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

// handleRequestMessageResend backs the "Request again" message action: the one
// repair available for a message that arrived unreadable, or whose content we
// lost. Nothing is guaranteed - the phone has to still hold the message and be
// reachable - so this stays silent and lets the message reappear on its own if
// it works.
func handleRequestMessageResend(raw json.RawMessage) {
	var data struct {
		JID       string `json:"jid"`
		MessageID string `json:"message_id"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}

	var senderJID string
	db.QueryRow("SELECT COALESCE(sender_jid, '') FROM messages WHERE id = ?", data.MessageID).Scan(&senderJID)
	requestMessageResend(data.MessageID, data.JID, senderJID)
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

	if _, sendErr := client.SendPeerMessage(context.Background(), req); sendErr != nil {
		log.Printf("history sync request for %s failed: %v", data.JID, sendErr)
	}
}
