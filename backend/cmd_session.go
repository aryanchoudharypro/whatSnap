package main

import (
	"context"
	"encoding/json"
	"fmt"

	"github.com/gorilla/websocket"
	"go.mau.fi/whatsmeow"
)

func handleGetQR(conn *websocket.Conn) {
	go func() {
		if client.Store.ID == nil {
			if client != nil && client.IsConnected() {
				client.Disconnect()
			}
			qrChan, err := client.GetQRChannel(context.Background())
			if err != nil {
				sendEvent(conn, "error", map[string]string{"message": err.Error()})
				return
			}
			err = client.Connect()
			if err != nil {
				sendEvent(conn, "error", map[string]string{"message": err.Error()})
				return
			}
			for qr := range qrChan {
				if qr.Event == "code" {
					sendEvent(conn, "qr", map[string]string{"code": qr.Code})
				}
			}
		}
	}()
}

func handleCancelPairing() {
	fmt.Println("Received cancel request")
	if client.Store.ID == nil {
		client.Disconnect()
	}
}

func handlePair(raw json.RawMessage) {
	fmt.Println("Received pair request for:", string(raw))
	var data PairCommandData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	fmt.Println("Parsed phone:", data.Phone)
	go func() {
		if client.Store.ID == nil {
			if !client.IsConnected() {
				qrChan, _ := client.GetQRChannel(context.Background())
				err := client.Connect()
				if err != nil {
					broadcastEvent("error", map[string]string{"message": err.Error()})
					return
				}
				if qrChan != nil {
					<-qrChan
					go func() {
						for range qrChan {
						}
					}()
				}
			}
			code, err := client.PairPhone(context.Background(), data.Phone, true, whatsmeow.PairClientFirefox, "Firefox (Windows)")
			if err != nil {
				broadcastEvent("error", map[string]string{"message": err.Error()})
				return
			}
			pairingCode = code
			broadcastEvent("pairing_code", map[string]string{"code": code})
		}
	}()
}
