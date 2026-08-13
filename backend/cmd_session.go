package main

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"os"
	"sync"

	"net"
	"go.mau.fi/whatsmeow"
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

		if client != nil && client.IsConnected() {
			client.Disconnect()
		}

		if db != nil {
			// Disconnect returns before every handler goroutine has released
			// its connection, and SQLite only drops the -wal/-shm files once
			// the last one closes. Checkpointing explicitly folds the WAL back
			// into the main database even if a straggler keeps a handle.
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
				qrChan, qrErr := client.GetQRChannel(context.Background())
				if qrErr != nil {
					broadcastEvent("error", map[string]string{"message": qrErr.Error()})
					return
				}
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
