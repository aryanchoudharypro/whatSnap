package main

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"net"
	"sort"
	"strings"
	"sync"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

// WhatsApp channels (whatsmeow calls them newsletters): one-way broadcasts
// that people follow. They live in the chats table with is_channel set, and
// their posts in messages with the post's server_id, which reactions and view
// receipts are addressed by. Channel history isn't end-to-end encrypted or
// synced from the phone; it's fetched from the server when a channel opens.

// channelPageSize is how many recent posts are fetched when a channel opens,
// the same page WhatsApp Web loads.
const channelPageSize = 50

func isChannelJID(jid string) bool {
	return strings.HasSuffix(jid, "@"+types.NewsletterServer)
}

// syncFollowedChannels mirrors the channels we follow into the chat list, and
// drops the ones we no longer follow.
func syncFollowedChannels() {
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	channels, err := client.GetSubscribedNewsletters(ctx)
	if err != nil {
		log.Printf("Couldn't list followed channels: %v", err)
		return
	}
	followed := make([]string, 0, len(channels))
	var writes []func(tx sqlExecer)
	for _, ch := range channels {
		if ch == nil {
			continue
		}
		jid := ch.ID.String()
		followed = append(followed, jid)
		name := ch.ThreadMeta.Name.Text
		role := ""
		muted := false
		if ch.ViewerMeta != nil {
			role = string(ch.ViewerMeta.Role)
			muted = ch.ViewerMeta.IsMuted()
		}
		writes = append(writes, func(tx sqlExecer) {
			tx.Exec(`INSERT INTO chats (jid, name, timestamp, is_channel, channel_role, muted) VALUES (?, ?, 0, 1, ?, ?)
				ON CONFLICT(jid) DO UPDATE SET
					name = COALESCE(NULLIF(excluded.name, ''), chats.name),
					is_channel = 1,
					channel_role = excluded.channel_role,
					muted = excluded.muted`,
				jid, name, role, boolToInt(muted))
		})
	}
	writeBatch(writes)
	markChannelsFollowed(followed)
	broadcastChats()
}

// markChannelsFollowed removes channel rows for channels not in followed,
// along with their posts - a channel we stopped following isn't kept.
func markChannelsFollowed(followed []string) {
	keep := make(map[string]bool, len(followed))
	for _, jid := range followed {
		keep[jid] = true
	}
	rows, err := db.Query(`SELECT jid FROM chats WHERE COALESCE(is_channel, 0) = 1`)
	if err != nil {
		return
	}
	var gone []string
	for rows.Next() {
		var jid string
		if rows.Scan(&jid) == nil && !keep[jid] {
			gone = append(gone, jid)
		}
	}
	rows.Close()
	for _, jid := range gone {
		purgeMessages("jid = ?", jid)
		db.Exec(`DELETE FROM chats WHERE jid = ?`, jid)
	}
}

// channelRole is our role in a channel ("owner", "admin", "subscriber",
// "guest"), or "" if we don't follow it.
func channelRole(jid string) string {
	var role string
	db.QueryRow(`SELECT COALESCE(channel_role, '') FROM chats WHERE jid = ?`, jid).Scan(&role)
	return strings.ToLower(role)
}

func canPostInChannel(jid string) bool {
	role := channelRole(jid)
	return role == "owner" || role == "admin"
}

// channelPostID is the ID a fetched post is stored under: its own message
// ID, or one made from its server ID when the server leaves that out.
func channelPostID(jid string, msg *types.NewsletterMessage) string {
	if msg.MessageID != "" {
		return msg.MessageID
	}
	return fmt.Sprintf("nl_%s_%d", strings.TrimSuffix(jid, "@"+types.NewsletterServer), msg.MessageServerID)
}

