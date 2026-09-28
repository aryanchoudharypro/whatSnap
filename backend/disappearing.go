package main

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"net"
	"strings"
	"time"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

// Disappearing messages. A chat's timer lives in chats.ephemeral_expiration
// (seconds, 0 = off), learned from history sync, the group metadata and the
// EPHEMERAL_SETTING message a 1:1 chat's toggle travels as. Every message
// sent into such a chat carries the timer in its ContextInfo - the official
// apps rely on it, and a message without one simply never disappears for the
// people receiving it. Each message that carries a timer, ours or theirs,
// gets messages.expires_at, and the sweep below deletes it locally once that
// passes, as the official apps do.

// disappearingSweepEvery is how often expired messages are removed.
const disappearingSweepEvery = time.Minute

func init() {
	go func() {
		ticker := time.NewTicker(disappearingSweepEvery)
		defer ticker.Stop()
		for range ticker.C {
			if db != nil {
				expireDisappearingMessages()
			}
		}
	}()
}

// disappearingDurationText names a timer the way the official apps do.
func disappearingDurationText(seconds uint32) string {
	switch {
	case seconds == 0:
		return "off"
	case seconds%86400 == 0:
		if days := seconds / 86400; days == 1 {
			return "24 hours"
		} else {
			return fmt.Sprintf("%d days", days)
		}
	case seconds%3600 == 0:
		if hours := seconds / 3600; hours == 1 {
			return "1 hour"
		} else {
			return fmt.Sprintf("%d hours", hours)
		}
	default:
		return fmt.Sprintf("%d seconds", seconds)
	}
}

// disappearingChangeText is the chat line for a timer change made by who.
func disappearingChangeText(who string, on bool, seconds uint32) string {
	if !on || seconds == 0 {
		return fmt.Sprintf("%s turned off disappearing messages.", who)
	}
	return fmt.Sprintf("%s turned on disappearing messages. New messages will disappear from this chat %s after they're sent.",
		who, disappearingDurationText(seconds))
}

// setChatDisappearing records a chat's timer. settingTS (unix seconds, 0 when
// unknown) keeps an older setting from undoing a newer one. Returns whether
// the stored timer changed.
func setChatDisappearing(jid string, seconds uint32, settingTS int64) bool {
	var current uint32
	var currentTS int64
	err := db.QueryRow(`SELECT COALESCE(ephemeral_expiration, 0), COALESCE(ephemeral_setting_ts, 0) FROM chats WHERE jid = ?`, jid).
		Scan(&current, &currentTS)
	if err == nil && settingTS > 0 && currentTS > settingTS {
		return false
	}
	if err == nil && current == seconds {
		if settingTS > currentTS {
			db.Exec(`UPDATE chats SET ephemeral_setting_ts = ? WHERE jid = ?`, settingTS, jid)
		}
		return false
	}
	if _, execErr := db.Exec(`INSERT INTO chats (jid, timestamp, ephemeral_expiration, ephemeral_setting_ts) VALUES (?, 0, ?, ?)
		ON CONFLICT(jid) DO UPDATE SET ephemeral_expiration = excluded.ephemeral_expiration,
			ephemeral_setting_ts = MAX(COALESCE(chats.ephemeral_setting_ts, 0), excluded.ephemeral_setting_ts)`,
		jid, seconds, settingTS); execErr != nil {
		log.Println("Error storing disappearing timer:", execErr)
		return false
	}
	broadcastEvent("chat_disappearing", map[string]interface{}{"jid": jid, "seconds": seconds})
	return true
}

// chatDisappearing is a chat's current timer and when it was set.
func chatDisappearing(jid string) (seconds uint32, settingTS int64) {
	db.QueryRow(`SELECT COALESCE(ephemeral_expiration, 0), COALESCE(ephemeral_setting_ts, 0) FROM chats WHERE jid = ?`, jid).
		Scan(&seconds, &settingTS)
	return
}

// applyDisappearingTimer stamps an outgoing chat message with the chat's
// timer. A plain Conversation has no ContextInfo, so it becomes an
// ExtendedTextMessage when a timer applies. Returns msg for chaining.
func applyDisappearingTimer(chat types.JID, msg *waE2E.Message) *waE2E.Message {
	if msg == nil || (chat.Server != types.DefaultUserServer && chat.Server != types.GroupServer && chat.Server != types.HiddenUserServer) {
		return msg
	}
	seconds, settingTS := chatDisappearing(chat.String())
	if seconds == 0 {
		return msg
	}
	if msg.Conversation != nil {
		text := msg.GetConversation()
		msg.Conversation = nil
		msg.ExtendedTextMessage = &waE2E.ExtendedTextMessage{Text: proto.String(text)}
	}
	ctx := getContextInfo(msg)
	if ctx == nil {
		ctx = &waE2E.ContextInfo{}
		if !setContextInfo(msg, ctx) {
			return msg
		}
	}
	ctx.Expiration = proto.Uint32(seconds)
	if settingTS > 0 {
		ctx.EphemeralSettingTimestamp = proto.Int64(settingTS)
	}
	return msg
}

