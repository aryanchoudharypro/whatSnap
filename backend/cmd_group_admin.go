package main

import (
	"context"
	"encoding/json"
	"net"
	"strings"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/types"
)

type GroupModerationData struct {
	JID                   string `json:"jid"`
	NoFrequentlyForwarded bool   `json:"no_frequently_forwarded"`
	AllowAdminReports     bool   `json:"allow_admin_reports"`
	HasGroupHistory       bool   `json:"group_history"`
	AdminsAddOnly         *bool  `json:"admins_add_only"`
	ApproveMembers        *bool  `json:"approve_members"`
}

func (acc *Account) handleSetGroupModeration(raw json.RawMessage) {
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
		info, err := acc.wa().GetGroupInfo(ctx, jid)
		if err != nil {
			acc.broadcastEvent("error", map[string]string{"message": "Couldn't read the group's settings: " + err.Error()})
			return
		}
		changes := []struct {
			current, wanted bool
			apply           func(context.Context, types.JID, bool) error
		}{
			{info.NoFrequentlyForwarded, data.NoFrequentlyForwarded, acc.wa().SetGroupNoFrequentlyForwarded},
			{info.AllowAdminReports, data.AllowAdminReports, acc.wa().SetGroupAllowAdminReports},
			{info.HasGroupHistory, data.HasGroupHistory, acc.wa().SetGroupHistorySharing},
		}
		if data.ApproveMembers != nil {
			changes = append(changes, struct {
				current, wanted bool
				apply           func(context.Context, types.JID, bool) error
			}{info.IsJoinApprovalRequired, *data.ApproveMembers, acc.wa().SetGroupJoinApprovalMode})
		}
		if data.AdminsAddOnly != nil {
			changes = append(changes, struct {
				current, wanted bool
				apply           func(context.Context, types.JID, bool) error
			}{info.MemberAddMode == types.GroupMemberAddModeAdmin, *data.AdminsAddOnly, func(ctx context.Context, jid types.JID, adminsOnly bool) error {
				mode := types.GroupMemberAddModeAllMember
				if adminsOnly {
					mode = types.GroupMemberAddModeAdmin
				}
				return acc.wa().SetGroupMemberAddMode(ctx, jid, mode)
			}})
		}
		changed := false
		for _, c := range changes {
			if c.current == c.wanted {
				continue
			}
			if err := c.apply(ctx, jid, c.wanted); err != nil {
				acc.broadcastEvent("error", map[string]string{"message": "Couldn't change a group setting: " + err.Error()})
				return
			}
			changed = true
		}
		if changed {
			acc.broadcastEvent("group_updated", map[string]interface{}{"jid": data.JID})
		}
	}()
}

type ReportToAdminsData struct {
	JID        string   `json:"jid"`
	MessageIDs []string `json:"message_ids"`
}

func (acc *Account) handleReportToAdmins(conn net.Conn, raw json.RawMessage) {
	var data ReportToAdminsData
	if json.Unmarshal(raw, &data) != nil {
		return
	}
	go func() {
		fail := func(msg string) { acc.sendEvent(conn, "error", map[string]string{"message": msg}) }
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
		if err := acc.wa().ReportGroupMessagesToAdmins(context.Background(), jid, ids); err != nil {
			if info, ierr := acc.wa().GetGroupInfo(context.Background(), jid); ierr == nil && !info.AllowAdminReports {
				fail("This group doesn't let members report messages to its admins.")
				return
			}
			fail("Couldn't report to the admins: " + err.Error())
			return
		}
		acc.sendEvent(conn, "reported", map[string]interface{}{"jid": data.JID, "admins": true})
	}()
}

func (acc *Account) reportedMessageEntry(chat string, report whatsmeow.ReportedGroupMessage) map[string]interface{} {
	var text, sender string
	acc.db.QueryRow(`SELECT COALESCE(text, ''), COALESCE(sender_jid, '') FROM messages WHERE id = ? AND jid = ?`,
		report.MessageID, chat).Scan(&text, &sender)
	senderName := ""
	if j, err := types.ParseJID(sender); err == nil && !j.IsEmpty() {
		senderName = acc.cachedDisplayName(j)
	}
	reporters := make([]string, 0, len(report.Reporters))
	for _, r := range report.Reporters {
		who := r.PhoneNumber
		if who.IsEmpty() {
			who = r.JID
		}
		name := acc.cachedDisplayName(who)
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

func (acc *Account) handleGetReportedMessages(conn net.Conn, raw json.RawMessage) {
	var data JIDData
	if json.Unmarshal(raw, &data) != nil {
		return
	}
	go func() {
		jid, err := types.ParseJID(data.JID)
		if err != nil || jid.Server != types.GroupServer {
			return
		}
		reports, err := acc.wa().GetReportedGroupMessages(context.Background(), jid)
		if err != nil {
			acc.sendEvent(conn, "error", map[string]string{"message": "Couldn't load reported messages: " + err.Error()})
			return
		}
		entries := make([]map[string]interface{}, 0, len(reports))
		for _, r := range reports {
			entries = append(entries, acc.reportedMessageEntry(data.JID, r))
		}
		acc.sendEvent(conn, "reported_messages", map[string]interface{}{"jid": data.JID, "reports": entries})
	}()
}
