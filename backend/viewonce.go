package main

import (
	"log"
	"os"
	"time"
)

// View-once media. whatsmeow unwraps a view-once message before whaNTV sees
// it, so the stored protobuf looks like any other photo, video or voice note;
// messages.view_once remembers what it was: 1 = not opened yet, 2 = opened.
// As in the official apps it can be opened once, and never saved, copied or
// forwarded. Once opened, the local copies are deleted.
const (
	viewOnceUnopened = 1
	viewOnceOpened   = 2
)

// viewOnceOpenedText replaces a view-once message's text once it's opened.
const viewOnceOpenedText = "[View once] Opened"

// viewOnceState is a message's view-once state (0 for an ordinary message).
func viewOnceState(messageID string) int {
	var state int
	db.QueryRow(`SELECT COALESCE(view_once, 0) FROM messages WHERE id = ?`, messageID).Scan(&state)
	return state
}

// viewOnceText labels a view-once message's text so the chat says so.
func viewOnceText(text string) string {
	return "[View once] " + text
}

// markViewOnce records that a stored message arrived as view-once. A copy
// delivered again later doesn't make an opened one openable again.
func markViewOnce(ex sqlExecer, messageID string) {
	if _, err := ex.Exec(`UPDATE messages SET view_once = ? WHERE id = ? AND COALESCE(view_once, 0) = 0`,
		viewOnceUnopened, messageID); err != nil {
		log.Println("Error marking view-once message:", err)
	}
}

// consumeViewOnce marks a view-once message opened and schedules its local
// copies for deletion: the cached media now, and the scratch copy handed to
// the viewer (tempPath, may be "") once the viewer has let go of it.
func consumeViewOnce(messageID, cachedPath, tempPath string) {
	var jid string
	db.QueryRow(`SELECT COALESCE(jid, '') FROM messages WHERE id = ?`, messageID).Scan(&jid)
	// raw goes too: it holds the media key, the one thing that could fetch
	// the media again.
	db.Exec(`UPDATE messages SET view_once = ?, text = ?, media_path = '', raw = NULL WHERE id = ?`,
		viewOnceOpened, viewOnceOpenedText, messageID)
	removeCachedMedia(cachedPath)
	broadcastEvent("message_edited", map[string]interface{}{
		"jid": jid, "message_id": messageID, "text": viewOnceOpenedText, "not_edit": true,
	})
	if tempPath == "" {
		return
	}
	// A viewer or player keeps the file open while it's showing it, and
	// Windows won't delete an open file; keep trying for a while.
	go func() {
		for range 30 {
			time.Sleep(time.Minute)
			if err := os.Remove(tempPath); err == nil || os.IsNotExist(err) {
				return
			}
		}
	}()
}

// viewOnceBlocked is the error for saving, copying or forwarding view-once
// media, or "" if the message isn't view-once.
func viewOnceBlocked(messageID string) string {
	switch viewOnceState(messageID) {
	case viewOnceUnopened:
		return "View once media can't be saved, copied or forwarded."
	case viewOnceOpened:
		return "You've already opened this view once message."
	}
	return ""
}
