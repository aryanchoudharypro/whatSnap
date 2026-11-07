package main

import (
	"testing"
	"time"

	"go.mau.fi/whatsmeow/types"
)

func TestMemberLabelsKeepTheNewest(t *testing.T) {
	openDisappearingTestDB(t)
	group := types.NewJID("120363000000000001", types.GroupServer)
	alice := types.NewJID("15550001111", types.DefaultUserServer)
	bob := types.NewJID("15550002222", types.DefaultUserServer)
	acc.noteParticipantLabels(group, []types.GroupParticipant{
		{JID: alice, Label: "Treasurer", LabelSetAt: time.Unix(200, 0)},
		{JID: bob},
	})
	acc.storeMemberLabel(group.String(), acc.memberLabelKey(alice), "Old", 100)
	if got := acc.memberLabels(group.String()); got[alice.String()] != "Treasurer" || len(got) != 1 {
		t.Fatalf("labels = %v", got)
	}
	acc.storeMemberLabel(group.String(), acc.memberLabelKey(alice), "", 300)
	if got := acc.memberLabels(group.String()); len(got) != 0 {
		t.Fatalf("a cleared tag must not show, got %v", got)
	}
}
