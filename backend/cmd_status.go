package main

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"image"
	"log"
	"net"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
)

const whatsappTeal = 0xFF075E54

type PostStatusData struct {
	Text           string `json:"text"`
	BackgroundArgb uint32 `json:"background_argb"`
	TextArgb       uint32 `json:"text_argb"`
	Font           int32  `json:"font"`
}

type PostMediaStatusData struct {
	Type      string `json:"type"`
	Path      string `json:"path"`
	Data      string `json:"data"`
	Mimetype  string `json:"mimetype"`
	Filename  string `json:"filename"`
	Caption   string `json:"caption"`
	Thumbnail string `json:"thumbnail"`
}

type MarkStatusViewedData struct {
	ID        string `json:"id"`
	PosterJID string `json:"poster_jid"`
}

func (acc *Account) handleGetStatuses(conn net.Conn) {
	go func() {
		acc.purgeExpiredStatuses()

		rows, err := acc.db.Query(`SELECT id, COALESCE(sender_jid, ''), from_me, COALESCE(text, ''),
				COALESCE(media_type, ''), COALESCE(media_mimetype, ''), COALESCE(media_filename, ''), COALESCE(media_path, ''),
				timestamp, COALESCE(viewed, 0),
				(SELECT COUNT(*) FROM status_viewers v WHERE v.status_id = messages.id AND v.viewer != 'me'),
				COALESCE((SELECT reaction FROM status_viewers v WHERE v.status_id = messages.id AND v.viewer = 'me'), ''),
				CASE WHEN COALESCE(media_type, '') = '' THEN raw END
			FROM messages WHERE jid = 'status@broadcast' AND COALESCE(is_deleted, 0) = 0 ORDER BY timestamp ASC`)
		muted := acc.statusMuted()

		type groupMeta struct {
			posterJID, posterName string
			fromMe, muted         bool
			items                 []map[string]interface{}
		}
		order := []string{}
		groups := map[string]*groupMeta{}

		if err == nil {
			defer rows.Close()
			for rows.Next() {
				var id, senderJID, text, mediaType, mediaMimetype, mediaFilename, mediaPath string
				var ts int64
				var fromMe bool
				var viewed, viewers int
				var myReaction string
				var raw []byte
				if scanErr := rows.Scan(&id, &senderJID, &fromMe, &text, &mediaType, &mediaMimetype, &mediaFilename, &mediaPath, &ts, &viewed, &viewers, &myReaction, &raw); scanErr != nil {
					continue
				}
				background, textColor, font := textStatusColors(raw)
				normJID := senderJID
				if parsed, perr := types.ParseJID(senderJID); perr == nil {
					normJID = acc.preferPN(parsed).String()
				}
				key := normJID
				if fromMe {
					key = "me"
				}
				g, ok := groups[key]
				if !ok {
					g = &groupMeta{posterJID: normJID, posterName: "You", fromMe: fromMe}
					groups[key] = g
					order = append(order, key)
				}
				// Your own posts don't carry a meaningful "have I seen this"
				// state - always report them viewed so they never show as new.
				g.items = append(g.items, map[string]interface{}{
					"id": id, "text": text, "mediaType": mediaType, "mediaMimetype": mediaMimetype,
					"mediaFilename": mediaFilename, "mediaPath": mediaPath, "timestamp": ts, "viewed": viewed == 1 || fromMe,
					"viewers": viewers, "myReaction": myReaction, "background": background, "textColor": textColor,
					"font": font,
				})
			}
			rows.Close()
		}
		itemCount := 0
		for _, g := range groups {
			itemCount += len(g.items)
		}
		log.Printf("Status tab: %d people, %d updates", len(groups), itemCount)
		for _, g := range groups {
			if g.fromMe {
				continue
			}
			g.muted = acc.statusMutedFor(muted, g.posterJID)
			if parsed, perr := types.ParseJID(g.posterJID); perr == nil {
				g.posterName = acc.resolveDisplayName(parsed)
			} else {
				g.posterName = g.posterJID
			}
		}

		result := make([]map[string]interface{}, 0, len(order))
		for _, key := range order {
			g := groups[key]
			allViewed := true
			for _, it := range g.items {
				if !it["viewed"].(bool) {
					allViewed = false
					break
				}
			}
			result = append(result, map[string]interface{}{
				"posterJid": g.posterJID, "posterName": g.posterName, "fromMe": g.fromMe,
				"allViewed": allViewed, "muted": g.muted, "statuses": g.items,
			})
		}
		sort.SliceStable(result, func(i, j int) bool {
			fi, fj := result[i]["fromMe"].(bool), result[j]["fromMe"].(bool)
			if fi != fj {
				return fi
			}
			if mi, mj := result[i]["muted"].(bool), result[j]["muted"].(bool); mi != mj {
				return mj
			}
			si, sj := result[i]["statuses"].([]map[string]interface{}), result[j]["statuses"].([]map[string]interface{})
			return si[len(si)-1]["timestamp"].(int64) > sj[len(sj)-1]["timestamp"].(int64)
		})

		acc.sendEvent(conn, "statuses", map[string]interface{}{"groups": result})
	}()
}

