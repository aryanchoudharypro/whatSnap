package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"image"
	"image/color"
	_ "image/gif"
	"image/jpeg"
	_ "image/png"
	"log"
	"net"
	"os"
	"strings"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
)

const channelPictureSize = 640

const channelFollowersLimit = 500

const channelNoticeAcceptedKey = "channel_creation_notice_accepted"

func (acc *Account) acceptChannelNotice(ctx context.Context) {
	var done int
	if acc.db.QueryRow(`SELECT COUNT(*) FROM app_settings WHERE key = ?`, channelNoticeAcceptedKey).Scan(&done) == nil && done > 0 {
		return
	}
	if err := acc.wa().AcceptTOSNotice(ctx, whatsmeow.NewsletterCreationNoticeID, "5"); err != nil {
		log.Printf("Couldn't accept the channel creation notice: %v", err)
		return
	}
	acc.db.Exec(`INSERT OR REPLACE INTO app_settings (key, value) VALUES (?, '1')`, channelNoticeAcceptedKey)
}

func channelPictureJPEG(path string) ([]byte, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	src, _, err := image.Decode(f)
	if err != nil {
		return nil, fmt.Errorf("this picture can't be used (only JPEG, PNG and GIF files can): %w", err)
	}
	b := src.Bounds()
	side := min(b.Dx(), b.Dy())
	if side == 0 {
		return nil, fmt.Errorf("the picture is empty")
	}
	crop := image.Rectangle{Min: image.Pt(b.Min.X+(b.Dx()-side)/2, b.Min.Y+(b.Dy()-side)/2)}
	crop.Max = crop.Min.Add(image.Pt(side, side))
	out := min(side, channelPictureSize)
	dst := image.NewRGBA(image.Rect(0, 0, out, out))
	for y := 0; y < out; y++ {
		sy0, sy1 := crop.Min.Y+y*side/out, crop.Min.Y+(y+1)*side/out
		for x := 0; x < out; x++ {
			sx0, sx1 := crop.Min.X+x*side/out, crop.Min.X+(x+1)*side/out
			var r, g, bl, n uint64
			for sy := sy0; sy < max(sy1, sy0+1); sy++ {
				for sx := sx0; sx < max(sx1, sx0+1); sx++ {
					cr, cg, cb, _ := src.At(sx, sy).RGBA()
					r, g, bl, n = r+uint64(cr), g+uint64(cg), bl+uint64(cb), n+1
				}
			}
			dst.Set(x, y, color.RGBA64{uint16(r / n), uint16(g / n), uint16(bl / n), 0xffff})
		}
	}
	var buf bytes.Buffer
	if err := jpeg.Encode(&buf, dst, &jpeg.Options{Quality: 85}); err != nil {
		return nil, err
	}
	return buf.Bytes(), nil
}

