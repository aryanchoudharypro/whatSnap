package main

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"image"
	"io"
	"log"
	"mime"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"time"

	"go.mau.fi/whatsmeow/proto/waMmsRetry"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"golang.org/x/sys/windows"
	"google.golang.org/protobuf/proto"
	"net"
)

func friendlyMediaError(err error) string {
	msg := err.Error()
	switch {
	case strings.Contains(msg, "hash of media ciphertext doesn't match"):
		return "This media is no longer available. The sender's phone no longer has the original file to resend."
	case strings.Contains(msg, "Phone could not re-upload"):
		return "This media is no longer available. The sender's phone couldn't resend it."
	case strings.Contains(msg, "Timeout waiting for phone to re-upload"):
		return "This media is no longer available. The sender's phone didn't respond in time."
	default:
		return msg
	}
}

func ensureMediaCached(messageID string) (path, mediaType, mimetype, filename string, err error) {
	if viewOnceState(messageID) == viewOnceOpened {
		err = fmt.Errorf("this view once message has already been opened")
		return
	}
	var cached string
	db.QueryRow("SELECT COALESCE(media_path, '') FROM messages WHERE id = ?", messageID).Scan(&cached)
	if cached != "" {
		if _, statErr := os.Stat(cached); statErr == nil {
			db.QueryRow("SELECT COALESCE(media_type,''), COALESCE(media_mimetype,''), COALESCE(media_filename,'') FROM messages WHERE id = ?", messageID).
				Scan(&mediaType, &mimetype, &filename)
			filename = sanitizeFilename(filename)
			return cached, mediaType, mimetype, filename, nil
		}
	}
	path, mt, mime, fn, dErr := downloadMessageMediaToFile(context.Background(), messageID, nil)
	if dErr != nil {
		if strings.Contains(dErr.Error(), "403") || strings.Contains(dErr.Error(), "404") || strings.Contains(dErr.Error(), "410") {
			fmt.Printf("Requesting media retry (sync) for %s...\n", messageID)
			retryErr := requestMediaRetry(messageID)
			if retryErr != nil {
				return "", "", "", "", fmt.Errorf("URL expired and retry request failed: %v", retryErr)
			}
			return "", "", "", "", fmt.Errorf("Media URL expired. Requesting a fresh copy from the sender's phone, please wait a moment and try again.")
		}
		return "", "", "", "", dErr
	}
	return path, mt, mime, fn, nil
}

var (
	activeDownloadsMu sync.Mutex
	activeDownloads   = map[string]context.CancelFunc{}
	// autoDownloadFailed caches auto_download_failed, the messages whose
	// automatic download failed (guarded by activeDownloadsMu). It's kept in
	// the database too, so the startup sticker pass doesn't try the same
	// broken files on every launch.
	autoDownloadFailed     = map[string]bool{}
	autoDownloadFailedOnce sync.Once
	activeUploadsMu   sync.Mutex
	activeUploads     = map[string]context.CancelFunc{}
	retryWaitersMu    sync.Mutex
	retryWaiters      = map[string]chan bool{}
	pendingAutoOpens  sync.Map
)

type progressFile struct {
	*os.File
	total      int64
	written    int64
	lastReport int
	onProgress func(percent int)
}

func (pf *progressFile) Write(p []byte) (int, error) {
	n, err := pf.File.Write(p)
	if n > 0 && pf.total > 0 && pf.onProgress != nil {
		pf.written += int64(n)
		percent := int(float64(pf.written) * 100 / float64(pf.total))
		if percent > 99 {
			percent = 99
		}
		if percent-pf.lastReport >= 5 {
			pf.lastReport = percent
			pf.onProgress(percent)
		}
	}
	return n, err
}

func downloadMessageMediaToFile(ctx context.Context, messageID string, onProgress func(int)) (path, mediaType, mimetype, filename string, err error) {
	rawBytes, _, _, _, dbErr := getMessageRaw(messageID)
	if dbErr != nil {
		err = dbErr
		return
	}
	if hd := hdMediaRaw(messageID); hd != nil && appPref("hd_auto_download") {
		path, mediaType, mimetype, filename, err = downloadRawMediaToFile(ctx, messageID, hd, onProgress)
		if err == nil || errors.Is(err, context.Canceled) {
			return
		}
		log.Printf("HD copy of %s didn't download (%v); getting the standard one", messageID, err)
	}
	return downloadRawMediaToFile(ctx, messageID, rawBytes, onProgress)
}

func downloadRawMediaToFile(ctx context.Context, messageID string, rawBytes []byte, onProgress func(int)) (path, mediaType, mimetype, filename string, err error) {
	msg := &waE2E.Message{}
	if perr := proto.Unmarshal(rawBytes, msg); perr != nil {
		err = perr
		return
	}
	msg = mediaCarrier(msg)
	var dl whatsmeow.DownloadableMessage
	var total int64
	var sticker *waE2E.StickerMessage
	switch {
	case msg.GetImageMessage() != nil:
		m := msg.GetImageMessage()
		dl, mediaType, mimetype, total = m, "image", m.GetMimetype(), int64(m.GetFileLength())
	case msg.GetVideoMessage() != nil:
		m := msg.GetVideoMessage()
		dl, mediaType, mimetype, total = m, "video", m.GetMimetype(), int64(m.GetFileLength())
	case msg.GetAudioMessage() != nil:
		m := msg.GetAudioMessage()
		mediaType = "audio"
		if m.GetPTT() {
			mediaType = "voice"
		}
		dl, mimetype, total = m, m.GetMimetype(), int64(m.GetFileLength())
	case msg.GetDocumentMessage() != nil:
		m := msg.GetDocumentMessage()
		filename = sanitizeFilename(m.GetFileName())
		if filename == "" {
			filename = sanitizeFilename(m.GetTitle())
		}
		dl, mediaType, mimetype, total = m, "document", m.GetMimetype(), int64(m.GetFileLength())
	case msg.GetStickerMessage() != nil:
		m := msg.GetStickerMessage()
		dl, mediaType, mimetype, total = m, "sticker", m.GetMimetype(), int64(m.GetFileLength())
		sticker = m
	default:
		err = fmt.Errorf("message has no downloadable media")
		return
	}
	finalPath := filepath.Join(mediaDir(), sanitizeFilenameComponent(messageID)+extForMime(mimetype))
	f, ferr := os.CreateTemp(filepath.Dir(finalPath), filepath.Base(finalPath)+".*.part")
	if ferr != nil {
		err = ferr
		return
	}
	partPath := f.Name()
	dlErr := wa().DownloadToFile(ctx, dl, &progressFile{File: f, total: total, onProgress: onProgress})
	f.Close()
	if dlErr != nil {
		os.Remove(partPath)
		err = dlErr
		return
	}
	if rerr := os.Rename(partPath, finalPath); rerr != nil {
		os.Remove(partPath)
		if _, statErr := os.Stat(finalPath); statErr != nil {
			err = rerr
			return
		}
	}
	setMessageMediaPath(messageID, finalPath)
	if sticker != nil && sticker.GetAccessibilityLabel() == "" {
		labelled := proto.Clone(sticker).(*waE2E.StickerMessage)
		labelStickerFromCache(labelled, finalPath)
		if newText := stickerText(labelled); newText != stickerText(sticker) {
			db.Exec("UPDATE messages SET text = ? WHERE id = ?", newText, messageID)
			var jid string
			db.QueryRow(`SELECT COALESCE(jid, '') FROM messages WHERE id = ?`, messageID).Scan(&jid)
			broadcastEvent("message_updated", map[string]interface{}{"jid": jid, "message_id": messageID, "text": newText})
		}
	}
	if filename != "" {
		db.Exec("UPDATE messages SET media_filename = ? WHERE id = ?", filename, messageID)
	}
	if mediaType != "" {
		db.Exec("UPDATE messages SET media_type = COALESCE(NULLIF(media_type,''), ?), media_mimetype = COALESCE(NULLIF(media_mimetype,''), ?) WHERE id = ?", mediaType, mimetype, messageID)
	}
	path = finalPath
	return
}

func handleDownloadMedia(conn net.Conn, raw json.RawMessage) {
	var data MessageRefData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	startMediaDownload(data, false)
}

func autoDownloadVoiceNote(messageID, jid string) {
	startMediaDownload(MessageRefData{MessageID: messageID, JID: jid}, true)
}

func startMediaDownload(data MessageRefData, auto bool) {
	downloadMedia(data, auto, false)
}

func downloadMediaNow(data MessageRefData, auto bool) {
	downloadMedia(data, auto, true)
}

