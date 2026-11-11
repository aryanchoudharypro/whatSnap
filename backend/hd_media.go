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

func (acc *Account) hdMediaRaw(parentID string) []byte {
	var raw []byte
	if acc.db.QueryRow(`SELECT raw FROM hd_media WHERE parent_id = ?`, parentID).Scan(&raw) != nil {
		return nil
	}
	return raw
}

// noteLiveHDMedia stores an HD copy that just arrived. The standard copy
// usually came seconds before and may already be downloaded; with HD
// downloads on, that file is swapped for the HD one.
func (acc *Account) noteLiveHDMedia(parentID string, msg *waE2E.Message) {
	storeHDMedia(acc.db, parentID, msg)
	log.Printf("HD copy arrived for %s", parentID)
	if !acc.appPref("hd_auto_download") {
		return
	}
	var jid, path string
	if acc.db.QueryRow(`SELECT COALESCE(jid, ''), COALESCE(media_path, '') FROM messages WHERE id = ?`, parentID).Scan(&jid, &path) != nil || path == "" {
		return
	}
	acc.activeDownloadsMu.Lock()
	_, busy := acc.activeDownloads[parentID]
	acc.activeDownloadsMu.Unlock()
	if busy {
		return
	}
	acc.db.Exec(`UPDATE messages SET media_path = '' WHERE id = ?`, parentID)
	os.Remove(path)
	go acc.downloadMediaNow(MessageRefData{MessageID: parentID, JID: jid}, true)
}

// albumParentID is the album header a photo or video links to through its
// MEDIA_ALBUM message association, the way WA Web groups an album. "" when
// the message carries none.
func albumParentID(msg *waE2E.Message) string {
	for m := msg; m != nil; {
		assoc := m.GetMessageContextInfo().GetMessageAssociation()
		if assoc.GetAssociationType() == waE2E.MessageAssociation_MEDIA_ALBUM {
			return assoc.GetParentMessageKey().GetID()
		}
		m = m.GetAssociatedChildMessage().GetMessage()
	}
	return ""
}
