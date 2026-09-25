package main

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"log"
	"runtime/debug"
	"strings"
	"sync"
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

// Shared zero-length slice so a message with no reactions marshals as
// `"reactions": []` (which the UI expects) without allocating one per message.
var emptyReactions = []ReactionInfo{}

// fetchMessagesPage runs the shared "one page of a chat's messages, oldest
// first" query used by both a plain get_messages and the around-a-target
// variant below, so a single page's worth of local history is always built
// the same way regardless of who's asking for it.
// pageCursor, when set, pages by position instead of offset: only messages
// strictly older than (Timestamp, ID) are returned. Offsets drift - a message
// arriving between two pages shifts every later one, and an offset counted
// from the newest message is meaningless for a window opened around a search
// hit in the middle of the chat.
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

// fetchMessagesPageAt returns one page of a chat, oldest first. With after
// set, it's the messages strictly newer than that cursor - paging forward
// from a window opened around a search hit; otherwise the newest ones (before
// the before cursor, or skipping offset).
func fetchMessagesPageAt(jid string, limit, offset int, before, after *pageCursor) []MessageJSON {
	// raw is only needed for the rare voice row stored without a duration;
	// selecting it unconditionally pulled every row's whole protobuf into
	// memory just to throw it away.
	const cols = `SELECT id, COALESCE(text, ''), timestamp, from_me, COALESCE(sender_jid, ''),
			COALESCE(status, ''), COALESCE(media_type, ''), COALESCE(media_mimetype, ''), COALESCE(media_filename, ''), COALESCE(media_path, ''), COALESCE(media_size, 0),
			COALESCE(quoted_id, ''), COALESCE(quoted_text, ''), COALESCE(quoted_sender, ''), COALESCE(is_edited, 0), COALESCE(is_deleted, 0),
			COALESCE(sent_at, 0), COALESCE(delivered_at, 0), COALESCE(read_at, 0), COALESCE(played_at, 0), COALESCE(edited_at, 0), COALESCE(starred, 0), COALESCE(album_id, ''),
			CASE WHEN media_type IN ('voice', 'audio') AND COALESCE(text, '') NOT LIKE '%(%' THEN raw ELSE NULL END,
			COALESCE(channel_views, 0), COALESCE(channel_reactions, ''), COALESCE(channel_pinned, 0)
		FROM messages WHERE jid = ?`
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

	// A page's size is known up front (a whole chat, limit -1, isn't).
	msgs := make([]MessageJSON, 0, min(max(limit, 0), 5000))
	ids := make([]string, 0, cap(msgs))
	var quotedIDs []string
	// A whole chat has thousands of rows from a handful of senders; resolving
	// each row's names from scratch (a contact-store lookup, often a query
	// too) was most of the cost of loading one.
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
			var channelPinned bool
			if err := rows.Scan(&id, &text, &ts, &fromMe, &senderJID, &status, &mediaType, &mediaMimetype, &mediaFilename, &mediaPath, &mediaSize,
				&quotedID, &quotedText, &quotedSender, &isEdited, &isDeleted, &sentAt, &deliveredAt, &readAt, &playedAt, &editedAt, &starred, &albumID, &rawBytes,
				&channelViews, &channelReactions, &channelPinned); err != nil {
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
			pollResults := ""
			if strings.HasPrefix(text, "[Poll]") {
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
				// Resolved to a name below, once every quoted original
				// has been looked up in one go.
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
				Reactions:     emptyReactions,
			})
		}
	} else {
		log.Println("Error querying messages:", queryErr)
	}

	if after == nil {
		// Queried newest first; the page goes out oldest first.
		for i, j := 0, len(msgs)-1; i < j; i, j = i+1, j-1 {
			msgs[i], msgs[j] = msgs[j], msgs[i]
		}
	}
	// The quoted_sender column is a denormalized snapshot taken when a reply
	// first arrived, and can go stale or wrong (e.g. a participant field
	// missing at delivery time). When we still have the original message
	// locally, its own from_me/sender_jid is the authoritative answer, so
	// prefer it over the snapshot. Looked up for the whole page at once:
	// one query per reply was one query per reply in the chat.
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

