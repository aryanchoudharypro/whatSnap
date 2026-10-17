package main

import (
	"testing"

	"go.mau.fi/whatsmeow/types"
)

func TestPrivacyListChanges(t *testing.T) {
	lid := func(u string) types.JID { return types.NewJID(u, types.HiddenUserServer) }
	pn := func(u string) types.JID { return types.NewJID(u, types.DefaultUserServer) }
	current := []privacyListPerson{{LID: lid("1"), PN: pn("911")}, {LID: lid("2")}}
	// 1 stays (matched by phone alone), 2 goes, 3 is added once.
	wanted := []privacyListPerson{{PN: pn("911")}, {LID: lid("3"), PN: pn("913")}, {LID: lid("3")}}
	changes := privacyListChanges(current, wanted)
	if len(changes) != 2 {
		t.Fatalf("changes = %+v", changes)
	}
	if changes[0].Action != types.PrivacyListActionRemove || changes[0].JID != lid("2") {
		t.Errorf("first change = %+v, want remove 2", changes[0])
	}
	if changes[1].Action != types.PrivacyListActionAdd || changes[1].JID != lid("3") || changes[1].PN != pn("913") {
		t.Errorf("second change = %+v, want add 3", changes[1])
	}
}

func TestPeopleFromListShowsPhoneJID(t *testing.T) {
	list := &types.PrivacyDisallowedList{Users: []types.PrivacyListUser{
		{JID: types.NewJID("1", types.HiddenUserServer), PN: types.NewJID("911", types.DefaultUserServer)},
		{JID: types.NewJID("2", types.HiddenUserServer)},
	}}
	noTwin := func(types.JID) types.JID { return types.EmptyJID }
	people := peopleFromList(list, noTwin)
	if got := people[0].shownJID().String(); got != "911@s.whatsapp.net" {
		t.Errorf("shown = %s", got)
	}
	if got := people[1].shownJID().String(); got != "2@lid" {
		t.Errorf("shown = %s", got)
	}
}
