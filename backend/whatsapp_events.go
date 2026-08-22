package main

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"log"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/proto/waWeb"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

func resolveLIDChat(v *events.Message) (chatJID, senderJID types.JID) {
	chatJID = v.Info.Chat.ToNonAD()
	senderJID = v.Info.Sender.ToNonAD()

	if senderJID.Server == "lid" {
		if !v.Info.SenderAlt.IsEmpty() {
			senderJID = v.Info.SenderAlt.ToNonAD()
		} else if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), senderJID); err == nil && !pn.IsEmpty() {
			senderJID = pn.ToNonAD()
		} else if v.Info.IsFromMe && client.Store.ID != nil {
			senderJID = client.Store.ID.ToNonAD()
		}
	}

	if chatJID.Server == "lid" {
		if v.Info.IsFromMe {
			if !v.Info.RecipientAlt.IsEmpty() {
				chatJID = v.Info.RecipientAlt.ToNonAD()
			} else if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), chatJID); err == nil && !pn.IsEmpty() {
				chatJID = pn.ToNonAD()
			}
		} else {
			chatJID = senderJID
		}
	} else if chatJID.Server == types.BroadcastServer && chatJID.User != "status" && !v.Info.IsFromMe {
		// Incoming broadcast-list messages belong in the sender's 1:1 chat,
		// like official WhatsApp, not in a chat keyed by the broadcast JID.
		chatJID = senderJID
	}

	return
}

// applyReaction stores an incoming reaction and updates the chat-list preview.
// It never creates a standalone chat message - if the reacted-to message isn't
// in the local DB, updateChatReactionPreview silently leaves the preview alone.
func applyReaction(v *events.Message, reaction *waE2E.ReactionMessage) {
	targetID := reaction.GetKey().GetID()
	chatJID, senderJID := resolveLIDChat(v)
	jid := chatJID.ToNonAD().String()
	emoji := reaction.GetText()

	reactorJID := senderJID.ToNonAD().String()
	reactorName := resolveDisplayName(senderJID)
	if v.Info.IsFromMe {
		reactorJID = "me"
		reactorName = "You"
	}

	ts := reaction.GetSenderTimestampMS() / 1000
	if ts == 0 {
		ts = v.Info.Timestamp.Unix()
	}

	go func() {
		upsertReaction(targetID, jid, reactorJID, emoji, ts)
		updateChatReactionPreview(jid, targetID, reactorJID, emoji, v.Info.IsFromMe)
		broadcastChats()
	}()
	broadcastEvent("message_reaction", map[string]interface{}{
		"jid":          jid,
		"message_id":   targetID,
		"reactor":      reactorJID,
		"reactor_name": reactorName,
		"emoji":        emoji,
	})
}

// storeReactionTx applies a HistorySync-backfilled reaction within the
// caller's transaction. Mirrors applyReaction but writes via tx and skips the
// live "message_reaction" broadcast, since this runs during bulk backfill.
func storeReactionTx(tx *sql.Tx, jid string, evtMsg *events.Message, reaction *waE2E.ReactionMessage) {
	targetID := reaction.GetKey().GetID()
	emoji := reaction.GetText()
	reactorJID := evtMsg.Info.Sender.ToNonAD().String()
	if evtMsg.Info.IsFromMe {
		reactorJID = "me"
	}
	ts := reaction.GetSenderTimestampMS() / 1000
	if ts == 0 {
		ts = evtMsg.Info.Timestamp.Unix()
	}
	if emoji == "" {
		tx.Exec(`DELETE FROM reactions WHERE message_id = ? AND (reactor_jid = ? OR reactor_jid = 'me')`, targetID, reactorJID)
	} else {
		tx.Exec(`INSERT INTO reactions (message_id, chat_jid, reactor_jid, emoji, timestamp) VALUES (?, ?, ?, ?, ?)
			ON CONFLICT(message_id, reactor_jid) DO UPDATE SET emoji = excluded.emoji, timestamp = excluded.timestamp`,
			targetID, jid, reactorJID, emoji, ts)
	}
}

func unwrapMessage(msg *waE2E.Message) *waE2E.Message {
	if msg == nil {
		return nil
	}
	for {
		if msg.GetEphemeralMessage() != nil && msg.GetEphemeralMessage().GetMessage() != nil {
			msg = msg.GetEphemeralMessage().GetMessage()
			continue
		}
		if msg.GetViewOnceMessage() != nil && msg.GetViewOnceMessage().GetMessage() != nil {
			msg = msg.GetViewOnceMessage().GetMessage()
			continue
		}
		if msg.GetViewOnceMessageV2() != nil && msg.GetViewOnceMessageV2().GetMessage() != nil {
			msg = msg.GetViewOnceMessageV2().GetMessage()
			continue
		}
		if msg.GetViewOnceMessageV2Extension() != nil && msg.GetViewOnceMessageV2Extension().GetMessage() != nil {
			msg = msg.GetViewOnceMessageV2Extension().GetMessage()
			continue
		}
		if msg.GetDocumentWithCaptionMessage() != nil && msg.GetDocumentWithCaptionMessage().GetMessage() != nil {
			msg = msg.GetDocumentWithCaptionMessage().GetMessage()
			continue
		}
		if msg.GetGroupMentionedMessage() != nil && msg.GetGroupMentionedMessage().GetMessage() != nil {
			msg = msg.GetGroupMentionedMessage().GetMessage()
			continue
		}
		if msg.GetDeviceSentMessage() != nil && msg.GetDeviceSentMessage().GetMessage() != nil {
			msg = msg.GetDeviceSentMessage().GetMessage()
			continue
		}
		if msg.GetBotInvokeMessage() != nil && msg.GetBotInvokeMessage().GetMessage() != nil {
			msg = msg.GetBotInvokeMessage().GetMessage()
			continue
		}
		if msg.GetLottieStickerMessage() != nil && msg.GetLottieStickerMessage().GetMessage() != nil {
			msg = msg.GetLottieStickerMessage().GetMessage()
			continue
		}
		if msg.GetEditedMessage() != nil && msg.GetEditedMessage().GetMessage() != nil {
			inner := msg.GetEditedMessage().GetMessage()
			if inner.GetProtocolMessage() != nil && inner.GetProtocolMessage().GetEditedMessage() != nil {
				msg = inner.GetProtocolMessage().GetEditedMessage()
				continue
			}
			msg = inner
			continue
		}
		if msg.GetProtocolMessage() != nil && msg.GetProtocolMessage().GetEditedMessage() != nil {
			msg = msg.GetProtocolMessage().GetEditedMessage()
			continue
		}
		break
	}
	return msg
}

