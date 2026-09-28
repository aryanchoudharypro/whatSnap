package main

import (
	"database/sql"
	"encoding/json"
	"fmt"
	"log"
	"strings"
	"time"

	"net"
	"go.mau.fi/whatsmeow/types"
)

func broadcastEvent(evtType string, data interface{}) {
	line := encodeEvent(evtType, data)
	if line == nil {
		return
	}
	wsMutex.Lock()
	defer wsMutex.Unlock()
	for conn := range wsClients {
		writeWithDeadline(conn, line)
	}
}

func sendEvent(conn net.Conn, evtType string, data interface{}) {
	// Encoded before taking wsMutex: marshalling a chat's history is the
	// slow part, and nothing else could write to the pipe meanwhile.
	line := encodeEvent(evtType, data)
	if line == nil {
		return
	}
	wsMutex.Lock()
	defer wsMutex.Unlock()
	writeWithDeadline(conn, line)
}

// encodeEvent builds one newline-terminated {"type":..,"data":..} line.
//
// Assembled by hand rather than marshalling an Event: the payload is already
// valid JSON once marshalled, so json.Marshal(evt) would copy the whole thing a
// second time purely to wrap it, and a trailing append could copy it a third.
// On a chat's full history that is tens of MB per copy. evtType still goes
// through Marshal so it stays correctly escaped, and the empty-data case
// matches Event's `omitempty`.
func encodeEvent(evtType string, data interface{}) []byte {
	var raw []byte
	if data != nil {
		bytes, err := json.Marshal(data)
		if err != nil {
			// Still send the bare event rather than dropping it, so the UI's
			// dispatch is unchanged - but say so, it used to fail silently.
			log.Println("encodeEvent: failed to marshal", evtType, "payload:", err)
		} else {
			raw = bytes
		}
	}
	typeJSON, err := json.Marshal(evtType)
	if err != nil {
		return nil
	}
	buf := make([]byte, 0, len(raw)+len(typeJSON)+32)
	buf = append(buf, `{"type":`...)
	buf = append(buf, typeJSON...)
	if len(raw) > 0 {
		buf = append(buf, `,"data":`...)
		buf = append(buf, raw...)
	}
	return append(buf, '}', '\n')
}

// pipeWriteTimeout bounds one write to the frontend. Writes happen under
// wsMutex, so a frontend that stopped reading (a hung UI thread) used to block
// every goroutine in the backend that tried to send anything, indefinitely.
const pipeWriteTimeout = 15 * time.Second

// writeWithDeadline writes buf to conn, dropping the connection if the
// frontend doesn't take it in time - it reconnects and resyncs on its own.
// Must be called with wsMutex held.
func writeWithDeadline(conn net.Conn, buf []byte) {
	conn.SetWriteDeadline(time.Now().Add(pipeWriteTimeout))
	if _, err := conn.Write(buf); err != nil {
		log.Println("Dropping IPC connection after failed write:", err)
		delete(wsClients, conn)
		conn.Close()
	}
}

// broadcastChats re-reads the local chat list cache and pushes it to every
// connected client. Display-name / pinned / muted / archived resolution is
// centralized in resolveDisplayName and the chat-settings lookup below so
// this and get_chats can't drift apart.
var (
	chatBroadcastTimer *time.Timer
	// When the oldest not-yet-sent request arrived; zero when none is pending.
	chatBroadcastPendingSince time.Time
)

const (
	chatBroadcastDebounce = 200 * time.Millisecond
	// A steady stream of events (history sync, a busy group) re-armed the
	// debounce forever, so the list never updated until things went quiet.
	chatBroadcastMaxWait = time.Second
)

// broadcastChats re-reads the local chat list cache and pushes it to every
// connected client. It is debounced to avoid flooding the UI during bulk
// syncs, but never held back more than chatBroadcastMaxWait.
func broadcastChats() {
	wsMutex.Lock()
	defer wsMutex.Unlock()

	now := time.Now()
	if chatBroadcastPendingSince.IsZero() {
		chatBroadcastPendingSince = now
	}
	delay := chatBroadcastDebounce
	if waited := now.Sub(chatBroadcastPendingSince); waited+delay > chatBroadcastMaxWait {
		delay = max(chatBroadcastMaxWait-waited, 0)
	}
	if chatBroadcastTimer != nil {
		chatBroadcastTimer.Stop()
	}
	chatBroadcastTimer = time.AfterFunc(delay, func() {
		wsMutex.Lock()
		chatBroadcastPendingSince = time.Time{}
		wsMutex.Unlock()
		broadcastEvent("chats", buildChatsList())
	})
}

