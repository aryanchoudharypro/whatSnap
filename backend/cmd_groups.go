package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"slices"
	"strings"
	"sync"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
	"net"
)

var pendingGroupDeletes sync.Map

func handleCreateGroup(conn net.Conn, raw json.RawMessage) {
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
				checkResp, checkErr := wa().IsOnWhatsApp(context.Background(), []string{phoneStr})
				if checkErr != nil || len(checkResp) == 0 || !checkResp[0].IsIn {
					sendEvent(conn, "create_group_status", map[string]interface{}{"status": "error", "error": fmt.Sprintf("Participant %s is not on WhatsApp", participant)})
					return
				}
				resolvedJID := checkResp[0].JID
				if !seenJIDs[resolvedJID.String()] {
					seenJIDs[resolvedJID.String()] = true
					participantsJIDs = append(participantsJIDs, resolvedJID)
				}
			}
		}
		createResp, createErr := wa().CreateGroup(context.Background(), whatsmeow.ReqCreateGroup{
			Name:         groupData.Name,
			Participants: participantsJIDs,
		})
		if createErr != nil {
			sendEvent(conn, "create_group_status", map[string]interface{}{"status": "error", "error": createErr.Error()})
			return
		}
		sendEvent(conn, "create_group_status", map[string]interface{}{"status": "success", "jid": createResp.JID.String()})
		db.Exec(`INSERT INTO chats (jid, name, timestamp) VALUES (?, ?, ?)
			ON CONFLICT(jid) DO UPDATE SET name=COALESCE(NULLIF(excluded.name, ''), chats.name)`,
			createResp.JID.String(), groupData.Name, time.Now().Unix())
		markGroupJoined(createResp.JID.String(), true)
		invalidateJoinedGroups()
		populateChatsFromStore()
		broadcastChats()
	}()
}

func handleGetGroupInfo(conn net.Conn, raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		info, err := wa().GetGroupInfo(context.Background(), jid)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}

		isMyAdmin := false
		participants := info.Participants
		if info.IsParent {
			participants = append(slices.Clone(participants), communityMembers(jid, participants)...)
		}
		participantsList := make([]map[string]interface{}, 0, len(participants))
		noteParticipantLabels(jid, info.Participants)
		labels := memberLabels(jid.String())

		for _, p := range participants {
			phoneJID := p.PhoneNumber
			if phoneJID.IsEmpty() {
				if pn := preferPN(p.JID); pn.Server == types.DefaultUserServer {
					phoneJID = pn
				}
			}

			if isParticipantMe(p) && (p.IsAdmin || p.IsSuperAdmin) {
				isMyAdmin = true
			}

			name := resolveDisplayName(p.JID)
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
			participantsList = append(participantsList, map[string]interface{}{
				"JID":          p.JID.String(),
				"PhoneJID":     phoneStr,
				"IsAdmin":      p.IsAdmin,
				"IsSuperAdmin": p.IsSuperAdmin,
				"Name":         name,
				"Label":        labels[memberLabelKey(p.JID)],
			})
		}

		db.Exec("UPDATE chats SET is_admin = ?, admins_only = ? WHERE jid = ?", boolToInt(isMyAdmin), boolToInt(info.IsAnnounce), jid.String())
		broadcastChats()

		sendEvent(conn, "group_info", map[string]interface{}{
			"Info":          info,
			"Participants":  participantsList,
			"ViewerIsAdmin": isMyAdmin,
			"MyLabel":       labels[ownVoterKey()],
		})
	}()
}

func handleLeaveGroup(raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		if err := wa().LeaveGroup(context.Background(), jid); err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		invalidateJoinedGroups()
		db.Exec("DELETE FROM chats WHERE jid = ?", data.JID)
		broadcastChats()
	}()
}

