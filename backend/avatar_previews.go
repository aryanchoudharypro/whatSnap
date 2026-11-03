package main

import (
	"context"
	"encoding/json"
	"errors"
	"log"
	"net"
	"os"
	"strings"
	"sync"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/types"
)

const avatarPreviewRecheck = 3 * 24 * time.Hour

type avatarPreviewsData struct {
	JIDs []string `json:"jids"`
}

type avatarPreview struct {
	JID  string `json:"jid"`
	Path string `json:"path"`
}

var avatarQueue = struct {
	sync.Mutex
	pending []string
	queued  map[string]bool
	running bool
}{queued: map[string]bool{}}

func ensureAvatarPreviews() error {
	_, err := db.Exec(`CREATE TABLE IF NOT EXISTS avatar_previews (
		jid TEXT PRIMARY KEY,
		path TEXT DEFAULT '',
		picture_id TEXT DEFAULT '',
		checked_at INTEGER DEFAULT 0
	)`)
	return err
}

func cachedAvatarPreview(jid string) (path, pictureID string, checkedAt int64, ok bool) {
	err := db.QueryRow(`SELECT path, picture_id, checked_at FROM avatar_previews WHERE jid = ?`, jid).
		Scan(&path, &pictureID, &checkedAt)
	if err != nil {
		return "", "", 0, false
	}
	if path != "" {
		if _, statErr := os.Stat(path); statErr != nil {
			return "", "", 0, false
		}
	}
	return path, pictureID, checkedAt, true
}

func storeAvatarPreview(jid, path, pictureID string) {
	db.Exec(`INSERT INTO avatar_previews (jid, path, picture_id, checked_at) VALUES (?, ?, ?, ?)
		ON CONFLICT(jid) DO UPDATE SET path=excluded.path, picture_id=excluded.picture_id, checked_at=excluded.checked_at`,
		jid, path, pictureID, time.Now().Unix())
}

func handleGetAvatarPreviews(conn net.Conn, raw json.RawMessage) {
	var data avatarPreviewsData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		var known []avatarPreview
		var fetch []string
		for _, jid := range data.JIDs {
			if jid == "" || strings.HasSuffix(jid, "@broadcast") {
				continue
			}
			path, _, checkedAt, ok := cachedAvatarPreview(jid)
			if ok {
				known = append(known, avatarPreview{JID: jid, Path: path})
			}
			if !ok || time.Since(time.Unix(checkedAt, 0)) > avatarPreviewRecheck {
				fetch = append(fetch, jid)
			}
		}
		if len(known) > 0 {
			sendEvent(conn, "avatar_previews", map[string]interface{}{"items": known})
		}
		queueAvatarPreviews(fetch)
	}()
}

func queueAvatarPreviews(jids []string) {
	avatarQueue.Lock()
	defer avatarQueue.Unlock()
	for _, jid := range jids {
		if !avatarQueue.queued[jid] {
			avatarQueue.queued[jid] = true
			avatarQueue.pending = append(avatarQueue.pending, jid)
		}
	}
	if !avatarQueue.running && len(avatarQueue.pending) > 0 {
		avatarQueue.running = true
		go runAvatarQueue()
	}
}

func runAvatarQueue() {
	for {
		avatarQueue.Lock()
		n := len(avatarQueue.pending)
		if n == 0 {
			avatarQueue.running = false
			avatarQueue.Unlock()
			return
		}
		jid := avatarQueue.pending[n-1]
		avatarQueue.pending = avatarQueue.pending[:n-1]
		avatarQueue.Unlock()

		path, ok := fetchAvatarPreview(jid)

		avatarQueue.Lock()
		delete(avatarQueue.queued, jid)
		avatarQueue.Unlock()
		if ok {
			broadcastEvent("avatar_previews", map[string]interface{}{"items": []avatarPreview{{JID: jid, Path: path}}})
		}
	}
}

func fetchAvatarPreview(jid string) (path string, ok bool) {
	cli := wa()
	if cli == nil || !cli.IsConnected() {
		return "", false
	}
	parsed, err := types.ParseJID(jid)
	if err != nil {
		return "", false
	}
	if parsed.ToNonAD() == types.PSAJID || parsed.ToNonAD() == types.StatusBroadcastJID {
		// The WhatsApp service chat and the status broadcast have no profile
		// picture to ask for; the server never answers, and the queue would
		// sit on the 20 s timeout every time.
		storeAvatarPreview(jid, "", "")
		return "", true
	}
	oldPath, oldID, _, cached := cachedAvatarPreview(jid)
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()

	var info *types.ProfilePictureInfo
	if parsed.Server == types.NewsletterServer {
		meta, err := cli.GetNewsletterInfo(ctx, parsed)
		if err != nil {
			return "", false
		}
		if meta != nil && (meta.ThreadMeta.Preview.URL != "" || meta.ThreadMeta.Preview.DirectPath != "") {
			preview := meta.ThreadMeta.Preview
			info = &preview
		}
	} else {
		params := &whatsmeow.GetProfilePictureParams{Preview: true, IsCommunity: isCommunityChat(jid)}
		if cached && oldPath != "" {
			params.ExistingID = oldID
		}
		info, err = cli.GetProfilePictureInfo(ctx, parsed, params)
		switch {
		case errors.Is(err, whatsmeow.ErrProfilePictureNotSet), errors.Is(err, whatsmeow.ErrProfilePictureUnauthorized):
			storeAvatarPreview(jid, "", "")
			return "", true
		case err != nil:
			log.Printf("Couldn't get the profile picture of %s: %v", jid, err)
			return "", false
		case info == nil && cached && oldPath != "":
			storeAvatarPreview(jid, oldPath, oldID)
			return oldPath, true
		}
	}
	if info == nil {
		storeAvatarPreview(jid, "", "")
		return "", true
	}
	if cached && oldPath != "" && info.ID != "" && info.ID == oldID {
		storeAvatarPreview(jid, oldPath, oldID)
		return oldPath, true
	}
	url := info.URL
	if url == "" && info.DirectPath != "" {
		url = "https://mmg.whatsapp.net" + info.DirectPath
	}
	newPath, _, err := downloadURLToFile("avatars", "preview_"+jid, url)
	if err != nil {
		log.Printf("Couldn't download the profile picture of %s: %v", jid, err)
		return "", false
	}
	if oldPath != "" && oldPath != newPath {
		os.Remove(oldPath)
	}
	storeAvatarPreview(jid, newPath, info.ID)
	return newPath, true
}

func isCommunityChat(jid string) bool {
	var community int
	db.QueryRow(`SELECT COALESCE(is_community, 0) FROM chats WHERE jid = ?`, jid).Scan(&community)
	return community == 1
}

func forgetAvatarPreview(jid string) {
	db.Exec(`DELETE FROM avatar_previews WHERE jid = ?`, jid)
}