func downloadMedia(data MessageRefData, auto, wait bool) {
	var cached, mt, mm, fn string
	db.QueryRow("SELECT COALESCE(media_path,''), COALESCE(media_type,''), COALESCE(media_mimetype,''), COALESCE(media_filename,'') FROM messages WHERE id = ?", data.MessageID).
		Scan(&cached, &mt, &mm, &fn)
	if cached != "" {
		if _, statErr := os.Stat(cached); statErr == nil {
			broadcastEvent("media_downloaded", map[string]interface{}{
				"message_id": data.MessageID,
				"jid":        data.JID,
				"auto":       auto,
				"path":       cached,
				"mediaType":  mt,
				"mimetype":   mm,
				"filename":   fn,
			})
			return
		}
	}
	activeDownloadsMu.Lock()
	if _, exists := activeDownloads[data.MessageID]; exists {
		activeDownloadsMu.Unlock()
		return
	}
	// Like WA Web, an automatic download that failed isn't tried again on its
	// own; the message waits for the user to download it (WA Web's "tap to
	// download", DownloadStage.NEED_POKE). Without this, the sticker panel
	// and the startup sticker pass kept retrying the same files.
	autoDownloadFailedOnce.Do(loadAutoDownloadFailed)
	if auto && autoDownloadFailed[data.MessageID] {
		activeDownloadsMu.Unlock()
		return
	}
	ctx, cancel := context.WithCancel(context.Background())
	activeDownloads[data.MessageID] = cancel
	activeDownloadsMu.Unlock()
	failed := false
	run := func() {
		defer func() {
			activeDownloadsMu.Lock()
			delete(activeDownloads, data.MessageID)
			if auto && failed {
				autoDownloadFailed[data.MessageID] = true
				db.Exec(`INSERT OR IGNORE INTO auto_download_failed (id, failed_at) VALUES (?, ?)`, data.MessageID, time.Now().Unix())
			} else if !auto && !failed {
				delete(autoDownloadFailed, data.MessageID)
				db.Exec(`DELETE FROM auto_download_failed WHERE id = ?`, data.MessageID)
			}
			activeDownloadsMu.Unlock()
			cancel()
		}()
		// Automatic downloads share WA Web's limit of 5 at a time
		// (WAWebMediaAutoDownloadQueue); maybeAutoDownload already holds a
		// slot when it waits for the download itself.
		if auto && !wait {
			select {
			case autoDownloadSlots <- struct{}{}:
				defer func() { <-autoDownloadSlots }()
			case <-ctx.Done():
				return
			}
		}
		onProgress := func(percent int) {
			broadcastEvent("media_download_progress", map[string]interface{}{
				"message_id": data.MessageID,
				"jid":        data.JID,
				"auto":       auto,
				"percent":    percent,
			})
		}
		started := time.Now()
		log.Printf("Media download started for message %s (auto %v)", data.MessageID, auto)
		for attempt := 0; attempt < 2; attempt++ {
			path, mediaType, mimetype, filename, err := downloadMessageMediaToFile(ctx, data.MessageID, onProgress)
			if err == nil {
				log.Printf("Media download finished for message %s in %s", data.MessageID, time.Since(started).Round(time.Millisecond))
			}
			if err == nil {
				broadcastEvent("media_downloaded", map[string]interface{}{
					"message_id": data.MessageID,
					"jid":        data.JID,
					"auto":       auto,
					"path":       path,
					"mediaType":  mediaType,
					"mimetype":   mimetype,
					"filename":   filename,
				})
				return
			}
			fmt.Printf("Media download failed for message %s (attempt %d): %v\n", data.MessageID, attempt+1, err)

			if errors.Is(err, context.Canceled) {
				broadcastEvent("media_download_cancelled", map[string]interface{}{
					"message_id": data.MessageID,
					"jid":        data.JID,
					"auto":       auto,
				})
				return
			}

			failed = true
			// WA Web only asks the phone to upload a file again when the user
			// asked for it (downloadEvenIfExpensive); an automatic download of
			// an expired file just stops.
			// A file whose bytes don't match its hash gets the same re-upload
			// request as an expired one when the user asked for it (WA Web:
			// "hash mismatch, requesting re-upload").
			hashMismatch := errors.Is(err, whatsmeow.ErrInvalidMediaEncSHA256) || errors.Is(err, whatsmeow.ErrInvalidMediaSHA256)
			if !auto && attempt == 0 && (hashMismatch || strings.Contains(err.Error(), "403") || strings.Contains(err.Error(), "404") || strings.Contains(err.Error(), "410")) {
				failed = false
				fmt.Printf("Requesting media retry for %s...\n", data.MessageID)
				retryErr := requestMediaRetry(data.MessageID)
				if retryErr != nil {
					fmt.Printf("Retry request failed: %v\n", retryErr)
					broadcastEvent("media_download_failed", map[string]interface{}{
						"message_id": data.MessageID,
						"jid":        data.JID,
						"auto":       auto,
						"message":    "URL expired and retry request failed: " + retryErr.Error(),
					})
					return
				}

				ch := make(chan bool, 1)
				retryWaitersMu.Lock()
				retryWaiters[data.MessageID] = ch
				retryWaitersMu.Unlock()

				select {
				case success := <-ch:
					if !success {
						broadcastEvent("media_download_failed", map[string]interface{}{
							"message_id": data.MessageID,
							"jid":        data.JID,
							"auto":       auto,
							"message":    "Phone could not re-upload the media",
						})
						return
					}
					continue
				case <-time.After(45 * time.Second):
					broadcastEvent("media_download_failed", map[string]interface{}{
						"message_id": data.MessageID,
						"jid":        data.JID,
						"auto":       auto,
						"message":    "Timeout waiting for phone to re-upload media",
					})
					retryWaitersMu.Lock()
					delete(retryWaiters, data.MessageID)
					retryWaitersMu.Unlock()
					return
				case <-ctx.Done():
					broadcastEvent("media_download_cancelled", map[string]interface{}{
						"message_id": data.MessageID,
						"jid":        data.JID,
						"auto":       auto,
					})
					retryWaitersMu.Lock()
					delete(retryWaiters, data.MessageID)
					retryWaitersMu.Unlock()
					return
				}
			}

			broadcastEvent("media_download_failed", map[string]interface{}{
				"message_id": data.MessageID,
				"jid":        data.JID,
				"auto":       auto,
				"message":    friendlyMediaError(err),
			})
			return
		}
	}
	if wait {
		run()
	} else {
		go run()
	}
}

func handleCancelUpload(raw json.RawMessage) {
	var data MessageRefData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	activeUploadsMu.Lock()
	cancel := activeUploads[data.MessageID]
	activeUploadsMu.Unlock()
	if cancel != nil {
		cancel()
	}
}

func handleCancelDownload(conn net.Conn, raw json.RawMessage) {
	var data MessageRefData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	activeDownloadsMu.Lock()
	cancel := activeDownloads[data.MessageID]
	activeDownloadsMu.Unlock()
	if cancel != nil {
		cancel()
	} else {
		broadcastEvent("media_download_cancelled", map[string]interface{}{
			"message_id": data.MessageID,
			"jid":        data.JID,
		})
	}
}

func openFileInOS(path string) error {
	verb, err := windows.UTF16PtrFromString("open")
	if err != nil {
		return err
	}
	file, err := windows.UTF16PtrFromString(path)
	if err != nil {
		return err
	}
	return windows.ShellExecute(0, verb, file, nil, nil, windows.SW_SHOWNORMAL)
}

func copyFileStreaming(src, dst string) error {
	in, err := os.Open(src)
	if err != nil {
		return err
	}
	defer in.Close()
	out, err := os.Create(dst)
	if err != nil {
		return err
	}
	if _, err := io.Copy(out, in); err != nil {
		out.Close()
		os.Remove(dst)
		return err
	}
	return out.Close()
}

func handleOpenMedia(conn net.Conn, raw json.RawMessage) {
	var data MessageRefData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		path, mediaType, mimetype, filename, err := ensureMediaCached(data.MessageID)
		if err != nil {
			if strings.Contains(err.Error(), "URL expired") {
				pendingAutoOpens.Store(data.MessageID, map[string]interface{}{
					"conn":   conn,
					"action": "open_media",
				})
				return
			}
			sendEvent(conn, "error", map[string]string{"message": friendlyMediaError(err)})
			return
		}
		openMediaFileWithRealName(conn, data.MessageID, path, mediaType, mimetype, filename)
	}()
}

