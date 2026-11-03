package main

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"log"
	"math"
	"runtime/debug"
	"strings"
	"sync"
	"time"

	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waCommon"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
	"net"
)

func handleGetChats(conn net.Conn) {
	go func() {
		sendEvent(conn, "chats", buildChatsList())
	}()
}

var emptyReactions = []ReactionInfo{}

type pageCursor struct {
	Timestamp int64
	ID        string
}

func fetchMessagesPage(jid string, limit, offset int) []MessageJSON {
	return fetchMessagesPageBefore(jid, limit, offset, nil)
}

func fetchMessagesPageBefore(jid string, limit, offset int, before *pageCursor) []MessageJSON {
	return fetchMessagesPageAt(jid, limit, offset, before, nil)
}

func fetchMessagesPageAt(jid string, limit, offset int, before, after *pageCursor) []MessageJSON {
	return fetchMessagesWhere(jid, "", limit, offset, before, after)
}

// fetchMessagesWhere is fetchMessagesPageAt for only the rows matching filter,
// a fixed SQL condition (never user input) starting with AND.
func fetchMessagesWhere(jid, filter string, limit, offset int, before, after *pageCursor) []MessageJSON {
	now := time.Now().Unix()
	var favorites map[string]bool
	cols := `SELECT id, COALESCE(text, ''), timestamp, from_me, COALESCE(sender_jid, ''),
			COALESCE(status, ''), COALESCE(media_type, ''), COALESCE(media_mimetype, ''), COALESCE(media_filename, ''), COALESCE(media_path, ''), COALESCE(media_size, 0),
			COALESCE(quoted_id, ''), COALESCE(quoted_text, ''), COALESCE(quoted_sender, ''), COALESCE(is_edited, 0), COALESCE(is_deleted, 0),
			COALESCE(sent_at, 0), COALESCE(delivered_at, 0), COALESCE(read_at, 0), COALESCE(played_at, 0), COALESCE(edited_at, 0), COALESCE(starred, 0), COALESCE(album_id, ''),
			CASE WHEN media_type IN ('voice', 'audio') AND COALESCE(text, '') NOT LIKE '%(%' THEN raw
				WHEN media_type = 'sticker' THEN raw ELSE NULL END,
			COALESCE(channel_views, 0), COALESCE(channel_reactions, ''), COALESCE(channel_pinned, 0),
			COALESCE(pinned_until, 0), COALESCE(kept, 0), COALESCE(expires_at, 0)
		FROM messages WHERE jid = ?` + filter
	var rows *sql.Rows
	var queryErr error
	if after != nil {
		rows, queryErr = db.Query(cols+` AND (timestamp > ? OR (timestamp = ? AND id > ?)) ORDER BY timestamp ASC, id ASC LIMIT ?`,
			jid, after.Timestamp, after.Timestamp, after.ID, limit)
	} else if before != nil {
		rows, queryErr = db.Query(cols+` AND (timestamp < ? OR (timestamp = ? AND id < ?)) ORDER BY timestamp DESC, id DESC LIMIT ?`,
			jid, before.Timestamp, before.Timestamp, before.ID, limit)
	} else {
		rows, queryErr = db.Query(cols+` ORDER BY timestamp DESC, id DESC LIMIT ? OFFSET ?`, jid, limit, offset)
	}

	msgs := make([]MessageJSON, 0, min(max(limit, 0), 5000))
	ids := make([]string, 0, cap(msgs))
	var quotedIDs []string
	names := map[string]string{}
	nameFor := func(jidStr string) string {
		if n, ok := names[jidStr]; ok {
			return n
		}
		n := jidStr
		if parsed, err := types.ParseJID(jidStr); err == nil {
			n = resolveDisplayName(parsed)
		}
		names[jidStr] = n
		return n
	}
	if queryErr == nil {
		defer rows.Close()
		for rows.Next() {
			var id, text, senderJID, status, mediaType, mediaMimetype, mediaFilename, mediaPath string
			var quotedID, quotedText, quotedSender, albumID string
			var ts, sentAt, deliveredAt, readAt, playedAt, editedAt, mediaSize int64
			var fromMe bool
			var isEdited, isDeleted, starred int
			var rawBytes []byte
			var channelViews int
			var channelReactions string
			var channelPinned, kept bool
			var pinnedUntil, expiresAt int64
			if err := rows.Scan(&id, &text, &ts, &fromMe, &senderJID, &status, &mediaType, &mediaMimetype, &mediaFilename, &mediaPath, &mediaSize,
				&quotedID, &quotedText, &quotedSender, &isEdited, &isDeleted, &sentAt, &deliveredAt, &readAt, &playedAt, &editedAt, &starred, &albumID, &rawBytes,
				&channelViews, &channelReactions, &channelPinned, &pinnedUntil, &kept, &expiresAt); err != nil {
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
			text = renderNames(text, nameFor)

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
				senderName = nameFor(senderJID)
			}
			if quotedID != "" {
				quotedIDs = append(quotedIDs, quotedID)
			}
			stickerFavorite := false
			if mediaType == "sticker" && len(rawBytes) > 0 {
				if favorites == nil {
					favorites = favoriteStickerHashes()
				}
				var pbMsg waE2E.Message
				if proto.Unmarshal(rawBytes, &pbMsg) == nil {
					stickerFavorite = favorites[stickerHash(unwrapMessage(&pbMsg).GetStickerMessage())]
				}
			}
			pollResults := ""
			if strings.HasPrefix(text, "[Poll]") || strings.HasPrefix(text, "[Quiz]") || strings.HasPrefix(text, "[Event]") {
				pollResults = pollResultsForMessage(id)
			}
			ids = append(ids, id)
			msgs = append(msgs, MessageJSON{
				ID:            id,
				Text:          text,
				Timestamp:     ts,
				FromMe:        fromMe,
				SenderJID:     senderJID,
				SenderName:    senderName,
				Status:        status,
				MediaType:     mediaType,
				MediaMimetype: mediaMimetype,
				MediaFilename: mediaFilename,
				MediaPath:     mediaPath,
				MediaSize:     mediaSize,
				QuotedID:      quotedID,
				QuotedText:    quotedText,
				QuotedSender:  quotedSender,
				IsEdited:      isEdited == 1,
				IsDeleted:     isDeleted == 1,
				SentAt:        sentAt,
				DeliveredAt:   deliveredAt,
				ReadAt:        readAt,
				PlayedAt:      playedAt,
				EditedAt:      editedAt,
				Starred:       starred == 1,
				AlbumID:       albumID,
				PollResults:   pollResults,
				ChannelStats:  channelStatsFromColumns(channelViews, channelReactions, channelPinned),
				ChannelPinned: channelPinned,
				Pinned:        pinnedUntil > now,
				Kept:          kept,
				Expiring:      expiresAt > 0,
				StickerFav:    stickerFavorite,
				Reactions:     emptyReactions,
			})
		}
	} else {
		log.Println("Error querying messages:", queryErr)
	}

	if after == nil {
		for i, j := 0, len(msgs)-1; i < j; i, j = i+1, j-1 {
			msgs[i], msgs[j] = msgs[j], msgs[i]
		}
	}
	origins := lookupMessageOrigins(quotedIDs)
	for i := range msgs {
		msgs[i].QuotedSender = quotedSenderName(msgs[i].QuotedID, msgs[i].QuotedSender, origins, nameFor)
	}
	if len(ids) > 0 {
		reactions := getReactionsForMessageIDs(ids)
		for i := range msgs {
			if rs, ok := reactions[msgs[i].ID]; ok {
				msgs[i].Reactions = rs
			}
		}
	}
	return msgs
}

