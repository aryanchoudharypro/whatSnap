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

	"github.com/gorilla/websocket"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

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

func handleDownloadMedia(conn *websocket.Conn, raw json.RawMessage) {
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
		path, mediaType, mimetype, filename, err := downloadMessageMediaToFile(ctx, data.MessageID, onProgress)
		if err != nil {
			fmt.Printf("Media download failed for message %s: %v\n", data.MessageID, err)
			if errors.Is(err, context.Canceled) {
				broadcastEvent("media_download_cancelled", map[string]interface{}{
					"message_id": data.MessageID,
					"jid":        data.JID,
				})
			} else if strings.Contains(err.Error(), "403") || strings.Contains(err.Error(), "404") || strings.Contains(err.Error(), "410") {
				fmt.Printf("Requesting media retry for %s...\n", data.MessageID)
				retryErr := requestMediaRetry(data.MessageID)
				if retryErr != nil {
					fmt.Printf("Retry request failed: %v\n", retryErr)
					broadcastEvent("media_download_failed", map[string]interface{}{
						"message_id": data.MessageID,
						"jid":        data.JID,
						"message":    "URL expired and retry request failed: " + retryErr.Error(),
					})
				} else {
					// Inform frontend we are requesting from phone (percent=0)
					broadcastEvent("media_download_progress", map[string]interface{}{
						"message_id": data.MessageID,
						"jid":        data.JID,
						"percent":    0,
					})
				}
			} else {
				broadcastEvent("media_download_failed", map[string]interface{}{
					"message_id": data.MessageID,
					"jid":        data.JID,
					"message":    err.Error(),
				})
			}
			return
		}
		broadcastEvent("media_downloaded", map[string]interface{}{
			"message_id": data.MessageID,
			"jid":        data.JID,
			"path":       path,
			"mediaType":  mediaType,
			"mimetype":   mimetype,
			"filename":   filename,
		})
	}()
}