func handleLeaveAndDeleteGroup(raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		pendingGroupDeletes.Store(data.JID, struct{}{})
		defer pendingGroupDeletes.Delete(data.JID)

		if err := wa().LeaveGroup(context.Background(), jid); err != nil {
			broadcastEvent("error", map[string]string{"message": "Failed to leave group: " + err.Error()})
			return
		}
		invalidateJoinedGroups()

		lastTS, lastKey := getLastMessageKeyForChat(data.JID)
		if err := sendAppStateWithRetry(context.Background(), withChatRange(appstate.BuildDeleteChat(jid, lastTS, lastKey, false), data.JID)); err != nil {
			broadcastEvent("error", map[string]string{"message": "Left the group, but failed to delete the chat: " + err.Error()})
			return
		}
		purgeMessages("jid = ?", data.JID)
		db.Exec("DELETE FROM chats WHERE jid = ?", data.JID)
		broadcastChats()
	}()
}

func handleGetGroupInviteLink(conn net.Conn, raw json.RawMessage) {
	var data GetGroupInviteLinkData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		link, err := wa().GetGroupInviteLink(context.Background(), jid, data.Reset)
		switch {
		case errors.Is(err, whatsmeow.ErrGroupInviteLinkUnauthorized):
			sendEvent(conn, "error", map[string]string{"message": "Only group admins can see this group's invite link."})
			return
		case err != nil:
			sendEvent(conn, "error", map[string]string{"message": "Couldn't get the invite link: " + err.Error()})
			return
		}
		sendEvent(conn, "group_invite_link", map[string]interface{}{"jid": data.JID, "link": link})
	}()
}

func handleUpdateGroupParticipants(raw json.RawMessage) {
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
		broadcastEvent("error", map[string]string{"message": "unknown participant action: " + data.Action})
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
		broadcastEvent("error", map[string]string{"message": "invalid participant number(s), skipped: " + strings.Join(invalid, ", ")})
	}
	if len(participants) == 0 {
		return
	}
	go func() {
		results, err := wa().UpdateGroupParticipants(context.Background(), jid, participants, change)
		if err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
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
			broadcastEvent("error", map[string]string{"message": "Some participants have privacy settings that prevent being added directly. An invite link was sent to them instead."})
		case len(failed) > 0:
			broadcastEvent("error", map[string]string{"message": "Couldn't update participant(s): " + strings.Join(failed, ", ")})
		}
		broadcastEvent("group_updated", map[string]interface{}{"jid": data.JID})
	}()
}

func groupInviteFromMessageID(messageID string) (groupJID, inviterJID types.JID, code string, expiration int64, err error) {
	rawBytes, chatJID, senderJID, _, dbErr := getMessageRaw(messageID)
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

func handleGetGroupInviteInfo(conn net.Conn, raw json.RawMessage) {
	var data MessageRefData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		groupJID, inviterJID, code, expiration, err := groupInviteFromMessageID(data.MessageID)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}
		info, err := wa().GetGroupInfoFromInvite(context.Background(), groupJID, inviterJID, code, expiration)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": "This invite is no longer valid: " + err.Error()})
			return
		}
		sendEvent(conn, "group_invite_info", map[string]interface{}{
			"messageId": data.MessageID,
			"jid":       info.JID.String(),
			"name":      info.Name,
			"topic":     info.Topic,
			"size":      len(info.Participants),
		})
	}()
}

func handleJoinGroupInvite(raw json.RawMessage) {
	var data MessageRefData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		groupJID, inviterJID, code, expiration, err := groupInviteFromMessageID(data.MessageID)
		if err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		switch joinErr := wa().JoinGroupWithInvite(context.Background(), groupJID, inviterJID, code, expiration); {
		case errors.Is(joinErr, whatsmeow.ErrJoinRequiresApproval):
			broadcastEvent("error", map[string]string{"message": "This group needs an admin to approve new members. Your request to join was sent."})
			return
		case errors.Is(joinErr, whatsmeow.ErrInviteExpired):
			broadcastEvent("error", map[string]string{"message": "This group invite has expired."})
			return
		case joinErr != nil:
			broadcastEvent("error", map[string]string{"message": "Couldn't join this group: " + joinErr.Error()})
			return
		}
		markGroupJoined(groupJID.String(), true)
		fetchGroupMetadata(groupJID)
		invalidateJoinedGroups()
		populateChatsFromStore()
		broadcastChats()
		broadcastEvent("group_invite_joined", map[string]interface{}{"jid": groupJID.String()})
	}()
}

