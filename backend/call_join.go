package main

import (
	"context"
	"encoding/json"
	"errors"
	"log"

	"github.com/purpshell/meowcaller"
	"go.mau.fi/whatsmeow/types"
)

func (acc *Account) joinableCallEvent(list []meowcaller.JoinableCall) []map[string]interface{} {
	calls := make([]map[string]interface{}, 0, len(list))
	for _, c := range list {
		calls = append(calls, map[string]interface{}{
			"call_id":      c.CallID,
			"group_jid":    c.GroupJID.String(),
			"creator_name": acc.resolveDisplayName(c.Creator),
			"video":        c.Video,
			"connected":    c.Connected,
		})
	}
	return calls
}

func (acc *Account) watchJoinableCalls(client *meowcaller.Client) {
	client.OnJoinableCalls(func(list []meowcaller.JoinableCall) {
		calls := acc.joinableCallEvent(list)
		acc.joinableCallsMu.Lock()
		acc.joinableCalls = calls
		acc.joinableCallsMu.Unlock()
		acc.broadcastEvent("call_joinable", map[string]interface{}{"calls": calls})
	})
}

func (acc *Account) handleGetJoinableCalls() {
	acc.joinableCallsMu.Lock()
	calls := acc.joinableCalls
	acc.joinableCallsMu.Unlock()
	if calls == nil {
		calls = []map[string]interface{}{}
	}
	acc.broadcastEvent("call_joinable", map[string]interface{}{"calls": calls})
}

type JoinGroupCallData struct {
	CallID string `json:"call_id"`
	Video  bool   `json:"video"`
}

func (acc *Account) handleJoinGroupCall(raw json.RawMessage) {
	var data JoinGroupCallData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		fail := func(err error) {
			acc.broadcastEvent("call_failed", map[string]string{"jid": "", "message": "Couldn't join the call: " + err.Error()})
		}
		if acc.callClient == nil {
			fail(errors.New("calls aren't available yet"))
			return
		}
		var groupJID types.JID
		for _, c := range acc.callClient.JoinableCalls() {
			if c.CallID == data.CallID {
				groupJID = c.GroupJID
			}
		}
		call, err := acc.callClient.JoinOngoingCall(context.Background(), data.CallID, data.Video)
		if err != nil {
			fail(err)
			return
		}
		acc.activeCallsMutex.Lock()
		acc.activeCalls[call.ID()] = call
		acc.activeCallsMutex.Unlock()
		chat := call.Peer()
		if !groupJID.IsEmpty() {
			chat = groupJID
		}
		acc.trackLiveCall(call.ID(), chat, types.EmptyJID, false, data.Video, true)
		acc.broadcastEvent("call_placed", map[string]interface{}{
			"call_id": call.ID(), "peer": chat.String(), "link": true, "video": data.Video,
		})
		acc.attachCallHandlers(call)
		log.Printf("Joined ongoing group call, call ID: %s", call.ID())
	}()
}
