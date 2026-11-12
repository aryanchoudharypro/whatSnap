package main

import (
	"context"
	"log"
	"strings"
	"time"

	"go.mau.fi/whatsmeow/proto/waHistorySync"
	"go.mau.fi/whatsmeow/store"
	"go.mau.fi/whatsmeow/types"
)

type lidHookStore struct {
	store.LIDStore
	acc *Account
}

func (acc *Account) hookLIDStore(dev *store.Device) {
	if dev == nil || dev.LIDs == nil {
		return
	}
	if _, ok := dev.LIDs.(*lidHookStore); ok {
		return
	}
	dev.LIDs = &lidHookStore{LIDStore: dev.LIDs, acc: acc}
}

func (s *lidHookStore) PutLIDMapping(ctx context.Context, lid, pn types.JID) error {
	isNew, oldPN := s.isNew(ctx, lid, pn)
	err := s.LIDStore.PutLIDMapping(ctx, lid, pn)
	if err == nil && isNew {
		s.acc.queueLIDMerge(lid, pn, oldPN)
	}
	return err
}

func (s *lidHookStore) PutManyLIDMappings(ctx context.Context, mappings []store.LIDMapping) error {
	type change struct {
		store.LIDMapping
		oldPN types.JID
	}
	var fresh []change
	for _, m := range mappings {
		if isNew, oldPN := s.isNew(ctx, m.LID, m.PN); isNew {
			fresh = append(fresh, change{m, oldPN})
		}
	}
	err := s.LIDStore.PutManyLIDMappings(ctx, mappings)
	if err == nil {
		for _, m := range fresh {
			s.acc.queueLIDMerge(m.LID, m.PN, m.oldPN)
		}
	}
	return err
}

// isNew reports whether lid -> pn is a mapping we don't have yet. oldPN is
// the number the LID had until now, when it had another one: the person
// changed their number but kept their account, so their chat moves with them
// (WA Web files it by LID, so it never splits there).
func (s *lidHookStore) isNew(ctx context.Context, lid, pn types.JID) (isNew bool, oldPN types.JID) {
	if lid.Server != types.HiddenUserServer || pn.Server != types.DefaultUserServer {
		return false, types.EmptyJID
	}
	cur, err := s.LIDStore.GetPNForLID(ctx, lid.ToNonAD())
	if err != nil {
		return true, types.EmptyJID
	}
	if cur.User == pn.User {
		return false, types.EmptyJID
	}
	if !cur.IsEmpty() {
		oldPN = normalizePN(cur)
	}
	return true, oldPN
}

const (
	lidMergeDebounce = 2 * time.Second
	lidMergeMaxWait  = 10 * time.Second
)

func (acc *Account) queueLIDMerge(lid, pn, oldPN types.JID) {
	q := &acc.lidMergeQueue
	q.Lock()
	defer q.Unlock()
	if q.pending == nil {
		q.pending = make(map[string]types.JID)
		q.renumbered = make(map[string]renumber)
		q.since = time.Now()
	}
	q.pending[lid.User] = pn.ToNonAD()
	if !oldPN.IsEmpty() && oldPN.User != pn.User {
		q.renumbered[oldPN.User] = renumber{newPN: normalizePN(pn), lid: lid.ToNonAD()}
	}
	delay := lidMergeDebounce
	if waited := time.Since(q.since); waited+delay > lidMergeMaxWait {
		delay = max(lidMergeMaxWait-waited, 0)
	}
	if q.timer != nil {
		q.timer.Stop()
	}
	q.timer = time.AfterFunc(delay, acc.flushLIDMerges)
}

func (acc *Account) flushLIDMerges() {
	q := &acc.lidMergeQueue
	q.Lock()
	pending := q.pending
	renumbered := q.renumbered
	q.pending = nil
	q.renumbered = nil
	q.Unlock()
	if len(pending) == 0 && len(renumbered) == 0 {
		return
	}
	merged := acc.mergeLIDRowsIntoPN(pending)
	if acc.mergeRenumberedRows(renumbered) {
		merged = true
	}
	if merged {
		acc.invalidateDisplayNames()
		acc.broadcastChats()
	}
}

// renumber is a contact who moved to newPN while keeping their LID.
type renumber struct {
	newPN, lid types.JID
}

// mergeRenumberedRows moves everything filed under a contact's old number
// (keyed by its user part) to their new one, so the chat stays one chat.
// The wire address stays the LID, which is what WhatsApp still uses.
func (acc *Account) mergeRenumberedRows(moves map[string]renumber) bool {
	sources := make([]mergeSource, 0, len(moves))
	for oldUser, r := range moves {
		if oldUser == "" || r.newPN.User == "" || r.newPN.Server != types.DefaultUserServer || oldUser == r.newPN.User {
			continue
		}
		oldPN := types.NewJID(oldUser, types.DefaultUserServer)
		log.Printf("Contact %s changed number; moving its chat to %s", r.lid, r.newPN)
		sources = append(sources, mergeSource{from: oldPN.String(), to: r.newPN.String(), wire: r.lid.String()})
	}
	return acc.mergeRowsInto(sources)
}