// noteMessageExpiry sets when a stored message disappears, if it carries a
// disappearing timer.
func noteMessageExpiry(ex sqlExecer, id string, ts int64, msg *waE2E.Message) {
	seconds := getContextInfo(msg).GetExpiration()
	if seconds == 0 || id == "" || ts <= 0 {
		return
	}
	if _, err := ex.Exec(`UPDATE messages SET expires_at = ? WHERE id = ?`, ts+int64(seconds), id); err != nil {
		log.Println("Error storing message expiry:", err)
	}
}

// adoptChatDisappearing learns a 1:1 chat's timer from a message that carries
// one - the only way to find out about a timer that was turned on before this
// device was linked or before whaNTV tracked it. A message stamped with its
// setting's time is applied if that's newer than ours; one without only fills
// in a timer we've never known. Groups get theirs from the server instead.
func adoptChatDisappearing(chat types.JID, msg *waE2E.Message) {
	if chat.Server != types.DefaultUserServer {
		return
	}
	ctx := getContextInfo(msg)
	seconds := ctx.GetExpiration()
	if seconds == 0 {
		return
	}
	settingTS := ctx.GetEphemeralSettingTimestamp()
	if settingTS == 0 {
		if current, currentTS := chatDisappearing(chat.String()); current != 0 || currentTS != 0 {
			return
		}
	}
	setChatDisappearing(chat.String(), seconds, settingTS)
}

// expireDisappearingMessages deletes the messages whose timer has run out.
// Starred ones stay, as the official apps keep them.
func expireDisappearingMessages() {
	now := time.Now().Unix()
	rows, err := db.Query(`SELECT id, jid FROM messages WHERE expires_at > 0 AND expires_at <= ? AND COALESCE(starred, 0) = 0`, now)
	if err != nil {
		return
	}
	type expired struct{ id, jid string }
	var gone []expired
	for rows.Next() {
		var e expired
		if rows.Scan(&e.id, &e.jid) == nil {
			gone = append(gone, e)
		}
	}
	rows.Close()
	if len(gone) == 0 {
		return
	}
	purgeMessages(`expires_at > 0 AND expires_at <= ? AND COALESCE(starred, 0) = 0`, now)
	chats := map[string]bool{}
	for _, e := range gone {
		chats[e.jid] = true
		broadcastEvent("message_deleted", map[string]interface{}{
			"jid": e.jid, "message_id": e.id, "by": "", "for_everyone": false,
		})
	}
	for jid := range chats {
		UpdateChatSnippet(jid)
	}
	broadcastChats()
	log.Printf("Removed %d disappearing messages from %d chats", len(gone), len(chats))
}

// applyEphemeralSettingMessage handles the EPHEMERAL_SETTING protocol message
// a 1:1 chat's disappearing toggle travels as (ours from another device, or
// the other person's). Groups announce theirs through group notifications,
// which GroupInfo turns into the chat line.
func applyEphemeralSettingMessage(v *events.Message, pm *waE2E.ProtocolMessage) {
	chat, sender := resolveLIDChat(v)
	jid := chat.String()
	settingTS := pm.GetEphemeralSettingTimestamp()
	if settingTS == 0 {
		settingTS = v.Info.Timestamp.Unix()
	}
	seconds := pm.GetEphemeralExpiration()
	if !setChatDisappearing(jid, seconds, settingTS) || chat.Server == types.GroupServer {
		return
	}
	who := "You"
	if !v.Info.IsFromMe {
		who = nameToken(sender)
	}
	insertGroupSystemMessage(jid, fmt.Sprintf("system_%s_%d_ephemeral", chat.User, v.Info.Timestamp.UnixNano()),
		disappearingChangeText(who, seconds > 0, seconds), v.Info.Timestamp)
}

// handleSetDisappearingTimer turns a chat's disappearing messages on or off
// ("seconds": 0 for off; WhatsApp accepts 86400, 604800 and 7776000).
func handleSetDisappearingTimer(conn net.Conn, raw json.RawMessage) {
	var data struct {
		JID     string `json:"jid"`
		Seconds uint32 `json:"seconds"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		now := time.Now()
		if err := client.SetDisappearingTimer(context.Background(), jid, time.Duration(data.Seconds)*time.Second, now); err != nil {
			msg := err.Error()
			if strings.Contains(msg, "not-authorized") || strings.Contains(msg, "403") {
				msg = "Only group admins can change disappearing messages in this group."
			}
			sendEvent(conn, "error", map[string]string{"message": "Couldn't change disappearing messages: " + msg})
			return
		}
		if setChatDisappearing(jid.String(), data.Seconds, now.Unix()) && jid.Server != types.GroupServer {
			insertGroupSystemMessage(jid.String(), fmt.Sprintf("system_%s_%d_ephemeral", jid.User, now.UnixNano()),
				disappearingChangeText("You", data.Seconds > 0, data.Seconds), now)
		}
	}()
}

// handleGetDisappearingTimer answers with a chat's current timer.
func handleGetDisappearingTimer(conn net.Conn, raw json.RawMessage) {
	var data struct {
		JID string `json:"jid"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	seconds, _ := chatDisappearing(data.JID)
	sendEvent(conn, "chat_disappearing", map[string]interface{}{"jid": data.JID, "seconds": seconds})
}
