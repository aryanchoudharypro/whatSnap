package main

import (
	"context"
	"fmt"
	"log"
	"sync"
	"time"

	"go.mau.fi/whatsmeow/types"
)

// groupInfoWrite stores what the server says about a group we're in: its
// name and community structure, our admin rights, its size and timer. Shared
// by the joined-groups refresh and the single-group lookup below.
func groupInfoWrite(g *types.GroupInfo) func(tx sqlExecer) {
	jidStr := g.JID.ToNonAD().String()
	isAdmin := isViewerAdmin(g)
	parentJID := ""
	if !g.LinkedParentJID.IsEmpty() {
		parentJID = g.LinkedParentJID.ToNonAD().String()
	}
	// The server's word on a group's timer is current by definition.
	disappearing := uint32(0)
	if g.IsEphemeral {
		disappearing = g.DisappearingTimer
	}
	return func(tx sqlExecer) {
		if _, err := tx.Exec(`INSERT INTO chats (jid, name, timestamp, is_community, is_admin, participant_count, parent_jid, is_announce_group, admins_only) VALUES (?, ?, 0, ?, ?, ?, ?, ?, ?)
			ON CONFLICT(jid) DO UPDATE SET
			name=COALESCE(NULLIF(excluded.name, ''), chats.name),
			is_community=excluded.is_community,
			is_admin=excluded.is_admin,
			participant_count=excluded.participant_count,
			parent_jid=excluded.parent_jid,
			is_announce_group=excluded.is_announce_group,
			admins_only=excluded.admins_only`,
			jidStr, g.GroupName.Name, boolToInt(g.IsParent), boolToInt(isAdmin), len(g.Participants), parentJID,
			boolToInt(g.IsDefaultSubGroup), boolToInt(g.IsAnnounce)); err != nil {
			fmt.Println("Error upserting group chat", jidStr, ":", err)
		}
		tx.Exec(`UPDATE chats SET ephemeral_expiration = ? WHERE jid = ?`, disappearing, jidStr)
	}
}

// A group can show up before anything told us its name: being added to it
// arrives as a member change rather than a "joined" notice, its first message
// can be handled before the notice, a history chunk can leave its subject out.
// WhatsApp Web asks the server for that one group's metadata whenever it meets
// a group it knows nothing about; requestGroupMetadata does the same, one
// group at a time so a burst of them can't trip the server's rate limit.

const (
	groupMetadataSpacing = 500 * time.Millisecond
	// A group that couldn't be looked up (left, removed, offline) isn't
	// asked about again for a while.
	groupMetadataRetryAfter = 2 * time.Minute
)

var groupMetadataQueue struct {
	sync.Mutex
	queued  []types.JID
	pending map[types.JID]bool
	tried   map[types.JID]time.Time
	running bool
}

// requestGroupMetadata queues a lookup of one group's name and settings.
func requestGroupMetadata(jid types.JID) {
	jid = jid.ToNonAD()
	if jid.Server != types.GroupServer {
		return
	}
	q := &groupMetadataQueue
	q.Lock()
	defer q.Unlock()
	if q.pending == nil {
		q.pending = make(map[types.JID]bool)
		q.tried = make(map[types.JID]time.Time)
	}
	if q.pending[jid] || time.Since(q.tried[jid]) < groupMetadataRetryAfter {
		return
	}
	q.pending[jid] = true
	q.queued = append(q.queued, jid)
	if !q.running {
		q.running = true
		go runGroupMetadataQueue()
	}
}

func runGroupMetadataQueue() {
	q := &groupMetadataQueue
	for {
		q.Lock()
		if len(q.queued) == 0 {
			q.running = false
			// Forget old attempts so the map doesn't grow with every group
			// ever looked up.
			for jid, at := range q.tried {
				if time.Since(at) >= groupMetadataRetryAfter {
					delete(q.tried, jid)
				}
			}
			q.Unlock()
			return
		}
		jid := q.queued[0]
		q.queued = q.queued[1:]
		q.tried[jid] = time.Now()
		q.Unlock()

		fetchGroupMetadata(jid)

		q.Lock()
		delete(q.pending, jid)
		q.Unlock()
		time.Sleep(groupMetadataSpacing)
	}
}

// fetchGroupMetadata asks the server about one group and stores the answer.
func fetchGroupMetadata(jid types.JID) {
	if wa() == nil || !wa().IsLoggedIn() {
		return
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	info, err := wa().GetGroupInfo(ctx, jid)
	if err != nil || info == nil {
		log.Printf("Couldn't look up group %s: %v", jid, err)
		return
	}
	writeBatch([]func(tx sqlExecer){groupInfoWrite(info)})
	markGroupJoined(jid.String(), true)
	broadcastChats()
	broadcastEvent("group_updated", map[string]interface{}{"jid": jid.String()})
}
