package main

import (
	"log"
	"os"
	"time"
)

// View-once media. whatsmeow unwraps a view-once message before whatSnap sees
// it, so the stored protobuf looks like any other photo, video or voice note;
// messages.view_once remembers what it was: 1 = not opened yet, 2 = opened.
// As in the official apps it can be opened once, and never saved, copied or
// forwarded. Once opened, the local copies are deleted.
const (
	viewOnceUnopened = 1
	viewOnceOpened   = 2
)

const viewOnceOpenedText = "[View once] Opened"

func (acc *Account) viewOnceState(messageID string) int {
	var state int
	acc.db.QueryRow(`SELECT COALESCE(view_once, 0) FROM messages WHERE id = ?`, messageID).Scan(&state)
	return state
}

func viewOnceText(text string) string {
	return "[View once] " + text
}

func markViewOnce(ex sqlExecer, messageID string) {
	if _, err := ex.Exec(`UPDATE messages SET view_once = ? WHERE id = ? AND COALESCE(view_once, 0) = 0`,
		viewOnceUnopened, messageID); err != nil {
		log.Println("Error marking view-once message:", err)
	}
}

func (acc *Account) consumeViewOnce(messageID, cachedPath, tempPath string) {
	var jid string
	acc.db.QueryRow(`SELECT COALESCE(jid, '') FROM messages WHERE id = ?`, messageID).Scan(&jid)
	acc.db.Exec(`UPDATE messages SET view_once = ?, text = ?, media_path = '', raw = NULL WHERE id = ?`,
		viewOnceOpened, viewOnceOpenedText, messageID)
	acc.removeCachedMedia(cachedPath)
	acc.broadcastEvent("message_edited", map[string]interface{}{
		"jid": jid, "message_id": messageID, "text": viewOnceOpenedText, "not_edit": true,
	})
	if tempPath == "" {
		return
	}
	go func() {
		for range 30 {
			time.Sleep(time.Minute)
			if err := os.Remove(tempPath); err == nil || os.IsNotExist(err) {
				return
			}
		}
	}()
}

func (acc *Account) viewOnceBlocked(messageID string) string {
	switch acc.viewOnceState(messageID) {
	case viewOnceUnopened:
		return "View once media can't be saved, copied or forwarded."
	case viewOnceOpened:
		return "You've already opened this view once message."
	}
	return ""
}
