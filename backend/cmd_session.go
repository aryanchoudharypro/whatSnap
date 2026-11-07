package main

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"os"
	"strings"
	"sync"
	"time"

	"net"

	"go.mau.fi/whatsmeow"
)

var shutdownOnce sync.Once

// shutdownBackend closes every account's connection and database, then
// exits.
func shutdownBackend() {
	shutdownOnce.Do(func() {
		log.Println("Shutdown requested")
		var wg sync.WaitGroup
		for _, acc := range openAccounts() {
			wg.Add(1)
			go func() {
				defer wg.Done()
				acc.close()
			}()
		}
		wg.Wait()
		log.Println("Shutdown complete")
		os.Exit(0)
	})
}

func (acc *Account) handleGetQR(conn net.Conn) {
	go func() {
		if acc.wa().Store.ID == nil {
			// Cancelling (Back to phone number login) ends this attempt, so
			// the channel closing then isn't reported as an expired code.
			ctx := acc.beginPairingAttempt()
			if acc.wa() != nil && acc.wa().IsConnected() {
				acc.wa().Disconnect()
			}
			qrChan, err := acc.wa().GetQRChannel(ctx)
			if err != nil {
				acc.sendEvent(conn, "error", map[string]string{"message": err.Error()})
				return
			}
			err = acc.wa().Connect()
			if err != nil {
				acc.sendEvent(conn, "error", map[string]string{"message": err.Error()})
				return
			}
			// whatsmeow shows a new code by itself as each one expires (60 s,
			// then 20 s each). Once they run out, WhatsApp Web shows a reload
			// button instead of asking for codes forever; so do we.
			for qr := range qrChan {
				if qr.Event == "code" {
					acc.sendEvent(conn, "qr", map[string]string{"code": qr.Code})
				}
			}
			if ctx.Err() == nil && (acc.wa() == nil || acc.wa().Store.ID == nil) {
				acc.sendEvent(conn, "qr_timeout", nil)
			}
		}
	}()
}

func (acc *Account) handleCancelPairing() {
	fmt.Println("Received cancel request")
	acc.endPairingAttempt()
	if acc.wa().Store.ID == nil {
		acc.wa().Disconnect()
	}
}

func (acc *Account) handleLogout() {
	go func() {
		if acc.wa() == nil || acc.wa().Store.ID == nil {
			acc.broadcastEvent("needs_login", nil)
			return
		}
		if err := acc.wa().Logout(context.Background()); err != nil {
			acc.broadcastEvent("error", map[string]string{"message": "Logout failed: " + err.Error()})
			return
		}

		acc.replaceWithFreshClient()
		acc.broadcastEvent("needs_login", nil)
	}()
}

// maxPairingCodes is how many codes one pairing request may go through
// before it gives up: the first one plus two replacements for codes that
// expired unused (about eight minutes in all). The server throttles code
// requests per phone number, so this must not run on forever.
const maxPairingCodes = 3

func (acc *Account) beginPairingAttempt() context.Context {
	ctx, cancel := context.WithCancel(context.Background())
	acc.pairingAttempt.Lock()
	if acc.pairingAttempt.cancel != nil {
		acc.pairingAttempt.cancel()
	}
	acc.pairingAttempt.cancel = cancel
	acc.pairingAttempt.Unlock()
	return ctx
}

func (acc *Account) endPairingAttempt() {
	acc.pairingAttempt.Lock()
	if acc.pairingAttempt.cancel != nil {
		acc.pairingAttempt.cancel()
		acc.pairingAttempt.cancel = nil
	}
	acc.pairingAttempt.Unlock()
}

func (acc *Account) handlePair(raw json.RawMessage) {
	var data PairCommandData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		if acc.wa().Store.ID != nil {
			return
		}
		ctx := acc.beginPairingAttempt()
		for range maxPairingCodes {
			if !acc.runPairingCode(ctx, data.Phone) {
				return
			}
		}
		if ctx.Err() == nil {
			acc.broadcastEvent("error", map[string]string{"message": "The pairing code expired. Ask for a new one to try again."})
		}
	}()
}

func (acc *Account) runPairingCode(ctx context.Context, phone string) (expired bool) {
	c := acc.wa()
	if c.IsConnected() {
		c.Disconnect()
	}
	qrChan, err := c.GetQRChannel(ctx)
	if err != nil {
		acc.broadcastEvent("error", map[string]string{"message": err.Error()})
		return false
	}
	if err = c.Connect(); err != nil {
		acc.broadcastEvent("error", map[string]string{"message": err.Error()})
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
			acc.broadcastEvent("error", map[string]string{"message": "Could not start pairing: " + first.Event})
		}
		return false
	}
	code, err := c.PairPhone(ctx, phone, true, whatsmeow.PairClientUWP, "Chrome (Windows)")
	if err != nil {
		if ctx.Err() == nil {
			acc.broadcastEvent("error", map[string]string{"message": err.Error()})
		}
		return false
	}
	if ctx.Err() != nil {
		return false
	}
	// whatsmeow gives "ABCD-EFGH"; the phone takes the 8 characters alone,
	// and a screen reader would otherwise read the dash out.
	code = strings.ReplaceAll(code, "-", "")
	acc.pairingCode = code
	acc.broadcastEvent("pairing_code", map[string]string{"code": code})
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
				acc.broadcastEvent("error", map[string]string{"message": "Pairing failed: " + item.Error.Error()})
				return false
			}
			log.Printf("Pairing: %s", item.Event)
		}
	}
}
