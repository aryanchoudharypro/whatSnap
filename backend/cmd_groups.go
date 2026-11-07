package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"slices"
	"strings"
	"time"

	"net"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
)

func (acc *Account) handleCreateGroup(conn net.Conn, raw json.RawMessage) {
	var groupData CreateGroupCommandData
	if err := json.Unmarshal(raw, &groupData); err != nil {
		return
	}
	go func() {
		seenJIDs := make(map[string]bool)
		var participantsJIDs []types.JID
		for _, participant := range groupData.Participants {
			if parsedJID, parseErr := types.ParseJID(participant); parseErr == nil && parsedJID.User != "" {
				if !seenJIDs[parsedJID.String()] {
					seenJIDs[parsedJID.String()] = true
					participantsJIDs = append(participantsJIDs, parsedJID)
				}
			} else {
				var phoneDigits []rune
				for _, char := range participant {
					if char >= '0' && char <= '9' {
						phoneDigits = append(phoneDigits, char)
					}
				}
				phoneStr := string(phoneDigits)
				checkResp, checkErr := acc.wa().IsOnWhatsApp(context.Background(), []string{phoneStr})
				if checkErr != nil || len(checkResp) == 0 || !checkResp[0].IsIn {
					acc.sendEvent(conn, "create_group_status", map[string]interface{}{"status": "error", "error": fmt.Sprintf("Participant %s is not on WhatsApp", participant)})
					return
				}
				resolvedJID := checkResp[0].JID
				if !seenJIDs[resolvedJID.String()] {
					seenJIDs[resolvedJID.String()] = true
					participantsJIDs = append(participantsJIDs, resolvedJID)
				}
			}
		}
		createResp, createErr := acc.wa().CreateGroup(context.Background(), whatsmeow.ReqCreateGroup{
			Name:         groupData.Name,
			Participants: participantsJIDs,
		})
		if createErr != nil {
			acc.sendEvent(conn, "create_group_status", map[string]interface{}{"status": "error", "error": createErr.Error()})
			return
		}
		acc.sendEvent(conn, "create_group_status", map[string]interface{}{"status": "success", "jid": createResp.JID.String()})
		acc.db.Exec(`INSERT INTO chats (jid, name, timestamp) VALUES (?, ?, ?)
			ON CONFLICT(jid) DO UPDATE SET name=COALESCE(NULLIF(excluded.name, ''), chats.name)`,
			createResp.JID.String(), groupData.Name, time.Now().Unix())
		acc.markGroupJoined(createResp.JID.String(), true)
		acc.invalidateJoinedGroups()
		acc.populateChatsFromStore()
		acc.broadcastChats()
	}()
}

func (acc *Account) handleGetGroupInfo(conn net.Conn, raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		info, err := acc.wa().GetGroupInfo(context.Background(), jid)
		if err != nil {
			acc.sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}

		isMyAdmin := false
		participants := info.Participants
		if info.IsParent {
			participants = append(slices.Clone(participants), acc.communityMembers(jid, participants)...)
		}
		// WA Web lists Meta AI first when it's a member of the group
		// (WAWebParticipantListUtils), named Meta AI with no number.
		if i := slices.IndexFunc(participants, func(p types.GroupParticipant) bool {
			bot := p.JID.ToNonAD()
			return bot == types.NewMetaAIJID || bot == types.TeeMetaAIJID
		}); i > 0 {
			participants = slices.Clone(participants)
			bot := participants[i]
			copy(participants[1:i+1], participants[:i])
			participants[0] = bot
		}
		participantsList := make([]map[string]interface{}, 0, len(participants))
		acc.noteParticipantLabels(jid, info.Participants)
		labels := acc.memberLabels(jid.String())
		saved := acc.savedContactJIDs()
		for _, p := range participants {
			if acc.isParticipantMe(p) && (p.IsAdmin || p.IsSuperAdmin) {
				isMyAdmin = true
			}
		}
		canShareHistory := groupCanShareHistory(info, isMyAdmin)

		for _, p := range participants {
			phoneJID := p.PhoneNumber
			if phoneJID.IsEmpty() {
				if pn := acc.preferPN(p.JID); pn.Server == types.DefaultUserServer {
					phoneJID = pn
				}
			}

			name := acc.resolveDisplayName(p.JID)
			if !phoneJID.IsEmpty() && name == "+"+phoneJID.User {
				name = ""
			}
			if strings.HasSuffix(name, "@"+types.HiddenUserServer) {
				name = p.DisplayName
			}

			phoneStr := ""
			if !phoneJID.IsEmpty() {
				phoneStr = phoneJID.ToNonAD().String()
			}
			historyCount := 0
			if canShareHistory && !acc.isParticipantMe(p) && acc.groupHistoryMemberEligible(jid, p) {
				historyCount = acc.groupHistoryCount(jid, p)
			}
			participantsList = append(participantsList, map[string]interface{}{
				"JID":          p.JID.String(),
				"PhoneJID":     phoneStr,
				"IsAdmin":      p.IsAdmin,
				"IsSuperAdmin": p.IsSuperAdmin,
				"Name":         name,
				"Label":        labels[acc.memberLabelKey(p.JID)],
				"IsMe":         acc.isParticipantMe(p),
				"Saved":        saved[p.JID.ToNonAD().String()] || (phoneStr != "" && saved[phoneStr]),
				"HistoryCount": historyCount,
			})
		}

		acc.db.Exec("UPDATE chats SET is_admin = ?, admins_only = ? WHERE jid = ?", boolToInt(isMyAdmin), boolToInt(info.IsAnnounce), jid.String())
		acc.broadcastChats()

		acc.sendEvent(conn, "group_info", map[string]interface{}{
			"Info":          info,
			"Participants":  participantsList,
			"ViewerIsAdmin": isMyAdmin,
			"MyLabel":       labels[acc.ownVoterKey()],
		})
	}()
}

