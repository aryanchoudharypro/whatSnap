package main

import (
	"cmp"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"net"
	"slices"
	"strings"
	"sync"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
)

// A community is a parent group whose own chat carries no messages. People
// talk in the groups linked under it: its announcements group (where only
// admins post) and any number of member groups. The chats table records
// which community each group belongs to (parent_jid) and which one is the
// announcements group (is_announce_group).

var (
	joinedGroupsMu sync.RWMutex
	// joinedGroups is the set of groups we're a member of, as of the last
	// joined-groups refresh plus the join/leave events since. nil until the
	// first refresh finishes.
	joinedGroups map[string]bool
	// subgroupParents remembers which community each listed group belongs to,
	// so joining one we aren't in yet (and so have no chat row for) can go
	// through its community.
	subgroupParents sync.Map
)

func setJoinedGroups(joined map[string]bool) {
	joinedGroupsMu.Lock()
	joinedGroups = joined
	joinedGroupsMu.Unlock()
}

func markGroupJoined(jid string, joined bool) {
	joinedGroupsMu.Lock()
	defer joinedGroupsMu.Unlock()
	if joinedGroups == nil {
		return
	}
	if joined {
		joinedGroups[jid] = true
	} else {
		delete(joinedGroups, jid)
	}
}

// knownNotMember reports whether the last joined-groups refresh, plus the
// join/leave events since, say we're not in a group. Before the first refresh
// nothing is known and it reports false.
func knownNotMember(jid string) bool {
	joinedGroupsMu.RLock()
	defer joinedGroupsMu.RUnlock()
	return joinedGroups != nil && !joinedGroups[jid]
}

// isJoinedGroup reports whether we're a member of a group. Before the first
// joined-groups refresh it falls back on having a chat row for it.
func isJoinedGroup(jid string) bool {
	joinedGroupsMu.RLock()
	known, loaded := joinedGroups[jid], joinedGroups != nil
	joinedGroupsMu.RUnlock()
	if loaded {
		return known
	}
	var n int
	db.QueryRow(`SELECT COUNT(*) FROM chats WHERE jid = ?`, jid).Scan(&n)
	return n > 0
}

// communityParentOf returns the community a group belongs to, or "".
func communityParentOf(jid string) string {
	var parent string
	db.QueryRow(`SELECT COALESCE(parent_jid, '') FROM chats WHERE jid = ?`, jid).Scan(&parent)
	if parent == "" {
		if cached, ok := subgroupParents.Load(jid); ok {
			parent = cached.(string)
		}
	}
	return parent
}

// communityName is the name we show for a community.
func communityName(jid string) string {
	var name string
	db.QueryRow(`SELECT COALESCE(name, '') FROM chats WHERE jid = ?`, jid).Scan(&name)
	return cmp.Or(name, "community")
}

// recordGroupCommunity stores where a group sits in a community, from its
// full group info (a join, or the group being created).
func recordGroupCommunity(info *types.GroupInfo) {
	parent := ""
	if !info.LinkedParentJID.IsEmpty() {
		parent = info.LinkedParentJID.ToNonAD().String()
	}
	db.Exec(`UPDATE chats SET is_community = ?, parent_jid = ?, is_announce_group = ? WHERE jid = ?`,
		boolToInt(info.IsParent), parent, boolToInt(info.IsDefaultSubGroup), info.JID.ToNonAD().String())
}

// applyCommunityLinkChange updates a group's community when it's linked into
// or unlinked from one. WhatsApp tells the community ("sub_group": this group
// was linked under you) and the group itself ("parent_group": you were linked
// under this community); only the second gets a line in the chat, matching
// the official apps.
func applyCommunityLinkChange(v *events.GroupInfo, change *types.GroupLinkChange, linked bool) {
	var parent, child types.JID
	switch change.Type {
	case types.GroupLinkChangeTypeSub:
		parent, child = v.JID, change.Group.JID
	case types.GroupLinkChangeTypeParent:
		parent, child = change.Group.JID, v.JID
	default:
		return
	}
	parentStr, childStr := parent.ToNonAD().String(), child.ToNonAD().String()
	if linked {
		db.Exec(`UPDATE chats SET parent_jid = ? WHERE jid = ?`, parentStr, childStr)
		if change.Group.IsDefaultSubGroup && change.Type == types.GroupLinkChangeTypeSub {
			db.Exec(`UPDATE chats SET is_announce_group = 1 WHERE jid = ?`, childStr)
		}
	} else {
		db.Exec(`UPDATE chats SET parent_jid = '', is_announce_group = 0 WHERE jid = ? AND parent_jid = ?`, childStr, parentStr)
	}
	broadcastChats()

	if change.Type != types.GroupLinkChangeTypeParent || !isJoinedGroup(childStr) {
		return
	}
	name := change.Group.Name
	if name == "" {
		name = communityName(parentStr)
	}
	actor := ""
	if v.Sender != nil {
		actor = nameToken(*v.Sender)
	}
	var text string
	switch {
	case linked && actor != "":
		text = fmt.Sprintf("%s added this group to the community \"%s\"", actor, name)
	case linked:
		text = fmt.Sprintf("This group was added to the community \"%s\"", name)
	case change.UnlinkReason == types.GroupUnlinkReasonDelete:
		text = fmt.Sprintf("The community \"%s\" was deactivated", name)
	case actor != "":
		text = fmt.Sprintf("%s removed this group from the community \"%s\"", actor, name)
	default:
		text = fmt.Sprintf("This group was removed from the community \"%s\"", name)
	}
	suffix := "link"
	if !linked {
		suffix = "unlink"
	}
	insertGroupSystemMessage(childStr, fmt.Sprintf("system_%s_%d_%s", child.User, v.Timestamp.UnixNano(), suffix), text, v.Timestamp)
}

