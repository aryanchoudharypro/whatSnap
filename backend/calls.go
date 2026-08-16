package main

import (
	"context"
	"log"
	"sync"
	"encoding/json"
	"github.com/purpshell/meowcaller"
	"go.mau.fi/whatsmeow/types"
)

var callClient *meowcaller.Client
var activeCalls = make(map[string]*meowcaller.Call)
var activeCallsMutex sync.Mutex

func initCallClient() {
	if client == nil {
		return
	}
	callClient = meowcaller.NewClient(client)
	
	callClient.OnIncomingCall(func(call *meowcaller.Call) {
		log.Printf("Incoming call from %v\n", call.Peer())
		
		activeCallsMutex.Lock()
		activeCalls[call.ID()] = call
		activeCallsMutex.Unlock()
		
		call.OnReady(func() {
			bridge, port, err := NewUDPBridge()
			if err != nil {
				log.Println("Failed to create UDP bridge:", err)
				return
			}
			call.Play(bridge)
			call.Receive(bridge)
			broadcastEvent("call_connected", map[string]interface{}{
				"call_id":    call.ID(),
				"audio_port": port,
			})
		})
		call.OnEnd(func(reason string) {
			broadcastEvent("call_ended", map[string]string{
				"call_id": call.ID(),
				"reason":  reason,
			})
			activeCallsMutex.Lock()
			delete(activeCalls, call.ID())
			activeCallsMutex.Unlock()
		})
		
		broadcastEvent("incoming_call", map[string]string{
			"call_id": call.ID(),
			"caller":  call.Peer().String(),
		})
	})
}

func handlePlaceCall(data []byte) {
	var req struct {
		JID string `json:"jid"`
	}
	if err := json.Unmarshal(data, &req); err != nil {
		log.Println("Invalid place_call format:", err)
		return
	}
	parsedJID, err := types.ParseJID(req.JID)
	if err != nil {
		log.Println("Invalid JID for place_call:", err)
		return
	}
	
	if callClient == nil {
		initCallClient()
	}
	
	ctx := context.Background()
	call, err := callClient.Call(ctx, parsedJID.String())
	if err != nil {
		log.Println("Failed to place call:", err)
		return
	}
	
	activeCallsMutex.Lock()
	activeCalls[call.ID()] = call
	activeCallsMutex.Unlock()
	
	call.OnReady(func() {
		bridge, port, err := NewUDPBridge()
		if err != nil {
			log.Println("Failed to create UDP bridge:", err)
			return
		}
		call.Play(bridge)
		call.Receive(bridge)
		broadcastEvent("call_connected", map[string]interface{}{
			"call_id":    call.ID(),
			"audio_port": port,
		})
	})
	
	broadcastEvent("call_placed", map[string]string{
		"call_id": call.ID(),
		"peer":    parsedJID.String(),
	})
	
	call.OnEnd(func(reason string) {
		broadcastEvent("call_ended", map[string]string{
			"call_id": call.ID(),
			"reason":  reason,
		})
		activeCallsMutex.Lock()
		delete(activeCalls, call.ID())
		activeCallsMutex.Unlock()
	})
	
	log.Printf("Placed call to %s, call ID: %s", parsedJID.String(), call.ID())
}

func handleAnswerCall(data []byte) {
	var req struct {
		CallID string `json:"call_id"`
	}
	if err := json.Unmarshal(data, &req); err != nil {
		log.Println("Invalid answer_call format:", err)
		return
	}
	
	activeCallsMutex.Lock()
	call, exists := activeCalls[req.CallID]
	activeCallsMutex.Unlock()
	
	if !exists {
		log.Println("Cannot answer call, not found:", req.CallID)
		return
	}
	
	err := call.Answer()
	if err != nil {
		log.Println("Failed to answer call:", err)
	} else {
		log.Println("Answered call:", req.CallID)
	}
}

func handleRejectCall(data []byte) {
	var req struct {
		CallID string `json:"call_id"`
	}
	if err := json.Unmarshal(data, &req); err != nil {
		log.Println("Invalid reject_call format:", err)
		return
	}
	
	activeCallsMutex.Lock()
	call, exists := activeCalls[req.CallID]
	activeCallsMutex.Unlock()
	
	if !exists {
		log.Println("Cannot reject call, not found:", req.CallID)
		return
	}
	
	err := call.Reject()
	if err != nil {
		log.Println("Failed to reject call:", err)
	} else {
		log.Println("Rejected call:", req.CallID)
	}
}

func handleLeaveCall(data []byte) {
	var req struct {
		CallID string `json:"call_id"`
	}
	if err := json.Unmarshal(data, &req); err != nil {
		log.Println("Invalid leave_call format:", err)
		return
	}
	
	activeCallsMutex.Lock()
	call, exists := activeCalls[req.CallID]
	activeCallsMutex.Unlock()
	
	if !exists {
		log.Println("Cannot leave call, not found:", req.CallID)
		return
	}
	
	err := call.Hangup()
	if err != nil {
		log.Println("Failed to leave call:", err)
	} else {
		log.Println("Left call:", req.CallID)
	}
}

func handleMuteCall(data []byte) {
	// Audio mute is handled locally in the Rust frontend by sending silence over UDP!
	// We don't need to do anything here for audio mute, but we could log it.
	log.Println("Mute toggled by frontend")
}

func handleAddParticipant(data []byte) {
	var req struct {
		CallID string `json:"call_id"`
		JID    string `json:"jid"`
	}
	if err := json.Unmarshal(data, &req); err != nil {
		log.Println("Invalid add_participant format:", err)
		return
	}
	parsedJID, err := types.ParseJID(req.JID)
	if err != nil {
		log.Println("Invalid JID for add_participant:", err)
		return
	}
	
	activeCallsMutex.Lock()
	call, exists := activeCalls[req.CallID]
	activeCallsMutex.Unlock()
	
	if !exists {
		log.Println("Cannot add participant, call not found:", req.CallID)
		return
	}
	
	err = call.AddParticipant(context.Background(), parsedJID.String())
	if err != nil {
		log.Println("Failed to add participant:", err)
	} else {
		log.Println("Added participant to call:", req.CallID)
	}
}
