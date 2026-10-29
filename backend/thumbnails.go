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

// The longest side of a picture made from a downloaded file: sharp in a
// row of the message list at twice the usual screen scale.
const listThumbMaxEdge = 320

type thumbnailsData struct {
	IDs []string `json:"ids"`
}

type thumbnailItem struct {
	ID   string `json:"id"`
	Path string `json:"path"` // "" when there's nothing to show
}

// Making one from a big photo takes a moment; one at a time is enough, and
// keeps two requests for the same row from both doing it.
var thumbnailMu sync.Mutex

func thumbsDir() string { return filepath.Join(mediaBaseDir(), "thumbs") }

func handleGetThumbnails(conn net.Conn, raw json.RawMessage) {
	var data thumbnailsData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		// Sent in small groups, so the first rows show while the rest are made.
		var batch []thumbnailItem
		for i, id := range data.IDs {
			if id == "" {
				continue
			}
			batch = append(batch, thumbnailItem{ID: id, Path: messageThumbnail(id)})
			if len(batch) == 4 || i == len(data.IDs)-1 {
				sendEvent(conn, "thumbnails", map[string]interface{}{"items": batch})
				batch = nil
			}
		}
		if len(batch) > 0 {
			sendEvent(conn, "thumbnails", map[string]interface{}{"items": batch})
		}
	}()
}

func messageThumbnail(id string) string {
	thumbnailMu.Lock()
	defer thumbnailMu.Unlock()
	var raw []byte
	var mediaType, mediaPath string
	var viewOnce int
	if db.QueryRow(`SELECT raw, COALESCE(media_type, ''), COALESCE(media_path, ''), COALESCE(view_once, 0) FROM messages WHERE id = ?`, id).
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
	if err := os.MkdirAll(thumbsDir(), 0o755); err != nil {
		return ""
	}
	base := filepath.Join(thumbsDir(), sanitizeFilenameComponent(id))

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

// makeFileThumbnail shrinks the picture at path into out: a JPEG for a
// photo, a PNG for a sticker so it keeps its see-through background.
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