func extractText(msg *waE2E.Message) string {
	msg = unwrapMessage(msg)
	if msg == nil {
		return ""
	}
	if msg.GetExtendedTextMessage() != nil {
		return msg.GetExtendedTextMessage().GetText()
	}
	if msg.GetConversation() != "" {
		return msg.GetConversation()
	}
	if msg.GetImageMessage() != nil {
		caption := msg.GetImageMessage().GetCaption()
		if caption != "" {
			return "[Image] " + caption
		}
		return "[Image]"
	}
	if msg.GetVideoMessage() != nil {
		caption := msg.GetVideoMessage().GetCaption()
		if caption != "" {
			return "[Video] " + caption
		}
		return "[Video]"
	}
	if msg.GetDocumentMessage() != nil {
		caption := msg.GetDocumentMessage().GetCaption()
		title := msg.GetDocumentMessage().GetTitle()
		if title == "" {
			title = msg.GetDocumentMessage().GetFileName()
		}
		if caption != "" {
			if title != "" {
				return "[Document] " + title + ": " + caption
			}
			return "[Document] " + caption
		}
		if title != "" {
			return "[Document] " + title
		}
		return "[Document]"
	}
	if msg.GetAudioMessage() != nil {
		audio := msg.GetAudioMessage()
		sec := audio.GetSeconds()
		durationStr := ""
		if sec > 0 {
			m := sec / 60
			s := sec % 60
			durationStr = fmt.Sprintf(" (%d:%02d)", m, s)
		}
		if audio.GetPTT() {
			return "[Voice Message]" + durationStr
		}
		return "[Audio]" + durationStr
	}
	if msg.GetLocationMessage() != nil {
		return "[Location]"
	}
	if msg.GetLiveLocationMessage() != nil {
		if caption := strings.TrimSpace(msg.GetLiveLocationMessage().GetCaption()); caption != "" {
			return "[Live Location] " + caption
		}
		return "[Live Location]"
	}
	if msg.GetContactMessage() != nil {
		name := msg.GetContactMessage().GetDisplayName()
		if name != "" {
			return "[Contact] " + name
		}
		return "[Contact]"
	}
	if msg.GetStickerMessage() != nil {
		return "[Sticker]"
	}
	if msg.GetReactionMessage() != nil {
		return "[Reaction]"
	}
	if msg.GetPollCreationMessage() != nil {
		return "[Poll] " + msg.GetPollCreationMessage().GetName()
	}
	if msg.GetProtocolMessage() != nil {
		if msg.GetProtocolMessage().GetType() == waE2E.ProtocolMessage_REVOKE {
			return "[Message deleted]"
		}
		return "[Protocol Message]"
	}
	if media, body, ok := templateParts(msg); ok {
		switch {
		case media.GetImageMessage() != nil:
			return strings.TrimSpace("[Image] " + body)
		case media.GetVideoMessage() != nil:
			return strings.TrimSpace("[Video] " + body)
		case media.GetDocumentMessage() != nil:
			return strings.TrimSpace("[Document] " + body)
		}
		if body != "" {
			return body
		}
	}
	if reply := msg.GetTemplateButtonReplyMessage(); reply != nil {
		if text := reply.GetSelectedDisplayText(); text != "" {
			return text
		}
	}
	if reply := msg.GetButtonsResponseMessage(); reply != nil {
		if text := reply.GetSelectedDisplayText(); text != "" {
			return text
		}
	}
	if reply := msg.GetListResponseMessage(); reply != nil {
		if text := reply.GetTitle(); text != "" {
			return text
		}
	}
	return "[Unsupported Message]"
}

// templateParts flattens a business template / interactive / buttons / list
// message into the two things the rest of the app knows how to handle: the
// header media (if any) and the visible text. Businesses send order updates,
// OTPs and marketing broadcasts as these, and without this they all decode to
// "[Unsupported Message]" with the header image unreachable, because the
// media sits nested inside the template rather than at the top level.
//
// The returned media message deliberately shares its ImageMessage/etc pointer
// with msg rather than copying, so callers that patch DirectPath after a media
// retry still write through to the original when they re-marshal msg.
func templateParts(msg *waE2E.Message) (media *waE2E.Message, text string, ok bool) {
	var blocks, buttons []string
	add := func(dst *[]string, s string) {
		if s = strings.TrimSpace(s); s != "" {
			*dst = append(*dst, s)
		}
	}
	switch {
	case msg.GetTemplateMessage() != nil:
		tpl := msg.GetTemplateMessage()
		hydrated := tpl.GetHydratedTemplate()
		if hydrated == nil {
			hydrated = tpl.GetHydratedFourRowTemplate()
		}
		if hydrated == nil {
			// interactiveMessageTemplate is just an InteractiveMessage in a
			// template wrapper, so hand it to the interactive branch below.
			if inner := tpl.GetInteractiveMessageTemplate(); inner != nil {
				return templateParts(&waE2E.Message{InteractiveMessage: inner})
			}
			return nil, "", false
		}
		media = headerMedia(hydrated.GetImageMessage(), hydrated.GetVideoMessage(), hydrated.GetDocumentMessage())
		add(&blocks, hydrated.GetHydratedTitleText())
		add(&blocks, hydrated.GetHydratedContentText())
		add(&blocks, hydrated.GetHydratedFooterText())
		for _, button := range hydrated.GetHydratedButtons() {
			// Quick-reply labels are just tap targets we can't offer, but URL
			// and call buttons carry a link/number that appears nowhere else
			// in the message, so they'd be lost if we dropped them.
			if url := button.GetUrlButton(); url != nil {
				add(&buttons, labeledValue(url.GetDisplayText(), url.GetURL()))
			} else if call := button.GetCallButton(); call != nil {
				add(&buttons, labeledValue(call.GetDisplayText(), call.GetPhoneNumber()))
			}
		}
		ok = true
	case msg.GetInteractiveMessage() != nil:
		interactive := msg.GetInteractiveMessage()
		header := interactive.GetHeader()
		media = headerMedia(header.GetImageMessage(), header.GetVideoMessage(), header.GetDocumentMessage())
		add(&blocks, header.GetTitle())
		add(&blocks, header.GetSubtitle())
		add(&blocks, interactive.GetBody().GetText())
		add(&blocks, interactive.GetFooter().GetText())
		ok = true
	case msg.GetButtonsMessage() != nil:
		buttonsMsg := msg.GetButtonsMessage()
		media = headerMedia(buttonsMsg.GetImageMessage(), buttonsMsg.GetVideoMessage(), buttonsMsg.GetDocumentMessage())
		add(&blocks, buttonsMsg.GetText())
		add(&blocks, buttonsMsg.GetContentText())
		add(&blocks, buttonsMsg.GetFooterText())
		ok = true
	case msg.GetListMessage() != nil:
		list := msg.GetListMessage()
		add(&blocks, list.GetTitle())
		add(&blocks, list.GetDescription())
		add(&blocks, list.GetFooterText())
		ok = true
	default:
		return nil, "", false
	}
	text = strings.Join(blocks, "\n\n")
	if len(buttons) > 0 {
		if text != "" {
			text += "\n\n"
		}
		text += strings.Join(buttons, "\n")
	}
	return media, text, ok
}

// labeledValue renders a template button as "Label: value", degrading to just
// the value when the button has no display text.
func labeledValue(label, value string) string {
	label, value = strings.TrimSpace(label), strings.TrimSpace(value)
	if label == "" || value == "" {
		return label + value
	}
	return label + ": " + value
}

// headerMedia wraps whichever of a template header's media slots is populated
// back into a plain Message, so the normal media code paths can handle it.
func headerMedia(image *waE2E.ImageMessage, video *waE2E.VideoMessage, document *waE2E.DocumentMessage) *waE2E.Message {
	switch {
	case image != nil:
		return &waE2E.Message{ImageMessage: image}
	case video != nil:
		return &waE2E.Message{VideoMessage: video}
	case document != nil:
		return &waE2E.Message{DocumentMessage: document}
	}
	return nil
}

// WhatsApp announces an album by sending a tiny albumMessage header ("3 videos
// are coming") immediately before the media itself. The header carries no
// content of its own, so storing it would show the user a stray empty bubble
// next to their photos - instead we keep it only long enough to stamp the
// media that follows with a shared album ID.
type pendingAlbum struct {
	id        string
	remaining int
	expires   time.Time
}

var (
	pendingAlbumsMu sync.Mutex
	pendingAlbums   = map[string]*pendingAlbum{}
)

// albumWindow bounds how long an album header waits for its media. The media
// normally follows within a couple of seconds; the window only has to outlast
// a slow upload, and its real job is to stop an album whose media never
// arrived from swallowing unrelated photos sent later.
const albumWindow = 2 * time.Minute

func albumKey(chatJID, senderJID string) string {
	return chatJID + "|" + senderJID
}

// registerAlbum records an incoming album header so the media that follows can
// claim it. Returns false if the header announced no media at all, in which
// case there is nothing to group.
func registerAlbum(chatJID, senderJID, headerID string, album *waE2E.AlbumMessage) bool {
	expected := int(album.GetExpectedImageCount() + album.GetExpectedVideoCount())
	if expected <= 0 || headerID == "" {
		return false
	}
	pendingAlbumsMu.Lock()
	defer pendingAlbumsMu.Unlock()
	pendingAlbums[albumKey(chatJID, senderJID)] = &pendingAlbum{
		id:        headerID,
		remaining: expected,
		expires:   time.Now().Add(albumWindow),
	}
	return true
}

