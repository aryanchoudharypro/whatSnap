package main

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"log"
	"time"

	"net"
	"go.mau.fi/whatsmeow/types"
)

func broadcastEvent(evtType string, data interface{}) {
	wsMutex.Lock()
	defer wsMutex.Unlock()

	var raw json.RawMessage
	if data != nil {
		bytes, err := json.Marshal(data)
		if err == nil {
			raw = bytes
		}
	}

	if evtType == "message" {
		log.Printf("Broadcasting 'message' event to WS: %s", string(raw))
	}

	evt := Event{
		Type: evtType,
		Data: raw,
	}

	bytes, err := json.Marshal(evt)
	if err != nil {
		return
	}

	for conn := range wsClients {
		conn.Write(append(bytes, '\n'))
	}
}

func sendEvent(conn net.Conn, evtType string, data interface{}) {
	wsMutex.Lock()
	defer wsMutex.Unlock()
	var raw json.RawMessage
	if data != nil {
		bytes, err := json.Marshal(data)
		if err != nil {
			// Matches the previous behaviour: still send the bare event rather
			// than dropping it, so the UI's dispatch is unchanged. Logged
			// because it used to fail silently.
			log.Println("sendEvent: failed to marshal", evtType, "payload:", err)
		} else {
			raw = bytes
		}
	}

	// Assembled by hand rather than marshalling an Event: `raw` is already
	// valid JSON, so json.Marshal(evt) would copy the whole payload a second
	// time purely to wrap it in {"type":..,"data":..}, and the trailing append
	// could copy it a third. On a chat's full history that is tens of MB per
	// copy. evtType still goes through Marshal so it stays correctly escaped,
	// and the empty-data case matches Event's `omitempty`.
	typeJSON, err := json.Marshal(evtType)
	if err != nil {
		return
	}
	buf := make([]byte, 0, len(raw)+len(typeJSON)+32)
	buf = append(buf, `{"type":`...)
	buf = append(buf, typeJSON...)
	if len(raw) > 0 {
		buf = append(buf, `,"data":`...)
		buf = append(buf, raw...)
	}
	buf = append(buf, '}', '\n')

	conn.Write(buf)
}

// broadcastChats re-reads the local chat list cache and pushes it to every
// connected client. Display-name / pinned / muted / archived resolution is
// centralized in resolveDisplayName and the chat-settings lookup below so
// this and get_chats can't drift apart.
var (
	chatBroadcastTimer *time.Timer
)

// broadcastChats re-reads the local chat list cache and pushes it to every
// connected client. It is debounced to avoid flooding the UI during bulk syncs.
func broadcastChats() {
	wsMutex.Lock()
	defer wsMutex.Unlock()

	if chatBroadcastTimer != nil {
		chatBroadcastTimer.Stop()
	}
	chatBroadcastTimer = time.AfterFunc(200*time.Millisecond, func() {
		broadcastEvent("chats", buildChatsList())
	})
}

// buildChatsList is the shared query+formatting logic behind both
// broadcastChats (pushed to everyone) and the get_chats command (requested
// by one client). Keeping this in one place means the two can't drift.
func buildChatsList() []map[string]interface{} {
	rows, err := db.Query(`SELECT jid, name, timestamp, unread_count, pinned, muted, preview_snippet, archived, COALESCE(preview_sender, ''), COALESCE(preview_from_me, 0), COALESCE(preview_is_reaction, 0), COALESCE(preview_reaction_emoji, ''), COALESCE(is_community, 0), COALESCE(is_admin, 0), COALESCE(is_locked, 0) FROM chats ORDER BY pinned DESC, timestamp DESC, name ASC`)
	if err != nil {
		fmt.Println("Error querying chats:", err)
		return []map[string]interface{}{}
	}
	defer rows.Close()

	chats := []map[string]interface{}{}
	for rows.Next() {
		var jid string
		var name sql.NullString
		var ts int64
		var unread int
		var pinned int
		var muted int
		var preview sql.NullString
		var archived int
		var previewSender sql.NullString
		var previewFromMe int
		var previewIsReaction int
		var previewReactionEmoji string
		var isCommunity int
		var isAdmin int
		var isLocked int

		if err := rows.Scan(&jid, &name, &ts, &unread, &pinned, &muted, &preview, &archived, &previewSender, &previewFromMe, &previewIsReaction, &previewReactionEmoji, &isCommunity, &isAdmin, &isLocked); err != nil {
			continue
		}
		n := jid
		if name.Valid && name.String != "" {
			n = name.String
		}

		parsedJID, errJid := types.ParseJID(jid)
		if errJid == nil {
			resolved := resolveDisplayName(parsedJID)
			if resolved != parsedJID.String() {
				n = resolved
			} else if n == jid && parsedJID.Server != "g.us" && parsedJID.Server != "lid" {
				n = "+" + parsedJID.User
			}

			if chatSettings, errSettings := client.Store.ChatSettings.GetChatSettings(context.Background(), parsedJID); errSettings == nil && chatSettings.Found {
				if chatSettings.Archived {
					archived = 1
				}
				if chatSettings.Pinned {
					pinned = 1
				}
				if !chatSettings.MutedUntil.IsZero() && chatSettings.MutedUntil.After(time.Now()) {
					muted = 1
				}
			}
			if unread > 0 {
				var unreadMsgCount int
				errCount := db.QueryRow(`SELECT COUNT(*) FROM messages WHERE jid = ? AND from_me = 0 AND status != 'read' AND status != 'played' AND (read_at IS NULL OR read_at = 0)`, jid).Scan(&unreadMsgCount)
				if errCount == nil && unreadMsgCount == 0 {
					var totalIncoming int
					_ = db.QueryRow(`SELECT COUNT(*) FROM messages WHERE jid = ? AND from_me = 0`, jid).Scan(&totalIncoming)
					if totalIncoming > 0 {
						unread = 0
						db.Exec("UPDATE chats SET unread_count = 0 WHERE jid = ?", jid)
					}
				}
			}
		}

		p := ""
		if preview.Valid {
			p = preview.String
		}

		ps := ""
		if previewSender.Valid && previewSender.String != "" {
			if parsedSender, sErr := types.ParseJID(previewSender.String); sErr == nil {
				ps = resolveDisplayName(parsedSender)
			} else {
				ps = previewSender.String
			}
		}

		chats = append(chats, map[string]interface{}{
			"JID":            jid,
			"FullName":       n,
			"Timestamp":      ts,
			"UnreadCount":    unread,
			"Pinned":         pinned == 1,
			"Muted":          muted == 1,
			"PreviewSnippet": p,
			"Archived":       archived == 1,
			"PreviewSender":         ps,
			"PreviewFromMe":         previewFromMe == 1,
			"PreviewIsReaction":     previewIsReaction == 1,
			"PreviewReactionEmoji":  previewReactionEmoji,
			"IsCommunity":           isCommunity == 1,
			"IsAdmin":               isAdmin == 1,
			"IsLocked":              isLocked == 1,
		})
	}
	return chats
}
