package main

import (
	"fmt"
	"log"
	"strings"
	"sync"
	"time"

	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
)

// Live call history. Synced call_log records (call_log.go) only arrive once
// the phone writes them, which can be well after the call - or never, for a
// call only this device saw. So every call this device rings for or places is
// also written straight into its chat as soon as it ends, as the same kind of
// "call:<callID>" row. Nothing is sent to WhatsApp for it: when the phone's
// own record for the call syncs in, it lands on the same row and replaces
// ours, since the phone's is authoritative.

type liveCall struct {
	chatJID       string
	incoming      bool
	video         bool
	group         bool
	started       time.Time
	connected     time.Time
	localReject   bool
	localHangup   bool
	rejectedByOwn bool
}

var (
	liveCallsMu sync.Mutex
	liveCalls   = map[string]*liveCall{}
)

func trackLiveCall(callID string, chatJID types.JID, incoming, video, group bool) {
	if callID == "" || chatJID.IsEmpty() {
		return
	}
	liveCallsMu.Lock()
	liveCalls[callID] = &liveCall{
		chatJID:  preferPN(chatJID).String(),
		incoming: incoming,
		video:    video,
		group:    group,
		started:  time.Now(),
	}
	liveCallsMu.Unlock()
}

func updateLiveCall(callID string, fn func(*liveCall)) {
	liveCallsMu.Lock()
	if c := liveCalls[callID]; c != nil {
		fn(c)
	}
	liveCallsMu.Unlock()
}

func noteOwnDeviceReject(evt interface{}) {
	rej, ok := evt.(*events.CallReject)
	if !ok || !isOwnJID(rej.From) {
		return
	}
	updateLiveCall(rej.CallID, func(c *liveCall) {
		if c.incoming {
			c.rejectedByOwn = true
		}
	})
}

func noteOfflineMissedCall(evt interface{}) {
	offer, ok := evt.(*events.CallOffer)
	if !ok || !offer.Offline || offer.CallID == "" {
		return
	}
	chatJID := offer.CallCreator
	if chatJID.IsEmpty() {
		chatJID = offer.From
	}
	if offer.Group != nil && !offer.Group.GroupJID.IsEmpty() {
		chatJID = offer.Group.GroupJID
	} else if !offer.GroupJID.IsEmpty() {
		chatJID = offer.GroupJID
	}
	if chatJID.IsEmpty() {
		return
	}
	started := offer.Timestamp
	if started.IsZero() {
		started = time.Now()
	}
	c := &liveCall{
		chatJID:  preferPN(chatJID).String(),
		incoming: true,
		video:    offer.Video,
		group:    offer.Group != nil,
		started:  started,
	}
	text := liveCallText(c, "", started)
	log.Printf("Call %s rang while offline; logging it as missed", offer.CallID)
	storeLiveCallRow(offer.CallID, c, text, liveCallResult(c, ""), 0)
}

func finishLiveCall(callID, reason string) string {
	liveCallsMu.Lock()
	c := liveCalls[callID]
	delete(liveCalls, callID)
	liveCallsMu.Unlock()
	if c == nil {
		return ""
	}
	now := time.Now()
	text := liveCallText(c, reason, now)
	storeLiveCallRow(callID, c, text, liveCallResult(c, reason), liveCallDuration(c, now))
	return text
}

func liveCallText(c *liveCall, reason string, now time.Time) string {
	kind := "Voice call"
	if c.video {
		kind = "Video call"
	}
	if c.group {
		kind = "Group " + strings.ToLower(kind)
	}
	lower := strings.ToLower(kind)

	if !c.connected.IsZero() {
		direction := "Outgoing"
		if c.incoming {
			direction = "Incoming"
		}
		if d := callDurationText(int64(now.Sub(c.connected).Seconds())); d != "" {
			return fmt.Sprintf("%s %s, %s", direction, lower, d)
		}
		return fmt.Sprintf("%s %s", direction, lower)
	}

	if c.incoming {
		switch {
		case reason == "accepted_elsewhere":
			return fmt.Sprintf("%s, answered on another device", kind)
		case c.rejectedByOwn:
			return fmt.Sprintf("%s, declined on another device", kind)
		case c.localReject:
			return fmt.Sprintf("%s, you declined", kind)
		}
		return fmt.Sprintf("Missed %s", lower)
	}

	switch {
	case c.localHangup:
		return fmt.Sprintf("%s, cancelled", kind)
	case reason == "rejected" || reason == "busy":
		return fmt.Sprintf("%s, declined", kind)
	case strings.HasPrefix(reason, "server:") || reason == "offer_failed" || reason == "connection_lost":
		return fmt.Sprintf("%s, failed", kind)
	}
	return fmt.Sprintf("%s, no answer", kind)
}

func liveCallResult(c *liveCall, reason string) string {
	if !c.connected.IsZero() {
		return "connected"
	}
	if c.incoming {
		switch {
		case reason == "accepted_elsewhere":
			return "accepted_elsewhere"
		case c.rejectedByOwn || c.localReject:
			return "rejected"
		}
		return "missed"
	}
	switch {
	case c.localHangup:
		return "cancelled"
	case reason == "rejected" || reason == "busy":
		return "rejected"
	case strings.HasPrefix(reason, "server:") || reason == "offer_failed" || reason == "connection_lost":
		return "failed"
	}
	return "missed"
}

func liveCallDuration(c *liveCall, now time.Time) int64 {
	if c.connected.IsZero() {
		return 0
	}
	return int64(now.Sub(c.connected).Seconds())
}

func storeLiveCallRow(callID string, c *liveCall, text, result string, duration int64) {
	id := "call:" + callID
	ts := c.started.Unix()
	fromMe := !c.incoming
	res, err := db.Exec(`INSERT INTO messages (id, jid, from_me, text, timestamp, sender_jid, status, media_type)
		VALUES (?, ?, ?, ?, ?, '', 'sent', 'call') ON CONFLICT(id) DO NOTHING`,
		id, c.chatJID, fromMe, text, ts)
	if err != nil {
		log.Printf("live call log: storing %s failed: %v", callID, err)
		return
	}
	if n, _ := res.RowsAffected(); n == 0 {
		return
	}
	storeCallRecord(callRecord{
		CallID: callID, JID: c.chatJID, FromMe: fromMe, IsVideo: c.video, IsGroup: c.group,
		Result: result, Duration: duration, Timestamp: ts, Text: text,
	})
	db.Exec(`INSERT INTO chats (jid, timestamp, unread_count, preview_snippet, preview_sender, preview_from_me) VALUES (?, ?, 0, ?, '', ?)
		ON CONFLICT(jid) DO UPDATE SET
			timestamp=MAX(timestamp, excluded.timestamp),
			preview_snippet=excluded.preview_snippet,
			preview_sender='',
			preview_from_me=excluded.preview_from_me,
			preview_is_reaction=0,
			preview_reaction_emoji=''`,
		c.chatJID, ts, text, boolToInt(fromMe))
	var chatName string
	db.QueryRow(`SELECT COALESCE(name, '') FROM chats WHERE jid = ?`, c.chatJID).Scan(&chatName)
	broadcastChats()
	broadcastEvent("message", map[string]interface{}{
		"jid":       c.chatJID,
		"from":      "",
		"id":        id,
		"text":      text,
		"from_me":   fromMe,
		"timestamp": ts,
		"has_media": false,
		"mediaType": "call",
		"chatName":  chatName,
		"call_id":   callID,
		"is_video":  c.video,
	})
}
