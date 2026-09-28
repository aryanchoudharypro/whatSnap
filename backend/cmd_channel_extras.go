package main

import (
	"context"
	"encoding/json"
	"log"
	"net"
	"sync"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
)

// What a channel's owner and admins see beyond posting: pinned posts, who
// reacted to a post and who voted in a poll, and the admin invites still
// waiting. Plus the similar channels anyone sees on a channel's info.

// channelPostRef names a post in a channel by its stored message ID.
type channelPostRef struct {
	JID       string `json:"jid"`
	MessageID string `json:"message_id"`
}

// channelPost resolves a post reference to the channel and the post's
// server ID, which everything about channel posts goes by.
func channelPost(ref channelPostRef) (types.JID, types.MessageServerID, bool) {
	jid, err := types.ParseJID(ref.JID)
	if err != nil || jid.Server != types.NewsletterServer {
		return types.JID{}, 0, false
	}
	var serverID int64
	db.QueryRow(`SELECT COALESCE(server_id, 0) FROM messages WHERE id = ?`, ref.MessageID).Scan(&serverID)
	return jid, types.MessageServerID(serverID), serverID != 0
}

// channelPersonName is who someone in a channel is to us: a contact's name,
// else their number, when the server gives one.
func channelPersonName(jid types.JID) string {
	pn := preferPN(jid)
	for _, candidate := range []types.JID{pn, jid} {
		if name := resolveDisplayName(candidate); name != "" && name != candidate.String() {
			return name
		}
	}
	if pn.Server == types.DefaultUserServer {
		return "+" + pn.User
	}
	return "Someone not in your contacts"
}

// refreshChannelPins reads which posts are pinned from the channel's info.
func refreshChannelPins(ctx context.Context, jid types.JID) {
	meta, err := wa().GetNewsletterInfo(ctx, jid)
	if err != nil || meta == nil {
		if err != nil {
			log.Printf("Couldn't read pinned posts of channel %s: %v", jid, err)
		}
		return
	}
	storeChannelPins(jid.String(), meta.ThreadMeta.PinnedMessages)
}

// serverFlags are the server's feature flags for this account, fetched on
// each connect like the official apps do, so whaNTV only offers what the
// server has turned on.
var (
	serverFlagsMu sync.RWMutex
	serverFlags   whatsmeow.ABProps
)

func refreshServerFlags() {
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	props, err := wa().GetABProps(ctx)
	if err != nil {
		log.Printf("Couldn't fetch the server's feature flags: %v", err)
		return
	}
	serverFlagsMu.Lock()
	serverFlags = props
	serverFlagsMu.Unlock()
	log.Printf("Fetched %d server feature flags; channel pinning for admins: %v", len(props), channelPinningEnabled())
	broadcastEvent("server_flags", serverFlagsPayload())
}

// channelPinningEnabled is whether the server lets us pin posts in our
// channels (off by default, as in WhatsApp Web).
func channelPinningEnabled() bool {
	serverFlagsMu.RLock()
	defer serverFlagsMu.RUnlock()
	return serverFlags.Bool(whatsmeow.ABPropChannelsPinAdminEnabled, false)
}

func serverFlagsPayload() map[string]interface{} {
	return map[string]interface{}{"channelPinning": channelPinningEnabled()}
}