// claimAlbumID hands the next piece of media in a chat the ID of the album
// header that announced it, or "" if no album is open. Must be called in
// arrival order - i.e. synchronously from the event handler, not from the
// goroutine that does the storing.
func claimAlbumID(chatJID, senderJID string) string {
	pendingAlbumsMu.Lock()
	defer pendingAlbumsMu.Unlock()
	key := albumKey(chatJID, senderJID)
	album, ok := pendingAlbums[key]
	if !ok {
		return ""
	}
	if time.Now().After(album.expires) {
		delete(pendingAlbums, key)
		return ""
	}
	album.remaining--
	if album.remaining <= 0 {
		delete(pendingAlbums, key)
	}
	return album.id
}

// stampAlbumMembers assigns an album ID to the media already sitting in the
// database that a given album header announced. Used for history-synced and
// previously-stored albums, where the header and its media arrive together (or
// arrived long ago) and the in-memory handoff above doesn't apply.
func stampAlbumMembers(headerID, chatJID, senderJID string, headerTS int64, expected int) {
	if headerID == "" || expected <= 0 {
		return
	}
	db.Exec(`UPDATE messages SET album_id = ? WHERE id IN (
			SELECT id FROM messages
			WHERE jid = ? AND sender_jid = ? AND media_type IN ('image', 'video')
				AND timestamp >= ? AND timestamp <= ?
				AND (album_id IS NULL OR album_id = '')
			ORDER BY timestamp ASC, id ASC LIMIT ?)`,
		headerID, chatJID, senderJID, headerTS-1, headerTS+int64(albumWindow/time.Second), expected)
}

// mediaCarrier returns the message that actually holds msg's downloadable
// media: msg itself for ordinary media messages, or the lifted-out header for
// template-style ones. Use this anywhere a raw stored message is about to be
// inspected for media.
func mediaCarrier(msg *waE2E.Message) *waE2E.Message {
	msg = unwrapMessage(msg)
	if msg == nil {
		return nil
	}
	if media, _, _ := templateParts(msg); media != nil {
		return media
	}
	return msg
}

// getContextInfo pulls the ContextInfo (which carries reply/quote data) out
// of whichever message type actually has it populated.
func getContextInfo(msg *waE2E.Message) *waE2E.ContextInfo {
	msg = unwrapMessage(msg)
	if msg == nil {
		return nil
	}
	switch {
	case msg.GetExtendedTextMessage() != nil:
		return msg.GetExtendedTextMessage().GetContextInfo()
	case msg.GetImageMessage() != nil:
		return msg.GetImageMessage().GetContextInfo()
	case msg.GetVideoMessage() != nil:
		return msg.GetVideoMessage().GetContextInfo()
	case msg.GetDocumentMessage() != nil:
		return msg.GetDocumentMessage().GetContextInfo()
	case msg.GetAudioMessage() != nil:
		return msg.GetAudioMessage().GetContextInfo()
	case msg.GetStickerMessage() != nil:
		return msg.GetStickerMessage().GetContextInfo()
	case msg.GetContactMessage() != nil:
		return msg.GetContactMessage().GetContextInfo()
	case msg.GetContactsArrayMessage() != nil:
		return msg.GetContactsArrayMessage().GetContextInfo()
	case msg.GetLocationMessage() != nil:
		return msg.GetLocationMessage().GetContextInfo()
	case msg.GetLiveLocationMessage() != nil:
		return msg.GetLiveLocationMessage().GetContextInfo()
	case msg.GetGroupInviteMessage() != nil:
		return msg.GetGroupInviteMessage().GetContextInfo()
	case msg.GetPollCreationMessage() != nil:
		return msg.GetPollCreationMessage().GetContextInfo()
	case msg.GetListMessage() != nil:
		return msg.GetListMessage().GetContextInfo()
	case msg.GetButtonsMessage() != nil:
		return msg.GetButtonsMessage().GetContextInfo()
	case msg.GetTemplateMessage() != nil:
		return msg.GetTemplateMessage().GetContextInfo()
	case msg.GetProductMessage() != nil:
		return msg.GetProductMessage().GetContextInfo()
	case msg.GetOrderMessage() != nil:
		return msg.GetOrderMessage().GetContextInfo()
	case msg.GetInteractiveMessage() != nil:
		return msg.GetInteractiveMessage().GetContextInfo()
	}
	return nil
}

// quotedPreview produces the small denormalized snapshot we store alongside
// a message that replies to another one, so the frontend can render
// "replying to: ..." without a second lookup.
func quotedPreview(msg *waE2E.Message) (quotedID, quotedText, quotedSender string) {
	if ctx := getContextInfo(msg); ctx != nil && ctx.GetStanzaID() != "" {
		quotedID = ctx.GetStanzaID()
		if participant := ctx.GetParticipant(); participant != "" {
			if parsed, err := types.ParseJID(participant); err == nil {
				quotedSender = resolveDisplayName(parsed)
			} else {
				quotedSender = participant
			}
		}
		if qm := ctx.GetQuotedMessage(); qm != nil {
			quotedText = extractText(qm)
		}
	}
	return
}

// mediaInfoFromMessage extracts a normalized (mediaType, mimetype, filename)
// triple for the media-bearing message kinds, so we can tag rows in the
// messages table for the frontend without it having to inspect raw protobuf.
func mediaInfoFromMessage(msg *waE2E.Message) (mediaType, mimetype, filename string) {
	msg = mediaCarrier(msg)
	if msg == nil {
		return "", "", ""
	}
	switch {
	case msg.GetImageMessage() != nil:
		return "image", msg.GetImageMessage().GetMimetype(), ""
	case msg.GetVideoMessage() != nil:
		return "video", msg.GetVideoMessage().GetMimetype(), ""
	case msg.GetAudioMessage() != nil:
		if msg.GetAudioMessage().GetPTT() {
			return "voice", msg.GetAudioMessage().GetMimetype(), ""
		}
		return "audio", msg.GetAudioMessage().GetMimetype(), ""
	case msg.GetDocumentMessage() != nil:
		name := msg.GetDocumentMessage().GetFileName()
		if name == "" {
			name = msg.GetDocumentMessage().GetTitle()
		}
		return "document", msg.GetDocumentMessage().GetMimetype(), sanitizeFilename(name)
	case msg.GetStickerMessage() != nil:
		return "sticker", msg.GetStickerMessage().GetMimetype(), ""
	}
	return "", "", ""
}

func isIgnoredChatJID(jid string) bool {
	return jid == "status@broadcast" || strings.HasSuffix(jid, "@broadcast") || strings.HasSuffix(jid, "@newsletter")
}

var historySyncMutex sync.Mutex