func quotedSenderName(quotedID, quotedSender string, origins map[string]messageOrigin, nameFor func(string) string) string {
	if quotedID != "" {
		if origin, ok := origins[quotedID]; ok {
			if origin.FromMe {
				return "You"
			}
			if _, err := types.ParseJID(origin.SenderJID); err == nil {
				return nameFor(origin.SenderJID)
			}
			return quotedSender
		}
	}
	if quotedSender == "me" {
		return "You"
	}
	if quotedSender != "" {
		if _, err := types.ParseJID(quotedSender); err == nil {
			return nameFor(quotedSender)
		}
	}
	return quotedSender
}

func handleGetMessages(conn net.Conn, raw json.RawMessage) {
	var data struct {
		JID             string `json:"jid"`
		Limit           int    `json:"limit"`
		Offset          int    `json:"offset"`
		BeforeID        string `json:"before_id"`
		BeforeTimestamp int64  `json:"before_timestamp"`
		AfterID         string `json:"after_id"`
		AfterTimestamp  int64  `json:"after_timestamp"`
		NoServerFetch   bool   `json:"no_server_fetch"`
		FromStart       bool   `json:"from_start"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		if data.FromStart {
			limit := data.Limit
			if limit == 0 {
				limit = 50
			}
			msgs := fetchMessagesPageAt(data.JID, limit, 0, nil, &pageCursor{Timestamp: math.MinInt64})
			older, newer := countAround(data.JID, msgs)
			log.Printf("Got the %d oldest messages for %s", len(msgs), data.JID)
			sendEvent(conn, "conversation_messages", map[string]interface{}{
				"jid":      data.JID,
				"messages": msgs,
				"isLatest": newer == 0,
				"older":    older,
				"newer":    newer,
			})
			return
		}
		if data.AfterID != "" {
			limit := data.Limit
			if limit == 0 {
				limit = 50
			}
			msgs := fetchMessagesPageAt(data.JID, limit, 0, nil, &pageCursor{Timestamp: data.AfterTimestamp, ID: data.AfterID})
			older, newer := countAround(data.JID, msgs)
			log.Printf("Got %d newer messages for %s", len(msgs), data.JID)
			sendEvent(conn, "conversation_messages", map[string]interface{}{
				"jid":       data.JID,
				"messages":  msgs,
				"loadNewer": true,
				"isLatest":  newer == 0,
				"older":     older,
				"newer":     newer,
			})
			return
		}
		var before *pageCursor
		if data.BeforeID != "" {
			before = &pageCursor{Timestamp: data.BeforeTimestamp, ID: data.BeforeID}
		}
		loadMore := data.Offset > 0 || before != nil
		if !loadMore {
			db.Exec("UPDATE chats SET unread_count = 0 WHERE jid = ?", data.JID)
			broadcastChats()
		}

		limit := data.Limit
		if limit == 0 {
			limit = 50
		} else if limit < 0 {
			limit = -1
		}
		offset := data.Offset
		if offset < 0 {
			offset = 0
		}

		log.Printf("Getting messages for %s (Limit: %d, Offset: %d)", data.JID, limit, offset)
		start := time.Now()
		msgs := fetchMessagesPageBefore(data.JID, limit, offset, before)
		log.Printf("Got %d messages for %s in %s", len(msgs), data.JID, time.Since(start).Round(time.Millisecond))
		if !loadMore && !data.NoServerFetch && isChannelJID(data.JID) {
			go refreshChannel(data.JID)
		}
		older, newer := countAround(data.JID, msgs)
		resp := map[string]interface{}{
			"jid":      data.JID,
			"messages": msgs,
			"loadMore": loadMore,
			"older":    older,
			"newer":    newer,
		}
		if !loadMore {
			resp["isLatest"] = newer == 0
		}
		sendEvent(conn, "conversation_messages", resp)
		if len(msgs) >= collectAfterMessages {
			debug.FreeOSMemory()
		}
	}()
}

func countAround(jid string, page []MessageJSON) (older, newer int) {
	if len(page) == 0 {
		return 0, 0
	}
	first, last := page[0], page[len(page)-1]
	older = rowCount(jid, `(timestamp < ? OR (timestamp = ? AND id < ?))`,
		[]interface{}{first.Timestamp, first.Timestamp, first.ID}, localDay(first.Timestamp))
	newer = rowCount(jid, `(timestamp > ? OR (timestamp = ? AND id > ?))`,
		[]interface{}{last.Timestamp, last.Timestamp, last.ID}, localDay(last.Timestamp))
	return older, newer
}

func localDay(ts int64) string {
	return time.Unix(ts, 0).Local().Format("2006-01-02")
}

func rowCount(jid, cond string, args []interface{}, skipDay string) int {
	rows, err := db.Query(`SELECT timestamp, COALESCE(from_me, 0), COALESCE(sender_jid, ''), COALESCE(album_id, ''),
			COALESCE(media_type, ''), substr(COALESCE(text, ''), 1, 7), COALESCE(is_deleted, 0)
		FROM messages WHERE jid = ? AND `+cond+` ORDER BY timestamp ASC, id ASC`, append([]interface{}{jid}, args...)...)
	if err != nil {
		return 0
	}
	defer rows.Close()
	type mediaRow struct {
		ts      int64
		fromMe  bool
		sender  string
		albumID string
	}
	count := 0
	day := ""
	var prev *mediaRow
	for rows.Next() {
		var m mediaRow
		var mediaType, prefix string
		var deleted int
		if rows.Scan(&m.ts, &m.fromMe, &m.sender, &m.albumID, &mediaType, &prefix, &deleted) != nil {
			continue
		}
		if d := localDay(m.ts); d != day {
			day = d
			prev = nil
			if d != skipDay {
				count++
			}
		}
		isMedia := deleted == 0 && (mediaType == "image" || mediaType == "video" ||
			strings.HasPrefix(prefix, "[Photo]") || strings.HasPrefix(prefix, "[Image]") || strings.HasPrefix(prefix, "[Video]"))
		if !isMedia {
			prev = nil
			count++
			continue
		}
		sameAlbum := prev != nil && prev.fromMe == m.fromMe && prev.sender == m.sender
		if sameAlbum && (prev.albumID != "" || m.albumID != "") {
			sameAlbum = prev.albumID == m.albumID
		} else if sameAlbum {
			sameAlbum = m.ts-prev.ts <= 10 && prev.ts-m.ts <= 10
		}
		if !sameAlbum {
			count++
		}
		prev = &m
	}
	return count
}

const (
	mediaFilter    = ` AND COALESCE(media_type, '') NOT IN ('voice', 'audio') AND (media_type IN ('image', 'video') OR text LIKE '[Photo]%' OR text LIKE '[Image]%' OR text LIKE '[Video]%')`
	documentFilter = ` AND (media_type = 'document' OR text LIKE '[Document]%')`
	linkFilter     = ` AND text NOT LIKE '[%' AND (text LIKE '%http://%' OR text LIKE '%https://%' OR text LIKE '%www.%')`
)

func handleGetChatMedia(conn net.Conn, raw json.RawMessage) {
	var data struct {
		JID string `json:"jid"`
	}
	if err := json.Unmarshal(raw, &data); err != nil || data.JID == "" {
		return
	}
	go func() {
		sendEvent(conn, "chat_media", map[string]interface{}{
			"jid":       data.JID,
			"media":     fetchMessagesWhere(data.JID, mediaFilter, -1, 0, nil, nil),
			"documents": fetchMessagesWhere(data.JID, documentFilter, -1, 0, nil, nil),
			"links":     fetchMessagesWhere(data.JID, linkFilter, -1, 0, nil, nil),
		})
	}()
}

const collectAfterMessages = 2000

func handleGetMessagesAround(conn net.Conn, raw json.RawMessage) {
	var data struct {
		JID       string `json:"jid"`
		MessageID string `json:"messageId"`
		Text      string `json:"text"`
		Context   int    `json:"context"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		context := data.Context
		if context <= 0 {
			context = 40
		}

		var targetTS int64
		var targetID string
		var lookupErr error
		if data.MessageID != "" {
			lookupErr = db.QueryRow(`SELECT timestamp, id FROM messages WHERE jid = ? AND id = ?`,
				data.JID, data.MessageID).Scan(&targetTS, &targetID)
		} else if data.Text != "" {
			lookupErr = db.QueryRow(`SELECT timestamp, id FROM messages WHERE jid = ? AND text LIKE ? ESCAPE '\'
				ORDER BY timestamp DESC LIMIT 1`, data.JID, "%"+escapeLike(data.Text)+"%").Scan(&targetTS, &targetID)
		} else {
			lookupErr = sql.ErrNoRows
		}

		offset := 0
		limit := context*2 + 1
		if lookupErr == nil {
			var position int
			if err := db.QueryRow(`SELECT COUNT(*) FROM messages WHERE jid = ? AND
					(timestamp > ? OR (timestamp = ? AND id > ?))`,
				data.JID, targetTS, targetTS, targetID).Scan(&position); err == nil {
				offset = position - context
				if offset < 0 {
					offset = 0
				}
			}
		} else {
			log.Printf("get_messages_around: target not found locally for %s, falling back to latest page", data.JID)
		}

		msgs := fetchMessagesPage(data.JID, limit, offset)
		older, newer := countAround(data.JID, msgs)
		log.Printf("Got %d messages around target for %s", len(msgs), data.JID)
		sendEvent(conn, "conversation_messages", map[string]interface{}{
			"jid":      data.JID,
			"messages": msgs,
			"loadMore": false,
			"isLatest": offset == 0,
			"older":    older,
			"newer":    newer,
			// Lets the UI stay where it is when jumping to a quoted message
			// that was never stored.
			"targetFound": lookupErr == nil,
		})
	}()
}

