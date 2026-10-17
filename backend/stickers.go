package main

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"net"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/proto/waSyncAction"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"golang.org/x/image/webp"
	"google.golang.org/protobuf/proto"
)

// Stickers: the picker's recent and favorite stickers, sending one again,
// and adding one to the favorites. Recent stickers are the ones seen in
// chats; favorites sync from the phone through app state, each named by the
// base64 SHA-256 of its file (WAWebStickersFavoriteSyncAction).

const recentStickerLimit = 30

type SendStickerData struct {
	JID         string `json:"jid"`
	MessageID   string `json:"message_id"` // a sticker message to send again
	Hash        string `json:"hash"`       // or a favorite, by file hash
	ReplyToID   string `json:"reply_to_id"`
	ReplySender string `json:"reply_to_sender"`
}

type StickerFavoriteData struct {
	MessageID string `json:"message_id"`
	Favorite  *bool  `json:"favorite"` // nil switches it
}

// stickerText is a sticker's text in a chat: "[Sticker]", and what it shows
// when the sender said ("[Sticker] 😂").
func stickerText(sticker *waE2E.StickerMessage) string {
	if label := firstNonEmpty(sticker.GetAccessibilityLabel(), sticker.GetEmojis()); label != "" {
		return "[Sticker] " + label
	}
	return "[Sticker]"
}

func stickerHash(sticker *waE2E.StickerMessage) string {
	if len(sticker.GetFileSHA256()) != 32 {
		return ""
	}
	return base64.StdEncoding.EncodeToString(sticker.GetFileSHA256())
}

// storedSticker is the sticker a stored message holds, or nil.
func storedSticker(messageID string) *waE2E.StickerMessage {
	raw, _, _, _, err := getMessageRaw(messageID)
	if err != nil || len(raw) == 0 {
		return nil
	}
	var msg waE2E.Message
	if proto.Unmarshal(raw, &msg) != nil {
		return nil
	}
	return unwrapMessage(&msg).GetStickerMessage()
}

type recentSticker struct {
	messageID string
	sticker   *waE2E.StickerMessage
}

// recentStickers are the stickers last seen in chats, newest first, one per
// sticker file.
func recentStickers() []recentSticker {
	rows, err := db.Query(`SELECT id, raw FROM messages WHERE media_type = 'sticker' AND raw IS NOT NULL
		AND COALESCE(is_deleted, 0) = 0 ORDER BY timestamp DESC LIMIT 500`)
	if err != nil {
		return nil
	}
	defer rows.Close()
	seen := map[string]bool{}
	var out []recentSticker
	for rows.Next() && len(out) < recentStickerLimit {
		var id string
		var raw []byte
		if rows.Scan(&id, &raw) != nil {
			continue
		}
		var msg waE2E.Message
		if proto.Unmarshal(raw, &msg) != nil {
			continue
		}
		sticker := unwrapMessage(&msg).GetStickerMessage()
		hash := stickerHash(sticker)
		if sticker == nil || hash == "" || seen[hash] || sticker.GetDirectPath() == "" {
			continue
		}
		seen[hash] = true
		out = append(out, recentSticker{id, sticker})
	}
	return out
}

func stickerDescription(sticker *waE2E.StickerMessage) string {
	label := firstNonEmpty(sticker.GetAccessibilityLabel(), sticker.GetEmojis())
	if label == "" {
		label = "Sticker"
	}
	if sticker.GetIsAnimated() {
		label += ", animated"
	}
	return label
}

// noteFavoriteSticker stores a favorite synced from another device.
func noteFavoriteSticker(v *events.FavoriteSticker) {
	if !v.Action.GetIsFavorite() {
		db.Exec(`DELETE FROM sticker_favorites WHERE hash = ?`, v.FileHash)
		return
	}
	raw, err := proto.Marshal(v.Action)
	if err != nil {
		return
	}
	db.Exec(`INSERT INTO sticker_favorites (hash, action, ts) VALUES (?, ?, ?)
		ON CONFLICT(hash) DO UPDATE SET action = excluded.action, ts = excluded.ts`, v.FileHash, raw, v.Timestamp.Unix())
}

type favoriteSticker struct {
	hash   string
	action *waSyncAction.StickerAction
}

// favoriteStickerHashes are the favorites' file hashes.
func favoriteStickerHashes() map[string]bool {
	hashes := map[string]bool{}
	rows, err := db.Query(`SELECT hash FROM sticker_favorites`)
	if err != nil {
		return hashes
	}
	defer rows.Close()
	for rows.Next() {
		var hash string
		if rows.Scan(&hash) == nil {
			hashes[hash] = true
		}
	}
	return hashes
}