func eventHandler(evt interface{}) {
	switch evt.(type) {
	case *events.Message, *events.Receipt, *events.HistorySync:
		log.Printf("Event received in eventHandler: %T", evt)
	}

	switch v := evt.(type) {
	case *events.MediaRetry:
		go handleMediaRetryEvent(v)
	case *events.HistorySync:
		go func() {
			historySyncMutex.Lock()
			defer historySyncMutex.Unlock()

			if v.Data == nil {
				return
			}

			type syncedAlbum struct {
				id, jid, senderJID string
				timestamp          int64
				expected           int
			}
			var albumHeaders []syncedAlbum

			for _, mapping := range v.Data.GetPhoneNumberToLidMappings() {
				if mapping == nil {
					continue
				}
				pnJID, err1 := types.ParseJID(mapping.GetPnJID())
				lidJID, err2 := types.ParseJID(mapping.GetLidJID())
				if err1 == nil && err2 == nil && !pnJID.IsEmpty() && !lidJID.IsEmpty() {
					client.Store.LIDs.PutLIDMapping(context.Background(), lidJID, pnJID)
					db.Exec(`UPDATE OR IGNORE chats SET jid = ? WHERE jid = ?`, pnJID.String(), lidJID.String())
					db.Exec(`UPDATE OR IGNORE messages SET jid = ? WHERE jid = ?`, pnJID.String(), lidJID.String())
					db.Exec(`UPDATE OR IGNORE messages SET sender_jid = ? WHERE sender_jid = ?`, pnJID.String(), lidJID.String())
				}
			}

			for _, p := range v.Data.GetPushnames() {
				if p == nil {
					continue
				}
				idStr := p.GetID()
				pushname := p.GetPushname()
				if idStr == "" || pushname == "" {
					continue
				}
				parsedJID, err := types.ParseJID(idStr)
				if err != nil {
					continue
				}
				targetJID := parsedJID.ToNonAD()
				if targetJID.Server == "lid" {
					if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), targetJID); err == nil && !pn.IsEmpty() {
						targetJID = pn.ToNonAD()
					}
				}
				client.Store.Contacts.PutPushName(context.Background(), targetJID, pushname)
				upsertContactCacheRow(targetJID.String(), pushname, targetJID.User, false)
				db.Exec(`UPDATE chats SET name = ? WHERE jid = ? AND (name IS NULL OR name = '' OR name LIKE '+%')`, pushname, targetJID.String())
			}

			for _, conv := range v.Data.GetConversations() {
				tx, txErr := db.Begin()
				if txErr != nil {
					fmt.Println("Error starting transaction for HistorySync:", txErr)
					continue
				}

				jid := conv.GetID()
				if isIgnoredChatJID(jid) {
					tx.Rollback()
					continue
				}
				chatJID, parseErr := types.ParseJID(jid)
				if parseErr != nil {
					tx.Rollback()
					continue
				}
				chatJID = chatJID.ToNonAD()
				if chatJID.Server == "lid" {
					if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), chatJID); err == nil && !pn.IsEmpty() {
						chatJID = pn.ToNonAD()
					}
				}
				jid = chatJID.String()

				name := conv.GetName()
				if name == "" {
					if contact, err := client.Store.Contacts.GetContact(context.Background(), chatJID); err == nil && contact.Found {
						if contact.FullName != "" {
							name = contact.FullName
						} else if contact.PushName != "" {
							name = contact.PushName
						}
					}
				}

				ts := conv.GetConversationTimestamp()
				if ts == 0 {
					ts = conv.GetLastMsgTimestamp()
				}
				if ts == 0 && len(conv.GetMessages()) > 0 {
					for _, historyMsg := range conv.GetMessages() {
						msg := historyMsg.GetMessage()
						if msg != nil && msg.MessageTimestamp != nil {
							mTS := uint64(msg.GetMessageTimestamp())
							if mTS > ts {
								ts = mTS
							}
						}
					}
				}
				unread := conv.GetUnreadCount()
				pinned := 0
				if conv.GetPinned() > 0 {
					pinned = 1
				}
				muted := 0
				if conv.GetMuteEndTime() > 0 {
					muted = 1
				}
				archived := 0
				if conv.GetArchived() {
					archived = 1
				}
				_, err := tx.Exec(`INSERT INTO chats (jid, name, timestamp, unread_count, pinned, muted, archived) VALUES (?, ?, ?, ?, ?, ?, ?)
					ON CONFLICT(jid) DO UPDATE SET
					timestamp=MAX(timestamp, excluded.timestamp),
					name=CASE WHEN chats.name IS NULL OR chats.name = '' OR chats.name LIKE '+%' THEN COALESCE(NULLIF(excluded.name, ''), chats.name) ELSE chats.name END,
					unread_count=MAX(chats.unread_count, excluded.unread_count),
					pinned=MAX(chats.pinned, excluded.pinned),
					muted=MAX(chats.muted, excluded.muted),
					archived=MAX(chats.archived, excluded.archived)`,
					jid, name, ts, unread, pinned, muted, archived)
				if err != nil {
					fmt.Println("Error inserting chat from HistorySync:", err)
				}

				for i, historyMsg := range conv.GetMessages() {
					if i > 0 && i%200 == 0 {
						tx.Commit()
						time.Sleep(5 * time.Millisecond)
						tx, _ = db.Begin()
					}
					webMsg := historyMsg.GetMessage()
					if webMsg == nil {
						continue
					}
					if webMsg.GetMessageStubType() == waWeb.WebMessageInfo_REVOKE {
						stubID := webMsg.GetKey().GetID()
						if stubID != "" {
							stubFromMe := webMsg.GetKey().GetFromMe()
							stubTS := int64(webMsg.GetMessageTimestamp())
							if _, err := tx.Exec(`INSERT INTO messages (id, jid, from_me, text, timestamp, is_deleted)
									VALUES (?, ?, ?, '', ?, 1)
									ON CONFLICT(id) DO UPDATE SET is_deleted=1, text=''`,
								stubID, jid, stubFromMe, stubTS); err != nil {
								log.Println("Error storing revoked stub from HistorySync:", err)
							}
						}
						continue
					}
					evtMsg, parseEvtErr := client.ParseWebMessage(chatJID, webMsg)
					if parseEvtErr != nil || evtMsg == nil {
						continue
					}
					if pm := evtMsg.Message.GetProtocolMessage(); pm != nil && pm.GetType() == waE2E.ProtocolMessage_REVOKE {
						if targetID := pm.GetKey().GetID(); targetID != "" {
							if _, err := tx.Exec(`INSERT INTO messages (id, jid, from_me, text, timestamp, is_deleted)
									VALUES (?, ?, ?, '', ?, 1)
									ON CONFLICT(id) DO UPDATE SET is_deleted=1, text=''`,
								targetID, jid, evtMsg.Info.IsFromMe, evtMsg.Info.Timestamp.Unix()); err != nil {
								log.Println("Error storing revoke from HistorySync:", err)
							}
						}
						continue
					}
					if reaction := evtMsg.Message.GetReactionMessage(); reaction != nil {
						storeReactionTx(tx, jid, evtMsg, reaction)
						continue
					}
					if evtMsg.Message.GetEncReactionMessage() != nil {
						// Same announcement-group encrypted-reaction case as the
						// live-event path below - decrypt it instead of letting
						// it fall through to extractText's "[Unsupported
						// Message]" and get inserted as a bogus chat message.
						if reaction, err := client.DecryptReaction(context.Background(), evtMsg); err == nil {
							storeReactionTx(tx, jid, evtMsg, reaction)
						} else {
							log.Printf("Failed to decrypt announcement-group reaction from HistorySync: %v", err)
						}
						continue
					}

					if album := unwrapMessage(evtMsg.Message).GetAlbumMessage(); album != nil {
						// Same header-only stanza as the live path, but here
						// the media it announces is being written in this very
						// loop, so the grouping is applied by SQL once the
						// batch has landed rather than as each one arrives.
						headerID := evtMsg.Info.ID
						if headerID == "" && webMsg.Key != nil {
							headerID = webMsg.GetKey().GetID()
						}
						albumHeaders = append(albumHeaders, syncedAlbum{
							id:        headerID,
							jid:       jid,
							senderJID: evtMsg.Info.Sender.ToNonAD().String(),
							timestamp: evtMsg.Info.Timestamp.Unix(),
							expected:  int(album.GetExpectedImageCount() + album.GetExpectedVideoCount()),
						})
						continue
					}

					msgText := extractText(evtMsg.Message)
					if msgText == "[Protocol Message]" {
						continue
					}
					msgID := evtMsg.Info.ID
					if msgID == "" && webMsg.Key != nil {
						msgID = webMsg.GetKey().GetID()
					}
					if msgID != "" {
						msgJID := evtMsg.Info.Chat.ToNonAD().String()
						if msgJID == "" || strings.HasSuffix(msgJID, "@lid") {
							msgJID = jid
						}
						msgFromMe := evtMsg.Info.IsFromMe
						msgTS := evtMsg.Info.Timestamp.Unix()
						if msgTS <= 0 && webMsg.MessageTimestamp != nil {
							msgTS = int64(webMsg.GetMessageTimestamp())
						}
						senderJID := evtMsg.Info.Sender.ToNonAD()
						if senderJID.Server == "lid" {
							if !evtMsg.Info.SenderAlt.IsEmpty() {
								senderJID = evtMsg.Info.SenderAlt.ToNonAD()
							} else if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), senderJID); err == nil && !pn.IsEmpty() {
								senderJID = pn.ToNonAD()
							}
						}
						rawBytes, _ := proto.Marshal(evtMsg.Message)
						mediaType, mimetype, filename := mediaInfoFromMessage(evtMsg.Message)
						quotedID, quotedText, quotedSender := quotedPreview(evtMsg.Message)
						status := ""
						var sentAt, deliveredAt, readAt, playedAt int64
						if msgFromMe {
							sentAt = msgTS
							for _, ur := range webMsg.GetUserReceipt() {
								if ts := ur.GetReceiptTimestamp(); ts > deliveredAt {
									deliveredAt = ts
								}
								if ts := ur.GetReadTimestamp(); ts > readAt {
									readAt = ts
								}
								if ts := ur.GetPlayedTimestamp(); ts > playedAt {
									playedAt = ts
								}
							}
							stStr := strings.ToLower(webMsg.GetStatus().String())
							switch {
							case strings.Contains(stStr, "played"):
								status = "played"
							case strings.Contains(stStr, "read"):
								status = "read"
							case strings.Contains(stStr, "delivery") || strings.Contains(stStr, "delivered"):
								status = "delivered"
							case strings.Contains(stStr, "server") || strings.Contains(stStr, "sent"):
								status = "sent"
							default:
								if webMsg.GetStatus() > 0 {
									status = "sent"
								}
							}
						}
						_, err := tx.Exec(`INSERT INTO messages
								(id, jid, from_me, text, timestamp, sender_jid, raw, status, media_type, media_mimetype, media_filename, quoted_id, quoted_text, quoted_sender, sent_at, delivered_at, read_at, played_at)
							VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
							ON CONFLICT(id) DO UPDATE SET text=excluded.text, timestamp=excluded.timestamp, sender_jid=excluded.sender_jid, raw=excluded.raw,
								status=CASE WHEN excluded.status != '' THEN excluded.status ELSE messages.status END,
								media_type=excluded.media_type, media_mimetype=excluded.media_mimetype, media_filename=excluded.media_filename,
								quoted_id=excluded.quoted_id, quoted_text=excluded.quoted_text, quoted_sender=excluded.quoted_sender,
								sent_at=CASE WHEN excluded.sent_at != 0 THEN excluded.sent_at ELSE messages.sent_at END,
								delivered_at=CASE WHEN messages.delivered_at != 0 THEN messages.delivered_at ELSE excluded.delivered_at END,
								read_at=CASE WHEN messages.read_at != 0 THEN messages.read_at ELSE excluded.read_at END,
								played_at=CASE WHEN messages.played_at != 0 THEN messages.played_at ELSE excluded.played_at END`,
							msgID, msgJID, msgFromMe, msgText, msgTS, senderJID.String(), rawBytes, status,
							mediaType, mimetype, filename, quotedID, quotedText, quotedSender, sentAt, deliveredAt, readAt, playedAt)
						if err != nil {
							log.Println("Error inserting message from HistorySync:", err)
						}
					}
				}
				if err := tx.Commit(); err != nil {
					log.Println("Error committing HistorySync transaction for conv:", err)
				}
			}
			for _, album := range albumHeaders {
				stampAlbumMembers(album.id, album.jid, album.senderJID, album.timestamp, album.expected)
			}
			UpdateAllChatSnippets()
			populateChatsFromStore()
			broadcastChats()
		}()

	case *events.Message:
		unwrappedMsg := unwrapMessage(v.Message)
		isEdit := v.IsEdit || v.Info.Edit == types.EditAttributeMessageEdit || (unwrappedMsg != nil && unwrappedMsg.GetProtocolMessage() != nil && unwrappedMsg.GetProtocolMessage().GetType() == waE2E.ProtocolMessage_MESSAGE_EDIT)
		if isEdit {
			var targetID string
			var newContent *waE2E.Message

			if enc := v.Message.GetSecretEncryptedMessage(); enc != nil && enc.GetSecretEncType() == waE2E.SecretEncryptedMessage_MESSAGE_EDIT {
				targetID = enc.GetTargetMessageKey().GetID()
				decrypted, err := client.DecryptSecretEncryptedMessage(context.Background(), v)
				if err != nil {
					log.Printf("Edit: failed to decrypt secret encrypted edit: %v", err)
				} else {
					if decrypted.GetProtocolMessage() != nil && decrypted.GetProtocolMessage().GetType() == waE2E.ProtocolMessage_MESSAGE_EDIT {
						newContent = decrypted.GetProtocolMessage().GetEditedMessage()
					} else {
						newContent = decrypted
					}
				}
			} else if v.Message.GetProtocolMessage() != nil {
				targetID = v.Message.GetProtocolMessage().GetKey().GetID()
				newContent = v.Message.GetProtocolMessage().GetEditedMessage()
			} else if unwrappedMsg != nil && unwrappedMsg.GetProtocolMessage() != nil {
				targetID = unwrappedMsg.GetProtocolMessage().GetKey().GetID()
				newContent = unwrappedMsg.GetProtocolMessage().GetEditedMessage()
			}

			if targetID == "" {
				targetID = v.Info.ID
			}
			newText := extractText(newContent)
			if newText == "" || newText == "[Unsupported Message]" || newText == "[Protocol Message]" {
				newText = extractText(v.Message)
			}
			if newText == "[Protocol Message]" || newText == "[Unsupported Message]" {
				newText = extractText(unwrappedMsg)
			}
			resolvedChat, _ := resolveLIDChat(v)
			jid := resolvedChat.String()
			editTime := v.Info.Timestamp.Unix()
			if targetID != "" {
				go func() {
					rawBytes, _ := proto.Marshal(newContent)
					markMessageEdited(targetID, jid, newText, rawBytes, editTime)
				}()
				broadcastEvent("message_edited", map[string]interface{}{
					"jid":        jid,
					"message_id": targetID,
					"text":       newText,
					"edited_at":  editTime,
				})
			}
			return
		}
		if unwrappedMsg != nil && unwrappedMsg.GetProtocolMessage() != nil && unwrappedMsg.GetProtocolMessage().GetType() == waE2E.ProtocolMessage_REVOKE {
			targetID := unwrappedMsg.GetProtocolMessage().GetKey().GetID()
			resolvedChat, _ := resolveLIDChat(v)
			jid := resolvedChat.String()
			go markMessageDeleted(targetID, jid)
			broadcastEvent("message_deleted", map[string]interface{}{
				"jid":        jid,
				"message_id": targetID,
				"by":         v.Info.Sender.String(),
			})
			return
		}
		if reaction := unwrappedMsg.GetReactionMessage(); reaction != nil {
			applyReaction(v, reaction)
			return
		}
		if unwrappedMsg.GetEncReactionMessage() != nil {
			// Reactions in community announcement groups are delivered as
			// EncReactionMessage (encrypted separately from the outer stanza,
			// since only admins can post but anyone can react). Without this,
			// GetReactionMessage() above is nil, extractText falls through to
			// "[Unsupported Message]", and the reaction gets inserted as a
			// bogus standalone chat message instead of being applied.
			reaction, err := client.DecryptReaction(context.Background(), v)
			if err != nil {
				log.Printf("Failed to decrypt announcement-group reaction from %s: %v", v.Info.Sender, err)
				return
			}
			applyReaction(v, reaction)
			return
		}
		if v.Info.Chat.User == "status" && v.Info.Chat.Server == types.BroadcastServer {
			handleIncomingStatus(v)
			return
		}

		if album := unwrappedMsg.GetAlbumMessage(); album != nil {
			// Header only, no content of its own - remember it so the media
			// that follows can be grouped, and store nothing.
			albumChat, albumSender := resolveLIDChat(v)
			if registerAlbum(albumChat.String(), albumSender.String(), v.Info.ID, album) {
				log.Printf("Album header %s from %s: expecting %d media", v.Info.ID, albumSender,
					album.GetExpectedImageCount()+album.GetExpectedVideoCount())
			}
			return
		}

		text := extractText(v.Message)
		if text == "[Protocol Message]" {
			return
		}
		log.Printf("Received events.Message. ID: %s, Sender: %s, Chat: %s, text length: %d", v.Info.ID, v.Info.Sender.String(), v.Info.Chat.String(), len(text))

		// Claimed here rather than inside the goroutine below so that media
		// gets stamped in arrival order, which is the order the album header
		// counted them in.
		albumID := ""
		if mt, _, _ := mediaInfoFromMessage(v.Message); mt == "image" || mt == "video" {
			albumChat, albumSender := resolveLIDChat(v)
			albumID = claimAlbumID(albumChat.String(), albumSender.String())
		}

		go func() {
			chatJID, senderJID := resolveLIDChat(v)

			jid := chatJID.String()

			if isIgnoredChatJID(jid) {
				return
			}
			ts := v.Info.Timestamp.Unix()
			increment := 1
			if v.Info.IsFromMe {
				increment = 0
			}
			if increment > 0 && v.Info.ID != "" {
				var alreadyExists int
				_ = db.QueryRow(`SELECT COUNT(*) FROM messages WHERE id = ?`, v.Info.ID).Scan(&alreadyExists)
				if alreadyExists > 0 {
					// Redelivered/duplicate event for a message we've already
					// counted (e.g. reconnect resync) - don't double-count it.
					increment = 0
				}
			}
			_, err := db.Exec(`INSERT INTO chats (jid, timestamp, unread_count, preview_snippet, preview_sender, preview_from_me) VALUES (?, ?, ?, ?, ?, ?)
				ON CONFLICT(jid) DO UPDATE SET
					timestamp=MAX(timestamp, excluded.timestamp),
					unread_count=chats.unread_count + excluded.unread_count,
					preview_snippet=COALESCE(NULLIF(excluded.preview_snippet, ''), chats.preview_snippet),
					preview_sender=excluded.preview_sender,
					preview_from_me=excluded.preview_from_me,
					preview_is_reaction=0,
					preview_reaction_emoji=''`,
				jid, ts, increment, text, senderJID.String(), v.Info.IsFromMe)
			if err != nil {
				fmt.Println("Error updating chat timestamp:", err)
			}
			if v.Info.ID != "" {
				mediaType, mimetype, filename := mediaInfoFromMessage(v.Message)
				quotedID, quotedText, quotedSender := quotedPreview(v.Message)
				status := ""
				if v.Info.IsFromMe {
					status = "sent"
				}
				rawBytes, _ := proto.Marshal(v.Message)
				err := saveMessage(StoredMessage{
					ID: v.Info.ID, JID: jid, SenderJID: senderJID.String(),
					FromMe: v.Info.IsFromMe, Text: text, Timestamp: ts, Raw: rawBytes, Status: status,
					MediaType: mediaType, MediaMimetype: mimetype, MediaFilename: filename,
					QuotedID: quotedID, QuotedText: quotedText, QuotedSender: quotedSender,
					AlbumID: albumID,
				})
				if err != nil {
					fmt.Println("Error inserting message from live event:", err)
				}
			}
			broadcastChats()
		}()
		if text != "" {
			chatJID, senderJID := resolveLIDChat(v)
			if isIgnoredChatJID(chatJID.String()) {
				return
			}
			liveMediaType, _, liveMediaFilename := mediaInfoFromMessage(v.Message)

			quotedID, quotedText, quotedSender := quotedPreview(v.Message)
			broadcastEvent("message", map[string]interface{}{
				"jid":           chatJID.String(),
				"from":          senderJID.String(),
				"id":            v.Info.ID,
				"text":          text,
				"from_me":       v.Info.IsFromMe,
				"timestamp":     v.Info.Timestamp.Unix(),
				"has_media":     liveMediaType != "",
				"mediaType":     liveMediaType,
				"mediaFilename": liveMediaFilename,
				"albumId":       albumID,
				"quotedId":      quotedID,
				"quotedText":    quotedText,
				"quotedSender":  quotedSender,
			})
		}
	case *events.PushName:
		go func() {
			jidStr := v.JID.ToNonAD().String()
			if v.NewPushName != "" {
				client.Store.Contacts.PutPushName(context.Background(), v.JID.ToNonAD(), v.NewPushName)
				upsertContactCacheRow(jidStr, v.NewPushName, v.JID.User, false)
				db.Exec(`UPDATE chats SET name = ? WHERE jid = ? AND (name IS NULL OR name = '' OR name LIKE '+%')`, v.NewPushName, jidStr)
				broadcastChats()
			}
		}()
	case *events.Contact:
		go func() {
			jidStr := v.JID.ToNonAD().String()
			fullName := ""
			if v.Action != nil {
				fullName = v.Action.GetFullName()
				if fullName == "" {
					fullName = v.Action.GetFirstName()
				}
			}
			if fullName != "" {
				client.Store.Contacts.PutContactName(context.Background(), v.JID.ToNonAD(), fullName, fullName)
				upsertContactCacheRow(jidStr, fullName, v.JID.User, false)
				db.Exec(`UPDATE chats SET name = ? WHERE jid = ?`, fullName, jidStr)
				broadcastChats()
			}
		}()
	case *events.BusinessName:
		go func() {
			jidStr := v.JID.ToNonAD().String()
			if v.NewBusinessName != "" {
				upsertContactCacheRow(jidStr, v.NewBusinessName, v.JID.User, true)
				db.Exec(`UPDATE chats SET name = ? WHERE jid = ? AND (name IS NULL OR name = '' OR name LIKE '+%')`, v.NewBusinessName, jidStr)
				broadcastChats()
			}
		}()
	case *events.OfflineSyncCompleted:
		log.Printf("OfflineSyncCompleted event received. Count: %d", v.Count)
		go func() {
			UpdateAllChatSnippets()
			populateChatsFromStore()
			broadcastChats()
		}()
	case *events.DeleteForMe:
		// Synced from another of our own linked devices (or replayed on
		// first link / history resync) - always a local-only hide, never a
		// tombstone. See deleteMessageForMe's doc comment.
		jid := v.ChatJID.String()
		go deleteMessageForMe(v.MessageID, jid)
		broadcastEvent("message_deleted", map[string]interface{}{
			"jid":          jid,
			"message_id":   v.MessageID,
			"by":           v.SenderJID.String(),
			"for_everyone": false,
		})
	case *events.Connected:
		appStateResyncMu.Lock()
		for k := range appStateResyncCount {
			delete(appStateResyncCount, k)
		}
		for k := range lastAppStateResync {
			delete(lastAppStateResync, k)
		}
		appStateResyncMu.Unlock()
		if client.Store.ID != nil {
			broadcastEvent("connected", map[string]string{"jid": client.Store.ID.String()})
			go client.SendPresence(context.Background(), types.PresenceAvailable)
			go retryPendingTextMessages()
			go func() {
				time.Sleep(3 * time.Second)
				populateChatsFromStore()
				broadcastChats()
			}()
		}
	case *events.PairSuccess:
		log.Printf("Successfully paired with %s", v.ID.String())
		broadcastEvent("pair_success", map[string]string{"jid": v.ID.String()})
		go client.SendPresence(context.Background(), types.PresenceAvailable)
		go func() {
			time.Sleep(5 * time.Second)
			populateChatsFromStore()
			broadcastChats()
		}()
	case *events.LoggedOut:
		broadcastEvent("disconnected", nil)
	case *events.Disconnected:
		broadcastEvent("socket_disconnected", nil)
	case *events.ChatPresence:
		isComposing := v.State == types.ChatPresenceComposing
		isRecording := isComposing && v.Media == types.ChatPresenceMediaAudio
		isTyping := isComposing && !isRecording

		chatJID := v.MessageSource.Chat.ToNonAD()
		senderJID := v.MessageSource.Sender.ToNonAD()

		if chatJID.Server == "lid" {
			if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), chatJID); err == nil && !pn.IsEmpty() {
				chatJID = pn.ToNonAD()
			}
		}
		if senderJID.Server == "lid" {
			if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), senderJID); err == nil && !pn.IsEmpty() {
				senderJID = pn.ToNonAD()
			}
		}

		broadcastEvent("typing", map[string]interface{}{
			"chat_jid":   chatJID.String(),
			"sender_jid": senderJID.String(),
			"state":      string(v.State),
			"media":      string(v.Media),
			"typing":     isTyping,
			"recording":  isRecording,
		})
	case *events.Presence:
		lastSeenUnix := int64(0)
		if !v.LastSeen.IsZero() {
			lastSeenUnix = v.LastSeen.Unix()
		}

		targetJID := v.From.ToNonAD()
		if targetJID.Server == "lid" {
			if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), targetJID); err == nil && !pn.IsEmpty() {
				targetJID = pn.ToNonAD()
			}
		}

		broadcastEvent("presence", map[string]interface{}{
			"jid":         targetJID.String(),
			"unavailable": v.Unavailable,
			"last_seen":   lastSeenUnix,
		})
	case *events.Receipt:
		chatJID := v.MessageSource.Chat.ToNonAD()
		senderJID := v.MessageSource.Sender.ToNonAD()

		if chatJID.Server == "lid" {
			if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), chatJID); err == nil && !pn.IsEmpty() {
				chatJID = pn.ToNonAD()
			}
		}
		if senderJID.Server == "lid" {
			if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), senderJID); err == nil && !pn.IsEmpty() {
				senderJID = pn.ToNonAD()
			}
		}

		broadcastEvent("receipt", map[string]interface{}{
			"chat_jid":    chatJID.String(),
			"sender_jid":  senderJID.String(),
			"message_ids": v.MessageIDs,
			"type":        string(v.Type),
			"timestamp":   v.Timestamp.Unix(),
		})
		log.Printf("Received Receipt event - Chat: %s, Sender: %s, Type: %s, MessageIDs: %v", chatJID.String(), senderJID.String(), string(v.Type), v.MessageIDs)
		status := "delivered"
		if v.Type == types.ReceiptTypeRead || v.Type == types.ReceiptTypeReadSelf {
			status = "read"
		} else if v.Type == types.ReceiptTypePlayed || v.Type == types.ReceiptTypePlayedSelf {
			status = "played"
		} else if string(v.Type) == "sender" {
			status = "sent"
		} else if string(v.Type) == "inactive" {
			status = "delivered"
		} else if string(v.Type) != "" {
			status = string(v.Type)
		}
		go func() {
			ids := make([]string, len(v.MessageIDs))
			for i, id := range v.MessageIDs {
				ids[i] = string(id)
			}
			receiptTS := v.Timestamp.Unix()
			updated := updateMessageStatusIfNewer(ids, status, receiptTS)
			resolvedChatJID := v.MessageSource.Chat.ToNonAD()
			if resolvedChatJID.Server == "lid" {
				if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), resolvedChatJID); err == nil && !pn.IsEmpty() {
					resolvedChatJID = pn.ToNonAD()
				}
			}
			if len(updated) > 0 {
				broadcastEvent("message_status", map[string]interface{}{
					"jid":         resolvedChatJID.String(),
					"message_ids": updated,
					"status":      status,
					"timestamp":   receiptTS,
				})
			}
			isReadSelf := v.Type == types.ReceiptTypeReadSelf || v.Type == types.ReceiptTypePlayedSelf || ((v.Type == types.ReceiptTypeRead || v.Type == types.ReceiptTypePlayed) && v.MessageSource.IsFromMe)
			if isReadSelf {
				db.Exec("UPDATE chats SET unread_count = 0 WHERE jid = ?", resolvedChatJID.String())
				for _, mid := range v.MessageIDs {
					db.Exec("UPDATE chats SET unread_count = 0 WHERE jid = (SELECT jid FROM messages WHERE id = ?)", string(mid))
				}
				broadcastChats()
			}
		}()
	case *events.AppStateSyncComplete:
		if len(v.Name) > 0 {
			go func() {
				populateChatsFromStore()
				broadcastChats()
			}()
		}
	case *events.AppStateSyncError:
		handleAppStateSyncError(v)
	case *events.Archive:
		go func() {
			if v.Action == nil {
				return
			}
			archivedInt := 0
			if v.Action.GetArchived() {
				archivedInt = 1
			}
			db.Exec("UPDATE chats SET archived = ? WHERE jid = ?", archivedInt, v.JID.String())
			broadcastChats()
		}()
	case *events.Mute:
		go func() {
			if v.Action == nil {
				return
			}
			mutedInt := 0
			if v.Action.GetMuted() {
				mutedInt = 1
			}
			db.Exec("UPDATE chats SET muted = ? WHERE jid = ?", mutedInt, v.JID.String())
			broadcastChats()
		}()
	case *events.Pin:
		go func() {
			if v.Action == nil {
				return
			}
			pinnedInt := 0
			if v.Action.GetPinned() {
				pinnedInt = 1
			}
			db.Exec("UPDATE chats SET pinned = ? WHERE jid = ?", pinnedInt, v.JID.String())
			broadcastChats()
		}()
	case *events.Picture:
		go func() {
			jidStr := v.JID.String()
			if v.Remove {
				updateContactAvatarCache(jidStr, "", "")
			}
			broadcastEvent("avatar_updated", map[string]interface{}{
				"jid":     jidStr,
				"removed": v.Remove,
			})
		}()
	case *events.UserAbout:
		go updateContactAboutCache(v.JID.String(), v.Status)
		broadcastEvent("about_updated", map[string]interface{}{
			"jid":    v.JID.String(),
			"status": v.Status,
		})
	case *events.GroupInfo:
		go func() {
			if v.Name != nil {
				db.Exec(`UPDATE chats SET name = ? WHERE jid = ?`, v.Name.Name, v.JID.String())
			}
		}()
		broadcastEvent("group_updated", map[string]interface{}{
			"jid": v.JID.String(),
		})
	case *events.JoinedGroup:
		go func() {
			populateChatsFromStore()
			broadcastChats()
		}()
		broadcastEvent("group_updated", map[string]interface{}{
			"jid": v.JID.String(),
		})
	case *events.Blocklist:
		go func() {
			for _, change := range v.Changes {
				setContactBlocked(change.JID.String(), change.Action == events.BlocklistChangeActionBlock)
			}
		}()
		broadcastEvent("blocklist_updated", nil)
	}
}

