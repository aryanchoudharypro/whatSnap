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
	"sync/atomic"
	"time"

	"github.com/purpshell/meowcaller"
	"github.com/purpshell/meowcaller/diag"
	"github.com/purpshell/meowcaller/signaling"
	"github.com/rs/zerolog"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
)

// Audio bridges, one per active call, keyed the same way as activeCalls. Each
// wraps a real UDP socket (see audio_udp.go), so it must be closed on
// OnEnd just like activeVideoBridges - otherwise every call leaks the socket
// for the rest of the process's life.

func newCallDiagnosticsOptions() []meowcaller.Option {
	var opts []meowcaller.Option

	diagDir := filepath.Join(logsDir(), "call-diag")
	if rec, err := diag.NewRecorder(diagDir); err == nil {
		opts = append(opts, meowcaller.WithDiagnostics(rec))
	} else {
		log.Println("Failed to create call diagnostics recorder:", err)
	}

	logPath := filepath.Join(logsDir(), "call.log")
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

func (acc *Account) recordIncomingGroupCallJID(evt interface{}) {
	offer, ok := evt.(*events.CallOffer)
	if !ok || offer.Data == nil || offer.Offline {
		return
	}
	snapshot, isGroup, err := signaling.ParseGroupInviteSnapshot(offer.Data)
	if err != nil || !isGroup || snapshot.GroupJID.IsEmpty() {
		return
	}
	acc.incomingGroupCallJIDsMutex.Lock()
	acc.incomingGroupCallJIDs[offer.CallID] = snapshot.GroupJID
	acc.incomingGroupCallJIDsMutex.Unlock()
}

func (acc *Account) takeIncomingGroupCallJID(callID string) (types.JID, bool) {
	acc.incomingGroupCallJIDsMutex.Lock()
	defer acc.incomingGroupCallJIDsMutex.Unlock()
	jid, ok := acc.incomingGroupCallJIDs[callID]
	delete(acc.incomingGroupCallJIDs, callID)
	return jid, ok
}

// outgoingCallProgress is how far each call we placed has got before it
// connects, for the call screen to show the way WhatsApp's does: "ringing"
// once the other phone has the offer (<preaccept>), "connecting" once they
// answer (<accept>). meowcaller's own phase can't be used: it jumps to
// Connecting as soon as the relay is up, before the phone even rings. A
// <preaccept> can land before handlePlaceCall has sent call_placed, so the
// latest state is kept and sent again right after it.
var outgoingCallProgress = make(map[string]string)
var outgoingCallProgressMutex sync.Mutex

func (acc *Account) noteOutgoingCallProgress(evt interface{}) {
	var meta types.BasicCallMeta
	var state string
	switch e := evt.(type) {
	case *events.CallPreAccept:
		meta, state = e.BasicCallMeta, "ringing"
	case *events.CallAccept:
		meta, state = e.BasicCallMeta, "connecting"
	default:
		return
	}
	if meta.CallID == "" || acc.isOwnJID(meta.From) || (!meta.CallCreator.IsEmpty() && !acc.isOwnJID(meta.CallCreator)) {
		return
	}
	outgoingCallProgressMutex.Lock()
	if outgoingCallProgress[meta.CallID] == "connecting" {
		outgoingCallProgressMutex.Unlock()
		return
	}
	outgoingCallProgress[meta.CallID] = state
	outgoingCallProgressMutex.Unlock()
	log.Printf("[CALL PROGRESS] Call %s progress: %s", meta.CallID, state)
	acc.broadcastCallProgress(meta.CallID, state)
}

func (acc *Account) broadcastCallProgress(callID, state string) {
	acc.broadcastEvent("call_progress", map[string]string{"call_id": callID, "state": state})
}

func (acc *Account) groupChatName(jid types.JID) string {
	var name string
	if acc.db != nil {
		_ = acc.db.QueryRow("SELECT name FROM chats WHERE jid = ?", jid.String()).Scan(&name)
	}
	return name
}

func (acc *Account) isOwnJID(jid types.JID) bool {
	if acc.wa() == nil || acc.wa().Store == nil || jid.User == "" {
		return false
	}
	for _, own := range []types.JID{acc.wa().Store.GetJID(), acc.wa().Store.GetLID()} {
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
func (acc *Account) attachCallHandlers(call *meowcaller.Call) {
	call.OnReady(func() {
		log.Println("Call ready (connected):", call.ID())
		acc.updateLiveCall(call.ID(), func(c *liveCall) { c.connected = time.Now() })
		bridge, port, err := NewUDPBridge()
		if err != nil {
			log.Println("Failed to create UDP bridge:", err)
			return
		}
		call.Play(bridge)
		call.Receive(bridge)
		acc.activeAudioBridgesMutex.Lock()
		acc.activeAudioBridges[call.ID()] = bridge
		acc.activeAudioBridgesMutex.Unlock()

		videoPort := 0
		if videoBridge, vport, err := NewVideoBridge(call.SendVideoWithDuration); err != nil {
			log.Println("Failed to create video bridge:", err)
		} else {
			videoPort = vport
			acc.activeVideoBridgesMutex.Lock()
			acc.activeVideoBridges[call.ID()] = videoBridge
			acc.activeVideoBridgesMutex.Unlock()
			go videoBridge.Run()
		}
		screenPort := 0
		if dualStreamScreenShare.Load() {
			if screenBridge, sport, err := NewVideoBridge(call.SendScreenVideo); err != nil {
				log.Println("Failed to create screen bridge:", err)
			} else {
				screenPort = sport
				acc.activeVideoBridgesMutex.Lock()
				acc.activeScreenBridges[call.ID()] = screenBridge
				acc.activeVideoBridgesMutex.Unlock()
				go screenBridge.Run()
			}
		}

		acc.broadcastEvent("call_connected", map[string]interface{}{
			"call_id":     call.ID(),
			"audio_port":  port,
			"video_port":  videoPort,
			"screen_port": screenPort,
		})
	})
	call.OnEnd(func(reason string) {
		outgoingCallProgressMutex.Lock()
		delete(outgoingCallProgress, call.ID())
		outgoingCallProgressMutex.Unlock()
		summary := acc.finishLiveCall(call.ID(), reason)
		acc.broadcastEvent("call_ended", map[string]string{
			"call_id": call.ID(),
			"reason":  reason,
			"summary": summary,
		})
		acc.activeCallsMutex.Lock()
		delete(acc.activeCalls, call.ID())
		acc.activeCallsMutex.Unlock()

		acc.activeVideoBridgesMutex.Lock()
		if bridge, ok := acc.activeVideoBridges[call.ID()]; ok {
			bridge.Close()
			delete(acc.activeVideoBridges, call.ID())
		}
		if bridge, ok := acc.activeScreenBridges[call.ID()]; ok {
			bridge.Close()
			delete(acc.activeScreenBridges, call.ID())
		}
		acc.activeVideoBridgesMutex.Unlock()
		acc.forgetScreenShare(call.ID())

		acc.activeAudioBridgesMutex.Lock()
		if bridge, ok := acc.activeAudioBridges[call.ID()]; ok {
			bridge.Close()
			delete(acc.activeAudioBridges, call.ID())
		}
		acc.activeAudioBridgesMutex.Unlock()
	})
	call.OnGroupState(func(state meowcaller.GroupCallState) {
		participants := make([]map[string]string, 0, len(state.Participants))
		for _, p := range state.Participants {
			if acc.isOwnJID(p.JID) || acc.isOwnJID(p.PN) {
				continue
			}
			participants = append(participants, map[string]string{
				"jid":   p.JID.String(),
				"name":  acc.callJoinerName(p.JID, p.PN, p.PushName, p.Guest),
				"state": p.State,
			})
		}
		acc.broadcastEvent("call_participants", map[string]interface{}{
			"call_id":      call.ID(),
			"participants": participants,
			"can_remove":   call.CanRemoveParticipants(),
		})
	})
	call.OnVideoState(func(vs meowcaller.VideoState) {
		event := peerVideoEvent(vs.Raw)
		if event == "" {
			return
		}
		acc.broadcastEvent("call_video_state", map[string]string{
			"call_id":    call.ID(),
			"peer_video": event,
		})
	})
	var peerMuted atomic.Bool
	call.OnMuteState(func(muted bool) { peerMuted.Store(muted) })
	call.OnAudioHealth(func(health meowcaller.AudioHealth) {
		if peerMuted.Load() {
			return
		}
		acc.broadcastEvent("call_audio_health", map[string]interface{}{
			"call_id":    call.ID(),
			"stalled":    health.Stalled,
			"reason":     string(health.Reason),
			"silent_for": int(health.SilentFor.Seconds()),
		})
	})
	// The call's media stopped and meowcaller is redialing its relays, or
	// it came back. One that never comes back ends as "connection_lost".
	call.OnReconnecting(func(reconnecting bool) {
		acc.broadcastEvent("call_reconnecting", map[string]interface{}{
			"call_id":      call.ID(),
			"reconnecting": reconnecting,
		})
	})
	// What the peer's reports say about our outbound audio, so the app can
	// size the encoder's loss protection to the network, as WhatsApp's does.
	go func() {
		last := uint32(0)
		for range time.Tick(2 * time.Second) {
			if call.State() == meowcaller.CallPhaseEnded {
				return
			}
			if loss := call.MediaStats().PeerReportedLossPercent; loss != last {
				last = loss
				acc.broadcastEvent("call_loss", map[string]interface{}{"call_id": call.ID(), "percent": loss})
			}
		}
	}()
	call.OnVideoKeyframeRequest(func() {
		acc.broadcastEvent("call_video_keyframe_request", map[string]string{
			"call_id": call.ID(),
		})
	})
	call.OnScreenKeyframeRequest(func() {
		acc.broadcastEvent("call_screen_keyframe_request", map[string]string{
			"call_id": call.ID(),
		})
	})
	acc.attachCallExtras(call)
}

func (acc *Account) initCallClient() {
	if acc.wa() == nil || acc.callClient != nil {
		return
	}
	// Must be registered before meowcaller.NewClient installs its own
	// handler - see incomingGroupCallJIDs.
	acc.wa().AddEventHandler(acc.recordIncomingGroupCallJID)
	acc.wa().AddEventHandler(acc.noteOwnDeviceReject)
	acc.wa().AddEventHandler(acc.noteOfflineMissedCall)
	acc.wa().AddEventHandler(acc.noteOutgoingCallProgress)
	acc.callClient = meowcaller.NewClient(acc.wa(), newCallDiagnosticsOptions()...)
	acc.callClient.SetDualStreamScreenShare(dualStreamScreenShare.Load())
	acc.watchJoinableCalls(acc.callClient)

	acc.callClient.OnIncomingCall(func(call *meowcaller.Call) {
		log.Printf("Incoming call from %v\n", call.Peer())
		groupJID, hasGroupJID := acc.takeIncomingGroupCallJID(call.ID())

		acc.activeCallsMutex.Lock()
		// A call only leaves activeCalls through its OnEnd handler, and one
		// that ended before attachCallHandlers registered it never gets
		// there. Drop those here, or whatSnap stays "busy" and declines every
		// call until it restarts.
		for id, c := range acc.activeCalls {
			if c.State() == meowcaller.CallPhaseEnded {
				delete(acc.activeCalls, id)
			}
		}
		alreadyOnACall := len(acc.activeCalls) > 0
		if !alreadyOnACall {
			acc.activeCalls[call.ID()] = call
		}
		acc.activeCallsMutex.Unlock()

		_, isGroupCall := call.GroupState()
		chatJID := call.Peer()
		if hasGroupJID {
			chatJID = groupJID
		}
		acc.trackLiveCall(call.ID(), chatJID, call.Peer(), true, call.IsVideo(), isGroupCall)

		if alreadyOnACall {
			defer acc.finishLiveCall(call.ID(), "busy")
			log.Printf("Already on a call, rejecting incoming call from %v as busy\n", call.Peer())
			if err := call.RejectBusy(); err != nil {
				log.Println("Failed to auto-reject incoming call while busy:", err)
			}
			return
		}

		acc.attachCallHandlers(call)

		peer := acc.preferPN(call.Peer().ToNonAD())
		payload := map[string]interface{}{
			"call_id":      call.ID(),
			"caller":       peer.String(),
			"chat_jid":     acc.preferPN(chatJID).String(),
			"caller_name":  acc.resolveDisplayName(peer),
			"caller_phone": acc.callerPhoneDigits(peer),
			"is_group":     isGroupCall,
			"is_video":     call.IsVideo(),
		}
		if hasGroupJID {
			payload["group_jid"] = groupJID.String()
			payload["group_name"] = acc.groupChatName(groupJID)
		}
		acc.broadcastEvent("incoming_call", payload)
	})
}

// callerPhoneDigits is the caller's phone number, digits only, to show beside
// their name on an incoming call. It's "" when we don't know it: a caller who
// hides their number is a LID, and only a LID we've mapped to a number has one.
func (acc *Account) callerPhoneDigits(jid types.JID) string {
	jid = jid.ToNonAD()
	pn := acc.preferPN(jid)
	if pn.Server == types.DefaultUserServer {
		return pn.User
	}
	if acc.db != nil {
		var phone string
		_ = acc.db.QueryRow(`SELECT phone FROM contacts_cache WHERE (jid = ? OR jid = ?) AND phone != '' LIMIT 1`, jid.String(), pn.String()).Scan(&phone)
		if phone != "" {
			return phone
		}
	}
	return ""
}

func (acc *Account) placeGroupChatCall(ctx context.Context, groupJID types.JID, video bool) (*meowcaller.Call, error) {
	info, err := acc.wa().GetGroupInfo(ctx, groupJID)
	if err != nil {
		return nil, fmt.Errorf("couldn't load the group's members: %w", err)
	}
	var others []types.JID
	for _, p := range info.Participants {
		if acc.isOwnJID(p.JID) || acc.isOwnJID(p.PhoneNumber) || acc.isOwnJID(p.LID) {
			continue
		}
		others = append(others, p.JID)
	}
	switch {
	case len(others) == 0:
		return nil, errors.New("There's no one else in this group to call.")
	case len(others) > 31:
		return nil, fmt.Errorf("This group has %d other members; WhatsApp group calls allow at most 32 people.", len(others))
	}
	return acc.callClient.GroupCallByIDWithOptions(ctx, groupJID.String(), meowcaller.GroupCallOptions{Video: video})
}

func (acc *Account) handlePlaceCall(data []byte) {
	var req struct {
		JID   string `json:"jid"`
		Video bool   `json:"video"`
	}
	if err := json.Unmarshal(data, &req); err != nil {
		log.Println("Invalid place_call format:", err)
		return
	}
	target := strings.TrimSpace(req.JID)
	if !strings.Contains(target, "@") {
		var digits strings.Builder
		for _, r := range target {
			if r >= '0' && r <= '9' {
				digits.WriteRune(r)
			}
		}
		if digits.Len() > 0 {
			target = digits.String() + "@s.whatsapp.net"
		}
	}
	parsedJID, err := types.ParseJID(target)
	if err != nil {
		log.Println("Invalid JID for place_call:", err)
		return
	}

	if acc.callClient == nil {
		acc.initCallClient()
	}

	ctx := context.Background()
	var call *meowcaller.Call
	if parsedJID.Server == types.GroupServer {
		call, err = acc.placeGroupChatCall(ctx, parsedJID, req.Video)
	} else if req.Video {
		call, err = acc.callClient.CallWithOptions(ctx, parsedJID.String(), meowcaller.CallOptions{Video: true})
	} else {
		call, err = acc.callClient.Call(ctx, parsedJID.String())
	}
	if err != nil {
		log.Println("Failed to place call:", err)
		// Without this, the frontend never hears back at all: it shows the
		// call UI optimistically on place_call and only learns the real
		// call_id (and un-races its Leave button) from call_placed. If the
		// offer never made it onto the wire, that event never arrives and
		// the UI is stuck showing a live-looking call forever with no way
		// to dismiss it.
		acc.broadcastEvent("call_failed", map[string]string{
			"jid":     parsedJID.String(),
			"message": err.Error(),
		})
		return
	}

	acc.activeCallsMutex.Lock()
	for id, c := range acc.activeCalls {
		if c.State() == meowcaller.CallPhaseEnded {
			delete(acc.activeCalls, id)
		}
	}
	acc.activeCalls[call.ID()] = call
	acc.activeCallsMutex.Unlock()

	chatJID := call.Peer()
	_, isGroupCall := call.GroupState()
	if parsedJID.Server == types.GroupServer && isGroupCall {
		chatJID = parsedJID
	}
	acc.trackLiveCall(call.ID(), chatJID, acc.ownCallCreator(), false, req.Video, isGroupCall)

	peerName := acc.resolveDisplayName(parsedJID)
	selfPhone := ""
	if acc.wa().Store.ID != nil {
		selfPhone = acc.wa().Store.ID.User
	}
	acc.broadcastEvent("call_placed", map[string]string{
		"call_id":    call.ID(),
		"peer":       parsedJID.String(),
		"peer_name":  peerName,
		"self_phone": selfPhone,
	})
	outgoingCallProgressMutex.Lock()
	progress := outgoingCallProgress[call.ID()]
	if progress == "" {
		progress = "calling"
		outgoingCallProgress[call.ID()] = progress
	}
	outgoingCallProgressMutex.Unlock()
	acc.broadcastCallProgress(call.ID(), progress)
	acc.attachCallHandlers(call)

	log.Printf("Placed call to %s, call ID: %s", parsedJID.String(), call.ID())
}

func (acc *Account) handleAnswerCall(data []byte) {
	var req struct {
		CallID string `json:"call_id"`
	}
	if err := json.Unmarshal(data, &req); err != nil {
		log.Println("Invalid answer_call format:", err)
		return
	}

	acc.activeCallsMutex.Lock()
	call, exists := acc.activeCalls[req.CallID]
	acc.activeCallsMutex.Unlock()

	if !exists {
		log.Println("Cannot answer call, not found:", req.CallID)
		acc.broadcastEvent("call_answer_failed", map[string]string{
			"call_id": req.CallID,
			"message": "That call is no longer available.",
		})
		return
	}

	err := call.Answer()
	if err != nil {
		log.Println("Failed to answer call:", err)
		acc.broadcastEvent("call_answer_failed", map[string]string{
			"call_id": req.CallID,
			"message": "Failed to answer call: " + err.Error(),
		})
	} else {
		log.Println("Answered call:", req.CallID)
	}
}

func (acc *Account) handleRejectCall(data []byte) {
	var req struct {
		CallID string `json:"call_id"`
	}
	if err := json.Unmarshal(data, &req); err != nil {
		log.Println("Invalid reject_call format:", err)
		return
	}

	acc.activeCallsMutex.Lock()
	call, exists := acc.activeCalls[req.CallID]
	acc.activeCallsMutex.Unlock()

	if !exists {
		log.Println("Cannot reject call, not found:", req.CallID)
		return
	}

	acc.updateLiveCall(req.CallID, func(c *liveCall) { c.localReject = true })
	err := call.Reject()
	if err != nil {
		log.Println("Failed to reject call:", err)
		acc.activeCallsMutex.Lock()
		delete(acc.activeCalls, req.CallID)
		acc.activeCallsMutex.Unlock()
	} else {
		log.Println("Rejected call:", req.CallID)
	}
}

func (acc *Account) handleLeaveCall(data []byte) {
	var req struct {
		CallID string `json:"call_id"`
	}
	if err := json.Unmarshal(data, &req); err != nil {
		log.Println("Invalid leave_call format:", err)
		return
	}

	acc.activeCallsMutex.Lock()
	call, exists := acc.activeCalls[req.CallID]
	acc.activeCallsMutex.Unlock()

	if !exists {
		log.Println("Cannot leave call, not found:", req.CallID)
		return
	}

	acc.updateLiveCall(req.CallID, func(c *liveCall) { c.localHangup = true })
	err := call.Hangup()
	if err != nil {
		log.Println("Failed to leave call:", err)
		acc.activeCallsMutex.Lock()
		delete(acc.activeCalls, req.CallID)
		acc.activeCallsMutex.Unlock()
	} else {
		log.Println("Left call:", req.CallID)
	}
}

// handleMuteCall tells the other side of a call that our microphone was
// muted or unmuted. The frontend already silences the audio itself; without
// this the peer's call screen never showed us as muted.
func (acc *Account) handleMuteCall(data []byte) {
	var req struct {
		CallID string `json:"call_id"`
		Muted  bool   `json:"muted"`
	}
	if err := json.Unmarshal(data, &req); err != nil {
		log.Println("Invalid mute_call format:", err)
		return
	}
	acc.activeCallsMutex.Lock()
	call, exists := acc.activeCalls[req.CallID]
	acc.activeCallsMutex.Unlock()
	if !exists {
		return
	}
	go func() {
		if err := call.SetMuted(req.Muted); err != nil {
			log.Println("Failed to announce mute state:", err)
		}
	}()
}

func (acc *Account) handleSetVideo(data []byte) {
	var req struct {
		CallID  string `json:"call_id"`
		Enabled bool   `json:"enabled"`
	}
	if err := json.Unmarshal(data, &req); err != nil {
		log.Println("Invalid set_video format:", err)
		return
	}

	acc.activeCallsMutex.Lock()
	call, exists := acc.activeCalls[req.CallID]
	acc.activeCallsMutex.Unlock()

	if !exists {
		log.Println("Cannot set video, call not found:", req.CallID)
		return
	}

	var err error
	switch {
	case req.Enabled && !call.IsVideo():
		err = call.StartVideo()
	default:
		err = call.SetVideoEnabled(req.Enabled)
	}
	if err != nil {
		log.Println("Failed to set video:", err)
		acc.broadcastEvent("call_set_video_failed", map[string]string{
			"call_id": req.CallID,
			"message": err.Error(),
		})
		return
	}
	log.Println("Set video for call:", req.CallID, "enabled:", req.Enabled)
}

type localScreenShare struct {
	wanted    bool
	announced bool
	dual      bool
}

var dualStreamScreenShare atomic.Bool

func (acc *Account) handleSetCallOptions(data []byte) {
	var req struct {
		DualStreamScreenShare bool `json:"dual_stream_screen_share"`
	}
	if err := json.Unmarshal(data, &req); err != nil {
		log.Println("Invalid set_call_options format:", err)
		return
	}
	dualStreamScreenShare.Store(req.DualStreamScreenShare)
	if acc.callClient != nil {
		acc.callClient.SetDualStreamScreenShare(req.DualStreamScreenShare)
	}
}

func (acc *Account) forgetScreenShare(callID string) {
	acc.localScreenSharesMutex.Lock()
	delete(acc.localScreenShares, callID)
	acc.localScreenSharesMutex.Unlock()
}

const screenShareAnnounceWindow = 15 * time.Second

func (acc *Account) handleSetScreenShare(data []byte) {
	var req struct {
		CallID  string `json:"call_id"`
		Enabled bool   `json:"enabled"`
		Dual    bool   `json:"dual"`
	}
	if err := json.Unmarshal(data, &req); err != nil {
		log.Println("Invalid set_screen_share format:", err)
		return
	}

	acc.activeCallsMutex.Lock()
	call, exists := acc.activeCalls[req.CallID]
	acc.activeCallsMutex.Unlock()
	if !exists {
		log.Println("Cannot set screen share, call not found:", req.CallID)
		return
	}

	acc.localScreenSharesMutex.Lock()
	share := acc.localScreenShares[req.CallID]
	if share == nil {
		share = &localScreenShare{}
		acc.localScreenShares[req.CallID] = share
	}
	alreadyWanted := share.wanted
	share.wanted = req.Enabled
	if req.Enabled && !alreadyWanted {
		share.dual = req.Dual
	}
	announced := share.announced
	if !req.Enabled {
		share.announced = false
	}
	acc.localScreenSharesMutex.Unlock()

	if req.Enabled {
		if !alreadyWanted {
			go acc.announceScreenShare(call, share)
		}
		return
	}
	if announced {
		go func() {
			if err := call.StopScreenShare(); err != nil {
				log.Println("Failed to announce the end of the screen share:", err)
			}
		}()
	}
}

func (acc *Account) announceScreenShare(call *meowcaller.Call, share *localScreenShare) {
	deadline := time.Now().Add(screenShareAnnounceWindow)
	for {
		acc.localScreenSharesMutex.Lock()
		wanted, dual := share.wanted, share.dual
		acc.localScreenSharesMutex.Unlock()
		if !wanted || call.State() == meowcaller.CallPhaseEnded {
			return
		}
		var err error
		if dual {
			err = call.StartDualStreamScreenShare(nil)
		} else {
			err = call.StartScreenShare(nil)
		}
		switch {
		case err == nil:
			acc.localScreenSharesMutex.Lock()
			stillWanted := share.wanted
			share.announced = stillWanted
			acc.localScreenSharesMutex.Unlock()
			if !stillWanted {
				if err := call.StopScreenShare(); err != nil {
					log.Println("Failed to announce the end of the screen share:", err)
				}
			}
			return
		case errors.Is(err, meowcaller.ErrScreenShareNeedsGroupCall) && time.Now().Before(deadline):
			time.Sleep(250 * time.Millisecond)
		case errors.Is(err, meowcaller.ErrScreenShareNeedsVideo) && time.Now().Before(deadline):
			time.Sleep(250 * time.Millisecond)
		default:
			log.Println("Failed to announce the screen share:", err)
			msg := "screen sharing isn't available on this call"
			if dual {
				msg = "dual-stream screen sharing isn't available on this call"
			}
			acc.broadcastEvent("call_screen_share_failed", map[string]string{
				"call_id": call.ID(),
				"message": msg,
			})
			return
		}
	}
}

func (acc *Account) handleAddParticipant(data []byte) {
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

	acc.activeCallsMutex.Lock()
	call, exists := acc.activeCalls[req.CallID]
	acc.activeCallsMutex.Unlock()

	name := acc.resolveDisplayName(parsedJID)
	fail := func(message string) {
		acc.broadcastEvent("call_participant_add_failed", map[string]string{
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
	if err != nil && strings.Contains(err.Error(), "already in the call roster") {
		// Someone who was invited but never picked up, or who left, is still
		// on the roster: adding them again means ringing them again.
		err = call.RingParticipant(context.Background(), parsedJID.String())
	}
	if err != nil {
		log.Println("Failed to add participant:", err)
		fail(addParticipantErrorMessage(err, name))
		return
	}
	log.Println("Added participant to call:", req.CallID)
	acc.broadcastEvent("call_participant_added", map[string]string{
		"call_id": req.CallID,
		"jid":     parsedJID.String(),
		"name":    name,
	})
}

func addParticipantErrorMessage(err error, name string) string {
	msg := err.Error()
	switch {
	case strings.Contains(msg, "call is not connected"):
		return "Wait for the call to connect before adding someone."
	case strings.Contains(msg, "already in the call roster"), strings.Contains(msg, "already connected"):
		return name + " is already in the call."
	case strings.Contains(msg, "limit reached"):
		return "Couldn't add " + name + ": the call is full."
	case strings.Contains(msg, "has no devices"):
		return "Couldn't add " + name + ": they have no device that can take a call."
	case strings.Contains(msg, "cannot add this client to its own call"):
		return "You can't add yourself to the call."
	case strings.Contains(msg, "capability is unavailable"):
		return "Couldn't add " + name + ": WhatsApp hasn't sent the call details needed to invite someone yet. Try again in a moment."
	}
	return "Couldn't add " + name + ": " + msg
}