// formatChannelStats renders a post's views and reaction totals, most-used
// reaction first: "1,204 views, 👍 31, ❤️ 12".
func formatChannelStats(views int, reactions map[string]int) string {
	var parts []string
	switch {
	case views == 1:
		parts = append(parts, "1 view")
	case views > 1:
		parts = append(parts, groupThousands(views)+" views")
	}
	type count struct {
		emoji string
		n     int
	}
	var counts []count
	for emoji, n := range reactions {
		if n > 0 {
			counts = append(counts, count{emoji, n})
		}
	}
	sort.Slice(counts, func(i, j int) bool {
		if counts[i].n != counts[j].n {
			return counts[i].n > counts[j].n
		}
		return counts[i].emoji < counts[j].emoji
	})
	for _, c := range counts {
		parts = append(parts, fmt.Sprintf("%s %s", c.emoji, groupThousands(c.n)))
	}
	return strings.Join(parts, ", ")
}

// channelStatsFromColumns renders stored counts for the message list.
func channelStatsFromColumns(views int, reactionsJSON string) string {
	var reactions map[string]int
	if reactionsJSON != "" {
		json.Unmarshal([]byte(reactionsJSON), &reactions)
	}
	return formatChannelStats(views, reactions)
}

func groupThousands(n int) string {
	s := fmt.Sprint(n)
	for i := len(s) - 3; i > 0; i -= 3 {
		s = s[:i] + "," + s[i:]
	}
	return s
}

// storeChannelCounts saves a post's views and reaction totals.
func storeChannelCounts(ex sqlExecer, jid string, msg *types.NewsletterMessage) {
	reactions, _ := json.Marshal(msg.ReactionCounts)
	ex.Exec(`UPDATE messages SET channel_views = CASE WHEN ? > 0 THEN ? ELSE channel_views END,
			channel_reactions = CASE WHEN ? != 'null' THEN ? ELSE channel_reactions END
		WHERE jid = ? AND server_id = ?`,
		msg.ViewsCount, msg.ViewsCount, string(reactions), string(reactions), jid, msg.MessageServerID)
}

var (
	channelFetchMu      sync.Mutex
	channelFetchRunning = map[string]bool{}
)

