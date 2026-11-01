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
	Text string `json:"text"`
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

func handleGetStatuses(conn net.Conn) {
	go func() {
		purgeExpiredStatuses()

		rows, err := db.Query(`SELECT id, COALESCE(sender_jid, ''), from_me, COALESCE(text, ''),
				COALESCE(media_type, ''), COALESCE(media_mimetype, ''), COALESCE(media_filename, ''), COALESCE(media_path, ''),
				timestamp, COALESCE(viewed, 0),
				(SELECT COUNT(*) FROM status_viewers v WHERE v.status_id = messages.id AND v.viewer != 'me'),
				COALESCE((SELECT reaction FROM status_viewers v WHERE v.status_id = messages.id AND v.viewer = 'me'), ''),
				CASE WHEN COALESCE(media_type, '') = '' THEN raw END
			FROM messages WHERE jid = 'status@broadcast' AND COALESCE(is_deleted, 0) = 0 ORDER BY timestamp ASC`)
		muted := statusMuted()

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
				background, textColor := textStatusColors(raw)
				key := senderJID
				if fromMe {
					key = "me"
				}
				g, ok := groups[key]
				if !ok {
					name := "You"
					if !fromMe {
						if parsed, perr := types.ParseJID(senderJID); perr == nil {
							name = resolveDisplayName(parsed)
						} else {
							name = senderJID
						}
					}
					g = &groupMeta{posterJID: senderJID, posterName: name, fromMe: fromMe, muted: !fromMe && statusMutedFor(muted, senderJID)}
					groups[key] = g
					order = append(order, key)
				}
				// Your own posts don't carry a meaningful "have I seen this"
				// state - always report them viewed so they never show as new.
				g.items = append(g.items, map[string]interface{}{
					"id": id, "text": text, "mediaType": mediaType, "mediaMimetype": mediaMimetype,
					"mediaFilename": mediaFilename, "mediaPath": mediaPath, "timestamp": ts, "viewed": viewed == 1 || fromMe,
					"viewers": viewers, "myReaction": myReaction, "background": background, "textColor": textColor,
				})
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

		sendEvent(conn, "statuses", map[string]interface{}{"groups": result})
	}()
}

func textStatusColors(raw []byte) (background, text uint32) {
	if len(raw) == 0 {
		return 0, 0
	}
	msg := &waE2E.Message{}
	if proto.Unmarshal(raw, msg) != nil {
		return 0, 0
	}
	ext := msg.GetExtendedTextMessage()
	return ext.GetBackgroundArgb(), ext.GetTextArgb()
}

func handlePostStatus(raw json.RawMessage) {
	var data PostStatusData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	text := strings.TrimSpace(data.Text)
	if text == "" {
		return
	}
	go func() {
		msg := &waE2E.Message{
			ExtendedTextMessage: &waE2E.ExtendedTextMessage{
				Text:           proto.String(text),
				BackgroundArgb: proto.Uint32(whatsappTeal),
			},
		}
		resp, err := wa().SendMessage(context.Background(), types.StatusBroadcastJID, msg, whatsmeow.SendRequestExtra{ID: wa().GenerateMessageID(), StatusRecipients: statusRecipients(context.Background())})
		if err != nil {
			broadcastEvent("error", map[string]string{"message": "Failed to post status: " + err.Error()})
			return
		}
		ownJID := ""
		if wa().Store.ID != nil {
			ownJID = wa().Store.ID.ToNonAD().String()
		}
		rawBytes, _ := proto.Marshal(msg)
		saveMessage(StoredMessage{
			ID: resp.ID, JID: "status@broadcast", SenderJID: ownJID,
			FromMe: true, Text: text, Timestamp: resp.Timestamp.Unix(), Raw: rawBytes, Status: "sent",
		})
		broadcastEvent("status_update", map[string]interface{}{
			"id": resp.ID, "posterJid": ownJID, "fromMe": true, "text": text,
			"mediaType": "", "timestamp": resp.Timestamp.Unix(),
		})
	}()
}

