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
	"go.mau.fi/whatsmeow/types"
)

// Creating and running our own channels: making one, changing its name,
// description and picture, deleting it, and seeing and managing who follows
// it. Only a channel's owner and admins get these; the server enforces it.

// channelPictureSize is the side of the square JPEG a channel picture is sent
// as, the size WhatsApp uses for profile pictures.
const channelPictureSize = 640

// channelFollowersLimit is how many followers the followers list asks for.
const channelFollowersLimit = 500

const channelNoticeAcceptedKey = "channel_creation_notice_accepted"

// acceptChannelNotice accepts WhatsApp's channel-creation notice once, the
// way the official apps do before someone's first channel.
func acceptChannelNotice(ctx context.Context) {
	var done int
	if db.QueryRow(`SELECT COUNT(*) FROM app_settings WHERE key = ?`, channelNoticeAcceptedKey).Scan(&done) == nil && done > 0 {
		return
	}
	if err := client.AcceptTOSNotice(ctx, whatsmeow.NewsletterCreationNoticeID, "5"); err != nil {
		log.Printf("Couldn't accept the channel creation notice: %v", err)
		return
	}
	db.Exec(`INSERT OR REPLACE INTO app_settings (key, value) VALUES (?, '1')`, channelNoticeAcceptedKey)
}

// channelPictureJPEG reads an image file and makes it a square JPEG: the
// middle of the picture, scaled down to channelPictureSize.
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
	// Each output pixel is the average of the source pixels it covers.
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

func handleCreateChannel(conn net.Conn, raw json.RawMessage) {
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
				sendEvent(conn, "error", map[string]string{"message": "Couldn't use the picture: " + err.Error()})
				return
			}
			params.Picture = picture
		}
		acceptChannelNotice(ctx)
		meta, err := client.CreateNewsletter(ctx, params)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": "Couldn't create the channel: " + err.Error()})
			return
		}
		jid := meta.ID.String()
		db.Exec(`INSERT INTO chats (jid, name, timestamp, is_channel, channel_role) VALUES (?, ?, ?, 1, 'owner')
			ON CONFLICT(jid) DO UPDATE SET name = excluded.name, is_channel = 1, channel_role = 'owner'`,
			jid, name, time.Now().Unix())
		broadcastChats()
		sendEvent(conn, "channel_created", map[string]string{"jid": jid, "name": name})
		go syncFollowedChannels()
	}()
}

func handleUpdateChannel(conn net.Conn, raw json.RawMessage) {
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
				sendEvent(conn, "error", map[string]string{"message": "Couldn't use the picture: " + err.Error()})
				return
			}
			update.Picture = &picture
		}
		if err := client.UpdateNewsletter(ctx, jid, update); err != nil {
			sendEvent(conn, "error", map[string]string{"message": "Couldn't change the channel: " + err.Error()})
			return
		}
		if update.Name != nil {
			db.Exec(`UPDATE chats SET name = ? WHERE jid = ?`, *update.Name, data.JID)
			broadcastChats()
		}
		if update.Picture != nil {
			updateContactAvatarCache(data.JID, "", "")
			broadcastEvent("avatar_updated", map[string]interface{}{"jid": data.JID, "removed": data.RemovePicture})
		}
		what := "Channel updated."
		switch {
		case data.RemovePicture:
			what = "Channel picture removed."
		case data.Picture != "":
			what = "Channel picture changed."
		}
		sendEvent(conn, "channel_updated", map[string]string{"jid": data.JID, "message": what})
	}()
}

func handleDeleteChannel(conn net.Conn, raw json.RawMessage) {
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
		if err := client.DeleteNewsletter(ctx, jid); err != nil {
			sendEvent(conn, "error", map[string]string{"message": "Couldn't delete the channel: " + err.Error()})
			return
		}
		purgeMessages("jid = ?", data.JID)
		db.Exec(`DELETE FROM chats WHERE jid = ?`, data.JID)
		broadcastChats()
		sendEvent(conn, "channel_deleted", map[string]string{"jid": data.JID})
	}()
}

func handleGetChannelFollowers(conn net.Conn, raw json.RawMessage) {
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
		followers, err := client.GetNewsletterFollowers(ctx, jid, channelFollowersLimit)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": "Couldn't load the channel's followers: " + err.Error()})
			return
		}
		list := make([]map[string]interface{}, 0, len(followers))
		for _, f := range followers {
			list = append(list, map[string]interface{}{
				"jid":  f.JID.String(),
				"name": channelFollowerName(f),
				"role": strings.ToLower(string(f.Role)),
			})
		}
		sendEvent(conn, "channel_followers", map[string]interface{}{"jid": data.JID, "followers": list})
	}()
}

// channelFollowerName is who a follower is: our name for them if they're a
// contact, else the name they gave, else their username or number.
func channelFollowerName(f whatsmeow.NewsletterFollower) string {
	for _, jid := range []types.JID{f.PhoneJID, f.JID} {
		if jid.IsEmpty() {
			continue
		}
		if name := resolveDisplayName(jid); name != "" && name != jid.String() && !strings.HasPrefix(name, "+") {
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

// handleChannelAdminChange hands the channel over to an admin, or makes an
// admin a plain follower again.
func handleChannelAdminChange(conn net.Conn, raw json.RawMessage, makeOwner bool) {
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
			err = client.ChangeNewsletterOwner(ctx, jid, user)
			what = "Ownership handed over. You're now an admin."
		} else {
			err = client.DemoteNewsletterAdmin(ctx, jid, user)
		}
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": "Couldn't change the channel's admins: " + err.Error()})
			return
		}
		if makeOwner {
			db.Exec(`UPDATE chats SET channel_role = 'admin' WHERE jid = ?`, data.JID)
			broadcastChats()
		}
		sendEvent(conn, "channel_updated", map[string]string{"jid": data.JID, "message": what})
	}()
}
