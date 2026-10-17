package main

import (
	"testing"
	"time"

	"go.mau.fi/whatsmeow/proto/waHistorySync"
	"go.mau.fi/whatsmeow/proto/waWeb"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
)

func TestHistoryConversationIsChat(t *testing.T) {
	pn := types.NewJID("919999999999", types.DefaultUserServer)
	group := types.NewJID("120363000000000000", types.GroupServer)
	future := uint64(time.Now().Add(time.Hour).Unix())
	cases := []struct {
		name string
		conv *waHistorySync.Conversation
		jid  types.JID
		want bool
	}{
		{"old chat with only a timestamp", &waHistorySync.Conversation{ConversationTimestamp: proto.Uint64(1500000000)}, pn, true},
		{"last message timestamp only", &waHistorySync.Conversation{LastMsgTimestamp: proto.Uint64(1500000000)}, pn, true},
		{"has messages", &waHistorySync.Conversation{Messages: []*waHistorySync.HistorySyncMsg{{Message: &waWeb.WebMessageInfo{}}}}, pn, true},
		{"pinned", &waHistorySync.Conversation{Pinned: proto.Uint32(1)}, pn, true},
		{"marked unread", &waHistorySync.Conversation{MarkedAsUnread: proto.Bool(true)}, pn, true},
		{"muted until later", &waHistorySync.Conversation{MuteEndTime: proto.Uint64(future)}, pn, true},
		{"mute that already ended", &waHistorySync.Conversation{MuteEndTime: proto.Uint64(1500000000)}, pn, false},
		{"privacy token only", &waHistorySync.Conversation{TcToken: []byte{1}, TcTokenTimestamp: proto.Uint64(1500000000)}, pn, false},
		{"name only", &waHistorySync.Conversation{Name: proto.String("Someone")}, pn, false},
		{"group with nothing", &waHistorySync.Conversation{}, group, true},
	}
	for _, c := range cases {
		if got := historyConversationIsChat(c.conv, c.jid); got != c.want {
			t.Errorf("%s: got %v, want %v", c.name, got, c.want)
		}
	}
}

func TestHistoryMuteActive(t *testing.T) {
	now := time.Now()
	if !historyMuteActive(^uint64(0)) {
		t.Error("-1 should be muted forever")
	}
	if !historyMuteActive(uint64(now.Add(time.Hour).UnixMilli())) {
		t.Error("a future time in milliseconds should be muted")
	}
	if historyMuteActive(uint64(now.Add(-time.Hour).Unix())) {
		t.Error("a past time should not be muted")
	}
	if historyMuteActive(0) {
		t.Error("0 should not be muted")
	}
}

func TestConversationLIDPair(t *testing.T) {
	lid := types.NewJID("123456789012345", types.HiddenUserServer)
	pn := types.NewJID("919999999999", types.DefaultUserServer)

	pair, ok := conversationLIDPair(&waHistorySync.Conversation{PnJID: proto.String("919999999999@c.us")}, lid)
	if !ok || pair.LID != lid || pair.PN != pn {
		t.Errorf("LID chat with a @c.us number: got %v %v", pair, ok)
	}
	pair, ok = conversationLIDPair(&waHistorySync.Conversation{LidJID: proto.String(lid.String())}, pn)
	if !ok || pair.LID != lid || pair.PN != pn {
		t.Errorf("phone chat with its LID: got %v %v", pair, ok)
	}
	if _, ok = conversationLIDPair(&waHistorySync.Conversation{}, lid); ok {
		t.Error("a LID chat naming no number has no pair")
	}
	if _, ok = conversationLIDPair(&waHistorySync.Conversation{PnJID: proto.String(pn.String())}, types.NewJID("1", types.GroupServer)); ok {
		t.Error("a group has no pair")
	}
}

func TestWireAddresses(t *testing.T) {
	lid := types.NewJID("123456789012345", types.HiddenUserServer)
	pn := types.NewJID("919999999999", types.DefaultUserServer)
	group := types.NewJID("120363000000000000", types.GroupServer)
	lidDevice := lid
	lidDevice.Device = 3

	if got := wireChat(lid, pn); got != lid.String() {
		t.Errorf("LID chat stored under its number: got %q", got)
	}
	if got := wireChat(pn, pn); got != "" {
		t.Errorf("phone chat: got %q", got)
	}
	if got := wireSender(group, lidDevice, pn); got != lid.String() {
		t.Errorf("LID sender in a group: got %q", got)
	}
	if got := wireSender(group, pn, pn); got != "" {
		t.Errorf("phone sender stored as is: got %q", got)
	}
	if got := wireSender(pn, lid, pn); got != "" {
		t.Errorf("1:1 messages keep no wire sender: got %q", got)
	}
}
