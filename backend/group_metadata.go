package main

import (
	"context"
	"fmt"
	"log"
	"time"

	"go.mau.fi/whatsmeow/types"
)

func (acc *Account) groupInfoWrite(g *types.GroupInfo) func(tx sqlExecer) {
	jidStr := g.JID.ToNonAD().String()
	isAdmin := acc.isViewerAdmin(g)
	parentJID := ""
	if !g.LinkedParentJID.IsEmpty() {
		parentJID = g.LinkedParentJID.ToNonAD().String()
	}
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

const (
	groupMetadataSpacing    = 500 * time.Millisecond
	groupMetadataRetryAfter = 2 * time.Minute
)

func (acc *Account) requestGroupMetadata(jid types.JID) {
	jid = jid.ToNonAD()
	if jid.Server != types.GroupServer {
		return
	}
	q := &acc.groupMetadataQueue
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
		go acc.runGroupMetadataQueue()
	}
}

func (acc *Account) runGroupMetadataQueue() {
	q := &acc.groupMetadataQueue
	for {
		q.Lock()
		if len(q.queued) == 0 {
			q.running = false
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

		acc.fetchGroupMetadata(jid)

		q.Lock()
		delete(q.pending, jid)
		q.Unlock()
		time.Sleep(groupMetadataSpacing)
	}
}

func (acc *Account) fetchGroupMetadata(jid types.JID) {
	if acc.wa() == nil || !acc.wa().IsLoggedIn() {
		return
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	info, err := acc.wa().GetGroupInfo(ctx, jid)
	if err != nil || info == nil {
		log.Printf("Couldn't look up group %s: %v", jid, err)
		return
	}
	acc.writeBatch([]func(tx sqlExecer){acc.groupInfoWrite(info)})
	acc.markGroupJoined(jid.String(), true)
	acc.broadcastChats()
	acc.broadcastEvent("group_updated", map[string]interface{}{"jid": jid.String()})
}
