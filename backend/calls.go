package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/purpshell/meowcaller"
	"github.com/purpshell/meowcaller/diag"
	"github.com/purpshell/meowcaller/signaling"
	"github.com/rs/zerolog"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
)

var callClient *meowcaller.Client
var activeCalls = make(map[string]*meowcaller.Call)
var activeCallsMutex sync.Mutex

// Outbound video (our camera) bridges, one per active call, keyed the same
// way as activeCalls. See video_bridge.go.
var activeVideoBridges = make(map[string]*VideoBridge)
var activeVideoBridgesMutex sync.Mutex

// Audio bridges, one per active call, keyed the same way as activeCalls. Each
// wraps a real UDP socket (see audio_udp.go), so it must be closed on
// OnEnd just like activeVideoBridges - otherwise every call leaks the socket
// for the rest of the process's life.
var activeAudioBridges = make(map[string]*UDPBridge)
var activeAudioBridgesMutex sync.Mutex

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

// meowcaller parses an incoming group offer's group-jid internally but never
// exposes it on *meowcaller.Call, so this records it by call-id from the same
// CallOffer event. whatsmeow runs event handlers one after another in
// registration order, and this one is registered before meowcaller's (see
// initCallClient), so the entry is always in place by the time
// OnIncomingCall fires.
var incomingGroupCallJIDs = make(map[string]types.JID)
var incomingGroupCallJIDsMutex sync.Mutex

func recordIncomingGroupCallJID(evt interface{}) {
	offer, ok := evt.(*events.CallOffer)
	if !ok || offer.Data == nil {
		return
	}
	snapshot, isGroup, err := signaling.ParseGroupInviteSnapshot(offer.Data)
	if err != nil || !isGroup || snapshot.GroupJID.IsEmpty() {
		return
	}
	incomingGroupCallJIDsMutex.Lock()
	incomingGroupCallJIDs[offer.CallID] = snapshot.GroupJID
	incomingGroupCallJIDsMutex.Unlock()
}

func takeIncomingGroupCallJID(callID string) (types.JID, bool) {
	incomingGroupCallJIDsMutex.Lock()
	defer incomingGroupCallJIDsMutex.Unlock()
	jid, ok := incomingGroupCallJIDs[callID]
	delete(incomingGroupCallJIDs, callID)
	return jid, ok
}

func groupChatName(jid types.JID) string {
	var name string
	if db != nil {
		_ = db.QueryRow("SELECT name FROM chats WHERE jid = ?", jid.String()).Scan(&name)
	}
	return name
}

func isOwnJID(jid types.JID) bool {
	if wa() == nil || wa().Store == nil || jid.User == "" {
		return false
	}
	for _, own := range []types.JID{wa().Store.GetJID(), wa().Store.GetLID()} {
		if own.User != "" && own.User == jid.User {
			return true
		}
	}
	return false
}

// peerVideoEvent maps the raw state of a <video> stanza from the other side
// of the call to the call_video_state event the frontend announces, or ""
// for states that don't need announcing. 4/5 (upgrade accept/reject) only
// answer an upgrade we asked for, and we never ask.
func peerVideoEvent(raw int) string {
	switch raw {
	case signaling.VideoStateEnabled:
		return "on"
	case signaling.VideoStateDisabled, signaling.VideoStateStopped:
		return "off"
	case signaling.VideoStateUpgradeRequest, signaling.VideoStateUpgradeRequestV2:
		return "upgrade_requested"
	}
	return ""
}