// ChatListEntry is one chat as sent to the UI in a "chats" event; the field
// names must stay in sync with the Rust side's ChatRow. A struct rather than a
// map per chat, for the same reason as MessageJSON: the whole list is rebuilt
// and re-sent after every message (debounced to 200ms), and a map meant a Go
// map plus a boxed value per field of every chat each time.
type ChatListEntry struct {
	JID                  string
	FullName             string
	Timestamp            int64
	UnreadCount          int
	Pinned               bool
	Muted                bool
	PreviewSnippet       string
	Archived             bool
	PreviewSender        string
	PreviewFromMe        bool
	PreviewIsReaction    bool
	PreviewReactionEmoji string
	IsCommunity          bool
	IsAdmin              bool
	IsLocked             bool
	ParentJID            string
	IsAnnouncements      bool
	CommunityName        string
	IsChannel            bool
	ChannelRole          string
}

// buildChatsList is the shared query+formatting logic behind both
// broadcastChats (pushed to everyone) and the get_chats command (requested
// by one client). Keeping this in one place means the two can't drift.
func buildChatsList() []ChatListEntry {
	rows, err := db.Query(`SELECT jid, name, timestamp, unread_count, pinned, muted, preview_snippet, archived, COALESCE(preview_sender, ''), COALESCE(preview_from_me, 0), COALESCE(preview_is_reaction, 0), COALESCE(preview_reaction_emoji, ''), COALESCE(is_community, 0), COALESCE(is_admin, 0), COALESCE(is_locked, 0), COALESCE(parent_jid, ''), COALESCE(is_announce_group, 0), COALESCE(is_channel, 0), COALESCE(channel_role, '') FROM chats ORDER BY pinned DESC, timestamp DESC, name ASC`)
	if err != nil {
		fmt.Println("Error querying chats:", err)
		return []ChatListEntry{}
	}
	defer rows.Close()

	// Community names, for labelling the groups inside each one.
	communityNames := map[string]string{}
	if nameRows, nameErr := db.Query(`SELECT jid, COALESCE(name, '') FROM chats WHERE COALESCE(is_community, 0) = 1`); nameErr == nil {
		for nameRows.Next() {
			var cj, cn string
			if nameRows.Scan(&cj, &cn) == nil {
				communityNames[cj] = cn
			}
		}
		nameRows.Close()
	}

	// Preview senders repeat across chats (you, and the busiest people in
	// your groups); each resolveDisplayName is several store lookups.
	senderNames := map[string]string{}
	settingsByChat := allChatSettings()
	chats := []ChatListEntry{}
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
		var parentJID string
		var isAnnounceGroup int
		var isChannel int
		var channelRole string

		if err := rows.Scan(&jid, &name, &ts, &unread, &pinned, &muted, &preview, &archived, &previewSender, &previewFromMe, &previewIsReaction, &previewReactionEmoji, &isCommunity, &isAdmin, &isLocked, &parentJID, &isAnnounceGroup, &isChannel, &channelRole); err != nil {
			continue
		}
		n := jid
		if name.Valid && name.String != "" {
			n = name.String
		}

		parsedJID, errJid := types.ParseJID(jid)
		if errJid == nil {
			// A group is named by its own subject, kept in the chats
			// table; only people (and channels) need resolving.
			if parsedJID.Server != types.GroupServer {
				resolved := cachedDisplayName(parsedJID)
				if resolved != parsedJID.String() {
					n = resolved
				} else if n == jid && parsedJID.Server != "lid" {
					n = "+" + parsedJID.User
				}
			}

			if chatSettings, found := chatSettingsFromMap(settingsByChat, parsedJID); found {
				if chatSettings.Archived {
					archived = 1
				}
				if chatSettings.Pinned {
					pinned = 1
				}
				// The synced mute end decides, not the stored flag: a mute "for 8
				// hours" ends by itself, with no unmute ever sent.
				muted = 0
				if !chatSettings.MutedUntil.IsZero() && chatSettings.MutedUntil.After(time.Now()) {
					muted = 1
				}
			}
		}

		p := ""
		if preview.Valid {
			p = renderNames(preview.String, nil)
		}

		ps := ""
		if previewSender.Valid && previewSender.String != "" {
			if cached, ok := senderNames[previewSender.String]; ok {
				ps = cached
			} else if parsedSender, sErr := types.ParseJID(previewSender.String); sErr == nil {
				ps = cachedDisplayName(parsedSender)
				senderNames[previewSender.String] = ps
			} else {
				ps = previewSender.String
			}
		}

		chats = append(chats, ChatListEntry{
			JID:                  jid,
			FullName:             n,
			Timestamp:            ts,
			UnreadCount:          unread,
			Pinned:               pinned == 1,
			Muted:                muted == 1,
			PreviewSnippet:       p,
			Archived:             archived == 1,
			PreviewSender:        ps,
			PreviewFromMe:        previewFromMe == 1,
			PreviewIsReaction:    previewIsReaction == 1,
			PreviewReactionEmoji: previewReactionEmoji,
			IsCommunity:          isCommunity == 1,
			IsAdmin:              isAdmin == 1,
			IsLocked:             isLocked == 1,
			ParentJID:            parentJID,
			IsAnnouncements:      isAnnounceGroup == 1 && parentJID != "",
			CommunityName:        communityNames[parentJID],
			IsChannel:            isChannel == 1,
			ChannelRole:          strings.ToLower(channelRole),
		})
	}
	return chats
}