func handleGetGroupLinkInfo(conn net.Conn, raw json.RawMessage) {
	var data struct {
		Code string `json:"code"`
	}
	if err := json.Unmarshal(raw, &data); err != nil || data.Code == "" {
		return
	}
	go func() {
		info, err := wa().GetGroupInfoFromLink(context.Background(), data.Code)
		switch {
		case errors.Is(err, whatsmeow.ErrInviteLinkRevoked):
			sendEvent(conn, "error", map[string]string{"message": "This invite link was reset by a group admin and no longer works."})
			return
		case errors.Is(err, whatsmeow.ErrInviteLinkInvalid):
			sendEvent(conn, "error", map[string]string{"message": "This invite link isn't valid."})
			return
		case err != nil:
			sendEvent(conn, "error", map[string]string{"message": "Couldn't open this invite link: " + err.Error()})
			return
		}
		size := info.ParticipantCount
		if size == 0 {
			size = len(info.Participants)
		}
		sendEvent(conn, "group_invite_info", map[string]interface{}{
			"code":  data.Code,
			"jid":   info.JID.String(),
			"name":  info.Name,
			"topic": info.Topic,
			"size":  size,
		})
	}()
}

func handleJoinGroupLink(raw json.RawMessage) {
	var data struct {
		Code string `json:"code"`
	}
	if err := json.Unmarshal(raw, &data); err != nil || data.Code == "" {
		return
	}
	go func() {
		jid, err := wa().JoinGroupWithLink(context.Background(), data.Code)
		switch {
		case errors.Is(err, whatsmeow.ErrJoinRequiresApproval):
			broadcastEvent("error", map[string]string{"message": "This group needs an admin to approve new members. Your request to join was sent."})
			return
		case err != nil:
			broadcastEvent("error", map[string]string{"message": "Couldn't join this group: " + err.Error()})
			return
		}
		markGroupJoined(jid.String(), true)
		fetchGroupMetadata(jid)
		invalidateJoinedGroups()
		populateChatsFromStore()
		broadcastChats()
		broadcastEvent("group_invite_joined", map[string]interface{}{"jid": jid.String()})
	}()
}

func handleSetGroupName(raw json.RawMessage) {
	var data SetGroupNameData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		if err := wa().SetGroupName(context.Background(), jid, data.Name); err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		db.Exec("UPDATE chats SET name = ? WHERE jid = ?", data.Name, data.JID)
		broadcastChats()
	}()
}

func handleSetGroupTopic(raw json.RawMessage) {
	var data SetGroupTopicData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		if err := wa().SetGroupTopic(context.Background(), jid, "", "", data.Topic); err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		broadcastEvent("group_updated", map[string]interface{}{"jid": data.JID})
	}()
}

func handleSetGroupPhoto(raw json.RawMessage) {
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
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		pictureID, err := wa().SetGroupPhoto(context.Background(), jid, photoBytes)
		if err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		updateContactAvatarCache(data.JID, "", pictureID)
		broadcastEvent("avatar_updated", map[string]interface{}{"jid": data.JID, "removed": false})
	}()
}

func handleSetGroupAnnounce(raw json.RawMessage) {
	var data ChatBoolData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		if err := wa().SetGroupAnnounce(context.Background(), jid, data.Value); err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		broadcastEvent("group_updated", map[string]interface{}{"jid": data.JID})
	}()
}

func handleSetGroupLocked(raw json.RawMessage) {
	var data ChatBoolData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		if err := wa().SetGroupLocked(context.Background(), jid, data.Value); err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		broadcastEvent("group_updated", map[string]interface{}{"jid": data.JID})
	}()
}