func favoriteStickers() []favoriteSticker {
	rows, err := db.Query(`SELECT hash, action FROM sticker_favorites ORDER BY ts DESC`)
	if err != nil {
		return nil
	}
	defer rows.Close()
	var out []favoriteSticker
	for rows.Next() {
		var hash string
		var raw []byte
		if rows.Scan(&hash, &raw) != nil {
			continue
		}
		var action waSyncAction.StickerAction
		if proto.Unmarshal(raw, &action) == nil {
			out = append(out, favoriteSticker{hash, &action})
		}
	}
	return out
}

// handleGetStickers lists the picker's stickers: favorites first, then
// recent ones that aren't favorites.
func handleGetStickers(conn net.Conn) {
	recent := recentStickers()
	byHash := map[string]*waE2E.StickerMessage{}
	for _, r := range recent {
		byHash[stickerHash(r.sticker)] = r.sticker
	}
	favorites := []map[string]string{}
	isFavorite := map[string]bool{}
	for _, f := range favoriteStickers() {
		isFavorite[f.hash] = true
		label := "Sticker"
		if s := byHash[f.hash]; s != nil {
			label = stickerDescription(s)
		}
		favorites = append(favorites, map[string]string{"hash": f.hash, "label": label})
	}
	recents := []map[string]string{}
	for _, r := range recent {
		if !isFavorite[stickerHash(r.sticker)] {
			recents = append(recents, map[string]string{"message_id": r.messageID, "label": stickerDescription(r.sticker)})
		}
	}
	sendEvent(conn, "stickers", map[string]interface{}{"favorites": favorites, "recent": recents})
}

// favoriteAsSticker turns a favorite into a sticker message: the sticker as
// seen in a chat when there is one, or else downloaded from where the
// favorite says, which also gives the encrypted file's hash a message needs.
func favoriteAsSticker(ctx context.Context, hash string) (*waE2E.StickerMessage, error) {
	for _, r := range recentStickers() {
		if stickerHash(r.sticker) == hash {
			return r.sticker, nil
		}
	}
	var raw []byte
	if err := db.QueryRow(`SELECT action FROM sticker_favorites WHERE hash = ?`, hash).Scan(&raw); err != nil {
		return nil, fmt.Errorf("that sticker isn't a favorite any more")
	}
	var action waSyncAction.StickerAction
	if err := proto.Unmarshal(raw, &action); err != nil {
		return nil, err
	}
	fileHash, err := base64.StdEncoding.DecodeString(hash)
	if err != nil {
		return nil, err
	}
	data, encHash, err := wa().DownloadMediaWithoutEncHash(ctx, action.GetDirectPath(), fileHash, action.GetMediaKey(), whatsmeow.MediaImage, "")
	if err != nil {
		return nil, fmt.Errorf("couldn't download the sticker: %w", err)
	}
	width, height, animated := webpInfo(data)
	sticker := &waE2E.StickerMessage{
		URL: proto.String(action.GetURL()), DirectPath: proto.String(action.GetDirectPath()),
		MediaKey: action.GetMediaKey(), FileSHA256: fileHash, FileEncSHA256: encHash,
		FileLength: proto.Uint64(uint64(len(data))), Mimetype: proto.String(firstNonEmpty(action.GetMimetype(), "image/webp")),
		IsAnimated: proto.Bool(animated), IsLottie: proto.Bool(action.GetIsLottie()), IsAvatar: proto.Bool(action.GetIsAvatarSticker()),
	}
	if width > 0 {
		sticker.Width, sticker.Height = proto.Uint32(uint32(width)), proto.Uint32(uint32(height))
	}
	return sticker, nil
}

