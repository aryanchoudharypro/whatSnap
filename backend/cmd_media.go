package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"mime"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"time"

	"go.mau.fi/whatsmeow/proto/waMmsRetry"

	"net"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

// friendlyMediaError turns known-unrecoverable whatsmeow media errors into a
// message a non-technical user can act on. The ciphertext-hash mismatch in
// particular means the sender's device re-uploaded media that no longer
// matches the original encryption key/hash (e.g. it deleted the original
// file) — retrying again won't help.
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

// downloadMessageMedia loads a stored message's raw protobuf, figures out
// which kind of media it carries, and downloads+decrypts it via whatsmeow.
func downloadMessageMedia(messageID string) (data []byte, mediaType, mimetype, filename string, err error) {
	rawBytes, _, _, _, dbErr := getMessageRaw(messageID)
	if dbErr != nil {
		err = dbErr
		return
	}
	msg := &waE2E.Message{}
	if perr := proto.Unmarshal(rawBytes, msg); perr != nil {
		err = perr
		return
	}
	msg = mediaCarrier(msg)
	switch {
	case msg.GetImageMessage() != nil:
		m := msg.GetImageMessage()
		data, err = client.Download(context.Background(), m)
		return data, "image", m.GetMimetype(), "", err
	case msg.GetVideoMessage() != nil:
		m := msg.GetVideoMessage()
		data, err = client.Download(context.Background(), m)
		return data, "video", m.GetMimetype(), "", err
	case msg.GetAudioMessage() != nil:
		m := msg.GetAudioMessage()
		data, err = client.Download(context.Background(), m)
		mt := "audio"
		if m.GetPTT() {
			mt = "voice"
		}
		return data, mt, m.GetMimetype(), "", err
	case msg.GetDocumentMessage() != nil:
		m := msg.GetDocumentMessage()
		data, err = client.Download(context.Background(), m)
		fn := m.GetFileName()
		if fn == "" {
			fn = m.GetTitle()
		}
		return data, "document", m.GetMimetype(), fn, err
	case msg.GetStickerMessage() != nil:
		m := msg.GetStickerMessage()
		data, err = client.Download(context.Background(), m)
		return data, "sticker", m.GetMimetype(), "", err
	}
	err = fmt.Errorf("message has no downloadable media")
	return
}

// ensureMediaCached returns the local path for a message's media, downloading
// and caching it first if it isn't already on disk.
func ensureMediaCached(messageID string) (path, mediaType, mimetype, filename string, err error) {
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
	data, mt, mime, fn, dErr := downloadMessageMedia(messageID)
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
	path, err = saveMediaBytes("media", messageID, mime, data)
	if err != nil {
		return "", "", "", "", err
	}
	setMessageMediaPath(messageID, path)
	if fn != "" {
		db.Exec("UPDATE messages SET media_filename = ? WHERE id = ?", fn, messageID)
	}
	return path, mt, mime, fn, nil
}

var (
	activeDownloadsMu sync.Mutex
	activeDownloads   = map[string]context.CancelFunc{}
	retryWaitersMu    sync.Mutex
	retryWaiters      = map[string]chan bool{}
	pendingAutoOpens  sync.Map
)

// progressFile wraps the cache file handed to whatsmeow's DownloadToFile.
// The network phase writes sequentially via Write, while the decrypt phase
// uses Read/WriteAt, so counting only Write gives accurate download progress.
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