func (acc *Account) handleLeaveGroup(raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		if err := acc.wa().LeaveGroup(context.Background(), jid); err != nil {
			acc.broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		acc.invalidateJoinedGroups()
		acc.db.Exec("DELETE FROM chats WHERE jid = ?", data.JID)
		acc.broadcastChats()
	}()
}

func (acc *Account) handleLeaveAndDeleteGroup(raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		acc.pendingGroupDeletes.Store(data.JID, struct{}{})
		defer acc.pendingGroupDeletes.Delete(data.JID)

		if err := acc.wa().LeaveGroup(context.Background(), jid); err != nil {
			acc.broadcastEvent("error", map[string]string{"message": "Failed to leave group: " + err.Error()})
			return
		}
		acc.invalidateJoinedGroups()

		lastTS, lastKey := acc.getLastMessageKeyForChat(data.JID)
		if err := acc.sendAppStateWithRetry(context.Background(), acc.withChatRange(appstate.BuildDeleteChat(jid, lastTS, lastKey, false), data.JID)); err != nil {
			acc.broadcastEvent("error", map[string]string{"message": "Left the group, but failed to delete the chat: " + err.Error()})
			return
		}
		acc.purgeMessages("jid = ?", data.JID)
		acc.db.Exec("DELETE FROM chats WHERE jid = ?", data.JID)
		acc.broadcastChats()
	}()
}

func (acc *Account) handleGetGroupInviteLink(conn net.Conn, raw json.RawMessage) {
	var data GetGroupInviteLinkData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		link, err := acc.wa().GetGroupInviteLink(context.Background(), jid, data.Reset)
		switch {
		case errors.Is(err, whatsmeow.ErrGroupInviteLinkUnauthorized):
			acc.sendEvent(conn, "error", map[string]string{"message": "Only group admins can see this group's invite link."})
			return
		case err != nil:
			acc.sendEvent(conn, "error", map[string]string{"message": "Couldn't get the invite link: " + err.Error()})
			return
		}
		acc.sendEvent(conn, "group_invite_link", map[string]interface{}{"jid": data.JID, "link": link})
	}()
}

