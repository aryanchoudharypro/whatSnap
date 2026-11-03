package main

import (
	"database/sql"
	"encoding/json"
	"fmt"
	"log"
	"sort"
	"strings"
	"time"

	"go.mau.fi/whatsmeow/types"
	"net"
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
	line := encodeEvent(evtType, data)
	if line == nil {
		return
	}
	wsMutex.Lock()
	defer wsMutex.Unlock()
	writeWithDeadline(conn, line)
}

func encodeEvent(evtType string, data interface{}) []byte {
	var raw []byte
	if data != nil {
		bytes, err := json.Marshal(data)
		if err != nil {
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

var (
	chatBroadcastTimer        *time.Timer
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
	UnreadMentions       int
	Pinned               bool
	Muted                bool
	PreviewSnippet       string
	Archived             bool
	PreviewSender        string
	PreviewFromMe        bool
	PreviewStatus        string
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
	AdminsOnly           bool
	NotMember            bool
	IsBusiness           bool
}

func buildChatsList() []ChatListEntry {
	refreshArchivedChats()
	rows, err := db.Query(`SELECT jid, name, timestamp, unread_count, pinned, muted, preview_snippet, archived, COALESCE(preview_sender, ''), COALESCE(preview_from_me, 0), COALESCE(preview_is_reaction, 0), COALESCE(preview_reaction_emoji, ''), COALESCE(is_community, 0), COALESCE(is_admin, 0), COALESCE(is_locked, 0), COALESCE(parent_jid, ''), COALESCE(is_announce_group, 0), COALESCE(is_channel, 0), COALESCE(channel_role, ''), COALESCE(admins_only, 0),
		CASE WHEN COALESCE(preview_from_me, 0) = 1 THEN COALESCE((SELECT m.status FROM messages m WHERE m.jid = chats.jid AND m.from_me = 1 ORDER BY m.timestamp DESC LIMIT 1), '') ELSE '' END
		, COALESCE(pin_changed_at, 0), COALESCE(unread_mentions, 0)
		FROM chats ORDER BY pinned DESC, COALESCE(pinned_at, 0) DESC, timestamp DESC, name ASC`)
	if err != nil {
		fmt.Println("Error querying chats:", err)
		return []ChatListEntry{}
	}
	defer rows.Close()

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

	businessJIDs := businessContactJIDs()

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
		var adminsOnly int
		var isChannel int
		var channelRole string
		var previewStatus string
		var pinChangedAt int64
		var unreadMentions int

		if err := rows.Scan(&jid, &name, &ts, &unread, &pinned, &muted, &preview, &archived, &previewSender, &previewFromMe, &previewIsReaction, &previewReactionEmoji, &isCommunity, &isAdmin, &isLocked, &parentJID, &isAnnounceGroup, &isChannel, &channelRole, &adminsOnly, &previewStatus, &pinChangedAt, &unreadMentions); err != nil {
			continue
		}
		n := jid
		if name.Valid && name.String != "" {
			n = name.String
		}

		parsedJID, errJid := types.ParseJID(jid)
		if errJid == nil {
			if parsedJID.Server != types.GroupServer {
				resolved := cachedDisplayName(parsedJID)
				if resolved != parsedJID.String() {
					n = resolved
				} else if n == jid && parsedJID.Server != "lid" {
					n = "+" + parsedJID.User
				}
			}

			if chatSettings, found := chatSettingsFromMap(settingsByChat, parsedJID); found {
				// The settings store only fills in a pin the chat row never
				// heard about (it has no time, so it can't outrank a stored
				// pin change).
				if chatSettings.Pinned && pinChangedAt == 0 {
					pinned = 1
				}
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
			UnreadMentions:       unreadMentions,
			Pinned:               pinned == 1,
			Muted:                muted == 1,
			PreviewSnippet:       p,
			Archived:             archived == 1,
			PreviewSender:        ps,
			PreviewFromMe:        previewFromMe == 1,
			PreviewStatus:        previewStatus,
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
			AdminsOnly:           adminsOnly == 1,
			NotMember:            strings.HasSuffix(jid, "@g.us") && isCommunity == 0 && knownNotMember(jid),
			IsBusiness:           errJid == nil && isBusinessChat(parsedJID, businessJIDs),
		})
	}
	// Pinned chats stay on top whatever comes in, as on the phone. The query
	// already put the chat rows' own pins first, newest pin first; this only
	// lifts the ones pinned through the settings store, keeping the rest in
	// order.
	sort.SliceStable(chats, func(i, j int) bool { return chats[i].Pinned && !chats[j].Pinned })
	return chats
}