func (acc *Account) handleCreateChannel(conn net.Conn, raw json.RawMessage) {
	var data struct {
		Name        string `json:"name"`
		Description string `json:"description"`
		Picture     string `json:"picture"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	name := strings.TrimSpace(data.Name)
	if name == "" {
		return
	}
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
		defer cancel()
		params := whatsmeow.CreateNewsletterParams{Name: name, Description: strings.TrimSpace(data.Description)}
		if data.Picture != "" {
			picture, err := channelPictureJPEG(data.Picture)
			if err != nil {
				acc.sendEvent(conn, "error", map[string]string{"message": "Couldn't use the picture: " + err.Error()})
				return
			}
			params.Picture = picture
		}
		acc.acceptChannelNotice(ctx)
		meta, err := acc.wa().CreateNewsletter(ctx, params)
		if err != nil {
			acc.sendEvent(conn, "error", map[string]string{"message": "Couldn't create the channel: " + err.Error()})
			return
		}
		jid := meta.ID.String()
		acc.db.Exec(`INSERT INTO chats (jid, name, timestamp, is_channel, channel_role) VALUES (?, ?, ?, 1, 'owner')
			ON CONFLICT(jid) DO UPDATE SET name = excluded.name, is_channel = 1, channel_role = 'owner'`,
			jid, name, time.Now().Unix())
		acc.broadcastChats()
		acc.sendEvent(conn, "channel_created", map[string]string{"jid": jid, "name": name})
		go acc.syncFollowedChannels()
	}()
}

func (acc *Account) handleUpdateChannel(conn net.Conn, raw json.RawMessage) {
	var data struct {
		JID           string  `json:"jid"`
		Name          *string `json:"name"`
		Description   *string `json:"description"`
		Picture       string  `json:"picture"`
		RemovePicture bool    `json:"removePicture"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
		defer cancel()
		update := whatsmeow.NewsletterUpdate{Name: data.Name, Description: data.Description}
		if data.Name != nil {
			trimmed := strings.TrimSpace(*data.Name)
			if trimmed == "" {
				return
			}
			update.Name = &trimmed
		}
		switch {
		case data.RemovePicture:
			empty := []byte{}
			update.Picture = &empty
		case data.Picture != "":
			picture, err := channelPictureJPEG(data.Picture)
			if err != nil {
				acc.sendEvent(conn, "error", map[string]string{"message": "Couldn't use the picture: " + err.Error()})
				return
			}
			update.Picture = &picture
		}
		if err := acc.wa().UpdateNewsletter(ctx, jid, update); err != nil {
			acc.sendEvent(conn, "error", map[string]string{"message": "Couldn't change the channel: " + err.Error()})
			return
		}
		if update.Name != nil {
			acc.db.Exec(`UPDATE chats SET name = ? WHERE jid = ?`, *update.Name, data.JID)
			acc.broadcastChats()
		}
		if update.Picture != nil {
			acc.updateContactAvatarCache(data.JID, "", "")
			acc.broadcastEvent("avatar_updated", map[string]interface{}{"jid": data.JID, "removed": data.RemovePicture})
		}
		what := "Channel updated."
		switch {
		case data.RemovePicture:
			what = "Channel picture removed."
		case data.Picture != "":
			what = "Channel picture changed."
		}
		acc.sendEvent(conn, "channel_updated", map[string]string{"jid": data.JID, "message": what})
	}()
}

func (acc *Account) handleDeleteChannel(conn net.Conn, raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		if err := acc.wa().DeleteNewsletter(ctx, jid); err != nil {
			acc.sendEvent(conn, "error", map[string]string{"message": "Couldn't delete the channel: " + err.Error()})
			return
		}
		acc.purgeMessages("jid = ?", data.JID)
		acc.db.Exec(`DELETE FROM chats WHERE jid = ?`, data.JID)
		acc.broadcastChats()
		acc.sendEvent(conn, "channel_deleted", map[string]string{"jid": data.JID})
	}()
}

func (acc *Account) handleGetChannelFollowers(conn net.Conn, raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		followers, err := acc.wa().GetNewsletterFollowers(ctx, jid, channelFollowersLimit)
		if err != nil {
			acc.sendEvent(conn, "error", map[string]string{"message": "Couldn't load the channel's followers: " + err.Error()})
			return
		}
		list := make([]map[string]interface{}, 0, len(followers))
		for _, f := range followers {
			list = append(list, map[string]interface{}{
				"jid":  f.JID.String(),
				"name": acc.channelFollowerName(f),
				"role": strings.ToLower(string(f.Role)),
			})
		}
		acc.sendEvent(conn, "channel_followers", map[string]interface{}{"jid": data.JID, "followers": list})
	}()
}

func (acc *Account) channelFollowerName(f whatsmeow.NewsletterFollower) string {
	for _, jid := range []types.JID{f.PhoneJID, f.JID} {
		if jid.IsEmpty() {
			continue
		}
		if name := acc.resolveDisplayName(jid); name != "" && name != jid.String() && !strings.HasPrefix(name, "+") {
			return name
		}
	}
	switch {
	case f.DisplayName != "":
		return f.DisplayName
	case f.Username != "":
		return "@" + f.Username
	case !f.PhoneJID.IsEmpty():
		return "+" + f.PhoneJID.User
	}
	return "Unknown follower"
}

