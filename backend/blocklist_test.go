package main

import (
	"testing"

	"go.mau.fi/whatsmeow/types"
)

var (
	testPN  = types.NewJID("919800000001", types.DefaultUserServer)
	testLID = types.NewJID("100000012345678", types.HiddenUserServer)
	loneLID = types.NewJID("100000099999999", types.HiddenUserServer)
)

func testTwin(jid types.JID) types.JID {
	switch jid {
	case testPN:
		return testLID
	case testLID:
		return testPN
	}
	return types.EmptyJID
}

func TestBuildBlockedEntriesResolvesLIDToPhone(t *testing.T) {
	names := map[types.JID]string{testPN: "Zoya", loneLID: "asha"}
	nameFor := func(j types.JID) string { return names[j] }
	// The same person under both addresses must come out once.
	got := buildBlockedEntries([]types.JID{testLID, loneLID, testPN}, testTwin, nameFor)
	if len(got) != 2 {
		t.Fatalf("want 2 entries, got %+v", got)
	}
	if got[0].Name != "asha" || got[0].JID != loneLID.String() || got[0].Alt != "" || got[0].Phone != "" {
		t.Fatalf("unmapped LID entry: %+v", got[0])
	}
	if got[1].JID != testPN.String() || got[1].Alt != testLID.String() || got[1].Phone != "+919800000001" {
		t.Fatalf("LID-keyed block should surface under the phone JID: %+v", got[1])
	}
}

func TestWithBlockChange(t *testing.T) {
	stored := []types.JID{testLID, loneLID}
	// Unblocking by phone JID removes the LID-keyed entry.
	got := withBlockChange(stored, testPN, false, testTwin)
	if len(got) != 1 || got[0] != loneLID {
		t.Fatalf("unblock by phone JID: %v", got)
	}
	// Blocking someone already listed under their other address doesn't
	// leave them in twice.
	got = withBlockChange(stored, testPN, true, testTwin)
	if len(got) != 2 || got[0] != loneLID || got[1] != testPN {
		t.Fatalf("re-block by phone JID: %v", got)
	}
	// A device suffix on the notification's JID is ignored.
	withDevice := types.JID{User: loneLID.User, Server: loneLID.Server, Device: 3}
	if got = withBlockChange(stored, withDevice, false, testTwin); len(got) != 1 || got[0] != testLID {
		t.Fatalf("unblock with device suffix: %v", got)
	}
}
