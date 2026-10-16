package main

import (
	"testing"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"google.golang.org/protobuf/proto"
)

func TestPrepareForQuoteMatchesWhatsAppWeb(t *testing.T) {
	stored := &waE2E.Message{
		MessageContextInfo: &waE2E.MessageContextInfo{MessageSecret: []byte("secret")},
		ExtendedTextMessage: &waE2E.ExtendedTextMessage{
			Text: proto.String("hi @123"),
			ContextInfo: &waE2E.ContextInfo{
				StanzaID:      proto.String("OLDER"),
				Participant:   proto.String("1@s.whatsapp.net"),
				QuotedMessage: &waE2E.Message{Conversation: proto.String("older")},
				MentionedJID:  []string{"123@s.whatsapp.net"},
				Expiration:    proto.Uint32(86400),
			},
		},
	}
	got := prepareForQuote(&waE2E.Message{EphemeralMessage: &waE2E.FutureProofMessage{Message: stored}})

	if got.GetEphemeralMessage() != nil || got.GetMessageContextInfo() != nil {
		t.Fatalf("wrapper or messageContextInfo kept: %v", got)
	}
	ctx := got.GetExtendedTextMessage().GetContextInfo()
	if ctx.GetQuotedMessage() != nil || ctx.GetStanzaID() != "" || ctx.GetParticipant() != "" || ctx.GetExpiration() != 0 {
		t.Fatalf("nested context kept: %v", ctx)
	}
	if len(ctx.GetMentionedJID()) != 1 || got.GetExtendedTextMessage().GetText() != "hi @123" {
		t.Fatalf("content or mentions lost: %v", got)
	}
	if stored.GetMessageContextInfo() == nil || stored.GetExtendedTextMessage().GetContextInfo().GetStanzaID() != "OLDER" {
		t.Fatal("stored message was modified")
	}
}