func escapeLike(s string) string {
	return strings.NewReplacer(`\`, `\\`, `%`, `\%`, `_`, `\_`).Replace(s)
}

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
						"text":       renderNames(text, nil),
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

func handleSearchMessages(conn net.Conn, raw json.RawMessage) {
	var data SearchMessagesData
	if err := json.Unmarshal(raw, &data); err != nil || data.Query == "" {
		return
	}
	go func() {
		like := "%" + escapeLike(data.Query) + "%"
		var rows *sql.Rows
		var err error
		if data.JID != "" {
			rows, err = db.Query(`SELECT id, jid, COALESCE(text,''), timestamp, from_me, COALESCE(sender_jid,''),
				COALESCE(status,''), COALESCE(media_type,''), COALESCE(media_mimetype,''), COALESCE(media_filename,''), COALESCE(media_path,''),
				COALESCE(quoted_id,''), COALESCE(quoted_text,''), COALESCE(quoted_sender,''), COALESCE(is_edited,0), COALESCE(is_deleted,0),
				COALESCE(sent_at,0), COALESCE(delivered_at,0), COALESCE(read_at,0), COALESCE(played_at,0), COALESCE(edited_at,0), COALESCE(starred,0) FROM messages
				WHERE jid = ? AND text LIKE ? ESCAPE '\' AND COALESCE(is_deleted, 0) = 0 ORDER BY timestamp ASC LIMIT 200`, data.JID, like)
		} else {
			rows, err = db.Query(`SELECT id, jid, COALESCE(text,''), timestamp, from_me, COALESCE(sender_jid,''),
				COALESCE(status,''), COALESCE(media_type,''), COALESCE(media_mimetype,''), COALESCE(media_filename,''), COALESCE(media_path,''),
				COALESCE(quoted_id,''), COALESCE(quoted_text,''), COALESCE(quoted_sender,''), COALESCE(is_edited,0), COALESCE(is_deleted,0),
				COALESCE(sent_at,0), COALESCE(delivered_at,0), COALESCE(read_at,0), COALESCE(played_at,0), COALESCE(edited_at,0), COALESCE(starred,0) FROM messages
				WHERE text LIKE ? ESCAPE '\' AND COALESCE(is_deleted, 0) = 0 ORDER BY timestamp ASC LIMIT 200`, like)
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
						"text":          renderNames(text, nil),
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
			"jid":     data.JID,
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
		// Real messages only: a synthesized group-event line or call-history
		// row has an ID WhatsApp never issued, and patches naming one as the
		// chat's last message don't match what the phone has.
		`SELECT id, timestamp, from_me, COALESCE(NULLIF(wire_sender, ''), sender_jid, '') FROM messages
			WHERE jid = ? AND COALESCE(media_type, '') NOT IN ('system', 'call') AND id NOT LIKE 'system\_%' ESCAPE '\' AND id NOT LIKE 'call:%'
			ORDER BY timestamp DESC LIMIT 1`,
		jid_str,
	).Scan(&msg_id, &msg_ts, &from_me, &sender_jid)
	if err != nil {
		return time.Time{}, nil
	}
	remote := jid_str
	if parsed, perr := types.ParseJID(jid_str); perr == nil {
		remote = syncdChatJID(parsed).String()
	}
	key := &waCommon.MessageKey{
		RemoteJID: proto.String(remote),
		ID:        proto.String(msg_id),
		FromMe:    proto.Bool(from_me),
	}
	if sender_jid != "" && sender_jid != jid_str {
		key.Participant = proto.String(sender_jid)
	}
	return time.Unix(msg_ts, 0), key
}

