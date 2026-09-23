package main

import (
	"testing"
	"time"
)

func TestLiveCallText(t *testing.T) {
	now := time.Unix(1_000_000, 0)
	connectedAt := now.Add(-95 * time.Second)
	cases := []struct {
		name   string
		call   liveCall
		reason string
		want   string
	}{
		{"answered here, incoming", liveCall{incoming: true, connected: connectedAt}, "hangup", "Incoming voice call, 1 minute 35 seconds"},
		{"answered here, outgoing video", liveCall{video: true, connected: connectedAt}, "hangup", "Outgoing video call, 1 minute 35 seconds"},
		{"answered on the phone", liveCall{incoming: true}, "accepted_elsewhere", "Voice call, answered on another device"},
		{"declined on the phone", liveCall{incoming: true, rejectedByOwn: true}, "rejected", "Voice call, declined on another device"},
		{"declined here", liveCall{incoming: true, localReject: true}, "rejected", "Voice call, you declined"},
		{"caller gave up", liveCall{incoming: true}, "timeout", "Missed voice call"},
		{"rang while busy", liveCall{incoming: true}, "busy", "Missed voice call"},
		{"group missed", liveCall{incoming: true, group: true}, "", "Missed group voice call"},
		{"they declined", liveCall{}, "rejected", "Voice call, declined"},
		{"we cancelled", liveCall{localHangup: true}, "hangup", "Voice call, cancelled"},
		{"nobody picked up", liveCall{}, "timeout", "Voice call, no answer"},
		{"server refused", liveCall{}, "server:403", "Voice call, failed"},
	}
	for _, tc := range cases {
		if got := liveCallText(&tc.call, tc.reason, now); got != tc.want {
			t.Errorf("%s: got %q, want %q", tc.name, got, tc.want)
		}
	}
}