func populateChatsFromStore() {
	if client == nil || client.Store == nil || client.Store.Contacts == nil {
		return
	}

	allContacts, err := client.Store.Contacts.GetAllContacts(context.Background())
	if err != nil {
		fmt.Println("Error getting contacts from store:", err)
		return
	}

	for jid, contact := range allContacts {
		targetJID := jid.ToNonAD()
		if targetJID.Server == "lid" {
			if pn, pnErr := client.Store.LIDs.GetPNForLID(context.Background(), targetJID); pnErr == nil && !pn.IsEmpty() {
				targetJID = pn.ToNonAD()
			}
		}

		jidStr := targetJID.String()
		if isIgnoredChatJID(jidStr) {
			continue
		}

		name := contact.FullName
		if name == "" {
			name = contact.PushName
		}
		if name == "" {
			name = contact.BusinessName
		}

		if name == "" && targetJID.Server != "g.us" && targetJID.Server != "lid" {
			name = "+" + targetJID.User
		}

		phone := ""
		if targetJID.Server != "g.us" && targetJID.Server != "lid" {
			phone = targetJID.User
		}

		if name != "" || phone != "" {
			upsertContactCacheRow(jidStr, name, phone, contact.BusinessName != "")
		}

		if name != "" {
			db.Exec(`UPDATE chats SET name = COALESCE(NULLIF(?, ''), name) WHERE jid = ?`, name, jidStr)
		}
	}

	groups, err := client.GetJoinedGroups(context.Background())
	if err == nil {
		for _, g := range groups {
			jidStr := g.JID.ToNonAD().String()
			db.Exec(`INSERT INTO chats (jid, name, timestamp, is_community) VALUES (?, ?, 0, ?)
				ON CONFLICT(jid) DO UPDATE SET
				name=COALESCE(NULLIF(excluded.name, ''), chats.name),
				is_community=excluded.is_community`,
				jidStr, g.GroupName.Name, boolToInt(g.IsParent))
		}
	}

	rows, err := db.Query(`SELECT jid FROM chats WHERE name IS NULL OR name = '' OR name LIKE '+%' OR jid LIKE '%@lid'`)
	if err == nil {
		var jidsToResolve []string
		for rows.Next() {
			var j string
			if rows.Scan(&j) == nil {
				jidsToResolve = append(jidsToResolve, j)
			}
		}
		rows.Close()

		for _, jStr := range jidsToResolve {
			pj, err := types.ParseJID(jStr)
			if err != nil {
				continue
			}
			resName := resolveDisplayName(pj)
			if resName != "" && resName != jStr && !strings.HasPrefix(resName, "+") {
				db.Exec(`UPDATE chats SET name = ? WHERE jid = ?`, resName, jStr)
			}
		}
	}

	rowsBroad, errBroad := db.Query(`SELECT DISTINCT sender_jid FROM messages
		WHERE jid LIKE '%@broadcast' AND jid != 'status@broadcast' AND from_me = 0 AND sender_jid != ''`)
	if errBroad == nil {
		var senders []string
		for rowsBroad.Next() {
			var s string
			if rowsBroad.Scan(&s) == nil {
				senders = append(senders, s)
			}
		}
		rowsBroad.Close()
		for _, s := range senders {
			parsed, err := types.ParseJID(s)
			if err != nil {
				continue
			}
			target := parsed.ToNonAD()
			if target.Server == "lid" {
				pn, err := client.Store.LIDs.GetPNForLID(context.Background(), target)
				if err != nil || pn.IsEmpty() {
					continue
				}
				target = pn.ToNonAD()
			}
			if target.Server != types.DefaultUserServer {
				continue
			}
			db.Exec(`INSERT INTO chats (jid, timestamp)
				SELECT ?, MAX(timestamp) FROM messages
				WHERE jid LIKE '%@broadcast' AND from_me = 0 AND sender_jid = ?
				ON CONFLICT(jid) DO UPDATE SET timestamp = MAX(chats.timestamp, excluded.timestamp)`,
				target.String(), s)
			db.Exec(`UPDATE messages SET jid = ?, sender_jid = ?
				WHERE jid LIKE '%@broadcast' AND from_me = 0 AND sender_jid = ?`,
				target.String(), target.String(), s)
			UpdateChatSnippet(target.String())
		}
	}
	db.Exec("DELETE FROM chats WHERE timestamp = 0 AND jid NOT IN (SELECT DISTINCT jid FROM messages) AND jid NOT IN (SELECT DISTINCT jid FROM chats WHERE pinned = 1 OR muted = 1 OR archived = 1)")
	db.Exec("DELETE FROM chats WHERE jid = 'status@broadcast' OR jid LIKE '%@newsletter' OR jid LIKE '%@broadcast'")
}

