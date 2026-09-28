package main

import (
	"context"
	"encoding/json"
	"log"
	"net"
	"strings"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
)

// Group member tags: the short label each member sets for themselves in a
// group, shown next to their name. Group info carries everyone's current
// tag; a member changing theirs sends the group a protocol message. The
// newest wins, kept in group_member_labels by group and member.

// memberLabelKey names a member the same way however they're addressed
// (phone number or LID, any device).
func memberLabelKey(jid types.JID) string {
	return preferPN(jid.ToNonAD()).String()
}

func storeMemberLabel(group, member, label string, setAt int64) {
	if group == "" || member == "" {
		return
	}
	if _, err := db.Exec(`INSERT INTO group_member_labels (group_jid, member, label, set_at) VALUES (?, ?, ?, ?)
		ON CONFLICT(group_jid, member) DO UPDATE SET label=excluded.label, set_at=excluded.set_at
		WHERE excluded.set_at >= group_member_labels.set_at`,
		group, member, strings.TrimSpace(label), setAt); err != nil {
		log.Printf("storing member tag in %s failed: %v", group, err)
	}
}

// memberLabels is every stored tag in a group, by member.
func memberLabels(group string) map[string]string {
	labels := map[string]string{}
	rows, err := db.Query(`SELECT member, label FROM group_member_labels WHERE group_jid = ? AND label != ''`, group)
	if err != nil {
		return labels
	}
	defer rows.Close()
	for rows.Next() {
		var member, label string
		if rows.Scan(&member, &label) == nil {
			labels[member] = label
		}
	}
	return labels
}

// noteParticipantLabels records the tags group info reports.
func noteParticipantLabels(group types.JID, participants []types.GroupParticipant) {
	for _, p := range participants {
		if p.LabelSetAt.IsZero() && p.Label == "" {
			continue
		}
		storeMemberLabel(group.String(), memberLabelKey(p.JID), p.Label, p.LabelSetAt.Unix())
	}
}

// applyMemberLabelChange handles a member changing their tag.
func applyMemberLabelChange(v *events.Message) {
	label := unwrapMessage(v.Message).GetProtocolMessage().GetMemberLabel()
	chat, sender := resolveLIDChat(v)
	if chat.Server != types.GroupServer {
		return
	}
	setAt := label.GetLabelTimestamp()
	if setAt == 0 {
		setAt = v.Info.Timestamp.Unix()
	}
	member := memberLabelKey(sender)
	storeMemberLabel(chat.String(), member, label.GetLabel(), setAt)
	broadcastEvent("member_label", map[string]interface{}{
		"jid": chat.String(), "member": member, "label": memberLabels(chat.String())[member],
	})
}

type SetMemberLabelData struct {
	JID   string `json:"jid"`
	Label string `json:"label"`
}

// handleSetMemberLabel sets (or, empty, clears) our own tag in a group.
func handleSetMemberLabel(conn net.Conn, raw json.RawMessage) {
	var data SetMemberLabelData
	if json.Unmarshal(raw, &data) != nil {
		return
	}
	go func() {
		fail := func(msg string) { sendEvent(conn, "error", map[string]string{"message": msg}) }
		group, err := types.ParseJID(data.JID)
		if err != nil || group.Server != types.GroupServer {
			fail("Member tags are only for groups.")
			return
		}
		label := strings.TrimSpace(data.Label)
		if _, err := wa().SendMessage(context.Background(), group, wa().BuildGroupMemberLabel(label), whatsmeow.SendRequestExtra{}); err != nil {
			fail("Couldn't change your tag: " + err.Error())
			return
		}
		if me := ownVoterKey(); me != "" {
			storeMemberLabel(data.JID, me, label, time.Now().Unix())
			broadcastEvent("member_label", map[string]interface{}{"jid": data.JID, "member": me, "label": label, "mine": true})
		}
	}()
}
