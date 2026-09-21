package main

import (
	"testing"
	"time"

	"go.mau.fi/whatsmeow/proto/waSyncAction"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
)

func jidOf(t *testing.T, s string) types.JID {
	t.Helper()
	j, err := types.ParseJID(s)
	if err != nil {
		t.Fatalf("parse %q: %v", s, err)
	}
	return j
}

func TestCallLogChatJIDResolvesThePeer(t *testing.T) {
	ownPN := jidOf(t, "5511000000001@s.whatsapp.net")
	ownLID := jidOf(t, "9988776655443@lid")
	peer := "5511999990000@s.whatsapp.net"
	peerLID := "7766554433221@lid"
	// Stands in for preferPN: the LID->PN mapping the store would supply.
	normalize := func(j types.JID) types.JID {
		if j.Server == types.HiddenUserServer && j.User == "7766554433221" {
			return jidOf(t, peer)
		}
		return j
	}
	identity := func(j types.JID) types.JID { return j }

	t.Run("group call names its group", func(t *testing.T) {
		rec := &waSyncAction.CallLogRecord{GroupJID: proto.String("120363000000000000@g.us")}
		got := callLogChatJID(rec, ownPN, true, ownPN, ownLID, identity)
		if got != "120363000000000000@g.us" {
			t.Errorf("group chat = %q, want the group JID", got)
		}
	})

	t.Run("incoming 1:1 is the creator", func(t *testing.T) {
		got := callLogChatJID(&waSyncAction.CallLogRecord{}, jidOf(t, peer), false, ownPN, ownLID, identity)
		if got != peer {
			t.Errorf("incoming chat = %q, want %q", got, peer)
		}
	})

	// The case the creator cannot answer: on an outgoing call the creator is us,
	// so the peer has to come from the participant list.
	t.Run("outgoing 1:1 is the non-self participant", func(t *testing.T) {
		rec := &waSyncAction.CallLogRecord{Participants: []*waSyncAction.CallLogRecord_ParticipantInfo{
			{UserJID: proto.String(ownPN.String())},
			{UserJID: proto.String(peer)},
		}}
		got := callLogChatJID(rec, ownPN, true, ownPN, ownLID, identity)
		if got != peer {
			t.Errorf("outgoing chat = %q, want %q", got, peer)
		}
	})

	// A peer LID whose digits spell our phone number must not be mistaken for us
	// and skipped, or the call would land in no chat at all.
	t.Run("peer LID spelling our phone number is still the peer", func(t *testing.T) {
		lookalike := ownPN.User + "@lid"
		rec := &waSyncAction.CallLogRecord{Participants: []*waSyncAction.CallLogRecord_ParticipantInfo{
			{UserJID: proto.String(ownLID.String())},
			{UserJID: proto.String(lookalike)},
		}}
		if got := callLogChatJID(rec, ownLID, true, ownPN, ownLID, identity); got != lookalike {
			t.Errorf("chat = %q, want %q", got, lookalike)
		}
	})

	// A @lid peer must land in the existing phone-JID-keyed chat, not spawn a
	// second nameless one beside it.
	t.Run("incoming from a LID maps to the phone chat", func(t *testing.T) {
		got := callLogChatJID(&waSyncAction.CallLogRecord{}, jidOf(t, peerLID), false, ownPN, ownLID, normalize)
		if got != peer {
			t.Errorf("incoming LID chat = %q, want the mapped phone JID %q", got, peer)
		}
	})

	t.Run("outgoing to a LID participant maps to the phone chat", func(t *testing.T) {
		rec := &waSyncAction.CallLogRecord{Participants: []*waSyncAction.CallLogRecord_ParticipantInfo{
			{UserJID: proto.String(ownPN.String())},
			{UserJID: proto.String(peerLID)},
		}}
		got := callLogChatJID(rec, ownPN, true, ownPN, ownLID, normalize)
		if got != peer {
			t.Errorf("outgoing LID chat = %q, want the mapped phone JID %q", got, peer)
		}
	})

	// A group is already its own canonical id and must pass through untouched.
	t.Run("group JID is never remapped", func(t *testing.T) {
		rec := &waSyncAction.CallLogRecord{GroupJID: proto.String("120363000000000000@g.us")}
		panicking := func(types.JID) types.JID { t.Fatal("group JID must not be normalized"); return types.EmptyJID }
		if got := callLogChatJID(rec, ownPN, true, ownPN, ownLID, panicking); got != "120363000000000000@g.us" {
			t.Errorf("group chat = %q", got)
		}
	})

	t.Run("nothing to resolve yields no chat", func(t *testing.T) {
		rec := &waSyncAction.CallLogRecord{Participants: []*waSyncAction.CallLogRecord_ParticipantInfo{
			{UserJID: proto.String(ownPN.String())},
		}}
		if got := callLogChatJID(rec, ownPN, true, ownPN, ownLID, identity); got != "" {
			t.Errorf("chat = %q, want empty", got)
		}
	})
}