// Serializes every SendAppState call process-wide. whatsmeow builds each
// patch against the collection's current version fetched from local store;
// firing several patches concurrently (e.g. bulk "delete for me" across a
// multi-message selection) races them against the same base version, so the
// server accepts only the first and rejects the rest with a version/hash
// mismatch that surfaces here as a silent per-patch failure - the affected
// rows just never disappear, with no error visible to the user. Holding this
// lock for the whole send-and-retry sequence makes concurrent callers queue
// up and each apply cleanly against the version the previous one left behind.
var appStateSendMu sync.Mutex

func sendAppStateWithRetry(ctx context.Context, patch appstate.PatchInfo) error {
	appStateSendMu.Lock()
	defer appStateSendMu.Unlock()

	err := wa().SendAppState(ctx, patch)
	if err == nil {
		return nil
	}
	log.Printf("SendAppState %s failed, attempting resync: %v", patch.Type, err)

	appStateResyncMu.Lock()
	lastAppStateResync[patch.Type] = time.Now()
	appStateResyncMu.Unlock()

	if sync_err := wa().FetchAppState(ctx, patch.Type, false, false); sync_err != nil {
		log.Printf("Resync of %s failed: %v", patch.Type, sync_err)
		return fmt.Errorf("app state sync failed for %s: %w", patch.Type, sync_err)
	}
	if retry_err := wa().SendAppState(ctx, patch); retry_err != nil {
		log.Printf("SendAppState %s retry also failed: %v", patch.Type, retry_err)
		return fmt.Errorf("app state retry failed for %s: %w", patch.Type, retry_err)
	}
	return nil
}

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
		// WA Web sends an unarchive together with every pin
		// (PinChatSync.getMutationsForPin), since a chat can't be both.
		patch := appstate.BuildPin(syncdChatJID(jid), data.Value)
		if data.Value {
			lastTS, lastKey := getLastMessageKeyForChat(data.JID)
			patch.Mutations = append(patch.Mutations, appstate.BuildArchive(syncdChatJID(jid), false, lastTS, lastKey).Mutations...)
			patch = withChatRange(patch, data.JID)
		}
		if err := sendAppStateWithRetry(context.Background(), patch); err != nil {
			broadcastChats()
			return
		}
		applyPinChange(data.JID, data.Value, time.Now().Unix())
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
		patch := withChatRange(appstate.BuildArchive(syncdChatJID(jid), data.Value, last_ts, last_key), data.JID)
		if err := sendAppStateWithRetry(context.Background(), patch); err != nil {
			broadcastChats()
			return
		}
		noteArchiveAction(data.JID, data.Value, patch.Mutations[0].Value.GetArchiveChatAction().GetMessageRange(), time.Now().Unix())
		db.Exec("UPDATE chats SET archived = ? WHERE jid = ?", boolToInt(data.Value), data.JID)
		if data.Value {
			// BuildArchive also unpins, as WA Web does.
			applyPinChange(data.JID, false, time.Now().Unix())
		}
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
	if isChannelJID(data.JID) {
		go handleChannelMute(data.JID, data.Muted)
		return
	}
	go func() {
		duration := time.Duration(data.DurationSeconds) * time.Second
		if err := sendAppStateWithRetry(context.Background(), appstate.BuildMute(syncdChatJID(jid), data.Muted, duration)); err != nil {
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
	if data.Value {
		// WA Web's Mark as read (WAWebUpdateUnreadChatAction.markUnread with
		// unread false -> sendConversationSeen) sends read receipts for the
		// unread messages; the markChatAsRead mutation is only sent for a
		// chat that was just marked unread with nothing really unread. The
		// mutation alone isn't enough: another device only applies it fully
		// when its range covers every message it has, otherwise it just
		// clears a marked-unread flag and leaves the chat unread.
		// handleMarkRead does both, the same way opening the chat does.
		payload, _ := json.Marshal(map[string]any{"jid": data.JID, "latest": true})
		handleMarkRead(payload)
		db.Exec("UPDATE chats SET unread_count = 0, marked_unread = 0 WHERE jid = ?", data.JID)
		broadcastChats()
		return
	}
	db.Exec("UPDATE chats SET unread_count = 1, marked_unread = 1 WHERE jid = ?", data.JID)
	broadcastChats()
	go func() {
		last_ts, last_key := getLastMessageKeyForChat(data.JID)
		if err := sendAppStateWithRetry(context.Background(), withChatRange(appstate.BuildMarkChatAsRead(syncdChatJID(jid), false, last_ts, last_key), data.JID)); err != nil {
			log.Printf("Marking %s unread on other devices failed: %v", data.JID, err)
		}
	}()
}

// handleDeleteChat previously only deleted the chat locally - it never told
// WhatsApp at all, so a chat deleted from this app stayed put on the phone
// and every other linked device. Sends the same "deleteChat" app-state
// mutation the phone itself would, same as handleSetChatLocked/
// handleSetChatRead already do for their own actions.
func handleDeleteChat(raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		last_ts, last_key := getLastMessageKeyForChat(data.JID)
		if err := sendAppStateWithRetry(context.Background(), withChatRange(appstate.BuildDeleteChat(syncdChatJID(jid), last_ts, last_key, false), data.JID)); err != nil {
			broadcastEvent("error", map[string]string{"message": "Failed to delete chat: " + err.Error()})
			return
		}
		purgeMessages("jid = ?", data.JID)
		db.Exec("DELETE FROM chats WHERE jid = ?", data.JID)
		broadcastChats()
	}()
}

