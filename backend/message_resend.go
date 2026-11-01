package main

import (
	"context"
	"log"
	"sync"

	"go.mau.fi/whatsmeow/types"
)

type resendTarget struct{ id, chatJID, senderJID string }

var (
	pendingResendsMu sync.Mutex
	pendingResends   []resendTarget
)

func requestMessageResend(id, chatJID, senderJID string) {
	if id == "" || chatJID == "" {
		return
	}
	target := resendTarget{id, chatJID, senderJID}
	if wa() == nil || !wa().IsConnected() {
		pendingResendsMu.Lock()
		pendingResends = append(pendingResends, target)
		pendingResendsMu.Unlock()
		return
	}
	sendResendRequest(target)
}

func sendResendRequest(t resendTarget) {
	chat, err := types.ParseJID(t.chatJID)
	if err != nil {
		return
	}
	sender, err := types.ParseJID(t.senderJID)
	if err != nil || sender.IsEmpty() {
		sender = chat
	}

	req := wa().BuildUnavailableMessageRequest(chat, sender, t.id)
	if _, err := wa().SendPeerMessage(context.Background(), req); err != nil {
		log.Printf("Resend request for %s in %s failed: %v", t.id, t.chatJID, err)
		return
	}
	log.Printf("Asked the phone to resend message %s in %s", t.id, t.chatJID)
}

func flushPendingResends() {
	pendingResendsMu.Lock()
	queued := pendingResends
	pendingResends = nil
	pendingResendsMu.Unlock()

	for _, t := range queued {
		sendResendRequest(t)
	}
}
