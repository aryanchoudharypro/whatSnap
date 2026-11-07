package main

import (
	"context"
	"encoding/json"
	"log"
	"net"

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
	"hd_auto_download":        false,
	"security_notifications":  true,
}

func (acc *Account) appPref(name string) bool {
	var v int
	if acc.db.QueryRow(`SELECT CAST(value AS INTEGER) FROM app_settings WHERE key = ?`, "pref:"+name).Scan(&v) != nil {
		return appPrefDefaults[name]
	}
	return v == 1
}

func (acc *Account) appPrefsPayload() map[string]interface{} {
	out := map[string]interface{}{"link_previews_disabled": acc.linkPreviewsDisabled()}
	for name := range appPrefDefaults {
		out[name] = acc.appPref(name)
	}
	return out
}

func (acc *Account) handleGetAppPrefs(conn net.Conn) {
	acc.sendEvent(conn, "app_prefs", acc.appPrefsPayload())
}

func (acc *Account) handleSetAppPrefs(raw json.RawMessage) {
	var data map[string]bool
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	changed := map[string]bool{}
	for name, v := range data {
		if _, ok := appPrefDefaults[name]; ok {
			if acc.appPref(name) != v {
				changed[name] = v
			}
			acc.db.Exec(`INSERT OR REPLACE INTO app_settings (key, value) VALUES (?, ?)`, "pref:"+name, boolToInt(v))
		}
	}
	acc.syncAppPrefs(changed)
	off, ok := data["link_previews_disabled"]
	if !ok || off == acc.linkPreviewsDisabled() {
		return
	}
	go func() {
		if err := acc.sendAppStateWithRetry(context.Background(), appstate.BuildDisableLinkPreviews(off)); err != nil {
			log.Printf("Turning link previews %v failed: %v", map[bool]string{true: "off", false: "on"}[off], err)
			acc.broadcastEvent("error", map[string]string{"message": "Couldn't change link previews: " + err.Error()})
			return
		}
		acc.db.Exec(`INSERT OR REPLACE INTO app_settings (key, value) VALUES (?, ?)`, linkPreviewsOffKey, boolToInt(off))
	}()
}

const (
	autoDownloadMaxSize   = 32 << 20
	autoDownloadAudioMax  = 512 << 10
	autoDownloadGIFMax    = 500 << 10
	autoDownloadParallel  = 5
	autoDownloadQueueSize = 512
)

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

// maybeAutoDownload starts downloading a newly arrived message's media when
// the matching auto-download setting is on and the sender is trusted, as
// WhatsApp Web decides it (shouldAutoDownloadMedia, isTrusted): it's from us
// or a saved contact, or we've written in that chat. Channel posts never are.
func (acc *Account) maybeAutoDownload(msg *waE2E.Message, info types.MessageInfo, jid, mediaType string, size int64) {
	gif := msg.GetVideoMessage().GetGifPlayback()
	pref := autoDownloadPref(mediaType, size, gif)
	if pref == "" || info.Chat.Server == types.NewsletterServer || !acc.appPref(pref) {
		return
	}
	if !info.IsFromMe && !acc.trustedForAutoDownload(info, jid) {
		return
	}
	acc.autoDownloadMu.Lock()
	if acc.autoDownloadPending >= autoDownloadQueueSize {
		acc.autoDownloadMu.Unlock()
		return
	}
	acc.autoDownloadPending++
	acc.autoDownloadMu.Unlock()
	go func() {
		acc.autoDownloadSlots <- struct{}{}
		defer func() {
			<-acc.autoDownloadSlots
			acc.autoDownloadMu.Lock()
			acc.autoDownloadPending--
			acc.autoDownloadMu.Unlock()
		}()
		if acc.appPref(pref) {
			acc.downloadMediaNow(MessageRefData{MessageID: info.ID, JID: jid}, true)
		}
	}()
}

func (acc *Account) trustedForAutoDownload(info types.MessageInfo, jid string) bool {
	sender := acc.preferPN(info.Sender.ToNonAD())
	if c, err := acc.wa().Store.Contacts.GetContact(context.Background(), sender); err == nil && c.Found && c.FullName != "" {
		return true
	}
	var sent int
	acc.db.QueryRow(`SELECT COUNT(*) FROM (SELECT 1 FROM messages WHERE jid = ? AND from_me = 1 LIMIT 1)`, jid).Scan(&sent)
	return sent > 0
}