func handleCancelDownload(conn *websocket.Conn, raw json.RawMessage) {
	var data MessageRefData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	activeDownloadsMu.Lock()
	cancel := activeDownloads[data.MessageID]
	activeDownloadsMu.Unlock()
	if cancel != nil {
		cancel()
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

func handleOpenMedia(conn *websocket.Conn, raw json.RawMessage) {
	var data MessageRefData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		path, _, _, filename, err := ensureMediaCached(data.MessageID)
		if err != nil {
			if strings.Contains(err.Error(), "URL expired") {
				pendingAutoOpens.Store(data.MessageID, map[string]interface{}{
					"conn": conn,
					"action": "open_media",
				})
				return
			}
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}

		openPath := path
		if filename != "" {
			tempDir := filepath.Join(os.TempDir(), "whantv_open")
			os.MkdirAll(tempDir, 0755)
			tempPath := filepath.Join(tempDir, filename)
			if bytes, err := os.ReadFile(path); err == nil {
				if os.WriteFile(tempPath, bytes, 0644) == nil {
					openPath = tempPath
				}
			}
		}

		if err := openFileInOS(openPath); err != nil {
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
		}
	}()
}

func handleSaveMediaAs(conn *websocket.Conn, raw json.RawMessage) {
	var data SaveMediaAsData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		path, _, _, _, err := ensureMediaCached(data.MessageID)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
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
	return ctxInfo, replyToID, preview, participant
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

func handleSendAttachment(conn *websocket.Conn, raw json.RawMessage) {
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
		var waMediaType whatsmeow.MediaType
		switch data.Type {
		case "image", "sticker":
			waMediaType = whatsmeow.MediaImage
		case "video":
			waMediaType = whatsmeow.MediaVideo
		default:
			waMediaType = whatsmeow.MediaDocument
		}

		sendEvent(conn, "attachment_uploading", map[string]interface{}{"jid": jid.String(), "filename": data.Filename, "status": "uploading", "percent": 0})

		var uploaded whatsmeow.UploadResponse
		var mimetype string
		var localPath string

		if data.Path != "" {
			f, err := os.Open(data.Path)
			if err != nil {
				sendEvent(conn, "error", map[string]string{"message": err.Error()})
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

			mimetype = data.Mimetype
			if mimetype == "" || mimetype == "application/octet-stream" || mimetype == "application/zip" {
				mimetype = detectMimeType(data.Filename, data.Path, header[:n])
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
							sendEvent(conn, "attachment_uploading", map[string]interface{}{
								"jid":      jid.String(),
								"filename": data.Filename,
								"percent":  pct,
								"status":   "uploading",
							})
						}
					},
				}
			}

			uploaded, err = client.UploadReader(context.Background(), pr, nil, waMediaType)
			if err != nil {
				sendEvent(conn, "error", map[string]string{"message": err.Error()})
				return
			}
			localPath = data.Path
		} else {
			fileBytes, err := readLocalFileOrDecode("", data.Data)
			if err != nil {
				sendEvent(conn, "error", map[string]string{"message": err.Error()})
				return
			}
			mimetype = data.Mimetype
			if mimetype == "" || mimetype == "application/octet-stream" || mimetype == "application/zip" {
				mimetype = detectMimeType(data.Filename, "", fileBytes)
			}
			uploaded, err = client.Upload(context.Background(), fileBytes, waMediaType)
			if err != nil {
				sendEvent(conn, "error", map[string]string{"message": err.Error()})
				return
			}
		}

		ctxInfo, quotedID, quotedText, quotedSender := buildAttachmentContext(data.ReplyToID, data.ReplySender)

		var msg *waE2E.Message
		switch data.Type {
		case "image":
			imgMsg := &waE2E.ImageMessage{
				Mimetype: proto.String(mimetype),
				URL: proto.String(uploaded.URL), DirectPath: proto.String(uploaded.DirectPath),
				MediaKey: uploaded.MediaKey, FileEncSHA256: uploaded.FileEncSHA256, FileSHA256: uploaded.FileSHA256,
				FileLength: proto.Uint64(uploaded.FileLength), ContextInfo: ctxInfo,
			}
			if data.Caption != "" {
				imgMsg.Caption = proto.String(data.Caption)
			}
			msg = &waE2E.Message{ImageMessage: imgMsg}
		case "video":
			vidMsg := &waE2E.VideoMessage{
				Mimetype: proto.String(mimetype),
				URL: proto.String(uploaded.URL), DirectPath: proto.String(uploaded.DirectPath),
				MediaKey: uploaded.MediaKey, FileEncSHA256: uploaded.FileEncSHA256, FileSHA256: uploaded.FileSHA256,
				FileLength: proto.Uint64(uploaded.FileLength), ContextInfo: ctxInfo,
			}
			if data.Caption != "" {
				vidMsg.Caption = proto.String(data.Caption)
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
			filename := data.Filename
			docMsg := &waE2E.DocumentMessage{
				Title: proto.String(filename), FileName: proto.String(filename), Mimetype: proto.String(mimetype),
				URL: proto.String(uploaded.URL), DirectPath: proto.String(uploaded.DirectPath),
				MediaKey: uploaded.MediaKey, FileEncSHA256: uploaded.FileEncSHA256, FileSHA256: uploaded.FileSHA256,
				FileLength: proto.Uint64(uploaded.FileLength), ContextInfo: ctxInfo,
			}
			if data.Caption != "" {
				docMsg.Caption = proto.String(data.Caption)
			}
			msg = &waE2E.Message{DocumentMessage: docMsg}
		}

		resp, err := client.SendMessage(context.Background(), jid, msg)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}

		rawBytes, _ := proto.Marshal(msg)
		saveMessage(StoredMessage{
			ID: resp.ID, JID: jid.String(), SenderJID: client.Store.ID.ToNonAD().String(), FromMe: true,
			Text: extractText(msg), Timestamp: resp.Timestamp.Unix(), Raw: rawBytes, Status: "sent",
			MediaType: data.Type, MediaMimetype: mimetype, MediaFilename: data.Filename, MediaPath: localPath,
			QuotedID: quotedID, QuotedText: quotedText, QuotedSender: quotedSender,
		})
		broadcastEvent("message", map[string]interface{}{
			"jid": jid.String(), "from": "me", "id": resp.ID, "text": extractText(msg),
		})
		broadcastChats()
		sendEvent(conn, "attachment_sent", map[string]interface{}{"jid": jid.String(), "filename": data.Filename, "status": "sent"})
	}()
}