func handleClearChat(raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		last_ts, last_key := getLastMessageKeyForChat(data.JID)
		if err := sendAppStateWithRetry(context.Background(), withChatRange(appstate.BuildClearChat(syncdChatJID(jid), last_ts, last_key, false, false), data.JID)); err != nil {
			broadcastEvent("error", map[string]string{"message": "Failed to clear chat: " + err.Error()})
			return
		}
		purgeMessages("jid = ?", data.JID)
		broadcastChats()
		broadcastEvent("chat_cleared", map[string]interface{}{"jid": data.JID})
	}()
}

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
	if chatJID.Server == types.NewsletterServer {
		go loadOlderChannelPosts(data.JID)
		return
	}
	requestOlderHistory(chatJID, data.JID, data.Count, true)
}

func requestOlderHistory(chatJID types.JID, jid string, count int, autoRetry bool) {
	data := struct {
		JID   string
		Count int
	}{jid, count}

	// The phone answers with what comes before the oldest message it's
	// told about, so that has to be a real WhatsApp message it knows:
	// the chat's oldest stored one, skipping the group-event and
	// call-history rows whatSnap makes up itself. Chosen here rather than
	// trusting the UI's pick - it used to send its first row, which is
	// always a date divider with no ID, so the lookup failed and the
	// request was silently never sent at all.
	oldest, err := oldestHistoryAnchor(data.JID, nil)
	if err != nil {
		requestHistoryForEmptyChat(chatJID, data.JID, count)
		return
	}

	// The UI asks at most once per time a chat is opened. This is the
	// backstop under that: the same request (same chat, same anchor) is
	// refused if it was sent moments ago, so a UI bug or a held-down key
	// can never turn into a stream of identical requests to the phone.
	var lastAnchor, lastOldest string
	var lastAt int64
	haveLast := db.QueryRow(`SELECT anchor_id, COALESCE(oldest_id, ''), requested_at FROM history_requests WHERE jid = ?`,
		data.JID).Scan(&lastAnchor, &lastOldest, &lastAt) == nil
	if haveLast && lastOldest == oldest.id && time.Since(time.Unix(lastAt, 0)) < historyRequestRepeatAfter {
		log.Printf("Not re-requesting history for %s: already asked before %s at %s", data.JID, lastAnchor, time.Unix(lastAt, 0).Format(time.RFC3339))
		broadcastEvent("no_more_history", map[string]string{"jid": data.JID})
		return
	}
	anchor := oldest
	if haveLast && lastOldest == oldest.id {
		if alt, altErr := oldestHistoryAnchor(data.JID, []string{oldest.id, lastAnchor}); altErr == nil {
			log.Printf("History request for %s before %s got nothing; trying %s instead", data.JID, lastAnchor, alt.id)
			anchor = alt
		}
	}
	msgID, senderJID, fromMe, ts := anchor.id, anchor.sender, anchor.fromMe, anchor.ts

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
		ID:        msgID,
		Timestamp: time.Unix(ts, 0),
	}

	if _, err := db.Exec(`INSERT INTO history_requests (jid, anchor_id, oldest_id, requested_at) VALUES (?, ?, ?, ?)
		ON CONFLICT(jid) DO UPDATE SET anchor_id = excluded.anchor_id, oldest_id = excluded.oldest_id, requested_at = excluded.requested_at`,
		data.JID, msgID, oldest.id, time.Now().Unix()); err != nil {
		log.Printf("history sync request for %s: couldn't record it, not sending: %v", data.JID, err)
		return
	}

	if count <= 0 || count > 50 {
		count = 50
	}
	log.Printf("Requesting older history for %s before %s (count %d)", data.JID, msgID, count)
	req := wa().BuildHistorySyncRequest(msgInfo, count)
	go func() {
		answered := make(chan struct{}, 1)
		onDemandWaiters.Store(data.JID, answered)
		defer onDemandWaiters.CompareAndDelete(data.JID, answered)
		historyRequestMu.Lock()
		if wait := historyRequestMinGap - time.Since(lastHistoryRequest); wait > 0 {
			time.Sleep(wait)
		}
		lastHistoryRequest = time.Now()
		_, sendErr := wa().SendPeerMessage(context.Background(), req)
		historyRequestMu.Unlock()
		if sendErr != nil {
			log.Printf("history sync request for %s failed: %v", data.JID, sendErr)
			return
		}
		if autoRetry {
			select {
			case <-answered:
			case <-time.After(onDemandRetryAfter):
				log.Printf("No answer from the phone for %s before %s after %s; retrying with another anchor", data.JID, msgID, onDemandRetryAfter)
				requestOlderHistory(chatJID, data.JID, count, false)
			}
			return
		}
		select {
		case <-answered:
			if now, err := oldestHistoryAnchor(data.JID, nil); err != nil || now.id != oldest.id {
				return
			}
			log.Printf("The phone's answer for %s held nothing older than %s; this is the start of the chat", data.JID, oldest.id)
		case <-time.After(onDemandStartAfter):
			log.Printf("No answer from the phone for %s after a retry either; this is the start of the chat", data.JID)
		}
		broadcastEvent("history_start_reached", map[string]string{"jid": data.JID})
	}()
}

