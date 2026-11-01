package main

import (
	"log"
	"os"

	"google.golang.org/protobuf/proto"

	"go.mau.fi/whatsmeow/proto/waE2E"
)

// HD photos and videos: the sender's phone sends the standard copy as an
// ordinary message, then uploads the HD copy and sends it as a second
// message (an associatedChildMessage whose messageAssociation is an
// *_DUAL_UPLOAD pointing at the first). WhatsApp shows them as one message
// with an HD badge, so whatSnap keeps the HD copy beside its parent rather
// than as a second photo in the chat. Which one gets downloaded follows the
// "auto-download quality" setting (hd_auto_download, synced with WhatsApp
// Web as mediaAutoDownloadQuality).

func hdParentID(msg *waE2E.Message) string {
	for m := msg; m != nil; {
		switch m.GetMessageContextInfo().GetMessageAssociation().GetAssociationType() {
		case waE2E.MessageAssociation_HD_IMAGE_DUAL_UPLOAD,
			waE2E.MessageAssociation_HD_VIDEO_DUAL_UPLOAD,
			waE2E.MessageAssociation_STREAMED_HD_VIDEO_DUAL_UPLOAD:
			return m.GetMessageContextInfo().GetMessageAssociation().GetParentMessageKey().GetID()
		}
		next := m.GetAssociatedChildMessage().GetMessage()
		if next == nil {
			return ""
		}
		m = next
	}
	return ""
}

func storeHDMedia(tx sqlExecer, parentID string, msg *waE2E.Message) {
	raw, err := proto.Marshal(msg)
	if err != nil {
		return
	}
	tx.Exec(`INSERT OR REPLACE INTO hd_media (parent_id, raw) VALUES (?, ?)`, parentID, raw)
}

func hdMediaRaw(parentID string) []byte {
	var raw []byte
	if db.QueryRow(`SELECT raw FROM hd_media WHERE parent_id = ?`, parentID).Scan(&raw) != nil {
		return nil
	}
	return raw
}

// noteLiveHDMedia stores an HD copy that just arrived. The standard copy
// usually came seconds before and may already be downloaded; with HD
// downloads on, that file is swapped for the HD one.
func noteLiveHDMedia(parentID string, msg *waE2E.Message) {
	storeHDMedia(db, parentID, msg)
	log.Printf("HD copy arrived for %s", parentID)
	if !appPref("hd_auto_download") {
		return
	}
	var jid, path string
	if db.QueryRow(`SELECT COALESCE(jid, ''), COALESCE(media_path, '') FROM messages WHERE id = ?`, parentID).Scan(&jid, &path) != nil || path == "" {
		return
	}
	activeDownloadsMu.Lock()
	_, busy := activeDownloads[parentID]
	activeDownloadsMu.Unlock()
	if busy {
		return
	}
	db.Exec(`UPDATE messages SET media_path = '' WHERE id = ?`, parentID)
	os.Remove(path)
	go downloadMediaNow(MessageRefData{MessageID: parentID, JID: jid}, true)
}
