package main

import (
	"context"
	"encoding/json"
	"errors"
	"log"
	"net"
	"os"
	"strings"
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

func (acc *Account) ensureAvatarPreviews() error {
	_, err := acc.db.Exec(`CREATE TABLE IF NOT EXISTS avatar_previews (
		jid TEXT PRIMARY KEY,
		path TEXT DEFAULT '',
		picture_id TEXT DEFAULT '',
		checked_at INTEGER DEFAULT 0
	)`)
	return err
}

func (acc *Account) cachedAvatarPreview(jid string) (path, pictureID string, checkedAt int64, ok bool) {
	err := acc.db.QueryRow(`SELECT path, picture_id, checked_at FROM avatar_previews WHERE jid = ?`, jid).
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

func (acc *Account) storeAvatarPreview(jid, path, pictureID string) {
	acc.db.Exec(`INSERT INTO avatar_previews (jid, path, picture_id, checked_at) VALUES (?, ?, ?, ?)
		ON CONFLICT(jid) DO UPDATE SET path=excluded.path, picture_id=excluded.picture_id, checked_at=excluded.checked_at`,
		jid, path, pictureID, time.Now().Unix())
}

func (acc *Account) handleGetAvatarPreviews(conn net.Conn, raw json.RawMessage) {
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
			path, _, checkedAt, ok := acc.cachedAvatarPreview(jid)
			if ok {
				known = append(known, avatarPreview{JID: jid, Path: path})
			}
			if !ok || time.Since(time.Unix(checkedAt, 0)) > avatarPreviewRecheck {
				fetch = append(fetch, jid)
			}
		}
		if len(known) > 0 {
			acc.sendEvent(conn, "avatar_previews", map[string]interface{}{"items": known})
		}
		acc.queueAvatarPreviews(fetch)
	}()
}

func (acc *Account) queueAvatarPreviews(jids []string) {
	acc.avatarQueue.Lock()
	defer acc.avatarQueue.Unlock()
	for _, jid := range jids {
		if !acc.avatarQueue.queued[jid] {
			acc.avatarQueue.queued[jid] = true
			acc.avatarQueue.pending = append(acc.avatarQueue.pending, jid)
		}
	}
	if !acc.avatarQueue.running && len(acc.avatarQueue.pending) > 0 {
		acc.avatarQueue.running = true
		go acc.runAvatarQueue()
	}
}

func (acc *Account) runAvatarQueue() {
	for {
		acc.avatarQueue.Lock()
		n := len(acc.avatarQueue.pending)
		if n == 0 {
			acc.avatarQueue.running = false
			acc.avatarQueue.Unlock()
			return
		}
		jid := acc.avatarQueue.pending[n-1]
		acc.avatarQueue.pending = acc.avatarQueue.pending[:n-1]
		acc.avatarQueue.Unlock()

		path, ok := acc.fetchAvatarPreview(jid)

		acc.avatarQueue.Lock()
		delete(acc.avatarQueue.queued, jid)
		acc.avatarQueue.Unlock()
		if ok {
			acc.broadcastEvent("avatar_previews", map[string]interface{}{"items": []avatarPreview{{JID: jid, Path: path}}})
		}
	}
}

func (acc *Account) fetchAvatarPreview(jid string) (path string, ok bool) {
	cli := acc.wa()
	if cli == nil || !cli.IsConnected() {
		return "", false
	}
	parsed, err := types.ParseJID(jid)
	if err != nil {
		return "", false
	}
	if jid == "me" && cli.Store.ID != nil {
		parsed = cli.Store.ID.ToNonAD()
	}
	if parsed.User == "" {
		// A bare word like "me" parses as a server name; asking for it sends
		// an iq the server never answers, which trips the dead-socket timer.
		acc.storeAvatarPreview(jid, "", "")
		return "", true
	}
	if parsed.ToNonAD() == types.PSAJID || parsed.ToNonAD() == types.StatusBroadcastJID {
		// The WhatsApp service chat and the status broadcast have no profile
		// picture to ask for; the server never answers, and the queue would
		// sit on the 20 s timeout every time.
		acc.storeAvatarPreview(jid, "", "")
		return "", true
	}
	oldPath, oldID, _, cached := acc.cachedAvatarPreview(jid)
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
		params := &whatsmeow.GetProfilePictureParams{Preview: true, IsCommunity: acc.isCommunityChat(jid)}
		if cached && oldPath != "" {
			params.ExistingID = oldID
		}
		info, err = cli.GetProfilePictureInfo(ctx, parsed, params)
		switch {
		case errors.Is(err, whatsmeow.ErrProfilePictureNotSet), errors.Is(err, whatsmeow.ErrProfilePictureUnauthorized):
			acc.storeAvatarPreview(jid, "", "")
			return "", true
		case err != nil:
			log.Printf("Couldn't get the profile picture of %s: %v", jid, err)
			return "", false
		case info == nil && cached && oldPath != "":
			acc.storeAvatarPreview(jid, oldPath, oldID)
			return oldPath, true
		}
	}
	if info == nil {
		acc.storeAvatarPreview(jid, "", "")
		return "", true
	}
	if cached && oldPath != "" && info.ID != "" && info.ID == oldID {
		acc.storeAvatarPreview(jid, oldPath, oldID)
		return oldPath, true
	}
	url := info.URL
	if url == "" && info.DirectPath != "" {
		url = "https://mmg.whatsapp.net" + info.DirectPath
	}
	newPath, _, err := acc.downloadURLToFile("avatars", "preview_"+jid, url)
	if err != nil {
		log.Printf("Couldn't download the profile picture of %s: %v", jid, err)
		return "", false
	}
	if oldPath != "" && oldPath != newPath {
		os.Remove(oldPath)
	}
	acc.storeAvatarPreview(jid, newPath, info.ID)
	return newPath, true
}

func (acc *Account) isCommunityChat(jid string) bool {
	var community int
	acc.db.QueryRow(`SELECT COALESCE(is_community, 0) FROM chats WHERE jid = ?`, jid).Scan(&community)
	return community == 1
}

func (acc *Account) forgetAvatarPreview(jid string) {
	acc.db.Exec(`DELETE FROM avatar_previews WHERE jid = ?`, jid)
}