func handlePrepareMedia(conn net.Conn, raw json.RawMessage) {
	var data MessageRefData
	if err := json.Unmarshal(raw, &data); err != nil || data.MessageID == "" {
		return
	}
	if viewOnceState(data.MessageID) != 0 {
		sendEvent(conn, "media_failed", map[string]string{"message_id": data.MessageID, "message": "View once media opens outside the viewer."})
		return
	}
	go func() {
		path, mediaType, mimetype, filename, err := ensureMediaCached(data.MessageID)
		if err != nil {
			if strings.Contains(err.Error(), "URL expired") && !strings.Contains(err.Error(), "retry request failed") {
				pendingAutoOpens.Store(data.MessageID, map[string]interface{}{"conn": conn, "action": "prepare_media"})
				sendEvent(conn, "media_pending", map[string]string{"message_id": data.MessageID})
				return
			}
			sendEvent(conn, "media_failed", map[string]string{"message_id": data.MessageID, "message": friendlyMediaError(err)})
			return
		}
		sendMediaReady(conn, data.MessageID, path, mediaType, mimetype, filename)
	}()
}

func sendMediaReady(conn net.Conn, messageID, path, mediaType, mimetype, filename string) {
	if absPath, err := filepath.Abs(path); err == nil {
		path = absPath
	}
	sendEvent(conn, "media_ready", map[string]string{
		"message_id": messageID, "path": path, "mediaType": mediaType, "mimetype": mimetype, "filename": filename,
	})
}

func handleSaveMediaAs(conn net.Conn, raw json.RawMessage) {
	var data SaveMediaAsData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	if msg := viewOnceBlocked(data.MessageID); msg != "" {
		sendEvent(conn, "error", map[string]string{"message": msg})
		return
	}
	go func() {
		path, _, _, _, err := ensureMediaCached(data.MessageID)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": friendlyMediaError(err)})
			return
		}
		if err := copyFileStreaming(path, data.DestPath); err != nil {
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}
		sendEvent(conn, "media_saved", map[string]interface{}{"message_id": data.MessageID, "path": data.DestPath})
	}()
}

func buildAttachmentContext(replyToID, replySender string) (ctxInfo *waE2E.ContextInfo, quotedID, quotedText, quotedSender string) {
	if replyToID == "" {
		return nil, "", "", ""
	}
	quotedMsg, storedSenderJID, preview, err := getMessageForQuote(replyToID)
	if err != nil {
		return nil, "", "", ""
	}
	participant := replySender
	if participant == "" {
		participant = storedSenderJID
	}
	displayParticipant := participant
	// The frontend uses the literal string "me" as its own-message sender
	// sentinel (see resolveQuotedSenderName, which expects it for display) -
	// but ContextInfo.Participant goes out on the wire and has to be a real
	// JID, or the quote never renders as a reply for the recipient.
	if participant == "me" {
		if wa().Store.ID != nil {
			participant = wa().Store.ID.ToNonAD().String()
		} else {
			participant = ""
		}
	}
	participant = wireParticipantFor(replyToID, participant)
	ctxInfo = &waE2E.ContextInfo{StanzaID: proto.String(replyToID), QuotedMessage: quotedMsg}
	quoteStatus(ctxInfo, replyToID)
	if participant != "" {
		ctxInfo.Participant = proto.String(participant)
	}
	return ctxInfo, replyToID, preview, resolveQuotedSenderName(displayParticipant)
}

type ProgressReader struct {
	reader     io.Reader
	total      int64
	read       int64
	lastReport int
	onProgress func(percent int)
}

func (pr *ProgressReader) Read(p []byte) (int, error) {
	n, err := pr.reader.Read(p)
	if n > 0 {
		pr.read += int64(n)
		if pr.total > 0 {
			percent := int(float64(pr.read) * 100 / float64(pr.total))
			if percent != pr.lastReport && (percent-pr.lastReport >= 1 || percent == 100) {
				pr.lastReport = percent
				if pr.onProgress != nil {
					pr.onProgress(percent)
				}
			}
		}
	}
	return n, err
}

type channelUploadReader struct {
	*os.File
	progress *ProgressReader
	seeks    int
}

func (r *channelUploadReader) Read(p []byte) (int, error) {
	if r.seeks == 0 {
		return r.File.Read(p)
	}
	return r.progress.Read(p)
}

func (r *channelUploadReader) Seek(offset int64, whence int) (int64, error) {
	r.seeks++
	r.progress.read = 0
	return r.File.Seek(offset, whence)
}

type encryptedUploadFile struct {
	*os.File
	progress *ProgressReader
}

func (u *encryptedUploadFile) Read(p []byte) (int, error) {
	return u.progress.Read(p)
}

func detectMimeType(filename, path string, header []byte) string {
	nameToTest := filename
	if nameToTest == "" {
		nameToTest = path
	}
	ext := strings.ToLower(filepath.Ext(nameToTest))
	switch ext {
	case ".docx":
		return "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
	case ".xlsx":
		return "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
	case ".pptx":
		return "application/vnd.openxmlformats-officedocument.presentationml.presentation"
	case ".doc":
		return "application/msword"
	case ".xls":
		return "application/vnd.ms-excel"
	case ".ppt":
		return "application/vnd.ms-powerpoint"
	case ".pdf":
		return "application/pdf"
	case ".zip":
		return "application/zip"
	case ".rar":
		return "application/x-rar-compressed"
	case ".7z":
		return "application/x-7z-compressed"
	case ".txt":
		return "text/plain"
	case ".csv":
		return "text/csv"
	case ".json":
		return "application/json"
	}
	if ext != "" {
		if mimeType := mime.TypeByExtension(ext); mimeType != "" {
			return mimeType
		}
	}
	if len(header) > 0 {
		sniffLen := len(header)
		if sniffLen > 512 {
			sniffLen = 512
		}
		return http.DetectContentType(header[:sniffLen])
	}
	return "application/octet-stream"
}

const (
	abPropAudioLimitMB       = 3657
	abPropMediaLimitMB       = 3660
	abPropVideoLimitMB       = 3185
	abPropStatusMediaLimitMB = 3659
	defaultMediaLimitMB      = 16
	maxDocumentBytes         = 2000 << 20
)

func attachmentSizeLimit(mediaType string) int64 {
	code := 0
	switch mediaType {
	case "image", "sticker":
		code = abPropMediaLimitMB
	case "video":
		code = abPropVideoLimitMB
	case "audio":
		code = abPropAudioLimitMB
	default:
		return maxDocumentBytes
	}
	return int64(serverFlagInt(code, defaultMediaLimitMB)) << 20
}

func attachmentTooLargeText(mediaType string, size, limit int64) string {
	mb := func(n int64) string { return strconv.FormatFloat(float64(n)/(1<<20), 'f', 1, 64) + " MB" }
	switch mediaType {
	case "image", "sticker", "video", "audio":
		return fmt.Sprintf("This %s is %s; WhatsApp sends %ss up to %s. Send it with the Document option instead.", mediaType, mb(size), mediaType, mb(limit))
	}
	return fmt.Sprintf("This file is %s; WhatsApp sends files up to %s.", mb(size), mb(limit))
}

func phonePlayableAudio(mimetype string) bool {
	switch mimetype {
	case "audio/ogg; codecs=opus", "audio/mp4", "audio/mpeg", "audio/aac", "audio/amr":
		return true
	}
	return false
}

func stampUploadedMedia(msg *waE2E.Message, uploaded whatsmeow.UploadResponse) {
	if len(uploaded.MediaKey) == 0 {
		return
	}
	keyTime := proto.Int64(time.Now().Unix())
	switch {
	case msg.ImageMessage != nil:
		msg.ImageMessage.MediaKeyTimestamp = keyTime
	case msg.VideoMessage != nil:
		msg.VideoMessage.MediaKeyTimestamp = keyTime
		msg.VideoMessage.StreamingSidecar = uploaded.StreamingSidecar
	case msg.AudioMessage != nil:
		msg.AudioMessage.MediaKeyTimestamp = keyTime
		msg.AudioMessage.StreamingSidecar = uploaded.StreamingSidecar
	case msg.StickerMessage != nil:
		msg.StickerMessage.MediaKeyTimestamp = keyTime
	case msg.DocumentMessage != nil:
		msg.DocumentMessage.MediaKeyTimestamp = keyTime
	}
}

