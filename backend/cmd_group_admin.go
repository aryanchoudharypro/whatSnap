package main

import (
	"context"
	"encoding/json"
	"net"
	"strings"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/types"
)

// Group moderation: the settings admins manage (restricting frequently
// forwarded messages, letting members report messages to the admins,
// sharing recent history with people added), members reporting a message to
// the admins, and admins reviewing what was reported.

type GroupModerationData struct {
	JID                   string `json:"jid"`
	NoFrequentlyForwarded bool   `json:"no_frequently_forwarded"`
	AllowAdminReports     bool   `json:"allow_admin_reports"`
	HasGroupHistory       bool   `json:"group_history"`
}

// handleSetGroupModeration applies the settings that differ from the group's
// current ones, so saving the dialog doesn't resend what hasn't changed.
func handleSetGroupModeration(raw json.RawMessage) {
	var data GroupModerationData
	if json.Unmarshal(raw, &data) != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil || jid.Server != types.GroupServer {
		return
	}
	go func() {
		ctx := context.Background()
		info, err := wa().GetGroupInfo(ctx, jid)
		if err != nil {
			broadcastEvent("error", map[string]string{"message": "Couldn't read the group's settings: " + err.Error()})
			return
		}
		changes := []struct {
			current, wanted bool
			apply           func(context.Context, types.JID, bool) error
		}{
			{info.NoFrequentlyForwarded, data.NoFrequentlyForwarded, wa().SetGroupNoFrequentlyForwarded},
			{info.AllowAdminReports, data.AllowAdminReports, wa().SetGroupAllowAdminReports},
			{info.HasGroupHistory, data.HasGroupHistory, wa().SetGroupHistorySharing},
		}
		changed := false
		for _, c := range changes {
			if c.current == c.wanted {
				continue
			}
			if err := c.apply(ctx, jid, c.wanted); err != nil {
				broadcastEvent("error", map[string]string{"message": "Couldn't change a group setting: " + err.Error()})
				return
			}
			changed = true
		}
		if changed {
			broadcastEvent("group_updated", map[string]interface{}{"jid": data.JID})
		}
	}()
}

type ReportToAdminsData struct {
	JID        string   `json:"jid"`
	MessageIDs []string `json:"message_ids"`
}

// handleReportToAdmins sends messages to the group's admins to review.
func handleReportToAdmins(conn net.Conn, raw json.RawMessage) {
	var data ReportToAdminsData
	if json.Unmarshal(raw, &data) != nil {
		return
	}
	go func() {
		fail := func(msg string) { sendEvent(conn, "error", map[string]string{"message": msg}) }
		jid, err := types.ParseJID(data.JID)
		if err != nil || jid.Server != types.GroupServer {
			fail("Only group messages can be reported to admins.")
			return
		}
		ids := make([]types.MessageID, 0, len(data.MessageIDs))
		for _, id := range data.MessageIDs {
			if id != "" {
				ids = append(ids, id)
			}
		}
		if err := wa().ReportGroupMessagesToAdmins(context.Background(), jid, ids); err != nil {
			if info, ierr := wa().GetGroupInfo(context.Background(), jid); ierr == nil && !info.AllowAdminReports {
				fail("This group doesn't let members report messages to its admins.")
				return
			}
			fail("Couldn't report to the admins: " + err.Error())
			return
		}
		sendEvent(conn, "reported", map[string]interface{}{"jid": data.JID, "admins": true})
	}()
}

// reportedMessageEntry is one reported message as the admins' list shows it.
func reportedMessageEntry(chat string, report whatsmeow.ReportedGroupMessage) map[string]interface{} {
	var text, sender string
	db.QueryRow(`SELECT COALESCE(text, ''), COALESCE(sender_jid, '') FROM messages WHERE id = ? AND jid = ?`,
		report.MessageID, chat).Scan(&text, &sender)
	senderName := ""
	if j, err := types.ParseJID(sender); err == nil && !j.IsEmpty() {
		senderName = cachedDisplayName(j)
	}
	reporters := make([]string, 0, len(report.Reporters))
	for _, r := range report.Reporters {
		who := r.PhoneNumber
		if who.IsEmpty() {
			who = r.JID
		}
		name := cachedDisplayName(who)
		if strings.HasSuffix(name, "@"+types.HiddenUserServer) && r.Username != "" {
			name = r.Username
		}
		reporters = append(reporters, name)
	}
	return map[string]interface{}{
		"message_id":  report.MessageID,
		"text":        text,
		"sender_name": senderName,
		"reporters":   reporters,
		"stored":      text != "",
	}
}

// handleGetReportedMessages lists the messages members reported to the
// group's admins (only admins can read it).
func handleGetReportedMessages(conn net.Conn, raw json.RawMessage) {
	var data JIDData
	if json.Unmarshal(raw, &data) != nil {
		return
	}
	go func() {
		jid, err := types.ParseJID(data.JID)
		if err != nil || jid.Server != types.GroupServer {
			return
		}
		reports, err := wa().GetReportedGroupMessages(context.Background(), jid)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": "Couldn't load reported messages: " + err.Error()})
			return
		}
		entries := make([]map[string]interface{}, 0, len(reports))
		for _, r := range reports {
			entries = append(entries, reportedMessageEntry(data.JID, r))
		}
		sendEvent(conn, "reported_messages", map[string]interface{}{"jid": data.JID, "reports": entries})
	}()
}
