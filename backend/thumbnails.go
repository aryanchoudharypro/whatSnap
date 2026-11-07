package main

import (
	"bytes"
	"encoding/json"
	"image"
	"image/jpeg"
	"image/png"
	"net"
	"os"
	"path/filepath"
	"sync"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"google.golang.org/protobuf/proto"
)

// The small pictures the message list draws for photos, videos and stickers,
// as WhatsApp Web does. A photo or sticker already downloaded is shrunk from
// the file itself; anything else uses the preview the message carries. They
// are kept as files under media/thumbs, one per message, so each is made
// once. View once media never gets one, as on WhatsApp.

const listThumbMaxEdge = 320

type thumbnailsData struct {
	IDs []string `json:"ids"`
}

type thumbnailItem struct {
	ID   string `json:"id"`
	Path string `json:"path"`
}

var thumbnailMu sync.Mutex

func (acc *Account) thumbsDir() string { return filepath.Join(acc.mediaBaseDir(), "thumbs") }

func (acc *Account) handleGetThumbnails(conn net.Conn, raw json.RawMessage) {
	var data thumbnailsData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		var batch []thumbnailItem
		for i, id := range data.IDs {
			if id == "" {
				continue
			}
			batch = append(batch, thumbnailItem{ID: id, Path: acc.messageThumbnail(id)})
			if len(batch) == 4 || i == len(data.IDs)-1 {
				acc.sendEvent(conn, "thumbnails", map[string]interface{}{"items": batch})
				batch = nil
			}
		}
		if len(batch) > 0 {
			acc.sendEvent(conn, "thumbnails", map[string]interface{}{"items": batch})
		}
	}()
}

func (acc *Account) messageThumbnail(id string) string {
	thumbnailMu.Lock()
	defer thumbnailMu.Unlock()
	var raw []byte
	var mediaType, mediaPath string
	var viewOnce int
	if acc.db.QueryRow(`SELECT raw, COALESCE(media_type, ''), COALESCE(media_path, ''), COALESCE(view_once, 0) FROM messages WHERE id = ?`, id).
		Scan(&raw, &mediaType, &mediaPath, &viewOnce) != nil || viewOnce != 0 {
		return ""
	}
	msg := &waE2E.Message{}
	if len(raw) > 0 && proto.Unmarshal(raw, msg) != nil {
		return ""
	}
	if isViewOnceProto(msg) {
		return ""
	}
	if err := os.MkdirAll(acc.thumbsDir(), 0o755); err != nil {
		return ""
	}
	base := filepath.Join(acc.thumbsDir(), sanitizeFilenameComponent(id))

	if mediaPath != "" && (mediaType == "image" || mediaType == "sticker") {
		ext := ".view.jpg"
		if mediaType == "sticker" {
			ext = ".view.png"
		}
		out := base + ext
		if _, err := os.Stat(out); err == nil {
			return out
		}
		if makeFileThumbnail(mediaPath, out, mediaType == "sticker") {
			return out
		}
	}

	inner := unwrapMessage(msg)
	var preview []byte
	switch {
	case inner.GetImageMessage() != nil:
		preview = inner.GetImageMessage().GetJPEGThumbnail()
	case inner.GetVideoMessage() != nil:
		preview = inner.GetVideoMessage().GetJPEGThumbnail()
	}
	if len(preview) == 0 {
		return ""
	}
	out := base + ".jpg"
	if _, err := os.Stat(out); err == nil {
		return out
	}
	if os.WriteFile(out, preview, 0o644) != nil {
		return ""
	}
	return out
}

func isViewOnceProto(msg *waE2E.Message) bool {
	if msg.GetViewOnceMessage() != nil || msg.GetViewOnceMessageV2() != nil || msg.GetViewOnceMessageV2Extension() != nil {
		return true
	}
	inner := unwrapMessage(msg)
	return inner.GetImageMessage().GetViewOnce() || inner.GetVideoMessage().GetViewOnce() ||
		inner.GetAudioMessage().GetViewOnce()
}

func makeFileThumbnail(path, out string, keepAlpha bool) bool {
	data, err := os.ReadFile(path)
	if err != nil {
		return false
	}
	img, format, err := image.Decode(bytes.NewReader(data))
	if err != nil {
		return false
	}
	if format == "jpeg" {
		img = applyEXIFOrientation(img, jpegOrientation(data))
	}
	img = fitWithin(img, listThumbMaxEdge, bilinear)
	var buf bytes.Buffer
	if keepAlpha {
		err = png.Encode(&buf, img)
	} else {
		err = jpeg.Encode(&buf, onWhite(img), &jpeg.Options{Quality: 85})
	}
	return err == nil && os.WriteFile(out, buf.Bytes(), 0o644) == nil
}

type waveformItem struct {
	ID   string `json:"id"`
	Bars []int  `json:"bars"`
}

func (acc *Account) handleGetWaveforms(conn net.Conn, raw json.RawMessage) {
	var data thumbnailsData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		items := make([]waveformItem, 0, len(data.IDs))
		for _, id := range data.IDs {
			items = append(items, waveformItem{ID: id, Bars: acc.messageWaveform(id)})
		}
		acc.sendEvent(conn, "waveforms", map[string]interface{}{"items": items})
	}()
}

func (acc *Account) messageWaveform(id string) []int {
	var raw []byte
	if acc.db.QueryRow(`SELECT raw FROM messages WHERE id = ?`, id).Scan(&raw) != nil || len(raw) == 0 {
		return []int{}
	}
	msg := &waE2E.Message{}
	if proto.Unmarshal(raw, msg) != nil {
		return []int{}
	}
	bars := []int{}
	for _, b := range unwrapMessage(msg).GetAudioMessage().GetWaveform() {
		bars = append(bars, min(int(b), 100))
	}
	return bars
}