// handleSendSticker sends a recent or favorite sticker again. Its media is
// already on WhatsApp's servers, so it isn't uploaded again, as with a
// forward.
func handleSendSticker(conn net.Conn, raw json.RawMessage) {
	var data SendStickerData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		ctx := context.Background()
		var sticker *waE2E.StickerMessage
		if data.Hash != "" {
			sticker, err = favoriteAsSticker(ctx, data.Hash)
		} else if sticker = storedSticker(data.MessageID); sticker == nil {
			err = fmt.Errorf("that sticker isn't here any more")
		}
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": "Couldn't send the sticker: " + err.Error()})
			return
		}
		sticker = proto.Clone(sticker).(*waE2E.StickerMessage)
		ctxInfo, quotedID, quotedText, quotedSender := buildAttachmentContext(data.ReplyToID, data.ReplySender)
		sticker.ContextInfo = ctxInfo
		sticker.StickerSentTS = proto.Int64(time.Now().UnixMilli())
		msg := &waE2E.Message{StickerMessage: sticker}

		var extra whatsmeow.SendRequestExtra
		if jid.Server == types.NewsletterServer {
			if !canPostInChannel(jid.String()) {
				sendEvent(conn, "error", map[string]string{"message": "Only this channel's admins can post in it."})
				return
			}
			// Channel media isn't encrypted, so it goes up again.
			if extra.MediaHandle, err = reuploadMedia(ctx, msg, true); err != nil {
				sendEvent(conn, "error", map[string]string{"message": "Couldn't send the sticker: " + err.Error()})
				return
			}
		} else {
			msg = applyDisappearingTimer(jid, msg)
		}
		extra.ID = wa().GenerateMessageID()
		resp, err := wa().SendMessage(ctx, jid, msg, extra)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": "Couldn't send the sticker: " + sendErrorText(err)})
			return
		}
		ts := resp.Timestamp.Unix()
		text := stickerText(sticker)
		rawBytes, _ := proto.Marshal(msg)
		own := ownJIDString()
		saveMessage(StoredMessage{
			ID: resp.ID, JID: jid.String(), SenderJID: own, FromMe: true, Text: text, Timestamp: ts, Raw: rawBytes,
			Status: "sent", MediaType: "sticker", MediaMimetype: sticker.GetMimetype(), MediaSize: int64(sticker.GetFileLength()),
			QuotedID: quotedID, QuotedText: quotedText, QuotedSender: quotedSender, ServerID: int64(resp.ServerID),
		})
		noteMessageExpiry(db, resp.ID, ts, msg)
		db.Exec(`INSERT INTO chats (jid, timestamp, preview_snippet, preview_sender, preview_from_me) VALUES (?, ?, ?, ?, 1)
			ON CONFLICT(jid) DO UPDATE SET
			timestamp=MAX(timestamp, excluded.timestamp),
			preview_snippet=excluded.preview_snippet,
			preview_sender=excluded.preview_sender,
			preview_from_me=1,
			preview_is_reaction=0,
			preview_reaction_emoji=''`, jid.String(), ts, text, own)
		broadcastEvent("message", map[string]interface{}{
			"jid": jid.String(), "from": "me", "id": resp.ID, "text": text, "from_me": true, "timestamp": ts,
			"has_media": true, "mediaType": "sticker", "status": "sent",
			"quotedId": quotedID, "quotedText": quotedText, "quotedSender": quotedSender,
		})
		broadcastChats()
	}()
}

// handleSetStickerFavorite adds a sticker in a chat to the favorites, or
// takes it off, on every device.
func handleSetStickerFavorite(conn net.Conn, raw json.RawMessage) {
	var data StickerFavoriteData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		sticker := storedSticker(data.MessageID)
		hash := stickerHash(sticker)
		if hash == "" {
			sendEvent(conn, "error", map[string]string{"message": "That isn't a sticker that can be added to favorites."})
			return
		}
		action := &waSyncAction.StickerAction{
			URL: proto.String(sticker.GetURL()), DirectPath: proto.String(sticker.GetDirectPath()),
			MediaKey: sticker.GetMediaKey(), Mimetype: proto.String(sticker.GetMimetype()),
			Height: proto.Uint32(sticker.GetHeight()), Width: proto.Uint32(sticker.GetWidth()),
			FileLength: proto.Uint64(sticker.GetFileLength()), IsLottie: proto.Bool(sticker.GetIsLottie()),
			IsAvatarSticker: proto.Bool(sticker.GetIsAvatar()), ImageHash: proto.String(hash),
		}
		favorite := !isStickerFavorite(data.MessageID)
		if data.Favorite != nil {
			favorite = *data.Favorite
		}
		if err := sendAppStateWithRetry(context.Background(), appstate.BuildFavoriteSticker(hash, action, favorite)); err != nil {
			sendEvent(conn, "error", map[string]string{"message": "Couldn't change your favorite stickers: " + err.Error()})
			return
		}
		action.IsFavorite = proto.Bool(favorite)
		noteFavoriteSticker(&events.FavoriteSticker{FileHash: hash, Timestamp: time.Now(), Action: action})
		sendEvent(conn, "sticker_favorite", map[string]interface{}{"message_id": data.MessageID, "favorite": favorite})
	}()
}

// isStickerFavorite says whether a sticker in a chat is a favorite.
func isStickerFavorite(messageID string) bool {
	hash := stickerHash(storedSticker(messageID))
	if hash == "" {
		return false
	}
	var n int
	db.QueryRow(`SELECT COUNT(*) FROM sticker_favorites WHERE hash = ?`, hash).Scan(&n)
	return n > 0
}

// webpInfo reads a WebP's size and whether it's animated: from its VP8X
// header when it has one, which is also the only place animation is noted.
func webpInfo(data []byte) (width, height int, animated bool) {
	if len(data) >= 30 && string(data[0:4]) == "RIFF" && string(data[8:12]) == "WEBP" && string(data[12:16]) == "VP8X" {
		animated = data[20]&0x02 != 0
		width = int(uint32(data[24])|uint32(data[25])<<8|uint32(data[26])<<16) + 1
		height = int(uint32(data[27])|uint32(data[28])<<8|uint32(data[29])<<16) + 1
		return
	}
	if cfg, err := webp.DecodeConfig(bytes.NewReader(data)); err == nil {
		return cfg.Width, cfg.Height, false
	}
	return 0, 0, false
}