// attachCallHandlers wires the callbacks shared by incoming and outgoing
// calls. OnReady/OnEnd/OnGroupState/OnVideoState are single-slot on *meowcaller.Call
// (each registration overwrites the last), so this must be the only place
// that registers them for a given call.
func attachCallHandlers(call *meowcaller.Call) {
	// Wiring the audio bridge here, rather than eagerly in handleAnswerCall,
	// is what makes "connected" mean the media path is actually ready
	// instead of just "Answer() didn't error".
	call.OnReady(func() {
		log.Println("Call ready (connected):", call.ID())
		updateLiveCall(call.ID(), func(c *liveCall) { c.connected = time.Now() })
		bridge, port, err := NewUDPBridge()
		if err != nil {
			log.Println("Failed to create UDP bridge:", err)
			return
		}
		call.Play(bridge)
		call.Receive(bridge)
		activeAudioBridgesMutex.Lock()
		activeAudioBridges[call.ID()] = bridge
		activeAudioBridgesMutex.Unlock()

		// The video bridge is created here too (even for a call that starts
		// as voice-only) so a mid-call camera toggle has a port ready to
		// connect to immediately - nothing is sent over it until the
		// frontend's encoder actually connects and meowcaller's own
		// StartVideo/SetVideoEnabled gating decides whether it's live.
		videoPort := 0
		if videoBridge, vport, err := NewVideoBridge(); err != nil {
			log.Println("Failed to create video bridge:", err)
		} else {
			videoPort = vport
			activeVideoBridgesMutex.Lock()
			activeVideoBridges[call.ID()] = videoBridge
			activeVideoBridgesMutex.Unlock()
			go videoBridge.Run(call)
		}

		broadcastEvent("call_connected", map[string]interface{}{
			"call_id":    call.ID(),
			"audio_port": port,
			"video_port": videoPort,
		})
	})
	call.OnEnd(func(reason string) {
		// summary is how the call ended in words ("Voice call, answered on
		// another device"); it also lands in the chat as a history row.
		summary := finishLiveCall(call.ID(), reason)
		broadcastEvent("call_ended", map[string]string{
			"call_id": call.ID(),
			"reason":  reason,
			"summary": summary,
		})
		activeCallsMutex.Lock()
		delete(activeCalls, call.ID())
		activeCallsMutex.Unlock()

		activeVideoBridgesMutex.Lock()
		if bridge, ok := activeVideoBridges[call.ID()]; ok {
			bridge.Close()
			delete(activeVideoBridges, call.ID())
		}
		activeVideoBridgesMutex.Unlock()

		activeAudioBridgesMutex.Lock()
		if bridge, ok := activeAudioBridges[call.ID()]; ok {
			bridge.Close()
			delete(activeAudioBridges, call.ID())
		}
		activeAudioBridgesMutex.Unlock()
	})
	// Fires for group calls (placed as one, or a 1:1 call that became an
	// ad-hoc group through add_participant) with WhatsApp's authoritative
	// roster, and replays the current roster immediately on registration.
	// Our own entry is dropped - the frontend only needs the other people.
	call.OnGroupState(func(state meowcaller.GroupCallState) {
		participants := make([]map[string]string, 0, len(state.Participants))
		for _, p := range state.Participants {
			if isOwnJID(p.JID) || isOwnJID(p.PN) {
				continue
			}
			nameJID := p.JID
			if !p.PN.IsEmpty() {
				nameJID = p.PN
			}
			participants = append(participants, map[string]string{
				"jid":   p.JID.String(),
				"name":  resolveDisplayName(nameJID),
				"state": p.State,
			})
		}
		broadcastEvent("call_participants", map[string]interface{}{
			"call_id":      call.ID(),
			"participants": participants,
		})
	})
	// The other side's camera, plus our own upgrade/enable acks land here
	// too (peerVideoEvent only reports the ones worth announcing).
	call.OnVideoState(func(vs meowcaller.VideoState) {
		event := peerVideoEvent(vs.Raw)
		if event == "" {
			return
		}
		broadcastEvent("call_video_state", map[string]string{
			"call_id":    call.ID(),
			"peer_video": event,
		})
	})
	// WhatsApp's PLI/FIR feedback asking our camera for a fresh keyframe -
	// forwarded to the frontend encoder, which has no other way to learn
	// the network lost a frame and needs an IDR to recover.
	call.OnVideoKeyframeRequest(func() {
		broadcastEvent("call_video_keyframe_request", map[string]string{
			"call_id": call.ID(),
		})
	})
}

