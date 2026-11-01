package main

import (
	"log"
	"sync/atomic"

	"go.mau.fi/whatsmeow/proto/waE2E"
)

var historyAccessNoticeSent atomic.Bool

func handleHistorySyncStatus(notif *waE2E.HistorySyncNotification) {
	switch notif.GetSyncType() {
	case waE2E.HistorySyncType_MESSAGE_ACCESS_STATUS:
		granted := notif.GetMessageAccessStatus().GetCompleteAccessGranted()
		log.Printf("Phone's history access status: complete access granted=%v", granted)
		if !granted && historyAccessNoticeSent.CompareAndSwap(false, true) {
			broadcastEvent("history_access", map[string]interface{}{"complete": false})
		}
	case waE2E.HistorySyncType_NO_HISTORY:
		log.Printf("Phone shared no message history with this device")
		if historyAccessNoticeSent.CompareAndSwap(false, true) {
			broadcastEvent("history_access", map[string]interface{}{"complete": false, "none": true})
		}
	default:
		log.Printf("History sync notification with nothing to download: type=%s chunk=%d progress=%d",
			notif.GetSyncType(), notif.GetChunkOrder(), notif.GetProgress())
	}
}
