package main

import (
	"context"
	"sync"
	"time"

	"go.mau.fi/whatsmeow/types"
)

type outgoingChatState struct {
	state types.ChatPresence
	media types.ChatPresenceMedia
}

// Keep the last requested state for each chat until paused has actually been
// sent. Serializing only these network sends prevents an older composing update
// from arriving after paused, without blocking the account's command queue.
type outgoingChatStates struct {
	sync.Mutex
	sendMu  sync.Mutex
	pending map[types.JID]*outgoingChatState
	closed  bool
}

type sendChatState func(context.Context, types.JID, types.ChatPresence, types.ChatPresenceMedia) error

func (s *outgoingChatStates) queue(jid types.JID, state types.ChatPresence, media types.ChatPresenceMedia) *outgoingChatState {
	s.Lock()
	defer s.Unlock()
	if s.closed {
		return nil
	}
	if s.pending == nil {
		s.pending = make(map[types.JID]*outgoingChatState)
	}
	update := &outgoingChatState{state, media}
	s.pending[jid] = update
	return update
}

func (s *outgoingChatStates) send(ctx context.Context, jid types.JID, update *outgoingChatState, send sendChatState) {
	s.sendMu.Lock()
	defer s.sendMu.Unlock()
	s.Lock()
	current := !s.closed && update != nil && s.pending[jid] == update
	s.Unlock()
	if !current || ctx.Err() != nil {
		return
	}
	ctx, cancel := context.WithTimeout(ctx, 2*time.Second)
	defer cancel()
	if err := send(ctx, jid, update.state, update.media); err == nil && update.state == types.ChatPresencePaused {
		s.Lock()
		if s.pending[jid] == update {
			delete(s.pending, jid)
		}
		s.Unlock()
	}
}

func (s *outgoingChatStates) pauseAll(send sendChatState) {
	s.Lock()
	s.closed = true
	pending := s.pending
	s.pending = nil
	s.Unlock()
	// Account cancellation interrupts any send in flight before we get here.
	s.sendMu.Lock()
	defer s.sendMu.Unlock()
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	for jid := range pending {
		if ctx.Err() != nil {
			break
		}
		_ = send(ctx, jid, types.ChatPresencePaused, "")
	}
}

func (acc *Account) queueOutgoingChatState(jid types.JID, state types.ChatPresence, media types.ChatPresenceMedia) {
	update := acc.outgoingChatStates.queue(jid, state, media)
	if update != nil {
		if cli := acc.wa(); cli != nil {
			go acc.outgoingChatStates.send(acc.ctx, jid, update, cli.SendChatPresence)
		}
	}
}