// audioMimetype labels an audio attachment the way WhatsApp Web does. Web
// sends audio messages only as these types (its media-type table in
// WAWebMimeTypes: audio/aac, audio/mp4, audio/amr, audio/mpeg,
// "audio/ogg; codecs=opus", audio/wav), but detectMimeType yields other
// names for the same files - sniffing gives "application/ogg" for Ogg,
// "video/mp4" for M4A and "audio/wave" for WAV, and the Windows registry
// often has no .ogg entry or maps .aac to "audio/vnd.dlna.adts". Phones
// don't play an AudioMessage labelled with a type they don't expect.
// The file's own bytes decide first; the extension is the fallback.
func audioMimetype(mimetype, filename string, header []byte) string {
	switch {
	case bytes.HasPrefix(header, []byte("OggS")):
		if len(header) >= 36 && bytes.Equal(header[28:36], []byte("OpusHead")) {
			return "audio/ogg; codecs=opus"
		}
		return "audio/ogg"
	case bytes.HasPrefix(header, []byte("ID3")):
		return "audio/mpeg"
	case len(header) >= 2 && header[0] == 0xFF && header[1]&0xF6 == 0xF0:
		return "audio/aac"
	case len(header) >= 2 && header[0] == 0xFF && header[1]&0xE0 == 0xE0:
		return "audio/mpeg"
	case len(header) >= 8 && bytes.Equal(header[4:8], []byte("ftyp")):
		return "audio/mp4"
	case len(header) >= 12 && bytes.HasPrefix(header, []byte("RIFF")) && bytes.Equal(header[8:12], []byte("WAVE")):
		return "audio/wav"
	case bytes.HasPrefix(header, []byte("#!AMR")):
		return "audio/amr"
	}
	switch strings.ToLower(filepath.Ext(filename)) {
	case ".opus":
		return "audio/ogg; codecs=opus"
	case ".ogg":
		return "audio/ogg"
	case ".mp3":
		return "audio/mpeg"
	case ".aac":
		return "audio/aac"
	case ".m4a":
		return "audio/mp4"
	case ".wav":
		return "audio/wav"
	case ".amr":
		return "audio/amr"
	}
	return mimetype
}

// synthesizeMediaFilename builds a WhatsApp-style display filename ("WhatsApp
// Image 2026-08-14-150405.jpg") for media that never carries a real filename
// from WhatsApp itself: images, videos, voice notes, audio, and stickers
// only get a real one when they're actually a DocumentMessage. Used whenever
// a file is handed to something outside this app (opened, saved, copied to
// the clipboard) so it gets a sensible name instead of an internal id.
func synthesizeMediaFilename(mediaType, mimetype string, timestamp int64, id string) string {
	ext := ""
	switch {
	case strings.Contains(mimetype, "jpeg") || strings.Contains(mimetype, "jpg"):
		ext = ".jpg"
	case strings.Contains(mimetype, "png"):
		ext = ".png"
	case strings.Contains(mimetype, "webp"):
		ext = ".webp"
	case strings.Contains(mimetype, "mp4"):
		ext = ".mp4"
	case strings.Contains(mimetype, "ogg") || mediaType == "voice":
		ext = ".ogg"
	case strings.Contains(mimetype, "mp3") || strings.Contains(mimetype, "mpeg") || mediaType == "audio":
		ext = ".mp3"
	case strings.Contains(mimetype, "pdf"):
		ext = ".pdf"
	}
	if ext == "" {
		ext = extForMime(mimetype)
	}

	prefix := "WhatsApp File"
	switch mediaType {
	case "image":
		prefix = "WhatsApp Image"
	case "video":
		prefix = "WhatsApp Video"
	case "voice":
		prefix = "WhatsApp PTT"
	case "audio":
		prefix = "WhatsApp Audio"
	}

	if timestamp > 0 {
		t := time.Unix(timestamp, 0)
		return fmt.Sprintf("%s %s%s", prefix, t.Format("2006-01-02-150405"), ext)
	}
	return fmt.Sprintf("%s-%s%s", prefix, id, ext)
}

func uniqueFileForDir(dir, filename string) string {
	base := safeFilenameForFS(filename)
	ext := filepath.Ext(base)
	stem := strings.TrimSuffix(base, ext)
	candidate := filepath.Join(dir, base)
	for i := 1; ; i++ {
		if _, statErr := os.Stat(candidate); statErr != nil {
			return candidate
		}
		candidate = filepath.Join(dir, fmt.Sprintf("%s (%d)%s", stem, i, ext))
	}
}

func openMediaFileWithRealName(conn net.Conn, messageID, path, mediaType, mimetype, filename string) {
	viewOnce := viewOnceState(messageID)
	if viewOnce == viewOnceOpened {
		if conn != nil {
			sendEvent(conn, "error", map[string]string{"message": "You've already opened this view once message."})
		}
		return
	}
	if filename == "" {
		var ts int64
		db.QueryRow("SELECT COALESCE(timestamp, 0) FROM messages WHERE id = ?", messageID).Scan(&ts)
		filename = synthesizeMediaFilename(mediaType, mimetype, ts, messageID)
	}

	openPath := path
	if absPath, absErr := filepath.Abs(path); absErr == nil {
		openPath = absPath
	}

	tempDir := filepath.Join(os.TempDir(), "whantv_open")
	os.MkdirAll(tempDir, 0755)
	tempPath := filepath.Join(tempDir, safeFilenameForFS(filename))
	if copyFileStreaming(path, tempPath) == nil {
		openPath = tempPath
	}

	if err := openFileInOS(openPath); err != nil && conn != nil {
		sendEvent(conn, "error", map[string]string{"message": err.Error()})
		return
	}
	if viewOnce == viewOnceUnopened {
		scratch := ""
		if openPath != path {
			scratch = openPath
		}
		consumeViewOnce(messageID, path, scratch)
	}
}

func attachmentDisplayText(mediaType, filename, caption string, seconds int) string {
	tag := "[Document]"
	switch mediaType {
	case "audio":
		if seconds > 0 {
			return fmt.Sprintf("[Audio] (%d:%02d)", seconds/60, seconds%60)
		}
		return "[Audio]"
	case "image":
		tag = "[Image]"
	case "video":
		tag = "[Video]"
	case "sticker":
		return "[Sticker]"
	}

	if mediaType == "image" || mediaType == "video" {
		if caption != "" {
			return tag + " " + caption
		}
		return tag
	}

	if filename != "" {
		if caption != "" {
			return tag + " " + filename + ": " + caption
		}
		return tag + " " + filename
	}
	if caption != "" {
		return tag + " " + caption
	}
	return tag
}

func handleSendAttachment(conn net.Conn, raw json.RawMessage) {
	var data SendAttachmentData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		sendEvent(conn, "error", map[string]string{"message": "invalid jid"})
		return
	}
	go func() {
		id := wa().GenerateMessageID()

		// Resolve to a real local file up front, including decoding+caching
		// base64 (e.g. clipboard-pasted) payloads immediately. That way the
		// pending/failed row always has a path to retry from, and we don't
		// end up sending a file we then have no local cache of.
		localPath := data.Path
		if localPath == "" {
			fileBytes, decErr := readLocalFileOrDecode("", data.Data)
			if decErr != nil {
				sendEvent(conn, "error", map[string]string{"message": decErr.Error()})
				return
			}
			mimeGuess := data.Mimetype
			if mimeGuess == "" || mimeGuess == "application/octet-stream" || mimeGuess == "application/zip" {
				mimeGuess = detectMimeType(data.Filename, "", fileBytes)
			}
			cached, saveErr := saveMediaBytes("outgoing", id, mimeGuess, fileBytes)
			if saveErr != nil {
				sendEvent(conn, "error", map[string]string{"message": saveErr.Error()})
				return
			}
			localPath = cached
		}
		if data.GifPlayback && isInTempDir(localPath) {
			cached := filepath.Join(mediaBaseDir(), "outgoing", sanitizeFilenameComponent(id)+".mp4")
			if copyFileStreaming(localPath, cached) == nil {
				os.Remove(localPath)
				localPath = cached
			}
		}

		ctxInfo, quotedID, quotedText, quotedSender := buildAttachmentContext(data.ReplyToID, data.ReplySender)

		senderJID := ""
		if wa().Store.ID != nil {
			senderJID = wa().Store.ID.ToNonAD().String()
		}
		ts := time.Now().Unix()

		viewOnce := data.ViewOnce && (data.Type == "image" || data.Type == "video") && jid.Server != types.NewsletterServer
		displayText := attachmentDisplayText(data.Type, data.Filename, data.Caption, data.Duration)
		if data.Type == "sticker" && data.Path != "" {
			st := &waE2E.StickerMessage{}
			labelStickerFromCache(st, data.Path)
			if text := stickerText(st); text != "[Sticker]" {
				displayText = text
			}
		}
		if viewOnce {
			displayText = viewOnceText(displayText)
		}
		saveMessage(StoredMessage{
			ID: id, JID: jid.String(), SenderJID: senderJID, FromMe: true,
			Text: displayText, Timestamp: ts, Status: "uploading",
			MediaType: data.Type, MediaMimetype: data.Mimetype, MediaFilename: data.Filename, MediaPath: localPath,
			QuotedID: quotedID, QuotedText: quotedText, QuotedSender: quotedSender,
		})
		if viewOnce {
			markViewOnce(db, id)
		}
		db.Exec(`INSERT INTO chats (jid, timestamp, preview_snippet, preview_sender, preview_from_me) VALUES (?, ?, ?, ?, 1)
			ON CONFLICT(jid) DO UPDATE SET
				timestamp=MAX(timestamp, excluded.timestamp),
				preview_snippet=excluded.preview_snippet,
				preview_sender=excluded.preview_sender,
				preview_from_me=1,
				preview_is_reaction=0,
				preview_reaction_emoji=''`, jid.String(), ts, displayText, senderJID)
		broadcastEvent("message", map[string]interface{}{
			"jid": jid.String(), "from": "me", "id": id, "text": displayText,
			"from_me":       true,
			"timestamp":     ts,
			"has_media":     true,
			"mediaType":     data.Type,
			"mediaFilename": data.Filename,
			"mediaPath":     localPath,
			"status":        "uploading",
			"quotedId":      quotedID,
			"quotedText":    quotedText,
			"quotedSender":  quotedSender,
		})
		if waveform := voiceWaveformBytes(data.Waveform); data.Type == "audio" && waveform != nil {
			db.Exec(`UPDATE messages SET voice_waveform = ? WHERE id = ?`, waveform, id)
		}
		broadcastChats()

		thumbPNG, _ := base64.StdEncoding.DecodeString(data.Thumbnail)
		sendUploadedAttachment(conn, id, jid, data.Type, localPath, data.Mimetype, data.Filename, data.Caption, data.Duration, thumbPNG, data.GifPlayback, ctxInfo, quotedID, quotedText, quotedSender)
	}()
}

