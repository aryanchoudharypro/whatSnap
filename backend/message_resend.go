package main

import (
	"context"
	"log"
	"sync"

	"go.mau.fi/whatsmeow/types"
)

// resendTarget is one message we want the primary device to send us again.
type resendTarget struct{ id, chatJID, senderJID string }

var (
	pendingResendsMu sync.Mutex
	pendingResends   []resendTarget
)

// requestMessageResend asks the phone for its copy of a single message we
// couldn't make sense of - the per-message counterpart to a history sync, and
// the only way back for a message whose content we lost (media keys included).
// The reply arrives as an ordinary events.Message carrying the same ID, so the
// normal save path upserts it over whatever stub we were left with.
//
// Requests can only go out over a live connection, so anything raised before
// we're connected (startup repairs, for one) waits for events.Connected.
func requestMessageResend(id, chatJID, senderJID string) {
	if id == "" || chatJID == "" {
		return
	}
	target := resendTarget{id, chatJID, senderJID}
	if client == nil || !client.IsConnected() {
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
		// Our own messages and one-to-one chats have no separate participant.
		sender = chat
	}

	req := client.BuildUnavailableMessageRequest(chat, sender, t.id)
	if _, err := client.SendPeerMessage(context.Background(), req); err != nil {
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
