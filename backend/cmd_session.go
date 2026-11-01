package main

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"os"
	"sync"
	"time"

	"go.mau.fi/whatsmeow"
	"net"
)

var shutdownOnce sync.Once

// handleShutdown closes the WhatsApp connection and the database before
// exiting. Without it the process is terminated abruptly, which leaves the
// SQLite -wal/-shm files behind (recovered on next open, but never
// checkpointed away) and drops the WhatsApp socket without a clean
// companion-device disconnect.
func handleShutdown() {
	shutdownOnce.Do(func() {
		log.Println("Shutdown requested")

		if wa() != nil && wa().IsConnected() {
			wa().Disconnect()
		}

		if db != nil {
			if _, err := db.Exec("PRAGMA wal_checkpoint(TRUNCATE)"); err != nil {
				log.Println("WAL checkpoint failed:", err)
			}
			if err := db.Close(); err != nil {
				log.Println("Error closing database:", err)
			}
		}

		log.Println("Shutdown complete")
		os.Exit(0)
	})
}

func handleGetQR(conn net.Conn) {
	go func() {
		if wa().Store.ID == nil {
			endPairingAttempt()
			if wa() != nil && wa().IsConnected() {
				wa().Disconnect()
			}
			qrChan, err := wa().GetQRChannel(context.Background())
			if err != nil {
				sendEvent(conn, "error", map[string]string{"message": err.Error()})
				return
			}
			err = wa().Connect()
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
	endPairingAttempt()
	if wa().Store.ID == nil {
		wa().Disconnect()
	}
}

func handleLogout() {
	go func() {
		if wa() == nil || wa().Store.ID == nil {
			broadcastEvent("needs_login", nil)
			return
		}
		if err := wa().Logout(context.Background()); err != nil {
			broadcastEvent("error", map[string]string{"message": "Logout failed: " + err.Error()})
			return
		}

		replaceWithFreshClient()
		broadcastEvent("needs_login", nil)
	}()
}

// maxPairingCodes is how many codes one pairing request may go through
// before it gives up: the first one plus two replacements for codes that
// expired unused (about eight minutes in all). The server throttles code
// requests per phone number, so this must not run on forever.
const maxPairingCodes = 3

var pairingAttempt struct {
	sync.Mutex
	cancel context.CancelFunc
}

func beginPairingAttempt() context.Context {
	ctx, cancel := context.WithCancel(context.Background())
	pairingAttempt.Lock()
	if pairingAttempt.cancel != nil {
		pairingAttempt.cancel()
	}
	pairingAttempt.cancel = cancel
	pairingAttempt.Unlock()
	return ctx
}

func endPairingAttempt() {
	pairingAttempt.Lock()
	if pairingAttempt.cancel != nil {
		pairingAttempt.cancel()
		pairingAttempt.cancel = nil
	}
	pairingAttempt.Unlock()
}

func handlePair(raw json.RawMessage) {
	var data PairCommandData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		if wa().Store.ID != nil {
			return
		}
		ctx := beginPairingAttempt()
		for range maxPairingCodes {
			if !runPairingCode(ctx, data.Phone) {
				return
			}
		}
		if ctx.Err() == nil {
			broadcastEvent("error", map[string]string{"message": "The pairing code expired. Ask for a new one to try again."})
		}
	}()
}

func runPairingCode(ctx context.Context, phone string) (expired bool) {
	c := wa()
	if c.IsConnected() {
		c.Disconnect()
	}
	qrChan, err := c.GetQRChannel(ctx)
	if err != nil {
		broadcastEvent("error", map[string]string{"message": err.Error()})
		return false
	}
	if err = c.Connect(); err != nil {
		broadcastEvent("error", map[string]string{"message": err.Error()})
		return false
	}
	var first whatsmeow.QRChannelItem
	select {
	case first = <-qrChan:
	case <-ctx.Done():
		return false
	}
	if first.Event != whatsmeow.QRChannelEventCode {
		if ctx.Err() == nil {
			broadcastEvent("error", map[string]string{"message": "Could not start pairing: " + first.Event})
		}
		return false
	}
	code, err := c.PairPhone(ctx, phone, true, whatsmeow.PairClientUWP, "Chrome (Windows)")
	if err != nil {
		if ctx.Err() == nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
		}
		return false
	}
	if ctx.Err() != nil {
		return false
	}
	pairingCode = code
	broadcastEvent("pairing_code", map[string]string{"code": code})
	for {
		var item whatsmeow.QRChannelItem
		var ok bool
		select {
		case item, ok = <-qrChan:
		case <-ctx.Done():
			return false
		}
		if !ok {
			return false
		}
		switch item.Event {
		case whatsmeow.QRChannelEventCode:
		case whatsmeow.QRChannelTimeout.Event:
			log.Println("Pairing code expired unused")
			for i := 0; i < 50 && c.IsConnected(); i++ {
				time.Sleep(100 * time.Millisecond)
			}
			return ctx.Err() == nil
		case whatsmeow.QRChannelSuccess.Event:
			return false
		default:
			if item.Error != nil {
				log.Printf("Pairing failed (%s): %v", item.Event, item.Error)
				broadcastEvent("error", map[string]string{"message": "Pairing failed: " + item.Error.Error()})
				return false
			}
			log.Printf("Pairing: %s", item.Event)
		}
	}
}
