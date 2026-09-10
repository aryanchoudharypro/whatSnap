package main

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"time"

	"net"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
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
		// whatsmeow's local joined-groups store (what populateChatsFromStore
		// reads from) doesn't reliably reflect a group we just created
		// ourselves until a later JoinedGroup/GroupInfo event syncs it - for
		// a self-created group that can take until the next message arrives,
		// leaving it out of the chat list and then showing its raw jid
		// instead of its name once it does appear. Insert it directly with
		// the name we already have so it shows up correctly right away.
		db.Exec(`INSERT INTO chats (jid, name, timestamp) VALUES (?, ?, ?)
			ON CONFLICT(jid) DO UPDATE SET name=COALESCE(NULLIF(excluded.name, ''), chats.name)`,
			createResp.JID.String(), groupData.Name, time.Now().Unix())
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
		participantsList := make([]map[string]interface{}, 0, len(info.Participants))

		for _, p := range info.Participants {
			// p.JID is whichever address WhatsApp uses to identify this
			// participant in this group (a phone JID, or a @lid JID in
			// privacy-mode groups) - it's what participant actions
			// (add/remove/promote/demote) need to target. For display we
			// want the phone number, which whatsmeow already resolves onto
			// PhoneNumber; only fall back to a live LID store lookup if that
			// wasn't populated.
			displayJID := p.JID
			if !p.PhoneNumber.IsEmpty() {
				displayJID = p.PhoneNumber
			} else if displayJID.Server == "lid" {
				if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), displayJID); err == nil && !pn.IsEmpty() {
					displayJID = pn
				}
			}

			if displayJID.User == myJID.User {
				if p.IsAdmin || p.IsSuperAdmin {
					isMyAdmin = true
				}
			}

			name := ""
			contact, err := client.Store.Contacts.GetContact(context.Background(), displayJID)
			if err == nil && contact.Found {
				if contact.FullName != "" {
					name = contact.FullName
				} else if contact.PushName != "" {
					name = contact.PushName
				} else if contact.BusinessName != "" {
					name = contact.BusinessName
				}
			}
			// Anonymous participants in announcement groups have no
			// resolvable contact - only an obfuscated phone number.
			if name == "" && p.DisplayName != "" {
				name = p.DisplayName
			}

			participantsList = append(participantsList, map[string]interface{}{
				"JID": p.JID.String(),
				"PhoneJID": displayJID.String(),
				"IsAdmin": p.IsAdmin,
				"IsSuperAdmin": p.IsSuperAdmin,
				"Name": name,
			})
		}

		sendEvent(conn, "group_info", map[string]interface{}{
			"Info": info,
			"Participants": participantsList,
			"ViewerIsAdmin": isMyAdmin,
		})
	}()
}

// handleGetCommunitySubGroups lists the groups linked under a community JID,
// for the read-only "Groups" tab in the group info dialog. Communities are
// just groups with IsParent set, so the request JID is the same community
// chat JID already shown in the chat list.
func handleGetCommunitySubGroups(conn net.Conn, raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		subGroups, err := client.GetSubGroups(context.Background(), jid)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}
		groups := make([]map[string]interface{}, 0, len(subGroups))
		for _, g := range subGroups {
			groups = append(groups, map[string]interface{}{
				"JID":               g.JID.String(),
				"Name":              g.Name,
				"IsDefaultSubGroup": g.IsDefaultSubGroup,
			})
		}
		sendEvent(conn, "community_subgroups", map[string]interface{}{"jid": data.JID, "groups": groups})
	}()
}

// handleJoinCommunitySubgroup joins the caller into a group linked under a
// community. WhatsApp lets community members self-join an open ("all_member_add")
// linked group without an invite link - the server authorizes it based on
// community membership, so this is just a normal add-participant request
// targeting our own JID.
func handleJoinCommunitySubgroup(raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		myJID := client.Store.ID.ToNonAD()
		participants, err := client.UpdateGroupParticipants(context.Background(), jid, []types.JID{myJID}, whatsmeow.ParticipantChangeAdd)
		if err != nil {
			broadcastEvent("error", map[string]string{"message": "Couldn't join this group: " + err.Error()})
			return
		}
		// A nil top-level err doesn't mean the add actually happened - WhatsApp
		// reports per-participant failures (e.g. the subgroup requires admin
		// approval, so we get an invite request queued instead of being added
		// directly) via a non-zero Error code on our own entry.
		if len(participants) > 0 && participants[0].Error != 0 {
			if participants[0].AddRequest != nil {
				broadcastEvent("error", map[string]string{"message": "This group requires admin approval to join. Your request has been sent."})
			} else {
				broadcastEvent("error", map[string]string{"message": "Couldn't join this group. It may be closed to new members or require an invite link."})
			}
			return
		}
		populateChatsFromStore()
		broadcastChats()
		broadcastEvent("community_subgroup_joined", map[string]interface{}{"jid": data.JID})
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
		results, err := client.UpdateGroupParticipants(context.Background(), jid, participants, change)
		if err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		// A nil top-level err doesn't mean every participant change actually
		// happened - WhatsApp reports per-participant failures (e.g. adding
		// someone whose privacy settings block direct group adds queues an
		// invite request instead) via a non-zero Error code on that entry.
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

// groupInviteFromMessageID re-parses a stored message's raw protobuf to pull
// out the fields a GroupInviteMessage needs for GetGroupInfoFromInvite /
// JoinGroupWithInvite - those aren't columns of their own, just part of the
// message content already saved for other purposes (forwarding, quoting).
// The "inviter" WhatsApp wants is whoever sent us the invite: the message's
// sender for a group-relayed invite, or the chat JID itself for a DM.
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
		info, err := client.GetGroupInfoFromInvite(context.Background(), groupJID, inviterJID, code, expiration)
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
		if joinErr := client.JoinGroupWithInvite(context.Background(), groupJID, inviterJID, code, expiration); joinErr != nil {
			broadcastEvent("error", map[string]string{"message": "Couldn't join this group: " + joinErr.Error()})
			return
		}
		populateChatsFromStore()
		broadcastChats()
		broadcastEvent("group_invite_joined", map[string]interface{}{"jid": groupJID.String()})
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
