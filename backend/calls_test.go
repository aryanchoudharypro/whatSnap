package main

import (
	"testing"

	"github.com/purpshell/meowcaller/signaling"
)

// peerVideoEvent decides what the frontend gets told about the other side's
// camera. Only states that mean something to a voice-only client are mapped;
// the rest must stay silent rather than announce a state we cannot act on.
func TestPeerVideoEventMapsOnlyAnnouncableStates(t *testing.T) {
	cases := []struct {
		name string
		raw  int
		want string
	}{
		{"peer enabled their camera", signaling.VideoStateEnabled, "on"},
		{"peer disabled their camera", signaling.VideoStateDisabled, "off"},
		{"peer stopped video", signaling.VideoStateStopped, "off"},
		{"peer asks to switch to video", signaling.VideoStateUpgradeRequest, "upgrade_requested"},
		{"peer asks to switch to video, v2", signaling.VideoStateUpgradeRequestV2, "upgrade_requested"},
		// Accept and reject only ever answer an upgrade we asked for, and we
		// never ask: announcing them would describe a negotiation the user was
		// never part of.
		{"upgrade accept is not ours to hear", signaling.VideoStateUpgradeAccept, ""},
		{"upgrade reject is not ours to hear", signaling.VideoStateUpgradeReject, ""},
		{"unknown state stays silent", 999, ""},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := peerVideoEvent(tc.raw); got != tc.want {
				t.Errorf("peerVideoEvent(%d) = %q, want %q", tc.raw, got, tc.want)
			}
		})
	}
}
