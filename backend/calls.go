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

// Debugging aid for call-audio issues reported in the field (e.g. one-sided
// silence): the diag.Recorder dumps per-call JSONL (rtp/rtcp/srtp/relay/stun/
// etc.) under logs/call-diag, and the zerolog logger writes meowcaller's own
// debug/trace lines to logs/call.log, both alongside the main backend.log.
func newCallDiagnosticsOptions() []meowcaller.Option {
	var opts []meowcaller.Option

	diagDir := filepath.Join(appDataDir(), "logs", "call-diag")
	if rec, err := diag.NewRecorder(diagDir); err == nil {
		opts = append(opts, meowcaller.WithDiagnostics(rec))
	} else {
		log.Println("Failed to create call diagnostics recorder:", err)
	}

	logPath := filepath.Join(appDataDir(), "logs", "call.log")
	if f, err := os.OpenFile(logPath, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0o644); err == nil {
		opts = append(opts, meowcaller.WithLogger(zerolog.New(f).With().Timestamp().Logger()))
	} else {
		log.Println("Failed to open call log file:", err)
	}

	return opts
}

func initCallClient() {
	if client == nil {
		return
	}
	callClient = meowcaller.NewClient(client, newCallDiagnosticsOptions()...)

	callClient.OnIncomingCall(func(call *meowcaller.Call) {
		log.Printf("Incoming call from %v\n", call.Peer())

		activeCallsMutex.Lock()
		alreadyOnACall := len(activeCalls) > 0
		if !alreadyOnACall {
			activeCalls[call.ID()] = call
		}
		activeCallsMutex.Unlock()

		if alreadyOnACall {
			// meowcaller has no protocol-level "busy" reason to send back
			// (WhatsApp's reject stanza doesn't carry one in this library),
			// so the closest equivalent is declining it the same way a
			// manual Reject does - this at least stops a second incoming
			// call from silently clobbering the active call's UI state, and
			// lets it go to voicemail/missed-call on the caller's end
			// instead of ringing against someone who's already on a call.
			log.Printf("Already on a call, auto-rejecting incoming call from %v\n", call.Peer())
			if err := call.Reject(); err != nil {
				log.Println("Failed to auto-reject incoming call while busy:", err)
			}
			return
		}

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

		// The peer JID is usually a @lid, which the frontend's contact map
		// (keyed by phone JID) cannot look up - it would show the raw
		// "...@lid" string instead of a name. Resolve it here, where the
		// LID -> PN -> contact-name fallback chain already lives.
		broadcastEvent("incoming_call", map[string]string{
			"call_id":     call.ID(),
			"caller":      call.Peer().String(),
			"caller_name": resolveDisplayName(call.Peer()),
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
		// Without this, the frontend never hears back at all: it shows the
		// call UI optimistically on place_call and only learns the real
		// call_id (and un-races its Leave button) from call_placed. If the
		// offer never made it onto the wire, that event never arrives and
		// the UI is stuck showing a live-looking call forever with no way
		// to dismiss it.
		broadcastEvent("call_failed", map[string]string{
			"jid":     parsedJID.String(),
			"message": err.Error(),
		})
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
		// Without this the frontend just sits on "Answering..." forever with
		// no way to know the call is already gone (e.g. the caller hung up
		// right as Accept was pressed) - it was previously indistinguishable
		// from Accept silently doing nothing.
		broadcastEvent("call_answer_failed", map[string]string{
			"call_id": req.CallID,
			"message": "That call is no longer available.",
		})
		return
	}

	// The audio bridge is wired in the call.OnReady callback registered
	// back in OnIncomingCall, once media is actually ready - not here.
	err := call.Answer()
	if err != nil {
		log.Println("Failed to answer call:", err)
		broadcastEvent("call_answer_failed", map[string]string{
			"call_id": req.CallID,
			"message": "Failed to answer call: " + err.Error(),
		})
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