// refreshChannel fetches a channel's recent posts from the server when it's
// opened, stores the new ones and their counts, marks them viewed the way
// the official apps do when posts are shown, and asks the open chat to reload
// if anything changed. It also subscribes to live count updates for a while.
func refreshChannel(jid string) {
	channelFetchMu.Lock()
	if channelFetchRunning[jid] {
		channelFetchMu.Unlock()
		return
	}
	channelFetchRunning[jid] = true
	channelFetchMu.Unlock()
	defer func() {
		channelFetchMu.Lock()
		delete(channelFetchRunning, jid)
		channelFetchMu.Unlock()
	}()

	parsed, err := types.ParseJID(jid)
	if err != nil {
		return
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	posts, err := client.GetNewsletterMessages(ctx, parsed, &whatsmeow.GetNewsletterMessagesParams{Count: channelPageSize})
	if err != nil {
		log.Printf("Couldn't fetch posts for channel %s: %v", jid, err)
		broadcastEvent("error", map[string]string{"message": "Couldn't load this channel's posts: " + err.Error()})
		return
	}
	if _, err := client.NewsletterSubscribeLiveUpdates(ctx, parsed); err != nil {
		log.Printf("Couldn't subscribe to live updates for channel %s: %v", jid, err)
	}

	changed, newest, serverIDs := storeChannelPosts(jid, posts)
	if newest > 0 {
		db.Exec(`UPDATE chats SET timestamp = MAX(timestamp, ?) WHERE jid = ?`, newest, jid)
		UpdateChatSnippet(jid)
	}
	if len(serverIDs) > 0 {
		if err := client.NewsletterMarkViewed(ctx, parsed, serverIDs); err != nil {
			log.Printf("Couldn't mark channel %s posts viewed: %v", jid, err)
		}
	}
	broadcastChats()
	broadcastEvent("channel_messages_updated", map[string]interface{}{"jid": jid, "newPosts": changed})
}

// storeChannelPosts saves fetched posts not stored yet, and every post's
// counts. Returns how many were new, the newest post's time and all their
// server IDs.
func storeChannelPosts(jid string, posts []*types.NewsletterMessage) (changed int, newest int64, serverIDs []types.MessageServerID) {
	var writes []func(tx sqlExecer)
	for _, post := range posts {
		if post == nil {
			continue
		}
		serverIDs = append(serverIDs, post.MessageServerID)
		id := channelPostID(jid, post)
		ts := post.Timestamp.Unix()
		newest = max(newest, ts)
		var exists, deleted int
		var storedText string
		db.QueryRow(`SELECT COUNT(*), COALESCE(MAX(is_deleted), 0), COALESCE(MAX(text), '') FROM messages WHERE id = ?`, id).Scan(&exists, &deleted, &storedText)
		if exists > 0 && deleted == 0 {
			// Catch up on what happened to a post since we stored it.
			switch {
			case post.Edit == types.EditAttributeAdminRevoke:
				changed++
				markMessageDeleted(id, jid)
			case post.Edit == types.EditAttributeAdminEdit && post.Message != nil:
				if text := extractText(post.Message); text != storedText {
					changed++
					markMessageEdited(id, jid, post.Message, text, ts)
				}
			}
		}
		if exists > 0 && post.IsSender {
			db.Exec(`UPDATE messages SET from_me = 1 WHERE id = ? AND from_me = 0`, id)
		}
		if exists == 0 && post.Message != nil {
			changed++
			text := extractText(post.Message)
			mediaType, mimetype, filename, size := mediaInfoFromMessage(post.Message)
			raw, _ := proto.Marshal(post.Message)
			sm := StoredMessage{
				ID: id, JID: jid, SenderJID: jid, FromMe: post.IsSender, Text: text, Timestamp: ts, Raw: raw, Status: "read",
				MediaType: mediaType, MediaMimetype: mimetype, MediaFilename: filename, MediaSize: size,
				ServerID: int64(post.MessageServerID),
			}
			if err := saveMessage(sm); err != nil {
				log.Printf("Couldn't store channel post %s: %v", id, err)
				continue
			}
		}
		writes = append(writes, func(tx sqlExecer) { storeChannelCounts(tx, jid, post) })
	}
	writeBatch(writes)
	return changed, newest, serverIDs
}

// loadOlderChannelPosts fetches the page of posts before the oldest one we
// have - what Home at the top of a channel asks for. It answers the way the
// phone's history does, so the chat view pages them in the same way.
func loadOlderChannelPosts(jid string) {
	parsed, err := types.ParseJID(jid)
	if err != nil {
		return
	}
	var oldest int64
	db.QueryRow(`SELECT COALESCE(MIN(server_id), 0) FROM messages WHERE jid = ? AND server_id > 0`, jid).Scan(&oldest)
	params := &whatsmeow.GetNewsletterMessagesParams{Count: channelPageSize}
	if oldest > 0 {
		params.Before = types.MessageServerID(oldest)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	posts, err := client.GetNewsletterMessages(ctx, parsed, params)
	if err != nil {
		log.Printf("Couldn't fetch older posts for channel %s: %v", jid, err)
		broadcastEvent("no_more_history", map[string]string{"jid": jid})
		return
	}
	changed, _, _ := storeChannelPosts(jid, posts)
	log.Printf("Fetched %d older posts for channel %s (%d new)", len(posts), jid, changed)
	if changed == 0 {
		broadcastEvent("no_more_history", map[string]string{"jid": jid})
		return
	}
	broadcastEvent("history_chats_synced", map[string]interface{}{"jids": []string{jid}, "onDemand": true})
}

// handleChannelDirectory browses (no query) or searches the channel
// directory, a page at a time.
func handleChannelDirectory(conn net.Conn, raw json.RawMessage) {
	var data struct {
		Query  string `json:"query"`
		View   string `json:"view"`
		Cursor string `json:"cursor"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
		defer cancel()
		const pageSize = 30
		var page *whatsmeow.NewsletterDirectoryPage
		var err error
		query := strings.TrimSpace(data.Query)
		if query != "" {
			page, err = client.SearchNewsletterDirectory(ctx, query, data.Cursor, pageSize)
		} else {
			page, err = browseChannelDirectory(ctx, data.Cursor, pageSize)
		}
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": "Couldn't load channels: " + err.Error()})
			return
		}
		channels := make([]map[string]interface{}, 0, len(page.Channels))
		for _, ch := range page.Channels {
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
		sendEvent(conn, "channel_directory", map[string]interface{}{
			"query":      query,
			"view":       data.View,
			"append":     data.Cursor != "",
			"nextCursor": page.NextCursor,
			"channels":   channels,
		})
	}()
}

// handleChannelReaction reacts to a channel post, which is addressed by its
// server ID rather than being a message of its own like a chat reaction.
func handleChannelReaction(data ReactMessageData) {
	var serverID int64
	db.QueryRow(`SELECT COALESCE(server_id, 0) FROM messages WHERE id = ?`, data.MessageID).Scan(&serverID)
	if serverID == 0 {
		broadcastEvent("error", map[string]string{"message": "Can't react to this post: it hasn't finished loading from the channel."})
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	if err := client.NewsletterSendReaction(context.Background(), jid, types.MessageServerID(serverID), data.Emoji, ""); err != nil {
		broadcastEvent("error", map[string]string{"message": "Couldn't react: " + err.Error()})
		return
	}
	upsertReaction(data.MessageID, data.JID, "me", data.Emoji, time.Now().Unix())
	broadcastEvent("message_reaction", map[string]interface{}{
		"jid": data.JID, "message_id": data.MessageID, "reactor": "me", "reactor_name": "You", "emoji": data.Emoji,
	})
}

// handleChannelMute mutes or unmutes a channel's posts, which for channels
// is a setting on the channel rather than an app-state mute.
func handleChannelMute(jidStr string, muted bool) {
	jid, err := types.ParseJID(jidStr)
	if err != nil {
		return
	}
	if err := client.NewsletterToggleMute(context.Background(), jid, muted); err != nil {
		broadcastEvent("error", map[string]string{"message": "Couldn't change this channel's mute setting: " + err.Error()})
		broadcastChats()
		return
	}
	db.Exec(`UPDATE chats SET muted = ? WHERE jid = ?`, boolToInt(muted), jidStr)
	broadcastChats()
}

// channelInfoPayload is what the channel info dialog shows.
func channelInfoPayload(meta *types.NewsletterMetadata, code string) map[string]interface{} {
	role, muted := "", false
	if meta.ViewerMeta != nil {
		role = strings.ToLower(string(meta.ViewerMeta.Role))
		muted = meta.ViewerMeta.IsMuted()
	}
	following := role == "subscriber" || role == "admin" || role == "owner"
	if !following {
		// An invite lookup has no viewer data; what we have locally decides.
		r := channelRole(meta.ID.String())
		following = r != "" && r != "guest"
		if following {
			role = r
		}
	}
	link := ""
	if meta.ThreadMeta.InviteCode != "" {
		link = whatsmeow.NewsletterLinkPrefix + meta.ThreadMeta.InviteCode
	}
	return map[string]interface{}{
		"jid":         meta.ID.String(),
		"code":        code,
		"name":        meta.ThreadMeta.Name.Text,
		"description": meta.ThreadMeta.Description.Text,
		"followers":   meta.ThreadMeta.SubscriberCount,
		"verified":    meta.ThreadMeta.VerificationState == types.NewsletterVerificationStateVerified,
		"role":        role,
		"following":   following,
		"muted":       muted,
		"link":        link,
		"state":       string(meta.State.Type),
	}
}

// handleGetChannelInfo looks a channel up by JID or by invite code (a
// whatsapp.com/channel/ link).
func handleGetChannelInfo(conn net.Conn, raw json.RawMessage) {
	var data struct {
		JID  string `json:"jid"`
		Code string `json:"code"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
		defer cancel()
		var meta *types.NewsletterMetadata
		var err error
		if data.Code != "" {
			meta, err = client.GetNewsletterInfoWithInvite(ctx, data.Code)
		} else if jid, perr := types.ParseJID(data.JID); perr == nil {
			meta, err = client.GetNewsletterInfo(ctx, jid)
		} else {
			return
		}
		if err != nil || meta == nil {
			msg := "Couldn't find this channel."
			if err != nil {
				msg = "Couldn't load this channel: " + err.Error()
			}
			sendEvent(conn, "error", map[string]string{"message": msg})
			return
		}
		sendEvent(conn, "channel_info", channelInfoPayload(meta, data.Code))
	}()
}

func handleFollowChannel(raw json.RawMessage, follow bool) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		if follow {
			err = client.FollowNewsletter(context.Background(), jid)
		} else {
			err = client.UnfollowNewsletter(context.Background(), jid)
		}
		if err != nil {
			verb := "follow"
			if !follow {
				verb = "unfollow"
			}
			broadcastEvent("error", map[string]string{"message": fmt.Sprintf("Couldn't %s this channel: %v", verb, err)})
			return
		}
		syncFollowedChannels()
		broadcastEvent("channel_follow_changed", map[string]interface{}{"jid": data.JID, "following": follow})
	}()
}