func TestCallDurationTextReadsAsProse(t *testing.T) {
	cases := []struct {
		secs int64
		want string
	}{
		{0, ""},
		{1, "1 second"},
		{45, "45 seconds"},
		{60, "1 minute"},
		{125, "2 minutes 5 seconds"},
		{600, "10 minutes"},
		{3600, "1 hour"},
		{3725, "1 hour 2 minutes"},
	}
	for _, tc := range cases {
		if got := callDurationText(tc.secs); got != tc.want {
			t.Errorf("callDurationText(%d) = %q, want %q", tc.secs, got, tc.want)
		}
	}
}

func TestCallLogTextStatesDirectionAndOutcome(t *testing.T) {
	connected := func(secs int64, video bool) *waSyncAction.CallLogRecord {
		return &waSyncAction.CallLogRecord{
			CallResult: waSyncAction.CallLogRecord_CONNECTED.Enum(),
			Duration:   proto.Int64(secs),
			IsVideo:    proto.Bool(video),
		}
	}
	result := func(r waSyncAction.CallLogRecord_CallResult) *waSyncAction.CallLogRecord {
		return &waSyncAction.CallLogRecord{CallResult: r.Enum()}
	}
	cases := []struct {
		name    string
		rec     *waSyncAction.CallLogRecord
		fromMe  bool
		isGroup bool
		want    string
	}{
		{"outgoing connected voice", connected(125, false), true, false, "Outgoing voice call, 2 minutes 5 seconds"},
		{"incoming connected video", connected(60, true), false, false, "Incoming video call, 1 minute"},
		{"connected with no duration", connected(0, false), false, false, "Incoming voice call"},
		{"missed incoming", result(waSyncAction.CallLogRecord_MISSED), false, false, "Missed voice call"},
		{"our call unanswered", result(waSyncAction.CallLogRecord_MISSED), true, false, "Voice call, no answer"},
		{"they declined", result(waSyncAction.CallLogRecord_REJECTED), true, false, "Voice call, declined"},
		{"we declined", result(waSyncAction.CallLogRecord_REJECTED), false, false, "Voice call, you declined"},
		{"answered elsewhere", result(waSyncAction.CallLogRecord_ACCEPTEDELSEWHERE), false, false, "Voice call, answered on another device"},
		{"group call", connected(600, false), true, true, "Outgoing group voice call, 10 minutes"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := callLogText(tc.rec, tc.fromMe, tc.isGroup); got != tc.want {
				t.Errorf("callLogText = %q, want %q", got, tc.want)
			}
		})
	}
}

// History order must follow when the call happened, not when it synced.
func TestCallLogTimestampPrefersTheCallsOwnTime(t *testing.T) {
	mutation := time.Unix(1_700_009_999, 0)
	rec := &waSyncAction.CallLogRecord{StartTime: proto.Int64(1_700_000_000)}
	if got := callLogTimestamp(rec, mutation); got != 1_700_000_000 {
		t.Errorf("timestamp = %d, want the call's start time", got)
	}
	if got := callLogTimestamp(&waSyncAction.CallLogRecord{}, mutation); got != mutation.Unix() {
		t.Errorf("fallback timestamp = %d, want the mutation's %d", got, mutation.Unix())
	}
}