var emptyChatHistoryAsked sync.Map

func requestHistoryForEmptyChat(chatJID types.JID, jid string, count int) {
	if _, asked := emptyChatHistoryAsked.LoadOrStore(jid, true); asked {
		broadcastEvent("history_start_reached", map[string]string{"jid": jid})
		return
	}
	if count <= 0 || count > 50 {
		count = 50
	}
	req := wa().BuildHistorySyncRequest(&types.MessageInfo{
		MessageSource: types.MessageSource{Chat: chatJID},
		Timestamp:     time.Now(),
	}, count)
	onDemand := req.GetProtocolMessage().GetPeerDataOperationRequestMessage().GetHistorySyncOnDemandRequest()
	onDemand.OldestMsgID, onDemand.OldestMsgFromMe = nil, nil
	log.Printf("Requesting the latest history for %s, which has no messages yet (count %d)", jid, count)
	go func() {
		answered := make(chan struct{}, 1)
		onDemandWaiters.Store(jid, answered)
		defer onDemandWaiters.CompareAndDelete(jid, answered)
		historyRequestMu.Lock()
		if wait := historyRequestMinGap - time.Since(lastHistoryRequest); wait > 0 {
			time.Sleep(wait)
		}
		lastHistoryRequest = time.Now()
		_, sendErr := wa().SendPeerMessage(context.Background(), req)
		historyRequestMu.Unlock()
		if sendErr != nil {
			log.Printf("history sync request for %s failed: %v", jid, sendErr)
			emptyChatHistoryAsked.Delete(jid)
			broadcastEvent("no_more_history", map[string]string{"jid": jid})
			return
		}
		select {
		case <-answered:
		case <-time.After(onDemandRetryAfter):
			log.Printf("No answer from the phone for %s, which has no messages; leaving it empty", jid)
			broadcastEvent("no_more_history", map[string]string{"jid": jid})
		}
	}()
}

