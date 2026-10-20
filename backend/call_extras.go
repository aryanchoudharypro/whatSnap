package main

import (
	"cmp"
	"context"
	"encoding/json"
	"errors"
	"log"
	"net"

	"github.com/purpshell/meowcaller"
	"go.mau.fi/whatsmeow/types"
)

// Call links, raising a hand, reactions, others' screen shares and a call
// link's waiting room, all of which meowcaller does; this is the IPC around them.

type CallLinkData struct {
	Link  string `json:"link"`
	Video bool   `json:"video"`
}

type CallToggleData struct {
	CallID string `json:"call_id"`
	On     bool   `json:"on"`
	Emoji  string `json:"emoji"`
	JID    string `json:"jid"`
}

func activeCall(callID string) *meowcaller.Call {
	activeCallsMutex.Lock()
	defer activeCallsMutex.Unlock()
	return activeCalls[callID]
}

// callPersonName names someone in a call by their phone number if we know it.
func callPersonName(jid, pn types.JID) string {
	if !pn.IsEmpty() {
		return resolveDisplayName(pn)
	}
	return resolveDisplayName(preferPN(jid))
}

// callJoinerName names someone who joined through a call link. Guests have no
// account, so the name they typed in the browser is all we have.
func callJoinerName(jid, pn types.JID, pushName string, guest bool) string {
	if guest {
		return cmp.Or(pushName, "Guest") + " (guest)"
	}
	return callPersonName(jid, pn)
}

// attachCallExtras wires the hand, reaction, screen share and waiting room
// listeners; attachCallHandlers calls it for every call.
func attachCallExtras(call *meowcaller.Call) {
	call.OnHandRaise(func(h meowcaller.HandRaiseState) {
		if isOwnJID(h.Participant) {
			return
		}
		broadcastEvent("call_hand", map[string]interface{}{
			"call_id": call.ID(), "name": callPersonName(h.Participant, types.EmptyJID), "raised": h.Raised,
		})
	})
	call.OnReaction(func(r meowcaller.CallReaction) {
		if isOwnJID(r.Sender) || r.Removed || r.Emoji == "" {
			return
		}
		broadcastEvent("call_reaction", map[string]interface{}{
			"call_id": call.ID(), "name": callPersonName(r.Sender, types.EmptyJID), "emoji": r.Emoji,
		})
	})
	call.OnScreenShare(func(s meowcaller.ScreenShareState) {
		if isOwnJID(s.Participant) || s.Synthetic {
			return
		}
		broadcastEvent("call_screen_share", map[string]interface{}{
			"call_id": call.ID(), "name": callPersonName(s.Participant, types.EmptyJID), "active": s.Active,
		})
	})
	call.OnWaitingRoomState(func(w meowcaller.WaitingRoomState) {
		users := []map[string]string{}
		for _, u := range w.Users {
			users = append(users, map[string]string{
				"jid": u.JID.String(), "name": callJoinerName(u.JID, u.PN, u.PushName, u.Guest), "state": u.State,
			})
		}
		broadcastEvent("call_waiting_room", map[string]interface{}{
			"call_id": call.ID(), "waiting": w.InWaitingRoom, "is_admin": w.IsAdmin,
			"enabled": w.Enabled, "users": users,
		})
	})
}

// handleCreateCallLink makes a call link to share; whoever opens it first
// starts the call.
func handleCreateCallLink(conn net.Conn, raw json.RawMessage) {
	var data CallLinkData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		if callClient == nil {
			initCallClient()
		}
		if callClient == nil {
			sendEvent(conn, "error", map[string]string{"message": "Calls aren't available yet."})
			return
		}
		link, err := callClient.CreateCallLink(context.Background(), meowcaller.CallLinkOptions{Video: data.Video})
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": "Couldn't create the call link: " + err.Error()})
			return
		}
		sendEvent(conn, "call_link_created", map[string]interface{}{"url": link.URL, "video": link.Video})
	}()
}

// handleJoinCallLink joins the call a link is for, or its waiting room when
// the link needs an admin to let people in. The frontend handles it as an
// outgoing group call.
func handleJoinCallLink(raw json.RawMessage) {
	var data CallLinkData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		if callClient == nil {
			initCallClient()
		}
		fail := func(err error) {
			broadcastEvent("call_failed", map[string]string{"jid": "", "message": "Couldn't join the call: " + err.Error()})
		}
		if callClient == nil {
			fail(errors.New("calls aren't available yet"))
			return
		}
		ctx := context.Background()
		video := data.Video
		if preview, err := callClient.PreviewCallLink(ctx, data.Link, meowcaller.CallLinkOptions{Video: data.Video}); err == nil {
			video = preview.Video
		}
		call, err := callClient.JoinCallLink(ctx, data.Link, meowcaller.CallLinkOptions{Video: video})
		if err != nil {
			fail(err)
			return
		}
		activeCallsMutex.Lock()
		activeCalls[call.ID()] = call
		activeCallsMutex.Unlock()
		trackLiveCall(call.ID(), call.Peer(), false, video, true)
		broadcastEvent("call_placed", map[string]interface{}{
			"call_id": call.ID(), "peer": call.Peer().String(), "link": true, "video": video,
		})
		attachCallHandlers(call)
		log.Printf("Joined call link, call ID: %s", call.ID())
	}()
}

func handleCallToggle(conn net.Conn, raw json.RawMessage, what string) {
	var data CallToggleData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	call := activeCall(data.CallID)
	if call == nil {
		return
	}
	go func() {
		var err error
		switch what {
		case "hand":
			err = call.SetHandRaised(data.On)
		case "reaction":
			err = call.SendReaction(data.Emoji)
		case "approval":
			err = call.SetApprovalRequired(context.Background(), data.On)
		case "admit":
			if data.On {
				err = call.AdmitParticipant(context.Background(), data.JID)
			} else {
				err = call.DenyParticipant(context.Background(), data.JID)
			}
		}
		if err != nil {
			sendEvent(conn, "call_action_failed", map[string]string{"call_id": data.CallID, "what": what, "message": err.Error()})
			return
		}
		sendEvent(conn, "call_action_done", map[string]interface{}{"call_id": data.CallID, "what": what, "on": data.On})
	}()
}
