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

var callClient *meowcaller.Client
var activeCalls = make(map[string]*meowcaller.Call)
var activeCallsMutex sync.Mutex

var activeVideoBridges = make(map[string]*VideoBridge)
var activeVideoBridgesMutex sync.Mutex

var activeScreenBridges = make(map[string]*VideoBridge)

// Audio bridges, one per active call, keyed the same way as activeCalls. Each
// wraps a real UDP socket (see audio_udp.go), so it must be closed on
// OnEnd just like activeVideoBridges - otherwise every call leaks the socket
// for the rest of the process's life.
var activeAudioBridges = make(map[string]*UDPBridge)
var activeAudioBridgesMutex sync.Mutex

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
	if !ok || offer.Data == nil || offer.Offline {
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

		videoPort := 0
		if videoBridge, vport, err := NewVideoBridge(call.SendVideoWithDuration); err != nil {
			log.Println("Failed to create video bridge:", err)
		} else {
			videoPort = vport
			activeVideoBridgesMutex.Lock()
			activeVideoBridges[call.ID()] = videoBridge
			activeVideoBridgesMutex.Unlock()
			go videoBridge.Run()
		}
		screenPort := 0
		if dualStreamScreenShare.Load() {
			if screenBridge, sport, err := NewVideoBridge(call.SendScreenVideo); err != nil {
				log.Println("Failed to create screen bridge:", err)
			} else {
				screenPort = sport
				activeVideoBridgesMutex.Lock()
				activeScreenBridges[call.ID()] = screenBridge
				activeVideoBridgesMutex.Unlock()
				go screenBridge.Run()
			}
		}

		broadcastEvent("call_connected", map[string]interface{}{
			"call_id":     call.ID(),
			"audio_port":  port,
			"video_port":  videoPort,
			"screen_port": screenPort,
		})
	})
	call.OnEnd(func(reason string) {
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
		if bridge, ok := activeScreenBridges[call.ID()]; ok {
			bridge.Close()
			delete(activeScreenBridges, call.ID())
		}
		activeVideoBridgesMutex.Unlock()
		forgetScreenShare(call.ID())

		activeAudioBridgesMutex.Lock()
		if bridge, ok := activeAudioBridges[call.ID()]; ok {
			bridge.Close()
			delete(activeAudioBridges, call.ID())
		}
		activeAudioBridgesMutex.Unlock()
	})
	call.OnGroupState(func(state meowcaller.GroupCallState) {
		participants := make([]map[string]string, 0, len(state.Participants))
		for _, p := range state.Participants {
			if isOwnJID(p.JID) || isOwnJID(p.PN) {
				continue
			}
			participants = append(participants, map[string]string{
				"jid":   p.JID.String(),
				"name":  callJoinerName(p.JID, p.PN, p.PushName, p.Guest),
				"state": p.State,
			})
		}
		broadcastEvent("call_participants", map[string]interface{}{
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
		broadcastEvent("call_video_state", map[string]string{
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
		broadcastEvent("call_audio_health", map[string]interface{}{
			"call_id":    call.ID(),
			"stalled":    health.Stalled,
			"reason":     string(health.Reason),
			"silent_for": int(health.SilentFor.Seconds()),
		})
	})
	// The call's media stopped and meowcaller is redialing its relays, or
	// it came back. One that never comes back ends as "connection_lost".
	call.OnReconnecting(func(reconnecting bool) {
		broadcastEvent("call_reconnecting", map[string]interface{}{
			"call_id":      call.ID(),
			"reconnecting": reconnecting,
		})
	})
	call.OnVideoKeyframeRequest(func() {
		broadcastEvent("call_video_keyframe_request", map[string]string{
			"call_id": call.ID(),
		})
	})
	call.OnScreenKeyframeRequest(func() {
		broadcastEvent("call_screen_keyframe_request", map[string]string{
			"call_id": call.ID(),
		})
	})
	attachCallExtras(call)
}

func initCallClient() {
	if wa() == nil {
		return
	}
	// Must be registered before meowcaller.NewClient installs its own
	// handler - see incomingGroupCallJIDs.
	wa().AddEventHandler(recordIncomingGroupCallJID)
	wa().AddEventHandler(noteOwnDeviceReject)
	wa().AddEventHandler(noteOfflineMissedCall)
	callClient = meowcaller.NewClient(wa(), newCallDiagnosticsOptions()...)
	callClient.SetDualStreamScreenShare(dualStreamScreenShare.Load())
	watchJoinableCalls(callClient)

	callClient.OnIncomingCall(func(call *meowcaller.Call) {
		log.Printf("Incoming call from %v\n", call.Peer())
		groupJID, hasGroupJID := takeIncomingGroupCallJID(call.ID())

		activeCallsMutex.Lock()
		// A call only leaves activeCalls through its OnEnd handler, and one
		// that ended before attachCallHandlers registered it never gets
		// there. Drop those here, or whatSnap stays "busy" and declines every
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
			defer finishLiveCall(call.ID(), "busy")
			log.Printf("Already on a call, rejecting incoming call from %v as busy\n", call.Peer())
			if err := call.RejectBusy(); err != nil {
				log.Println("Failed to auto-reject incoming call while busy:", err)
			}
			return
		}

		attachCallHandlers(call)

		payload := map[string]interface{}{
			"call_id":      call.ID(),
			"caller":       call.Peer().String(),
			"caller_name":  resolveDisplayName(call.Peer()),
			"caller_phone": callerPhoneDigits(call.Peer()),
			"is_group":     isGroupCall,
			"is_video":     call.IsVideo(),
		}
		if hasGroupJID {
			payload["group_jid"] = groupJID.String()
			payload["group_name"] = groupChatName(groupJID)
		}
		broadcastEvent("incoming_call", payload)
	})
}

// callerPhoneDigits is the caller's phone number, digits only, to show beside
// their name on an incoming call. It's "" when we don't know it: a caller who
// hides their number is a LID, and only a LID we've mapped to a number has one.
func callerPhoneDigits(jid types.JID) string {
	number := jid
	if jid.Server == types.HiddenUserServer && wa() != nil && wa().Store != nil && wa().Store.LIDs != nil {
		if mapped, err := wa().Store.LIDs.GetPNForLID(context.Background(), jid); err == nil && !mapped.IsEmpty() {
			number = mapped
		}
	}
	if number.Server != types.DefaultUserServer {
		return ""
	}
	return number.User
}

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

	chatJID := call.Peer()
	_, isGroupCall := call.GroupState()
	if parsedJID.Server == types.GroupServer && isGroupCall {
		chatJID = parsedJID
	}
	trackLiveCall(call.ID(), chatJID, false, req.Video, isGroupCall)

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
		broadcastEvent("call_answer_failed", map[string]string{
			"call_id": req.CallID,
			"message": "That call is no longer available.",
		})
		return
	}

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

// handleMuteCall tells the other side of a call that our microphone was
// muted or unmuted. The frontend already silences the audio itself; without
// this the peer's call screen never showed us as muted.
func handleMuteCall(data []byte) {
	var req struct {
		CallID string `json:"call_id"`
		Muted  bool   `json:"muted"`
	}
	if err := json.Unmarshal(data, &req); err != nil {
		log.Println("Invalid mute_call format:", err)
		return
	}
	activeCallsMutex.Lock()
	call, exists := activeCalls[req.CallID]
	activeCallsMutex.Unlock()
	if !exists {
		return
	}
	go func() {
		if err := call.SetMuted(req.Muted); err != nil {
			log.Println("Failed to announce mute state:", err)
		}
	}()
}

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

type localScreenShare struct {
	wanted    bool
	announced bool
	dual      bool
}

var (
	localScreenShares      = make(map[string]*localScreenShare)
	localScreenSharesMutex sync.Mutex
)

var dualStreamScreenShare atomic.Bool

func handleSetCallOptions(data []byte) {
	var req struct {
		DualStreamScreenShare bool `json:"dual_stream_screen_share"`
	}
	if err := json.Unmarshal(data, &req); err != nil {
		log.Println("Invalid set_call_options format:", err)
		return
	}
	dualStreamScreenShare.Store(req.DualStreamScreenShare)
	if callClient != nil {
		callClient.SetDualStreamScreenShare(req.DualStreamScreenShare)
	}
}

func forgetScreenShare(callID string) {
	localScreenSharesMutex.Lock()
	delete(localScreenShares, callID)
	localScreenSharesMutex.Unlock()
}

const screenShareAnnounceWindow = 15 * time.Second

func handleSetScreenShare(data []byte) {
	var req struct {
		CallID  string `json:"call_id"`
		Enabled bool   `json:"enabled"`
		Dual    bool   `json:"dual"`
	}
	if err := json.Unmarshal(data, &req); err != nil {
		log.Println("Invalid set_screen_share format:", err)
		return
	}

	activeCallsMutex.Lock()
	call, exists := activeCalls[req.CallID]
	activeCallsMutex.Unlock()
	if !exists {
		log.Println("Cannot set screen share, call not found:", req.CallID)
		return
	}

	localScreenSharesMutex.Lock()
	share := localScreenShares[req.CallID]
	if share == nil {
		share = &localScreenShare{}
		localScreenShares[req.CallID] = share
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
	localScreenSharesMutex.Unlock()

	if req.Enabled {
		if !alreadyWanted {
			go announceScreenShare(call, share)
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

func announceScreenShare(call *meowcaller.Call, share *localScreenShare) {
	deadline := time.Now().Add(screenShareAnnounceWindow)
	for {
		localScreenSharesMutex.Lock()
		wanted, dual := share.wanted, share.dual
		localScreenSharesMutex.Unlock()
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
			localScreenSharesMutex.Lock()
			stillWanted := share.wanted
			share.announced = stillWanted
			localScreenSharesMutex.Unlock()
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
			if dual {
				broadcastEvent("call_screen_share_failed", map[string]string{
					"call_id": call.ID(),
					"message": "dual-stream screen sharing isn't available on this call",
				})
			}
			return
		}
	}
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
	broadcastEvent("call_participant_added", map[string]string{
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