func initCallClient() {
	if wa() == nil {
		return
	}
	// Must be registered before meowcaller.NewClient installs its own
	// handler - see incomingGroupCallJIDs.
	wa().AddEventHandler(recordIncomingGroupCallJID)
	wa().AddEventHandler(noteOwnDeviceReject)
	callClient = meowcaller.NewClient(wa(), newCallDiagnosticsOptions()...)

	callClient.OnIncomingCall(func(call *meowcaller.Call) {
		log.Printf("Incoming call from %v\n", call.Peer())
		// Taken up front so the entry is consumed even when the call is
		// auto-rejected as busy below.
		groupJID, hasGroupJID := takeIncomingGroupCallJID(call.ID())

		activeCallsMutex.Lock()
		// A call only leaves activeCalls through its OnEnd handler, and one
		// that ended before attachCallHandlers registered it never gets
		// there. Drop those here, or whaNTV stays "busy" and declines every
		// call until it restarts.
		for id, c := range activeCalls {
			if c.State() == meowcaller.CallPhaseEnded {
				delete(activeCalls, id)
			}
		}
		alreadyOnACall := len(activeCalls) > 0
		if !alreadyOnACall {
			activeCalls[call.ID()] = call
		}
		activeCallsMutex.Unlock()

		_, isGroupCall := call.GroupState()
		chatJID := call.Peer()
		if hasGroupJID {
			chatJID = groupJID
		}
		trackLiveCall(call.ID(), chatJID, true, call.IsVideo(), isGroupCall)

		if alreadyOnACall {
			// Still goes in the history, as the official apps show it: a
			// call that rang while you were on another one is a missed call.
			defer finishLiveCall(call.ID(), "busy")
			// reason="busy" says only this device can't take the call. A
			// plain reject would decline it for the user, ending the
			// caller's call and silencing the phone's ringing too.
			log.Printf("Already on a call, rejecting incoming call from %v as busy\n", call.Peer())
			if err := call.RejectBusy(); err != nil {
				log.Println("Failed to auto-reject incoming call while busy:", err)
			}
			return
		}

		attachCallHandlers(call)

		// The peer JID is usually a @lid, which the frontend's contact map
		// (keyed by phone JID) cannot look up - it would show the raw
		// "...@lid" string instead of a name. Resolve it here, where the
		// LID -> PN -> contact-name fallback chain already lives.
		payload := map[string]interface{}{
			"call_id":     call.ID(),
			"caller":      call.Peer().String(),
			"caller_name": resolveDisplayName(call.Peer()),
			"is_group":    isGroupCall,
			"is_video":    call.IsVideo(),
		}
		if hasGroupJID {
			payload["group_jid"] = groupJID.String()
			payload["group_name"] = groupChatName(groupJID)
		}
		broadcastEvent("incoming_call", payload)
	})
}

// placeGroupChatCall calls every other member of a WhatsApp group chat.
// meowcaller's GroupCallByID refuses groups with fewer than two other
// members, so a group that is just you and one other person gets an
// ordinary 1:1 call to that person instead - the same people end up in the
// call either way.
func placeGroupChatCall(ctx context.Context, groupJID types.JID, video bool) (*meowcaller.Call, error) {
	info, err := wa().GetGroupInfo(ctx, groupJID)
	if err != nil {
		return nil, fmt.Errorf("couldn't load the group's members: %w", err)
	}
	var others []types.JID
	for _, p := range info.Participants {
		if isOwnJID(p.JID) || isOwnJID(p.PhoneNumber) || isOwnJID(p.LID) {
			continue
		}
		others = append(others, p.JID)
	}
	switch {
	case len(others) == 0:
		return nil, errors.New("There's no one else in this group to call.")
	case len(others) == 1:
		return callClient.CallWithOptions(ctx, others[0].String(), meowcaller.CallOptions{Video: video})
	case len(others) > 31:
		return nil, fmt.Errorf("This group has %d other members; WhatsApp group calls allow at most 32 people.", len(others))
	}
	return callClient.GroupCallByIDWithOptions(ctx, groupJID.String(), meowcaller.GroupCallOptions{Video: video})
}