// handlePinChannelPost pins or unpins a post in a channel we own or run.
func handlePinChannelPost(conn net.Conn, raw json.RawMessage) {
	var data struct {
		channelPostRef
		Pin bool `json:"pin"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	if data.Pin && !channelPinningEnabled() {
		sendEvent(conn, "error", map[string]string{"message": "WhatsApp hasn't turned on pinning channel posts for your account yet."})
		return
	}
	jid, serverID, ok := channelPost(data.channelPostRef)
	if !ok {
		sendEvent(conn, "error", map[string]string{"message": "Can't pin this post: it hasn't finished loading from the channel."})
		return
	}
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		ids := []types.MessageServerID{serverID}
		var err error
		if data.Pin {
			_, err = wa().PinNewsletterMessages(ctx, jid, ids)
		} else {
			_, err = wa().UnpinNewsletterMessages(ctx, jid, ids)
		}
		if err != nil {
			verb := "pin"
			if !data.Pin {
				verb = "unpin"
			}
			sendEvent(conn, "error", map[string]string{"message": "Couldn't " + verb + " the post: " + err.Error()})
			return
		}
		db.Exec(`UPDATE messages SET channel_pinned = ? WHERE id = ?`, boolToInt(data.Pin), data.MessageID)
		// What the server now has pinned, in case pinning replaced another.
		refreshChannelPins(ctx, jid)
		what := "Post pinned."
		if !data.Pin {
			what = "Post unpinned."
		}
		sendEvent(conn, "channel_notice", map[string]string{"jid": data.JID, "message": what})
		broadcastEvent("channel_messages_updated", map[string]interface{}{"jid": data.JID, "newPosts": 0})
	}()
}

// handleGetChannelReactions lists who reacted to a post, by emoji.
func handleGetChannelReactions(conn net.Conn, raw json.RawMessage) {
	var data channelPostRef
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, serverID, ok := channelPost(data)
	if !ok {
		sendEvent(conn, "error", map[string]string{"message": "This post hasn't finished loading from the channel."})
		return
	}
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		reactions, err := wa().GetNewsletterReactionSenders(ctx, jid, serverID)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": "Couldn't load who reacted: " + err.Error()})
			return
		}
		groups := make([]map[string]interface{}, 0, len(reactions))
		for _, reaction := range reactions {
			names := make([]string, 0, len(reaction.Senders))
			for _, sender := range reaction.Senders {
				names = append(names, channelPersonName(sender))
			}
			groups = append(groups, map[string]interface{}{"label": reaction.Code, "names": names})
		}
		sendEvent(conn, "channel_people", map[string]interface{}{
			"title":  "Reactions",
			"kind":   "reactions",
			"groups": groups,
		})
	}()
}

// channelPollVotersLimit is how many voters are listed for each option.
const channelPollVotersLimit = 100

// handleGetChannelPollVoters lists who voted for each option of a poll in a
// channel we own or run.
func handleGetChannelPollVoters(conn net.Conn, raw json.RawMessage) {
	var data channelPostRef
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, serverID, ok := channelPost(data)
	p, isPoll := loadPoll(data.MessageID)
	if !ok || !isPoll {
		sendEvent(conn, "error", map[string]string{"message": "This poll hasn't finished loading from the channel."})
		return
	}
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
		defer cancel()
		groups := make([]map[string]interface{}, 0, len(p.options))
		for _, option := range p.options {
			voters, err := wa().GetNewsletterPollVoters(ctx, jid, serverID, option, channelPollVotersLimit)
			if err != nil {
				sendEvent(conn, "error", map[string]string{"message": "Couldn't load who voted: " + err.Error()})
				return
			}
			names := make([]string, 0, len(voters))
			for _, voter := range voters {
				names = append(names, channelPersonName(voter.JID))
			}
			groups = append(groups, map[string]interface{}{"label": option, "names": names})
		}
		sendEvent(conn, "channel_people", map[string]interface{}{
			"title":  "Votes: " + p.question,
			"kind":   "votes",
			"groups": groups,
		})
	}()
}

// handleGetChannelPendingInvites lists the admin invites of our channel no
// one has accepted yet.
func handleGetChannelPendingInvites(conn net.Conn, raw json.RawMessage) {
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
		invites, err := wa().GetNewsletterPendingAdminInvites(ctx, jid)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": "Couldn't load the pending admin invites: " + err.Error()})
			return
		}
		list := make([]map[string]string, 0, len(invites))
		for _, invite := range invites {
			who := invite.JID
			if !invite.Phone.IsEmpty() {
				who = invite.Phone
			}
			list = append(list, map[string]string{"jid": invite.JID.String(), "name": channelPersonName(who)})
		}
		sendEvent(conn, "channel_pending_invites", map[string]interface{}{"jid": data.JID, "invites": list})
	}()
}

// similarChannelsLimit is how many similar channels are suggested.
const similarChannelsLimit = 20

// handleGetSimilarChannels lists channels like one, answered as a page of
// the channel directory so the Find channels list can show them.
func handleGetSimilarChannels(conn net.Conn, raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
		defer cancel()
		similar, err := wa().GetSimilarNewsletters(ctx, jid, similarChannelsLimit)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": "Couldn't load similar channels: " + err.Error()})
			return
		}
		channels := make([]map[string]interface{}, 0, len(similar))
		for _, ch := range similar {
			role := channelRole(ch.ID.String())
			channels = append(channels, map[string]interface{}{
				"jid":         ch.ID.String(),
				"name":        ch.Name,
				"description": ch.Description,
				"followers":   ch.Subscribers,
				"verified":    ch.Verified,
				"following":   role != "" && role != "guest",
				"code":        ch.InviteCode,
			})
		}
		sendEvent(conn, "similar_channels", map[string]interface{}{"jid": data.JID, "channels": channels})
	}()
}

// forwardedChannelInfo says which channel a forwarded post came from.
func forwardedChannelInfo(channelJID, messageID string) *waE2E.ContextInfo_ForwardedNewsletterMessageInfo {
	var serverID int64
	var name string
	db.QueryRow(`SELECT COALESCE(server_id, 0) FROM messages WHERE id = ?`, messageID).Scan(&serverID)
	db.QueryRow(`SELECT COALESCE(name, '') FROM chats WHERE jid = ?`, channelJID).Scan(&name)
	info := &waE2E.ContextInfo_ForwardedNewsletterMessageInfo{
		NewsletterJID: proto.String(channelJID),
		ContentType:   waE2E.ContextInfo_ForwardedNewsletterMessageInfo_UPDATE.Enum(),
	}
	if serverID != 0 {
		info.ServerMessageID = proto.Int32(int32(serverID))
	}
	if name != "" {
		info.NewsletterName = proto.String(name)
	}
	return info
}

// mediaPart is a message's media, or nil if it has none.
func mediaPart(msg *waE2E.Message) whatsmeow.DownloadableMessage {
	switch {
	case msg.GetImageMessage() != nil:
		return msg.GetImageMessage()
	case msg.GetVideoMessage() != nil:
		return msg.GetVideoMessage()
	case msg.GetAudioMessage() != nil:
		return msg.GetAudioMessage()
	case msg.GetDocumentMessage() != nil:
		return msg.GetDocumentMessage()
	case msg.GetStickerMessage() != nil:
		return msg.GetStickerMessage()
	}
	return nil
}

// reuploadMedia downloads a message's media and uploads it again, for a
// channel (unencrypted, named by the returned handle) or for a chat
// (encrypted), and points the message at the new copy.
func reuploadMedia(ctx context.Context, msg *waE2E.Message, toChannel bool) (string, error) {
	media := mediaPart(msg)
	if media == nil {
		return "", nil
	}
	data, err := wa().Download(ctx, media)
	if err != nil {
		return "", err
	}
	mediaType := whatsmeow.GetMediaType(media)
	var up whatsmeow.UploadResponse
	if toChannel {
		up, err = wa().UploadNewsletter(ctx, data, mediaType)
	} else {
		up, err = wa().Upload(ctx, data, mediaType)
	}
	if err != nil {
		return "", err
	}
	url, directPath, length := proto.String(up.URL), proto.String(up.DirectPath), proto.Uint64(up.FileLength)
	switch m := media.(type) {
	case *waE2E.ImageMessage:
		m.URL, m.DirectPath, m.MediaKey, m.FileEncSHA256, m.FileSHA256, m.FileLength = url, directPath, up.MediaKey, up.FileEncSHA256, up.FileSHA256, length
	case *waE2E.VideoMessage:
		m.URL, m.DirectPath, m.MediaKey, m.FileEncSHA256, m.FileSHA256, m.FileLength = url, directPath, up.MediaKey, up.FileEncSHA256, up.FileSHA256, length
	case *waE2E.AudioMessage:
		m.URL, m.DirectPath, m.MediaKey, m.FileEncSHA256, m.FileSHA256, m.FileLength = url, directPath, up.MediaKey, up.FileEncSHA256, up.FileSHA256, length
	case *waE2E.DocumentMessage:
		m.URL, m.DirectPath, m.MediaKey, m.FileEncSHA256, m.FileSHA256, m.FileLength = url, directPath, up.MediaKey, up.FileEncSHA256, up.FileSHA256, length
	case *waE2E.StickerMessage:
		m.URL, m.DirectPath, m.MediaKey, m.FileEncSHA256, m.FileSHA256, m.FileLength = url, directPath, up.MediaKey, up.FileEncSHA256, up.FileSHA256, length
	}
	return up.Handle, nil
}
