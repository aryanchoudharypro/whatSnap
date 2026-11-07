package main

import (
	"log"
	"time"

	"go.mau.fi/whatsmeow/proto/waHistorySync"
	"go.mau.fi/whatsmeow/types/events"
)

// Progress of the phone's "recent" history sync after linking, worked out the
// way WA Web does for "Syncing older messages / N% complete" in its You tab
// (WAWebHistorySyncProgress, WAWebGetHistorySyncProgress):
//
//   - Every RECENT chunk carries the phone's own progress (0-100) next to its
//     chunk order. WA Web estimates the chunk total as
//     ceil(chunkOrder / (progress/100)) and shows lastDoneChunk/total, which is
//     that same progress value, so the chunk's progress field is shown as is.
//   - The RECENT chunk at progress 100 (or any FULL chunk) ends it.
//   - With no new chunk for 120s while it's unfinished, it shows "Syncing
//     paused" (the phone stopped sending, usually because WhatsApp on it was
//     closed). WA Web drops the paused notice after 20 minutes.
//
// FULL (older) history after that has no progress shown, as in WA Web.
const (
	recentSyncPauseAfter = 120 * time.Second
	recentSyncPausedFor  = 20 * time.Minute
)

// -1 until the first RECENT chunk is done

func (acc *Account) broadcastRecentSyncLocked() {
	acc.broadcastEvent("history_sync_progress", map[string]interface{}{
		"active":  acc.recentSync.active,
		"percent": acc.recentSync.percent,
		"paused":  acc.recentSync.paused,
	})
}

// noteHistoryChunkProgress is called once a history sync chunk has been
// written.
func (acc *Account) noteHistoryChunkProgress(syncType waHistorySync.HistorySync_HistorySyncType, chunkOrder uint32, progress uint32) {
	acc.recentSync.Lock()
	defer acc.recentSync.Unlock()
	switch syncType {
	case waHistorySync.HistorySync_INITIAL_BOOTSTRAP:
		if !acc.recentSync.active {
			acc.recentSync.active = true
			acc.recentSync.percent = -1
		}
	case waHistorySync.HistorySync_RECENT:
		acc.recentSync.active = true
		pct := int(min(progress, 100))
		if pct > acc.recentSync.percent {
			acc.recentSync.percent = pct
		}
	case waHistorySync.HistorySync_FULL:
		if !acc.recentSync.active {
			return
		}
		acc.recentSync.percent = 100
	default:
		return
	}
	log.Printf("Recent history sync progress: type=%s chunk=%d progress=%d shown=%d%%", syncType, chunkOrder, progress, acc.recentSync.percent)
	acc.recentSync.paused = false
	if acc.recentSync.timer != nil {
		acc.recentSync.timer.Stop()
		acc.recentSync.timer = nil
	}
	if acc.recentSync.percent >= 100 {
		acc.recentSync.active = false
		acc.broadcastRecentSyncLocked()
		return
	}
	acc.recentSync.timer = time.AfterFunc(recentSyncPauseAfter, acc.recentSyncStalled)
	acc.broadcastRecentSyncLocked()
}

func (acc *Account) recentSyncStalled() {
	acc.recentSync.Lock()
	defer acc.recentSync.Unlock()
	if !acc.recentSync.active {
		return
	}
	if !acc.recentSync.paused {
		log.Printf("Recent history sync paused: no chunk for %s at %d%%", recentSyncPauseAfter, acc.recentSync.percent)
		acc.recentSync.paused = true
		acc.recentSync.timer = time.AfterFunc(recentSyncPausedFor, acc.recentSyncStalled)
	} else {
		log.Printf("Recent history sync paused for %s; no longer showing it", recentSyncPausedFor)
		acc.recentSync.active = false
		acc.recentSync.paused = false
		acc.recentSync.timer = nil
	}
	acc.broadcastRecentSyncLocked()
}

func (acc *Account) resetRecentSyncProgress() {
	acc.recentSync.Lock()
	defer acc.recentSync.Unlock()
	if acc.recentSync.timer != nil {
		acc.recentSync.timer.Stop()
		acc.recentSync.timer = nil
	}
	acc.recentSync.active = false
	acc.recentSync.paused = false
	acc.recentSync.percent = -1
}

// The backlog the server kept while whatSnap was closed or offline, shown the
// way WA Web's loading screen does ("Downloading messages: N%"), from the
// offline_preview count and the stanzas received since.
func (acc *Account) handleOfflineSyncProgress(evt interface{}) {
	switch v := evt.(type) {
	case *events.OfflineSyncPreview:
		if v.Total > 0 {
			acc.broadcastEvent("offline_sync", map[string]interface{}{"active": true, "percent": 0, "total": v.Total})
		}
	case *events.OfflineSyncProgress:
		acc.broadcastEvent("offline_sync", map[string]interface{}{"active": v.Percent < 100, "percent": v.Percent, "total": v.Total})
	case *events.OfflineSyncCompleted:
		acc.broadcastEvent("offline_sync", map[string]interface{}{"active": false, "percent": 100})
	}
}
