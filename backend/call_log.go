package main

import (
	"fmt"
	"log"
	"strings"
	"time"

	"go.mau.fi/whatsmeow/proto/waSyncAction"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
)

func callLogChatJID(
	rec *waSyncAction.CallLogRecord,
	creator types.JID,
	fromMe bool,
	ownPN, ownLID types.JID,
	normalize func(types.JID) types.JID,
) string {
	if g := rec.GetGroupJID(); g != "" {
		if jid, err := types.ParseJID(g); err == nil {
			return jid.String()
		}
	}
	if !fromMe && !creator.IsEmpty() {
		return normalize(creator.ToNonAD()).String()
	}
	isOwn := func(jid types.JID) bool { return ownInSameMode(jid, ownPN, ownLID) }
	for _, p := range rec.GetParticipants() {
		jid, err := types.ParseJID(p.GetUserJID())
		if err != nil || jid.IsEmpty() || isOwn(jid) {
			continue
		}
		return normalize(jid.ToNonAD()).String()
	}
	if !creator.IsEmpty() && !isOwn(creator) {
		return normalize(creator.ToNonAD()).String()
	}
	return ""
}

// callLogKeyChat is the chat WA Web files a synced call under, which is the
// chat its Delete for me names (WAWebVoipBackendCallLogTargetResolver): the
// group for a group call, the creator for a call someone else placed, and for
// a call we placed the one other person in it, or our own chat when there
// isn't exactly one (call links, ad hoc group calls). It is not the chat
// whatSnap shows the call in, which for those last ones is the first person.
func callLogKeyChat(rec *waSyncAction.CallLogRecord, creator types.JID, fromMe bool, ownPN, ownLID types.JID) string {
	if g := rec.GetGroupJID(); g != "" {
		if jid, err := types.ParseJID(g); err == nil {
			return jid.String()
		}
		return ""
	}
	if creator.IsEmpty() {
		return ""
	}
	if !fromMe {
		return creator.ToNonAD().String()
	}
	var others []types.JID
	for _, p := range rec.GetParticipants() {
		jid, err := types.ParseJID(p.GetUserJID())
		if err != nil || jid.IsEmpty() || ownInSameMode(jid, ownPN, ownLID) {
			continue
		}
		others = append(others, jid)
	}
	if len(others) == 1 && !rec.GetIsCallLink() && rec.GetCallLinkToken() == "" {
		return others[0].ToNonAD().String()
	}
	return creator.ToNonAD().String()
}

// ownInSameMode reports whether jid is this account, keyed on the addressing
// mode, never across it: a peer LID whose digits spell our phone number is not
// us.
func ownInSameMode(jid, ownPN, ownLID types.JID) bool {
	switch jid.Server {
	case types.HiddenUserServer:
		return !ownLID.IsEmpty() && ownLID.User == jid.User
	case types.DefaultUserServer:
		return !ownPN.IsEmpty() && ownPN.User == jid.User
	}
	return false
}

// callDurationText renders a call length the way it is spoken, since a screen
// reader reads this line out as prose.
func callDurationText(seconds int64) string {
	if seconds <= 0 {
		return ""
	}
	d := time.Duration(seconds) * time.Second
	parts := make([]string, 0, 2)
	if h := int(d.Hours()); h > 0 {
		parts = append(parts, plural(h, "hour"))
	}
	if m := int(d.Minutes()) % 60; m > 0 {
		parts = append(parts, plural(m, "minute"))
	}
	if s := int(d.Seconds()) % 60; s > 0 && len(parts) < 2 && int(d.Hours()) == 0 {
		parts = append(parts, plural(s, "second"))
	}
	if len(parts) == 0 {
		return ""
	}
	return strings.Join(parts, " ")
}

func plural(n int, unit string) string {
	if n == 1 {
		return fmt.Sprintf("%d %s", n, unit)
	}
	return fmt.Sprintf("%d %ss", n, unit)
}