func sendUploadedAttachment(conn net.Conn, id string, jid types.JID, mediaType, path, mimetypeHint, filename, caption string, seconds int, thumbPNG []byte, gifPlayback bool, ctxInfo *waE2E.ContextInfo, quotedID, quotedText, quotedSender string) {
	displayText := attachmentDisplayText(mediaType, filename, caption, seconds)
	viewOnce := (mediaType == "image" || mediaType == "video") && viewOnceState(id) == viewOnceUnopened
	if viewOnce {
		displayText = viewOnceText(displayText)
	}
	uploadCtx, cancelUpload := context.WithCancel(context.Background())
	activeUploadsMu.Lock()
	activeUploads[id] = cancelUpload
	activeUploadsMu.Unlock()
	defer func() {
		activeUploadsMu.Lock()
		delete(activeUploads, id)
		activeUploadsMu.Unlock()
		cancelUpload()
	}()
	fail := func(errMsg string) {
		if uploadCtx.Err() != nil {
			// The user cancelled: the message goes away rather than sitting
			// in the list as a failed send.
			deleteMessageForMe(id, jid.String())
			if conn != nil {
				sendEvent(conn, "attachment_upload_ended", map[string]interface{}{"jid": jid.String(), "filename": filename, "id": id})
			}
			broadcastEvent("message_deleted", map[string]interface{}{
				"jid": jid.String(), "message_id": id, "by": "me", "for_everyone": false, "upload_cancelled": true,
			})
			return
		}
		db.Exec(`UPDATE messages SET status = 'failed' WHERE id = ?`, id)
		if conn != nil {
			sendEvent(conn, "attachment_upload_ended", map[string]interface{}{"jid": jid.String(), "filename": filename, "id": id})
			sendEvent(conn, "error", map[string]string{"message": errMsg})
		}
		broadcastEvent("message", map[string]interface{}{
			"jid": jid.String(), "from": "me", "id": id, "text": displayText,
			"from_me": true, "timestamp": time.Now().Unix(), "has_media": true,
			"mediaType": mediaType, "mediaFilename": filename, "mediaPath": path, "status": "failed",
			"quotedId": quotedID, "quotedText": quotedText, "quotedSender": quotedSender,
		})
	}

	var waMediaType whatsmeow.MediaType
	switch mediaType {
	case "image", "sticker":
		waMediaType = whatsmeow.MediaImage
	case "video":
		waMediaType = whatsmeow.MediaVideo
	case "audio":
		waMediaType = whatsmeow.MediaAudio
	default:
		waMediaType = whatsmeow.MediaDocument
	}

	var photo *preparedPhoto
	if mediaType == "image" {
		if p, err := preparePhoto(id, path, appPref("hd_media")); err == nil {
			photo = p
			if p.path != path {
				path, mimetypeHint = p.path, "image/jpeg"
			}
		} else {
			log.Printf("Sending photo %s unprepared: %v", id, err)
		}
	}

	// A video carries its length, size and a preview (the first frame, which
	// the frontend decodes). An H.264 .mov goes as an .mp4, as WhatsApp Web
	// sends it; that's done to a copy, never the picked file itself.
	var video *videoInfo
	var videoThumb []byte
	if mediaType == "video" {
		if info, err := readVideoInfo(path); err == nil {
			video = info
			if info.brand == "qt  " && (info.codec == "avc1" || info.codec == "avc3") {
				copyPath := filepath.Join(mediaBaseDir(), "outgoing", sanitizeFilenameComponent(id)+".mp4")
				if copyFileStreaming(path, copyPath) == nil && mp4Brand(copyPath) == nil {
					path, mimetypeHint = copyPath, "video/mp4"
				}
			}
		} else {
			log.Printf("Sending video %s without its details: %v", id, err)
		}
		if img, _, err := image.Decode(bytes.NewReader(thumbPNG)); err == nil {
			videoThumb = jpegThumbnail(img)
		}
	}

	f, err := os.Open(path)
	if err != nil {
		fail(err.Error())
		return
	}
	defer f.Close()

	stat, _ := f.Stat()
	totalSize := int64(0)
	if stat != nil {
		totalSize = stat.Size()
	}

	if limit := attachmentSizeLimit(mediaType); totalSize > limit {
		fail(attachmentTooLargeText(mediaType, totalSize, limit))
		return
	}

	header := make([]byte, 512)
	n, _ := f.Read(header)
	f.Seek(0, 0)

	mimetype := mimetypeHint
	if mimetype == "" || mimetype == "application/octet-stream" || mimetype == "application/zip" {
		mimetype = detectMimeType(filename, path, header[:n])
	}
	if mediaType == "audio" {
		mimetype = audioMimetype(mimetype, filename, header[:n])
		if !phonePlayableAudio(mimetype) {
			mediaType, waMediaType = "document", whatsmeow.MediaDocument
			displayText = attachmentDisplayText(mediaType, filename, caption, seconds)
			db.Exec(`UPDATE messages SET media_type = ? WHERE id = ?`, mediaType, id)
		}
	}

	if conn != nil {
		sendEvent(conn, "attachment_uploading", map[string]interface{}{"id": id, "jid": jid.String(), "filename": filename, "status": "uploading", "percent": 0})
	}

	isChannel := jid.Server == types.NewsletterServer
	var encrypted *os.File
	if !isChannel {
		encrypted, err = os.CreateTemp("", "whantv-upload-*")
		if err != nil {
			fail(err.Error())
			return
		}
		defer func() {
			encrypted.Close()
			os.Remove(encrypted.Name())
		}()
	}

	var pr io.Reader = f
	var uploadFile io.ReadWriteSeeker = encrypted
	if totalSize > 0 {
		lastPct := -1
		source, uploadSize := f, totalSize
		if !isChannel {
			source, uploadSize = encrypted, totalSize+16-totalSize%16+10
		}
		progress := &ProgressReader{
			reader: source,
			total:  uploadSize,
			onProgress: func(pct int) {
				if pct != lastPct {
					lastPct = pct
					if conn != nil {
						sendEvent(conn, "attachment_uploading", map[string]interface{}{
							"id":       id,
							"jid":      jid.String(),
							"filename": filename,
							"percent":  pct,
							"status":   "uploading",
						})
					}
				}
			},
		}
		if isChannel {
			pr = &channelUploadReader{File: f, progress: progress}
		} else {
			uploadFile = &encryptedUploadFile{File: encrypted, progress: progress}
		}
	}

	uploadStarted := time.Now()
	var uploaded whatsmeow.UploadResponse
	if isChannel {
		seeker, ok := pr.(io.ReadSeeker)
		if !ok {
			seeker = f
		}
		uploaded, err = wa().UploadNewsletterReader(uploadCtx, seeker, waMediaType)
	} else {
		uploaded, err = wa().UploadReader(uploadCtx, f, uploadFile, waMediaType)
	}
	uploadTook := time.Since(uploadStarted)
	if err != nil {
		log.Printf("Attachment upload of %d bytes failed after %s: %v", totalSize, uploadTook.Round(time.Millisecond), err)
		fail(err.Error())
		return
	}
	log.Printf("Attachment upload: %d bytes in %s (%.0f KB/s)", totalSize, uploadTook.Round(time.Millisecond),
		float64(totalSize)/1024/max(uploadTook.Seconds(), 0.001))

	var msg *waE2E.Message
	switch mediaType {
	case "image":
		imgMsg := &waE2E.ImageMessage{
			Mimetype: proto.String(mimetype),
			URL:      proto.String(uploaded.URL), DirectPath: proto.String(uploaded.DirectPath),
			MediaKey: uploaded.MediaKey, FileEncSHA256: uploaded.FileEncSHA256, FileSHA256: uploaded.FileSHA256,
			FileLength: proto.Uint64(uploaded.FileLength), ContextInfo: ctxInfo,
		}
		if photo != nil {
			imgMsg.Width, imgMsg.Height = proto.Uint32(uint32(photo.width)), proto.Uint32(uint32(photo.height))
			if !viewOnce {
				imgMsg.JPEGThumbnail = photo.thumbnail
			}
		}
		if caption != "" {
			imgMsg.Caption = proto.String(caption)
		}
		if viewOnce {
			imgMsg.ViewOnce = proto.Bool(true)
		}
		msg = &waE2E.Message{ImageMessage: imgMsg}
	case "video":
		vidMsg := &waE2E.VideoMessage{
			Mimetype: proto.String(mimetype),
			URL:      proto.String(uploaded.URL), DirectPath: proto.String(uploaded.DirectPath),
			MediaKey: uploaded.MediaKey, FileEncSHA256: uploaded.FileEncSHA256, FileSHA256: uploaded.FileSHA256,
			FileLength: proto.Uint64(uploaded.FileLength), ContextInfo: ctxInfo,
		}
		if video != nil {
			vidMsg.Seconds = proto.Uint32(uint32(video.seconds))
			vidMsg.Width, vidMsg.Height = proto.Uint32(uint32(video.width)), proto.Uint32(uint32(video.height))
		}
		if videoThumb != nil && !viewOnce {
			vidMsg.JPEGThumbnail = videoThumb
		}
		if gifPlayback {
			vidMsg.GifPlayback = proto.Bool(true)
		}
		if caption != "" {
			vidMsg.Caption = proto.String(caption)
		}
		if viewOnce {
			vidMsg.ViewOnce = proto.Bool(true)
		}
		msg = &waE2E.Message{VideoMessage: vidMsg}
	case "audio":
		var waveform []byte
		db.QueryRow(`SELECT voice_waveform FROM messages WHERE id = ?`, id).Scan(&waveform)
		msg = &waE2E.Message{AudioMessage: &waE2E.AudioMessage{
			Waveform: waveform,
			Mimetype: proto.String(mimetype), PTT: proto.Bool(false),
			Seconds: proto.Uint32(uint32(max(seconds, 0))),
			URL:     proto.String(uploaded.URL), DirectPath: proto.String(uploaded.DirectPath),
			MediaKey: uploaded.MediaKey, FileEncSHA256: uploaded.FileEncSHA256, FileSHA256: uploaded.FileSHA256,
			FileLength: proto.Uint64(uploaded.FileLength), ContextInfo: ctxInfo,
		}}
	case "sticker":
		st := &waE2E.StickerMessage{
			Mimetype: proto.String(mimetype),
			URL:      proto.String(uploaded.URL), DirectPath: proto.String(uploaded.DirectPath),
			MediaKey: uploaded.MediaKey, FileEncSHA256: uploaded.FileEncSHA256, FileSHA256: uploaded.FileSHA256,
			FileLength: proto.Uint64(uploaded.FileLength), ContextInfo: ctxInfo,
		}
		if fileBytes, err := os.ReadFile(path); err == nil {
			width, height, animated := webpInfo(fileBytes)
			if width > 0 {
				st.Width, st.Height = proto.Uint32(uint32(width)), proto.Uint32(uint32(height))
			}
			st.IsAnimated = proto.Bool(animated)
			labelStickerFromFile(st, fileBytes)
		}
		if text := stickerText(st); text != "[Sticker]" {
			displayText = text
		}
		msg = &waE2E.Message{StickerMessage: st}
	default:
		docMsg := &waE2E.DocumentMessage{
			Title: proto.String(filename), FileName: proto.String(filename), Mimetype: proto.String(mimetype),
			URL: proto.String(uploaded.URL), DirectPath: proto.String(uploaded.DirectPath),
			MediaKey: uploaded.MediaKey, FileEncSHA256: uploaded.FileEncSHA256, FileSHA256: uploaded.FileSHA256,
			FileLength: proto.Uint64(uploaded.FileLength), ContextInfo: ctxInfo,
		}
		if caption != "" {
			docMsg.Caption = proto.String(caption)
		}
		msg = &waE2E.Message{DocumentMessage: docMsg}
	}
	stampUploadedMedia(msg, uploaded)

	msg = applyDisappearingTimer(jid, msg)
	if viewOnce {
		msg = &waE2E.Message{ViewOnceMessage: &waE2E.FutureProofMessage{Message: msg}}
	}
	resp, err := wa().SendMessage(context.Background(), jid, msg, whatsmeow.SendRequestExtra{ID: id, MediaHandle: uploaded.Handle, CheckPermissions: true})
	if err != nil {
		fail(sendErrorText(err))
		return
	}

	rawBytes, _ := proto.Marshal(msg)
	ts := resp.Timestamp.Unix()
	db.Exec(`UPDATE messages SET status='sent', text=?, timestamp=?, sent_at=?, raw=?, media_path=?, media_mimetype=? WHERE id=?`,
		displayText, ts, ts, rawBytes, path, mimetype, id)
	noteMessageExpiry(db, id, ts, msg)
	noteChannelPostSent(jid, id, resp)

	broadcastEvent("message", map[string]interface{}{
		"jid": jid.String(), "from": "me", "id": id, "text": displayText,
		"from_me":       true,
		"timestamp":     ts,
		"has_media":     true,
		"mediaType":     mediaType,
		"mediaFilename": filename,
		"mediaPath":     path,
		"status":        "sent",
		"quotedId":      quotedID,
		"quotedText":    quotedText,
		"quotedSender":  quotedSender,
	})
	broadcastChats()
	if conn != nil {
		sendEvent(conn, "attachment_sent", map[string]interface{}{"id": id, "jid": jid.String(), "filename": filename, "status": "sent"})
	}
}