func handleSendVoiceMessage(conn *websocket.Conn, raw json.RawMessage) {
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
		fileBytes, err := readLocalFileOrDecode(data.Path, data.Data)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}
		mimetype := data.Mimetype
		if mimetype == "" {
			mimetype = "audio/ogg; codecs=opus"
		}

		uploaded, err := client.Upload(context.Background(), fileBytes, whatsmeow.MediaAudio)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}

		waveform := []byte{0, 5, 12, 25, 45, 70, 90, 100, 85, 60, 40, 20, 10, 5, 0, 10, 30, 60, 80, 95, 75, 50, 30, 15, 5, 0}

		msg := &waE2E.Message{AudioMessage: &waE2E.AudioMessage{
			Mimetype: proto.String(mimetype), PTT: proto.Bool(true),
			Seconds: proto.Uint32(uint32(data.Duration)),
			URL:     proto.String(uploaded.URL), DirectPath: proto.String(uploaded.DirectPath),
			MediaKey: uploaded.MediaKey, FileEncSHA256: uploaded.FileEncSHA256, FileSHA256: uploaded.FileSHA256,
			FileLength: proto.Uint64(uploaded.FileLength),
			Waveform:   waveform,
		}}

		resp, err := client.SendMessage(context.Background(), jid, msg)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}

		rawBytes, _ := proto.Marshal(msg)
		localPath := data.Path
		if localPath == "" {
			if cachedPath, cacheErr := saveMediaBytes("outgoing", resp.ID, mimetype, fileBytes); cacheErr == nil {
				localPath = cachedPath
			}
		}
		voiceText := fmt.Sprintf("[Voice Message] (%d:%02d)", data.Duration/60, data.Duration%60)
		saveMessage(StoredMessage{
			ID: resp.ID, JID: jid.String(), SenderJID: client.Store.ID.ToNonAD().String(), FromMe: true,
			Text: voiceText, Timestamp: resp.Timestamp.Unix(), Raw: rawBytes, Status: "sent",
			MediaType: "voice", MediaMimetype: mimetype, MediaPath: localPath,
		})
		broadcastEvent("message", map[string]interface{}{
			"jid": jid.String(), "from": "me", "id": resp.ID, "text": voiceText,
		})
		broadcastChats()
	}()
}

