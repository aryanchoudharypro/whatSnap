package main

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"net"
	"time"

	"go.mau.fi/whatsmeow/proto/waCommon"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/proto/waWeb"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
)

// WA Web's group history limits (abprops group_history_new_user_threshold_secs,
// group_history_messages_time_limit_secs, group_history_message_count_limit).
const (
	groupHistoryNewMemberWindow = 30 * 24 * time.Hour
	groupHistoryMessagesWindow  = 30 * 24 * time.Hour
	groupHistoryMaxMessages     = 100
)

// sentGroupHistory remembers "group|member" pairs we sent history to this
// run, until the server's group_history_sent attribute catches up.

// groupCanShareHistory is WA Web's isPostJoinHistoryGroupStructurallyEligible:
// not a community itself, and you're an admin or the group lets every
// member share history.
func groupCanShareHistory(info *types.GroupInfo, viewerAdmin bool) bool {
	if info.IsParent {
		return false
	}
	return viewerAdmin || info.MemberShareHistoryMode == types.GroupMemberShareHistoryModeAllMember
}

// groupHistoryMemberEligible is WA Web's getParticipantPostJoinEligibility:
// not a bot, joined in the last 30 days and hasn't had history yet.
func (acc *Account) groupHistoryMemberEligible(group types.JID, p types.GroupParticipant) bool {
	if p.JID.Server == types.BotServer || p.JoinTime.IsZero() || p.HistorySent == "true" {
		return false
	}
	if _, sent := acc.sentGroupHistory.Load(group.String() + "|" + p.JID.String()); sent {
		return false
	}
	return time.Since(p.JoinTime) <= groupHistoryNewMemberWindow
}

type groupHistoryRow struct {
	id, sender, wireSender string
	fromMe                 bool
	timestamp              int64
	raw                    []byte
}

// groupHistoryMessages picks what WA Web's retrieveMessagesForBundle would:
// sent or received messages of a supported type from the 30 days before the
// member joined, newest first, at most 100.
func (acc *Account) groupHistoryMessages(group types.JID, joined time.Time) []groupHistoryRow {
	rows, err := acc.db.Query(`SELECT id, from_me, timestamp, COALESCE(sender_jid, ''), COALESCE(wire_sender, ''), raw
		FROM messages
		WHERE jid = ? AND timestamp >= ? AND timestamp <= ?
			AND raw IS NOT NULL AND length(raw) > 0
			AND COALESCE(is_deleted, 0) = 0 AND COALESCE(view_once, 0) = 0
			AND COALESCE(status, '') NOT IN ('failed', 'pending', 'uploading')
			AND (COALESCE(expires_at, 0) = 0 OR expires_at > ?)
		ORDER BY timestamp DESC LIMIT 400`,
		group.String(), joined.Add(-groupHistoryMessagesWindow).Unix(), joined.Unix(), time.Now().Unix())
	if err != nil {
		return nil
	}
	var all []groupHistoryRow
	for rows.Next() {
		var r groupHistoryRow
		if rows.Scan(&r.id, &r.fromMe, &r.timestamp, &r.sender, &r.wireSender, &r.raw) == nil {
			all = append(all, r)
		}
	}
	rows.Close()
	picked := make([]groupHistoryRow, 0, min(len(all), groupHistoryMaxMessages))
	for _, r := range all {
		msg := &waE2E.Message{}
		if proto.Unmarshal(r.raw, msg) != nil || !groupHistorySupported(msg) {
			continue
		}
		picked = append(picked, r)
		if len(picked) == groupHistoryMaxMessages {
			break
		}
	}
	return picked
}

// groupHistorySupported is WA Web's WAWebGroupHistorySupportedMessageTypesUtil.
func groupHistorySupported(msg *waE2E.Message) bool {
	if inner := msg.GetEphemeralMessage().GetMessage(); inner != nil {
		msg = inner
	}
	if inner := msg.GetDocumentWithCaptionMessage().GetMessage(); inner != nil {
		msg = inner
	}
	return msg.Conversation != nil || msg.ExtendedTextMessage != nil || msg.ImageMessage != nil ||
		msg.AudioMessage != nil || msg.VideoMessage != nil || msg.PtvMessage != nil ||
		msg.ContactMessage != nil || msg.ContactsArrayMessage != nil || msg.LocationMessage != nil ||
		msg.DocumentMessage != nil || msg.StickerMessage != nil || msg.GroupInviteMessage != nil ||
		msg.PollCreationMessage != nil || msg.PollCreationMessageV2 != nil || msg.PollCreationMessageV3 != nil ||
		msg.EventMessage != nil || msg.AlbumMessage != nil
}