func handleSendVoiceMessage(conn net.Conn, raw json.RawMessage) {
	var data SendVoiceMessageData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		sendEvent(conn, "error", map[string]string{"message": "invalid jid"})
		return
	}
	go func() {
		id := wa().GenerateMessageID()

		fileBytes, err := readLocalFileOrDecode(data.Path, data.Data)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}
		mimetype := data.Mimetype
		if mimetype == "" {
			mimetype = "audio/ogg; codecs=opus"
		}
		localPath := data.Path
		if cachedPath, cacheErr := saveMediaBytes("outgoing", id, mimetype, fileBytes); cacheErr == nil {
			localPath = cachedPath
			if data.Path != "" && isInTempDir(data.Path) {
				os.Remove(data.Path)
			}
		}

		ctxInfo, quotedID, quotedText, quotedSender := buildAttachmentContext(data.ReplyToID, data.ReplySender)

		senderJID := ""
		if wa().Store.ID != nil {
			senderJID = wa().Store.ID.ToNonAD().String()
		}
		voiceText := fmt.Sprintf("[Voice Message] (%d:%02d)", data.Duration/60, data.Duration%60)
		viewOnce := data.ViewOnce && jid.Server != types.NewsletterServer
		if viewOnce {
			voiceText = viewOnceText(voiceText)
		}
		ts := time.Now().Unix()

		saveMessage(StoredMessage{
			ID: id, JID: jid.String(), SenderJID: senderJID, FromMe: true,
			Text: voiceText, Timestamp: ts, Status: "uploading",
			MediaType: "voice", MediaMimetype: mimetype, MediaPath: localPath,
			QuotedID: quotedID, QuotedText: quotedText, QuotedSender: quotedSender,
		})
		if viewOnce {
			markViewOnce(db, id)
		}
		db.Exec(`INSERT INTO chats (jid, timestamp, preview_snippet, preview_sender, preview_from_me) VALUES (?, ?, ?, ?, 1)
			ON CONFLICT(jid) DO UPDATE SET
				timestamp=MAX(timestamp, excluded.timestamp),
				preview_snippet=excluded.preview_snippet,
				preview_sender=excluded.preview_sender,
				preview_from_me=1,
				preview_is_reaction=0,
				preview_reaction_emoji=''`, jid.String(), ts, voiceText, senderJID)
		broadcastEvent("message", map[string]interface{}{
			"jid": jid.String(), "from": "me", "id": id, "text": voiceText,
			"from_me":      true,
			"timestamp":    ts,
			"has_media":    true,
			"mediaType":    "voice",
			"status":       "uploading",
			"quotedId":     quotedID,
			"quotedText":   quotedText,
			"quotedSender": quotedSender,
		})
		waveform := voiceWaveformBytes(data.Waveform)
		if waveform != nil {
			db.Exec(`UPDATE messages SET voice_waveform = ? WHERE id = ?`, waveform, id)
		}
		broadcastChats()

		sendUploadedVoiceMessage(conn, id, jid, localPath, mimetype, data.Duration, waveform, ctxInfo, quotedID, quotedText, quotedSender)
	}()
}

