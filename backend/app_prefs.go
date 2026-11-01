package main

import (
	"context"
	"encoding/json"
	"log"
	"net"
	"sync"

	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
)

var appPrefDefaults = map[string]bool{
	"auto_download_photos":    true,
	"auto_download_audio":     true,
	"auto_download_videos":    false,
	"auto_download_documents": false,
	"hd_media":                false,
	"security_notifications":  true,
}

func appPref(name string) bool {
	var v int
	if db.QueryRow(`SELECT CAST(value AS INTEGER) FROM app_settings WHERE key = ?`, "pref:"+name).Scan(&v) != nil {
		return appPrefDefaults[name]
	}
	return v == 1
}

func appPrefsPayload() map[string]interface{} {
	out := map[string]interface{}{"link_previews_disabled": linkPreviewsDisabled()}
	for name := range appPrefDefaults {
		out[name] = appPref(name)
	}
	return out
}

func handleGetAppPrefs(conn net.Conn) {
	sendEvent(conn, "app_prefs", appPrefsPayload())
}

func handleSetAppPrefs(raw json.RawMessage) {
	var data map[string]bool
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	changed := map[string]bool{}
	for name, v := range data {
		if _, ok := appPrefDefaults[name]; ok {
			if appPref(name) != v {
				changed[name] = v
			}
			db.Exec(`INSERT OR REPLACE INTO app_settings (key, value) VALUES (?, ?)`, "pref:"+name, boolToInt(v))
		}
	}
	syncAppPrefs(changed)
	off, ok := data["link_previews_disabled"]
	if !ok || off == linkPreviewsDisabled() {
		return
	}
	go func() {
		if err := sendAppStateWithRetry(context.Background(), appstate.BuildDisableLinkPreviews(off)); err != nil {
			log.Printf("Turning link previews %v failed: %v", map[bool]string{true: "off", false: "on"}[off], err)
			broadcastEvent("error", map[string]string{"message": "Couldn't change link previews: " + err.Error()})
			return
		}
		db.Exec(`INSERT OR REPLACE INTO app_settings (key, value) VALUES (?, ?)`, linkPreviewsOffKey, boolToInt(off))
	}()
}

const (
	autoDownloadMaxSize   = 32 << 20
	autoDownloadAudioMax  = 512 << 10
	autoDownloadGIFMax    = 500 << 10
	autoDownloadParallel  = 5
	autoDownloadQueueSize = 512
)

var autoDownloadSlots = make(chan struct{}, autoDownloadParallel)

func autoDownloadPref(mediaType string, size int64, gif bool) string {
	switch mediaType {
	case "image":
		if size <= autoDownloadMaxSize {
			return "auto_download_photos"
		}
	case "video":
		if gif && size < autoDownloadGIFMax {
			return "auto_download_photos"
		}
		if size <= autoDownloadMaxSize {
			return "auto_download_videos"
		}
	case "document":
		if size <= autoDownloadMaxSize {
			return "auto_download_documents"
		}
	case "audio":
		if size < autoDownloadAudioMax {
			return "auto_download_audio"
		}
	}
	return ""
}

var (
	autoDownloadMu      sync.Mutex
	autoDownloadPending int
)

// maybeAutoDownload starts downloading a newly arrived message's media when
// the matching auto-download setting is on and the sender is trusted, as
// WhatsApp Web decides it (shouldAutoDownloadMedia, isTrusted): it's from us
// or a saved contact, or we've written in that chat. Channel posts never are.
func maybeAutoDownload(msg *waE2E.Message, info types.MessageInfo, jid, mediaType string, size int64) {
	gif := msg.GetVideoMessage().GetGifPlayback()
	pref := autoDownloadPref(mediaType, size, gif)
	if pref == "" || info.Chat.Server == types.NewsletterServer || !appPref(pref) {
		return
	}
	if !info.IsFromMe && !trustedForAutoDownload(info, jid) {
		return
	}
	autoDownloadMu.Lock()
	if autoDownloadPending >= autoDownloadQueueSize {
		autoDownloadMu.Unlock()
		return
	}
	autoDownloadPending++
	autoDownloadMu.Unlock()
	go func() {
		autoDownloadSlots <- struct{}{}
		defer func() {
			<-autoDownloadSlots
			autoDownloadMu.Lock()
			autoDownloadPending--
			autoDownloadMu.Unlock()
		}()
		if appPref(pref) {
			downloadMediaNow(MessageRefData{MessageID: info.ID, JID: jid}, true)
		}
	}()
}

func trustedForAutoDownload(info types.MessageInfo, jid string) bool {
	sender := preferPN(info.Sender.ToNonAD())
	if c, err := wa().Store.Contacts.GetContact(context.Background(), sender); err == nil && c.Found && c.FullName != "" {
		return true
	}
	var sent int
	db.QueryRow(`SELECT COUNT(*) FROM (SELECT 1 FROM messages WHERE jid = ? AND from_me = 1 LIMIT 1)`, jid).Scan(&sent)
	return sent > 0
}
