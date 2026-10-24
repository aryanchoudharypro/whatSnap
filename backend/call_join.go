package main

import (
	"context"
	"encoding/json"
	"errors"
	"log"
	"sync"

	"github.com/purpshell/meowcaller"
	"go.mau.fi/whatsmeow/types"
)

// The group calls still going on that we were invited to but aren't in,
// as last reported by meowcaller. The frontend swaps a group chat's call
// buttons for "Join call" while one of these is for that group.
var (
	joinableCallsMu sync.Mutex
	joinableCalls   []map[string]interface{}
)

func joinableCallEvent(list []meowcaller.JoinableCall) []map[string]interface{} {
	calls := make([]map[string]interface{}, 0, len(list))
	for _, c := range list {
		calls = append(calls, map[string]interface{}{
			"call_id":      c.CallID,
			"group_jid":    c.GroupJID.String(),
			"creator_name": resolveDisplayName(c.Creator),
			"video":        c.Video,
			"connected":    c.Connected,
		})
	}
	return calls
}

func watchJoinableCalls(client *meowcaller.Client) {
	client.OnJoinableCalls(func(list []meowcaller.JoinableCall) {
		calls := joinableCallEvent(list)
		joinableCallsMu.Lock()
		joinableCalls = calls
		joinableCallsMu.Unlock()
		broadcastEvent("call_joinable", map[string]interface{}{"calls": calls})
	})
}

// handleGetJoinableCalls replays the current list, for a frontend that
// (re)connected after it last changed.
func handleGetJoinableCalls() {
	joinableCallsMu.Lock()
	calls := joinableCalls
	joinableCallsMu.Unlock()
	if calls == nil {
		calls = []map[string]interface{}{}
	}
	broadcastEvent("call_joinable", map[string]interface{}{"calls": calls})
}

type JoinGroupCallData struct {
	CallID string `json:"call_id"`
	Video  bool   `json:"video"`
}

// handleJoinGroupCall joins a group call that's already going on. The
// frontend handles it like joining a call link: no ringing, straight in.
func handleJoinGroupCall(raw json.RawMessage) {
	var data JoinGroupCallData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		fail := func(err error) {
			broadcastEvent("call_failed", map[string]string{"jid": "", "message": "Couldn't join the call: " + err.Error()})
		}
		if callClient == nil {
			fail(errors.New("calls aren't available yet"))
			return
		}
		var groupJID types.JID
		for _, c := range callClient.JoinableCalls() {
			if c.CallID == data.CallID {
				groupJID = c.GroupJID
			}
		}
		call, err := callClient.JoinOngoingCall(context.Background(), data.CallID, data.Video)
		if err != nil {
			fail(err)
			return
		}
		activeCallsMutex.Lock()
		activeCalls[call.ID()] = call
		activeCallsMutex.Unlock()
		chat := call.Peer()
		if !groupJID.IsEmpty() {
			chat = groupJID
		}
		trackLiveCall(call.ID(), chat, false, data.Video, true)
		broadcastEvent("call_placed", map[string]interface{}{
			"call_id": call.ID(), "peer": chat.String(), "link": true, "video": data.Video,
		})
		attachCallHandlers(call)
		log.Printf("Joined ongoing group call, call ID: %s", call.ID())
	}()
}
