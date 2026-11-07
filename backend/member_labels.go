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

func (acc *Account) memberLabelKey(jid types.JID) string {
	return acc.preferPN(jid.ToNonAD()).String()
}

func (acc *Account) storeMemberLabel(group, member, label string, setAt int64) {
	if group == "" || member == "" {
		return
	}
	if _, err := acc.db.Exec(`INSERT INTO group_member_labels (group_jid, member, label, set_at) VALUES (?, ?, ?, ?)
		ON CONFLICT(group_jid, member) DO UPDATE SET label=excluded.label, set_at=excluded.set_at
		WHERE excluded.set_at >= group_member_labels.set_at`,
		group, member, strings.TrimSpace(label), setAt); err != nil {
		log.Printf("storing member tag in %s failed: %v", group, err)
	}
}

func (acc *Account) memberLabels(group string) map[string]string {
	labels := map[string]string{}
	rows, err := acc.db.Query(`SELECT member, label FROM group_member_labels WHERE group_jid = ? AND label != ''`, group)
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

func (acc *Account) noteParticipantLabels(group types.JID, participants []types.GroupParticipant) {
	for _, p := range participants {
		if p.LabelSetAt.IsZero() && p.Label == "" {
			continue
		}
		acc.storeMemberLabel(group.String(), acc.memberLabelKey(p.JID), p.Label, p.LabelSetAt.Unix())
	}
}

func (acc *Account) applyMemberLabelChange(v *events.Message) {
	label := unwrapMessage(v.Message).GetProtocolMessage().GetMemberLabel()
	chat, sender := acc.resolveLIDChat(v)
	if chat.Server != types.GroupServer {
		return
	}
	setAt := label.GetLabelTimestamp()
	if setAt == 0 {
		setAt = v.Info.Timestamp.Unix()
	}
	member := acc.memberLabelKey(sender)
	acc.storeMemberLabel(chat.String(), member, label.GetLabel(), setAt)
	acc.broadcastEvent("member_label", map[string]interface{}{
		"jid": chat.String(), "member": member, "label": acc.memberLabels(chat.String())[member],
	})
}

type SetMemberLabelData struct {
	JID   string `json:"jid"`
	Label string `json:"label"`
}

func (acc *Account) handleSetMemberLabel(conn net.Conn, raw json.RawMessage) {
	var data SetMemberLabelData
	if json.Unmarshal(raw, &data) != nil {
		return
	}
	go func() {
		fail := func(msg string) { acc.sendEvent(conn, "error", map[string]string{"message": msg}) }
		group, err := types.ParseJID(data.JID)
		if err != nil || group.Server != types.GroupServer {
			fail("Member tags are only for groups.")
			return
		}
		label := strings.TrimSpace(data.Label)
		if _, err := acc.wa().SendMessage(context.Background(), group, acc.wa().BuildGroupMemberLabel(label), whatsmeow.SendRequestExtra{}); err != nil {
			fail("Couldn't change your tag: " + err.Error())
			return
		}
		if me := acc.ownVoterKey(); me != "" {
			acc.storeMemberLabel(data.JID, me, label, time.Now().Unix())
			acc.broadcastEvent("member_label", map[string]interface{}{"jid": data.JID, "member": me, "label": label, "mine": true})
		}
	}()
}