// handleChannelEvent keeps followed channels and post counts current from
// what the server pushes.
func handleChannelEvent(evt any) {
	switch v := evt.(type) {
	case *events.NewsletterJoin, *events.NewsletterLeave:
		go syncFollowedChannels()
	case *events.NewsletterMuteChange:
		muted := v.Mute == types.NewsletterMuteOn
		db.Exec(`UPDATE chats SET muted = ? WHERE jid = ?`, boolToInt(muted), v.ID.String())
		broadcastChats()
	case *events.NewsletterLiveUpdate:
		jid := v.JID.String()
		var writes []func(tx sqlExecer)
		for _, post := range v.Messages {
			if post != nil {
				writes = append(writes, func(tx sqlExecer) { storeChannelCounts(tx, jid, post) })
			}
		}
		writeBatch(writes)
		if len(writes) > 0 {
			broadcastEvent("channel_messages_updated", map[string]interface{}{"jid": jid, "newPosts": 0})
		}
	}
}

// isFollowedChannelPost reports whether a live message in jid is a post from
// a channel we follow, which is kept like a chat message. Other channel
// traffic stays ignored.
func isFollowedChannelPost(jid string) bool {
	if !isChannelJID(jid) {
		return false
	}
	var n int
	db.QueryRow(`SELECT COUNT(*) FROM chats WHERE jid = ? AND COALESCE(is_channel, 0) = 1`, jid).Scan(&n)
	return n > 0
}

