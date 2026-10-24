package main

import (
	"strings"
	"testing"
	"time"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

func TestVideoNoteIsAVideo(t *testing.T) {
	msg := &waE2E.Message{PtvMessage: &waE2E.VideoMessage{Seconds: proto.Uint32(75), Mimetype: proto.String("video/mp4")}}
	if got := extractTextRaw(msg); got != "[Video Note] (1:15)" {
		t.Fatalf("text = %q", got)
	}
	if mediaType, _, _, _ := mediaInfoFromMessage(msg); mediaType != "video" {
		t.Fatalf("media type = %q, want video", mediaType)
	}
	// The download path reads the carrier; it must share the pointer so a
	// media retry's new DirectPath lands in the stored message.
	if mediaCarrier(msg).GetVideoMessage() != msg.GetPtvMessage() {
		t.Fatal("carrier doesn't share the video note")
	}
}

func TestBookkeepingMessagesAreNotShown(t *testing.T) {
	for name, msg := range map[string]*waE2E.Message{
		"pin":            {PinInChatMessage: &waE2E.PinInChatMessage{}},
		"keep":           {KeepInChatMessage: &waE2E.KeepInChatMessage{}},
		"event rsvp":     {EncEventResponseMessage: &waE2E.EncEventResponseMessage{}},
		"comment":        {EncCommentMessage: &waE2E.EncCommentMessage{}},
		"history bundle": {MessageHistoryBundle: &waE2E.MessageHistoryBundle{}},
	} {
		if got := extractTextRaw(msg); got != "[Protocol Message]" {
			t.Errorf("%s: got %q", name, got)
		}
	}
}

func TestLessCommonMessagesAreDescribed(t *testing.T) {
	for want, msg := range map[string]*waE2E.Message{
		"[Event] Launch":           {EventMessage: &waE2E.EventMessage{Name: proto.String("Launch")}},
		"[Scheduled Call] Standup": {ScheduledCallCreationMessage: &waE2E.ScheduledCallCreationMessage{Title: proto.String("Standup")}},
		"[Sticker Pack] Cats":      {StickerPackMessage: &waE2E.StickerPackMessage{Name: proto.String("Cats")}},
		"[Poll results] Lunch?":    {PollResultSnapshotMessage: &waE2E.PollResultSnapshotMessage{Name: proto.String("Lunch?")}},
	} {
		if got := extractTextRaw(msg); got != want {
			t.Errorf("got %q, want %q", got, want)
		}
	}
	wrapped := &waE2E.Message{AssociatedChildMessage: &waE2E.FutureProofMessage{
		Message: &waE2E.Message{ImageMessage: &waE2E.ImageMessage{Caption: proto.String("hi")}},
	}}
	if got := extractTextRaw(wrapped); got != "[Image] hi" {
		t.Errorf("associated child: got %q", got)
	}
}

func TestDisappearingTexts(t *testing.T) {
	cases := map[uint32]string{0: "off", 86400: "24 hours", 604800: "7 days", 7776000: "90 days", 3600: "1 hour"}
	for seconds, want := range cases {
		if got := disappearingDurationText(seconds); got != want {
			t.Errorf("%d: got %q, want %q", seconds, got, want)
		}
	}
	if got := disappearingChangeText("You", false, 0); got != "You turned off disappearing messages." {
		t.Errorf("off: got %q", got)
	}
	if got := disappearingChangeText("You", true, 604800); !strings.Contains(got, "7 days") {
		t.Errorf("on: got %q", got)
	}
}

func TestGroupSettingLines(t *testing.T) {
	sender := types.NewJID("111", types.DefaultUserServer)
	lines := groupChangeMessages(&events.GroupInfo{
		Sender:    &sender,
		Timestamp: time.Unix(1700000000, 0),
		Name:      &types.GroupName{Name: "Trip"},
		Announce:  &types.GroupAnnounce{IsAnnounce: true},
		Ephemeral: &types.GroupEphemeral{IsEphemeral: true, DisappearingTimer: 86400},
	})
	if len(lines) != 3 {
		t.Fatalf("got %d lines: %q", len(lines), lines)
	}
	// Index 0 must be the subject line: history sync's GROUP_CHANGE_SUBJECT
	// stub is stored under the same id, so a resync doesn't repeat it.
	if !strings.Contains(lines[0], `changed the group name to "Trip"`) {
		t.Errorf("line 0 = %q", lines[0])
	}
	if !strings.Contains(lines[1], "only admins to send messages") || !strings.Contains(lines[2], "24 hours") {
		t.Errorf("lines = %q", lines)
	}
	if lines := groupChangeMessages(&events.GroupInfo{Topic: &types.GroupTopic{TopicDeleted: true}}); len(lines) != 1 ||
		lines[0] != "Someone deleted the group description" {
		t.Errorf("senderless topic delete = %q", lines)
	}
}

func TestMessageHistoryNotice(t *testing.T) {
	notice := &waE2E.MessageHistoryNotice{
		MessageHistoryMetadata: &waE2E.MessageHistoryMetadata{
			HistoryReceivers: []string{"919800000005@s.whatsapp.net"},
		},
	}
	sender, _ := types.ParseJID("919800000006@s.whatsapp.net")
	got := messageHistoryNoticeText(notice, sender)
	if !strings.Contains(got, "shared recent message history with") || !strings.Contains(got, "919800000005") {
		t.Fatalf("notice text = %q", got)
	}
}
