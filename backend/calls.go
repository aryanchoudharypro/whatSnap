package main

import (
	"context"
	"log"
	"encoding/json"
	"github.com/purpshell/meowcaller"
	"go.mau.fi/whatsmeow/types"
)

var callClient *meowcaller.Client

func initCallClient() {
	if client == nil {
		return
	}
	callClient = meowcaller.NewClient(client)
	
	callClient.OnIncomingCall(func(call *meowcaller.Call) {
		log.Printf("Incoming call from %v\n", call.Peer())
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
	
	log.Printf("Placed call to %s, call ID: %s", parsedJID.String(), call.ID())
}
