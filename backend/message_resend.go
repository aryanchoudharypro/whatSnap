package main

import (
	"context"
	"log"

	"go.mau.fi/whatsmeow/types"
)

type resendTarget struct{ id, chatJID, senderJID string }

func (acc *Account) requestMessageResend(id, chatJID, senderJID string) {
	if id == "" || chatJID == "" {
		return
	}
	target := resendTarget{id, chatJID, senderJID}
	if acc.wa() == nil || !acc.wa().IsConnected() {
		acc.pendingResendsMu.Lock()
		acc.pendingResends = append(acc.pendingResends, target)
		acc.pendingResendsMu.Unlock()
		return
	}
	acc.sendResendRequest(target)
}

func (acc *Account) sendResendRequest(t resendTarget) {
	chat, err := types.ParseJID(t.chatJID)
	if err != nil {
		return
	}
	sender, err := types.ParseJID(t.senderJID)
	if err != nil || sender.IsEmpty() {
		sender = chat
	}

	req := acc.wa().BuildUnavailableMessageRequest(chat, sender, t.id)
	if _, err := acc.wa().SendPeerMessage(context.Background(), req); err != nil {
		log.Printf("Resend request for %s in %s failed: %v", t.id, t.chatJID, err)
		return
	}
	log.Printf("Asked the phone to resend message %s in %s", t.id, t.chatJID)
}

func (acc *Account) flushPendingResends() {
	acc.pendingResendsMu.Lock()
	queued := acc.pendingResends
	acc.pendingResends = nil
	acc.pendingResendsMu.Unlock()

	for _, t := range queued {
		acc.sendResendRequest(t)
	}
}
