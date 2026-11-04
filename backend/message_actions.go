package main

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"net"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waCommon"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

const (
	pinDefaultSeconds = 7 * 24 * 60 * 60
	maxPinsPerChat    = 3
)

type PinMessageData struct {
	JID       string `json:"jid"`
	MessageID string `json:"message_id"`
	Pin       bool   `json:"pin"`
	Duration  int    `json:"duration"`
}

type KeepMessageData struct {
	JID       string `json:"jid"`
	MessageID string `json:"message_id"`
	Keep      bool   `json:"keep"`
}

type SetChatPrivacyData struct {
	JID     string `json:"jid"`
	Enabled bool   `json:"enabled"`
}

func storedMessageKey(chat types.JID, messageID string) (*waCommon.MessageKey, error) {
	var senderStr string
	var fromMe bool
	if err := db.QueryRow(`SELECT COALESCE(sender_jid, ''), from_me FROM messages WHERE id = ?`, messageID).Scan(&senderStr, &fromMe); err != nil {
		return nil, fmt.Errorf("that message isn't here any more")
	}
	sender := types.EmptyJID
	if !fromMe {
		if chat.Server == types.GroupServer {
			senderStr = wireParticipantFor(messageID, senderStr)
		}
		if parsed, err := types.ParseJID(senderStr); err == nil {
			sender = parsed
		}
	}
	return wa().BuildMessageKey(chat, sender, types.MessageID(messageID)), nil
}

func ownJIDString() string {
	if wa().Store.ID == nil {
		return ""
	}
	return wa().Store.ID.ToNonAD().String()
}

func handlePinMessage(raw json.RawMessage) {
	var data PinMessageData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	chat, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		key, err := storedMessageKey(chat, data.MessageID)
		if err != nil {
			broadcastEvent("error", map[string]string{"message": "Couldn't pin: " + err.Error()})
			return
		}
		seconds := 0
		pinType := waE2E.PinInChatMessage_UNPIN_FOR_ALL
		if data.Pin {
			pinType = waE2E.PinInChatMessage_PIN_FOR_ALL
			seconds = data.Duration
			if seconds <= 0 {
				seconds = pinDefaultSeconds
			}
		}
		now := time.Now()
		msg := &waE2E.Message{
			PinInChatMessage: &waE2E.PinInChatMessage{
				Key:               key,
				Type:              pinType.Enum(),
				SenderTimestampMS: proto.Int64(now.UnixMilli()),
			},
			MessageContextInfo: &waE2E.MessageContextInfo{
				MessageAddOnDurationInSecs: proto.Uint32(uint32(seconds)),
			},
		}
		if _, err := wa().SendMessage(context.Background(), chat, msg, whatsmeow.SendRequestExtra{ID: wa().GenerateMessageID()}); err != nil {
			verb := "unpin"
			if data.Pin {
				verb = "pin"
			}
			broadcastEvent("error", map[string]string{"message": fmt.Sprintf("Couldn't %s the message: %s", verb, sendErrorText(err))})
			return
		}
		until := int64(0)
		if data.Pin {
			until = now.Unix() + int64(seconds)
		}
		own, _ := types.ParseJID(ownJIDString())
		setMessagePin(data.JID, data.MessageID, until, own, now)
	}()
}

func applyPinInChat(v *events.Message, pin *waE2E.PinInChatMessage) {
	targetID := pin.GetKey().GetID()
	if targetID == "" {
		return
	}
	chat, sender := resolveLIDChat(v)
	ts := v.Info.Timestamp
	if ms := pin.GetSenderTimestampMS(); ms > 0 {
		ts = time.UnixMilli(ms)
	}
	until := int64(0)
	if pin.GetType() == waE2E.PinInChatMessage_PIN_FOR_ALL {
		seconds := v.Message.GetMessageContextInfo().GetMessageAddOnDurationInSecs()
		if seconds == 0 {
			seconds = unwrapMessage(v.Message).GetMessageContextInfo().GetMessageAddOnDurationInSecs()
		}
		if seconds == 0 {
			seconds = pinDefaultSeconds
		}
		until = ts.Unix() + int64(seconds)
	}
	go setMessagePin(chat.String(), targetID, until, sender, ts)
}