var onDemandWaiters sync.Map

func noteOnDemandAnswer(jid string) {
	if ch, ok := onDemandWaiters.Load(jid); ok {
		select {
		case ch.(chan struct{}) <- struct{}{}:
		default:
		}
	}
}

const (
	historyRequestRepeatAfter = 10 * time.Second
	historyRequestMinGap      = 2 * time.Second
	onDemandRetryAfter        = 12 * time.Second
	onDemandStartAfter        = 10 * time.Second
)

var (
	historyRequestMu   sync.Mutex
	lastHistoryRequest time.Time
)

type historyAnchor struct {
	id, sender string
	fromMe     bool
	ts         int64
}

func oldestHistoryAnchor(jid string, exclude []string) (historyAnchor, error) {
	var a historyAnchor
	if len(exclude) == 0 {
		err := db.QueryRow(`SELECT id, COALESCE(sender_jid, ''), from_me, timestamp FROM messages
			WHERE jid = ? AND COALESCE(media_type, '') NOT IN ('system', 'call')
			ORDER BY timestamp ASC, id ASC LIMIT 1`, jid).Scan(&a.id, &a.sender, &a.fromMe, &a.ts)
		return a, err
	}
	args := []interface{}{jid}
	notIn := strings.TrimSuffix(strings.Repeat("?, ", len(exclude)), ", ")
	for _, id := range exclude {
		args = append(args, id)
	}
	err := db.QueryRow(`SELECT id, COALESCE(sender_jid, ''), from_me, timestamp FROM messages
		WHERE jid = ? AND COALESCE(media_type, '') NOT IN ('system', 'call') AND id NOT IN (`+notIn+`)
		ORDER BY (COALESCE(is_deleted, 0) = 0 AND COALESCE(media_type, '') = '' AND COALESCE(album_id, '') = '') DESC,
			timestamp ASC, id ASC LIMIT 1`, args...).Scan(&a.id, &a.sender, &a.fromMe, &a.ts)
	return a, err
}
