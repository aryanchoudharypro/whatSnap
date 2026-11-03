package main

import (
	"archive/zip"
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"strings"
	"sync"
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

const recentStickerLimit = 50

type SendStickerData struct {
	JID         string `json:"jid"`
	MessageID   string `json:"message_id"`
	Hash        string `json:"hash"`
	PackID      string `json:"pack_id"`
	ReplyToID   string `json:"reply_to_id"`
	ReplySender string `json:"reply_to_sender"`
}

type StickerFavoriteData struct {
	MessageID string `json:"message_id"`
	Favorite  *bool  `json:"favorite"`
}

type stickerFileMeta struct {
	PackID        string   `json:"sticker-pack-id"`
	PackName      string   `json:"sticker-pack-name"`
	AuthorName    string   `json:"sticker-author-name"`
	Publisher     string   `json:"sticker-pack-publisher"`
	Emojis        []string `json:"emojis"`
	Accessibility string   `json:"accessibility-text"`
	FirstParty    int      `json:"is-first-party-sticker"`
}

func decodeStickerMeta(data []byte) *stickerFileMeta {
	if len(data) == 0 {
		return nil
	}
	var meta stickerFileMeta
	var typeErr *json.UnmarshalTypeError
	if err := json.Unmarshal(data, &meta); err != nil && !errors.As(err, &typeErr) {
		return nil
	}
	if meta.AuthorName == "" {
		meta.AuthorName = meta.Publisher
	}
	return &meta
}

func parseStickerMeta(data []byte) *stickerFileMeta {
	if len(data) >= 4 && string(data[0:4]) == "PK\x03\x04" {
		return parseLottieStickerMeta(data)
	}
	if len(data) < 12 || string(data[0:4]) != "RIFF" || string(data[8:12]) != "WEBP" {
		return nil
	}
	idx := 12
	for idx+8 <= len(data) {
		fourCC := string(data[idx : idx+4])
		chunkSize := int(uint32(data[idx+4]) | uint32(data[idx+5])<<8 | uint32(data[idx+6])<<16 | uint32(data[idx+7])<<24)
		idx += 8
		if chunkSize < 0 || idx+chunkSize > len(data) {
			break
		}
		if fourCC == "EXIF" {
			payload := data[idx : idx+chunkSize]
			start := bytes.IndexByte(payload, '{')
			end := bytes.LastIndexByte(payload, '}')
			if start != -1 && end > start {
				if meta := decodeStickerMeta(payload[start : end+1]); meta != nil {
					return meta
				}
			}
		}
		idx += chunkSize
		if chunkSize%2 != 0 {
			idx++
		}
	}
	return nil
}

func parseLottieStickerMeta(data []byte) *stickerFileMeta {
	zr, err := zip.NewReader(bytes.NewReader(data), int64(len(data)))
	if err != nil {
		return nil
	}
	read := func(f *zip.File, v any) bool {
		rc, err := f.Open()
		if err != nil {
			return false
		}
		defer rc.Close()
		return json.NewDecoder(io.LimitReader(rc, 16<<20)).Decode(v) == nil
	}
	var custom, overridden json.RawMessage
	for _, f := range zr.File {
		switch {
		case strings.HasSuffix(f.Name, "animation/animation.json"):
			var doc struct {
				Metadata struct {
					CustomProps json.RawMessage `json:"customProps"`
				} `json:"metadata"`
			}
			if read(f, &doc) {
				custom = doc.Metadata.CustomProps
			}
		case strings.HasSuffix(f.Name, "animation/animation.json.overridden_metadata"):
			read(f, &overridden)
		}
	}
	meta := decodeStickerMeta(custom)
	if over := decodeStickerMeta(overridden); over != nil {
		if meta == nil {
			return over
		}
		if over.Accessibility != "" {
			meta.Accessibility = over.Accessibility
		}
		if len(over.Emojis) > 0 {
			meta.Emojis = over.Emojis
		}
		meta.PackID = firstNonEmpty(over.PackID, meta.PackID)
		meta.PackName = firstNonEmpty(over.PackName, meta.PackName)
		if over.FirstParty != 0 {
			meta.FirstParty = over.FirstParty
		}
	}
	return meta
}

func labelStickerFromFile(st *waE2E.StickerMessage, data []byte) {
	if st == nil || st.GetAccessibilityLabel() != "" {
		return
	}
	meta := parseStickerMeta(data)
	if meta == nil {
		return
	}
	label := meta.Accessibility
	if label == "" && meta.FirstParty == 1 && meta.PackID != "" {
		hash := st.GetFileSHA256()
		if len(hash) != 32 {
			sum := sha256.Sum256(data)
			hash = sum[:]
		}
		label = storeStickerLabel(meta.PackID, hash)
	}
	if st.GetEmojis() == "" && len(meta.Emojis) > 0 {
		st.Emojis = proto.String(strings.Join(meta.Emojis, " "))
	}
	if label == "" && st.GetEmojis() == "" {
		label = meta.PackName
	}
	if label != "" {
		st.AccessibilityLabel = proto.String(label)
	}
}

func labelStickerFromCache(st *waE2E.StickerMessage, mediaPath string) {
	if st == nil || st.GetAccessibilityLabel() != "" || mediaPath == "" {
		return
	}
	if data, err := os.ReadFile(mediaPath); err == nil {
		labelStickerFromFile(st, data)
	}
}

func stickerText(sticker *waE2E.StickerMessage) string {
	if sticker == nil {
		return "[Sticker]"
	}
	desc := stickerDescription(sticker)
	if desc == "Sticker" {
		return "[Sticker]"
	}
	return "[Sticker] " + desc
}

func stickerHash(sticker *waE2E.StickerMessage) string {
	if len(sticker.GetFileSHA256()) != 32 {
		return ""
	}
	return base64.StdEncoding.EncodeToString(sticker.GetFileSHA256())
}

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

func findStickerByHash(hash string) *waE2E.StickerMessage {
	if hash == "" {
		return nil
	}
	rows, err := db.Query(`SELECT raw, COALESCE(media_path, '') FROM messages WHERE media_type = 'sticker' AND raw IS NOT NULL ORDER BY timestamp DESC LIMIT 200`)
	if err != nil {
		return nil
	}
	defer rows.Close()
	for rows.Next() {
		var raw []byte
		var mediaPath string
		if rows.Scan(&raw, &mediaPath) != nil {
			continue
		}
		var msg waE2E.Message
		if proto.Unmarshal(raw, &msg) != nil {
			continue
		}
		st := unwrapMessage(&msg).GetStickerMessage()
		if st != nil && stickerHash(st) == hash {
			labelStickerFromCache(st, mediaPath)
			return st
		}
	}
	return nil
}

type recentSticker struct {
	messageID string
	sticker   *waE2E.StickerMessage
}

func recentStickers() []recentSticker {
	rows, err := db.Query(`SELECT id, raw, COALESCE(media_path, '') FROM messages WHERE media_type = 'sticker' AND raw IS NOT NULL
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
		var mediaPath string
		if rows.Scan(&id, &raw, &mediaPath) != nil {
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
		if sticker.GetAccessibilityLabel() == "" {
			if mediaPath != "" {
				labelStickerFromCache(sticker, mediaPath)
			} else {
				startMediaDownload(MessageRefData{MessageID: id}, true)
			}
		}
		out = append(out, recentSticker{
			messageID: id,
			sticker:   sticker,
		})
	}
	return out
}

func stickerDescription(sticker *waE2E.StickerMessage) string {
	if sticker == nil {
		return "Sticker"
	}
	label := firstNonEmpty(sticker.GetAccessibilityLabel(), sticker.GetEmojis())
	if label == "" {
		if sticker.GetIsAvatar() {
			label = "Avatar sticker"
		} else if sticker.GetIsAiSticker() {
			label = "AI sticker"
		} else {
			label = "Sticker"
		}
	} else {
		if sticker.GetIsAvatar() && !strings.HasPrefix(strings.ToLower(label), "avatar") {
			label = "Avatar: " + label
		} else if sticker.GetIsAiSticker() && !strings.HasPrefix(strings.ToLower(label), "ai") {
			label = "AI sticker: " + label
		}
	}
	if sticker.GetIsAnimated() && !strings.Contains(label, "animated") {
		label = strings.TrimSuffix(label, ".") + ", animated"
	}
	return label
}

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
		label := ""
		if s := byHash[f.hash]; s != nil {
			label = stickerDescription(s)
		}
		if label == "" || label == "Sticker" {
			if s := findStickerByHash(f.hash); s != nil {
				label = stickerDescription(s)
			}
		}
		if label == "" || label == "Sticker" {
			if f.action.GetIsAvatarSticker() {
				label = "Avatar sticker"
			} else {
				label = "Sticker"
			}
			if f.action.GetIsLottie() && !strings.Contains(label, "animated") {
				label += ", animated"
			}
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

func favoriteAsSticker(ctx context.Context, hash string) (*waE2E.StickerMessage, error) {
	for _, r := range recentStickers() {
		if stickerHash(r.sticker) == hash {
			return r.sticker, nil
		}
	}
	if s := findStickerByHash(hash); s != nil {
		return s, nil
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
	labelStickerFromFile(sticker, data)
	return sticker, nil
}

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
		if data.PackID != "" {
			sticker, err = storePackSticker(ctx, data.PackID, data.Hash)
		} else if data.Hash != "" {
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
			"has_media": true, "mediaType": "sticker", "status": "sent", "sentHere": true,
			"quotedId": quotedID, "quotedText": quotedText, "quotedSender": quotedSender,
		})
		broadcastChats()
	}()
}

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

func isStickerFavorite(messageID string) bool {
	hash := stickerHash(storedSticker(messageID))
	if hash == "" {
		return false
	}
	var n int
	db.QueryRow(`SELECT COUNT(*) FROM sticker_favorites WHERE hash = ?`, hash).Scan(&n)
	return n > 0
}

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

func fixupLegacyStickerTexts() {
	for i := 0; i < 60 && wa() == nil; i++ {
		time.Sleep(time.Second)
	}
	rows, err := db.Query(`SELECT id, raw, COALESCE(media_path, ''), COALESCE(text, '') FROM messages WHERE media_type = 'sticker' AND raw IS NOT NULL
		AND (text = '[Sticker]' OR text = '' OR COALESCE(media_path, '') != '') ORDER BY timestamp DESC LIMIT 1000`)
	if err != nil {
		return
	}
	defer rows.Close()
	type fix struct {
		id   string
		text string
	}
	var fixes []fix
	for rows.Next() {
		var id string
		var raw []byte
		var mediaPath, text string
		if rows.Scan(&id, &raw, &mediaPath, &text) != nil {
			continue
		}
		var msg waE2E.Message
		if proto.Unmarshal(raw, &msg) != nil {
			continue
		}
		st := unwrapMessage(&msg).GetStickerMessage()
		if st == nil {
			continue
		}
		labelStickerFromCache(st, mediaPath)
		txt := stickerText(st)
		if txt != "[Sticker]" && txt != text {
			fixes = append(fixes, fix{id, txt})
		}
	}
	for _, f := range fixes {
		db.Exec(`UPDATE messages SET text = ? WHERE id = ?`, f.text, f.id)
	}

	rowsRecent, errRecent := db.Query(`SELECT id, jid FROM messages WHERE media_type = 'sticker' AND (media_path IS NULL OR media_path = '') AND COALESCE(is_deleted, 0) = 0 ORDER BY timestamp DESC LIMIT 50`)
	if errRecent == nil {
		defer rowsRecent.Close()
		for rowsRecent.Next() {
			var mID, mJID string
			if rowsRecent.Scan(&mID, &mJID) == nil {
				startMediaDownload(MessageRefData{MessageID: mID, JID: mJID}, true)
			}
		}
	}
}

const stickerStoreRetry = 10 * time.Minute

type stickerPackFetch struct {
	pack *types.StickerPack
	at   time.Time
}

var stickerStore struct {
	sync.Mutex
	packs []types.StickerPack
	at    time.Time
	data  map[string]stickerPackFetch
}

const stickerStoreLocale = "en"

func storeStickerPacks(ctx context.Context) ([]types.StickerPack, error) {
	stickerStore.Lock()
	defer stickerStore.Unlock()
	if stickerStore.packs != nil {
		return stickerStore.packs, nil
	}
	if time.Since(stickerStore.at) < time.Minute {
		return nil, fmt.Errorf("the sticker store isn't answering, try again in a minute")
	}
	country := firstNonEmpty(ownCountryCode(), "default")
	packs, err := wa().FetchStickerStore(ctx, stickerStoreLocale, country)
	stickerStore.at = time.Now()
	if err != nil {
		return nil, err
	}
	stickerStore.packs = packs
	return packs, nil
}

func storeStickerPack(ctx context.Context, id string) *types.StickerPack {
	if id == "" || wa() == nil {
		return nil
	}
	stickerStore.Lock()
	defer stickerStore.Unlock()
	if f, ok := stickerStore.data[id]; ok && (f.pack != nil || time.Since(f.at) < stickerStoreRetry) {
		return f.pack
	}
	ctx, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()
	pack, err := wa().FetchStickerPack(ctx, id)
	if err != nil {
		pack = nil
	}
	if stickerStore.data == nil {
		stickerStore.data = map[string]stickerPackFetch{}
	}
	stickerStore.data[id] = stickerPackFetch{pack, time.Now()}
	return pack
}

func storeStickerLabel(packID string, fileHash []byte) string {
	pack := storeStickerPack(context.Background(), packID)
	if pack == nil {
		return ""
	}
	for _, item := range pack.Stickers {
		if bytes.Equal(item.FileHash, fileHash) {
			return item.AccessibilityText
		}
	}
	return ""
}

func storeSticker(pack *types.StickerPack, item *types.StickerPackItem) *waE2E.StickerMessage {
	mimetype := firstNonEmpty(item.MimeType, "image/webp")
	st := &waE2E.StickerMessage{
		URL: proto.String(item.URL), DirectPath: proto.String(item.DirectPath),
		MediaKey: item.MediaKey, FileSHA256: item.FileHash, FileEncSHA256: item.EncFileHash,
		FileLength: proto.Uint64(uint64(item.FileSize)), Mimetype: proto.String(mimetype),
		IsAnimated: proto.Bool(pack.Animated == 1 || pack.Lottie == 1),
		IsLottie:   proto.Bool(mimetype == "application/was"),
	}
	if item.Width > 0 && item.Height > 0 {
		st.Width, st.Height = proto.Uint32(uint32(item.Width)), proto.Uint32(uint32(item.Height))
	}
	if len(item.Emojis) > 0 {
		st.Emojis = proto.String(strings.Join(item.Emojis, " "))
	}
	if item.AccessibilityText != "" {
		st.AccessibilityLabel = proto.String(item.AccessibilityText)
	}
	return st
}

type StickerPackRequest struct {
	ID string `json:"id"`
}

func handleGetStickerStore(conn net.Conn) {
	packs, err := storeStickerPacks(context.Background())
	if err != nil {
		sendEvent(conn, "sticker_store", map[string]interface{}{"error": "Couldn't load WhatsApp's sticker packs: " + err.Error()})
		return
	}
	out := make([]map[string]interface{}, 0, len(packs))
	for _, p := range packs {
		if p.StickerPackID == "" {
			continue
		}
		out = append(out, map[string]interface{}{
			"id": p.StickerPackID, "name": firstNonEmpty(p.Name, "Sticker pack"), "publisher": p.Publisher,
			"animated": p.Animated == 1 || p.Lottie == 1,
		})
	}
	sendEvent(conn, "sticker_store", map[string]interface{}{"packs": out})
}

func handleGetStickerPack(conn net.Conn, raw json.RawMessage) {
	var data StickerPackRequest
	if json.Unmarshal(raw, &data) != nil {
		return
	}
	pack := storeStickerPack(context.Background(), data.ID)
	if pack == nil {
		sendEvent(conn, "sticker_pack", map[string]interface{}{"id": data.ID, "error": "Couldn't load this sticker pack."})
		return
	}
	stickers := make([]map[string]string, 0, len(pack.Stickers))
	for _, item := range pack.Stickers {
		if len(item.FileHash) != 32 || item.DirectPath == "" {
			continue
		}
		stickers = append(stickers, map[string]string{
			"hash":  base64.StdEncoding.EncodeToString(item.FileHash),
			"label": stickerDescription(storeSticker(pack, item)),
		})
	}
	sendEvent(conn, "sticker_pack", map[string]interface{}{"id": data.ID, "stickers": stickers})
}

func storePackSticker(ctx context.Context, packID, hash string) (*waE2E.StickerMessage, error) {
	pack := storeStickerPack(ctx, packID)
	if pack == nil {
		return nil, fmt.Errorf("couldn't load its sticker pack")
	}
	for _, item := range pack.Stickers {
		if base64.StdEncoding.EncodeToString(item.FileHash) == hash {
			return storeSticker(pack, item), nil
		}
	}
	return nil, fmt.Errorf("that sticker isn't in its pack any more")
}
