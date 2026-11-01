package main

import (
	"context"
	"log"
	"strings"
	"sync"
	"time"

	"go.mau.fi/whatsmeow/proto/waHistorySync"
	"go.mau.fi/whatsmeow/store"
	"go.mau.fi/whatsmeow/types"
)

type lidHookStore struct {
	store.LIDStore
}

func hookLIDStore(dev *store.Device) {
	if dev == nil || dev.LIDs == nil {
		return
	}
	if _, ok := dev.LIDs.(*lidHookStore); ok {
		return
	}
	dev.LIDs = &lidHookStore{LIDStore: dev.LIDs}
}

func (s *lidHookStore) PutLIDMapping(ctx context.Context, lid, pn types.JID) error {
	isNew := s.isNew(ctx, lid, pn)
	err := s.LIDStore.PutLIDMapping(ctx, lid, pn)
	if err == nil && isNew {
		queueLIDMerge(lid, pn)
	}
	return err
}

func (s *lidHookStore) PutManyLIDMappings(ctx context.Context, mappings []store.LIDMapping) error {
	var fresh []store.LIDMapping
	for _, m := range mappings {
		if s.isNew(ctx, m.LID, m.PN) {
			fresh = append(fresh, m)
		}
	}
	err := s.LIDStore.PutManyLIDMappings(ctx, mappings)
	if err == nil {
		for _, m := range fresh {
			queueLIDMerge(m.LID, m.PN)
		}
	}
	return err
}

func (s *lidHookStore) isNew(ctx context.Context, lid, pn types.JID) bool {
	if lid.Server != types.HiddenUserServer || pn.Server != types.DefaultUserServer {
		return false
	}
	cur, err := s.LIDStore.GetPNForLID(ctx, lid.ToNonAD())
	return err != nil || cur.User != pn.User
}

const (
	lidMergeDebounce = 2 * time.Second
	lidMergeMaxWait  = 10 * time.Second
)

var lidMergeQueue struct {
	sync.Mutex
	pending map[string]types.JID
	timer   *time.Timer
	since   time.Time
}

func queueLIDMerge(lid, pn types.JID) {
	q := &lidMergeQueue
	q.Lock()
	defer q.Unlock()
	if q.pending == nil {
		q.pending = make(map[string]types.JID)
		q.since = time.Now()
	}
	q.pending[lid.User] = pn.ToNonAD()
	delay := lidMergeDebounce
	if waited := time.Since(q.since); waited+delay > lidMergeMaxWait {
		delay = max(lidMergeMaxWait-waited, 0)
	}
	if q.timer != nil {
		q.timer.Stop()
	}
	q.timer = time.AfterFunc(delay, flushLIDMerges)
}

func flushLIDMerges() {
	q := &lidMergeQueue
	q.Lock()
	pending := q.pending
	q.pending = nil
	q.Unlock()
	if len(pending) == 0 {
		return
	}
	if mergeLIDRowsIntoPN(pending) {
		invalidateDisplayNames()
		broadcastChats()
	}
}

func mergeAllKnownLIDRows() {
	if wa() == nil || wa().Store == nil || wa().Store.ID == nil {
		return
	}
	rows, err := db.Query(`SELECT jid FROM chats WHERE jid LIKE '%@lid'
		UNION SELECT sender_jid FROM messages WHERE sender_jid LIKE '%@lid'
		UNION SELECT jid FROM contacts_cache WHERE jid LIKE '%@lid'
		UNION SELECT reactor_jid FROM reactions WHERE reactor_jid LIKE '%@lid'
		UNION SELECT voter FROM poll_votes WHERE voter LIKE '%@lid'`)
	if err != nil {
		return
	}
	var lids []types.JID
	for rows.Next() {
		var s string
		if rows.Scan(&s) == nil {
			if j, perr := types.ParseJID(s); perr == nil && j.Server == types.HiddenUserServer {
				lids = append(lids, j)
			}
		}
	}
	rows.Close()
	pns := make(map[string]types.JID)
	for _, lid := range lids {
		if pn, err := wa().Store.LIDs.GetPNForLID(context.Background(), lid); err == nil && !pn.IsEmpty() {
			pns[lid.User] = pn.ToNonAD()
		}
	}
	if mergeLIDRowsIntoPN(pns) {
		invalidateDisplayNames()
		broadcastChats()
	}
}