// ownCountryCode is the ISO country of our own phone number ("IN" for a +91
// number), which picks whose channels the directory lists.
func ownCountryCode() string {
	if client == nil || client.Store == nil || client.Store.ID == nil {
		return ""
	}
	return regionForPhone("+" + client.Store.ID.User)
}

// regionForPhone is the ISO country of a number ("+919812345678" is "IN"):
// its calling code's main country, unless its leading digits belong to
// another one sharing that code (a Canadian +1 area code, say).
func regionForPhone(number string) string {
	digits := strings.TrimPrefix(number, "+")
	for n := min(len(digits), 6); n > 0; n-- {
		if region, ok := numberPrefixRegions[digits[:n]]; ok {
			return region
		}
	}
	for n := min(len(digits), 3); n > 0; n-- {
		if region, ok := callingCodeRegions[digits[:n]]; ok {
			return region
		}
	}
	return ""
}

// browseChannelDirectory lists channels to follow for our country: the
// directory's recommended view, falling back to the separate
// recommended-channels query WhatsApp Web also uses (which has no further
// pages) if the server won't take the first.
func browseChannelDirectory(ctx context.Context, cursor string, limit int) (*whatsmeow.NewsletterDirectoryPage, error) {
	var countries []string
	if cc := ownCountryCode(); cc != "" {
		countries = []string{cc}
	}
	page, err := client.GetNewsletterDirectory(ctx, whatsmeow.NewsletterDirectoryRecommended, countries, cursor, limit)
	if err == nil {
		return page, nil
	}
	log.Printf("Channel directory list failed (countries %v): %v; trying recommended channels", countries, err)
	if cursor != "" {
		return nil, err
	}
	page, fallbackErr := client.GetRecommendedNewsletters(ctx, countries, limit)
	if fallbackErr != nil {
		log.Printf("Recommended channels failed too: %v", fallbackErr)
		return nil, err
	}
	return page, nil
}
