package main

import (
	"testing"

	"go.mau.fi/whatsmeow/proto/waCommon"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"google.golang.org/protobuf/proto"
)

func TestApplyMentions(t *testing.T) {
	mentions := []MentionRef{
		{JID: "111@lid", Name: "Ana"},
		{JID: "222@s.whatsapp.net", Name: "Ana Maria"},
		{JID: "867051314767696@bot", Name: "Meta AI"},
		{JID: "333@lid", Name: "Gone"},
	}
	text, jids := applyMentions("hi @Ana Maria and @Ana, @Anabel stays. @Meta AI help", mentions)
	if want := "hi @222 and @111, @Anabel stays. @867051314767696 help"; text != want {
		t.Fatalf("text = %q, want %q", text, want)
	}
	if len(jids) != 3 || jids[0].String() != "111@lid" || jids[1].String() != "222@s.whatsapp.net" || !invokedBot(jids).IsBot() {
		t.Fatalf("jids = %v", jids)
	}
	if text, jids := applyMentions("no mentions", nil); text != "no mentions" || len(jids) != 0 {
		t.Fatalf("got %q %v", text, jids)
	}
}

func TestACP2InMessage(t *testing.T) {
	on := &waCommon.ACP2Setting{Enabled: proto.Bool(true), SettingTimestamp: proto.Int64(5)}
	toggle := &waE2E.Message{Acp2SettingMessage: &waE2E.FutureProofMessage{Message: &waE2E.Message{
		ProtocolMessage: &waE2E.ProtocolMessage{Type: waE2E.ProtocolMessage_ACP2_SETTING.Enum(), Acp2Setting: on},
	}}}
	if got, isToggle := acp2InMessage(toggle); got != on || !isToggle {
		t.Errorf("wrapped toggle: got %v, %v", got, isToggle)
	}
	bare := &waE2E.Message{ProtocolMessage: toggle.GetAcp2SettingMessage().GetMessage().GetProtocolMessage()}
	if got, isToggle := acp2InMessage(bare); got != on || !isToggle {
		t.Errorf("bare toggle: got %v, %v", got, isToggle)
	}
	stamped := &waE2E.Message{
		Conversation:       proto.String("hi"),
		MessageContextInfo: &waE2E.MessageContextInfo{Acp2Setting: on},
	}
	if got, isToggle := acp2InMessage(stamped); got != on || isToggle {
		t.Errorf("stamp: got %v, %v", got, isToggle)
	}
	if got, _ := acp2InMessage(&waE2E.Message{Conversation: proto.String("hi")}); got != nil {
		t.Errorf("plain message carries %v", got)
	}
}