func historyChunkLIDMap(mappings []*waHistorySync.PhoneNumberToLIDMapping) map[string]types.JID {
	pns := make(map[string]types.JID, len(mappings))
	for _, mapping := range mappings {
		if mapping == nil {
			continue
		}
		pnJID, err1 := types.ParseJID(mapping.GetPnJID())
		lidJID, err2 := types.ParseJID(mapping.GetLidJID())
		if err1 == nil && err2 == nil && !pnJID.IsEmpty() && !lidJID.IsEmpty() {
			pns[lidJID.ToNonAD().User] = normalizePN(pnJID)
		}
	}
	return pns
}

func normalizePN(j types.JID) types.JID {
	j = j.ToNonAD()
	if j.Server == types.LegacyUserServer {
		j.Server = types.DefaultUserServer
	}
	return j
}

var keyedLIDTables = []struct{ table, key, person, ts string }{
	{"reactions", "message_id", "reactor_jid", "timestamp"},
	{"poll_votes", "poll_id", "voter", "timestamp"},
	{"event_responses", "event_id", "responder", "timestamp"},
	{"message_receipts", "message_id", "participant_jid", "timestamp"},
	{"group_member_labels", "group_jid", "member", "set_at"},
}

var lidMergeMu sync.Mutex

func mergeLIDRowsIntoPN(pns map[string]types.JID) bool {
	if len(pns) == 0 {
		return false
	}
	lidMergeMu.Lock()
	defer lidMergeMu.Unlock()
	tx, err := db.Begin()
	if err != nil {
		log.Println("Error starting LID merge transaction:", err)
		return false
	}
	defer tx.Rollback()
	tx.Exec(`CREATE TEMP TABLE IF NOT EXISTS lid_merge_map (lid TEXT PRIMARY KEY, pn TEXT NOT NULL)`)
	tx.Exec(`DELETE FROM lid_merge_map`)
	insert, err := tx.Prepare(`INSERT OR REPLACE INTO lid_merge_map (lid, pn) VALUES (?, ?)`)
	if err != nil {
		log.Println("Error preparing LID merge:", err)
		return false
	}
	for lidUser, pn := range pns {
		pn = normalizePN(pn)
		if lidUser == "" || pn.User == "" || pn.Server != types.DefaultUserServer {
			continue
		}
		insert.Exec(types.NewJID(lidUser, types.HiddenUserServer).String(), pn.String())
	}
	insert.Close()

	stmts := []string{
		`UPDATE chats SET
			timestamp = MAX(COALESCE(chats.timestamp, 0), COALESCE(l.timestamp, 0)),
			unread_count = COALESCE(chats.unread_count, 0) + COALESCE(l.unread_count, 0),
			pinned = CASE WHEN COALESCE(l.pin_changed_at, 0) > COALESCE(chats.pin_changed_at, 0) THEN COALESCE(l.pinned, 0)
				WHEN COALESCE(chats.pin_changed_at, 0) > 0 THEN COALESCE(chats.pinned, 0)
				ELSE MAX(COALESCE(chats.pinned, 0), COALESCE(l.pinned, 0)) END,
			pinned_at = CASE WHEN COALESCE(l.pin_changed_at, 0) > COALESCE(chats.pin_changed_at, 0) THEN COALESCE(l.pinned_at, 0)
				WHEN COALESCE(chats.pin_changed_at, 0) > 0 THEN COALESCE(chats.pinned_at, 0)
				ELSE MAX(COALESCE(chats.pinned_at, 0), COALESCE(l.pinned_at, 0)) END,
			pin_changed_at = MAX(COALESCE(chats.pin_changed_at, 0), COALESCE(l.pin_changed_at, 0)),
			muted = MAX(COALESCE(chats.muted, 0), COALESCE(l.muted, 0)),
			archived = MAX(COALESCE(chats.archived, 0), COALESCE(l.archived, 0)),
			is_locked = MAX(COALESCE(chats.is_locked, 0), COALESCE(l.is_locked, 0)),
			marked_unread = MAX(COALESCE(chats.marked_unread, 0), COALESCE(l.marked_unread, 0)),
			name = CASE WHEN COALESCE(chats.name, '') = '' OR chats.name LIKE '+%' THEN COALESCE(NULLIF(l.name, ''), chats.name) ELSE chats.name END,
			preview_snippet = CASE WHEN COALESCE(l.timestamp, 0) > COALESCE(chats.timestamp, 0) THEN l.preview_snippet ELSE chats.preview_snippet END,
			preview_sender = CASE WHEN COALESCE(l.timestamp, 0) > COALESCE(chats.timestamp, 0) THEN l.preview_sender ELSE chats.preview_sender END,
			preview_from_me = CASE WHEN COALESCE(l.timestamp, 0) > COALESCE(chats.timestamp, 0) THEN l.preview_from_me ELSE chats.preview_from_me END,
			preview_is_reaction = CASE WHEN COALESCE(l.timestamp, 0) > COALESCE(chats.timestamp, 0) THEN l.preview_is_reaction ELSE chats.preview_is_reaction END,
			preview_reaction_emoji = CASE WHEN COALESCE(l.timestamp, 0) > COALESCE(chats.timestamp, 0) THEN l.preview_reaction_emoji ELSE chats.preview_reaction_emoji END,
			ephemeral_expiration = CASE WHEN COALESCE(l.ephemeral_setting_ts, 0) > COALESCE(chats.ephemeral_setting_ts, 0) THEN l.ephemeral_expiration ELSE chats.ephemeral_expiration END,
			ephemeral_setting_ts = MAX(COALESCE(chats.ephemeral_setting_ts, 0), COALESCE(l.ephemeral_setting_ts, 0)),
			limit_sharing = CASE WHEN COALESCE(l.limit_sharing_ts, 0) > COALESCE(chats.limit_sharing_ts, 0) THEN l.limit_sharing ELSE chats.limit_sharing END,
			limit_sharing_ts = MAX(COALESCE(chats.limit_sharing_ts, 0), COALESCE(l.limit_sharing_ts, 0)),
			restricted = CASE WHEN COALESCE(l.restricted_ts, 0) > COALESCE(chats.restricted_ts, 0) THEN l.restricted ELSE chats.restricted END,
			restricted_ts = MAX(COALESCE(chats.restricted_ts, 0), COALESCE(l.restricted_ts, 0)),
			wire_jid = COALESCE(NULLIF(chats.wire_jid, ''), NULLIF(l.wire_jid, ''), l.jid)
		FROM (SELECT c.*, m.pn AS merge_pn FROM chats c JOIN lid_merge_map m ON m.lid = c.jid) AS l
		WHERE chats.jid = l.merge_pn`,
		`DELETE FROM chats WHERE jid IN (SELECT lid FROM lid_merge_map)
			AND EXISTS (SELECT 1 FROM lid_merge_map m JOIN chats p ON p.jid = m.pn WHERE m.lid = chats.jid)`,
		`UPDATE chats SET wire_jid = COALESCE(NULLIF(wire_jid, ''), jid), jid = (SELECT pn FROM lid_merge_map WHERE lid = chats.jid)
			WHERE jid IN (SELECT lid FROM lid_merge_map)`,
		`UPDATE chats SET preview_sender = (SELECT pn FROM lid_merge_map WHERE lid = chats.preview_sender)
			WHERE preview_sender IN (SELECT lid FROM lid_merge_map)`,
		`UPDATE messages SET jid = (SELECT pn FROM lid_merge_map WHERE lid = messages.jid)
			WHERE jid IN (SELECT lid FROM lid_merge_map)`,
		`UPDATE messages SET
			wire_sender = CASE WHEN COALESCE(wire_sender, '') = '' AND jid LIKE '%@g.us' THEN sender_jid ELSE wire_sender END,
			sender_jid = (SELECT pn FROM lid_merge_map WHERE lid = messages.sender_jid)
			WHERE sender_jid IN (SELECT lid FROM lid_merge_map)`,
		`UPDATE reactions SET chat_jid = (SELECT pn FROM lid_merge_map WHERE lid = reactions.chat_jid)
			WHERE chat_jid IN (SELECT lid FROM lid_merge_map)`,
		`UPDATE contacts_cache SET name = l.name
			FROM (SELECT c.name, m.pn AS merge_pn FROM contacts_cache c JOIN lid_merge_map m ON m.lid = c.jid WHERE c.name != '') AS l
			WHERE contacts_cache.jid = l.merge_pn AND COALESCE(contacts_cache.name, '') = ''`,
		`DELETE FROM contacts_cache WHERE jid IN (SELECT lid FROM lid_merge_map)
			AND EXISTS (SELECT 1 FROM lid_merge_map m JOIN contacts_cache p ON p.jid = m.pn WHERE m.lid = contacts_cache.jid)`,
		`UPDATE contacts_cache SET
			phone = substr((SELECT pn FROM lid_merge_map WHERE lid = contacts_cache.jid), 1, instr((SELECT pn FROM lid_merge_map WHERE lid = contacts_cache.jid), '@') - 1),
			jid = (SELECT pn FROM lid_merge_map WHERE lid = contacts_cache.jid)
			WHERE jid IN (SELECT lid FROM lid_merge_map)`,
		`UPDATE contacts_cache SET phone = substr(jid, 1, instr(jid, '@') - 1)
			WHERE jid IN (SELECT pn FROM lid_merge_map) AND COALESCE(phone, '') = ''`,
		`DELETE FROM history_requests WHERE jid IN (SELECT lid FROM lid_merge_map)
			AND EXISTS (SELECT 1 FROM lid_merge_map m JOIN history_requests p ON p.jid = m.pn WHERE m.lid = history_requests.jid)`,
		`UPDATE history_requests SET jid = (SELECT pn FROM lid_merge_map WHERE lid = history_requests.jid)
			WHERE jid IN (SELECT lid FROM lid_merge_map)`,
	}
	for _, t := range keyedLIDTables {
		r := strings.NewReplacer("{T}", t.table, "{K}", t.key, "{P}", t.person, "{S}", t.ts)
		stmts = append(stmts,
			r.Replace(`DELETE FROM {T} WHERE {P} IN (SELECT pn FROM lid_merge_map)
				AND EXISTS (SELECT 1 FROM {T} o JOIN lid_merge_map m ON o.{P} = m.lid
					WHERE m.pn = {T}.{P} AND o.{K} = {T}.{K} AND COALESCE(o.{S}, 0) > COALESCE({T}.{S}, 0))`),
			r.Replace(`DELETE FROM {T} WHERE {P} IN (SELECT lid FROM lid_merge_map)
				AND EXISTS (SELECT 1 FROM {T} o JOIN lid_merge_map m ON o.{P} = m.pn
					WHERE m.lid = {T}.{P} AND o.{K} = {T}.{K})`),
			r.Replace(`UPDATE {T} SET {P} = (SELECT pn FROM lid_merge_map WHERE lid = {T}.{P})
				WHERE {P} IN (SELECT lid FROM lid_merge_map)`),
		)
	}
	for _, stmt := range stmts {
		if _, err := tx.Exec(stmt); err != nil {
			log.Println("Error merging LID rows:", err)
		}
	}
	tx.Exec(`DELETE FROM lid_merge_map`)
	if err := tx.Commit(); err != nil {
		log.Println("Error committing LID merge:", err)
		return false
	}
	return true
}

func conversationLIDPair(conv *waHistorySync.Conversation, id types.JID) (store.LIDMapping, bool) {
	var lid, pn types.JID
	switch id.Server {
	case types.HiddenUserServer:
		lid = id
		pn, _ = types.ParseJID(conv.GetPnJID())
	case types.DefaultUserServer, types.LegacyUserServer:
		pn = id
		lid, _ = types.ParseJID(conv.GetLidJID())
	default:
		return store.LIDMapping{}, false
	}
	lid, pn = lid.ToNonAD(), normalizePN(pn)
	if lid.Server != types.HiddenUserServer || lid.User == "" || pn.Server != types.DefaultUserServer || pn.User == "" {
		return store.LIDMapping{}, false
	}
	return store.LIDMapping{LID: lid, PN: pn}, true
}