func handlePlaceCall(data []byte) {
	var req struct {
		JID   string `json:"jid"`
		Video bool   `json:"video"`
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
	var call *meowcaller.Call
	if parsedJID.Server == types.GroupServer {
		call, err = placeGroupChatCall(ctx, parsedJID, req.Video)
	} else if req.Video {
		call, err = callClient.CallWithOptions(ctx, parsedJID.String(), meowcaller.CallOptions{Video: true})
	} else {
		call, err = callClient.Call(ctx, parsedJID.String())
	}
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

	// A group chat's call belongs in the group, unless it went out as a 1:1
	// (a group with just one other member) - then it's that person's.
	chatJID := call.Peer()
	_, isGroupCall := call.GroupState()
	if parsedJID.Server == types.GroupServer && isGroupCall {
		chatJID = parsedJID
	}
	trackLiveCall(call.ID(), chatJID, false, req.Video, isGroupCall)

	// call_placed goes out before attachCallHandlers: OnGroupState replays
	// the seeded roster immediately, and the frontend ignores
	// call_participants for a call_id it hasn't learned yet.
	// self_phone picks the ringback the frontend plays: WhatsApp rings in
	// the tone of the caller's own country.
	selfPhone := ""
	if wa().Store.ID != nil {
		selfPhone = wa().Store.ID.User
	}
	broadcastEvent("call_placed", map[string]string{
		"call_id":    call.ID(),
		"peer":       parsedJID.String(),
		"self_phone": selfPhone,
	})
	attachCallHandlers(call)

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

	// meowcaller's accept always declares video as decode-only (no "enc"
	// attribute, regardless of our local video-enabled state - see
	// signaling.videoAcceptNode) when the offer was video, so the peer
	// already learns we won't be sending anything: no separate camera-off
	// signal is needed. Sending one anyway (an explicit <video state="0"/>
	// right after answering) was tried and made the caller's own client
	// collapse the whole call to voice - their camera went dark too, and
	// they had to send an explicit upgrade request to get video back.
	//
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

	updateLiveCall(req.CallID, func(c *liveCall) { c.localReject = true })
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

	updateLiveCall(req.CallID, func(c *liveCall) { c.localHangup = true })
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

// handleSetVideo turns the local camera on or off for an active call. This is
// the only place local video state is ever transitioned, so it's what both
// "start video call" (once connected) and "toggle camera mid-call" go
// through - see [[video_calls_work_status]] Phase 2.
func handleSetVideo(data []byte) {
	var req struct {
		CallID  string `json:"call_id"`
		Enabled bool   `json:"enabled"`
	}
	if err := json.Unmarshal(data, &req); err != nil {
		log.Println("Invalid set_video format:", err)
		return
	}

	activeCallsMutex.Lock()
	call, exists := activeCalls[req.CallID]
	activeCallsMutex.Unlock()

	if !exists {
		log.Println("Cannot set video, call not found:", req.CallID)
		return
	}

	var err error
	switch {
	case req.Enabled && !call.IsVideo():
		// A plain voice call has no video state yet - this is the only path
		// that gets one started. StartVideo already gates outbound sending
		// on, unlike SetVideoEnabled(true), which requires video to already
		// be part of the call (see meowcaller's transitionVideo/
		// setVideoEnabled) - using SetVideoEnabled here would silently no-op.
		err = call.StartVideo()
	default:
		err = call.SetVideoEnabled(req.Enabled)
	}
	if err != nil {
		log.Println("Failed to set video:", err)
		broadcastEvent("call_set_video_failed", map[string]string{
			"call_id": req.CallID,
			"message": err.Error(),
		})
		return
	}
	log.Println("Set video for call:", req.CallID, "enabled:", req.Enabled)
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

	name := resolveDisplayName(parsedJID)
	// The frontend used to announce "Participant added." the moment the
	// contact was picked, so every failure here was silent to the user.
	// Report the real outcome instead.
	fail := func(message string) {
		broadcastEvent("call_participant_add_failed", map[string]string{
			"call_id": req.CallID,
			"jid":     parsedJID.String(),
			"name":    name,
			"message": message,
		})
	}

	if !exists {
		log.Println("Cannot add participant, call not found:", req.CallID)
		fail("That call has already ended.")
		return
	}

	err = call.AddParticipant(context.Background(), parsedJID.String())
	if err != nil {
		log.Println("Failed to add participant:", err)
		fail(addParticipantErrorMessage(err, name))
		return
	}
	log.Println("Added participant to call:", req.CallID)
	broadcastEvent("call_participant_added", map[string]string{
		"call_id": req.CallID,
		"jid":     parsedJID.String(),
		"name":    name,
	})
}

// addParticipantErrorMessage turns meowcaller's invite errors into something
// worth reading aloud. meowcaller has no typed errors for these, so this
// matches on its messages and falls back to the raw error otherwise.
func addParticipantErrorMessage(err error, name string) string {
	msg := err.Error()
	switch {
	case strings.Contains(msg, "call is not connected"):
		return "Wait for the call to connect before adding someone."
	case strings.Contains(msg, "already in the call roster"):
		return name + " is already in the call."
	case strings.Contains(msg, "cannot add this client to its own call"):
		return "You can't add yourself to the call."
	case strings.Contains(msg, "capability is unavailable"):
		return "Couldn't add " + name + ": WhatsApp hasn't sent the call details needed to invite someone yet. Try again in a moment."
	}
	return "Couldn't add " + name + ": " + msg
}