// callLogText renders one call record as the line shown in the chat. Direction
// comes first because that is what distinguishes two otherwise identical rows,
// and a screen reader reads the start of a line first.
func callLogText(rec *waSyncAction.CallLogRecord, fromMe bool, isGroup bool) string {
	kind := "Voice call"
	if rec.GetIsVideo() {
		kind = "Video call"
	}
	if isGroup {
		kind = "Group " + strings.ToLower(kind)
	}
	direction := "Incoming"
	if fromMe {
		direction = "Outgoing"
	}

	switch rec.GetCallResult() {
	case waSyncAction.CallLogRecord_CONNECTED, waSyncAction.CallLogRecord_ONGOING:
		if d := callDurationText(rec.GetDuration()); d != "" {
			return fmt.Sprintf("%s %s, %s", direction, strings.ToLower(kind), d)
		}
		return fmt.Sprintf("%s %s", direction, strings.ToLower(kind))
	case waSyncAction.CallLogRecord_MISSED:
		if fromMe {
			return fmt.Sprintf("%s, no answer", kind)
		}
		return fmt.Sprintf("Missed %s", strings.ToLower(kind))
	case waSyncAction.CallLogRecord_REJECTED:
		if fromMe {
			return fmt.Sprintf("%s, declined", kind)
		}
		return fmt.Sprintf("%s, you declined", kind)
	case waSyncAction.CallLogRecord_CANCELLED:
		if fromMe {
			return fmt.Sprintf("%s, cancelled", kind)
		}
		return fmt.Sprintf("Missed %s", strings.ToLower(kind))
	case waSyncAction.CallLogRecord_UNAVAILABLE:
		return fmt.Sprintf("%s, unavailable", kind)
	case waSyncAction.CallLogRecord_FAILED, waSyncAction.CallLogRecord_INVALID:
		return fmt.Sprintf("%s, failed", kind)
	case waSyncAction.CallLogRecord_ABANDONED:
		return fmt.Sprintf("%s, abandoned", kind)
	case waSyncAction.CallLogRecord_ACCEPTEDELSEWHERE:
		return fmt.Sprintf("%s, answered on another device", kind)
	case waSyncAction.CallLogRecord_UPCOMING:
		return fmt.Sprintf("Upcoming %s", strings.ToLower(kind))
	}
	return fmt.Sprintf("%s %s", direction, strings.ToLower(kind))
}

func callLogTimestamp(rec *waSyncAction.CallLogRecord, mutation time.Time) int64 {
	if st := rec.GetStartTime(); st > 0 {
		return st
	}
	if !mutation.IsZero() {
		return mutation.Unix()
	}
	return time.Now().Unix()
}

func handleCallLogSync(v *events.CallLogSync) {
	storeSyncedCall(v.CallID, v.CallCreatorJID, v.FromMe, callLogTimestamp(v.Record, v.Timestamp), v.Record, v.FromFullSync)
}

func handleCallLogHistory(v *events.CallLogHistory) {
	rec := v.Record
	if rec == nil {
		return
	}
	fromMe := v.FromMe
	if !v.FromMeKnown {
		fromMe = !rec.GetIsIncoming()
	}
	ts := callLogTimestamp(rec, v.Timestamp)
	id := v.CallID
	if id == "" {
		id = fmt.Sprintf("hist-%d-%s-%t", ts, v.CallCreatorJID.User, fromMe)
	}
	storeSyncedCall(id, v.CallCreatorJID, fromMe, ts, rec, true)
}

