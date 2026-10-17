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

// Call history arrives as app-state `call_log` mutations rather than as live
// call stanzas, and it is the only channel that carries a call handled on the
// phone: a call the handset places or answers puts nothing on this device's
// socket. Each record is stored as an ordinary message row (media_type "call")
// so it persists, lands in history order, and reads like any other line.

// callLogChatJID works out which chat a call record belongs to. A group call
// names its group outright. A 1:1 call names only its creator, which is us on an
// outgoing call, so the peer has to come from the participant list instead.
//
// Every 1:1 answer goes through normalize (preferPN in production). A call
// record addresses its peer by whichever form the authoring client used, and a
// @lid stored as-is does not match the existing phone-JID-keyed chat: the row
// lands in a second, nameless chat sitting beside the real one instead of in the
// conversation it belongs to. Self-detection still runs on the RAW jid, before
// normalizing, because that comparison is keyed on the addressing mode.
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
	isOwn := func(jid types.JID) bool {
		// Keyed on the addressing mode, never across it: a peer LID whose digits
		// spell our phone number is not us.
		switch jid.Server {
		case types.HiddenUserServer:
			return !ownLID.IsEmpty() && ownLID.User == jid.User
		case types.DefaultUserServer:
			return !ownPN.IsEmpty() && ownPN.User == jid.User
		}
		return false
	}
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

// callLogTimestamp prefers the call's own start time. The mutation's timestamp
// is its envelope's, and sorting history by it would file a call under when it
// happened to sync rather than when it happened.
func callLogTimestamp(rec *waSyncAction.CallLogRecord, mutation time.Time) int64 {
	if st := rec.GetStartTime(); st > 0 {
		return st
	}
	if !mutation.IsZero() {
		return mutation.Unix()
	}
	return time.Now().Unix()
}

// handleCallLogSync stores one synced call record in its chat's history. Calls
// are history, not an alert: the row goes in with no unread bump, so a chat does
// not light up as having something to read because the phone rang.
func handleCallLogSync(v *events.CallLogSync) {
	rec := v.Record
	if rec == nil {
		return
	}
	ownPN := types.EmptyJID
	if wa() != nil && wa().Store != nil {
		ownPN = wa().Store.GetJID()
	}
	ownLID := types.EmptyJID
	if wa() != nil && wa().Store != nil {
		ownLID = wa().Store.GetLID()
	}
	jid := callLogChatJID(rec, v.CallCreatorJID, v.FromMe, ownPN, ownLID, preferPN)
	if jid == "" || isIgnoredChatJID(jid) {
		return
	}
	isGroup := rec.GetGroupJID() != ""
	text := callLogText(rec, v.FromMe, isGroup)
	ts := callLogTimestamp(rec, v.Timestamp)

	// The call id is stable across the devices that sync this record, so a
	// re-sync or a full sync updates the same row instead of adding another.
	id := "call:" + v.CallID
	if err := saveMessage(StoredMessage{
		ID: id, JID: jid, Text: text, Timestamp: ts,
		FromMe: v.FromMe, Status: "sent", MediaType: "call",
	}); err != nil {
		log.Printf("call log: storing %s failed: %v", v.CallID, err)
		return
	}
	// A call is the chat's latest line only if nothing newer is there - a
	// full sync replays calls from long ago.
	if v.FromFullSync {
		// The phone's call history is the Calls tab's, not a list of chats:
		// WhatsApp Web doesn't start a chat for everyone ever called. The
		// row above still puts the call in that chat's history, should it
		// exist or be started later.
		res, err := db.Exec(`UPDATE chats SET
				preview_snippet = CASE WHEN ? >= COALESCE(timestamp, 0) THEN ? ELSE preview_snippet END,
				preview_sender = CASE WHEN ? >= COALESCE(timestamp, 0) THEN '' ELSE preview_sender END,
				preview_from_me = CASE WHEN ? >= COALESCE(timestamp, 0) THEN ? ELSE preview_from_me END,
				preview_is_reaction = CASE WHEN ? >= COALESCE(timestamp, 0) THEN 0 ELSE preview_is_reaction END,
				timestamp = MAX(COALESCE(timestamp, 0), ?)
			WHERE jid = ?`, ts, text, ts, ts, boolToInt(v.FromMe), ts, ts, jid)
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
			jid, ts, text, boolToInt(v.FromMe))
	}
	var chatName string
	db.QueryRow(`SELECT name FROM chats WHERE jid = ?`, jid).Scan(&chatName)

	broadcastChats()
	broadcastEvent("message", map[string]interface{}{
		"jid":       jid,
		"from":      "",
		"id":        id,
		"text":      text,
		"from_me":   v.FromMe,
		"timestamp": ts,
		"has_media": false,
		"mediaType": "call",
		"chatName":  chatName,
		"call_id":   v.CallID,
		"is_video":  rec.GetIsVideo(),
	})
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
