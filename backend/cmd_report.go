package main

import (
	"context"
	"encoding/json"
	"net"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
)

// Reporting to WhatsApp, as the official apps' "Report" does: a message
// (from its menu) or a chat (the last message they sent), optionally
// blocking the contact or exiting the group afterwards.

type ReportData struct {
	JID string `json:"jid"`
	// MessageID is the reported message; empty reports the chat.
	MessageID string `json:"message_id"`
	Block     bool   `json:"block"`
	Leave     bool   `json:"leave"`
}

// reportableMessage is what a report needs of a stored message.
type reportableMessage struct {
	id, sender, mediaType string
	ts                    int64
	raw                   []byte
}

// latestReportable is the newest message in chat that someone else sent,
// the one a chat report names.
func latestReportable(chat string) (reportableMessage, bool) {
	var m reportableMessage
	err := db.QueryRow(`SELECT id, COALESCE(sender_jid, ''), COALESCE(media_type, ''), COALESCE(timestamp, 0), raw FROM messages
		WHERE jid = ? AND from_me = 0 AND COALESCE(is_deleted, 0) = 0 AND COALESCE(media_type, '') NOT IN ('system', 'call')
		ORDER BY timestamp DESC LIMIT 1`, chat).Scan(&m.id, &m.sender, &m.mediaType, &m.ts, &m.raw)
	return m, err == nil
}

func loadReportable(chat, messageID string) (reportableMessage, bool) {
	var m reportableMessage
	var fromMe bool
	err := db.QueryRow(`SELECT id, COALESCE(sender_jid, ''), COALESCE(media_type, ''), COALESCE(timestamp, 0), raw, from_me FROM messages
		WHERE id = ? AND jid = ?`, messageID, chat).Scan(&m.id, &m.sender, &m.mediaType, &m.ts, &m.raw, &fromMe)
	return m, err == nil && !fromMe
}

// buildSpamReport describes a stored message for SendSpamReport. In a group
// the message is named by the group and its sender, with the group's name.
func buildSpamReport(chat types.JID, m reportableMessage, wholeChat bool, groupName string) whatsmeow.SpamReport {
	report := whatsmeow.SpamReport{
		MessageID:        m.id,
		MessageTimestamp: m.ts,
		From:             chat,
		RawMessage:       m.raw,
		Flow:             whatsmeow.SpamFlowMessageMenu,
	}
	switch m.mediaType {
	case "image", "video", "audio", "document", "sticker":
		report.MediaType = m.mediaType
	case "voice":
		report.MediaType = "ptt"
	}
	if chat.Server == types.GroupServer {
		if sender, err := types.ParseJID(m.sender); err == nil {
			report.Participant = sender
		}
		report.Group, report.GroupSubject = chat, groupName
		if wholeChat {
			report.Flow = whatsmeow.SpamFlowGroupInfoReport
		}
	} else if wholeChat {
		report.Flow = whatsmeow.SpamFlowContactInfo
	}
	return report
}

func handleReport(conn net.Conn, raw json.RawMessage) {
	var data ReportData
	if json.Unmarshal(raw, &data) != nil {
		return
	}
	go func() {
		fail := func(msg string) { sendEvent(conn, "error", map[string]string{"message": msg}) }
		chat, err := types.ParseJID(data.JID)
		if err != nil {
			fail("invalid jid")
			return
		}
		if chat.Server == types.NewsletterServer {
			fail("Channels are reported from WhatsApp on your phone.")
			return
		}
		var m reportableMessage
		var ok bool
		if data.MessageID != "" {
			m, ok = loadReportable(data.JID, data.MessageID)
			if !ok {
				fail("Only messages other people sent can be reported.")
				return
			}
		} else if m, ok = latestReportable(data.JID); !ok {
			fail("There's no message from them here to report.")
			return
		}
		groupName := ""
		if chat.Server == types.GroupServer {
			db.QueryRow(`SELECT COALESCE(name, '') FROM chats WHERE jid = ?`, data.JID).Scan(&groupName)
		}
		report := buildSpamReport(chat, m, data.MessageID == "", groupName)
		if _, err := wa().SendSpamReport(context.Background(), report); err != nil {
			fail("Couldn't send the report: " + err.Error())
			return
		}
		result := map[string]interface{}{"jid": data.JID, "message_id": data.MessageID}
		if data.Block && chat.Server != types.GroupServer {
			if _, err := wa().UpdateBlocklist(context.Background(), chat, events.BlocklistChangeActionBlock); err != nil {
				fail("Reported, but couldn't block them: " + err.Error())
			} else {
				setContactBlocked(data.JID, true)
				broadcastEvent("blocklist_updated", nil)
				result["blocked"] = true
			}
		}
		if data.Leave && chat.Server == types.GroupServer {
			if err := wa().LeaveGroup(context.Background(), chat); err != nil {
				fail("Reported, but couldn't exit the group: " + err.Error())
			} else {
				invalidateJoinedGroups()
				db.Exec("DELETE FROM chats WHERE jid = ?", data.JID)
				broadcastChats()
				result["left"] = true
			}
		}
		sendEvent(conn, "reported", result)
	}()
}
