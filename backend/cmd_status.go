package main

import (
	"context"
	"encoding/json"
	"net"
	"sort"
	"strings"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
)

// whatsappTeal is the ARGB background official WhatsApp defaults a plain-text
// status to when the poster doesn't pick a color.
const whatsappTeal = 0xFF075E54

type PostStatusData struct {
	Text string `json:"text"`
}

type MarkStatusViewedData struct {
	ID        string `json:"id"`
	PosterJID string `json:"poster_jid"`
}

// handleGetStatuses returns every still-live (posted within the last 24h)
// Status/Stories update, grouped by poster: "My Status" first, then everyone
// else newest-post-first. Statuses live in the same `messages` table as
// regular chat messages, under the reserved jid 'status@broadcast' (see
// handleIncomingStatus) - that's what lets media download/open/save reuse
// their existing message-ID-keyed code paths unchanged.
func handleGetStatuses(conn net.Conn) {
	go func() {
		cutoff := time.Now().Add(-24 * time.Hour).Unix()
		db.Exec("DELETE FROM messages WHERE jid = 'status@broadcast' AND timestamp <= ?", cutoff)

		rows, err := db.Query(`SELECT id, COALESCE(sender_jid, ''), from_me, COALESCE(text, ''),
				COALESCE(media_type, ''), COALESCE(media_mimetype, ''), COALESCE(media_filename, ''), COALESCE(media_path, ''),
				timestamp, COALESCE(viewed, 0)
			FROM messages WHERE jid = 'status@broadcast' ORDER BY timestamp ASC`)

		type groupMeta struct {
			posterJID, posterName string
			fromMe                bool
			items                 []map[string]interface{}
		}
		order := []string{}
		groups := map[string]*groupMeta{}

		if err == nil {
			defer rows.Close()
			for rows.Next() {
				var id, senderJID, text, mediaType, mediaMimetype, mediaFilename, mediaPath string
				var ts int64
				var fromMe bool
				var viewed int
				if scanErr := rows.Scan(&id, &senderJID, &fromMe, &text, &mediaType, &mediaMimetype, &mediaFilename, &mediaPath, &ts, &viewed); scanErr != nil {
					continue
				}
				key := senderJID
				if fromMe {
					key = "me"
				}
				g, ok := groups[key]
				if !ok {
					name := "You"
					if !fromMe {
						if parsed, perr := types.ParseJID(senderJID); perr == nil {
							name = resolveDisplayName(parsed)
						} else {
							name = senderJID
						}
					}
					g = &groupMeta{posterJID: senderJID, posterName: name, fromMe: fromMe}
					groups[key] = g
					order = append(order, key)
				}
				// Your own posts don't carry a meaningful "have I seen this"
				// state - always report them viewed so they never show as new.
				g.items = append(g.items, map[string]interface{}{
					"id": id, "text": text, "mediaType": mediaType, "mediaMimetype": mediaMimetype,
					"mediaFilename": mediaFilename, "mediaPath": mediaPath, "timestamp": ts, "viewed": viewed == 1 || fromMe,
				})
			}
		}

		result := make([]map[string]interface{}, 0, len(order))
		for _, key := range order {
			g := groups[key]
			allViewed := true
			for _, it := range g.items {
				if !it["viewed"].(bool) {
					allViewed = false
					break
				}
			}
			result = append(result, map[string]interface{}{
				"posterJid": g.posterJID, "posterName": g.posterName, "fromMe": g.fromMe,
				"allViewed": allViewed, "statuses": g.items,
			})
		}
		sort.SliceStable(result, func(i, j int) bool {
			fi, fj := result[i]["fromMe"].(bool), result[j]["fromMe"].(bool)
			if fi != fj {
				return fi
			}
			si, sj := result[i]["statuses"].([]map[string]interface{}), result[j]["statuses"].([]map[string]interface{})
			return si[len(si)-1]["timestamp"].(int64) > sj[len(sj)-1]["timestamp"].(int64)
		})

		sendEvent(conn, "statuses", map[string]interface{}{"groups": result})
	}()
}

func handlePostStatus(raw json.RawMessage) {
	var data PostStatusData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	text := strings.TrimSpace(data.Text)
	if text == "" {
		return
	}
	go func() {
		msg := &waE2E.Message{
			ExtendedTextMessage: &waE2E.ExtendedTextMessage{
				Text:           proto.String(text),
				BackgroundArgb: proto.Uint32(whatsappTeal),
			},
		}
		resp, err := client.SendMessage(context.Background(), types.StatusBroadcastJID, msg, whatsmeow.SendRequestExtra{ID: whatsmeow.GenerateMessageID()})
		if err != nil {
			broadcastEvent("error", map[string]string{"message": "Failed to post status: " + err.Error()})
			return
		}
		ownJID := ""
		if client.Store.ID != nil {
			ownJID = client.Store.ID.ToNonAD().String()
		}
		rawBytes, _ := proto.Marshal(msg)
		saveMessage(StoredMessage{
			ID: resp.ID, JID: "status@broadcast", SenderJID: ownJID,
			FromMe: true, Text: text, Timestamp: resp.Timestamp.Unix(), Raw: rawBytes, Status: "sent",
		})
		broadcastEvent("status_update", map[string]interface{}{
			"id": resp.ID, "posterJid": ownJID, "fromMe": true, "text": text,
			"mediaType": "", "timestamp": resp.Timestamp.Unix(),
		})
	}()
}

func handleMarkStatusViewed(raw json.RawMessage) {
	var data MarkStatusViewedData
	if err := json.Unmarshal(raw, &data); err != nil || data.ID == "" || data.PosterJID == "" || data.PosterJID == "me" {
		return
	}
	db.Exec("UPDATE messages SET viewed = 1 WHERE id = ? AND jid = 'status@broadcast'", data.ID)
	go func() {
		poster, err := types.ParseJID(data.PosterJID)
		if err != nil || poster.IsEmpty() {
			return
		}
		if client.Store.ID != nil && poster.User == client.Store.ID.User {
			return
		}
		_ = client.MarkRead(context.Background(), []types.MessageID{types.MessageID(data.ID)}, time.Now(), types.StatusBroadcastJID, poster, types.ReceiptTypeRead)
	}()
}