// handleIncomingStatus stores a Status/Stories post (someone else's, or our
// own posted from another linked device) using the exact same messages-table
// row shape as a normal message, just under the reserved "status@broadcast"
// jid, which the rest of the app already treats as invisible to the regular
// chat list (see isIgnoredChatJID). Reusing that table means every existing
// media download/open/save code path (keyed purely by message ID) works for
// status media with zero changes.
func handleIncomingStatus(v *events.Message) {
	text := extractText(v.Message)
	if text == "[Protocol Message]" || text == "[Unsupported Message]" {
		text = ""
	}
	senderJID := v.Info.Sender.ToNonAD()
	mediaType, mimetype, filename := mediaInfoFromMessage(v.Message)
	if text == "" && mediaType == "" {
		return
	}
	ts := v.Info.Timestamp.Unix()
	status := ""
	if v.Info.IsFromMe {
		status = "sent"
	}
	rawBytes, _ := proto.Marshal(v.Message)
	if err := saveMessage(StoredMessage{
		ID: v.Info.ID, JID: "status@broadcast", SenderJID: senderJID.String(),
		FromMe: v.Info.IsFromMe, Text: text, Timestamp: ts, Raw: rawBytes, Status: status,
		MediaType: mediaType, MediaMimetype: mimetype, MediaFilename: filename,
	}); err != nil {
		log.Println("Error saving status update:", err)
		return
	}
	go func() {
		cutoff := time.Now().Add(-24 * time.Hour).Unix()
		db.Exec("DELETE FROM messages WHERE jid = 'status@broadcast' AND timestamp <= ?", cutoff)
	}()
	broadcastEvent("status_update", map[string]interface{}{
		"id": v.Info.ID, "posterJid": senderJID.String(), "fromMe": v.Info.IsFromMe,
		"text": text, "mediaType": mediaType, "timestamp": ts,
	})
}