func (acc *Account) handleUpdateGroupParticipants(raw json.RawMessage) {
	var data UpdateGroupParticipantsData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	var change whatsmeow.ParticipantChange
	switch data.Action {
	case "add":
		change = whatsmeow.ParticipantChangeAdd
	case "remove":
		change = whatsmeow.ParticipantChangeRemove
	case "promote":
		change = whatsmeow.ParticipantChangePromote
	case "demote":
		change = whatsmeow.ParticipantChangeDemote
	default:
		acc.broadcastEvent("error", map[string]string{"message": "unknown participant action: " + data.Action})
		return
	}
	var participants []types.JID
	var invalid []string
	for _, p := range data.Participants {
		if parsed, perr := types.ParseJID(p); perr == nil {
			participants = append(participants, parsed)
		} else {
			invalid = append(invalid, p)
		}
	}
	if len(invalid) > 0 {
		acc.broadcastEvent("error", map[string]string{"message": "invalid participant number(s), skipped: " + strings.Join(invalid, ", ")})
	}
	if len(participants) == 0 {
		return
	}
	go func() {
		results, err := acc.wa().UpdateGroupParticipants(context.Background(), jid, participants, change)
		if err != nil {
			acc.broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		var failed []string
		var pendingInvite bool
		for _, p := range results {
			if p.Error == 0 {
				continue
			}
			if data.Action == "add" && p.AddRequest != nil {
				pendingInvite = true
				continue
			}
			failed = append(failed, p.JID.User)
		}
		switch {
		case pendingInvite && len(failed) == 0:
			acc.broadcastEvent("error", map[string]string{"message": "Some participants have privacy settings that prevent being added directly. An invite link was sent to them instead."})
		case len(failed) > 0:
			acc.broadcastEvent("error", map[string]string{"message": "Couldn't update participant(s): " + strings.Join(failed, ", ")})
		}
		acc.broadcastEvent("group_updated", map[string]interface{}{"jid": data.JID})
	}()
}

func (acc *Account) groupInviteFromMessageID(messageID string) (groupJID, inviterJID types.JID, code string, expiration int64, err error) {
	rawBytes, chatJID, senderJID, _, dbErr := acc.getMessageRaw(messageID)
	if dbErr != nil || len(rawBytes) == 0 {
		err = fmt.Errorf("message not found")
		return
	}
	var msg waE2E.Message
	if unmarshalErr := proto.Unmarshal(rawBytes, &msg); unmarshalErr != nil {
		err = fmt.Errorf("failed to parse message")
		return
	}
	invite := msg.GetGroupInviteMessage()
	if invite == nil {
		err = fmt.Errorf("message is not a group invite")
		return
	}
	groupJID, parseErr := types.ParseJID(invite.GetGroupJID())
	if parseErr != nil {
		err = fmt.Errorf("invalid group JID in invite")
		return
	}
	inviterStr := senderJID
	if inviterStr == "" {
		inviterStr = chatJID
	}
	inviterJID, parseErr = types.ParseJID(inviterStr)
	if parseErr != nil {
		err = fmt.Errorf("invalid inviter JID")
		return
	}
	code = invite.GetInviteCode()
	expiration = invite.GetInviteExpiration()
	return
}

func (acc *Account) handleGetGroupInviteInfo(conn net.Conn, raw json.RawMessage) {
	var data MessageRefData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		groupJID, inviterJID, code, expiration, err := acc.groupInviteFromMessageID(data.MessageID)
		if err != nil {
			acc.sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}
		info, err := acc.wa().GetGroupInfoFromInvite(context.Background(), groupJID, inviterJID, code, expiration)
		if err != nil {
			acc.sendEvent(conn, "error", map[string]string{"message": "This invite is no longer valid: " + err.Error()})
			return
		}
		acc.sendEvent(conn, "group_invite_info", map[string]interface{}{
			"messageId": data.MessageID,
			"jid":       info.JID.String(),
			"name":      info.Name,
			"topic":     info.Topic,
			"size":      len(info.Participants),
		})
	}()
}

func (acc *Account) handleJoinGroupInvite(raw json.RawMessage) {
	var data MessageRefData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		groupJID, inviterJID, code, expiration, err := acc.groupInviteFromMessageID(data.MessageID)
		if err != nil {
			acc.broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		switch joinErr := acc.wa().JoinGroupWithInvite(context.Background(), groupJID, inviterJID, code, expiration); {
		case errors.Is(joinErr, whatsmeow.ErrJoinRequiresApproval):
			acc.broadcastEvent("error", map[string]string{"message": "This group needs an admin to approve new members. Your request to join was sent."})
			return
		case errors.Is(joinErr, whatsmeow.ErrInviteExpired):
			acc.broadcastEvent("error", map[string]string{"message": "This group invite has expired."})
			return
		case joinErr != nil:
			acc.broadcastEvent("error", map[string]string{"message": "Couldn't join this group: " + joinErr.Error()})
			return
		}
		acc.markGroupJoined(groupJID.String(), true)
		acc.fetchGroupMetadata(groupJID)
		acc.invalidateJoinedGroups()
		acc.populateChatsFromStore()
		acc.broadcastChats()
		acc.broadcastEvent("group_invite_joined", map[string]interface{}{"jid": groupJID.String()})
	}()
}