func textStatusColors(raw []byte) (background, text uint32, font int32) {
	if len(raw) == 0 {
		return 0, 0, 0
	}
	msg := &waE2E.Message{}
	if proto.Unmarshal(raw, msg) != nil {
		return 0, 0, 0
	}
	msg = unwrapMessage(msg)
	if msg == nil {
		return 0, 0, 0
	}
	ext := msg.GetExtendedTextMessage()
	if ext == nil {
		return 0, 0, 0
	}
	fontVal := int32(0)
	if ext.Font != nil {
		fontVal = int32(*ext.Font)
	}
	return ext.GetBackgroundArgb(), ext.GetTextArgb(), fontVal
}

func (acc *Account) handlePostStatus(raw json.RawMessage) {
	var data PostStatusData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	text := strings.TrimSpace(data.Text)
	if text == "" {
		return
	}
	bg := uint32(whatsappTeal)
	if data.BackgroundArgb != 0 {
		bg = data.BackgroundArgb
	}
	textColor := uint32(0xFFFFFFFF)
	if data.TextArgb != 0 {
		textColor = data.TextArgb
	}
	font := data.Font
	go func() {
		if reason := acc.statusPostBlocker(context.Background()); reason != "" {
			acc.broadcastEvent("error", map[string]string{"message": reason})
			return
		}
		ext := &waE2E.ExtendedTextMessage{
			Text:           proto.String(text),
			BackgroundArgb: proto.Uint32(bg),
			TextArgb:       proto.Uint32(textColor),
		}
		if font > 0 {
			f := waE2E.ExtendedTextMessage_FontType(font)
			ext.Font = &f
		}
		msg := &waE2E.Message{
			ExtendedTextMessage: ext,
		}
		recipients := acc.statusRecipients(context.Background())
		resp, err := acc.wa().SendMessage(context.Background(), types.StatusBroadcastJID, msg, whatsmeow.SendRequestExtra{ID: acc.wa().GenerateMessageID(), StatusRecipients: recipients, StatusSetting: acc.statusSetting(context.Background())})
		if err != nil {
			acc.broadcastEvent("error", map[string]string{"message": "Failed to post status: " + err.Error()})
			return
		}
		acc.rememberStatusRecipients(resp.ID, recipients)
		ownJID := ""
		if acc.wa().Store.ID != nil {
			ownJID = acc.wa().Store.ID.ToNonAD().String()
		}
		rawBytes, _ := proto.Marshal(msg)
		acc.saveMessage(StoredMessage{
			ID: resp.ID, JID: "status@broadcast", SenderJID: ownJID,
			FromMe: true, Text: text, Timestamp: resp.Timestamp.Unix(), Raw: rawBytes, Status: "sent",
		})
		acc.broadcastEvent("status_update", map[string]interface{}{
			"id": resp.ID, "posterJid": ownJID, "fromMe": true, "text": text,
			"mediaType": "", "timestamp": resp.Timestamp.Unix(),
			"background": bg, "textColor": textColor, "font": font,
		})
	}()
}