// handlePostMediaStatus posts a photo or video Status update, mirroring
// handlePostStatus's text-only flow but reusing the upload machinery from
// sendUploadedAttachment (minus ContextInfo/quoting, which don't apply to
// statuses). Sending to types.StatusBroadcastJID is what makes whatsmeow
// broadcast-encrypt this to the status audience instead of a single chat.
func handlePostMediaStatus(raw json.RawMessage) {
	var data PostMediaStatusData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	if data.Type != "image" && data.Type != "video" {
		return
	}
	go func() {
		id := wa().GenerateMessageID()

		srcPath := data.Path
		if srcPath == "" {
			fileBytes, err := readLocalFileOrDecode("", data.Data)
			if err != nil {
				broadcastEvent("error", map[string]string{"message": err.Error()})
				return
			}
			tmp, saveErr := saveMediaBytes("outgoing", id+"_src", detectMimeType(data.Filename, "", fileBytes), fileBytes)
			if saveErr != nil {
				broadcastEvent("error", map[string]string{"message": saveErr.Error()})
				return
			}
			srcPath = tmp
		}
		var photo *preparedPhoto
		preparedCopy := ""
		if data.Type == "image" {
			if p, err := preparePhoto(id+"_status", srcPath, false); err == nil {
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
			if limit := int64(serverFlagInt(abPropStatusMediaLimitMB, defaultMediaLimitMB)) << 20; st.Size() > limit {
				if data.Path == "" {
					os.Remove(srcPath)
				}
				broadcastEvent("error", map[string]string{"message": attachmentTooLargeText(data.Type, st.Size(), limit)})
				return
			}
		}
		mimetype := data.Mimetype
		if mimetype == "" || mimetype == "application/octet-stream" || mimetype == "application/zip" {
			mimetype = detectMimeType(data.Filename, srcPath, sniffFileHeader(srcPath))
		}
		cachedPath := filepath.Join(mediaBaseDir(), "outgoing", sanitizeFilenameComponent(id)+extForMime(mimetype))
		if srcPath != cachedPath {
			if err := copyFileStreaming(srcPath, cachedPath); err != nil {
				broadcastEvent("error", map[string]string{"message": err.Error()})
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
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		uploaded, err := wa().UploadReader(context.Background(), f, nil, waMediaType)
		f.Close()
		if err != nil {
			broadcastEvent("error", map[string]string{"message": "Failed to post status: " + err.Error()})
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

		resp, err := wa().SendMessage(context.Background(), types.StatusBroadcastJID, msg, whatsmeow.SendRequestExtra{ID: id, StatusRecipients: statusRecipients(context.Background())})
		if err != nil {
			broadcastEvent("error", map[string]string{"message": "Failed to post status: " + err.Error()})
			return
		}

		ownJID := ""
		if wa().Store.ID != nil {
			ownJID = wa().Store.ID.ToNonAD().String()
		}
		rawBytes, _ := proto.Marshal(msg)
		saveMessage(StoredMessage{
			ID: resp.ID, JID: "status@broadcast", SenderJID: ownJID,
			FromMe: true, Text: data.Caption, Timestamp: resp.Timestamp.Unix(), Raw: rawBytes, Status: "sent",
			MediaType: data.Type, MediaMimetype: mimetype, MediaFilename: data.Filename, MediaPath: cachedPath,
		})
		broadcastEvent("status_update", map[string]interface{}{
			"id": resp.ID, "posterJid": ownJID, "fromMe": true, "text": data.Caption,
			"mediaType": data.Type, "timestamp": resp.Timestamp.Unix(),
		})
	}()
}

func handleMarkStatusViewed(raw json.RawMessage) {
	var data MarkStatusViewedData
	if err := json.Unmarshal(raw, &data); err != nil || data.ID == "" || data.PosterJID == "" || data.PosterJID == "me" {
		return
	}
	db.Exec("UPDATE messages SET viewed = 1 WHERE id = ? AND jid = 'status@broadcast'", data.ID)
	go func() {
		poster, err := types.ParseJID(data.PosterJID)
		if err != nil || poster.IsEmpty() {
			return
		}
		if wa().Store.ID != nil && poster.User == wa().Store.ID.User {
			return
		}
		ensureAvailableForRead()
		_ = wa().MarkRead(context.Background(), []types.MessageID{types.MessageID(data.ID)}, time.Now(), types.StatusBroadcastJID, poster, types.ReceiptTypeRead)
	}()
}
