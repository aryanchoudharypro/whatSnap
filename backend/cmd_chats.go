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
	"time"

	"net"

	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waCommon"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
)

func (acc *Account) handleGetChats(conn net.Conn) {
	go func() {
		acc.sendEvent(conn, "chats", acc.buildChatsList())
	}()
}

var emptyReactions = []ReactionInfo{}

type pageCursor struct {
	Timestamp int64
	ID        string
}

func (acc *Account) fetchMessagesPage(jid string, limit, offset int) []MessageJSON {
	return acc.fetchMessagesPageBefore(jid, limit, offset, nil)
}

func (acc *Account) fetchMessagesPageBefore(jid string, limit, offset int, before *pageCursor) []MessageJSON {
	return acc.fetchMessagesPageAt(jid, limit, offset, before, nil)
}

func (acc *Account) fetchMessagesPageAt(jid string, limit, offset int, before, after *pageCursor) []MessageJSON {
	return acc.fetchMessagesWhere(jid, "", limit, offset, before, after)
}

// fetchMessagesWhere is fetchMessagesPageAt for only the rows matching filter,
// a fixed SQL condition (never user input) starting with AND.
func (acc *Account) fetchMessagesWhere(jid, filter string, limit, offset int, before, after *pageCursor) []MessageJSON {
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
		rows, queryErr = acc.db.Query(cols+` AND (timestamp > ? OR (timestamp = ? AND id > ?)) ORDER BY timestamp ASC, id ASC LIMIT ?`,
			jid, after.Timestamp, after.Timestamp, after.ID, limit)
	} else if before != nil {
		rows, queryErr = acc.db.Query(cols+` AND (timestamp < ? OR (timestamp = ? AND id < ?)) ORDER BY timestamp DESC, id DESC LIMIT ?`,
			jid, before.Timestamp, before.Timestamp, before.ID, limit)
	} else {
		rows, queryErr = acc.db.Query(cols+` ORDER BY timestamp DESC, id DESC LIMIT ? OFFSET ?`, jid, limit, offset)
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
			n = acc.hidePhoneInGroup(jid, acc.resolveDisplayName(parsed))
		}
		names[jidStr] = n
		return n
	}
	// The rows are read in full and closed before anything else touches the
	// database: names, quotes and polls below run their own queries, and
	// doing that with the rows still open held one pooled connection while
	// waiting for another (and for whatsmeow's LID/contact cache locks, which
	// are held across their own queries) - enough of those at once, as after
	// a wake, and every database user hung for good.
	type messageRow struct {
		id, text, senderJID, status, mediaType, mediaMimetype, mediaFilename, mediaPath string
		quotedID, quotedText, quotedSender, albumID                                     string
		ts, sentAt, deliveredAt, readAt, playedAt, editedAt, mediaSize                  int64
		fromMe                                                                          bool
		isEdited, isDeleted, starred                                                    int
		rawBytes                                                                        []byte
		channelViews                                                                    int
		channelReactions                                                                string
		channelPinned, kept                                                             bool
		pinnedUntil, expiresAt                                                          int64
	}
	var scanned []messageRow
	if queryErr == nil {
		for rows.Next() {
			var r messageRow
			if err := rows.Scan(&r.id, &r.text, &r.ts, &r.fromMe, &r.senderJID, &r.status, &r.mediaType, &r.mediaMimetype, &r.mediaFilename, &r.mediaPath, &r.mediaSize,
				&r.quotedID, &r.quotedText, &r.quotedSender, &r.isEdited, &r.isDeleted, &r.sentAt, &r.deliveredAt, &r.readAt, &r.playedAt, &r.editedAt, &r.starred, &r.albumID, &r.rawBytes,
				&r.channelViews, &r.channelReactions, &r.channelPinned, &r.pinnedUntil, &r.kept, &r.expiresAt); err != nil {
				log.Println("Error scanning message row:", err)
				continue
			}
			scanned = append(scanned, r)
		}
		rows.Close()
	} else {
		log.Println("Error querying messages:", queryErr)
	}
	for _, r := range scanned {
		{
			id, text, senderJID, status, mediaType, mediaMimetype, mediaFilename, mediaPath := r.id, r.text, r.senderJID, r.status, r.mediaType, r.mediaMimetype, r.mediaFilename, r.mediaPath
			quotedID, quotedText, quotedSender, albumID := r.quotedID, r.quotedText, r.quotedSender, r.albumID
			ts, sentAt, deliveredAt, readAt, playedAt, editedAt, mediaSize := r.ts, r.sentAt, r.deliveredAt, r.readAt, r.playedAt, r.editedAt, r.mediaSize
			fromMe, isEdited, isDeleted, starred, rawBytes := r.fromMe, r.isEdited, r.isDeleted, r.starred, r.rawBytes
			channelViews, channelReactions, channelPinned, kept := r.channelViews, r.channelReactions, r.channelPinned, r.kept
			pinnedUntil, expiresAt := r.pinnedUntil, r.expiresAt
			if isDeleted == 1 {
				if fromMe {
					text = "[You deleted this message]"
				} else {
					text = "[This message was deleted]"
				}
			}
			text = acc.renderNames(text, nameFor)

			if mediaType == "voice" || mediaType == "audio" || strings.HasPrefix(text, "[Voice Message]") || strings.HasPrefix(text, "[Audio]") {
				if !strings.Contains(text, "(") && len(rawBytes) > 0 {
					var pbMsg waE2E.Message
					if err := proto.Unmarshal(rawBytes, &pbMsg); err == nil {
						extracted := acc.extractText(&pbMsg)
						if extracted != "" {
							text = extracted
						}
					}
				}
			}

			if strings.HasPrefix(quotedText, "[Voice Message]") || strings.HasPrefix(quotedText, "[Audio]") {
				if !strings.Contains(quotedText, "(") && quotedID != "" {
					if _, _, qPreview, qErr := acc.getMessageForQuote(quotedID); qErr == nil && qPreview != "" {
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
					favorites = acc.favoriteStickerHashes()
				}
				var pbMsg waE2E.Message
				if proto.Unmarshal(rawBytes, &pbMsg) == nil {
					stickerFavorite = favorites[stickerHash(unwrapMessage(&pbMsg).GetStickerMessage())]
				}
			}
			pollResults := ""
			if strings.HasPrefix(text, "[Poll]") || strings.HasPrefix(text, "[Quiz]") || strings.HasPrefix(text, "[Event]") {
				pollResults = acc.pollResultsForMessage(id)
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
	}

	if after == nil {
		for i, j := 0, len(msgs)-1; i < j; i, j = i+1, j-1 {
			msgs[i], msgs[j] = msgs[j], msgs[i]
		}
	}
	origins := acc.lookupMessageOrigins(quotedIDs)
	for i := range msgs {
		msgs[i].QuotedSender = quotedSenderName(msgs[i].QuotedID, msgs[i].QuotedSender, origins, nameFor)
	}
	if len(ids) > 0 {
		reactions := acc.getReactionsForMessageIDs(ids)
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

func (acc *Account) handleGetMessages(conn net.Conn, raw json.RawMessage) {
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
			msgs := acc.fetchMessagesPageAt(data.JID, limit, 0, nil, &pageCursor{Timestamp: math.MinInt64})
			older, newer := acc.countAround(data.JID, msgs)
			log.Printf("Got the %d oldest messages for %s", len(msgs), data.JID)
			acc.sendEvent(conn, "conversation_messages", map[string]interface{}{
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
			msgs := acc.fetchMessagesPageAt(data.JID, limit, 0, nil, &pageCursor{Timestamp: data.AfterTimestamp, ID: data.AfterID})
			older, newer := acc.countAround(data.JID, msgs)
			log.Printf("Got %d newer messages for %s", len(msgs), data.JID)
			acc.sendEvent(conn, "conversation_messages", map[string]interface{}{
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
			acc.db.Exec("UPDATE chats SET unread_count = 0 WHERE jid = ?", data.JID)
			acc.broadcastChats()
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
		msgs := acc.fetchMessagesPageBefore(data.JID, limit, offset, before)
		log.Printf("Got %d messages for %s in %s", len(msgs), data.JID, time.Since(start).Round(time.Millisecond))
		if !loadMore && !data.NoServerFetch && isChannelJID(data.JID) {
			go acc.refreshChannel(data.JID)
		}
		older, newer := acc.countAround(data.JID, msgs)
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
		acc.sendEvent(conn, "conversation_messages", resp)
		if len(msgs) >= collectAfterMessages {
			debug.FreeOSMemory()
		}
	}()
}

func (acc *Account) countAround(jid string, page []MessageJSON) (older, newer int) {
	if len(page) == 0 {
		return 0, 0
	}
	first, last := page[0], page[len(page)-1]
	older = acc.rowCount(jid, `(timestamp < ? OR (timestamp = ? AND id < ?))`,
		[]interface{}{first.Timestamp, first.Timestamp, first.ID}, localDay(first.Timestamp))
	newer = acc.rowCount(jid, `(timestamp > ? OR (timestamp = ? AND id > ?))`,
		[]interface{}{last.Timestamp, last.Timestamp, last.ID}, localDay(last.Timestamp))
	return older, newer
}

func localDay(ts int64) string {
	return time.Unix(ts, 0).Local().Format("2006-01-02")
}

func (acc *Account) rowCount(jid, cond string, args []interface{}, skipDay string) int {
	rows, err := acc.db.Query(`SELECT timestamp, COALESCE(from_me, 0), COALESCE(sender_jid, ''), COALESCE(album_id, ''),
			COALESCE(media_type, ''), substr(COALESCE(text, ''), 1, 7), COALESCE(is_deleted, 0),
			COALESCE(length(text), 0), COALESCE(quoted_id, '')
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
		legacy  bool
	}
	count := 0
	day := ""
	var prev *mediaRow
	// runLen counts the captionless photos in a row that WA Web shows as one
	// album once there are four of them (see whatsnap::cluster_legacy_albums).
	var run *mediaRow
	runLen := 0
	flushRun := func() {
		if runLen >= 4 {
			count += (runLen + 101) / 102
		} else {
			count += runLen
		}
		run, runLen = nil, 0
	}
	for rows.Next() {
		var m mediaRow
		var mediaType, prefix, quotedID string
		var deleted, textLen int
		if rows.Scan(&m.ts, &m.fromMe, &m.sender, &m.albumID, &mediaType, &prefix, &deleted, &textLen, &quotedID) != nil {
			continue
		}
		if d := localDay(m.ts); d != day {
			flushRun()
			day = d
			prev = nil
			if d != skipDay {
				count++
			}
		}
		isMedia := deleted == 0 && (mediaType == "image" || mediaType == "video" ||
			strings.HasPrefix(prefix, "[Photo]") || strings.HasPrefix(prefix, "[Image]") || strings.HasPrefix(prefix, "[Video]"))
		if !isMedia {
			flushRun()
			prev = nil
			count++
			continue
		}
		m.legacy = m.albumID == "" && quotedID == "" && textLen == 7 &&
			(prefix == "[Photo]" || prefix == "[Image]" || prefix == "[Video]")
		if m.legacy {
			if run != nil && !(run.fromMe == m.fromMe && run.sender == m.sender && m.ts-run.ts >= 0 && m.ts-run.ts <= 600) {
				flushRun()
			}
			run = &m
			runLen++
			prev = nil
			continue
		}
		flushRun()
		sameAlbum := prev != nil && prev.fromMe == m.fromMe && prev.sender == m.sender && m.albumID != "" && prev.albumID == m.albumID
		if !sameAlbum {
			count++
		}
		prev = &m
	}
	flushRun()
	return count
}

const (
	mediaFilter    = ` AND COALESCE(media_type, '') NOT IN ('voice', 'audio') AND (media_type IN ('image', 'video') OR text LIKE '[Photo]%' OR text LIKE '[Image]%' OR text LIKE '[Video]%')`
	documentFilter = ` AND (media_type = 'document' OR text LIKE '[Document]%')`
	linkFilter     = ` AND text NOT LIKE '[%' AND (text LIKE '%http://%' OR text LIKE '%https://%' OR text LIKE '%www.%')`
)

func (acc *Account) handleGetChatMedia(conn net.Conn, raw json.RawMessage) {
	var data struct {
		JID string `json:"jid"`
	}
	if err := json.Unmarshal(raw, &data); err != nil || data.JID == "" {
		return
	}
	go func() {
		acc.sendEvent(conn, "chat_media", map[string]interface{}{
			"jid":       data.JID,
			"media":     acc.fetchMessagesWhere(data.JID, mediaFilter, -1, 0, nil, nil),
			"documents": acc.fetchMessagesWhere(data.JID, documentFilter, -1, 0, nil, nil),
			"links":     acc.fetchMessagesWhere(data.JID, linkFilter, -1, 0, nil, nil),
		})
	}()
}

const collectAfterMessages = 2000

func (acc *Account) handleGetMessagesAround(conn net.Conn, raw json.RawMessage) {
	var data struct {
		JID       string `json:"jid"`
		MessageID string `json:"messageId"`
		Text      string `json:"text"`
		Date      string `json:"date"`
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
			lookupErr = acc.db.QueryRow(`SELECT timestamp, id FROM messages WHERE jid = ? AND id = ?`,
				data.JID, data.MessageID).Scan(&targetTS, &targetID)
		} else if data.Date != "" {
			// The first message on or after that local day; a date past the
			// last message lands on the latest one.
			day, perr := time.ParseInLocation("2006-01-02", data.Date, time.Local)
			if perr != nil {
				lookupErr = perr
			} else {
				lookupErr = acc.db.QueryRow(`SELECT timestamp, id FROM messages WHERE jid = ? AND timestamp >= ?
					ORDER BY timestamp ASC, id ASC LIMIT 1`, data.JID, day.Unix()).Scan(&targetTS, &targetID)
				if lookupErr == sql.ErrNoRows {
					lookupErr = acc.db.QueryRow(`SELECT timestamp, id FROM messages WHERE jid = ?
						ORDER BY timestamp DESC, id DESC LIMIT 1`, data.JID).Scan(&targetTS, &targetID)
				}
			}
		} else if data.Text != "" {
			lookupErr = acc.db.QueryRow(`SELECT timestamp, id FROM messages WHERE jid = ? AND text LIKE ? ESCAPE '\'
				ORDER BY timestamp DESC LIMIT 1`, data.JID, "%"+escapeLike(data.Text)+"%").Scan(&targetTS, &targetID)
		} else {
			lookupErr = sql.ErrNoRows
		}

		offset := 0
		limit := context*2 + 1
		if lookupErr == nil {
			var position int
			if err := acc.db.QueryRow(`SELECT COUNT(*) FROM messages WHERE jid = ? AND
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

		msgs := acc.fetchMessagesPage(data.JID, limit, offset)
		older, newer := acc.countAround(data.JID, msgs)
		log.Printf("Got %d messages around target for %s", len(msgs), data.JID)
		acc.sendEvent(conn, "conversation_messages", map[string]interface{}{
			"jid":      data.JID,
			"messages": msgs,
			"loadMore": false,
			"isLatest": offset == 0,
			"older":    older,
			"newer":    newer,
			// Lets the UI stay where it is when jumping to a quoted message
			// that was never stored.
			"targetFound": lookupErr == nil,
			"targetId":    targetID,
		})
	}()
}

// fillSenderNames adds senderName and renders names in text for result rows
// read with their database rows already closed: resolving a name runs its own
// queries, and doing that while still reading rows held a pooled connection
// waiting on another.
func (acc *Account) fillSenderNames(results []map[string]interface{}) {
	names := map[string]string{}
	for _, r := range results {
		senderJID, _ := r["senderJid"].(string)
		name, ok := names[senderJID]
		if !ok && senderJID != "" {
			if parsed, err := types.ParseJID(senderJID); err == nil {
				name = acc.resolveDisplayName(parsed)
			}
			names[senderJID] = name
		}
		r["senderName"] = name
		if text, ok := r["text"].(string); ok {
			r["text"] = acc.renderNames(text, nil)
		}
	}
}

func escapeLike(s string) string {
	return strings.NewReplacer(`\`, `\\`, `%`, `\%`, `_`, `\_`).Replace(s)
}

func (acc *Account) handleGetStarredMessages(conn net.Conn) {
	go func() {
		rows, err := acc.db.Query(`SELECT id, jid, COALESCE(text,''), timestamp, from_me, COALESCE(sender_jid,''),
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
					results = append(results, map[string]interface{}{
						"id":         id,
						"jid":        jid,
						"fromMe":     fromMe,
						"text":       text,
						"timestamp":  ts,
						"senderJid":  senderJID,
						"mediaType":  mediaType,
						"quotedText": quotedText,
						"starred":    true,
					})
				}
			}
			rows.Close()
			acc.fillSenderNames(results)
		} else {
			log.Println("Error querying starred messages:", err)
		}
		acc.sendEvent(conn, "starred_messages", map[string]interface{}{
			"results": results,
		})
	}()
}

// globalSearchPageSize is how many messages a search of every chat sends at
// once, as WA Web's message search pages them.
const globalSearchPageSize = 30

func (acc *Account) handleSearchMessages(conn net.Conn, raw json.RawMessage) {
	var data SearchMessagesData
	if err := json.Unmarshal(raw, &data); err != nil || data.Query == "" || (data.JID == "" && strings.TrimSpace(data.Query) == "") {
		return
	}
	offset := max(data.Offset, 0)
	go func() {
		like := "%" + escapeLike(data.Query) + "%"
		var rows *sql.Rows
		var err error
		if data.JID != "" {
			rows, err = acc.db.Query(`SELECT id, jid, COALESCE(text,''), timestamp, from_me, COALESCE(sender_jid,''),
				COALESCE(status,''), COALESCE(media_type,''), COALESCE(media_mimetype,''), COALESCE(media_filename,''), COALESCE(media_path,''),
				COALESCE(quoted_id,''), COALESCE(quoted_text,''), COALESCE(quoted_sender,''), COALESCE(is_edited,0), COALESCE(is_deleted,0),
				COALESCE(sent_at,0), COALESCE(delivered_at,0), COALESCE(read_at,0), COALESCE(played_at,0), COALESCE(edited_at,0), COALESCE(starred,0) FROM messages
				WHERE jid = ? AND text LIKE ? ESCAPE '\' AND COALESCE(is_deleted, 0) = 0 ORDER BY timestamp ASC LIMIT 200`, data.JID, like)
		} else {
			// Every word, in any order, as WA Web's message search matches.
			words := strings.Fields(data.Query)
			conds := make([]string, len(words))
			wordArgs := make([]interface{}, len(words))
			for i, word := range words {
				conds[i] = `text LIKE ? ESCAPE '\'`
				wordArgs[i] = "%" + escapeLike(word) + "%"
			}
			wordsLike := strings.Join(conds, " AND ")
			rows, err = acc.db.Query(`SELECT id, jid, COALESCE(text,''), timestamp, from_me, COALESCE(sender_jid,''),
				COALESCE(status,''), COALESCE(media_type,''), COALESCE(media_mimetype,''), COALESCE(media_filename,''), COALESCE(media_path,''),
				COALESCE(quoted_id,''), COALESCE(quoted_text,''), COALESCE(quoted_sender,''), COALESCE(is_edited,0), COALESCE(is_deleted,0),
				COALESCE(sent_at,0), COALESCE(delivered_at,0), COALESCE(read_at,0), COALESCE(played_at,0), COALESCE(edited_at,0), COALESCE(starred,0) FROM messages
				WHERE `+wordsLike+` AND COALESCE(is_deleted, 0) = 0 AND COALESCE(media_type, '') NOT IN ('system', 'call')
				AND jid NOT IN (SELECT jid FROM chats WHERE COALESCE(is_locked, 0) = 1)
				ORDER BY timestamp DESC LIMIT ? OFFSET ?`, append(wordArgs, globalSearchPageSize+1, offset)...)
		}
		results := []map[string]interface{}{}
		hasMore := false
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
					results = append(results, map[string]interface{}{
						"id":            id,
						"jid":           jid,
						"fromMe":        fromMe,
						"text":          text,
						"timestamp":     ts,
						"senderJid":     senderJID,
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
			rows.Close()
			// A search of every chat comes newest first, a page at a time, as
			// in WA Web's chat list; one row past the page says more follow.
			if data.JID == "" && len(results) > globalSearchPageSize {
				results = results[:globalSearchPageSize]
				hasMore = true
			}
			acc.fillSenderNames(results)
		} else {
			log.Println("Error searching messages:", err)
		}
		acc.sendEvent(conn, "search_results", map[string]interface{}{
			"jid":     data.JID,
			"query":   data.Query,
			"offset":  offset,
			"hasMore": hasMore,
			"results": results,
		})
	}()
}

func (acc *Account) getLastMessageKeyForChat(jid_str string) (time.Time, *waCommon.MessageKey) {
	var msg_id string
	var msg_ts int64
	var from_me bool
	var sender_jid string
	err := acc.db.QueryRow(
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
		remote = acc.syncdChatJID(parsed).String()
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

func (acc *Account) sendAppStateWithRetry(ctx context.Context, patch appstate.PatchInfo) error {
	acc.appStateSendMu.Lock()
	defer acc.appStateSendMu.Unlock()

	err := acc.wa().SendAppState(ctx, patch)
	if err == nil {
		return nil
	}
	log.Printf("SendAppState %s failed, attempting resync: %v", patch.Type, err)

	acc.appStateResyncMu.Lock()
	acc.lastAppStateResync[patch.Type] = time.Now()
	acc.appStateResyncMu.Unlock()

	if sync_err := acc.wa().FetchAppState(ctx, patch.Type, false, false); sync_err != nil {
		log.Printf("Resync of %s failed: %v", patch.Type, sync_err)
		return fmt.Errorf("app state sync failed for %s: %w", patch.Type, sync_err)
	}
	if retry_err := acc.wa().SendAppState(ctx, patch); retry_err != nil {
		log.Printf("SendAppState %s retry also failed: %v", patch.Type, retry_err)
		return fmt.Errorf("app state retry failed for %s: %w", patch.Type, retry_err)
	}
	return nil
}

func (acc *Account) handleSetChatPinned(raw json.RawMessage) {
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
		patch := appstate.BuildPin(acc.syncdChatJID(jid), data.Value)
		if data.Value {
			lastTS, lastKey := acc.getLastMessageKeyForChat(data.JID)
			patch.Mutations = append(patch.Mutations, appstate.BuildArchive(acc.syncdChatJID(jid), false, lastTS, lastKey).Mutations...)
			patch = acc.withChatRange(patch, data.JID)
		}
		if err := acc.sendAppStateWithRetry(context.Background(), patch); err != nil {
			acc.broadcastChats()
			return
		}
		acc.applyPinChange(data.JID, data.Value, time.Now().Unix())
	}()
}

func (acc *Account) handleSetChatArchived(raw json.RawMessage) {
	var data ChatBoolData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		last_ts, last_key := acc.getLastMessageKeyForChat(data.JID)
		patch := acc.withChatRange(appstate.BuildArchive(acc.syncdChatJID(jid), data.Value, last_ts, last_key), data.JID)
		if err := acc.sendAppStateWithRetry(context.Background(), patch); err != nil {
			acc.broadcastChats()
			return
		}
		acc.noteArchiveAction(data.JID, data.Value, patch.Mutations[0].Value.GetArchiveChatAction().GetMessageRange(), time.Now().Unix())
		acc.db.Exec("UPDATE chats SET archived = ? WHERE jid = ?", boolToInt(data.Value), data.JID)
		if data.Value {
			// BuildArchive also unpins, as WA Web does.
			acc.applyPinChange(data.JID, false, time.Now().Unix())
		}
		acc.broadcastChats()
	}()
}

func (acc *Account) handleSetChatMuted(raw json.RawMessage) {
	var data SetChatMutedData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	if isChannelJID(data.JID) {
		go acc.handleChannelMute(data.JID, data.Muted)
		return
	}
	go func() {
		duration := time.Duration(data.DurationSeconds) * time.Second
		if err := acc.sendAppStateWithRetry(context.Background(), appstate.BuildMute(acc.syncdChatJID(jid), data.Muted, duration)); err != nil {
			acc.broadcastChats()
			return
		}
		muted_int := 0
		if data.Muted {
			muted_int = 1
		}
		acc.db.Exec("UPDATE chats SET muted = ? WHERE jid = ?", muted_int, data.JID)
		acc.broadcastChats()
	}()
}

func (acc *Account) handleSetChatRead(raw json.RawMessage) {
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
		payload, _ := json.Marshal(map[string]any{"jid": data.JID})
		acc.handleMarkRead(payload)
		acc.db.Exec("UPDATE chats SET unread_count = 0, marked_unread = 0 WHERE jid = ?", data.JID)
		acc.broadcastChats()
		return
	}
	acc.db.Exec("UPDATE chats SET unread_count = 1, marked_unread = 1 WHERE jid = ?", data.JID)
	acc.broadcastChats()
	go func() {
		last_ts, last_key := acc.getLastMessageKeyForChat(data.JID)
		if err := acc.sendAppStateWithRetry(context.Background(), acc.withChatRange(appstate.BuildMarkChatAsRead(acc.syncdChatJID(jid), false, last_ts, last_key), data.JID)); err != nil {
			log.Printf("Marking %s unread on other devices failed: %v", data.JID, err)
		}
	}()
}

// handleDeleteChat previously only deleted the chat locally - it never told
// WhatsApp at all, so a chat deleted from this app stayed put on the phone
// and every other linked device. Sends the same "deleteChat" app-state
// mutation the phone itself would, same as handleSetChatLocked/
// handleSetChatRead already do for their own actions.
func (acc *Account) handleDeleteChat(raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		last_ts, last_key := acc.getLastMessageKeyForChat(data.JID)
		if err := acc.sendAppStateWithRetry(context.Background(), acc.withChatRange(appstate.BuildDeleteChat(acc.syncdChatJID(jid), last_ts, last_key, false), data.JID)); err != nil {
			acc.broadcastEvent("error", map[string]string{"message": "Failed to delete chat: " + err.Error()})
			return
		}
		acc.purgeMessages("jid = ?", data.JID)
		acc.db.Exec("DELETE FROM chats WHERE jid = ?", data.JID)
		acc.broadcastChats()
	}()
}

func (acc *Account) handleClearChat(raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		last_ts, last_key := acc.getLastMessageKeyForChat(data.JID)
		if err := acc.sendAppStateWithRetry(context.Background(), acc.withChatRange(appstate.BuildClearChat(acc.syncdChatJID(jid), last_ts, last_key, false, false), data.JID)); err != nil {
			acc.broadcastEvent("error", map[string]string{"message": "Failed to clear chat: " + err.Error()})
			return
		}
		acc.purgeMessages("jid = ?", data.JID)
		acc.clearChatPreview(data.JID)
		acc.broadcastChats()
		acc.broadcastEvent("chat_cleared", map[string]interface{}{"jid": data.JID})
	}()
}

func (acc *Account) handleRequestMessageResend(raw json.RawMessage) {
	var data struct {
		JID       string `json:"jid"`
		MessageID string `json:"message_id"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}

	var senderJID string
	acc.db.QueryRow("SELECT COALESCE(sender_jid, '') FROM messages WHERE id = ?", data.MessageID).Scan(&senderJID)
	acc.requestMessageResend(data.MessageID, data.JID, senderJID)
}

func (acc *Account) handleRequestHistorySync(raw json.RawMessage) {
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
		go acc.loadOlderChannelPosts(data.JID)
		return
	}
	acc.requestOlderHistory(chatJID, data.JID, data.Count, true)
}

func (acc *Account) requestOlderHistory(chatJID types.JID, jid string, count int, autoRetry bool) {
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
	oldest, err := acc.oldestHistoryAnchor(data.JID, nil)
	if err != nil {
		acc.requestHistoryForEmptyChat(chatJID, data.JID, count)
		return
	}

	// The UI asks at most once per time a chat is opened. This is the
	// backstop under that: the same request (same chat, same anchor) is
	// refused if it was sent moments ago, so a UI bug or a held-down key
	// can never turn into a stream of identical requests to the phone.
	var lastAnchor, lastOldest string
	var lastAt int64
	haveLast := acc.db.QueryRow(`SELECT anchor_id, COALESCE(oldest_id, ''), requested_at FROM history_requests WHERE jid = ?`,
		data.JID).Scan(&lastAnchor, &lastOldest, &lastAt) == nil
	if haveLast && lastOldest == oldest.id && time.Since(time.Unix(lastAt, 0)) < historyRequestRepeatAfter {
		log.Printf("Not re-requesting history for %s: already asked before %s at %s", data.JID, lastAnchor, time.Unix(lastAt, 0).Format(time.RFC3339))
		acc.broadcastEvent("no_more_history", map[string]string{"jid": data.JID})
		return
	}
	anchor := oldest
	if haveLast && lastOldest == oldest.id {
		if alt, altErr := acc.oldestHistoryAnchor(data.JID, []string{oldest.id, lastAnchor}); altErr == nil {
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

	if _, err := acc.db.Exec(`INSERT INTO history_requests (jid, anchor_id, oldest_id, requested_at) VALUES (?, ?, ?, ?)
		ON CONFLICT(jid) DO UPDATE SET anchor_id = excluded.anchor_id, oldest_id = excluded.oldest_id, requested_at = excluded.requested_at`,
		data.JID, msgID, oldest.id, time.Now().Unix()); err != nil {
		log.Printf("history sync request for %s: couldn't record it, not sending: %v", data.JID, err)
		return
	}

	if count <= 0 || count > 50 {
		count = 50
	}
	log.Printf("Requesting older history for %s before %s (count %d)", data.JID, msgID, count)
	req := acc.wa().BuildHistorySyncRequest(msgInfo, count)
	go func() {
		answered := make(chan struct{}, 1)
		acc.onDemandWaiters.Store(data.JID, answered)
		defer acc.onDemandWaiters.CompareAndDelete(data.JID, answered)
		acc.historyRequestMu.Lock()
		if wait := historyRequestMinGap - time.Since(acc.lastHistoryRequest); wait > 0 {
			time.Sleep(wait)
		}
		acc.lastHistoryRequest = time.Now()
		_, sendErr := acc.wa().SendPeerMessage(context.Background(), req)
		acc.historyRequestMu.Unlock()
		if sendErr != nil {
			log.Printf("history sync request for %s failed: %v", data.JID, sendErr)
			return
		}
		if autoRetry {
			select {
			case <-answered:
			case <-time.After(onDemandRetryAfter):
				log.Printf("No answer from the phone for %s before %s after %s; retrying with another anchor", data.JID, msgID, onDemandRetryAfter)
				acc.requestOlderHistory(chatJID, data.JID, count, false)
			}
			return
		}
		select {
		case <-answered:
			if now, err := acc.oldestHistoryAnchor(data.JID, nil); err != nil || now.id != oldest.id {
				return
			}
			log.Printf("The phone's answer for %s held nothing older than %s; this is the start of the chat", data.JID, oldest.id)
		case <-time.After(onDemandStartAfter):
			log.Printf("No answer from the phone for %s after a retry either; this is the start of the chat", data.JID)
		}
		acc.broadcastEvent("history_start_reached", map[string]string{"jid": data.JID})
	}()
}

func (acc *Account) requestHistoryForEmptyChat(chatJID types.JID, jid string, count int) {
	if _, asked := acc.emptyChatHistoryAsked.LoadOrStore(jid, true); asked {
		acc.broadcastEvent("history_start_reached", map[string]string{"jid": jid})
		return
	}
	if count <= 0 || count > 50 {
		count = 50
	}
	req := acc.wa().BuildHistorySyncRequest(&types.MessageInfo{
		MessageSource: types.MessageSource{Chat: chatJID},
		Timestamp:     time.Now(),
	}, count)
	onDemand := req.GetProtocolMessage().GetPeerDataOperationRequestMessage().GetHistorySyncOnDemandRequest()
	onDemand.OldestMsgID, onDemand.OldestMsgFromMe = nil, nil
	log.Printf("Requesting the latest history for %s, which has no messages yet (count %d)", jid, count)
	go func() {
		answered := make(chan struct{}, 1)
		acc.onDemandWaiters.Store(jid, answered)
		defer acc.onDemandWaiters.CompareAndDelete(jid, answered)
		acc.historyRequestMu.Lock()
		if wait := historyRequestMinGap - time.Since(acc.lastHistoryRequest); wait > 0 {
			time.Sleep(wait)
		}
		acc.lastHistoryRequest = time.Now()
		_, sendErr := acc.wa().SendPeerMessage(context.Background(), req)
		acc.historyRequestMu.Unlock()
		if sendErr != nil {
			log.Printf("history sync request for %s failed: %v", jid, sendErr)
			acc.emptyChatHistoryAsked.Delete(jid)
			acc.broadcastEvent("no_more_history", map[string]string{"jid": jid})
			return
		}
		select {
		case <-answered:
		case <-time.After(onDemandRetryAfter):
			log.Printf("No answer from the phone for %s, which has no messages; leaving it empty", jid)
			acc.broadcastEvent("no_more_history", map[string]string{"jid": jid})
		}
	}()
}

func (acc *Account) noteOnDemandAnswer(jid string) {
	if ch, ok := acc.onDemandWaiters.Load(jid); ok {
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

type historyAnchor struct {
	id, sender string
	fromMe     bool
	ts         int64
}

func (acc *Account) oldestHistoryAnchor(jid string, exclude []string) (historyAnchor, error) {
	var a historyAnchor
	if len(exclude) == 0 {
		err := acc.db.QueryRow(`SELECT id, COALESCE(sender_jid, ''), from_me, timestamp FROM messages
			WHERE jid = ? AND COALESCE(media_type, '') NOT IN ('system', 'call')
			ORDER BY timestamp ASC, id ASC LIMIT 1`, jid).Scan(&a.id, &a.sender, &a.fromMe, &a.ts)
		return a, err
	}
	args := []interface{}{jid}
	notIn := strings.TrimSuffix(strings.Repeat("?, ", len(exclude)), ", ")
	for _, id := range exclude {
		args = append(args, id)
	}
	err := acc.db.QueryRow(`SELECT id, COALESCE(sender_jid, ''), from_me, timestamp FROM messages
		WHERE jid = ? AND COALESCE(media_type, '') NOT IN ('system', 'call') AND id NOT IN (`+notIn+`)
		ORDER BY (COALESCE(is_deleted, 0) = 0 AND COALESCE(media_type, '') = '' AND COALESCE(album_id, '') = '') DESC,
			timestamp ASC, id ASC LIMIT 1`, args...).Scan(&a.id, &a.sender, &a.fromMe, &a.ts)
	return a, err
}