// downloadMessageMediaToFile streams a message's media straight into the
// local cache, reporting progress and honoring ctx cancellation.
func downloadMessageMediaToFile(ctx context.Context, messageID string, onProgress func(int)) (path, mediaType, mimetype, filename string, err error) {
	rawBytes, _, _, _, dbErr := getMessageRaw(messageID)
	if dbErr != nil {
		err = dbErr
		return
	}
	msg := &waE2E.Message{}
	if perr := proto.Unmarshal(rawBytes, msg); perr != nil {
		err = perr
		return
	}
	msg = mediaCarrier(msg)
	var dl whatsmeow.DownloadableMessage
	var total int64
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
	default:
		err = fmt.Errorf("message has no downloadable media")
		return
	}
	finalPath := filepath.Join(mediaDir(), sanitizeFilenameComponent(messageID)+extForMime(mimetype))
	partPath := finalPath + ".part"
	f, ferr := os.Create(partPath)
	if ferr != nil {
		err = ferr
		return
	}
	dlErr := client.DownloadToFile(ctx, dl, &progressFile{File: f, total: total, onProgress: onProgress})
	f.Close()
	if dlErr != nil {
		os.Remove(partPath)
		err = dlErr
		return
	}
	if rerr := os.Rename(partPath, finalPath); rerr != nil {
		os.Remove(partPath)
		err = rerr
		return
	}
	setMessageMediaPath(messageID, finalPath)
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
	var cached, mt, mm, fn string
	db.QueryRow("SELECT COALESCE(media_path,''), COALESCE(media_type,''), COALESCE(media_mimetype,''), COALESCE(media_filename,'') FROM messages WHERE id = ?", data.MessageID).
		Scan(&cached, &mt, &mm, &fn)
	if cached != "" {
		if _, statErr := os.Stat(cached); statErr == nil {
			broadcastEvent("media_downloaded", map[string]interface{}{
				"message_id": data.MessageID,
				"jid":        data.JID,
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
	ctx, cancel := context.WithCancel(context.Background())
	activeDownloads[data.MessageID] = cancel
	activeDownloadsMu.Unlock()
	go func() {
		defer func() {
			activeDownloadsMu.Lock()
			delete(activeDownloads, data.MessageID)
			activeDownloadsMu.Unlock()
			cancel()
		}()
		onProgress := func(percent int) {
			broadcastEvent("media_download_progress", map[string]interface{}{
				"message_id": data.MessageID,
				"jid":        data.JID,
				"percent":    percent,
			})
		}
		for attempt := 0; attempt < 2; attempt++ {
			path, mediaType, mimetype, filename, err := downloadMessageMediaToFile(ctx, data.MessageID, onProgress)
			if err == nil {
				broadcastEvent("media_downloaded", map[string]interface{}{
					"message_id": data.MessageID,
					"jid":        data.JID,
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
				})
				return
			}

			if attempt == 0 && (strings.Contains(err.Error(), "403") || strings.Contains(err.Error(), "404") || strings.Contains(err.Error(), "410")) {
				fmt.Printf("Requesting media retry for %s...\n", data.MessageID)
				retryErr := requestMediaRetry(data.MessageID)
				if retryErr != nil {
					fmt.Printf("Retry request failed: %v\n", retryErr)
					broadcastEvent("media_download_failed", map[string]interface{}{
						"message_id": data.MessageID,
						"jid":        data.JID,
						"message":    "URL expired and retry request failed: " + retryErr.Error(),
					})
					return
				}

				// Wait for retry response
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
							"message":    "Phone could not re-upload the media",
						})
						return
					}
					// Success! Loop again to download the new path
					continue
				case <-time.After(45 * time.Second):
					broadcastEvent("media_download_failed", map[string]interface{}{
						"message_id": data.MessageID,
						"jid":        data.JID,
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
					})
					retryWaitersMu.Lock()
					delete(retryWaiters, data.MessageID)
					retryWaitersMu.Unlock()
					return
				}
			}

			// Non-retryable error or second attempt failed
			broadcastEvent("media_download_failed", map[string]interface{}{
				"message_id": data.MessageID,
				"jid":        data.JID,
				"message":    friendlyMediaError(err),
			})
			return
		}
	}()
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
	var cmd *exec.Cmd
	switch runtime.GOOS {
	case "windows":
		cmd = exec.Command("cmd", "/c", "start", "", path)
	case "darwin":
		cmd = exec.Command("open", path)
	default:
		cmd = exec.Command("xdg-open", path)
	}
	return cmd.Start()
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