func (acc *Account) mergeAllKnownLIDRows() {
	if acc.wa() == nil || acc.wa().Store == nil || acc.wa().Store.ID == nil {
		return
	}
	rows, err := acc.db.Query(`SELECT jid FROM chats WHERE jid LIKE '%@lid'
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
		if pn, err := acc.wa().Store.LIDs.GetPNForLID(context.Background(), lid); err == nil && !pn.IsEmpty() {
			pns[lid.User] = pn.ToNonAD()
		}
	}
	if acc.mergeLIDRowsIntoPN(pns) {
		acc.invalidateDisplayNames()
		acc.broadcastChats()
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

func (acc *Account) mergeLIDRowsIntoPN(pns map[string]types.JID) bool {
	sources := make([]mergeSource, 0, len(pns))
	for lidUser, pn := range pns {
		pn = normalizePN(pn)
		if lidUser == "" || pn.User == "" || pn.Server != types.DefaultUserServer {
			continue
		}
		lid := types.NewJID(lidUser, types.HiddenUserServer).String()
		sources = append(sources, mergeSource{from: lid, to: pn.String(), wire: lid})
	}
	return acc.mergeRowsInto(sources)
}

// mergeSource folds the rows filed under from (a LID, or a contact's old
// number) into to (their phone number). wire is the LID WhatsApp addresses
// them by, kept as the chat's wire_jid and a group sender's wire_sender.
type mergeSource struct {
	from, to, wire string
}

func (acc *Account) mergeRowsInto(sources []mergeSource) bool {
	if len(sources) == 0 {
		return false
	}
	acc.lidMergeMu.Lock()
	defer acc.lidMergeMu.Unlock()
	tx, err := acc.db.Begin()
	if err != nil {
		log.Println("Error starting LID merge transaction:", err)
		return false
	}
	defer tx.Rollback()
	// lid is the JID merged away (a LID or an old number), pn the number it
	// merges into, wire the LID to keep as the WhatsApp-side address.
	tx.Exec(`DROP TABLE IF EXISTS temp.lid_merge_map`)
	tx.Exec(`CREATE TEMP TABLE lid_merge_map (lid TEXT PRIMARY KEY, pn TEXT NOT NULL, wire TEXT NOT NULL)`)
	insert, err := tx.Prepare(`INSERT OR REPLACE INTO lid_merge_map (lid, pn, wire) VALUES (?, ?, ?)`)
	if err != nil {
		log.Println("Error preparing LID merge:", err)
		return false
	}
	for _, src := range sources {
		insert.Exec(src.from, src.to, src.wire)
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
			wire_jid = COALESCE(NULLIF(chats.wire_jid, ''), NULLIF(l.wire_jid, ''), l.merge_wire)
		FROM (SELECT c.*, m.pn AS merge_pn, m.wire AS merge_wire FROM chats c JOIN lid_merge_map m ON m.lid = c.jid) AS l
		WHERE chats.jid = l.merge_pn`,
		`DELETE FROM chats WHERE jid IN (SELECT lid FROM lid_merge_map)
			AND EXISTS (SELECT 1 FROM lid_merge_map m JOIN chats p ON p.jid = m.pn WHERE m.lid = chats.jid)`,
		`UPDATE chats SET wire_jid = COALESCE(NULLIF(wire_jid, ''), (SELECT wire FROM lid_merge_map WHERE lid = chats.jid)), jid = (SELECT pn FROM lid_merge_map WHERE lid = chats.jid)
			WHERE jid IN (SELECT lid FROM lid_merge_map)`,
		`UPDATE chats SET preview_sender = (SELECT pn FROM lid_merge_map WHERE lid = chats.preview_sender)
			WHERE preview_sender IN (SELECT lid FROM lid_merge_map)`,
		`UPDATE messages SET jid = (SELECT pn FROM lid_merge_map WHERE lid = messages.jid)
			WHERE jid IN (SELECT lid FROM lid_merge_map)`,
		`UPDATE messages SET
			wire_sender = CASE WHEN COALESCE(wire_sender, '') = '' AND jid LIKE '%@g.us' THEN (SELECT wire FROM lid_merge_map WHERE lid = messages.sender_jid) ELSE wire_sender END,
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
		`UPDATE OR IGNORE chat_favorites SET jid = (SELECT pn FROM lid_merge_map WHERE lid = chat_favorites.jid)
			WHERE jid IN (SELECT lid FROM lid_merge_map)`,
		`DELETE FROM chat_favorites WHERE jid IN (SELECT lid FROM lid_merge_map)`,
		`UPDATE OR IGNORE chat_list_members SET jid = (SELECT pn FROM lid_merge_map WHERE lid = chat_list_members.jid)
			WHERE jid IN (SELECT lid FROM lid_merge_map)`,
		`DELETE FROM chat_list_members WHERE jid IN (SELECT lid FROM lid_merge_map)`,
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
