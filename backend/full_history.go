package main

import (
	"context"
	"log"
	"sync"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/types/events"
)

// Asking the phone for its full history again without relinking
// (FULL_HISTORY_SYNC_ON_DEMAND; see the fork's historysync_retry.go). WA Web
// never sends it, and its answer codes (ERROR_REQUEST_ON_NON_SMB_PRIMARY and
// the hosted-device ones) suggest phones only honour it for WhatsApp
// Business, so the UI says plainly what the phone answered.
const fullHistoryAnswerTimeout = 60 * time.Second

var fullHistoryRequest struct {
	sync.Mutex
	id    string
	timer *time.Timer
}

func handleRequestFullHistory() {
	if wa() == nil || !isOnline(wa()) {
		broadcastEvent("full_history_result", map[string]interface{}{"code": "OFFLINE"})
		return
	}
	id := string(wa().GenerateMessageID())
	// Ask for as much as at linking (store.DeviceProps' history sync config:
	// 3650 days, 2 GB).
	msg := whatsmeow.BuildFullHistorySyncOnDemandRequest(id, time.Time{}, 3650, nil)
	fullHistoryRequest.Lock()
	if fullHistoryRequest.timer != nil {
		fullHistoryRequest.timer.Stop()
	}
	fullHistoryRequest.id = id
	fullHistoryRequest.timer = time.AfterFunc(fullHistoryAnswerTimeout, func() {
		fullHistoryRequest.Lock()
		defer fullHistoryRequest.Unlock()
		if fullHistoryRequest.id != id {
			return
		}
		fullHistoryRequest.id = ""
		log.Printf("Full history request %s: no answer from the phone after %s", id, fullHistoryAnswerTimeout)
		broadcastEvent("full_history_result", map[string]interface{}{"code": "NO_ANSWER"})
	})
	fullHistoryRequest.Unlock()
	if _, err := wa().SendPeerMessage(context.Background(), msg); err != nil {
		log.Printf("Full history request %s failed to send: %v", id, err)
		fullHistoryRequest.Lock()
		fullHistoryRequest.timer.Stop()
		fullHistoryRequest.id = ""
		fullHistoryRequest.Unlock()
		broadcastEvent("full_history_result", map[string]interface{}{"code": "SEND_FAILED"})
		return
	}
	log.Printf("Asked the phone for full history on demand (request %s)", id)
}

func handleFullHistoryResult(v *events.FullHistorySyncOnDemandResult) {
	fullHistoryRequest.Lock()
	// An answer without the request's metadata echoed back still answers the
	// one request in flight.
	ours := fullHistoryRequest.id != "" && (v.RequestID == "" || v.RequestID == fullHistoryRequest.id)
	if ours {
		fullHistoryRequest.id = ""
		fullHistoryRequest.timer.Stop()
	}
	fullHistoryRequest.Unlock()
	log.Printf("Phone answered full history request %s: %s (ours %v)", v.RequestID, v.Code, ours)
	if !ours {
		return
	}
	broadcastEvent("full_history_result", map[string]interface{}{"code": v.Code.String()})
}

func logChunkRetryResult(v *events.HistorySyncChunkRetryResult) {
	log.Printf("Phone answered history sync chunk retry for %s chunk %d: code %s (sent %v), can recover %v",
		v.SyncType, v.ChunkOrder, v.Code, v.HasCode, v.CanRecover)
}