// quotedSenderName is who a reply quotes, as shown: "You", their name, or the
// stored snapshot when it isn't a JID.
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
		JID    string `json:"jid"`
		Limit  int    `json:"limit"`
		Offset int    `json:"offset"`
		// Page cursor (see pageCursor): the oldest message already loaded.
		BeforeID        string `json:"before_id"`
		BeforeTimestamp int64  `json:"before_timestamp"`
		// The newest message already loaded, for paging forward from a
		// window that stops short of the latest message.
		AfterID        string `json:"after_id"`
		AfterTimestamp int64  `json:"after_timestamp"`
		// Set on the reload that follows a channel fetch, so it doesn't
		// start another one.
		NoServerFetch bool `json:"no_server_fetch"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		if data.AfterID != "" {
			limit := data.Limit
			if limit == 0 {
				limit = 50
			}
			msgs := fetchMessagesPageAt(data.JID, limit, 0, nil, &pageCursor{Timestamp: data.AfterTimestamp, ID: data.AfterID})
			log.Printf("Got %d newer messages for %s", len(msgs), data.JID)
			sendEvent(conn, "conversation_messages", map[string]interface{}{
				"jid":       data.JID,
				"messages":  msgs,
				"loadNewer": true,
				// A short page (or "all") means it reached the newest message.
				"isLatest": limit < 0 || len(msgs) < limit,
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

		// A negative limit asks for every message (SQLite's LIMIT -1): the
		// UI loads a whole chat when it's opened.
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
		// A channel's posts come from the server, not the phone: fetch the
		// latest when it's opened. The stored ones show meanwhile.
		if !loadMore && !data.NoServerFetch && isChannelJID(data.JID) {
			go refreshChannel(data.JID)
		}
		sendEvent(conn, "conversation_messages", map[string]interface{}{
			"jid":      data.JID,
			"messages": msgs,
			"loadMore": loadMore,
		})
		// Opening a large chat is by far this process's biggest allocation
		// burst, and every byte of it is garbage the moment the response is on
		// the wire. Go's scavenger releases that back to the OS only gradually,
		// so without an explicit nudge the process sits at its peak for minutes
		// after the user has already closed the chat.
		if len(msgs) >= collectAfterMessages {
			debug.FreeOSMemory()
		}
	}()
}

// Messages in one response past which it is worth forcing the heap back to the
// OS. Small responses are frequent and their garbage is cheap; paying for a
// full GC plus scavenge on each of those would cost more than it returns.
const collectAfterMessages = 2000

// handleGetMessagesAround finds a target message (by id, falling back to a
// text match) and returns a bounded window of local history centered on it,
// instead of a plain get_messages with an enormous limit fetching the whole
// chat just to jump to one message in it. If the target can't be found
// locally, falls back to the most recent page so the caller still gets a
// usable (if not-scrolled-to-target) view rather than nothing.
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
			limit = 50
		}

		msgs := fetchMessagesPage(data.JID, limit, offset)
		log.Printf("Got %d messages around target for %s", len(msgs), data.JID)
		sendEvent(conn, "conversation_messages", map[string]interface{}{
			"jid":      data.JID,
			"messages": msgs,
			"loadMore": false,
			// offset 0 means the window runs right up to the newest message;
			// otherwise newer ones exist that this view doesn't hold.
			"isLatest": offset == 0,
		})
	}()
}

// escapeLike makes a user's search text match literally in a LIKE pattern
// (with ESCAPE '\'): otherwise "%" and "_" in it act as wildcards.
func escapeLike(s string) string {
	return strings.NewReplacer(`\`, `\\`, `%`, `\%`, `_`, `\_`).Replace(s)
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

// handleSearchMessages implements real full-text search over the local
// message cache (previously a stub). Search is scoped to one chat when jid
// is provided, otherwise it searches everything.
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
		`SELECT id, timestamp, from_me, COALESCE(sender_jid, '') FROM messages
			WHERE jid = ? AND COALESCE(media_type, '') NOT IN ('system', 'call') AND id NOT LIKE 'system\_%' ESCAPE '\' AND id NOT LIKE 'call:%'
			ORDER BY timestamp DESC LIMIT 1`,
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
	if isChannelJID(data.JID) {
		go handleChannelMute(data.JID, data.Muted)
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
		if err := sendAppStateWithRetry(context.Background(), appstate.BuildDeleteChat(jid, last_ts, last_key, false)); err != nil {
			broadcastEvent("error", map[string]string{"message": "Failed to delete chat: " + err.Error()})
			return
		}
		purgeMessages("jid = ?", data.JID)
		db.Exec("DELETE FROM chats WHERE jid = ?", data.JID)
		broadcastChats()
	}()
}

// handleClearChat empties a chat's messages while leaving the chat itself in
// the list, mirroring "Clear chat" in official WhatsApp clients - distinct
// from handleDeleteChat, which removes the chat entirely.
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
		if err := sendAppStateWithRetry(context.Background(), appstate.BuildClearChat(jid, last_ts, last_key, false, false)); err != nil {
			broadcastEvent("error", map[string]string{"message": "Failed to clear chat: " + err.Error()})
			return
		}
		purgeMessages("jid = ?", data.JID)
		broadcastChats()
		broadcastEvent("chat_cleared", map[string]interface{}{"jid": data.JID})
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
	// A channel's history is on the server, not the phone.
	if chatJID.Server == types.NewsletterServer {
		go loadOlderChannelPosts(data.JID)
		return
	}

	// The phone answers with what comes before the oldest message it's
	// told about, so that has to be a real WhatsApp message it knows:
	// the chat's oldest stored one, skipping the group-event and
	// call-history rows whaNTV makes up itself. Chosen here rather than
	// trusting the UI's pick - it used to send its first row, which is
	// always a date divider with no ID, so the lookup failed and the
	// request was silently never sent at all.
	var msgID, senderJID string
	var fromMe bool
	var ts int64
	err = db.QueryRow(`SELECT id, COALESCE(sender_jid, ''), from_me, timestamp FROM messages
		WHERE jid = ? AND COALESCE(media_type, '') NOT IN ('system', 'call')
		ORDER BY timestamp ASC, id ASC LIMIT 1`, data.JID).Scan(&msgID, &senderJID, &fromMe, &ts)
	if err != nil {
		log.Printf("history sync request for %s: no message to anchor it on: %v", data.JID, err)
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
		ID: msgID,
		// BuildHistorySyncRequest sends this as the anchor's time; left
		// unset it went out as year 1.
		Timestamp: time.Unix(ts, 0),
	}

	// The UI asks at most once per time a chat is opened. This is the
	// backstop under that: the same request (same chat, same anchor) is
	// refused if it was sent moments ago, so a UI bug or a held-down key
	// can never turn into a stream of identical requests to the phone.
	var lastAnchor string
	var lastAt int64
	if db.QueryRow(`SELECT anchor_id, requested_at FROM history_requests WHERE jid = ?`, data.JID).Scan(&lastAnchor, &lastAt) == nil &&
		lastAnchor == msgID && time.Since(time.Unix(lastAt, 0)) < historyRequestRepeatAfter {
		log.Printf("Not re-requesting history for %s: already asked before %s at %s", data.JID, msgID, time.Unix(lastAt, 0).Format(time.RFC3339))
		broadcastEvent("no_more_history", map[string]string{"jid": data.JID})
		return
	}
	if _, err := db.Exec(`INSERT INTO history_requests (jid, anchor_id, requested_at) VALUES (?, ?, ?)
		ON CONFLICT(jid) DO UPDATE SET anchor_id = excluded.anchor_id, requested_at = excluded.requested_at`,
		data.JID, msgID, time.Now().Unix()); err != nil {
		log.Printf("history sync request for %s: couldn't record it, not sending: %v", data.JID, err)
		return
	}

	count := data.Count
	if count <= 0 || count > 50 {
		count = 50
	}
	log.Printf("Requesting older history for %s before %s (count %d)", data.JID, msgID, count)
	req := client.BuildHistorySyncRequest(msgInfo, count)
	go func() {
		historyRequestMu.Lock()
		defer historyRequestMu.Unlock()
		// Spaced out even across different chats, so paging quickly through
		// many of them can't turn into a burst.
		if wait := historyRequestMinGap - time.Since(lastHistoryRequest); wait > 0 {
			time.Sleep(wait)
		}
		lastHistoryRequest = time.Now()
		if _, sendErr := client.SendPeerMessage(context.Background(), req); sendErr != nil {
			log.Printf("history sync request for %s failed: %v", data.JID, sendErr)
		}
	}()
}

const (
	// historyRequestRepeatAfter is how long before the same on-demand
	// history request (same chat, same anchor message) may be sent again.
	historyRequestRepeatAfter = 10 * time.Second
	// historyRequestMinGap spaces out on-demand history requests.
	historyRequestMinGap = 2 * time.Second
)

var (
	historyRequestMu   sync.Mutex
	lastHistoryRequest time.Time
)
