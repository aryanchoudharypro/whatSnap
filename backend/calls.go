package main

import (
	"context"
	"log"
	"os"
	"path/filepath"
	"sync"
	"encoding/json"
	"github.com/purpshell/meowcaller"
	"github.com/purpshell/meowcaller/diag"
	"github.com/rs/zerolog"
	"go.mau.fi/whatsmeow/types"
)

var callClient *meowcaller.Client
var activeCalls = make(map[string]*meowcaller.Call)
var activeCallsMutex sync.Mutex

func initCallClient() {
	if client == nil {
		return
	}
	// meowcaller "never configures logging" on its own - without WithLogger
	// every diagnostic it has (relay allocation, SRTP unprotect failures,
	// first-RTP-in timing) is silently discarded, which left us guessing
	// blind about where inbound audio was actually failing. os.Stderr is
	// already redirected to backend.log in main(), so this lands in the
	// same place as everything else.
	callLog := zerolog.New(os.Stderr).Level(zerolog.DebugLevel).With().Timestamp().Str("component", "meowcaller").Logger()
	opts := []meowcaller.Option{meowcaller.WithLogger(callLog)}

	// WithDiagnostics gives per-packet visibility (every STUN/RTCP/RTP packet
	// on the relay channel, not just the summarized log lines above) - this
	// is what lets us tell whether the relay transport itself goes quiet
	// mid-call or the peer specifically stops sending audio while everything
	// else keeps flowing. Written next to backend.log; harmless if it can't
	// be created (call still works, we just lose this extra detail).
	if diagDir, err := os.UserCacheDir(); err == nil {
		if rec, err := diag.NewRecorder(filepath.Join(diagDir, "whaNTV", "logs", "call-diag")); err == nil {
			opts = append(opts, meowcaller.WithDiagnostics(rec))
		} else {
			log.Println("Failed to create call diagnostics recorder:", err)
		}
	}

	callClient = meowcaller.NewClient(client, opts...)

	callClient.OnIncomingCall(func(call *meowcaller.Call) {
		log.Printf("Incoming call from %v\n", call.Peer())

		activeCallsMutex.Lock()
		activeCalls[call.ID()] = call
		activeCallsMutex.Unlock()

		// OnReady is a single-slot callback on *meowcaller.Call (overwritten,
		// not a list), so this is the one and only place that may register
		// it for an incoming call - wiring the audio bridge here, rather
		// than eagerly in handleAnswerCall, is what makes "connected" mean
		// the media path is actually ready instead of just "Answer() didn't
		// error".
		call.OnReady(func() {
			log.Println("Incoming call ready (connected):", call.ID())
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

	// The audio bridge is wired in the call.OnReady callback registered
	// back in OnIncomingCall, once media is actually ready - not here.
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