func handleSaveMediaAs(conn net.Conn, raw json.RawMessage) {
	var data SaveMediaAsData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		path, _, _, _, err := ensureMediaCached(data.MessageID)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": friendlyMediaError(err)})
			return
		}
		bytes, err := os.ReadFile(path)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}
		if err := os.WriteFile(data.DestPath, bytes, 0644); err != nil {
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}
		sendEvent(conn, "media_saved", map[string]interface{}{"message_id": data.MessageID, "path": data.DestPath})
	}()
}

// buildAttachmentContext resolves optional reply-to fields into a
// ContextInfo plus the local snapshot fields we store alongside the message.
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
	ctxInfo = &waE2E.ContextInfo{StanzaID: proto.String(replyToID), QuotedMessage: quotedMsg}
	if participant != "" {
		ctxInfo.Participant = proto.String(participant)
	}
	return ctxInfo, replyToID, preview, resolveQuotedSenderName(participant)
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

// uniqueFileForDir returns a path in dir for filename, disambiguating with
// " (1)", " (2)", ... before the extension if that name is already taken —
// the same trick Explorer/WhatsApp itself uses. This matters most for
// multi-item operations like copying an album to the clipboard: every photo
// in an album shares (or is a second apart on) the same timestamp, so
// without this every synthesized name would collide and only the last file
// written would actually end up in the clipboard.
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

// openMediaFileWithRealName copies a locally-cached media file to a
// friendly-named temp copy — so whatever app opens it (and its window
// title / "recent files" entry) shows something like "WhatsApp Image
// ....jpg" instead of the internal cache filename — and opens it with the
// OS default handler. filename may be empty (true for anything but a
// WhatsApp DocumentMessage), in which case a display name is synthesized.
func openMediaFileWithRealName(conn net.Conn, messageID, path, mediaType, mimetype, filename string) {
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
	if bytes, readErr := os.ReadFile(path); readErr == nil {
		if os.WriteFile(tempPath, bytes, 0644) == nil {
			openPath = tempPath
		}
	}

	if err := openFileInOS(openPath); err != nil && conn != nil {
		sendEvent(conn, "error", map[string]string{"message": err.Error()})
	}
}

// attachmentDisplayText mirrors extractText's formatting for the equivalent
// incoming message type, so an outgoing attachment's local echo (and the
// text persisted to the DB) shows the same "[Document] filename" style
// instead of just the caption (which is usually empty).
func attachmentDisplayText(mediaType, filename, caption string) string {
	tag := "[Document]"
	switch mediaType {
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
		id := client.GenerateMessageID()

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

		ctxInfo, quotedID, quotedText, quotedSender := buildAttachmentContext(data.ReplyToID, data.ReplySender)

		senderJID := ""
		if client.Store.ID != nil {
			senderJID = client.Store.ID.ToNonAD().String()
		}
		ts := time.Now().Unix()

		// Persist and broadcast as "uploading" immediately: this is the
		// local echo, and what makes a failed/offline upload recoverable
		// (marked "failed", retryable from the message context menu)
		// instead of just vanishing.
		displayText := attachmentDisplayText(data.Type, data.Filename, data.Caption)
		saveMessage(StoredMessage{
			ID: id, JID: jid.String(), SenderJID: senderJID, FromMe: true,
			Text: displayText, Timestamp: ts, Status: "uploading",
			MediaType: data.Type, MediaMimetype: data.Mimetype, MediaFilename: data.Filename, MediaPath: localPath,
			QuotedID: quotedID, QuotedText: quotedText, QuotedSender: quotedSender,
		})
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
			"status":        "uploading",
			"quotedId":      quotedID,
			"quotedText":    quotedText,
			"quotedSender":  quotedSender,
		})
		broadcastChats()

		sendUploadedAttachment(conn, id, jid, data.Type, localPath, data.Mimetype, data.Filename, data.Caption, ctxInfo, quotedID, quotedText, quotedSender)
	}()
}

