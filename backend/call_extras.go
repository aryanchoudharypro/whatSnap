package main

import (
	"cmp"
	"context"
	"encoding/json"
	"errors"
	"log"
	"net"
	"strings"

	"github.com/purpshell/meowcaller"
	"go.mau.fi/whatsmeow/types"
)

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

func (acc *Account) activeCall(callID string) *meowcaller.Call {
	acc.activeCallsMutex.Lock()
	defer acc.activeCallsMutex.Unlock()
	return acc.activeCalls[callID]
}

func (acc *Account) callPersonName(jid, pn types.JID) string {
	if !pn.IsEmpty() {
		return acc.resolveDisplayName(pn)
	}
	return acc.resolveDisplayName(acc.preferPN(jid))
}

func (acc *Account) callJoinerName(jid, pn types.JID, pushName string, guest bool) string {
	if guest {
		return cmp.Or(pushName, "Guest") + " (guest)"
	}
	return acc.callPersonName(jid, pn)
}

func (acc *Account) attachCallExtras(call *meowcaller.Call) {
	call.OnMuteRequested(func(by types.JID) {
		name := ""
		if !by.IsEmpty() {
			name = acc.callPersonName(by, types.EmptyJID)
		}
		acc.broadcastEvent("call_muted_by", map[string]interface{}{"call_id": call.ID(), "name": name})
	})
	call.OnHandRaise(func(h meowcaller.HandRaiseState) {
		if acc.isOwnJID(h.Participant) {
			return
		}
		acc.broadcastEvent("call_hand", map[string]interface{}{
			"call_id": call.ID(), "name": acc.callPersonName(h.Participant, types.EmptyJID), "raised": h.Raised,
		})
	})
	call.OnReaction(func(r meowcaller.CallReaction) {
		if acc.isOwnJID(r.Sender) || r.Removed || r.Emoji == "" {
			return
		}
		acc.broadcastEvent("call_reaction", map[string]interface{}{
			"call_id": call.ID(), "name": acc.callPersonName(r.Sender, types.EmptyJID), "emoji": r.Emoji,
		})
	})
	call.OnScreenShare(func(s meowcaller.ScreenShareState) {
		if acc.isOwnJID(s.Participant) || s.Synthetic {
			return
		}
		acc.broadcastEvent("call_screen_share", map[string]interface{}{
			"call_id": call.ID(), "name": acc.callPersonName(s.Participant, types.EmptyJID), "active": s.Active,
		})
	})
	call.OnWaitingRoomState(func(w meowcaller.WaitingRoomState) {
		users := []map[string]string{}
		for _, u := range w.Users {
			users = append(users, map[string]string{
				"jid": u.JID.String(), "name": acc.callJoinerName(u.JID, u.PN, u.PushName, u.Guest), "state": u.State,
			})
		}
		acc.broadcastEvent("call_waiting_room", map[string]interface{}{
			"call_id": call.ID(), "waiting": w.InWaitingRoom, "is_admin": w.IsAdmin,
			"enabled": w.Enabled, "users": users,
		})
	})
}

func (acc *Account) handleCreateCallLink(conn net.Conn, raw json.RawMessage) {
	var data CallLinkData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		if acc.callClient == nil {
			acc.initCallClient()
		}
		if acc.callClient == nil {
			acc.sendEvent(conn, "error", map[string]string{"message": "Calls aren't available yet."})
			return
		}
		link, err := acc.callClient.CreateCallLink(context.Background(), meowcaller.CallLinkOptions{Video: data.Video})
		if err != nil {
			acc.sendEvent(conn, "error", map[string]string{"message": "Couldn't create the call link: " + err.Error()})
			return
		}
		acc.sendEvent(conn, "call_link_created", map[string]interface{}{"url": link.URL, "video": link.Video})
	}()
}

func callLinkJoinPushName(name string) (string, error) {
	name = strings.TrimSpace(name)
	if name == "" {
		return "", errors.New("missing push name")
	}
	return name, nil
}

func (acc *Account) handleJoinCallLink(raw json.RawMessage) {
	var data CallLinkData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		if acc.callClient == nil {
			acc.initCallClient()
		}
		fail := func(err error) {
			acc.broadcastEvent("call_failed", map[string]string{"jid": "", "message": "Couldn't join the call: " + err.Error()})
		}
		if acc.callClient == nil {
			fail(errors.New("calls aren't available yet"))
			return
		}
		ctx := context.Background()
		video := data.Video
		if preview, err := acc.callClient.PreviewCallLink(ctx, data.Link, meowcaller.CallLinkOptions{Video: data.Video}); err == nil {
			video = preview.Video
		}
		call, err := acc.callClient.JoinCallLink(ctx, data.Link, meowcaller.CallLinkOptions{Video: video})
		if err != nil {
			fail(err)
			return
		}
		acc.activeCallsMutex.Lock()
		acc.activeCalls[call.ID()] = call
		acc.activeCallsMutex.Unlock()
		acc.trackLiveCall(call.ID(), call.Peer(), types.EmptyJID, false, video, true)
		acc.broadcastEvent("call_placed", map[string]interface{}{
			"call_id": call.ID(), "peer": call.Peer().String(), "link": true, "video": video,
		})
		acc.attachCallHandlers(call)
		log.Printf("Joined call link, call ID: %s", call.ID())
	}()
}

func (acc *Account) handleCallToggle(conn net.Conn, raw json.RawMessage, what string) {
	var data CallToggleData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	call := acc.activeCall(data.CallID)
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
		case "mute_person":
			err = call.RequestPeerMute(context.Background(), data.JID)
		case "remove_person":
			err = call.RemoveParticipant(context.Background(), data.JID)
		case "admit":
			if data.On {
				err = call.AdmitParticipant(context.Background(), data.JID)
			} else {
				err = call.DenyParticipant(context.Background(), data.JID)
			}
		}
		if err != nil {
			acc.sendEvent(conn, "call_action_failed", map[string]string{"call_id": data.CallID, "what": what, "message": err.Error()})
			return
		}
		acc.sendEvent(conn, "call_action_done", map[string]interface{}{"call_id": data.CallID, "what": what, "on": data.On})
	}()
}