// voiceWaveformBytes turns the frontend's measured waveform into the
// AudioMessage field: 64 bars of 0-100, as WhatsApp's own clients send.
// Anything else yields nil and the message goes without one - receivers then
// draw a plain seek bar, rather than bars that don't match the audio (and a
// fixed pattern would be identical on every voice note we ever sent).
func voiceWaveformBytes(bars []int) []byte {
	if len(bars) != 64 {
		return nil
	}
	out := make([]byte, len(bars))
	for i, v := range bars {
		out[i] = byte(min(max(v, 0), 100))
	}
	return out
}

func sendUploadedVoiceMessage(conn net.Conn, id string, jid types.JID, path, mimetypeHint string, duration int, waveform []byte, ctxInfo *waE2E.ContextInfo, quotedID, quotedText, quotedSender string) {
	voiceText := fmt.Sprintf("[Voice Message] (%d:%02d)", duration/60, duration%60)
	viewOnce := viewOnceState(id) == viewOnceUnopened
	if viewOnce {
		voiceText = viewOnceText(voiceText)
	}

	fail := func(errMsg string) {
		db.Exec(`UPDATE messages SET status = 'failed' WHERE id = ?`, id)
		if conn != nil {
			sendEvent(conn, "error", map[string]string{"message": errMsg})
		}
		broadcastEvent("message", map[string]interface{}{
			"jid": jid.String(), "from": "me", "id": id, "text": voiceText,
			"from_me": true, "timestamp": time.Now().Unix(), "has_media": true,
			"mediaType": "voice", "status": "failed",
			"quotedId": quotedID, "quotedText": quotedText, "quotedSender": quotedSender,
		})
	}

	f, err := os.Open(path)
	if err != nil {
		fail(err.Error())
		return
	}
	defer f.Close()
	mimetype := mimetypeHint
	if mimetype == "" {
		mimetype = "audio/ogg; codecs=opus"
	}

	var uploaded whatsmeow.UploadResponse
	if jid.Server == types.NewsletterServer {
		uploaded, err = wa().UploadNewsletterReader(context.Background(), f, whatsmeow.MediaAudio)
	} else {
		uploaded, err = wa().UploadReader(context.Background(), f, nil, whatsmeow.MediaAudio)
	}
	if err != nil {
		fail(err.Error())
		return
	}

	msg := &waE2E.Message{AudioMessage: &waE2E.AudioMessage{
		Mimetype: proto.String(mimetype), PTT: proto.Bool(true),
		Seconds: proto.Uint32(uint32(duration)),
		URL:     proto.String(uploaded.URL), DirectPath: proto.String(uploaded.DirectPath),
		MediaKey: uploaded.MediaKey, FileEncSHA256: uploaded.FileEncSHA256, FileSHA256: uploaded.FileSHA256,
		FileLength:  proto.Uint64(uploaded.FileLength),
		Waveform:    waveform,
		ContextInfo: ctxInfo,
	}}
	if viewOnce {
		msg.AudioMessage.ViewOnce = proto.Bool(true)
	}
	stampUploadedMedia(msg, uploaded)

	msg = applyDisappearingTimer(jid, msg)
	if viewOnce {
		msg = &waE2E.Message{ViewOnceMessageV2Extension: &waE2E.FutureProofMessage{Message: msg}}
	}
	resp, err := wa().SendMessage(context.Background(), jid, msg, whatsmeow.SendRequestExtra{ID: id, MediaHandle: uploaded.Handle, CheckPermissions: true})
	if err != nil {
		fail(sendErrorText(err))
		return
	}

	rawBytes, _ := proto.Marshal(msg)
	ts := resp.Timestamp.Unix()
	db.Exec(`UPDATE messages SET status='sent', timestamp=?, sent_at=?, raw=?, media_path=?, media_mimetype=? WHERE id=?`,
		ts, ts, rawBytes, path, mimetype, id)
	noteMessageExpiry(db, id, ts, msg)
	noteChannelPostSent(jid, id, resp)

	broadcastEvent("message", map[string]interface{}{
		"jid": jid.String(), "from": "me", "id": id, "text": voiceText,
		"from_me":      true,
		"timestamp":    ts,
		"has_media":    true,
		"mediaType":    "voice",
		"mediaPath":    path,
		"status":       "sent",
		"quotedId":     quotedID,
		"quotedText":   quotedText,
		"quotedSender": quotedSender,
	})
	broadcastChats()
}