var (
	appStateResyncMu     sync.Mutex
	lastAppStateResync   = make(map[appstate.WAPatchName]time.Time)
	appStateResyncCount  = make(map[appstate.WAPatchName]int)
)

func handleAppStateSyncError(v *events.AppStateSyncError) {
	appStateResyncMu.Lock()
	count := appStateResyncCount[v.Name]
	if count >= 3 {
		appStateResyncMu.Unlock()
		log.Printf("AppStateSyncError for %s: giving up after %d failed attempts. Will retry on next connection.", v.Name, count)
		return
	}
	lastTime := lastAppStateResync[v.Name]
	if time.Since(lastTime) < 60*time.Second {
		appStateResyncMu.Unlock()
		return
	}
	appStateResyncCount[v.Name] = count + 1
	lastAppStateResync[v.Name] = time.Now()
	appStateResyncMu.Unlock()

	go func() {
		log.Printf("AppStateSyncError for %s (attempt %d/3): %v.", v.Name, count+1, v.Error)
		if client == nil {
			return
		}
		if v.Error != nil {
			errMsg := v.Error.Error()
			if strings.Contains(errMsg, "mismatching LTHash") || strings.Contains(errMsg, "failed to verify snapshot") {
				re := regexp.MustCompile(`patch v(\d+)`)
				matches := re.FindStringSubmatch(errMsg)
				if len(matches) < 2 {
					re = regexp.MustCompile(`v(\d+)`)
					matches = re.FindStringSubmatch(errMsg)
				}
				if len(matches) > 1 {
					ver, err := strconv.ParseUint(matches[1], 10, 64)
					if err == nil && ver > 0 {
						var jid string
						if client.Store != nil && client.Store.ID != nil {
							jid = client.Store.ID.String()
						}
						log.Printf("Seeding version %d for %s (JID: %s)...", ver, v.Name, jid)
						db.Exec("INSERT INTO whatsmeow_app_state_version (jid, name, version, hash) VALUES (?, ?, ?, ?) ON CONFLICT(jid, name) DO UPDATE SET version=excluded.version", jid, string(v.Name), ver, make([]byte, 128))
						if resyncErr := client.FetchAppState(context.Background(), v.Name, false, false); resyncErr != nil {
							log.Printf("Post-seeding resync for %s failed: %v", v.Name, resyncErr)
						} else {
							log.Printf("Post-seeding resync for %s succeeded", v.Name)
							appStateResyncMu.Lock()
							appStateResyncCount[v.Name] = 0
							appStateResyncMu.Unlock()
						}
						return
					}
				}
			}
		}
		err := client.FetchAppState(context.Background(), v.Name, false, false)
		if err != nil {
			if errors.Is(err, whatsmeow.ErrIQRateOverLimit) || strings.Contains(err.Error(), "429") || strings.Contains(err.Error(), "rate-overlimit") {
				log.Printf("Rate limit 429 during resync of %s. Scheduling retry in 90s...", v.Name)
				time.AfterFunc(90*time.Second, func() {
					appStateResyncMu.Lock()
					lastAppStateResync[v.Name] = time.Now()
					appStateResyncMu.Unlock()
					if client != nil {
						if retryErr := client.FetchAppState(context.Background(), v.Name, false, false); retryErr != nil {
							log.Printf("Delayed retry for %s failed: %v", v.Name, retryErr)
						} else {
							log.Printf("Delayed resync for %s succeeded", v.Name)
							appStateResyncMu.Lock()
							appStateResyncCount[v.Name] = 0
							appStateResyncMu.Unlock()
						}
					}
				})
			} else {
				log.Printf("Failed to resync app state %s: %v", v.Name, err)
			}
		} else {
			log.Printf("App state %s resync succeeded", v.Name)
			appStateResyncMu.Lock()
			appStateResyncCount[v.Name] = 0
			appStateResyncMu.Unlock()
		}
	}()
}