// handlePostMediaStatus posts a photo or video Status update, mirroring
// handlePostStatus's text-only flow but reusing the upload machinery from
// sendUploadedAttachment (minus ContextInfo/quoting, which don't apply to
// statuses). Sending to types.StatusBroadcastJID is what makes whatsmeow
// broadcast-encrypt this to the status audience instead of a single chat.
func (acc *Account) handlePostMediaStatus(raw json.RawMessage) {
	var data PostMediaStatusData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	if data.Type != "image" && data.Type != "video" {
		return
	}
	go func() {
		if reason := acc.statusPostBlocker(context.Background()); reason != "" {
			acc.broadcastEvent("error", map[string]string{"message": reason})
			return
		}
		id := acc.wa().GenerateMessageID()

		srcPath := data.Path
		if srcPath == "" {
			fileBytes, err := readLocalFileOrDecode("", data.Data)
			if err != nil {
				acc.broadcastEvent("error", map[string]string{"message": err.Error()})
				return
			}
			tmp, saveErr := acc.saveMediaBytes("outgoing", id+"_src", detectMimeType(data.Filename, "", fileBytes), fileBytes)
			if saveErr != nil {
				acc.broadcastEvent("error", map[string]string{"message": saveErr.Error()})
				return
			}
			srcPath = tmp
		}
		var photo *preparedPhoto
		preparedCopy := ""
		if data.Type == "image" {
			if p, err := acc.preparePhoto(id+"_status", srcPath, false); err == nil {
				photo = p
				if p.path != srcPath {
					if data.Path == "" {
						os.Remove(srcPath)
					}
					srcPath, preparedCopy, data.Mimetype = p.path, p.path, "image/jpeg"
				}
			} else {
				log.Printf("Posting status photo %s unprepared: %v", id, err)
			}
		}
		if st, err := os.Stat(srcPath); err == nil {
			if limit := int64(acc.serverFlagInt(abPropStatusMediaLimitMB, defaultMediaLimitMB)) << 20; st.Size() > limit {
				if data.Path == "" {
					os.Remove(srcPath)
				}
				acc.broadcastEvent("error", map[string]string{"message": attachmentTooLargeText(data.Type, st.Size(), limit)})
				return
			}
		}
		mimetype := data.Mimetype
		if mimetype == "" || mimetype == "application/octet-stream" || mimetype == "application/zip" {
			mimetype = detectMimeType(data.Filename, srcPath, sniffFileHeader(srcPath))
		}
		cachedPath := filepath.Join(acc.mediaBaseDir(), "outgoing", sanitizeFilenameComponent(id)+extForMime(mimetype))
		if srcPath != cachedPath {
			if err := copyFileStreaming(srcPath, cachedPath); err != nil {
				acc.broadcastEvent("error", map[string]string{"message": err.Error()})
				return
			}
			if data.Path == "" || srcPath == preparedCopy {
				os.Remove(srcPath)
			}
		}

		var video *videoInfo
		var videoThumb []byte
		if data.Type == "video" {
			if info, err := readVideoInfo(cachedPath); err == nil {
				video = info
				if info.brand == "qt  " && (info.codec == "avc1" || info.codec == "avc3") && mp4Brand(cachedPath) == nil {
					mimetype = "video/mp4"
				}
			}
			if png, err := base64.StdEncoding.DecodeString(data.Thumbnail); err == nil {
				if img, _, err := image.Decode(bytes.NewReader(png)); err == nil {
					videoThumb = jpegThumbnail(img)
				}
			}
		}

		waMediaType := whatsmeow.MediaImage
		if data.Type == "video" {
			waMediaType = whatsmeow.MediaVideo
		}
		f, err := os.Open(cachedPath)
		if err != nil {
			acc.broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		uploaded, err := acc.wa().UploadReader(context.Background(), f, nil, waMediaType)
		f.Close()
		if err != nil {
			acc.broadcastEvent("error", map[string]string{"message": "Failed to post status: " + err.Error()})
			return
		}

		var msg *waE2E.Message
		if data.Type == "image" {
			imgMsg := &waE2E.ImageMessage{
				Mimetype:      proto.String(mimetype),
				URL:           proto.String(uploaded.URL),
				DirectPath:    proto.String(uploaded.DirectPath),
				MediaKey:      uploaded.MediaKey,
				FileEncSHA256: uploaded.FileEncSHA256,
				FileSHA256:    uploaded.FileSHA256,
				FileLength:    proto.Uint64(uploaded.FileLength),
			}
			if photo != nil {
				imgMsg.Width, imgMsg.Height = proto.Uint32(uint32(photo.width)), proto.Uint32(uint32(photo.height))
				imgMsg.JPEGThumbnail = photo.thumbnail
			}
			if data.Caption != "" {
				imgMsg.Caption = proto.String(data.Caption)
			}
			msg = &waE2E.Message{ImageMessage: imgMsg}
		} else {
			vidMsg := &waE2E.VideoMessage{
				Mimetype:      proto.String(mimetype),
				URL:           proto.String(uploaded.URL),
				DirectPath:    proto.String(uploaded.DirectPath),
				MediaKey:      uploaded.MediaKey,
				FileEncSHA256: uploaded.FileEncSHA256,
				FileSHA256:    uploaded.FileSHA256,
				FileLength:    proto.Uint64(uploaded.FileLength),
			}
			if video != nil {
				vidMsg.Seconds = proto.Uint32(uint32(video.seconds))
				vidMsg.Width, vidMsg.Height = proto.Uint32(uint32(video.width)), proto.Uint32(uint32(video.height))
			}
			if videoThumb != nil {
				vidMsg.JPEGThumbnail = videoThumb
			}
			if data.Caption != "" {
				vidMsg.Caption = proto.String(data.Caption)
			}
			msg = &waE2E.Message{VideoMessage: vidMsg}
		}

		recipients := acc.statusRecipients(context.Background())
		resp, err := acc.wa().SendMessage(context.Background(), types.StatusBroadcastJID, msg, whatsmeow.SendRequestExtra{ID: id, StatusRecipients: recipients, StatusSetting: acc.statusSetting(context.Background())})
		if err != nil {
			acc.broadcastEvent("error", map[string]string{"message": "Failed to post status: " + err.Error()})
			return
		}
		acc.rememberStatusRecipients(resp.ID, recipients)

		ownJID := ""
		if acc.wa().Store.ID != nil {
			ownJID = acc.wa().Store.ID.ToNonAD().String()
		}
		rawBytes, _ := proto.Marshal(msg)
		acc.saveMessage(StoredMessage{
			ID: resp.ID, JID: "status@broadcast", SenderJID: ownJID,
			FromMe: true, Text: data.Caption, Timestamp: resp.Timestamp.Unix(), Raw: rawBytes, Status: "sent",
			MediaType: data.Type, MediaMimetype: mimetype, MediaFilename: data.Filename, MediaPath: cachedPath,
		})
		acc.broadcastEvent("status_update", map[string]interface{}{
			"id": resp.ID, "posterJid": ownJID, "fromMe": true, "text": data.Caption,
			"mediaType": data.Type, "timestamp": resp.Timestamp.Unix(),
		})
	}()
}

func (acc *Account) handleMarkStatusViewed(raw json.RawMessage) {
	var data MarkStatusViewedData
	if err := json.Unmarshal(raw, &data); err != nil || data.ID == "" || data.PosterJID == "" || data.PosterJID == "me" {
		return
	}
	acc.db.Exec("UPDATE messages SET viewed = 1 WHERE id = ? AND jid = 'status@broadcast'", data.ID)
	go func() {
		// The receipt goes to the address the status came from, which can be
		// the poster's LID while the Status tab shows them by phone number.
		poster, ok := acc.statusPoster(data.ID)
		if !ok || poster.IsEmpty() {
			return
		}
		if acc.wa().Store.ID != nil && poster.User == acc.wa().Store.ID.User {
			return
		}
		acc.ensureAvailableForRead()
		receiptType := types.ReceiptTypeRead
		if poster.User == "0" {
			receiptType = types.ReceiptTypeReadSelf
		}
		_ = acc.wa().MarkRead(context.Background(), []types.MessageID{types.MessageID(data.ID)}, time.Now(), types.StatusBroadcastJID, poster, receiptType)
	}()
}