// groupHistoryCount is how many messages a new member would get; 0 hides
// "Send message history" like WA Web's no_available_messages.
func (acc *Account) groupHistoryCount(group types.JID, p types.GroupParticipant) int {
	return len(acc.groupHistoryMessages(group, p.JoinTime))
}

func (acc *Account) handleSendGroupHistory(conn net.Conn, raw json.RawMessage) {
	var data struct {
		JID         string `json:"jid"`
		Participant string `json:"participant"`
	}
	if json.Unmarshal(raw, &data) != nil {
		return
	}
	group, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	fail := func(message string) {
		acc.sendEvent(conn, "group_history_result", map[string]interface{}{"jid": data.JID, "ok": false, "message": message})
	}
	go func() {
		ctx := context.Background()
		info, err := acc.wa().GetGroupInfo(ctx, group)
		if err != nil {
			fail("Couldn't load the group: " + err.Error())
			return
		}
		viewerAdmin := false
		var member *types.GroupParticipant
		for i, p := range info.Participants {
			if acc.isParticipantMe(p) {
				viewerAdmin = p.IsAdmin || p.IsSuperAdmin
			}
			if p.JID.String() == data.Participant {
				member = &info.Participants[i]
			}
		}
		if member == nil {
			fail("They aren't in this group anymore.")
			return
		}
		if !groupCanShareHistory(info, viewerAdmin) || !acc.groupHistoryMemberEligible(group, *member) {
			fail("Message history can't be sent to them anymore. It's only for people who joined in the last 30 days and haven't had it yet.")
			return
		}
		rows := acc.groupHistoryMessages(group, member.JoinTime)
		if len(rows) == 0 {
			fail("There are no messages from before they joined to send.")
			return
		}
		me := acc.wa().Store.GetLID().ToNonAD()
		if me.IsEmpty() {
			me = acc.wa().Store.GetJID().ToNonAD()
		}
		infos := make([]*waWeb.WebMessageInfo, 0, len(rows))
		for _, r := range rows {
			msg := &waE2E.Message{}
			if proto.Unmarshal(r.raw, msg) != nil {
				continue
			}
			participant := r.wireSender
			if participant == "" {
				participant = r.sender
			}
			if r.fromMe {
				participant = me.String()
			}
			infos = append(infos, &waWeb.WebMessageInfo{
				Key: &waCommon.MessageKey{
					RemoteJID:   proto.String(group.String()),
					FromMe:      proto.Bool(r.fromMe),
					ID:          proto.String(r.id),
					Participant: proto.String(participant),
				},
				Message:          msg,
				MessageTimestamp: proto.Uint64(uint64(r.timestamp)),
			})
		}
		receiver := member.JID
		if !member.LID.IsEmpty() {
			receiver = member.LID
		}
		if err := acc.wa().SendGroupHistory(ctx, group, []types.JID{receiver}, nil, infos); err != nil {
			fail("Couldn't send the message history: " + err.Error())
			return
		}
		acc.sentGroupHistory.Store(group.String()+"|"+member.JID.String(), true)
		acc.sendEvent(conn, "group_history_result", map[string]interface{}{"jid": data.JID, "ok": true, "count": len(infos)})
	}()
}

func (acc *Account) handleGetSecurityCode(conn net.Conn, raw json.RawMessage) {
	var data JIDData
	if json.Unmarshal(raw, &data) != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		code, err := acc.wa().GetSecurityCode(context.Background(), jid)
		if err != nil {
			acc.sendEvent(conn, "security_code", map[string]interface{}{"jid": data.JID, "error": err.Error()})
			return
		}
		acc.sendEvent(conn, "security_code", map[string]interface{}{
			"jid":    data.JID,
			"digits": code.Digits,
			"qr":     hex.EncodeToString(code.QR),
		})
	}()
}