func (acc *Account) handleGetGroupLinkInfo(conn net.Conn, raw json.RawMessage) {
	var data struct {
		Code string `json:"code"`
	}
	if err := json.Unmarshal(raw, &data); err != nil || data.Code == "" {
		return
	}
	go func() {
		info, err := acc.wa().GetGroupInfoFromLink(context.Background(), data.Code)
		switch {
		case errors.Is(err, whatsmeow.ErrInviteLinkRevoked):
			acc.sendEvent(conn, "error", map[string]string{"message": "This invite link was reset by a group admin and no longer works."})
			return
		case errors.Is(err, whatsmeow.ErrInviteLinkInvalid):
			acc.sendEvent(conn, "error", map[string]string{"message": "This invite link isn't valid."})
			return
		case err != nil:
			acc.sendEvent(conn, "error", map[string]string{"message": "Couldn't open this invite link: " + err.Error()})
			return
		}
		size := info.ParticipantCount
		if size == 0 {
			size = len(info.Participants)
		}
		acc.sendEvent(conn, "group_invite_info", map[string]interface{}{
			"code":  data.Code,
			"jid":   info.JID.String(),
			"name":  info.Name,
			"topic": info.Topic,
			"size":  size,
		})
	}()
}

func (acc *Account) handleJoinGroupLink(raw json.RawMessage) {
	var data struct {
		Code string `json:"code"`
	}
	if err := json.Unmarshal(raw, &data); err != nil || data.Code == "" {
		return
	}
	go func() {
		jid, err := acc.wa().JoinGroupWithLink(context.Background(), data.Code)
		switch {
		case errors.Is(err, whatsmeow.ErrJoinRequiresApproval):
			acc.broadcastEvent("error", map[string]string{"message": "This group needs an admin to approve new members. Your request to join was sent."})
			return
		case err != nil:
			acc.broadcastEvent("error", map[string]string{"message": "Couldn't join this group: " + err.Error()})
			return
		}
		acc.markGroupJoined(jid.String(), true)
		acc.fetchGroupMetadata(jid)
		acc.invalidateJoinedGroups()
		acc.populateChatsFromStore()
		acc.broadcastChats()
		acc.broadcastEvent("group_invite_joined", map[string]interface{}{"jid": jid.String()})
	}()
}

func (acc *Account) handleSetGroupName(raw json.RawMessage) {
	var data SetGroupNameData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		if err := acc.wa().SetGroupName(context.Background(), jid, data.Name); err != nil {
			acc.broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		acc.db.Exec("UPDATE chats SET name = ? WHERE jid = ?", data.Name, data.JID)
		acc.broadcastChats()
	}()
}

func (acc *Account) handleSetGroupTopic(raw json.RawMessage) {
	var data SetGroupTopicData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		if err := acc.wa().SetGroupTopic(context.Background(), jid, "", "", data.Topic); err != nil {
			acc.broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		acc.broadcastEvent("group_updated", map[string]interface{}{"jid": data.JID})
	}()
}

func (acc *Account) handleSetGroupPhoto(raw json.RawMessage) {
	var data SetGroupPhotoData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		photoBytes, err := readLocalFileOrDecode(data.Path, data.Data)
		if err != nil {
			acc.broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		pictureID, err := acc.wa().SetGroupPhoto(context.Background(), jid, photoBytes)
		if err != nil {
			acc.broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		acc.updateContactAvatarCache(data.JID, "", pictureID)
		acc.broadcastEvent("avatar_updated", map[string]interface{}{"jid": data.JID, "removed": false})
	}()
}

func (acc *Account) handleSetGroupAnnounce(raw json.RawMessage) {
	var data ChatBoolData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		if err := acc.wa().SetGroupAnnounce(context.Background(), jid, data.Value); err != nil {
			acc.broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		acc.broadcastEvent("group_updated", map[string]interface{}{"jid": data.JID})
	}()
}

func (acc *Account) handleSetGroupLocked(raw json.RawMessage) {
	var data ChatBoolData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		if err := acc.wa().SetGroupLocked(context.Background(), jid, data.Value); err != nil {
			acc.broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		acc.broadcastEvent("group_updated", map[string]interface{}{"jid": data.JID})
	}()
}