// handleGetCommunitySubGroups lists a community's groups for the Groups tab
// of the group info dialog. Asked about a group inside a community, it lists
// that community's groups instead, so the tab works from any of them.
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
		community := jid
		if parent := communityParentOf(jid.String()); parent != "" {
			if parsed, perr := types.ParseJID(parent); perr == nil {
				community = parsed
			}
		}
		subGroups, err := wa().GetSubGroups(context.Background(), community)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": "Couldn't load the community's groups: " + err.Error()})
			return
		}
		// Member counts come from a separate query, as in WhatsApp Web. The
		// list is still worth showing without them.
		counts, countErr := wa().GetSubGroupParticipantCounts(context.Background(), community)
		if countErr != nil {
			log.Printf("Couldn't get member counts for %s's groups: %v", community, countErr)
		}
		type entry struct {
			jid, name                  string
			announcements, generalChat bool
			joined                     bool
			size                       int
		}
		entries := make([]entry, 0, len(subGroups))
		for _, g := range subGroups {
			jidStr := g.JID.ToNonAD().String()
			subgroupParents.Store(jidStr, community.String())
			entries = append(entries, entry{
				jid:           jidStr,
				name:          g.Name,
				announcements: g.IsDefaultSubGroup,
				generalChat:   g.IsGeneralChat,
				joined:        isJoinedGroup(jidStr),
				size:          cmp.Or(counts[g.JID.ToNonAD()], g.ParticipantCount),
			})
		}
		// The order the official apps use: announcements, then the groups
		// you're in, then the ones you could join.
		rank := func(e entry) int {
			switch {
			case e.announcements:
				return 0
			case e.joined:
				return 1
			default:
				return 2
			}
		}
		slices.SortStableFunc(entries, func(a, b entry) int {
			return cmp.Or(cmp.Compare(rank(a), rank(b)), strings.Compare(strings.ToLower(a.name), strings.ToLower(b.name)))
		})
		groups := make([]map[string]interface{}, 0, len(entries))
		for _, e := range entries {
			groups = append(groups, map[string]interface{}{
				"JID":               e.jid,
				"Name":              e.name,
				"IsDefaultSubGroup": e.announcements,
				"IsGeneralChat":     e.generalChat,
				"Joined":            e.joined,
				"ParticipantCount":  e.size,
			})
		}
		sendEvent(conn, "community_subgroups", map[string]interface{}{
			"jid":           data.JID,
			"community":     community.String(),
			"communityName": communityName(community.String()),
			"groups":        groups,
		})
	}()
}

// handleJoinCommunitySubgroup joins a group in a community we're a member
// of, through the community, the way WhatsApp Web does. A group that needs
// admin approval gets a join request instead.
func handleJoinCommunitySubgroup(raw json.RawMessage) {
	var data struct {
		JID       string `json:"jid"`
		Community string `json:"community"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	child, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	parent, err := types.ParseJID(cmp.Or(data.Community, communityParentOf(data.JID)))
	if err != nil || parent.IsEmpty() {
		broadcastEvent("error", map[string]string{"message": "Couldn't join this group: its community isn't known. Open the community's groups list and try again."})
		return
	}
	go func() {
		err := wa().JoinLinkedGroup(context.Background(), parent, child)
		switch {
		case errors.Is(err, whatsmeow.ErrJoinRequiresApproval):
			broadcastEvent("error", map[string]string{"message": "This group needs an admin to approve new members. Your request to join was sent."})
			return
		case err != nil:
			broadcastEvent("error", map[string]string{"message": "Couldn't join this group: " + err.Error()})
			return
		}
		markGroupJoined(child.String(), true)
		invalidateJoinedGroups()
		populateChatsFromStore()
		broadcastChats()
		broadcastEvent("community_subgroup_joined", map[string]interface{}{"jid": data.JID})
	}()
}

// communityMembers adds everyone in a community's groups to the community's
// own participant list, which on its own holds only a few of them. Returns
// the extra people, each flagged as a plain member.
func communityMembers(community types.JID, known []types.GroupParticipant) []types.GroupParticipant {
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	all, err := wa().GetLinkedGroupsParticipants(ctx, community)
	if err != nil {
		return nil
	}
	seen := make(map[string]bool, len(known))
	for _, p := range known {
		seen[preferPN(p.JID).String()] = true
	}
	var extra []types.GroupParticipant
	for _, jid := range all {
		key := preferPN(jid).String()
		if seen[key] {
			continue
		}
		seen[key] = true
		p := types.GroupParticipant{JID: jid}
		if jid.Server == types.HiddenUserServer {
			p.LID = jid
		} else {
			p.PhoneNumber = jid
		}
		extra = append(extra, p)
	}
	return extra
}