func setMessagePin(jid, messageID string, until int64, by types.JID, ts time.Time) {
	res, err := db.Exec(`UPDATE messages SET pinned_until = ?, pinned_at = ? WHERE id = ? AND jid = ?`, until, ts.Unix(), messageID, jid)
	if err != nil {
		log.Printf("Couldn't store pin for %s: %v", messageID, err)
		return
	}
	if n, _ := res.RowsAffected(); n == 0 {
		return
	}
	if until > 0 {
		rows, err := db.Query(`SELECT id FROM messages WHERE jid = ? AND pinned_until > ? ORDER BY pinned_at DESC, id DESC LIMIT -1 OFFSET ?`,
			jid, time.Now().Unix(), maxPinsPerChat)
		if err == nil {
			var dropped []string
			for rows.Next() {
				var id string
				if rows.Scan(&id) == nil {
					dropped = append(dropped, id)
				}
			}
			rows.Close()
			for _, id := range dropped {
				db.Exec(`UPDATE messages SET pinned_until = 0 WHERE id = ?`, id)
				broadcastEvent("message_pinned", map[string]interface{}{"jid": jid, "message_id": id, "pinned": false})
			}
		}
		who := "You"
		if !isOwnJID(by) && !by.IsEmpty() {
			who = nameToken(by)
		}
		insertGroupSystemMessage(jid, fmt.Sprintf("system_pin_%s_%d", messageID, ts.Unix()), who+" pinned a message", ts)
	}
	broadcastEvent("message_pinned", map[string]interface{}{
		"jid": jid, "message_id": messageID, "pinned": until > time.Now().Unix(),
	})
}

func handleGetPinnedMessages(conn net.Conn, raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	rows, err := db.Query(`SELECT id, COALESCE(text, ''), from_me, COALESCE(sender_jid, '') FROM messages
		WHERE jid = ? AND pinned_until > ? AND COALESCE(is_deleted, 0) = 0 ORDER BY pinned_at DESC`, data.JID, time.Now().Unix())
	if err != nil {
		sendEvent(conn, "error", map[string]string{"message": "Couldn't load the pinned messages."})
		return
	}
	defer rows.Close()
	pins := []map[string]interface{}{}
	for rows.Next() {
		var id, text, sender string
		var fromMe bool
		if rows.Scan(&id, &text, &fromMe, &sender) != nil {
			continue
		}
		name := "You"
		if !fromMe {
			name = sender
		}
		pins = append(pins, map[string]interface{}{"id": id, "text": text, "sender": name, "fromMe": fromMe})
	}
	rows.Close()
	// Names are looked up with the rows closed, as that runs queries of its own.
	for _, p := range pins {
		p["text"] = renderNames(p["text"].(string), nil)
		if !p["fromMe"].(bool) {
			if parsed, err := types.ParseJID(p["sender"].(string)); err == nil {
				p["sender"] = resolveDisplayName(parsed)
			}
		}
		delete(p, "fromMe")
	}
	sendEvent(conn, "pinned_messages", map[string]interface{}{"jid": data.JID, "messages": pins})
}

func handleKeepMessage(raw json.RawMessage) {
	var data KeepMessageData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	chat, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		key, err := storedMessageKey(chat, data.MessageID)
		if err != nil {
			broadcastEvent("error", map[string]string{"message": "Couldn't keep the message: " + err.Error()})
			return
		}
		keepType := waE2E.KeepType_UNDO_KEEP_FOR_ALL
		if data.Keep {
			keepType = waE2E.KeepType_KEEP_FOR_ALL
		}
		msg := &waE2E.Message{KeepInChatMessage: &waE2E.KeepInChatMessage{
			Key:         key,
			KeepType:    keepType.Enum(),
			TimestampMS: proto.Int64(time.Now().UnixMilli()),
		}}
		if _, err := wa().SendMessage(context.Background(), chat, msg, whatsmeow.SendRequestExtra{ID: wa().GenerateMessageID()}); err != nil {
			broadcastEvent("error", map[string]string{"message": "Couldn't keep the message: " + sendErrorText(err)})
			return
		}
		setMessageKept(data.JID, data.MessageID, data.Keep)
	}()
}