// handlePlayVoiceMessage generalizes the original ad-hoc voice-download code
// into the shared media cache, and still sends WhatsApp the "played" receipt
// so the sender sees their voice note was listened to.
func handlePlayVoiceMessage(conn *websocket.Conn, raw json.RawMessage) {
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

func handleCopyAlbumToClipboard(conn *websocket.Conn, raw json.RawMessage) {
	var data CopyAlbumToClipboardData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		tempDir := filepath.Join(os.TempDir(), "whantv_clipboard")
		os.MkdirAll(tempDir, 0755)

		var paths []string
		for _, msgID := range data.MessageIDs {
			path, _, _, filename, err := ensureMediaCached(msgID)
			if err != nil {
				continue
			}

			if filename != "" {
				tempPath := filepath.Join(tempDir, filename)
				if bytes, err := os.ReadFile(path); err == nil {
					if os.WriteFile(tempPath, bytes, 0644) == nil {
						paths = append(paths, tempPath)
					}
				}
			} else {
				tempPath := filepath.Join(tempDir, msgID+".jpg")
				if bytes, err := os.ReadFile(path); err == nil {
					if os.WriteFile(tempPath, bytes, 0644) == nil {
						paths = append(paths, tempPath)
					}
				}
			}
		}

		if len(paths) > 0 {
			var psPaths []string
			for _, p := range paths {
				psPaths = append(psPaths, fmt.Sprintf("'%s'", p))
			}
			psPathsStr := strings.Join(psPaths, ",")
			script := fmt.Sprintf("Add-Type -AssemblyName System.Windows.Forms; $files = @(%s); $sc = New-Object System.Collections.Specialized.StringCollection; foreach ($f in $files) { $sc.Add($f) | Out-Null }; [System.Windows.Forms.Clipboard]::SetFileDropList($sc)", psPathsStr)

			cmd := exec.Command("powershell", "-NoProfile", "-Command", script)
			if err := cmd.Run(); err == nil {
				sendEvent(conn, "copied_album_to_clipboard", map[string]interface{}{})
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
	var mediaKey []byte
	switch {
	case msg.GetImageMessage() != nil:
		mediaKey = msg.GetImageMessage().GetMediaKey()
	case msg.GetVideoMessage() != nil:
		mediaKey = msg.GetVideoMessage().GetMediaKey()
	case msg.GetAudioMessage() != nil:
		mediaKey = msg.GetAudioMessage().GetMediaKey()
	case msg.GetDocumentMessage() != nil:
		mediaKey = msg.GetDocumentMessage().GetMediaKey()
	case msg.GetStickerMessage() != nil:
		mediaKey = msg.GetStickerMessage().GetMediaKey()
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
	var mediaKey []byte
	switch {
	case msg.GetImageMessage() != nil:
		mediaKey = msg.GetImageMessage().GetMediaKey()
	case msg.GetVideoMessage() != nil:
		mediaKey = msg.GetVideoMessage().GetMediaKey()
	case msg.GetAudioMessage() != nil:
		mediaKey = msg.GetAudioMessage().GetMediaKey()
	case msg.GetDocumentMessage() != nil:
		mediaKey = msg.GetDocumentMessage().GetMediaKey()
	case msg.GetStickerMessage() != nil:
		mediaKey = msg.GetStickerMessage().GetMediaKey()
	}
	if mediaKey == nil {
		return
	}
	retryData, err := whatsmeow.DecryptMediaRetryNotification(evt, mediaKey)
	if err != nil || retryData.GetResult() != waMmsRetry.MediaRetryNotification_SUCCESS {
		fmt.Printf("Failed to decrypt media retry or result not success for %s: %v\n", evt.MessageID, err)
		broadcastEvent("media_download_failed", map[string]interface{}{
			"message_id": string(evt.MessageID),
			"jid":        jidStr,
			"message":    "Phone could not re-upload the expired media",
		})
		return
	}

	newPath := retryData.GetDirectPath()
	if newPath == "" {
		return
	}
	switch {
	case msg.GetImageMessage() != nil:
		msg.GetImageMessage().DirectPath = proto.String(newPath)
	case msg.GetVideoMessage() != nil:
		msg.GetVideoMessage().DirectPath = proto.String(newPath)
	case msg.GetAudioMessage() != nil:
		msg.GetAudioMessage().DirectPath = proto.String(newPath)
	case msg.GetDocumentMessage() != nil:
		msg.GetDocumentMessage().DirectPath = proto.String(newPath)
	case msg.GetStickerMessage() != nil:
		msg.GetStickerMessage().DirectPath = proto.String(newPath)
	}

	newRawBytes, _ := proto.Marshal(msg)
	db.Exec("UPDATE messages SET raw = ? WHERE id = ?", newRawBytes, string(evt.MessageID))

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
				"message":    err.Error(),
			})
			if c, ok := pendingAutoOpens.Load(string(evt.MessageID)); ok {
				data := c.(map[string]interface{})
				conn := data["conn"].(*websocket.Conn)
				sendEvent(conn, "error", map[string]string{"message": "Failed to automatically fetch old media: " + err.Error()})
				pendingAutoOpens.Delete(string(evt.MessageID))
			}
			return
		}

		if c, ok := pendingAutoOpens.Load(string(evt.MessageID)); ok {
			data := c.(map[string]interface{})
			conn := data["conn"].(*websocket.Conn)
			action := data["action"].(string)
			if action == "open_media" {
				if mediaType == "document" || mediaType == "audio" {
					absPath, _ := filepath.Abs(path)
					sendEvent(conn, "open_media", map[string]string{"path": absPath})
				} else {
					sendEvent(conn, "open_media", map[string]string{"path": path})
				}
			} else if action == "play_audio" {
				sendEvent(conn, "play_audio", map[string]interface{}{"path": path, "message_id": string(evt.MessageID)})
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