func handlePlayVoiceMessage(conn net.Conn, raw json.RawMessage) {
	var data struct {
		ID string `json:"id"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	if viewOnceState(data.ID) == viewOnceOpened {
		sendEvent(conn, "error", map[string]string{"message": "You've already played this view once voice message."})
		return
	}
	go func() {
		_, jidStr, senderJidStr, fromMe, dbErr := getMessageRaw(data.ID)
		// Playing back our own voice note mustn't tell the other side we
		// "played" it: that receipt is for the recipient's copy, and sending
		// it for a message we sent claims a play that never happened.
		if dbErr == nil && !fromMe {
			chatJID, _ := types.ParseJID(jidStr)
			senderJID, _ := types.ParseJID(senderJidStr)
			if senderJID.IsEmpty() {
				senderJID = chatJID
			}
			if chatJID.Server == "lid" {
				if pn, err := wa().Store.LIDs.GetPNForLID(context.Background(), chatJID); err == nil && !pn.IsEmpty() {
					chatJID = pn
				}
			}
			if senderJID.Server == "lid" {
				if pn, err := wa().Store.LIDs.GetPNForLID(context.Background(), senderJID); err == nil && !pn.IsEmpty() {
					senderJID = pn
				}
			}

			now := time.Now()
			if res, err := db.Exec(`UPDATE messages SET played_at = ? WHERE id = ? AND COALESCE(played_at, 0) = 0`, now.Unix(), data.ID); err == nil {
				if n, _ := res.RowsAffected(); n == 1 {
					go func() {
						ensureAvailableForRead()
						err := wa().MarkRead(context.Background(), []types.MessageID{types.MessageID(data.ID)}, now, chatJID, senderJID, types.ReceiptTypePlayed)
						if err != nil {
							log.Printf("MarkRead played error for message %s: %v", data.ID, err)
							db.Exec(`UPDATE messages SET played_at = 0 WHERE id = ? AND played_at = ?`, data.ID, now.Unix())
						}
					}()
				}
			}
		}

		path, _, _, _, err := ensureMediaCached(data.ID)
		if err != nil {
			if strings.Contains(err.Error(), "URL expired") {
				pendingAutoOpens.Store(data.ID, map[string]interface{}{
					"conn":   conn,
					"action": "play_audio",
				})
				return
			}
			fmt.Println("Error downloading audio:", err)
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}
		sendEvent(conn, "play_audio", map[string]interface{}{"path": path, "message_id": data.ID})
		if viewOnceState(data.ID) == viewOnceUnopened {
			consumeViewOnce(data.ID, "", path)
		}
	}()
}

type CopyAlbumToClipboardData struct {
	MessageIDs []string `json:"message_ids"`
}

func handleCopyAlbumToClipboard(conn net.Conn, raw json.RawMessage) {
	var data CopyAlbumToClipboardData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		tempDir := filepath.Join(os.TempDir(), "whantv_clipboard")
		os.MkdirAll(tempDir, 0755)

		var paths []string
		for _, msgID := range data.MessageIDs {
			if msg := viewOnceBlocked(msgID); msg != "" {
				sendEvent(conn, "error", map[string]string{"message": msg})
				continue
			}
			path, mediaType, mimetype, filename, err := ensureMediaCached(msgID)
			if err != nil {
				continue
			}

			destFilename := filename
			if destFilename == "" {
				var ts int64
				db.QueryRow("SELECT COALESCE(timestamp, 0) FROM messages WHERE id = ?", msgID).Scan(&ts)
				destFilename = synthesizeMediaFilename(mediaType, mimetype, ts, msgID)
			}

			tempPath := uniqueFileForDir(tempDir, destFilename)
			if copyFileStreaming(path, tempPath) == nil {
				paths = append(paths, tempPath)
			}
		}

		if len(paths) > 0 {
			var psPaths []string
			for _, p := range paths {
				psPaths = append(psPaths, fmt.Sprintf("'%s'", strings.ReplaceAll(p, "'", "''")))
			}
			psPathsStr := strings.Join(psPaths, ",")
			script := fmt.Sprintf("Add-Type -AssemblyName System.Windows.Forms; $files = @(%s); $sc = New-Object System.Collections.Specialized.StringCollection; foreach ($f in $files) { $sc.Add($f) | Out-Null }; [System.Windows.Forms.Clipboard]::SetFileDropList($sc)", psPathsStr)

			cmd := exec.Command("powershell", "-NoProfile", "-Command", script)
			if err := cmd.Run(); err == nil {
				sendEvent(conn, "copied_album_to_clipboard", map[string]interface{}{
					"message_ids": data.MessageIDs,
					"count":       len(paths),
				})
			} else {
				sendEvent(conn, "error", map[string]string{"message": err.Error()})
			}
		}
	}()
}

func requestMediaRetry(messageID string) error {
	rawBytes, jidStr, senderJidStr, fromMe, dbErr := getMessageRaw(messageID)
	if dbErr != nil {
		return dbErr
	}
	msg := &waE2E.Message{}
	if err := proto.Unmarshal(rawBytes, msg); err != nil {
		return err
	}
	chatJID, _ := types.ParseJID(jidStr)
	senderJID, _ := types.ParseJID(senderJidStr)
	if senderJID.IsEmpty() {
		senderJID = chatJID
	}

	if chatJID.Server == "lid" {
		if pn, err := wa().Store.LIDs.GetPNForLID(context.Background(), chatJID); err == nil && !pn.IsEmpty() {
			chatJID = pn
		}
	}
	if senderJID.Server == "lid" {
		if pn, err := wa().Store.LIDs.GetPNForLID(context.Background(), senderJID); err == nil && !pn.IsEmpty() {
			senderJID = pn
		}
	}

	msgInfo := &types.MessageInfo{
		ID: types.MessageID(messageID),
		MessageSource: types.MessageSource{
			Chat:     chatJID,
			Sender:   senderJID,
			IsFromMe: fromMe,
			IsGroup:  chatJID.Server == "g.us",
		},
	}
	media := mediaCarrier(msg)
	var mediaKey []byte
	switch {
	case media.GetImageMessage() != nil:
		mediaKey = media.GetImageMessage().GetMediaKey()
	case media.GetVideoMessage() != nil:
		mediaKey = media.GetVideoMessage().GetMediaKey()
	case media.GetAudioMessage() != nil:
		mediaKey = media.GetAudioMessage().GetMediaKey()
	case media.GetDocumentMessage() != nil:
		mediaKey = media.GetDocumentMessage().GetMediaKey()
	case media.GetStickerMessage() != nil:
		mediaKey = media.GetStickerMessage().GetMediaKey()
	}
	if mediaKey == nil {
		return fmt.Errorf("no media key found")
	}
	return wa().SendMediaRetryReceipt(context.Background(), msgInfo, mediaKey)
}

func handleMediaRetryEvent(evt *events.MediaRetry) {
	rawBytes, jidStr, _, _, dbErr := getMessageRaw(string(evt.MessageID))
	if dbErr != nil {
		return
	}
	msg := &waE2E.Message{}
	proto.Unmarshal(rawBytes, msg)
	media := mediaCarrier(msg)
	var mediaKey []byte
	switch {
	case media.GetImageMessage() != nil:
		mediaKey = media.GetImageMessage().GetMediaKey()
	case media.GetVideoMessage() != nil:
		mediaKey = media.GetVideoMessage().GetMediaKey()
	case media.GetAudioMessage() != nil:
		mediaKey = media.GetAudioMessage().GetMediaKey()
	case media.GetDocumentMessage() != nil:
		mediaKey = media.GetDocumentMessage().GetMediaKey()
	case media.GetStickerMessage() != nil:
		mediaKey = media.GetStickerMessage().GetMediaKey()
	}
	if mediaKey == nil {
		return
	}
	retryData, err := whatsmeow.DecryptMediaRetryNotification(evt, mediaKey)
	if err != nil || retryData.GetResult() != waMmsRetry.MediaRetryNotification_SUCCESS {
		fmt.Printf("Failed to decrypt media retry or result not success for %s: %v\n", evt.MessageID, err)
		retryWaitersMu.Lock()
		if ch, ok := retryWaiters[string(evt.MessageID)]; ok {
			ch <- false
			delete(retryWaiters, string(evt.MessageID))
		} else {
			broadcastEvent("media_download_failed", map[string]interface{}{
				"message_id": string(evt.MessageID),
				"jid":        jidStr,
				"message":    "Phone could not re-upload the expired media",
			})
		}
		retryWaitersMu.Unlock()
		return
	}

	newPath := retryData.GetDirectPath()
	if newPath == "" {
		return
	}
	switch {
	case media.GetImageMessage() != nil:
		media.GetImageMessage().DirectPath = proto.String(newPath)
	case media.GetVideoMessage() != nil:
		media.GetVideoMessage().DirectPath = proto.String(newPath)
	case media.GetAudioMessage() != nil:
		media.GetAudioMessage().DirectPath = proto.String(newPath)
	case media.GetDocumentMessage() != nil:
		media.GetDocumentMessage().DirectPath = proto.String(newPath)
	case media.GetStickerMessage() != nil:
		media.GetStickerMessage().DirectPath = proto.String(newPath)
	}

	newRawBytes, _ := proto.Marshal(msg)
	db.Exec("UPDATE messages SET raw = ? WHERE id = ?", newRawBytes, string(evt.MessageID))

	retryWaitersMu.Lock()
	if ch, ok := retryWaiters[string(evt.MessageID)]; ok {
		ch <- true
		delete(retryWaiters, string(evt.MessageID))
		retryWaitersMu.Unlock()
		return
	}
	retryWaitersMu.Unlock()

	ctx, cancel := context.WithCancel(context.Background())
	activeDownloadsMu.Lock()
	activeDownloads[string(evt.MessageID)] = cancel
	activeDownloadsMu.Unlock()
	go func() {
		defer func() {
			activeDownloadsMu.Lock()
			delete(activeDownloads, string(evt.MessageID))
			activeDownloadsMu.Unlock()
			cancel()
		}()
		onProgress := func(percent int) {
			broadcastEvent("media_download_progress", map[string]interface{}{
				"message_id": string(evt.MessageID),
				"jid":        jidStr,
				"percent":    percent,
			})
		}
		path, mediaType, mimetype, filename, err := downloadMessageMediaToFile(ctx, string(evt.MessageID), onProgress)
		if err != nil {
			broadcastEvent("media_download_failed", map[string]interface{}{
				"message_id": string(evt.MessageID),
				"jid":        jidStr,
				"message":    friendlyMediaError(err),
			})
			if c, ok := pendingAutoOpens.Load(string(evt.MessageID)); ok {
				if data, ok := c.(map[string]interface{}); ok {
					if conn, ok := data["conn"].(net.Conn); ok {
						if data["action"] == "prepare_media" {
							sendEvent(conn, "media_failed", map[string]string{"message_id": string(evt.MessageID), "message": friendlyMediaError(err)})
						} else {
							sendEvent(conn, "error", map[string]string{"message": friendlyMediaError(err)})
						}
					}
				}
				pendingAutoOpens.Delete(string(evt.MessageID))
			}
			return
		}

		if c, ok := pendingAutoOpens.Load(string(evt.MessageID)); ok {
			data, dataOk := c.(map[string]interface{})
			conn, connOk := data["conn"].(net.Conn)
			action, _ := data["action"].(string)
			if dataOk {
				if action == "open_media" {
					openMediaFileWithRealName(conn, string(evt.MessageID), path, mediaType, mimetype, filename)
				} else if action == "prepare_media" && connOk {
					sendMediaReady(conn, string(evt.MessageID), path, mediaType, mimetype, filename)
				} else if action == "play_audio" && connOk {
					sendEvent(conn, "play_audio", map[string]interface{}{"path": path, "message_id": string(evt.MessageID)})
				}
			}
			pendingAutoOpens.Delete(string(evt.MessageID))
		}

		broadcastEvent("media_downloaded", map[string]interface{}{
			"message_id": string(evt.MessageID),
			"jid":        jidStr,
			"path":       path,
			"mediaType":  mediaType,
			"mimetype":   mimetype,
			"filename":   filename,
		})
	}()
}

// loadAutoDownloadFailed fills autoDownloadFailed from the database (called
// once, under activeDownloadsMu).
func loadAutoDownloadFailed() {
	db.Exec(`CREATE TABLE IF NOT EXISTS auto_download_failed (id TEXT PRIMARY KEY, failed_at INTEGER NOT NULL DEFAULT 0)`)
	rows, err := db.Query(`SELECT id FROM auto_download_failed`)
	if err != nil {
		return
	}
	defer rows.Close()
	for rows.Next() {
		var id string
		if rows.Scan(&id) == nil {
			autoDownloadFailed[id] = true
		}
	}
}