func applyKeepInChat(v *events.Message, keep *waE2E.KeepInChatMessage) {
	targetID := keep.GetKey().GetID()
	if targetID == "" {
		return
	}
	chat, _ := resolveLIDChat(v)
	go setMessageKept(chat.String(), targetID, keep.GetKeepType() == waE2E.KeepType_KEEP_FOR_ALL)
}

func setMessageKept(jid, messageID string, kept bool) {
	res, err := db.Exec(`UPDATE messages SET kept = ? WHERE id = ? AND jid = ?`, boolToInt(kept), messageID, jid)
	if err != nil {
		return
	}
	if n, _ := res.RowsAffected(); n == 0 {
		return
	}
	broadcastEvent("message_kept", map[string]interface{}{"jid": jid, "message_id": messageID, "kept": kept})
}

func handleGetChatPrivacy(conn net.Conn, raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		if jid.Server == types.GroupServer {
			if info, err := wa().GetGroupInfo(context.Background(), jid); err == nil {
				setChatLimitSharing(jid.String(), info.IsLimitSharingEnabled, 0)
			}
		}
		sendEvent(conn, "chat_privacy", map[string]interface{}{
			"jid": data.JID, "enabled": chatLimitSharing(data.JID), "restricted": chatRestricted(data.JID),
		})
	}()
}

func handleSetChatPrivacy(raw json.RawMessage) {
	var data SetChatPrivacyData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		now := time.Now()
		var changed bool
		if jid.Server == types.GroupServer {
			if err := wa().SetGroupLimitSharing(context.Background(), jid, data.Enabled); err != nil {
				broadcastEvent("error", map[string]string{"message": "Couldn't change advanced chat privacy: " + err.Error()})
				return
			}
			changed = setChatLimitSharing(data.JID, data.Enabled, 0)
		} else {
			msg := &waE2E.Message{LimitSharingMessage: &waE2E.FutureProofMessage{Message: &waE2E.Message{
				ProtocolMessage: &waE2E.ProtocolMessage{
					Type: waE2E.ProtocolMessage_LIMIT_SHARING.Enum(),
					LimitSharing: &waCommon.LimitSharing{
						SharingLimited:               proto.Bool(data.Enabled),
						Trigger:                      waCommon.LimitSharing_CHAT_SETTING.Enum(),
						LimitSharingSettingTimestamp: proto.Int64(now.UnixMilli()),
						InitiatedByMe:                proto.Bool(true),
					},
				},
			}}}
			if _, err := wa().SendMessage(context.Background(), jid, msg, whatsmeow.SendRequestExtra{ID: wa().GenerateMessageID()}); err != nil {
				broadcastEvent("error", map[string]string{"message": "Couldn't change advanced chat privacy: " + sendErrorText(err)})
				return
			}
			changed = setChatLimitSharing(data.JID, data.Enabled, now.UnixMilli())
		}
		if changed {
			noteChatPrivacyChange(data.JID, types.EmptyJID, data.Enabled, now)
		}
	}()
}

func noteRestrictedChatChange(jid string, by types.JID, enabled bool, ts time.Time) {
	who := "You"
	if !by.IsEmpty() && !isOwnJID(by) {
		who = nameToken(by)
	}
	text := who + " turned off restricted chat"
	if enabled {
		text = who + " turned on restricted chat. New messages in this chat won't appear on linked devices like this one"
	}
	insertGroupSystemMessage(jid, fmt.Sprintf("system_restricted_%d", ts.UnixMilli()), text, ts)
}

func noteChatPrivacyChange(jid string, by types.JID, enabled bool, ts time.Time) {
	who := "You"
	if !by.IsEmpty() && !isOwnJID(by) {
		who = nameToken(by)
	}
	state := "off"
	if enabled {
		state = "on"
	}
	insertGroupSystemMessage(jid, fmt.Sprintf("system_privacy_%d", ts.UnixMilli()), who+" turned "+state+" advanced chat privacy", ts)
}