// sendUploadedAttachment uploads the file at path and sends it as mediaType,
// updating the row already persisted by handleSendAttachment (or reused by
// handleRetryMessage) to "sent" on success or "failed" on error. Shared by
// the initial send and a manual retry so both behave identically.
func sendUploadedAttachment(conn net.Conn, id string, jid types.JID, mediaType, path, mimetypeHint, filename, caption string, ctxInfo *waE2E.ContextInfo, quotedID, quotedText, quotedSender string) {
	displayText := attachmentDisplayText(mediaType, filename, caption)
	fail := func(errMsg string) {
		db.Exec(`UPDATE messages SET status = 'failed' WHERE id = ?`, id)
		if conn != nil {
			sendEvent(conn, "error", map[string]string{"message": errMsg})
		}
		broadcastEvent("message", map[string]interface{}{
			"jid": jid.String(), "from": "me", "id": id, "text": displayText,
			"from_me": true, "timestamp": time.Now().Unix(), "has_media": true,
			"mediaType": mediaType, "mediaFilename": filename, "status": "failed",
			"quotedId": quotedID, "quotedText": quotedText, "quotedSender": quotedSender,
		})
	}

	var waMediaType whatsmeow.MediaType
	switch mediaType {
	case "image", "sticker":
		waMediaType = whatsmeow.MediaImage
	case "video":
		waMediaType = whatsmeow.MediaVideo
	default:
		waMediaType = whatsmeow.MediaDocument
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

	header := make([]byte, 512)
	n, _ := f.Read(header)
	f.Seek(0, 0)

	mimetype := mimetypeHint
	if mimetype == "" || mimetype == "application/octet-stream" || mimetype == "application/zip" {
		mimetype = detectMimeType(filename, path, header[:n])
	}

	if conn != nil {
		sendEvent(conn, "attachment_uploading", map[string]interface{}{"id": id, "jid": jid.String(), "filename": filename, "status": "uploading", "percent": 0})
	}

	var pr io.Reader = f
	if totalSize > 0 {
		lastPct := -1
		pr = &ProgressReader{
			reader: f,
			total:  totalSize,
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
	}

	uploaded, err := client.UploadReader(context.Background(), pr, nil, waMediaType)
	if err != nil {
		fail(err.Error())
		return
	}

	var msg *waE2E.Message
	switch mediaType {
	case "image":
		imgMsg := &waE2E.ImageMessage{
			Mimetype: proto.String(mimetype),
			URL: proto.String(uploaded.URL), DirectPath: proto.String(uploaded.DirectPath),
			MediaKey: uploaded.MediaKey, FileEncSHA256: uploaded.FileEncSHA256, FileSHA256: uploaded.FileSHA256,
			FileLength: proto.Uint64(uploaded.FileLength), ContextInfo: ctxInfo,
		}
		if caption != "" {
			imgMsg.Caption = proto.String(caption)
		}
		msg = &waE2E.Message{ImageMessage: imgMsg}
	case "video":
		vidMsg := &waE2E.VideoMessage{
			Mimetype: proto.String(mimetype),
			URL: proto.String(uploaded.URL), DirectPath: proto.String(uploaded.DirectPath),
			MediaKey: uploaded.MediaKey, FileEncSHA256: uploaded.FileEncSHA256, FileSHA256: uploaded.FileSHA256,
			FileLength: proto.Uint64(uploaded.FileLength), ContextInfo: ctxInfo,
		}
		if caption != "" {
			vidMsg.Caption = proto.String(caption)
		}
		msg = &waE2E.Message{VideoMessage: vidMsg}
	case "sticker":
		msg = &waE2E.Message{StickerMessage: &waE2E.StickerMessage{
			Mimetype: proto.String(mimetype),
			URL:      proto.String(uploaded.URL), DirectPath: proto.String(uploaded.DirectPath),
			MediaKey: uploaded.MediaKey, FileEncSHA256: uploaded.FileEncSHA256, FileSHA256: uploaded.FileSHA256,
			FileLength: proto.Uint64(uploaded.FileLength), ContextInfo: ctxInfo,
		}}
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

	resp, err := client.SendMessage(context.Background(), jid, msg, whatsmeow.SendRequestExtra{ID: id})
	if err != nil {
		fail(err.Error())
		return
	}

	rawBytes, _ := proto.Marshal(msg)
	ts := resp.Timestamp.Unix()
	db.Exec(`UPDATE messages SET status='sent', text=?, timestamp=?, sent_at=?, raw=?, media_path=?, media_mimetype=? WHERE id=?`,
		displayText, ts, ts, rawBytes, path, mimetype, id)

	broadcastEvent("message", map[string]interface{}{
		"jid": jid.String(), "from": "me", "id": id, "text": displayText,
		"from_me":       true,
		"timestamp":     ts,
		"has_media":     true,
		"mediaType":     mediaType,
		"mediaFilename": filename,
		"status":        "sent",
		"quotedId":      quotedID,
		"quotedText":    quotedText,
		"quotedSender":  quotedSender,
	})
	broadcastChats()
	if conn != nil {
		sendEvent(conn, "attachment_sent", map[string]interface{}{"jid": jid.String(), "filename": filename, "status": "sent"})
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
		id := client.GenerateMessageID()

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
		if localPath == "" {
			if cachedPath, cacheErr := saveMediaBytes("outgoing", id, mimetype, fileBytes); cacheErr == nil {
				localPath = cachedPath
			}
		}

		ctxInfo, quotedID, quotedText, quotedSender := buildAttachmentContext(data.ReplyToID, data.ReplySender)

		senderJID := ""
		if client.Store.ID != nil {
			senderJID = client.Store.ID.ToNonAD().String()
		}
		voiceText := fmt.Sprintf("[Voice Message] (%d:%02d)", data.Duration/60, data.Duration%60)
		ts := time.Now().Unix()

		saveMessage(StoredMessage{
			ID: id, JID: jid.String(), SenderJID: senderJID, FromMe: true,
			Text: voiceText, Timestamp: ts, Status: "uploading",
			MediaType: "voice", MediaMimetype: mimetype, MediaPath: localPath,
			QuotedID: quotedID, QuotedText: quotedText, QuotedSender: quotedSender,
		})
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
		broadcastChats()

		sendUploadedVoiceMessage(conn, id, jid, localPath, mimetype, data.Duration, ctxInfo, quotedID, quotedText, quotedSender)
	}()
}

// sendUploadedVoiceMessage uploads the recording at path and sends it as a
// PTT audio message, updating the row already persisted by
// handleSendVoiceMessage (or reused by handleRetryMessage) to "sent" on
// success or "failed" on error.
func sendUploadedVoiceMessage(conn net.Conn, id string, jid types.JID, path, mimetypeHint string, duration int, ctxInfo *waE2E.ContextInfo, quotedID, quotedText, quotedSender string) {
	voiceText := fmt.Sprintf("[Voice Message] (%d:%02d)", duration/60, duration%60)

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

	fileBytes, err := os.ReadFile(path)
	if err != nil {
		fail(err.Error())
		return
	}
	mimetype := mimetypeHint
	if mimetype == "" {
		mimetype = "audio/ogg; codecs=opus"
	}

	uploaded, err := client.Upload(context.Background(), fileBytes, whatsmeow.MediaAudio)
	if err != nil {
		fail(err.Error())
		return
	}

	// WhatsApp strict requirement: Waveform MUST be exactly 64 bytes.
	waveform := []byte{
		0, 10, 20, 35, 50, 75, 90, 110, 100, 85, 70, 50, 30, 15, 5, 0,
		5, 20, 45, 65, 80, 100, 120, 110, 95, 75, 50, 30, 20, 10, 5, 0,
		10, 25, 40, 60, 85, 105, 115, 100, 80, 60, 45, 25, 15, 5, 0, 5,
		15, 30, 55, 75, 95, 110, 90, 70, 50, 35, 20, 10, 5, 0, 0, 0,
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

	resp, err := client.SendMessage(context.Background(), jid, msg, whatsmeow.SendRequestExtra{ID: id})
	if err != nil {
		fail(err.Error())
		return
	}

	rawBytes, _ := proto.Marshal(msg)
	ts := resp.Timestamp.Unix()
	db.Exec(`UPDATE messages SET status='sent', timestamp=?, sent_at=?, raw=?, media_path=?, media_mimetype=? WHERE id=?`,
		ts, ts, rawBytes, path, mimetype, id)

	broadcastEvent("message", map[string]interface{}{
		"jid": jid.String(), "from": "me", "id": id, "text": voiceText,
		"from_me":      true,
		"timestamp":    ts,
		"has_media":    true,
		"mediaType":    "voice",
		"status":       "sent",
		"quotedId":     quotedID,
		"quotedText":   quotedText,
		"quotedSender": quotedSender,
	})
	broadcastChats()
}

// handlePlayVoiceMessage generalizes the original ad-hoc voice-download code
// into the shared media cache, and still sends WhatsApp the "played" receipt
// so the sender sees their voice note was listened to.
func handlePlayVoiceMessage(conn net.Conn, raw json.RawMessage) {
	var data struct {
		ID string `json:"id"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		_, jidStr, senderJidStr, _, dbErr := getMessageRaw(data.ID)
		if dbErr == nil {
			chatJID, _ := types.ParseJID(jidStr)
			senderJID, _ := types.ParseJID(senderJidStr)
			if senderJID.IsEmpty() {
				senderJID = chatJID
			}
			if chatJID.Server == "lid" {
				if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), chatJID); err == nil && !pn.IsEmpty() {
					chatJID = pn
				}
			}
			if senderJID.Server == "lid" {
				if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), senderJID); err == nil && !pn.IsEmpty() {
					senderJID = pn
				}
			}

			go func() {
				err := client.MarkRead(context.Background(), []types.MessageID{types.MessageID(data.ID)}, time.Now(), chatJID, senderJID, types.ReceiptTypePlayed)
				if err != nil {
					log.Printf("MarkRead played error for message %s: %v", data.ID, err)
				}
			}()
		}

		path, _, _, _, err := ensureMediaCached(data.ID)
		if err != nil {
			if strings.Contains(err.Error(), "URL expired") {
				pendingAutoOpens.Store(data.ID, map[string]interface{}{
					"conn": conn,
					"action": "play_audio",
				})
				return
			}
			fmt.Println("Error downloading audio:", err)
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}
		sendEvent(conn, "play_audio", map[string]interface{}{"path": path, "message_id": data.ID})
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

			// Album items are sent within the same second (often the exact
			// same one), so without disambiguating here every photo's
			// synthesized name would collide and only the last file written
			// would actually end up on the clipboard - looking like the
			// same image copied over and over instead of the whole album.
			tempPath := uniqueFileForDir(tempDir, destFilename)
			if bytes, err := os.ReadFile(path); err == nil {
				if os.WriteFile(tempPath, bytes, 0644) == nil {
					paths = append(paths, tempPath)
				}
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
		if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), chatJID); err == nil && !pn.IsEmpty() {
			chatJID = pn
		}
	}
	if senderJID.Server == "lid" {
		if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), senderJID); err == nil && !pn.IsEmpty() {
			senderJID = pn
		}
	}

	msgInfo := &types.MessageInfo{
		ID:       types.MessageID(messageID),
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
	return client.SendMediaRetryReceipt(context.Background(), msgInfo, mediaKey)
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
	// media shares its nested pointers with msg, so patching DirectPath here
	// still shows up when msg is re-marshalled below.
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

	// If there's an active waiter (handleDownloadMedia), signal it to resume downloading
	retryWaitersMu.Lock()
	if ch, ok := retryWaiters[string(evt.MessageID)]; ok {
		ch <- true
		delete(retryWaiters, string(evt.MessageID))
		retryWaitersMu.Unlock()
		return // Let the waiting goroutine handle the download!
	}
	retryWaitersMu.Unlock()

	// If no waiter, start a background download
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
						sendEvent(conn, "error", map[string]string{"message": friendlyMediaError(err)})
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
					// This used to just sendEvent "open_media" with a path,
					// but nothing on the frontend ever listened for that —
					// the auto-open silently did nothing once an expired
					// media URL had to be re-fetched. Open it directly here
					// instead, same as the fast (already-cached) path. conn
					// is safely nil if the assertion above failed.
					openMediaFileWithRealName(conn, string(evt.MessageID), path, mediaType, mimetype, filename)
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