func (acc *Account) handleChannelAdminChange(conn net.Conn, raw json.RawMessage, makeOwner bool) {
	var data struct {
		JID  string `json:"jid"`
		User string `json:"user"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	user, err := types.ParseJID(data.User)
	if err != nil {
		return
	}
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		what := "Admin removed."
		if makeOwner {
			err = acc.wa().ChangeNewsletterOwner(ctx, jid, user)
			what = "Ownership handed over. You're now an admin."
		} else {
			err = acc.wa().DemoteNewsletterAdmin(ctx, jid, user)
		}
		if err != nil {
			acc.sendEvent(conn, "error", map[string]string{"message": "Couldn't change the channel's admins: " + err.Error()})
			return
		}
		if makeOwner {
			acc.db.Exec(`UPDATE chats SET channel_role = 'admin' WHERE jid = ?`, data.JID)
			acc.broadcastChats()
		}
		acc.sendEvent(conn, "channel_updated", map[string]string{"jid": data.JID, "message": what})
	}()
}

const adminInviteCaption = "I'd like you to be an admin of my WhatsApp channel."

func (acc *Account) handleInviteChannelAdmins(conn net.Conn, raw json.RawMessage) {
	var data struct {
		JID     string   `json:"jid"`
		Users   []string `json:"users"`
		Caption string   `json:"caption"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	channel, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	caption := strings.TrimSpace(data.Caption)
	if caption == "" {
		caption = adminInviteCaption
	}
	go func() {
		var channelName string
		acc.db.QueryRow(`SELECT COALESCE(name, '') FROM chats WHERE jid = ?`, data.JID).Scan(&channelName)
		sent, failed := 0, []string{}
		for _, userStr := range data.Users {
			user, err := types.ParseJID(userStr)
			if err != nil {
				continue
			}
			if err := acc.sendChannelAdminInvite(channel, channelName, user, caption); err != nil {
				log.Printf("Couldn't invite %s to be an admin of %s: %v", user, data.JID, err)
				failed = append(failed, acc.resolveDisplayName(user)+" ("+err.Error()+")")
				continue
			}
			sent++
		}
		if len(failed) > 0 {
			acc.sendEvent(conn, "error", map[string]string{"message": "Couldn't send an admin invite to " + strings.Join(failed, ", ")})
		}
		if sent > 0 {
			what := "Admin invite sent."
			if sent > 1 {
				what = fmt.Sprintf("Admin invites sent to %d people.", sent)
			}
			acc.sendEvent(conn, "channel_notice", map[string]string{"jid": data.JID, "message": what})
		}
	}()
}

func (acc *Account) sendChannelAdminInvite(channel types.JID, channelName string, user types.JID, caption string) error {
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	chat := user.ToNonAD()
	if chat.Server == types.HiddenUserServer {
		if pn, err := acc.wa().Store.LIDs.GetPNForLID(ctx, chat); err == nil && !pn.IsEmpty() {
			chat = pn
		}
	}
	expiration, err := acc.wa().CreateNewsletterAdminInvite(ctx, channel, chat)
	if err != nil {
		return err
	}
	invite := &waE2E.NewsletterAdminInviteMessage{
		NewsletterJID:  proto.String(channel.String()),
		NewsletterName: proto.String(channelName),
		Caption:        proto.String(caption),
	}
	if !expiration.IsZero() {
		invite.InviteExpiration = proto.Int64(expiration.Unix())
	}
	msg := &waE2E.Message{NewsletterAdminInviteMessage: invite}
	resp, err := acc.wa().SendMessage(ctx, chat, msg)
	if err != nil {
		return err
	}
	acc.storeOwnSentMessage(chat.String(), resp.ID, msg, resp.Timestamp.Unix())
	return nil
}

func (acc *Account) storeOwnSentMessage(chatJID, id string, msg *waE2E.Message, ts int64) {
	senderJID := ""
	if acc.wa().Store.ID != nil {
		senderJID = acc.wa().Store.ID.ToNonAD().String()
	}
	text := acc.extractText(msg)
	raw, _ := proto.Marshal(msg)
	acc.saveMessage(StoredMessage{ID: id, JID: chatJID, SenderJID: senderJID, FromMe: true, Text: text, Timestamp: ts, Raw: raw, Status: "sent"})
	acc.db.Exec(`INSERT INTO chats (jid, timestamp, preview_snippet, preview_sender, preview_from_me) VALUES (?, ?, ?, ?, 1)
		ON CONFLICT(jid) DO UPDATE SET
			timestamp=MAX(timestamp, excluded.timestamp),
			preview_snippet=excluded.preview_snippet,
			preview_sender=excluded.preview_sender,
			preview_from_me=1,
			preview_is_reaction=0,
			preview_reaction_emoji=''`,
		chatJID, ts, text, senderJID)
	acc.broadcastEvent("message", map[string]interface{}{
		"jid": chatJID, "from": "me", "id": id, "text": text, "from_me": true,
		"timestamp": ts, "has_media": false, "status": "sent", "sentHere": true,
	})
	acc.broadcastChats()
}

func (acc *Account) adminInviteFromMessage(messageID string) (*waE2E.NewsletterAdminInviteMessage, string, bool, error) {
	raw, chatJID, _, fromMe, err := acc.getMessageRaw(messageID)
	if err != nil || len(raw) == 0 {
		return nil, "", false, fmt.Errorf("message not found")
	}
	var msg waE2E.Message
	if err := proto.Unmarshal(raw, &msg); err != nil {
		return nil, "", false, fmt.Errorf("failed to parse message")
	}
	invite := unwrapMessage(&msg).GetNewsletterAdminInviteMessage()
	if invite == nil {
		return nil, "", false, fmt.Errorf("message is not a channel admin invite")
	}
	return invite, chatJID, fromMe, nil
}

func (acc *Account) handleGetChannelAdminInvite(conn net.Conn, raw json.RawMessage) {
	var data MessageRefData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		invite, chatJID, fromMe, err := acc.adminInviteFromMessage(data.MessageID)
		if err != nil {
			acc.sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}
		userName := chatJID
		if parsed, err := types.ParseJID(chatJID); err == nil {
			userName = acc.resolveDisplayName(parsed)
		}
		expiry := invite.GetInviteExpiration()
		acc.sendEvent(conn, "channel_admin_invite", map[string]interface{}{
			"messageId": data.MessageID,
			"jid":       invite.GetNewsletterJID(),
			"name":      invite.GetNewsletterName(),
			"caption":   invite.GetCaption(),
			"expired":   expiry > 0 && time.Unix(expiry, 0).Before(time.Now()),
			"fromMe":    fromMe,
			"user":      chatJID,
			"userName":  userName,
		})
	}()
}

func (acc *Account) handleChannelAdminInviteAnswer(conn net.Conn, raw json.RawMessage, accept bool) {
	var data struct {
		JID  string `json:"jid"`
		User string `json:"user"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	channel, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		if accept {
			if err := acc.wa().AcceptNewsletterAdminInvite(ctx, channel); err != nil {
				acc.sendEvent(conn, "error", map[string]string{"message": "Couldn't accept the invite: " + err.Error()})
				return
			}
			acc.syncFollowedChannels()
			acc.sendEvent(conn, "channel_notice", map[string]string{"jid": data.JID, "message": "You're now an admin of this channel."})
			return
		}
		user, err := types.ParseJID(data.User)
		if err != nil {
			return
		}
		if err := acc.wa().RevokeNewsletterAdminInvite(ctx, channel, user); err != nil {
			acc.sendEvent(conn, "error", map[string]string{"message": "Couldn't cancel the invite: " + err.Error()})
			return
		}
		acc.sendEvent(conn, "channel_notice", map[string]string{"jid": data.JID, "message": "Admin invite cancelled."})
	}()
}
