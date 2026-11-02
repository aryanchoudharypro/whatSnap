package main

import (
	"log"
	"sync"
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

var recentSync struct {
	sync.Mutex
	active  bool
	percent int // -1 until the first RECENT chunk is done
	paused  bool
	timer   *time.Timer
}

func broadcastRecentSyncLocked() {
	broadcastEvent("history_sync_progress", map[string]interface{}{
		"active":  recentSync.active,
		"percent": recentSync.percent,
		"paused":  recentSync.paused,
	})
}

// noteHistoryChunkProgress is called once a history sync chunk has been
// written.
func noteHistoryChunkProgress(syncType waHistorySync.HistorySync_HistorySyncType, chunkOrder uint32, progress uint32) {
	recentSync.Lock()
	defer recentSync.Unlock()
	switch syncType {
	case waHistorySync.HistorySync_INITIAL_BOOTSTRAP:
		if !recentSync.active {
			recentSync.active = true
			recentSync.percent = -1
		}
	case waHistorySync.HistorySync_RECENT:
		recentSync.active = true
		pct := int(min(progress, 100))
		if pct > recentSync.percent {
			recentSync.percent = pct
		}
	case waHistorySync.HistorySync_FULL:
		if !recentSync.active {
			return
		}
		recentSync.percent = 100
	default:
		return
	}
	log.Printf("Recent history sync progress: type=%s chunk=%d progress=%d shown=%d%%", syncType, chunkOrder, progress, recentSync.percent)
	recentSync.paused = false
	if recentSync.timer != nil {
		recentSync.timer.Stop()
		recentSync.timer = nil
	}
	if recentSync.percent >= 100 {
		recentSync.active = false
		broadcastRecentSyncLocked()
		return
	}
	recentSync.timer = time.AfterFunc(recentSyncPauseAfter, recentSyncStalled)
	broadcastRecentSyncLocked()
}

func recentSyncStalled() {
	recentSync.Lock()
	defer recentSync.Unlock()
	if !recentSync.active {
		return
	}
	if !recentSync.paused {
		log.Printf("Recent history sync paused: no chunk for %s at %d%%", recentSyncPauseAfter, recentSync.percent)
		recentSync.paused = true
		recentSync.timer = time.AfterFunc(recentSyncPausedFor, recentSyncStalled)
	} else {
		log.Printf("Recent history sync paused for %s; no longer showing it", recentSyncPausedFor)
		recentSync.active = false
		recentSync.paused = false
		recentSync.timer = nil
	}
	broadcastRecentSyncLocked()
}

func resetRecentSyncProgress() {
	recentSync.Lock()
	defer recentSync.Unlock()
	if recentSync.timer != nil {
		recentSync.timer.Stop()
		recentSync.timer = nil
	}
	recentSync.active = false
	recentSync.paused = false
	recentSync.percent = -1
}

// The backlog the server kept while whatSnap was closed or offline, shown the
// way WA Web's loading screen does ("Downloading messages: N%"), from the
// offline_preview count and the stanzas received since.
func handleOfflineSyncProgress(evt interface{}) {
	switch v := evt.(type) {
	case *events.OfflineSyncPreview:
		if v.Total > 0 {
			broadcastEvent("offline_sync", map[string]interface{}{"active": true, "percent": 0, "total": v.Total})
		}
	case *events.OfflineSyncProgress:
		broadcastEvent("offline_sync", map[string]interface{}{"active": v.Percent < 100, "percent": v.Percent, "total": v.Total})
	case *events.OfflineSyncCompleted:
		broadcastEvent("offline_sync", map[string]interface{}{"active": false, "percent": 100})
	}
}
