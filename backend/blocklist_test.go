package main

import (
	"testing"

	"go.mau.fi/whatsmeow/types"
)

var (
	testPN     = types.NewJID("919800000001", types.DefaultUserServer)
	testLID    = types.NewJID("100000012345678", types.HiddenUserServer)
	testOldLID = types.NewJID("1099645911040", types.HiddenUserServer)
	mappedLID  = types.NewJID("100000055555555", types.HiddenUserServer)
	mappedPN   = types.NewJID("919800000002", types.DefaultUserServer)
	loneLID    = types.NewJID("100000099999999", types.HiddenUserServer)
)

// testTwin knows one pair the server's list doesn't spell out.
func testTwin(jid types.JID) types.JID {
	switch jid {
	case mappedPN:
		return mappedLID
	case mappedLID:
		return mappedPN
	}
	return types.EmptyJID
}

func testRows() []blockedRow {
	return []blockedRow{
		{JID: testLID, PN: testPN},
		{JID: loneLID},
		{JID: testOldLID, PN: testPN},
		{JID: mappedLID},
	}
}

func TestBuildBlockedEntriesOnePerPerson(t *testing.T) {
	names := map[types.JID]string{testPN: "Zoya", loneLID: "asha", mappedPN: "Meera"}
	got := buildBlockedEntries(testRows(), testTwin, func(j types.JID) string { return names[j] })
	if len(got) != 3 {
		t.Fatalf("want 3 people, got %+v", got)
	}
	if got[0].Name != "asha" || got[0].JID != loneLID.String() || len(got[0].Alts) != 0 || got[0].Phone != "" {
		t.Fatalf("LID with no known number: %+v", got[0])
	}
	if got[1].Name != "Meera" || got[1].JID != mappedPN.String() || got[1].Phone != "+919800000002" {
		t.Fatalf("LID resolved through the store: %+v", got[1])
	}
	// Both of their LIDs fold into the one entry under the server's number.
	z := got[2]
	if z.Name != "Zoya" || z.JID != testPN.String() || z.Phone != "+919800000001" ||
		len(z.Alts) != 2 || z.Alts[0] != testLID.String() || z.Alts[1] != testOldLID.String() {
		t.Fatalf("person under two LIDs: %+v", z)
	}
}

func TestBlockedAddressesOf(t *testing.T) {
	// By phone JID or by either LID, unblocking reaches both LIDs.
	for _, jid := range []types.JID{testPN, testLID, testOldLID} {
		got := blockedAddressesOf(testRows(), jid, testTwin)
		if len(got) != 2 || got[0] != testLID || got[1] != testOldLID {
			t.Fatalf("addresses of %s: %v", jid, got)
		}
	}
	if got := blockedAddressesOf(testRows(), mappedPN, testTwin); len(got) != 1 || got[0] != mappedLID {
		t.Fatalf("addresses through the store's mapping: %v", got)
	}
	if got := blockedAddressesOf(testRows(), types.NewJID("911", types.DefaultUserServer), testTwin); len(got) != 0 {
		t.Fatalf("someone not blocked: %v", got)
	}
}
