package main

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"

	"net"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/types"
)

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
				checkResp, checkErr := client.IsOnWhatsApp(context.Background(), []string{phoneStr})
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
		createResp, createErr := client.CreateGroup(context.Background(), whatsmeow.ReqCreateGroup{
			Name:         groupData.Name,
			Participants: participantsJIDs,
		})
		if createErr != nil {
			sendEvent(conn, "create_group_status", map[string]interface{}{"status": "error", "error": createErr.Error()})
			return
		}
		sendEvent(conn, "create_group_status", map[string]interface{}{"status": "success", "jid": createResp.JID.String()})
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
		info, err := client.GetGroupInfo(context.Background(), jid)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}
		
		myJID := client.Store.ID.ToNonAD()
		isMyAdmin := false
		for _, p := range info.Participants {
			if p.JID.ToNonAD().String() == myJID.String() {
				if p.IsAdmin || p.IsSuperAdmin {
					isMyAdmin = true
				}
				break
			}
		}

		sendEvent(conn, "group_info", map[string]interface{}{
			"Info": info,
			"ViewerIsAdmin": isMyAdmin,
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
		if err := client.LeaveGroup(context.Background(), jid); err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
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
		link, err := client.GetGroupInviteLink(context.Background(), jid, data.Reset)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
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
		_, err := client.UpdateGroupParticipants(context.Background(), jid, participants, change)
		if err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		broadcastEvent("group_updated", map[string]interface{}{"jid": data.JID})
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
		if err := client.SetGroupName(context.Background(), jid, data.Name); err != nil {
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
		if err := client.SetGroupTopic(context.Background(), jid, "", "", data.Topic); err != nil {
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
		pictureID, err := client.SetGroupPhoto(context.Background(), jid, photoBytes)
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
		if err := client.SetGroupAnnounce(context.Background(), jid, data.Value); err != nil {
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
		if err := client.SetGroupLocked(context.Background(), jid, data.Value); err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		broadcastEvent("group_updated", map[string]interface{}{"jid": data.JID})
	}()
}
