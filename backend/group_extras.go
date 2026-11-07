package main

import (
	"context"
	"encoding/json"
	"net"
	"strings"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/types"
)

type JoinRequestsData struct {
	JID     string   `json:"jid"`
	JIDs    []string `json:"jids"`
	Approve bool     `json:"approve"`
}

type CommunityGroupData struct {
	Community string `json:"community"`
	Group     string `json:"group"`
}

type CreateCommunityData struct {
	Name        string `json:"name"`
	Description string `json:"description"`
}

func (acc *Account) handleGetJoinRequests(conn net.Conn, raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	requests, err := acc.wa().GetGroupRequestParticipants(context.Background(), jid)
	if err != nil {
		acc.sendEvent(conn, "error", map[string]string{"message": "Couldn't load the join requests: " + err.Error()})
		return
	}
	list := make([]map[string]interface{}, 0, len(requests))
	for _, r := range requests {
		list = append(list, map[string]interface{}{
			"jid": r.JID.String(), "name": acc.resolveDisplayName(r.JID), "requested_at": r.RequestedAt.Unix(),
		})
	}
	acc.sendEvent(conn, "join_requests", map[string]interface{}{"jid": data.JID, "requests": list})
}

func (acc *Account) handleAnswerJoinRequests(conn net.Conn, raw json.RawMessage) {
	var data JoinRequestsData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	var people []types.JID
	for _, s := range data.JIDs {
		if p, err := types.ParseJID(s); err == nil {
			people = append(people, p)
		}
	}
	if len(people) == 0 {
		return
	}
	go func() {
		action := whatsmeow.ParticipantChangeReject
		if data.Approve {
			action = whatsmeow.ParticipantChangeApprove
		}
		results, err := acc.wa().UpdateGroupRequestParticipants(context.Background(), jid, people, action)
		if err != nil {
			acc.sendEvent(conn, "error", map[string]string{"message": "Couldn't answer the join requests: " + err.Error()})
			return
		}
		failed := 0
		for _, r := range results {
			if r.Error != 0 {
				failed++
			}
		}
		acc.sendEvent(conn, "join_requests_answered", map[string]interface{}{
			"jid": data.JID, "approve": data.Approve, "count": len(people) - failed, "failed": failed,
		})
	}()
}

func (acc *Account) handleCreateCommunity(conn net.Conn, raw json.RawMessage) {
	var data CreateCommunityData
	if err := json.Unmarshal(raw, &data); err != nil || strings.TrimSpace(data.Name) == "" {
		return
	}
	go func() {
		ctx := context.Background()
		info, err := acc.wa().CreateGroup(ctx, whatsmeow.ReqCreateGroup{
			Name:        strings.TrimSpace(data.Name),
			GroupParent: types.GroupParent{IsParent: true},
		})
		if err != nil {
			acc.sendEvent(conn, "error", map[string]string{"message": "Couldn't create the community: " + err.Error()})
			return
		}
		if desc := strings.TrimSpace(data.Description); desc != "" {
			if err := acc.wa().SetGroupTopic(ctx, info.JID, "", "", desc); err != nil {
				acc.sendEvent(conn, "error", map[string]string{"message": "The community was created, but its description couldn't be set: " + err.Error()})
			}
		}
		acc.db.Exec(`INSERT INTO chats (jid, name, timestamp) VALUES (?, ?, ?)
			ON CONFLICT(jid) DO UPDATE SET name=COALESCE(NULLIF(excluded.name, ''), chats.name)`,
			info.JID.String(), info.Name, time.Now().Unix())
		acc.recordGroupCommunity(info)
		acc.markGroupJoined(info.JID.String(), true)
		acc.invalidateJoinedGroups()
		acc.populateChatsFromStore()
		acc.broadcastChats()
		acc.sendEvent(conn, "community_created", map[string]interface{}{"jid": info.JID.String(), "name": info.Name})
	}()
}

func (acc *Account) handleGetLinkableGroups(conn net.Conn, raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	groups, err := acc.wa().GetJoinedGroups(context.Background())
	if err != nil {
		acc.sendEvent(conn, "error", map[string]string{"message": "Couldn't load your groups: " + err.Error()})
		return
	}
	list := []map[string]string{}
	for _, g := range groups {
		if g.IsParent || !g.LinkedParentJID.IsEmpty() || g.IsDefaultSubGroup {
			continue
		}
		admin := false
		for _, p := range g.Participants {
			if acc.isParticipantMe(p) && (p.IsAdmin || p.IsSuperAdmin) {
				admin = true
				break
			}
		}
		if admin {
			list = append(list, map[string]string{"jid": g.JID.String(), "name": g.Name})
		}
	}
	acc.sendEvent(conn, "linkable_groups", map[string]interface{}{"community": data.JID, "groups": list})
}

func (acc *Account) handleLinkCommunityGroup(conn net.Conn, raw json.RawMessage, link bool) {
	var data CommunityGroupData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	community, err1 := types.ParseJID(data.Community)
	group, err2 := types.ParseJID(data.Group)
	if err1 != nil || err2 != nil {
		return
	}
	go func() {
		ctx := context.Background()
		var err error
		if link {
			err = acc.wa().LinkGroup(ctx, community, group)
		} else {
			err = acc.wa().UnlinkGroup(ctx, community, group)
		}
		if err != nil {
			verb := "remove the group from"
			if link {
				verb = "add the group to"
			}
			acc.sendEvent(conn, "error", map[string]string{"message": "Couldn't " + verb + " the community: " + err.Error()})
			return
		}
		if info, err := acc.wa().GetGroupInfo(ctx, group); err == nil {
			acc.recordGroupCommunity(info)
		}
		acc.invalidateJoinedGroups()
		acc.broadcastChats()
		acc.sendEvent(conn, "community_groups_changed", map[string]interface{}{"community": data.Community, "linked": link})
	}()
}

func (acc *Account) handleDeactivateCommunity(conn net.Conn, raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		if err := acc.wa().DeactivateCommunity(context.Background(), jid); err != nil {
			acc.sendEvent(conn, "error", map[string]string{"message": "Couldn't deactivate the community: " + err.Error()})
			return
		}
		acc.invalidateJoinedGroups()
		acc.populateChatsFromStore()
		acc.broadcastChats()
		acc.sendEvent(conn, "community_deactivated", map[string]interface{}{"jid": data.JID})
	}()
}