func storeSyncedCall(callID string, creator types.JID, fromMe bool, ts int64, rec *waSyncAction.CallLogRecord, fromFullSync bool) {
	if rec == nil || callDeleted(callID) {
		return
	}
	ownPN := types.EmptyJID
	ownLID := types.EmptyJID
	if wa() != nil && wa().Store != nil {
		ownPN = wa().Store.GetJID()
		ownLID = wa().Store.GetLID()
	}
	jid := callLogChatJID(rec, creator, fromMe, ownPN, ownLID, preferPN)
	if jid == "" || isIgnoredChatJID(jid) {
		return
	}
	isGroup := rec.GetGroupJID() != ""
	text := callLogText(rec, fromMe, isGroup)

	id := "call:" + callID
	if err := saveMessage(StoredMessage{
		ID: id, JID: jid, Text: text, Timestamp: ts,
		FromMe: fromMe, Status: "sent", MediaType: "call",
	}); err != nil {
		log.Printf("call log: storing %s failed: %v", callID, err)
		return
	}
	creatorStr := ""
	if !creator.IsEmpty() {
		creatorStr = creator.ToNonAD().String()
	}
	storeCallRecord(callRecord{
		CallID: callID, JID: jid, FromMe: fromMe, IsVideo: rec.GetIsVideo(), IsGroup: isGroup,
		Result: callResultName(rec.GetCallResult()), Duration: rec.GetDuration(),
		Timestamp: ts, Text: text, Authoritative: true,
		Creator: creatorStr, KeyChat: callLogKeyChat(rec, creator, fromMe, ownPN, ownLID),
	})
	if fromFullSync {
		res, err := db.Exec(`UPDATE chats SET
				preview_snippet = CASE WHEN ? >= COALESCE(timestamp, 0) THEN ? ELSE preview_snippet END,
				preview_sender = CASE WHEN ? >= COALESCE(timestamp, 0) THEN '' ELSE preview_sender END,
				preview_from_me = CASE WHEN ? >= COALESCE(timestamp, 0) THEN ? ELSE preview_from_me END,
				preview_is_reaction = CASE WHEN ? >= COALESCE(timestamp, 0) THEN 0 ELSE preview_is_reaction END,
				timestamp = MAX(COALESCE(timestamp, 0), ?)
			WHERE jid = ?`, ts, text, ts, ts, boolToInt(fromMe), ts, ts, jid)
		if err != nil {
			return
		}
		if n, _ := res.RowsAffected(); n == 0 {
			return
		}
	} else {
		db.Exec(`INSERT INTO chats (jid, timestamp, unread_count, preview_snippet, preview_sender, preview_from_me) VALUES (?, ?, 0, ?, '', ?)
			ON CONFLICT(jid) DO UPDATE SET
				preview_snippet=CASE WHEN excluded.timestamp >= COALESCE(chats.timestamp, 0) THEN excluded.preview_snippet ELSE chats.preview_snippet END,
				preview_sender=CASE WHEN excluded.timestamp >= COALESCE(chats.timestamp, 0) THEN '' ELSE chats.preview_sender END,
				preview_from_me=CASE WHEN excluded.timestamp >= COALESCE(chats.timestamp, 0) THEN excluded.preview_from_me ELSE chats.preview_from_me END,
				preview_is_reaction=CASE WHEN excluded.timestamp >= COALESCE(chats.timestamp, 0) THEN 0 ELSE chats.preview_is_reaction END,
				timestamp=MAX(timestamp, excluded.timestamp)`,
			jid, ts, text, boolToInt(fromMe))
	}
	var chatName string
	db.QueryRow(`SELECT name FROM chats WHERE jid = ?`, jid).Scan(&chatName)

	broadcastChats()
	broadcastEvent("message", map[string]interface{}{
		"jid":       jid,
		"from":      "",
		"id":        id,
		"text":      text,
		"from_me":   fromMe,
		"timestamp": ts,
		"has_media": false,
		"mediaType": "call",
		"chatName":  chatName,
		"call_id":   callID,
		"is_video":  rec.GetIsVideo(),
	})
}

func handleDeleteIndividualCallLog(v *events.DeleteIndividualCallLog) {
	peer := v.PeerJID
	if peer.IsEmpty() {
		return
	}
	jid := preferPN(peer.ToNonAD()).String()
	until := v.Timestamp.Unix()
	if v.Timestamp.IsZero() {
		until = time.Now().Unix()
	}
	if id, ok := deleteNewestCallRecord(jid, !v.IsIncoming, until); ok {
		db.Exec(`DELETE FROM messages WHERE id = ?`, "call:"+id)
		broadcastChats()
		broadcastEvent("call_logs_changed", map[string]interface{}{})
	}
}

// dropCallHistoryOnlyChats removes the chats older builds started for every
// call in the phone's synced call history: people only ever called, never
// written to. Their call lines stay in the messages table, so a chat started
// with them later still shows its calls.
func dropCallHistoryOnlyChats() {
	db.Exec(`DELETE FROM chats
		WHERE jid NOT LIKE '%@g.us' AND jid NOT LIKE '%@newsletter'
			AND COALESCE(pinned, 0) = 0 AND COALESCE(archived, 0) = 0 AND COALESCE(muted, 0) = 0 AND COALESCE(is_locked, 0) = 0
			AND EXISTS (SELECT 1 FROM messages m WHERE m.jid = chats.jid AND m.media_type = 'call')
			AND NOT EXISTS (SELECT 1 FROM messages m WHERE m.jid = chats.jid AND COALESCE(m.media_type, '') != 'call')`)
}
