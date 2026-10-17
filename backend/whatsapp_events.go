package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"os"
	"path/filepath"
	"runtime/debug"
	"runtime/pprof"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/proto/waHistorySync"
	"go.mau.fi/whatsmeow/proto/waWeb"
	"go.mau.fi/whatsmeow/store"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"go.mau.fi/whatsmeow/types/richresponse"
	"google.golang.org/protobuf/proto"
	"google.golang.org/protobuf/reflect/protoreflect"
)

func resolveLIDChat(v *events.Message) (chatJID, senderJID types.JID) {
	chatJID = v.Info.Chat.ToNonAD()
	senderJID = v.Info.Sender.ToNonAD()

	if senderJID.Server == "lid" {
		if !v.Info.SenderAlt.IsEmpty() {
			senderJID = v.Info.SenderAlt.ToNonAD()
		} else if pn, err := wa().Store.LIDs.GetPNForLID(context.Background(), senderJID); err == nil && !pn.IsEmpty() {
			senderJID = pn.ToNonAD()
		} else if v.Info.IsFromMe && wa().Store.ID != nil {
			senderJID = wa().Store.ID.ToNonAD()
		}
	}

	if chatJID.Server == "lid" {
		if v.Info.IsFromMe {
			if !v.Info.RecipientAlt.IsEmpty() {
				chatJID = v.Info.RecipientAlt.ToNonAD()
			} else if pn, err := wa().Store.LIDs.GetPNForLID(context.Background(), chatJID); err == nil && !pn.IsEmpty() {
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

// resolveSourcePN is resolveLIDChat for events that only carry a message
// source (receipts, typing): the chat and sender as the phone JIDs the app
// keys by, using the addresses the stanza itself paired them with first.
func resolveSourcePN(src types.MessageSource) (chatJID, senderJID types.JID) {
	chatJID = src.Chat.ToNonAD()
	senderJID = src.Sender.ToNonAD()
	rawSender := senderJID
	if senderJID.Server == types.HiddenUserServer {
		if src.SenderAlt.Server == types.DefaultUserServer {
			senderJID = src.SenderAlt.ToNonAD()
		} else {
			senderJID = preferPN(senderJID)
		}
	}
	if chatJID.Server == types.HiddenUserServer {
		switch {
		case chatJID == rawSender:
			chatJID = senderJID
		case src.RecipientAlt.Server == types.DefaultUserServer:
			chatJID = src.RecipientAlt.ToNonAD()
		default:
			chatJID = preferPN(chatJID)
		}
	}
	return
}

// applyReaction stores an incoming reaction and updates the chat-list preview.
// It never creates a standalone chat message - if the reacted-to message isn't
// in the local DB, updateChatReactionPreview silently leaves the preview alone.
func applyReaction(v *events.Message, reaction *waE2E.ReactionMessage) {
	// A reaction to a status update comes to its poster alone.
	if reaction.GetKey().GetRemoteJID() == types.StatusBroadcastJID.String() {
		noteStatusReaction(v, reaction)
		return
	}
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

	// Written before returning, in arrival order: a reaction and its removal
	// moments later must not land the other way round.
	upsertReaction(targetID, jid, reactorJID, emoji, ts)
	updateChatReactionPreview(jid, targetID, reactorJID, emoji, v.Info.IsFromMe)
	broadcastChats()

	// Included so clients can show a WhatsApp-style "<name> reacted <emoji>
	// to: <text>" desktop notification without having the reacted-to
	// message loaded locally (e.g. reactions on chats other than the one
	// currently open).
	var reactedText string
	var isDeleted int
	if err := db.QueryRow(`SELECT COALESCE(text, ''), is_deleted FROM messages WHERE id = ?`, targetID).
		Scan(&reactedText, &isDeleted); err != nil || isDeleted == 1 {
		reactedText = ""
	}

	broadcastEvent("message_reaction", map[string]interface{}{
		"jid":          jid,
		"message_id":   targetID,
		"reactor":      reactorJID,
		"reactor_name": reactorName,
		"emoji":        emoji,
		"text":         reactedText,
	})
}

// storeReactionTx applies a HistorySync-backfilled reaction within the
// caller's transaction. Mirrors applyReaction but writes via tx and skips the
// live "message_reaction" broadcast, since this runs during bulk backfill.
func storeReactionTx(tx sqlExecer, jid string, evtMsg *events.Message, reaction *waE2E.ReactionMessage) {
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
		tx.Exec(`DELETE FROM reactions WHERE message_id = ? AND reactor_jid = ?`, targetID, reactorJID)
	} else {
		tx.Exec(`INSERT INTO reactions (message_id, chat_jid, reactor_jid, emoji, timestamp) VALUES (?, ?, ?, ?, ?)
			ON CONFLICT(message_id, reactor_jid) DO UPDATE SET emoji = excluded.emoji, timestamp = excluded.timestamp`,
			targetID, jid, reactorJID, emoji, ts)
	}
}

// isKeyDistributionOnly reports whether a decrypted payload carries nothing but
// a sender key distribution message. Group messages arrive as two encrypted
// blobs - a per-device one holding the sender key, and the skmsg holding the
// actual content - and whatsmeow dispatches an events.Message for each, both
// with the same message ID. The key-distribution half has no content to show,
// so storing it would either create a phantom "[Unsupported Message]" bubble or
// (when it lands last) overwrite the real message with one.
func isKeyDistributionOnly(msg *waE2E.Message) bool {
	if msg == nil || msg.GetSenderKeyDistributionMessage() == nil {
		return false
	}
	contentful := false
	msg.ProtoReflect().Range(func(fd protoreflect.FieldDescriptor, _ protoreflect.Value) bool {
		switch fd.Name() {
		case "senderKeyDistributionMessage", "messageContextInfo":
			return true
		}
		contentful = true
		return false
	})
	return !contentful
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
		if msg.GetPollCreationMessageV4() != nil && msg.GetPollCreationMessageV4().GetMessage() != nil {
			msg = msg.GetPollCreationMessageV4().GetMessage()
			continue
		}
		if msg.GetLottieStickerMessage() != nil && msg.GetLottieStickerMessage().GetMessage() != nil {
			msg = msg.GetLottieStickerMessage().GetMessage()
			continue
		}
		// An album item arrives wrapped as the album's associated child, and a
		// Meta AI answer forwarded into a chat as a bot-forwarded wrapper;
		// both carry an ordinary message inside.
		if msg.GetAssociatedChildMessage() != nil && msg.GetAssociatedChildMessage().GetMessage() != nil {
			msg = msg.GetAssociatedChildMessage().GetMessage()
			continue
		}
		if msg.GetBotForwardedMessage() != nil && msg.GetBotForwardedMessage().GetMessage() != nil {
			msg = msg.GetBotForwardedMessage().GetMessage()
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

// extractText returns the display text for a message, with any WhatsApp
// @mention tokens (which arrive as raw "@<jid-user>" digit strings) resolved
// to display names the same way the chat list resolves them.
func extractText(msg *waE2E.Message) string {
	return resolveMentions(extractTextRaw(msg), getContextInfo(msg))
}

// resolveMentions replaces each "@<jid-user>" token named in ctx's
// mentionedJID list with "@<resolved display name>". WhatsApp sends mentions
// as the bare numeric JID user part (a LID number for LID-addressed groups,
// which is what rendered as "@10000000000003" instead of "@Sam Rivers").
func resolveMentions(text string, ctx *waE2E.ContextInfo) string {
	if text == "" || ctx == nil {
		return text
	}
	for _, raw := range ctx.GetMentionedJID() {
		jid, err := types.ParseJID(raw)
		if err != nil || jid.User == "" {
			continue
		}
		text = replaceMentionToken(text, "@"+jid.User, "@"+resolveDisplayName(jid))
	}
	return text
}

// replaceMentionToken replaces occurrences of token in text, skipping any
// occurrence immediately followed by another digit (so "@123" inside
// "@12345" - a different, longer JID user - is left untouched).
func replaceMentionToken(text, token, replacement string) string {
	var b strings.Builder
	for {
		i := strings.Index(text, token)
		if i == -1 {
			b.WriteString(text)
			break
		}
		end := i + len(token)
		if end < len(text) && text[end] >= '0' && text[end] <= '9' {
			b.WriteString(text[:end])
			text = text[end:]
			continue
		}
		b.WriteString(text[:i])
		b.WriteString(replacement)
		text = text[end:]
	}
	return b.String()
}

func extractTextRaw(msg *waE2E.Message) string {
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
	// A round "video note" - WhatsApp's video counterpart to a voice message -
	// is a plain VideoMessage in a field of its own.
	if ptv := msg.GetPtvMessage(); ptv != nil {
		if sec := ptv.GetSeconds(); sec > 0 {
			return fmt.Sprintf("[Video Note] (%d:%02d)", sec/60, sec%60)
		}
		return "[Video Note]"
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
	if msg.GetContactsArrayMessage() != nil {
		arr := msg.GetContactsArrayMessage()
		names := make([]string, 0, len(arr.GetContacts()))
		for _, c := range arr.GetContacts() {
			if n := c.GetDisplayName(); n != "" {
				names = append(names, n)
			}
		}
		if len(names) > 0 {
			return "[Contact] " + strings.Join(names, ", ")
		}
		if name := arr.GetDisplayName(); name != "" {
			return "[Contact] " + name
		}
		return "[Contact]"
	}
	if sticker := msg.GetStickerMessage(); sticker != nil {
		return stickerText(sticker)
	}
	if msg.GetReactionMessage() != nil {
		return "[Reaction]"
	}
	if invite := msg.GetGroupInviteMessage(); invite != nil {
		if name := invite.GetGroupName(); name != "" {
			return "[Group Invite] " + name
		}
		return "[Group Invite]"
	}
	if invite := msg.GetNewsletterAdminInviteMessage(); invite != nil {
		text := "[Channel Admin Invite] " + invite.GetNewsletterName()
		if caption := invite.GetCaption(); caption != "" {
			text += ": " + caption
		}
		return text
	}
	if poll := pollCreation(msg); poll != nil {
		if poll.GetPollType() == waE2E.PollType_QUIZ {
			return "[Quiz] " + poll.GetName()
		}
		return "[Poll] " + poll.GetName()
	}
	if text := describeOtherMessage(msg); text != "" {
		return text
	}
	// An advanced chat privacy toggle: recorded on the chat, not shown.
	if msg.GetLimitSharingMessage() != nil {
		return "[Protocol Message]"
	}
	// Bookkeeping with nothing to show as a bubble of its own: pinning or
	// keeping a message, an RSVP to an event, a comment under a channel post,
	// sticker-sync and scheduled-call edits. Left unhandled they were stored as
	// "[Unsupported Message]" rows in the chat.
	if msg.GetPinInChatMessage() != nil || msg.GetKeepInChatMessage() != nil ||
		msg.GetEncEventResponseMessage() != nil || msg.GetEncCommentMessage() != nil ||
		msg.GetStickerSyncRmrMessage() != nil || msg.GetScheduledCallEditMessage() != nil ||
		msg.GetCallLogMesssage() != nil {
		return "[Protocol Message]"
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
	if text := richResponseText(msg.GetRichResponseMessage()); text != "" {
		return text
	}
	// WhatsApp withholds some messages from linked devices on purpose -
	// businesses' one-time login codes, for one - and sends them only a
	// placeholder. There is nothing to fetch: the phone never shares them.
	if msg.GetPlaceholderMessage() != nil {
		return "This message can only be seen on your phone. Open WhatsApp on your phone to view it."
	}
	return "[Unsupported Message]"
}

// describeOtherMessage renders the less common message kinds that carry
// something worth reading - events, scheduled calls, business products and
// orders, poll result snapshots, sticker packs, payments, status mentions and
// channel invites - or "" for anything else.
func describeOtherMessage(msg *waE2E.Message) string {
	withDetail := func(label, detail string) string {
		if detail = strings.TrimSpace(detail); detail != "" {
			return label + " " + detail
		}
		return label
	}
	switch {
	case msg.GetEventMessage() != nil:
		event := msg.GetEventMessage()
		label := "[Event]"
		if event.GetIsCanceled() {
			label = "[Event cancelled]"
		}
		parts := []string{strings.TrimSpace(event.GetName())}
		if start := event.GetStartTime(); start > 0 {
			parts = append(parts, time.Unix(start, 0).Format("Mon 2 Jan 2006, 15:04"))
		}
		if loc := strings.TrimSpace(event.GetLocation().GetName()); loc != "" {
			parts = append(parts, loc)
		}
		if desc := strings.TrimSpace(event.GetDescription()); desc != "" {
			parts = append(parts, desc)
		}
		if link := strings.TrimSpace(event.GetJoinLink()); link != "" {
			parts = append(parts, link)
		}
		var kept []string
		for _, p := range parts {
			if p != "" {
				kept = append(kept, p)
			}
		}
		return withDetail(label, strings.Join(kept, "\n"))
	case msg.GetScheduledCallCreationMessage() != nil:
		call := msg.GetScheduledCallCreationMessage()
		detail := call.GetTitle()
		if ms := call.GetScheduledTimestampMS(); ms > 0 {
			when := time.UnixMilli(ms).Format("Mon 2 Jan 2006, 15:04")
			detail = strings.TrimSpace(detail + "\n" + when)
		}
		return withDetail("[Scheduled Call]", detail)
	case msg.GetProductMessage() != nil:
		product := msg.GetProductMessage()
		detail := product.GetProduct().GetTitle()
		if body := product.GetBody(); body != "" {
			detail = strings.TrimSpace(detail + "\n" + body)
		}
		return withDetail("[Product]", detail)
	case msg.GetOrderMessage() != nil:
		order := msg.GetOrderMessage()
		detail := order.GetOrderTitle()
		if text := order.GetMessage(); text != "" {
			detail = strings.TrimSpace(detail + "\n" + text)
		}
		return withDetail("[Order]", detail)
	case msg.GetPollResultSnapshotMessage() != nil:
		return withDetail("[Poll results]", msg.GetPollResultSnapshotMessage().GetName())
	case msg.GetPollResultSnapshotMessageV3() != nil:
		return withDetail("[Poll results]", msg.GetPollResultSnapshotMessageV3().GetName())
	case msg.GetStickerPackMessage() != nil:
		return withDetail("[Sticker Pack]", msg.GetStickerPackMessage().GetName())
	case msg.GetRequestPhoneNumberMessage() != nil:
		return "[Asked for your phone number]"
	case msg.GetSendPaymentMessage() != nil, msg.GetRequestPaymentMessage() != nil,
		msg.GetPaymentInviteMessage() != nil, msg.GetDeclinePaymentRequestMessage() != nil,
		msg.GetCancelPaymentRequestMessage() != nil:
		return "[Payment] Open WhatsApp on your phone to see this payment."
	case msg.GetStatusMentionMessage() != nil, msg.GetGroupStatusMentionMessage() != nil:
		return "[Mentioned you in a status update]"
	case msg.GetNewsletterFollowerInviteMessageV2() != nil:
		invite := msg.GetNewsletterFollowerInviteMessageV2()
		detail := invite.GetNewsletterName()
		if caption := invite.GetCaption(); caption != "" {
			detail = strings.TrimSpace(detail + ": " + caption)
		}
		return withDetail("[Channel Invite]", detail)
	}
	return ""
}

// richResponseSection matches the real wire shape of one entry in a rich
// response's "sections" array - unlike richresponse.RichResponse.Sections,
// which expects "__typename" directly on the section and silently resolves
// to an empty UnknownViewModel here, since WhatsApp nests it one level
// deeper under "view_model".
type richResponseSection struct {
	ViewModel richresponse.ViewModelContainer `json:"view_model"`
}

// richResponseText renders a bot's AIRichResponseMessage (Meta AI and other
// WhatsApp bots' replies) to plain text. The actual content isn't in any of
// the normal Message oneof fields - it's JSON, describing a tree of
// "primitives" (markdown text, code blocks, tables, ...), packed into
// UnifiedResponse.Data.
func richResponseText(rich *waE2E.AIRichResponseMessage) string {
	data := rich.GetUnifiedResponse().GetData()
	if len(data) == 0 {
		return ""
	}
	var parsed struct {
		Sections []richResponseSection `json:"sections"`
	}
	if err := json.Unmarshal(data, &parsed); err != nil {
		log.Printf("Failed to parse rich response JSON: %v", err)
		return ""
	}
	parts := make([]string, 0, len(parsed.Sections))
	for _, section := range parsed.Sections {
		if section.ViewModel.Model == nil {
			continue
		}
		if s := strings.TrimSpace(section.ViewModel.Model.String()); s != "" {
			parts = append(parts, s)
		}
	}
	return strings.Join(parts, "\n")
}

// pollCreation returns whichever of the poll-creation fields is populated.
// WhatsApp has revised the poll format six times and each revision landed on a
// new field number rather than replacing the last, so a client that only reads
// the original one renders every poll sent by a current app as an unsupported
// message. (V4 is a FutureProofMessage wrapper and is peeled off in
// unwrapMessage, so it arrives here as one of the others.)
func pollCreation(msg *waE2E.Message) *waE2E.PollCreationMessage {
	for _, poll := range []*waE2E.PollCreationMessage{
		msg.GetPollCreationMessage(),
		msg.GetPollCreationMessageV2(),
		msg.GetPollCreationMessageV3(),
		msg.GetPollCreationMessageV5(),
		msg.GetPollCreationMessageV6(),
	} {
		if poll != nil {
			return poll
		}
	}
	return nil
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
	// A video note is downloaded, retried and played like any video. Shares
	// the pointer, like templateParts, so a media retry's new DirectPath
	// still lands in msg.
	if ptv := msg.GetPtvMessage(); ptv != nil {
		return &waE2E.Message{VideoMessage: ptv}
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
	case pollCreation(msg) != nil:
		return pollCreation(msg).GetContextInfo()
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
	case msg.GetPtvMessage() != nil:
		return msg.GetPtvMessage().GetContextInfo()
	case msg.GetEventMessage() != nil:
		return msg.GetEventMessage().GetContextInfo()
	case msg.GetStickerPackMessage() != nil:
		return msg.GetStickerPackMessage().GetContextInfo()
	case msg.GetPollResultSnapshotMessage() != nil:
		return msg.GetPollResultSnapshotMessage().GetContextInfo()
	}
	return nil
}

// setContextInfo replaces the ContextInfo on whichever message type msg
// carries. Returns false for a type that has no ContextInfo to set.
func setContextInfo(msg *waE2E.Message, ctx *waE2E.ContextInfo) bool {
	switch {
	case msg.GetExtendedTextMessage() != nil:
		msg.GetExtendedTextMessage().ContextInfo = ctx
	case msg.GetImageMessage() != nil:
		msg.GetImageMessage().ContextInfo = ctx
	case msg.GetVideoMessage() != nil:
		msg.GetVideoMessage().ContextInfo = ctx
	case msg.GetDocumentMessage() != nil:
		msg.GetDocumentMessage().ContextInfo = ctx
	case msg.GetAudioMessage() != nil:
		msg.GetAudioMessage().ContextInfo = ctx
	case msg.GetStickerMessage() != nil:
		msg.GetStickerMessage().ContextInfo = ctx
	case msg.GetContactMessage() != nil:
		msg.GetContactMessage().ContextInfo = ctx
	case msg.GetContactsArrayMessage() != nil:
		msg.GetContactsArrayMessage().ContextInfo = ctx
	case msg.GetLocationMessage() != nil:
		msg.GetLocationMessage().ContextInfo = ctx
	case msg.GetLiveLocationMessage() != nil:
		msg.GetLiveLocationMessage().ContextInfo = ctx
	case msg.GetGroupInviteMessage() != nil:
		msg.GetGroupInviteMessage().ContextInfo = ctx
	case pollCreation(msg) != nil:
		pollCreation(msg).ContextInfo = ctx
	case msg.GetPtvMessage() != nil:
		msg.GetPtvMessage().ContextInfo = ctx
	default:
		return false
	}
	return true
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

// resolveQuotedSenderName turns a reply's raw participant value - a JID
// string, the literal "me", or empty - into the display name the frontend
// shows in a "replied to X" suffix. It mirrors the resolveDisplayName(...)
// call quotedPreview makes for *received* replies; the send paths below used
// to skip it, leaving a raw JID (or "me") in QuotedSender until the chat was
// reloaded and re-derived from the raw protobuf from scratch. The frontend
// treats a QuotedSender containing '@' as unresolved and hides it behind a
// generic "quoted message" instead of showing it, which is what made a reply
// sent while the chat was open look different from one loaded on reopen.
func resolveQuotedSenderName(participant string) string {
	if participant == "" || participant == "me" {
		return participant
	}
	if parsed, err := types.ParseJID(participant); err == nil {
		return resolveDisplayName(parsed)
	}
	return participant
}

// mediaInfoFromMessage extracts a normalized (mediaType, mimetype, filename,
// fileSize) tuple for the media-bearing message kinds, so we can tag rows in
// the messages table for the frontend without it having to inspect raw
// protobuf. fileSize is the sender-reported byte size (0 if unset) - the same
// FileLength field downloadMessageMediaToFile uses, surfaced up front so the
// frontend can show it before the user chooses to download.
func mediaInfoFromMessage(msg *waE2E.Message) (mediaType, mimetype, filename string, fileSize int64) {
	msg = mediaCarrier(msg)
	if msg == nil {
		return "", "", "", 0
	}
	switch {
	case msg.GetImageMessage() != nil:
		m := msg.GetImageMessage()
		return "image", m.GetMimetype(), "", int64(m.GetFileLength())
	case msg.GetVideoMessage() != nil:
		m := msg.GetVideoMessage()
		return "video", m.GetMimetype(), "", int64(m.GetFileLength())
	case msg.GetAudioMessage() != nil:
		m := msg.GetAudioMessage()
		if m.GetPTT() {
			return "voice", m.GetMimetype(), "", int64(m.GetFileLength())
		}
		return "audio", m.GetMimetype(), "", int64(m.GetFileLength())
	case msg.GetDocumentMessage() != nil:
		m := msg.GetDocumentMessage()
		name := m.GetFileName()
		if name == "" {
			name = m.GetTitle()
		}
		return "document", m.GetMimetype(), sanitizeFilename(name), int64(m.GetFileLength())
	case msg.GetStickerMessage() != nil:
		m := msg.GetStickerMessage()
		return "sticker", m.GetMimetype(), "", int64(m.GetFileLength())
	}
	return "", "", "", 0
}

func isIgnoredChatJID(jid string) bool {
	return jid == "status@broadcast" || strings.HasSuffix(jid, "@broadcast") || strings.HasSuffix(jid, "@newsletter")
}

var (
	historySyncMutex sync.Mutex
	// historySyncChatsDone counts chats processed across every HistorySync
	// chunk since the last (re)connect. WhatsApp splits history into many
	// independent chunks - each with its own conversation count and a
	// "progress" percentage that's only meaningful for that one chunk/sync
	// type - so showing per-chunk numbers to the user just looks like the
	// sync is jumping backward and forward. This cumulative, monotonically
	// increasing count is what actually reads as progress.
	historySyncChatsDone int
	// historySyncMsgsDone mirrors historySyncChatsDone but counts individual
	// messages. A single conversation can hold tens of thousands of messages,
	// which takes a while to insert - without this, the status only updates
	// once per *conversation*, so it looks stalled (and an idle-timeout on
	// the frontend even reverts it to plain "Connected") while that one huge
	// chat is still being written.
	historySyncMsgsDone int
)

// resetHistorySyncProgress zeroes the running counters shown for "Syncing
// history...". Called on every fresh connect so a reconnect (or a new
// account after logging out) doesn't keep adding to a stale total.
func resetHistorySyncProgress() {
	historySyncMutex.Lock()
	historySyncChatsDone = 0
	historySyncMsgsDone = 0
	historySyncMutex.Unlock()
}

func eventHandler(evt interface{}) {
	// whatsmeow calls this synchronously from its one event-dispatch
	// goroutine, so an unrecovered panic here doesn't just drop one event -
	// it's fatal to the whole process (silently, since the runtime's panic
	// trace goes straight to the OS-level stderr fd, past our log file
	// redirect in main()). Untrusted, ever-changing server data (e.g. a bot's
	// rich-response JSON) flows through here, so one section this code
	// doesn't expect must not be able to take the backend down.
	defer func() {
		if r := recover(); r != nil {
			log.Printf("PANIC in eventHandler for %T: %v\n%s", evt, r, debug.Stack())
		}
	}()

	switch evt.(type) {
	case *events.Message, *events.Receipt, *events.HistorySync:
		debugf("Event received in eventHandler: %T", evt)
	}

	switch v := evt.(type) {
	case *events.MediaRetry:
		go handleMediaRetryEvent(v)
	case *events.HistorySyncStatus:
		handleHistorySyncStatus(v.Notification)
	case *events.HistorySync:
		historySyncPending.Add(1)
		go func() {
			historySyncMutex.Lock()
			defer historySyncMutex.Unlock()
			defer historySyncPending.Add(-1)
			chunkStart := time.Now()
			stallTimer := time.AfterFunc(historySyncStallThreshold, func() {
				dumpGoroutinesForStall("history sync chunk still writing after " + historySyncStallThreshold.String())
			})
			defer stallTimer.Stop()
			statsAtStart := db.Stats()
			chunkMsgs := 0

			if v.Data == nil {
				return
			}

			// With StreamHistorySyncConversations on, the chats arrive still
			// encoded and are decoded one at a time below, so a big chunk
			// never sits in memory whole.
			convCount := len(v.Data.GetConversations())
			conversations := func(yield func(int, *waHistorySync.Conversation) bool) {
				for i, conv := range v.Data.GetConversations() {
					if !yield(i, conv) {
						return
					}
				}
			}
			if v.Conversations != nil {
				convCount = v.Conversations.Len()
				conversations = func(yield func(int, *waHistorySync.Conversation) bool) {
					i := 0
					for conv, err := range v.Conversations.All() {
						if err != nil {
							log.Println("Skipping history sync conversation:", err)
							continue
						}
						if !yield(i, conv) {
							return
						}
						i++
					}
				}
			}
			// On-demand syncs (fetching one older chat's history when it's
			// opened) and chunks with no chat data (push names, app state)
			// shouldn't flash the "Syncing history..." status - they're not
			// part of the bulk sync the user is waiting on.
			reportProgress := v.Data.GetSyncType() != waHistorySync.HistorySync_ON_DEMAND && convCount > 0
			if reportProgress {
				broadcastEvent("history_sync", map[string]interface{}{
					"status":    "syncing",
					"chatsDone": historySyncChatsDone,
					"msgsDone":  historySyncMsgsDone,
				})
			}

			type syncedAlbum struct {
				id, jid, senderJID string
				timestamp          int64
				expected           int
			}
			var albumHeaders []syncedAlbum
			var syncedJIDs []string
			// The chats this chunk actually brought messages for - the only
			// ones the UI is told about (see history_chats_synced below).
			var jidsWithMessages []string
			// What this chunk's conversation entries turned out to be (see
			// historyConversationIsChat), logged once per chunk.
			var chunkListed, chunkSkipped, chunkTokenOnly int
			// LID pairs that only the conversations themselves named, handed
			// to whatsmeow's store once the chunk is written.
			var convLIDPairs []store.LIDMapping

			if gs := v.Data.GetGlobalSettings(); gs != nil {
				if cls := gs.GetChatLockSettings(); cls != nil {
					if sc := cls.GetSecretCode(); sc != nil {
						if b, err := proto.Marshal(sc); err == nil {
							saveChatLockSecretCode(b)
						}
					}
				}
			}

			// whatsmeow stores this chunk's LID->phone mappings in a goroutine of
			// its own (DownloadHistorySync with synchronousStorage=false), so
			// the store may not have them yet while this chunk is written -
			// its chats then landed under @lid JIDs, split off from the
			// phone-JID chat the rest of the app looks up. Resolve against the
			// chunk's own mappings first.
			chunkPNs := historyChunkLIDMap(v.Data.GetPhoneNumberToLidMappings())
			resolvePN := func(j types.JID) types.JID {
				j = j.ToNonAD()
				if j.Server != types.HiddenUserServer {
					return j
				}
				if pn, ok := chunkPNs[j.User]; ok {
					return pn
				}
				if pn, err := wa().Store.LIDs.GetPNForLID(context.Background(), j); err == nil && !pn.IsEmpty() {
					return pn.ToNonAD()
				}
				return j
			}
			mergeLIDRowsIntoPN(chunkPNs)

			// A push-name chunk can carry thousands of names; one autocommit
			// write per name made it crawl. Resolve every LID first (that goes
			// through whatsmeow's store), then write them in batches.
			var pushNameWrites []func(tx sqlExecer)
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
				targetJID := resolvePN(parsedJID)
				phone := cachePhone(targetJID)
				// whatsmeow already put these push names in its contact store
				// before dispatching the chunk; only our own tables need them.
				pushNameWrites = append(pushNameWrites, func(tx sqlExecer) {
					upsertContactCacheRowWith(tx, targetJID.String(), pushname, phone, false)
					tx.Exec(`UPDATE chats SET name = ? WHERE jid = ? AND (name IS NULL OR name = '' OR name LIKE '+%')`, pushname, targetJID.String())
				})
			}
			writeBatch(pushNameWrites)
			if len(pushNameWrites) > 0 || len(chunkPNs) > 0 {
				invalidateDisplayNames()
			}

			for convIdx, conv := range conversations {
				if reportProgress && convIdx > 0 && convIdx%5 == 0 {
					broadcastEvent("history_sync", map[string]interface{}{
						"status":    "syncing",
						"chatsDone": historySyncChatsDone + convIdx,
						"msgsDone":  historySyncMsgsDone,
					})
				}
				// Everything this chat needs is worked out first - including
				// every lookup that goes through whatsmeow's stores - and only
				// then written, in short transactions that run nothing but SQL.
				// whatsmeow's stores hold their own in-memory locks (contact
				// cache, LID map) *while* they wait for SQLite's write lock, so
				// calling into them with our write transaction open deadlocked
				// until the 10s busy timeout: sync stalled for seconds at a
				// time, and whatsmeow's own writes failed with "database is
				// locked" - once dropping a whole history-sync chunk because it
				// couldn't store the identity needed to decrypt it.
				var writes []func(tx sqlExecer)

				jid := conv.GetID()
				if isIgnoredChatJID(jid) {
					continue
				}
				chatJID, parseErr := types.ParseJID(jid)
				if parseErr != nil {
					continue
				}
				// Each chat names its own other address too; the chunk-wide
				// mapping list doesn't always include it.
				if pair, ok := conversationLIDPair(conv, chatJID); ok {
					if _, known := chunkPNs[pair.LID.User]; !known {
						chunkPNs[pair.LID.User] = pair.PN
						convLIDPairs = append(convLIDPairs, pair)
					}
				}
				wireJID := ""
				if chatJID.Server == types.HiddenUserServer {
					wireJID = chatJID.ToNonAD().String()
				}
				chatJID = resolvePN(chatJID)
				jid = chatJID.String()
				if wireJID == jid {
					wireJID = ""
				}
				listed := historyConversationIsChat(conv, chatJID)
				if listed {
					chunkListed++
				} else {
					chunkSkipped++
					if conv.GetTcToken() != nil {
						chunkTokenOnly++
					}
				}
				if listed {
					syncedJIDs = append(syncedJIDs, jid)
				}
				if len(conv.GetMessages()) > 0 {
					jidsWithMessages = append(jidsWithMessages, jid)
				}

				name := conv.GetName()
				if name == "" && chatJID.Server == types.GroupServer && listed {
					// A group's name is its subject, which only the server has.
					requestGroupMetadata(chatJID)
				}
				if name == "" && chatJID.Server != types.GroupServer {
					if contact, err := wa().Store.Contacts.GetContact(context.Background(), chatJID); err == nil && contact.Found {
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
				muted := boolToInt(historyMuteActive(conv.GetMuteEndTime()))
				archived := 0
				if conv.GetArchived() {
					archived = 1
				}
				locked := 0
				if conv.GetLocked() {
					locked = 1
				}
				markedUnread := boolToInt(conv.GetMarkedAsUnread())
				if markedUnread == 1 && unread == 0 {
					unread = 1
				}
				if listed {
					writes = append(writes, func(tx sqlExecer) {
						_, err := tx.Exec(`INSERT INTO chats (jid, name, timestamp, unread_count, pinned, muted, archived, is_locked, marked_unread, wire_jid) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
							ON CONFLICT(jid) DO UPDATE SET
							timestamp=MAX(timestamp, excluded.timestamp),
							name=CASE WHEN chats.name IS NULL OR chats.name = '' OR chats.name LIKE '+%' THEN COALESCE(NULLIF(excluded.name, ''), chats.name) ELSE chats.name END,
							unread_count=MAX(chats.unread_count, excluded.unread_count),
							pinned=MAX(chats.pinned, excluded.pinned),
							muted=MAX(chats.muted, excluded.muted),
							archived=MAX(chats.archived, excluded.archived),
							is_locked=MAX(chats.is_locked, excluded.is_locked),
							marked_unread=MAX(COALESCE(chats.marked_unread, 0), excluded.marked_unread),
							wire_jid=COALESCE(NULLIF(chats.wire_jid, ''), excluded.wire_jid)`,
							jid, name, ts, unread, pinned, muted, archived, locked, markedUnread, wireJID)
						if err != nil {
							fmt.Println("Error inserting chat from HistorySync:", err)
						}
					})
				}
				// Advanced chat privacy, applied only if newer than what we have.
				if conv.LimitSharing != nil {
					limitSharing, limitSharingTS := boolToInt(conv.GetLimitSharing()), conv.GetLimitSharingSettingTimestamp()
					writes = append(writes, func(tx sqlExecer) {
						tx.Exec(`UPDATE chats SET limit_sharing = ?, limit_sharing_ts = ? WHERE jid = ? AND COALESCE(limit_sharing_ts, 0) <= ?`,
							limitSharing, limitSharingTS, jid, limitSharingTS)
					})
				}
				// Disappearing messages, likewise only if newer than ours.
				if conv.EphemeralExpiration != nil {
					expiration, expirationTS := conv.GetEphemeralExpiration(), conv.GetEphemeralSettingTimestamp()
					writes = append(writes, func(tx sqlExecer) {
						tx.Exec(`UPDATE chats SET ephemeral_expiration = ?, ephemeral_setting_ts = ? WHERE jid = ? AND COALESCE(ephemeral_setting_ts, 0) <= ?`,
							expiration, expirationTS, jid, expirationTS)
					})
				}

				chunkMsgs += len(conv.GetMessages())
				for _, historyMsg := range conv.GetMessages() {
					webMsg := historyMsg.GetMessage()
					if webMsg == nil {
						continue
					}
					if webMsg.GetMessageStubType() == waWeb.WebMessageInfo_REVOKE {
						stubID := webMsg.GetKey().GetID()
						if stubID != "" {
							stubFromMe := webMsg.GetKey().GetFromMe()
							stubTS := int64(webMsg.GetMessageTimestamp())
							// Without a sender the chat can only show "Contact:
							// deleted this message" in a group, rather than who
							// deleted it. 1:1 chats leave it empty as before.
							stubSender := ""
							if !stubFromMe {
								if s := historyStubActor(webMsg); !s.IsEmpty() {
									stubSender = s.String()
								}
							}
							writes = append(writes, func(tx sqlExecer) {
								if _, err := tx.Exec(`INSERT INTO messages (id, jid, from_me, text, timestamp, is_deleted, sender_jid)
										VALUES (?, ?, ?, '', ?, 1, ?)
										ON CONFLICT(id) DO UPDATE SET is_deleted=1, text='',
											sender_jid=COALESCE(NULLIF(messages.sender_jid, ''), excluded.sender_jid)`,
									stubID, jid, stubFromMe, stubTS, stubSender); err != nil {
									log.Println("Error storing revoked stub from HistorySync:", err)
								}
							})
						}
						continue
					}
					// Group events (someone added, left, made admin, the
					// subject changed...) arrive in history sync as content-less
					// "stub" entries. Stored as-is they became empty rows the
					// chat rendered as "[Media]"; turn the ones the live path
					// also describes into the same system lines, drop the rest.
					if webMsg.GetMessageStubType() != 0 && webMsg.GetMessage() == nil {
						if text := historyStubText(webMsg); text != "" {
							stubTS := int64(webMsg.GetMessageTimestamp())
							// Same id the live GroupInfo handler gives this line, so
							// a resync doesn't duplicate one we already showed live.
							stubID := fmt.Sprintf("system_%s_%d_0", chatJID.User, time.Unix(stubTS, 0).UnixNano())
							writes = append(writes, func(tx sqlExecer) {
								if _, err := tx.Exec(`INSERT INTO messages (id, jid, from_me, text, timestamp, status, media_type)
										VALUES (?, ?, 0, ?, ?, 'sent', 'system')
										ON CONFLICT(id) DO NOTHING`,
									stubID, jid, text, stubTS); err != nil {
									log.Println("Error storing group event from HistorySync:", err)
								}
							})
						}
						continue
					}
					evtMsg, parseEvtErr := wa().ParseWebMessage(chatJID, webMsg)
					if parseEvtErr != nil || evtMsg == nil {
						continue
					}
					if pm := evtMsg.Message.GetProtocolMessage(); pm != nil && pm.GetType() == waE2E.ProtocolMessage_REVOKE {
						if targetID := pm.GetKey().GetID(); targetID != "" {
							fromMe, revokeTS := evtMsg.Info.IsFromMe, evtMsg.Info.Timestamp.Unix()
							writes = append(writes, func(tx sqlExecer) {
								if _, err := tx.Exec(`INSERT INTO messages (id, jid, from_me, text, timestamp, is_deleted)
										VALUES (?, ?, ?, '', ?, 1)
										ON CONFLICT(id) DO UPDATE SET is_deleted=1, text=''`,
									targetID, jid, fromMe, revokeTS); err != nil {
									log.Println("Error storing revoke from HistorySync:", err)
								}
							})
						}
						continue
					}
					if reaction := evtMsg.Message.GetReactionMessage(); reaction != nil {
						writes = append(writes, func(tx sqlExecer) { storeReactionTx(tx, jid, evtMsg, reaction) })
						continue
					}
					if evtMsg.Message.GetEncReactionMessage() != nil {
						// Same announcement-group encrypted-reaction case as the
						// live-event path below - decrypt it instead of letting
						// it fall through to extractText's "[Unsupported
						// Message]" and get inserted as a bogus chat message.
						if reaction, err := wa().DecryptReaction(context.Background(), evtMsg); err == nil {
							writes = append(writes, func(tx sqlExecer) { storeReactionTx(tx, jid, evtMsg, reaction) })
						} else {
							log.Printf("Failed to decrypt announcement-group reaction from HistorySync: %v", err)
						}
						continue
					}
					if update := unwrapMessage(evtMsg.Message).GetPollUpdateMessage(); update != nil {
						// A vote is not a message of its own: record it on
						// its poll instead of storing an empty row.
						if vote, err := decryptPollVote(evtMsg); err == nil {
							pollID, voter := update.GetPollCreationMessageKey().GetID(), pollVoterKey(evtMsg.Info.Sender)
							ts := evtMsg.Info.Timestamp.Unix()
							writes = append(writes, func(tx sqlExecer) { storePollVote(tx, pollID, voter, vote.GetSelectedOptions(), ts) })
						}
						continue
					}
					if len(webMsg.GetPollUpdates()) > 0 {
						pollMsg := webMsg
						writes = append(writes, func(tx sqlExecer) { historyPollVotes(tx, chatJID, pollMsg) })
					}
					if enc := unwrapMessage(evtMsg.Message).GetEncEventResponseMessage(); enc != nil {
						// An RSVP is recorded on its event, like a vote.
						if resp, err := decryptEventResponse(evtMsg); err == nil {
							eventID, responder := enc.GetEventCreationMessageKey().GetID(), pollVoterKey(evtMsg.Info.Sender)
							ts := evtMsg.Info.Timestamp.Unix()
							writes = append(writes, func(tx sqlExecer) { storeEventResponse(tx, eventID, responder, resp, ts) })
						}
						continue
					}
					if len(webMsg.GetEventResponses()) > 0 {
						eventMsg := webMsg
						writes = append(writes, func(tx sqlExecer) { historyEventResponses(tx, chatJID, eventMsg) })
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
					if evtMsg.IsViewOnce {
						msgText = viewOnceText(msgText)
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
							} else {
								senderJID = resolvePN(senderJID)
							}
						}
						msgWireSender := wireSender(chatJID, evtMsg.Info.Sender, senderJID)
						rawBytes, _ := proto.Marshal(evtMsg.Message)
						mediaType, mimetype, filename, mediaSize := mediaInfoFromMessage(evtMsg.Message)
						quotedID, quotedText, quotedSender := quotedPreview(evtMsg.Message)
						status := ""
						var sentAt, deliveredAt, readAt, playedAt int64
						if msgFromMe {
							sentAt = msgTS
							isGroupMsg := strings.HasSuffix(msgJID, "@g.us")
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
								// A group message's receipt list is per member - the
								// only way Message Info's "Read by" breakdown can
								// cover messages whose live receipts we never saw
								// (sent before this device was linked, or while
								// re-linking).
								if !isGroupMsg {
									continue
								}
								userJID, err := types.ParseJID(ur.GetUserJID())
								if err != nil || userJID.User == "" {
									continue
								}
								participant := preferPN(userJID).String()
								for _, r := range []struct {
									status string
									ts     int64
								}{
									{"delivered", ur.GetReceiptTimestamp()},
									{"read", ur.GetReadTimestamp()},
									{"played", ur.GetPlayedTimestamp()},
								} {
									if r.ts > 0 {
										writes = append(writes, func(tx sqlExecer) {
											if err := recordParticipantReceipt(tx, msgID, participant, r.status, r.ts); err != nil {
												log.Println("Error storing participant receipt from HistorySync:", err)
											}
										})
									}
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
						writes = append(writes, func(tx sqlExecer) {
							_, err := tx.Exec(`INSERT INTO messages
								(id, jid, from_me, text, timestamp, sender_jid, raw, status, media_type, media_mimetype, media_filename, media_size, quoted_id, quoted_text, quoted_sender, sent_at, delivered_at, read_at, played_at, wire_sender)
							VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
							ON CONFLICT(id) DO UPDATE SET text=excluded.text, timestamp=excluded.timestamp, sender_jid=excluded.sender_jid, raw=excluded.raw,
								wire_sender=COALESCE(NULLIF(excluded.wire_sender, ''), messages.wire_sender),
								status=CASE WHEN excluded.status != '' THEN excluded.status ELSE messages.status END,
								media_type=excluded.media_type, media_mimetype=excluded.media_mimetype, media_filename=excluded.media_filename, media_size=excluded.media_size,
								quoted_id=excluded.quoted_id, quoted_text=excluded.quoted_text, quoted_sender=excluded.quoted_sender,
								sent_at=CASE WHEN excluded.sent_at != 0 THEN excluded.sent_at ELSE messages.sent_at END,
								delivered_at=CASE WHEN messages.delivered_at != 0 THEN messages.delivered_at ELSE excluded.delivered_at END,
								read_at=CASE WHEN messages.read_at != 0 THEN messages.read_at ELSE excluded.read_at END,
								played_at=CASE WHEN messages.played_at != 0 THEN messages.played_at ELSE excluded.played_at END`,
								msgID, msgJID, msgFromMe, msgText, msgTS, senderJID.String(), rawBytes, status,
								mediaType, mimetype, filename, mediaSize, quotedID, quotedText, quotedSender, sentAt, deliveredAt, readAt, playedAt, msgWireSender)
							if err != nil {
								log.Println("Error inserting message from HistorySync:", err)
								return
							}
							noteMessageExpiry(tx, msgID, msgTS, evtMsg.Message)
							if evtMsg.IsViewOnce {
								markViewOnce(tx, msgID)
							}
						})
					}
				}

				writeBatch(writes)
				historySyncMsgsDone += len(conv.GetMessages())
				historySyncChatsDone++
				if reportProgress && len(conv.GetMessages()) >= 200 {
					broadcastEvent("history_sync", map[string]interface{}{
						"status":    "syncing",
						"chatsDone": historySyncChatsDone,
						"msgsDone":  historySyncMsgsDone,
					})
				}
			}
			for _, album := range albumHeaders {
				stampAlbumMembers(album.id, album.jid, album.senderJID, album.timestamp, album.expected)
			}
			if len(convLIDPairs) > 0 {
				if err := wa().Store.LIDs.PutManyLIDMappings(context.Background(), convLIDPairs); err != nil {
					log.Println("Error storing LID mappings from history-sync conversations:", err)
				}
			}
			loopTook := time.Since(chunkStart)
			tailStart := time.Now()
			// Only this chunk's chats can have a new latest message.
			for _, j := range syncedJIDs {
				UpdateChatSnippet(j)
			}
			// The all-chats snippet pass and the contact/group re-sync (which
			// also fetches the group list from WhatsApp) cost more the bigger
			// the database gets. Running them whenever the queue was empty
			// meant nearly every chunk, since the phone sends slower than we
			// write - all while holding historySyncMutex. They now run once the
			// sync has gone quiet, outside the lock.
			//
			// Not for an on-demand chunk: that's older messages for one chat,
			// whose preview (its newest message) can't change from them, and it
			// brings no new chats, contacts or groups. Refreshing after each one
			// re-read every chat's preview and fetched the group list again for
			// every 50 older messages loaded.
			if v.Data.GetSyncType() != waHistorySync.HistorySync_ON_DEMAND {
				scheduleHistorySyncRefresh()
			}
			broadcastChats()
			tailTook := time.Since(tailStart)

			if reportProgress {
				broadcastEvent("history_sync", map[string]interface{}{
					"status":    "syncing",
					"chatsDone": historySyncChatsDone,
					"msgsDone":  historySyncMsgsDone,
				})
			}
			// Sent for every chunk, on-demand ones included: an on-demand
			// chunk is the phone's answer to "load older messages", and the
			// UI waits on exactly this to page them in. It used to ride on the
			// progress event above, which on-demand chunks skip - so the answer
			// was written but never announced, the UI timed out and marked the
			// chat as having no more history until whaNTV was restarted.
			// Only chats that got messages: an empty answer to an on-demand
			// request (the phone has nothing older) must not look like new
			// history, or the UI would un-mark the chat and ask again.
			if v.Data.GetSyncType() == waHistorySync.HistorySync_ON_DEMAND {
				for _, j := range jidsWithMessages {
					noteOnDemandAnswer(j)
				}
			}
			if len(jidsWithMessages) > 0 {
				broadcastEvent("history_chats_synced", map[string]interface{}{
					"jids":     jidsWithMessages,
					"onDemand": v.Data.GetSyncType() == waHistorySync.HistorySync_ON_DEMAND,
				})
			}
			if convCount > 0 || v.Data.GetSyncType() == waHistorySync.HistorySync_FULL {
				stats := db.Stats()
				log.Printf("History sync chunk written: type=%s chunk=%d progress=%d chats=%d (listed=%d skipped=%d tokenOnly=%d) msgs=%d took=%s (messages %s, snippets %s) queued=%d dbWait=%d/%s inUse=%d",
					v.Data.GetSyncType(), v.Data.GetChunkOrder(), v.Data.GetProgress(), convCount, chunkListed, chunkSkipped, chunkTokenOnly, chunkMsgs,
					time.Since(chunkStart).Round(time.Millisecond), loopTook.Round(time.Millisecond), tailTook.Round(time.Millisecond),
					historySyncPending.Load()-1,
					stats.WaitCount-statsAtStart.WaitCount, (stats.WaitDuration - statsAtStart.WaitDuration).Round(time.Millisecond),
					stats.InUse)
			}
			if v.Data.GetSyncType() == waHistorySync.HistorySync_FULL && v.Data.GetProgress() >= 100 {
				scheduleHistorySyncCheckpoint()
			}
		}()

	case *events.Message:
		unwrappedMsg := unwrapMessage(v.Message)
		if setting, toggle := limitSharingInMessage(v.Message); setting != nil {
			chat, _ := resolveLIDChat(v)
			ts := setting.GetLimitSharingSettingTimestamp()
			if ts == 0 {
				ts = v.Info.Timestamp.UnixMilli()
			}
			go func() {
				if setChatLimitSharing(chat.String(), setting.GetSharingLimited(), ts) && toggle {
					noteChatPrivacyChange(chat.String(), v.Info.Sender, setting.GetSharingLimited(), time.UnixMilli(ts))
				}
			}()
			if toggle {
				return
			}
		}
		// A channel admin's edit or deletion of a post comes as the post's
		// own ID with an edit attribute: the new content, or nothing.
		if v.Info.Chat.Server == types.NewsletterServer {
			switch v.Info.Edit {
			case types.EditAttributeAdminRevoke:
				jid := v.Info.Chat.String()
				markMessageDeleted(v.Info.ID, jid)
				broadcastEvent("message_deleted", map[string]interface{}{
					"jid": jid, "message_id": v.Info.ID, "by": jid,
				})
				return
			case types.EditAttributeAdminEdit:
				jid, id := v.Info.Chat.String(), v.Info.ID
				editTime := v.Info.Timestamp.Unix()
				func() {
					shown, _ := markMessageEdited(id, jid, v.Message, extractText(v.Message), editTime)
					broadcastEvent("message_edited", map[string]interface{}{
						"jid": jid, "message_id": id, "text": shown, "edited_at": editTime,
					})
				}()
				return
			}
		}
		isEdit := v.IsEdit || v.Info.Edit == types.EditAttributeMessageEdit || (unwrappedMsg != nil && unwrappedMsg.GetProtocolMessage() != nil && unwrappedMsg.GetProtocolMessage().GetType() == waE2E.ProtocolMessage_MESSAGE_EDIT)
		if isEdit {
			var targetID string
			var newContent *waE2E.Message

			if enc := v.Message.GetSecretEncryptedMessage(); enc != nil && enc.GetSecretEncType() == waE2E.SecretEncryptedMessage_MESSAGE_EDIT {
				targetID = enc.GetTargetMessageKey().GetID()
				decrypted, err := wa().DecryptSecretEncryptedMessage(context.Background(), v)
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
				func() {
					// An edit made on another of our devices reaches us as a
					// normal message; its receipts will name this stanza too.
					if v.Info.IsFromMe {
						recordEditStanza(v.Info.ID, targetID, jid, editTime)
					}
					// Broadcast the merged text, not the edit's bare text: a
					// caption edit carries only the caption, and the row
					// would otherwise lose its "[Image]" marker on screen.
					shown, _ := markMessageEdited(targetID, jid, newContent, newText, editTime)
					broadcastEvent("message_edited", map[string]interface{}{
						"jid":        jid,
						"message_id": targetID,
						"text":       shown,
						"edited_at":  editTime,
					})
				}()
			}
			return
		}
		if pm := unwrappedMsg.GetProtocolMessage(); pm != nil && pm.GetType() == waE2E.ProtocolMessage_EPHEMERAL_SETTING {
			applyEphemeralSettingMessage(v, pm)
			return
		}
		if pm := unwrappedMsg.GetProtocolMessage(); pm != nil && pm.GetType() == waE2E.ProtocolMessage_GROUP_MEMBER_LABEL_CHANGE {
			applyMemberLabelChange(v)
			return
		}
		if unwrappedMsg != nil && unwrappedMsg.GetProtocolMessage() != nil && unwrappedMsg.GetProtocolMessage().GetType() == waE2E.ProtocolMessage_REVOKE {
			targetID := unwrappedMsg.GetProtocolMessage().GetKey().GetID()
			if v.Info.Chat == types.StatusBroadcastJID {
				go removeStatus(targetID)
				return
			}
			resolvedChat, _ := resolveLIDChat(v)
			jid := resolvedChat.String()
			markMessageDeleted(targetID, jid)
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
		if pin := unwrappedMsg.GetPinInChatMessage(); pin != nil {
			applyPinInChat(v, pin)
			return
		}
		if keep := unwrappedMsg.GetKeepInChatMessage(); keep != nil {
			applyKeepInChat(v, keep)
			return
		}
		if unwrappedMsg.GetPollUpdateMessage() != nil {
			applyPollVote(v)
			return
		}
		if unwrappedMsg.GetEncEventResponseMessage() != nil {
			applyEventResponse(v)
			return
		}
		if unwrappedMsg.GetEncReactionMessage() != nil {
			// Reactions in community announcement groups are delivered as
			// EncReactionMessage (encrypted separately from the outer stanza,
			// since only admins can post but anyone can react). Without this,
			// GetReactionMessage() above is nil, extractText falls through to
			// "[Unsupported Message]", and the reaction gets inserted as a
			// bogus standalone chat message instead of being applied.
			reaction, err := wa().DecryptReaction(context.Background(), v)
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

		if isKeyDistributionOnly(unwrappedMsg) {
			return
		}

		text := extractText(v.Message)
		if text == "[Protocol Message]" {
			return
		}
		if v.IsViewOnce {
			// Already opened here: a redelivered copy must not bring it back.
			if viewOnceState(v.Info.ID) == viewOnceOpened {
				return
			}
			text = viewOnceText(text)
		}
		if text == "[Unsupported Message]" && v.Info.Sender.IsBot() {
			// Bots (Meta AI etc.) occasionally ship new response primitives
			// richResponseText doesn't cover yet - dump the decrypted message
			// so a new one can be identified and added there.
			// Structure only unless verbose logging is on - this is message content.
			if *verboseLogFlag {
				log.Printf("Unrecognized bot message content from %s: %s", v.Info.Sender, unwrapMessage(v.Message).String())
			} else {
				log.Printf("Unrecognized bot message content from %s (enable verbose logging to capture it)", v.Info.Sender)
			}
		}
		if (v.Info.MsgBotInfo.EditType == types.EditTypeInner || v.Info.MsgBotInfo.EditType == types.EditTypeLast) &&
			v.Info.MsgBotInfo.EditTargetID != "" {
			// A bot streams one reply as a sequence of frames sharing the
			// first frame's message ID via edit_target_id ("first" once,
			// then "inner" repeatedly as it generates, then "last") rather
			// than sending the finished text once - collapse them into
			// updates of that first bubble instead of a new one per frame.
			resolvedChat, _ := resolveLIDChat(v)
			jid := resolvedChat.String()
			targetID := v.Info.MsgBotInfo.EditTargetID
			editTime := v.Info.Timestamp.Unix()
			markMessageEdited(targetID, jid, unwrapMessage(v.Message), text, editTime)
			broadcastEvent("message_edited", map[string]interface{}{
				"jid":        jid,
				"message_id": targetID,
				"text":       text,
				"edited_at":  editTime,
				// "inner" frames are Meta AI still generating - the client
				// updates the bubble's text from these but only announces
				// the "last" one, or a screen reader would read out every
				// growing partial draft of the answer as it streams in.
				"final": v.Info.MsgBotInfo.EditType == types.EditTypeLast,
			})
			return
		}
		debugf("Received events.Message. ID: %s, Sender: %s, Chat: %s, text length: %d", v.Info.ID, v.Info.Sender.String(), v.Info.Chat.String(), len(text))

		// Claimed here rather than inside the goroutine below so that media
		// gets stamped in arrival order, which is the order the album header
		// counted them in.
		albumID := ""
		if mt, _, _, _ := mediaInfoFromMessage(v.Message); mt == "image" || mt == "video" {
			albumChat, albumSender := resolveLIDChat(v)
			albumID = claimAlbumID(albumChat.String(), albumSender.String())
		}

		// Stored before the handler returns, not on a goroutine of its own:
		// whatsmeow counts the event as handled (and drops its buffered
		// plaintext) once we return, and events are handled in the order they
		// arrive - so an edit or delete that follows this message can no longer
		// be applied before the message itself exists.
		func() {
			chatJID, senderJID := resolveLIDChat(v)

			jid := chatJID.String()

			if isIgnoredChatJID(jid) && !isFollowedChannelPost(jid) {
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
			_, err := db.Exec(`INSERT INTO chats (jid, timestamp, unread_count, preview_snippet, preview_sender, preview_from_me, wire_jid) VALUES (?, ?, ?, ?, ?, ?, ?)
				ON CONFLICT(jid) DO UPDATE SET
					timestamp=MAX(timestamp, excluded.timestamp),
					unread_count=chats.unread_count + excluded.unread_count,
					preview_snippet=COALESCE(NULLIF(excluded.preview_snippet, ''), chats.preview_snippet),
					preview_sender=excluded.preview_sender,
					preview_from_me=excluded.preview_from_me,
					preview_is_reaction=0,
					preview_reaction_emoji='',
					wire_jid=COALESCE(NULLIF(excluded.wire_jid, ''), chats.wire_jid)`,
				jid, ts, increment, text, senderJID.String(), v.Info.IsFromMe, wireChat(v.Info.Chat, chatJID))
			if err != nil {
				fmt.Println("Error updating chat timestamp:", err)
			}
			// A group we have no name for - just joined, or the join notice
			// hasn't been handled yet - is looked up, as WhatsApp Web does,
			// rather than listed as its bare JID.
			if chatJID.Server == types.GroupServer {
				var name string
				db.QueryRow(`SELECT COALESCE(name, '') FROM chats WHERE jid = ?`, jid).Scan(&name)
				if name == "" {
					requestGroupMetadata(chatJID)
				}
			}
			if v.Info.ID != "" {
				mediaType, mimetype, filename, mediaSize := mediaInfoFromMessage(v.Message)
				quotedID, quotedText, quotedSender := quotedPreview(v.Message)
				status := ""
				if v.Info.IsFromMe {
					status = "sent"
				}
				rawBytes, _ := proto.Marshal(v.Message)
				err := saveMessage(StoredMessage{
					ID: v.Info.ID, JID: jid, SenderJID: senderJID.String(), WireSender: wireSender(chatJID, v.Info.Sender, senderJID),
					FromMe: v.Info.IsFromMe, Text: text, Timestamp: ts, Raw: rawBytes, Status: status,
					MediaType: mediaType, MediaMimetype: mimetype, MediaFilename: filename, MediaSize: mediaSize,
					QuotedID: quotedID, QuotedText: quotedText, QuotedSender: quotedSender,
					AlbumID: albumID, ServerID: int64(v.Info.ServerID),
				})
				if err != nil {
					fmt.Println("Error inserting message from live event:", err)
				} else {
					noteMessageExpiry(db, v.Info.ID, ts, v.Message)
					adoptChatDisappearing(chatJID, v.Message)
					if v.IsViewOnce {
						markViewOnce(db, v.Info.ID)
					}
					if mediaType == "voice" {
						autoDownloadVoiceNote(v.Info.ID, jid)
					}
				}
			}
			broadcastChats()
		}()
		if text != "" {
			chatJID, senderJID := resolveLIDChat(v)
			if isIgnoredChatJID(chatJID.String()) && !isFollowedChannelPost(chatJID.String()) {
				return
			}
			liveMediaType, _, liveMediaFilename, liveMediaSize := mediaInfoFromMessage(v.Message)

			quotedID, quotedText, quotedSender := quotedPreview(v.Message)
			senderName := ""
			if !v.Info.IsFromMe {
				senderName = resolveDisplayName(senderJID)
			}
			broadcastEvent("message", map[string]interface{}{
				"jid":           chatJID.String(),
				"from":          senderJID.String(),
				"senderName":    senderName,
				"id":            v.Info.ID,
				"text":          text,
				"from_me":       v.Info.IsFromMe,
				"timestamp":     v.Info.Timestamp.Unix(),
				"has_media":     liveMediaType != "",
				"mediaType":     liveMediaType,
				"mediaFilename": liveMediaFilename,
				"mediaSize":     liveMediaSize,
				"albumId":       albumID,
				"quotedId":      quotedID,
				"quotedText":    quotedText,
				"quotedSender":  quotedSender,
				"pollResults":   newPollResults(unwrappedMsg),
			})
		}
	case *events.UndecryptableMessage:
		go storeUndecryptablePlaceholder(v)
	case *events.IdentityChange:
		go noteSecurityCodeChange(v)
	case *events.PushName:
		invalidateDisplayNames()
		go func() {
			// whatsmeow has already stored the name. A sender named only by
			// LID is cached under their phone JID when that's known, and a
			// LID's digits are never cached as a phone number.
			target := preferPN(v.JID)
			if target.Server == types.HiddenUserServer && v.JIDAlt.Server == types.DefaultUserServer {
				target = v.JIDAlt.ToNonAD()
			}
			jidStr := target.String()
			if v.NewPushName != "" {
				upsertContactCacheRow(jidStr, v.NewPushName, cachePhone(target), false)
				db.Exec(`UPDATE chats SET name = ? WHERE jid = ? AND (name IS NULL OR name = '' OR name LIKE '+%')`, v.NewPushName, jidStr)
				broadcastChats()
			}
		}()
	case *events.Contact:
		invalidateDisplayNames()
		if v.FromFullSync && !v.Removed {
			// A full app-state sync (first link, or a resync) delivers every
			// address-book entry at once - thousands of events, each of which
			// cost a goroutine, two single-row writes and a chat-list rebuild,
			// all fighting history sync for SQLite's write lock. whatsmeow has
			// already stored the name itself, and the AppStateSyncComplete that
			// ends the sync reruns populateChatsFromStore, which writes every
			// contact (LID twins merged) in batches.
			return
		}
		go func() {
			jidStr := preferPN(v.JID).String()
			if v.Removed {
				deleteContactCacheRow(jidStr)
				db.Exec(`UPDATE chats SET name = '' WHERE jid = ?`, jidStr)
				populateChatsFromStore()
				broadcastChats()
				return
			}
			fullName := ""
			if v.Action != nil {
				fullName = v.Action.GetFullName()
				if fullName == "" {
					fullName = v.Action.GetFirstName()
				}
			}
			if fullName != "" {
				// whatsmeow already stored this name (with the real first name)
				// before dispatching the event.
				phone := ""
				if pj, err := types.ParseJID(jidStr); err == nil && pj.Server == types.DefaultUserServer {
					phone = pj.User
				}
				upsertContactCacheRow(jidStr, fullName, phone, false)
				db.Exec(`UPDATE chats SET name = ? WHERE jid = ?`, fullName, jidStr)
				broadcastChats()
			}
		}()
	case *events.BusinessName:
		invalidateDisplayNames()
		go func() {
			target := preferPN(v.JID)
			jidStr := target.String()
			if v.NewBusinessName != "" {
				upsertContactCacheRow(jidStr, v.NewBusinessName, cachePhone(target), true)
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
		forgetPresenceSubscriptions()
		forgetChannelRefreshes()
		convertStoredVotesOnce.Do(func() { go runOnce("convert_stored_poll_votes_v1", convertStoredPollVotes) })
		go refreshServerFlags()
		appStateResyncMu.Lock()
		for k := range appStateResyncCount {
			delete(appStateResyncCount, k)
		}
		for k := range lastAppStateResync {
			delete(lastAppStateResync, k)
		}
		appStateResyncMu.Unlock()
		connFailures.Store(0)
		if wa().Store.ID != nil {
			setConnState(connStateOnline, "", 0)
			// Deliberately not resetting history-sync progress here: this event
			// also fires on every keepalive-triggered auto-reconnect, not just
			// the first connect of a session (that case is covered once in
			// main.go). A sync spanning a large account can easily outlast a
			// couple of those blips, and zeroing the counters (or the frontend
			// clobbering its "Syncing history..." status text back to plain
			// "Connected") on each one made the status look stalled or reset
			// while it was still writing in the background.
			broadcastEvent("connected", map[string]string{"jid": wa().Store.ID.String()})
			// Whatever the window's actually doing - not "available" on every
			// reconnect regardless (see desiredPresence).
			go sendDesiredPresence(true)
			go retryPendingTextMessages()
			go flushPendingResends()
			go refreshBlocklistOnConnect()
			go func() {
				time.Sleep(3 * time.Second)
				populateChatsFromStore()
				broadcastChats()
			}()
		}
	case *events.PairSuccess:
		log.Printf("Successfully paired with %s", v.ID.String())
		broadcastEvent("pair_success", map[string]string{"jid": v.ID.String()})
		go sendDesiredPresence(true)
		go func() {
			time.Sleep(5 * time.Second)
			populateChatsFromStore()
			broadcastChats()
		}()
	case *events.LoggedOut:
		// WhatsApp has already unpaired this device server-side (phone
		// unlinked it, session expired, etc.) and whatsmeow has already
		// wiped the store's credentials by the time this event arrives - the
		// old *never* auto-reconnects from here, it would just fail forever.
		// Swap in a fresh, unpaired client so re-pairing works without
		// restarting the app, same as an explicit "Log Out".
		reason := v.Reason.String()
		log.Println("Logged out by WhatsApp:", reason)
		go func() {
			replaceWithFreshClient()
			broadcastEvent("needs_login", map[string]string{"reason": reason})
		}()
	case *events.StreamReplaced:
		// Another client (a second copy of the backend, or this account
		// linked from elsewhere) took over this session's connection.
		// whatsmeow won't auto-reconnect after this - doing so immediately
		// would just bounce the other session right back off.
		log.Println("Session replaced by another connection")
		msg := "whaNTV was disconnected because this WhatsApp account connected from another session. Use Reconnect Now in the Account menu to take the session back."
		broadcastEvent("error", map[string]string{"message": msg})
		setConnState(connStateStopped, "Disconnected: this account connected from another session.", 0)
	case *events.TemporaryBan:
		// whatsmeow stops reconnecting on a temp ban; try again once it
		// should have lifted rather than staying offline until a restart.
		log.Println("Temporary ban:", v.String())
		broadcastEvent("error", map[string]string{"message": v.String()})
		if v.Expire > 0 {
			setConnState(connStateOffline, "Temporarily banned by WhatsApp.", v.Expire)
			requestConnect(v.Expire + 5*time.Second)
		} else {
			scheduleRetryAfterFailure("Temporarily banned by WhatsApp.")
		}
	case *events.ClientOutdated:
		log.Println("WhatsApp rejected this client as outdated")
		broadcastEvent("error", map[string]string{
			"message": "WhatsApp has rejected this version of whaNTV as outdated. Please update whaNTV to reconnect.",
		})
		setConnState(connStateStopped, "This version of whaNTV is outdated. Please update it.", 0)
	case *events.CATRefreshError:
		log.Println("CAT refresh error:", v.Error)
		scheduleRetryAfterFailure("Couldn't refresh WhatsApp's connection token.")
	case *events.ConnectFailure:
		// Reasons whatsmeow doesn't know how to recover from: it has
		// already given up, so retry ourselves instead of leaving the app
		// permanently offline behind an error popup.
		log.Println("Unhandled connect failure:", v.Reason.String(), v.Message)
		scheduleRetryAfterFailure("WhatsApp refused the connection (" + v.Reason.String() + ").")
	case *events.StreamError:
		log.Println("Unknown stream error, code:", v.Code)
	case *events.Disconnected:
		// whatsmeow's own reconnect loop is already running; autoReconnectHook
		// reports any failed attempts from there.
		setConnState(connStateConnecting, "Connection lost.", 0)
	case *events.ChatPresence:
		isComposing := v.State == types.ChatPresenceComposing
		isRecording := isComposing && v.Media == types.ChatPresenceMediaAudio
		isTyping := isComposing && !isRecording

		chatJID, senderJID := resolveSourcePN(v.MessageSource)

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
			if pn, err := wa().Store.LIDs.GetPNForLID(context.Background(), targetJID); err == nil && !pn.IsEmpty() {
				targetJID = pn.ToNonAD()
			}
		}

		broadcastEvent("presence", map[string]interface{}{
			"jid":         targetJID.String(),
			"unavailable": v.Unavailable,
			"last_seen":   lastSeenUnix,
		})
	case *events.Receipt:
		if v.MessageSource.Chat == types.StatusBroadcastJID && !v.MessageSource.IsFromMe {
			go noteStatusViewers(v)
		}
		chatJID, senderJID := resolveSourcePN(v.MessageSource)

		broadcastEvent("receipt", map[string]interface{}{
			"chat_jid":    chatJID.String(),
			"sender_jid":  senderJID.String(),
			"message_ids": v.MessageIDs,
			"type":        string(v.Type),
			"timestamp":   v.Timestamp.Unix(),
		})
		debugf("Received Receipt event - Chat: %s, Sender: %s, Type: %s, MessageIDs: %v", chatJID.String(), senderJID.String(), string(v.Type), v.MessageIDs)
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
			resolvedChatJID := chatJID
			// One member's receipt for our own group message only counts
			// towards that member; the message's own ticks follow the whole
			// group (see updateGroupMessageStatuses).
			updatedByStatus := map[string][]string{}
			if chatJID.Server == "g.us" && !v.MessageSource.IsFromMe && (status == "delivered" || status == "read" || status == "played") {
				updateParticipantReceiptIfNewer(ids, senderJID.String(), status, receiptTS)
				updatedByStatus = updateGroupMessageStatuses(ids, chatJID.String(), receiptTS)
			} else if updated := updateMessageStatusIfNewer(ids, status, receiptTS); len(updated) > 0 {
				updatedByStatus[status] = updated
			}
			for newStatus, updated := range updatedByStatus {
				broadcastEvent("message_status", map[string]interface{}{
					"jid":         resolvedChatJID.String(),
					"message_ids": updated,
					"status":      newStatus,
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
	case *events.AppState:
		go func() {
			if v.SyncActionValue == nil {
				return
			}
			if cls := v.GetChatLockSettings(); cls != nil {
				if sc := cls.GetSecretCode(); sc != nil {
					if b, err := proto.Marshal(sc); err == nil {
						saveChatLockSecretCode(b)
					}
				}
			}
			if lca := v.GetLockChatAction(); lca != nil {
				if len(v.Index) > 1 {
					jid := appStateChatJID(v.Index[1])
					isLockedInt := 0
					if lca.GetLocked() {
						isLockedInt = 1
					}
					db.Exec("UPDATE chats SET is_locked = ? WHERE jid = ?", isLockedInt, jid)
					broadcastChats()
				}
			}
			// Deleting/clearing a chat from the phone or another linked
			// device only reaches us as this app-state sync - unlike
			// mute/archive/pin, which whatsmeow keeps in its own
			// ChatSettings store that broadcastChats already reads fresh
			// every time, whantv's messages/chats cache is its own and
			// needs to be told about these explicitly or the chat never
			// goes away locally (handleDeleteChat/handleClearChat cover the
			// other direction: it happening here first).
			if v.GetDeleteChatAction() != nil && len(v.Index) > 1 {
				jid := appStateChatJID(v.Index[1])
				purgeMessages("jid = ?", jid)
				db.Exec("DELETE FROM chats WHERE jid = ?", jid)
				broadcastChats()
			}
			if v.GetClearChatAction() != nil && len(v.Index) > 1 {
				jid := appStateChatJID(v.Index[1])
				purgeMessages("jid = ?", jid)
				broadcastChats()
				broadcastEvent("chat_cleared", map[string]interface{}{"jid": jid})
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
	case *events.CallLogSync:
		go handleCallLogSync(v)
	case *events.Archive:
		go func() {
			if v.Action == nil {
				return
			}
			archivedInt := 0
			if v.Action.GetArchived() {
				archivedInt = 1
			}
			db.Exec("UPDATE chats SET archived = ? WHERE jid = ?", archivedInt, preferPN(v.JID).String())
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
			db.Exec("UPDATE chats SET muted = ? WHERE jid = ?", mutedInt, preferPN(v.JID).String())
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
			db.Exec("UPDATE chats SET pinned = ? WHERE jid = ?", pinnedInt, preferPN(v.JID).String())
			broadcastChats()
		}()
	case *events.LockChat:
		// Previously only whantv's own handleSetChatLocked wrote is_locked -
		// a lock/unlock from the phone or another linked device never
		// reached here at all (the fork didn't dispatch anything for it
		// until now), so this chat's lock state silently drifted from what
		// the phone actually had.
		go func() {
			if v.Action == nil {
				return
			}
			lockedInt := 0
			if v.Action.GetLocked() {
				lockedInt = 1
			}
			db.Exec("UPDATE chats SET is_locked = ? WHERE jid = ?", lockedInt, preferPN(v.JID).String())
			broadcastChats()
		}()
	case *events.MarkChatAsRead:
		// Previously only handleSetChatRead's own outbound action updated
		// unread_count - marking a chat read/unread from the phone or
		// another linked device never reached here, so this chat's read
		// state could silently drift from what the phone actually had.
		go func() {
			if v.Action == nil {
				return
			}
			jidStr := preferPN(v.JID).String()
			if v.Action.GetRead() {
				db.Exec("UPDATE chats SET unread_count = 0 WHERE jid = ?", jidStr)
			} else {
				// "Mark as unread" only needs the chat to show as unread - it
				// must not replace a real count history sync already gave it
				// (a full app-state sync replays this for every such chat).
				db.Exec("UPDATE chats SET unread_count = MAX(unread_count, 1), marked_unread = 1 WHERE jid = ?", jidStr)
			}
			broadcastChats()
		}()
	case *events.Star:
		go func() {
			if v.Action == nil {
				return
			}
			starredInt := 0
			if v.Action.GetStarred() {
				starredInt = 1
			}
			db.Exec("UPDATE messages SET starred = ? WHERE id = ?", starredInt, v.MessageID)
			broadcastEvent("message_starred", map[string]interface{}{
				"jid": preferPN(v.ChatJID).String(), "message_id": v.MessageID, "starred": v.Action.GetStarred(),
			})
		}()
	case *events.ClearChat:
		// Previously nothing here at all - clearing a chat from the phone
		// or another linked device left this app's copy of the chat's
		// messages in place indefinitely.
		go func() {
			jidStr := preferPN(v.JID).String()
			purgeMessages("jid = ?", jidStr)
			broadcastChats()
			broadcastEvent("chat_cleared", map[string]interface{}{"jid": jidStr})
		}()
	case *events.DeleteChat:
		// Previously nothing here at all - handleDeleteChat only ever
		// deleted a chat locally and never told WhatsApp (see its own doc
		// comment), so this case could never have fired from this app's own
		// actions; it only ever meant the phone or another linked device
		// deleted the chat.
		go func() {
			jidStr := preferPN(v.JID).String()
			purgeMessages("jid = ?", jidStr)
			db.Exec("DELETE FROM chats WHERE jid = ?", jidStr)
			broadcastChats()
		}()
	case *events.Picture:
		go func() {
			jidStr := v.JID.String()
			if v.Remove {
				updateContactAvatarCache(jidStr, "", "")
			}
			// A group's icon change is also a line in the group, as in the
			// official apps.
			if v.JID.Server == types.GroupServer && !v.Timestamp.IsZero() {
				insertGroupSystemMessage(jidStr, fmt.Sprintf("system_%s_%d_picture", v.JID.User, v.Timestamp.UnixNano()),
					groupPictureChangeText(v.Author, v.Remove), v.Timestamp)
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
	case *events.DisableLinkPreviews:
		go noteDisableLinkPreviews(v)
	case *events.FavoriteSticker:
		go noteFavoriteSticker(v)
	case *events.UserStatusMute:
		go noteStatusMute(v)
	case *events.StatusPrivacy:
		go noteStatusPrivacy(v)
	case *events.LabelEdit:
		go noteLabelEdit(v)
	case *events.LabelAssociationChat:
		go noteLabelChat(v)
	case *events.Favorites:
		go noteFavorites(v)
	case *events.GroupLimitSharingUpdate:
		go func() {
			if setChatLimitSharing(v.JID.String(), v.Enabled(), 0) {
				by := types.EmptyJID
				for _, s := range []string{v.UpdatedBy.PN, v.UpdatedBy.ID} {
					if parsed, err := types.ParseJID(s); err == nil && !parsed.IsEmpty() {
						by = parsed
						break
					}
				}
				ts := time.Now()
				if secs, err := v.UpdateTime.Int64(); err == nil && secs > 0 {
					ts = time.Unix(secs, 0)
				}
				noteChatPrivacyChange(v.JID.String(), by, v.Enabled(), ts)
			}
		}()
	case *events.GroupInfo:
		invalidateJoinedGroups()
		if _, deleting := pendingGroupDeletes.Load(v.JID.String()); deleting {
			// This group is mid-"delete chat and exit group" - most likely
			// this very event is the "You left" notification LeaveGroup just
			// triggered. Applying it (as usual, an unconditional upsert)
			// would resurrect the chat row the delete is about to remove.
			return
		}
		go func() {
			if v.Name != nil {
				db.Exec(`INSERT INTO chats (jid, name, timestamp) VALUES (?, ?, 0) ON CONFLICT(jid) DO UPDATE SET name=excluded.name`, v.JID.String(), v.Name.Name)
			}
			if v.Ephemeral != nil {
				seconds := uint32(0)
				if v.Ephemeral.IsEphemeral {
					seconds = v.Ephemeral.DisappearingTimer
				}
				setChatDisappearing(v.JID.String(), seconds, v.Timestamp.Unix())
			}

			// Who may send here changed: the chat list carries it to the composer.
			sendRightsChanged := false
			if v.Announce != nil {
				db.Exec("UPDATE chats SET admins_only = ? WHERE jid = ?", boolToInt(v.Announce.IsAnnounce), v.JID.String())
				sendRightsChanged = true
			}
			isAdminChanged := false
			for _, j := range v.Promote {
				if isParticipantMe(types.GroupParticipant{JID: j}) {
					db.Exec("UPDATE chats SET is_admin = 1 WHERE jid = ?", v.JID.String())
					isAdminChanged = true
					break
				}
			}
			for _, j := range v.Demote {
				if isParticipantMe(types.GroupParticipant{JID: j}) {
					db.Exec("UPDATE chats SET is_admin = 0 WHERE jid = ?", v.JID.String())
					isAdminChanged = true
					break
				}
			}

			for _, j := range v.Join {
				if isParticipantMe(types.GroupParticipant{JID: j}) {
					markGroupJoined(v.JID.String(), true)
					sendRightsChanged = true
					// Being added to a group (or let in after asking) comes as
					// this member change, not as JoinedGroup, and carries no
					// name: ask the server, as WhatsApp Web does.
					requestGroupMetadata(v.JID)
				}
			}
			for _, j := range v.Leave {
				if isParticipantMe(types.GroupParticipant{JID: j}) {
					markGroupJoined(v.JID.String(), false)
					sendRightsChanged = true
				}
			}
			if v.Link != nil {
				applyCommunityLinkChange(v, v.Link, true)
			}
			if v.Unlink != nil {
				applyCommunityLinkChange(v, v.Unlink, false)
			}

			// Keep the member count behind group ticks current between
			// joined-groups refreshes.
			if delta := len(v.Join) - len(v.Leave); delta != 0 {
				db.Exec(`UPDATE chats SET participant_count = MAX(0, participant_count + ?) WHERE jid = ? AND participant_count > 0`, delta, v.JID.String())
			}

			for i, text := range groupChangeMessages(v) {
				insertGroupSystemMessage(v.JID.String(), fmt.Sprintf("system_%s_%d_%d", v.JID.User, v.Timestamp.UnixNano(), i), text, v.Timestamp)
			}

			if isAdminChanged || sendRightsChanged {
				broadcastChats()
			}
		}()
		broadcastEvent("group_updated", map[string]interface{}{
			"jid": v.JID.String(),
		})
	case *events.NewsletterJoin, *events.NewsletterLeave, *events.NewsletterMuteChange, *events.NewsletterLiveUpdate:
		handleChannelEvent(v)
	case *events.GroupsDirty:
		// whatsmeow has dropped its cached info for these groups; refresh
		// ours (names, admin status, community structure) the same way.
		log.Printf("Server marked %d groups as out of date; refreshing", len(v.Groups))
		go func() {
			invalidateJoinedGroups()
			populateChatsFromStore()
			broadcastChats()
		}()
	case *events.DirtyState:
		switch v.Type {
		case whatsmeow.DirtyTypeGroups:
			log.Printf("Server marked the group list as out of date; refreshing")
			go func() {
				invalidateJoinedGroups()
				populateChatsFromStore()
				broadcastChats()
			}()
		case whatsmeow.DirtyTypeAccountSync:
			// Account data changed while we were away; the blocklist is the
			// part of it kept here.
			go refreshBlocklist()
		}
	case *events.JoinedGroup:
		invalidateJoinedGroups()
		go func() {
			if v.GroupName.Name != "" {
				db.Exec(`INSERT INTO chats (jid, name, timestamp) VALUES (?, ?, 0) ON CONFLICT(jid) DO UPDATE SET name=excluded.name`, v.JID.String(), v.GroupName.Name)
			} else {
				requestGroupMetadata(v.JID)
			}
			recordGroupCommunity(&v.GroupInfo)
			markGroupJoined(v.JID.ToNonAD().String(), true)
			actor := ""
			if v.Sender != nil && !isParticipantMe(types.GroupParticipant{JID: *v.Sender}) {
				actor = nameToken(*v.Sender)
			}
			msg := ""
			if v.Type == "new" {
				if actor != "" {
					msg = fmt.Sprintf("%s created the group", actor)
				} else {
					msg = "You created the group"
				}
			} else if v.Reason == "invite" {
				msg = "You joined using this group's invite link"
			} else {
				if actor != "" {
					msg = fmt.Sprintf("%s added you", actor)
				} else {
					msg = "You joined the group"
				}
			}
			insertGroupSystemMessage(v.JID.String(), fmt.Sprintf("system_%s_%d", v.JID.User, time.Now().UnixNano()), msg, time.Now())

			invalidateJoinedGroups()
			populateChatsFromStore()
			broadcastChats()
		}()
		broadcastEvent("group_updated", map[string]interface{}{
			"jid": v.JID.String(),
		})
	case *events.Blocklist:
		// Whatever changed, the server's list says who it is by number too.
		go refreshBlocklist()
	case *events.ABPropsRefreshed:
		go refreshServerFlags()
	}
}

// joinedNamesList formats a list of JIDs as "A", "A and B", or "A, B and C",
// matching how the official app lists multiple participants in one
// group-change line. Each person is a name token (see nameToken), named when
// the line is shown.
func joinedNamesList(jids []types.JID) string {
	names := make([]string, 0, len(jids))
	for _, j := range jids {
		names = append(names, nameToken(j))
	}
	switch len(names) {
	case 0:
		return ""
	case 1:
		return names[0]
	default:
		return strings.Join(names[:len(names)-1], ", ") + " and " + names[len(names)-1]
	}
}

// groupChangeMessages turns a GroupInfo event's participant/admin changes
// into the human-readable lines the official WhatsApp app shows inline in the
// chat (e.g. "Aryan added Rishit", "Rishit left"). Returns one line per kind
// of change present in the event.
func groupChangeMessages(v *events.GroupInfo) []string {
	var lines []string
	actor := ""
	if v.Sender != nil {
		actor = nameToken(*v.Sender)
	}
	// "Someone" never reads out a raw JID: settings changes can come with no
	// sender (the server applying them, or an old notification).
	who := actor
	if who == "" {
		who = "Someone"
	}

	// Settings first: a subject change is index 0, the same id history sync
	// gives its GROUP_CHANGE_SUBJECT stub, so a resync doesn't repeat it.
	if v.Name != nil && v.Name.Name != "" {
		lines = append(lines, fmt.Sprintf("%s changed the group name to \"%s\"", who, v.Name.Name))
	}
	if v.Topic != nil {
		if v.Topic.TopicDeleted || v.Topic.Topic == "" {
			lines = append(lines, fmt.Sprintf("%s deleted the group description", who))
		} else {
			lines = append(lines, fmt.Sprintf("%s changed the group description", who))
		}
	}
	if v.Announce != nil {
		if v.Announce.IsAnnounce {
			lines = append(lines, fmt.Sprintf("%s changed this group's settings to allow only admins to send messages to this group", who))
		} else {
			lines = append(lines, fmt.Sprintf("%s changed this group's settings to allow all members to send messages to this group", who))
		}
	}
	if v.Locked != nil {
		if v.Locked.IsLocked {
			lines = append(lines, fmt.Sprintf("%s changed this group's settings to allow only admins to edit this group's info", who))
		} else {
			lines = append(lines, fmt.Sprintf("%s changed this group's settings to allow all members to edit this group's info", who))
		}
	}
	if v.Ephemeral != nil {
		lines = append(lines, disappearingChangeText(who, v.Ephemeral.IsEphemeral, v.Ephemeral.DisappearingTimer))
	}
	if v.MembershipApprovalMode != nil {
		if v.MembershipApprovalMode.IsJoinApprovalRequired {
			lines = append(lines, fmt.Sprintf("%s turned on admin approval to join this group", who))
		} else {
			lines = append(lines, fmt.Sprintf("%s turned off admin approval to join this group", who))
		}
	}
	if v.MemberAddMode != nil {
		switch *v.MemberAddMode {
		case types.GroupMemberAddModeAdmin:
			lines = append(lines, fmt.Sprintf("%s changed this group's settings to allow only admins to add others to this group", who))
		case types.GroupMemberAddModeAllMember:
			lines = append(lines, fmt.Sprintf("%s changed this group's settings to allow all members to add others to this group", who))
		}
	}
	if v.Delete != nil && v.Delete.Deleted {
		lines = append(lines, fmt.Sprintf("%s deleted this group", who))
	}
	if v.NewInviteLink != nil && actor != "" {
		lines = append(lines, fmt.Sprintf("%s reset this group's invite link", actor))
	}

	if len(v.Join) > 0 {
		names := joinedNamesList(v.Join)
		switch {
		case actor != "" && actor != names:
			lines = append(lines, fmt.Sprintf("%s added %s", actor, names))
		case v.JoinReason == "invite":
			lines = append(lines, fmt.Sprintf("%s joined using this group's invite link", names))
		default:
			lines = append(lines, fmt.Sprintf("%s joined", names))
		}
	}

	if len(v.Leave) > 0 {
		names := joinedNamesList(v.Leave)
		if actor != "" && actor != names {
			lines = append(lines, fmt.Sprintf("%s removed %s", actor, names))
		} else {
			lines = append(lines, fmt.Sprintf("%s left", names))
		}
	}

	if len(v.Promote) > 0 {
		names := joinedNamesList(v.Promote)
		if actor != "" {
			lines = append(lines, fmt.Sprintf("%s made %s a group admin", actor, names))
		} else {
			lines = append(lines, fmt.Sprintf("%s is now a group admin", names))
		}
	}

	// The official apps name the person by their new number here.
	for _, j := range v.Modify {
		lines = append(lines, fmt.Sprintf("%s changed their phone number", nameToken(j)))
	}

	if len(v.Demote) > 0 {
		names := joinedNamesList(v.Demote)
		if actor != "" {
			lines = append(lines, fmt.Sprintf("%s removed %s as group admin", actor, names))
		} else {
			lines = append(lines, fmt.Sprintf("%s is no longer a group admin", names))
		}
	}

	return lines
}

// historySyncStallThreshold is how long one chunk may take to write before
// the backend records what every goroutine is doing - a chunk normally takes
// well under a second per few thousand messages, so anything past this is
// almost certainly stuck behind a lock.
const historySyncStallThreshold = 8 * time.Second

var lastStallDump atomic.Int64

// dumpGoroutinesForStall writes every goroutine's stack to
// logs/stall-<time>.txt (at most once a minute), so a lock stall can be traced
// to exactly who holds what and who is waiting on it.
func dumpGoroutinesForStall(reason string) {
	now := time.Now()
	if last := lastStallDump.Load(); last != 0 && now.Sub(time.Unix(0, last)) < time.Minute {
		return
	}
	lastStallDump.Store(now.UnixNano())
	path := filepath.Join(appDataDir(), "logs", "stall-"+now.Format("20060102-150405")+".txt")
	f, err := os.Create(path)
	if err != nil {
		log.Println("Failed to write stall dump:", err)
		return
	}
	defer f.Close()
	fmt.Fprintf(f, "%s\n%s\n\n", now.Format(time.RFC3339Nano), reason)
	pprof.Lookup("goroutine").WriteTo(f, 2)
	log.Printf("Stall detected (%s); goroutine dump written to %s", reason, path)
}

var (
	historySyncRefreshMu    sync.Mutex
	historySyncRefreshTimer *time.Timer
)

// historySyncRefreshDelay is how long history sync has to stay quiet before
// the full chat-list refresh runs.
const historySyncRefreshDelay = 10 * time.Second

// scheduleHistorySyncRefresh (re)arms the all-chats snippet pass and the
// contact/group re-sync to run once no chunk has finished for
// historySyncRefreshDelay and none are waiting to be written.
func scheduleHistorySyncRefresh() {
	historySyncRefreshMu.Lock()
	defer historySyncRefreshMu.Unlock()
	if historySyncRefreshTimer != nil {
		historySyncRefreshTimer.Stop()
	}
	historySyncRefreshTimer = time.AfterFunc(historySyncRefreshDelay, func() {
		if historySyncPending.Load() > 0 {
			scheduleHistorySyncRefresh()
			return
		}
		start := time.Now()
		UpdateAllChatSnippets()
		populateChatsFromStore()
		log.Printf("History sync refresh (snippets, contacts, groups) took %s", time.Since(start).Round(time.Millisecond))
	})
}

var (
	historySyncCheckpointMu  sync.Mutex
	historySyncCheckpointGen int
	// historySyncPending counts HistorySync chunks received but not yet
	// written - each one waits its turn on historySyncMutex.
	historySyncPending atomic.Int32
)

// historySyncCheckpointDelay is how long after the final history-sync chunk
// the WAL is folded back into the main database.
const historySyncCheckpointDelay = 5 * time.Second

// scheduleHistorySyncCheckpoint folds the WAL back into session.db once a
// full history sync is done. It leaves tens of MB in -wal, and SQLite's
// automatic checkpoint only moves that across in small passive steps
// piggybacked on later writes, so session.db kept creeping up in bursts with
// multi-second stalls long after the sync itself was done. One TRUNCATE
// checkpoint does it in a single pass and shrinks -wal back to zero.
//
// Only called for the final chunk: an earlier version ran it after any 5s
// lull, but the phone regularly pauses that long between chunks, so it
// checkpointed mid-sync, held up the next chunk while it ran, and made
// SQLite regrow a truncated -wal from scratch - a full sync got slower.
func scheduleHistorySyncCheckpoint() {
	historySyncCheckpointMu.Lock()
	historySyncCheckpointGen++
	gen := historySyncCheckpointGen
	historySyncCheckpointMu.Unlock()
	runHistorySyncCheckpointAfter(gen, 0)
}

func runHistorySyncCheckpointAfter(gen, attempt int) {
	time.AfterFunc(historySyncCheckpointDelay, func() {
		historySyncCheckpointMu.Lock()
		stale := gen != historySyncCheckpointGen
		historySyncCheckpointMu.Unlock()
		if stale {
			return
		}
		// The final chunk isn't necessarily the last one written - chunks
		// wait on historySyncMutex in no particular order. Wait until none
		// are left rather than checkpointing between them.
		if historySyncPending.Load() > 0 {
			runHistorySyncCheckpointAfter(gen, attempt)
			return
		}
		historySyncMutex.Lock()
		defer historySyncMutex.Unlock()
		start := time.Now()
		var busy, walFrames, checkpointed int
		if err := db.QueryRow("PRAGMA wal_checkpoint(TRUNCATE)").Scan(&busy, &walFrames, &checkpointed); err != nil {
			log.Println("History sync WAL checkpoint failed:", err)
			return
		}
		log.Printf("History sync WAL checkpoint: busy=%d took=%s", busy, time.Since(start).Round(time.Millisecond))
		if busy != 0 && attempt < 3 {
			// A reader held an old snapshot past the busy timeout; the
			// frames it pinned are still in -wal. Try again later - and if
			// it keeps failing, SQLite's own passive checkpoints still get
			// there eventually.
			runHistorySyncCheckpointAfter(gen, attempt+1)
		}
	})
}

// appStateChatJID turns the chat JID in an app-state mutation index into the
// phone JID the chats table is keyed by - newer accounts' indexes can carry
// @lid JIDs.
func appStateChatJID(raw string) string {
	j, err := types.ParseJID(raw)
	if err != nil {
		return raw
	}
	return preferPN(j).String()
}

// preferPN swaps a @lid for the phone JID it maps to, when known - the
// messages table and the frontend's contact lookups are keyed by phone JID.
func preferPN(jid types.JID) types.JID {
	jid = jid.ToNonAD()
	if jid.Server == types.HiddenUserServer && wa() != nil {
		if pn, err := wa().Store.LIDs.GetPNForLID(context.Background(), jid); err == nil && !pn.IsEmpty() {
			return pn.ToNonAD()
		}
	}
	return jid
}

// historyStubActor is whoever caused a history-sync stub entry (the person
// who deleted a message, added someone, ...), as a phone JID when known.
func historyStubActor(webMsg *waWeb.WebMessageInfo) types.JID {
	raw := webMsg.GetParticipant()
	if raw == "" {
		raw = webMsg.GetKey().GetParticipant()
	}
	if raw == "" {
		return types.EmptyJID
	}
	jid, err := types.ParseJID(raw)
	if err != nil {
		return types.EmptyJID
	}
	return preferPN(jid)
}

// historyStubJIDs parses a stub's parameters as participant JIDs. Newer
// WhatsApp versions sometimes send a small JSON object per participant
// instead of a bare JID, so both are accepted; anything else is skipped.
func historyStubJIDs(params []string) []types.JID {
	var jids []types.JID
	for _, p := range params {
		p = strings.TrimSpace(p)
		if strings.HasPrefix(p, "{") {
			var obj map[string]interface{}
			if json.Unmarshal([]byte(p), &obj) != nil {
				continue
			}
			for _, key := range []string{"pn", "phone_number", "jid", "lid", "id"} {
				if s, ok := obj[key].(string); ok && s != "" {
					p = s
					break
				}
			}
		}
		if jid, err := types.ParseJID(p); err == nil && jid.User != "" {
			jids = append(jids, preferPN(jid))
		}
	}
	return jids
}

// historyStubText describes a history-sync group-event stub with the same
// wording groupChangeMessages uses for the live event, or "" for stub types
// the live path doesn't show either.
func historyStubText(webMsg *waWeb.WebMessageInfo) string {
	actor := ""
	if a := historyStubActor(webMsg); !a.IsEmpty() {
		actor = nameToken(a)
	}
	params := webMsg.GetMessageStubParameters()
	names := joinedNamesList(historyStubJIDs(params))
	switch webMsg.GetMessageStubType() {
	case waWeb.WebMessageInfo_GROUP_PARTICIPANT_ADD,
		waWeb.WebMessageInfo_GROUP_PARTICIPANT_ADD_REQUEST_JOIN,
		waWeb.WebMessageInfo_GROUP_PARTICIPANT_ACCEPT,
		waWeb.WebMessageInfo_GROUP_PARTICIPANT_LINKED_GROUP_JOIN:
		switch {
		case names == "":
			return ""
		case actor != "" && actor != names:
			return fmt.Sprintf("%s added %s", actor, names)
		default:
			return fmt.Sprintf("%s joined", names)
		}
	case waWeb.WebMessageInfo_GROUP_PARTICIPANT_INVITE:
		if names == "" {
			names = actor
		}
		if names == "" {
			return ""
		}
		return fmt.Sprintf("%s joined using this group's invite link", names)
	case waWeb.WebMessageInfo_GROUP_PARTICIPANT_REMOVE:
		switch {
		case names == "":
			return ""
		case actor != "" && actor != names:
			return fmt.Sprintf("%s removed %s", actor, names)
		default:
			return fmt.Sprintf("%s left", names)
		}
	case waWeb.WebMessageInfo_GROUP_PARTICIPANT_LEAVE:
		if names == "" {
			names = actor
		}
		if names == "" {
			return ""
		}
		return fmt.Sprintf("%s left", names)
	case waWeb.WebMessageInfo_GROUP_PARTICIPANT_PROMOTE:
		switch {
		case names == "":
			return ""
		case actor != "":
			return fmt.Sprintf("%s made %s a group admin", actor, names)
		default:
			return fmt.Sprintf("%s is now a group admin", names)
		}
	case waWeb.WebMessageInfo_GROUP_PARTICIPANT_DEMOTE:
		switch {
		case names == "":
			return ""
		case actor != "":
			return fmt.Sprintf("%s removed %s as group admin", actor, names)
		default:
			return fmt.Sprintf("%s is no longer a group admin", names)
		}
	case waWeb.WebMessageInfo_GROUP_CHANGE_SUBJECT:
		if len(params) == 0 || params[0] == "" {
			return ""
		}
		if actor != "" {
			return fmt.Sprintf("%s changed the group name to \"%s\"", actor, params[0])
		}
		return fmt.Sprintf("Group name changed to \"%s\"", params[0])
	case waWeb.WebMessageInfo_GROUP_CREATE:
		if actor != "" {
			return fmt.Sprintf("%s created this group", actor)
		}
		return "Group created"
	}
	return ""
}

// insertGroupSystemMessage stores a synthesized "X added Y"-style line as a
// message row (media_type "system") so it persists in history and appears in
// the chat the same way a real message does, then broadcasts it live.
func insertGroupSystemMessage(jid, id, text string, ts time.Time) {
	if text == "" {
		return
	}
	if isIgnoredChatJID(jid) {
		return
	}
	tsUnix := ts.Unix()
	err := saveMessage(StoredMessage{
		ID: id, JID: jid, Text: text, Timestamp: tsUnix, Status: "sent", MediaType: "system",
	})
	if err != nil {
		fmt.Println("Error inserting group system message:", err)
		return
	}
	db.Exec(`INSERT INTO chats (jid, timestamp, unread_count, preview_snippet, preview_sender, preview_from_me) VALUES (?, ?, 0, ?, '', 0)
		ON CONFLICT(jid) DO UPDATE SET
			timestamp=MAX(timestamp, excluded.timestamp),
			preview_snippet=excluded.preview_snippet,
			preview_sender='',
			preview_from_me=0,
			preview_is_reaction=0,
			preview_reaction_emoji=''`,
		jid, tsUnix, text)
	var chatName string
	db.QueryRow(`SELECT name FROM chats WHERE jid = ?`, jid).Scan(&chatName)

	broadcastChats()
	broadcastEvent("message", map[string]interface{}{
		"jid":       jid,
		"from":      "",
		"id":        id,
		"text":      renderNames(text, nil),
		"from_me":   false,
		"timestamp": tsUnix,
		"has_media": false,
		"mediaType": "system",
		"chatName":  chatName,
	})
}

var (
	populateChatsMu      sync.Mutex
	populateChatsRunning bool
	populateChatsPending bool
)

// populateChatsFromStore is called from many event handlers that can fire in
// a burst (reconnect, history sync, bulk group joins). Without coalescing,
// each call redundantly re-queries the whole joined-groups list over the
// wire and races the others' chats-table writes against each other.
// Concurrent calls collapse into one run followed by at most one more.
func populateChatsFromStore() {
	populateChatsMu.Lock()
	if populateChatsRunning {
		populateChatsPending = true
		populateChatsMu.Unlock()
		return
	}
	populateChatsRunning = true
	populateChatsMu.Unlock()

	for {
		populateChatsFromStoreOnce()
		broadcastChats()

		populateChatsMu.Lock()
		if populateChatsPending {
			populateChatsPending = false
			populateChatsMu.Unlock()
			continue
		}
		populateChatsRunning = false
		populateChatsMu.Unlock()
		break
	}
}

func populateChatsFromStoreOnce() {
	if wa() == nil || wa().Store == nil || wa().Store.Contacts == nil {
		return
	}

	allContacts, err := wa().Store.Contacts.GetAllContacts(context.Background())
	if err != nil {
		fmt.Println("Error getting contacts from store:", err)
		return
	}

	// The same person is often stored twice - once under their phone JID
	// (usually carrying the address-book name) and once under their @lid
	// (usually nameless) - and both resolve to the same phone JID below.
	// Writing each store entry straight to the cache let whichever came
	// last in Go's random map order win, so a nameless twin's "+<phone>"
	// fallback regularly overwrote the real name: the contact picker showed
	// only a number for people whose name was known all along, and which
	// people were affected changed from sync to sync. Merge the twins first
	// and keep the best name instead.
	type mergedContact struct {
		name       string
		nameRank   int // 3 = address-book name, 2 = push name, 1 = business name, 0 = "+<phone>"/none
		phone      string
		isBusiness bool
	}
	merged := make(map[string]*mergedContact, len(allContacts))
	var order []string
	// Once a @lid identity resolves to its phone-number JID, its contact gets
	// merged into (and written under) the PN's jidStr below - but a *previous*
	// run, before the mapping was known, may already have written that same
	// @lid as its own separate contacts_cache row (usually nameless/push-name
	// only). Nothing ever deleted that stale row, so the picker went on
	// showing both the resolved "Name +91 xxx" entry and the leftover
	// "PushName +1 <lid-as-fake-phone>" twin forever. Track every @lid that
	// resolves this run and delete its stale row once the merge is written.
	var resolvedLIDJIDs []string
	for jid, contact := range allContacts {
		targetJID := jid.ToNonAD()
		if targetJID.Server == "lid" {
			if pn, pnErr := wa().Store.LIDs.GetPNForLID(context.Background(), targetJID); pnErr == nil && !pn.IsEmpty() {
				resolvedLIDJIDs = append(resolvedLIDJIDs, targetJID.String())
				targetJID = pn.ToNonAD()
			}
		}

		jidStr := targetJID.String()
		if isIgnoredChatJID(jidStr) {
			continue
		}

		name, rank := contact.FullName, 3
		if name == "" {
			name, rank = contact.PushName, 2
		}
		if name == "" {
			name, rank = contact.BusinessName, 1
		}

		if name == "" {
			rank = 0
			if targetJID.Server != "g.us" && targetJID.Server != "lid" {
				name = "+" + targetJID.User
			}
		}

		phone := ""
		if targetJID.Server != "g.us" && targetJID.Server != "lid" {
			phone = targetJID.User
		}

		entry, exists := merged[jidStr]
		if !exists {
			entry = &mergedContact{nameRank: -1}
			merged[jidStr] = entry
			order = append(order, jidStr)
		}
		if rank > entry.nameRank {
			entry.name, entry.nameRank = name, rank
		}
		if entry.phone == "" {
			entry.phone = phone
		}
		entry.isBusiness = entry.isBusiness || contact.BusinessName != ""
	}

	var contactWrites []func(tx sqlExecer)
	for _, jidStr := range order {
		entry := merged[jidStr]
		// An unresolved "lid" identity (GetPNForLID above came up empty - the
		// mapping to the person's phone number isn't known yet) is sometimes
		// a duplicate of that same person's phone-number contact and
		// sometimes the only entry that exists for them at all (their PN
		// identity may never get synced as a separate contact). Caching it
		// is still correct either way; the "Name +1 10000000000004"-looking
		// duplicate row was the picker formatting the lid's own large
		// numeric ID as if it were a phone number - fixed at display time in
		// contact_picker_label (main.rs), not by dropping the contact here.
		contactWrites = append(contactWrites, func(tx sqlExecer) {
			if entry.name != "" || entry.phone != "" {
				upsertContactCacheRowWith(tx, jidStr, entry.name, entry.phone, entry.isBusiness)
			}

			// Only a real name may replace a chat's name - the "+<phone>"
			// fallback must never clobber a name the chat already has.
			if entry.nameRank > 0 {
				tx.Exec(`UPDATE chats SET name = ? WHERE jid = ?`, entry.name, jidStr)
			} else if entry.name != "" {
				tx.Exec(`UPDATE chats SET name = ? WHERE jid = ? AND (name IS NULL OR name = '')`, entry.name, jidStr)
			}
		})
	}
	for _, lidJIDStr := range resolvedLIDJIDs {
		lidJIDStr := lidJIDStr
		contactWrites = append(contactWrites, func(tx sqlExecer) {
			tx.Exec(`DELETE FROM contacts_cache WHERE jid = ?`, lidJIDStr)
		})
	}
	// Thousands of contacts, each its own autocommit write, made this the
	// slowest part of every refresh. Everything above that touches
	// whatsmeow's stores is already done, so these can be batched.
	writeBatch(contactWrites)
	// Names may have changed in the store or the cache above.
	invalidateDisplayNames()
	healStaleUnreadCounts()

	groups, err := serverGroupList()
	var joinedGroupsSnapshot map[string]bool
	if err == nil {
		var groupWrites []func(tx sqlExecer)
		joined := make(map[string]bool, len(groups))
		joinedGroupsSnapshot = joined
		for _, g := range groups {
			joined[g.JID.ToNonAD().String()] = true
			groupWrites = append(groupWrites, groupInfoWrite(g))
		}
		writeBatch(groupWrites)
		setJoinedGroups(joined)
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
			// Only the server knows a group's name; a person's can be
			// worked out here. The group list just written named every group
			// we're in, so this is only for when fetching it failed.
			if pj.Server == types.GroupServer {
				if joinedGroupsSnapshot == nil {
					requestGroupMetadata(pj)
				}
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
				pn, err := wa().Store.LIDs.GetPNForLID(context.Background(), target)
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
	// Community parent groups are exempt: they legitimately never carry
	// messages of their own (those go to the community's announcement and
	// member sub-groups), so this sweep would otherwise delete every one of
	// them the moment populateChatsFromStore inserted it, and the Communities
	// tab would always be empty.
	//
	// Groups we are in are exempt too. The phone no longer sends every
	// chat's history when linking, so a group nobody has written in lately
	// arrives with no messages and no timestamp, and this used to delete it:
	// the group couldn't be found in whaNTV at all. Being in the group is
	// reason enough to list it (at the bottom, having no activity to sort
	// by). The same can't be done for people: the phone also sends an empty
	// entry for contacts who were never chatted with, and those must go.
	const emptyChat = `timestamp = 0 AND COALESCE(is_community, 0) = 0 AND COALESCE(is_channel, 0) = 0 AND jid NOT IN (SELECT DISTINCT jid FROM messages) AND jid NOT IN (SELECT DISTINCT jid FROM chats WHERE pinned = 1 OR muted = 1 OR archived = 1)`
	db.Exec("DELETE FROM chats WHERE jid NOT LIKE '%@g.us' AND " + emptyChat)
	// Without the server's list there is no telling which groups we are in,
	// so no group is removed on a refresh where fetching it failed.
	if joinedGroupsSnapshot != nil {
		if emptyGroups, qErr := db.Query("SELECT jid FROM chats WHERE jid LIKE '%@g.us' AND " + emptyChat); qErr == nil {
			var left []string
			for emptyGroups.Next() {
				var j string
				if emptyGroups.Scan(&j) == nil && !joinedGroupsSnapshot[j] {
					left = append(left, j)
				}
			}
			emptyGroups.Close()
			for _, j := range left {
				db.Exec("DELETE FROM chats WHERE jid = ?", j)
			}
		}
	}
	// Channels we follow stay (see syncFollowedChannels); any other channel
	// row is one history sync brought in, and isn't ours to show.
	db.Exec("DELETE FROM chats WHERE jid = 'status@broadcast' OR (jid LIKE '%@newsletter' AND COALESCE(is_channel, 0) = 0) OR jid LIKE '%@broadcast'")
	go syncFollowedChannelsThrottled()
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
	mediaType, mimetype, filename, mediaSize := mediaInfoFromMessage(v.Message)
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
		MediaType: mediaType, MediaMimetype: mimetype, MediaFilename: filename, MediaSize: mediaSize,
	}); err != nil {
		log.Println("Error saving status update:", err)
		return
	}
	go purgeExpiredStatuses()
	broadcastEvent("status_update", map[string]interface{}{
		"id": v.Info.ID, "posterJid": senderJID.String(), "fromMe": v.Info.IsFromMe,
		"text": text, "mediaType": mediaType, "timestamp": ts,
	})
}

var (
	appStateResyncMu    sync.Mutex
	lastAppStateResync  = make(map[appstate.WAPatchName]time.Time)
	appStateResyncCount = make(map[appstate.WAPatchName]int)
	// lastFatalRecoveryRequest tracks the last time we asked our own primary
	// device (the phone) for an unencrypted snapshot of a collection, because
	// that request is a message sent to the phone rather than a plain HTTP-ish
	// IQ - worth a longer, separate backoff so a phone that's briefly offline
	// doesn't get hit repeatedly while the regular resync loop keeps retrying.
	lastFatalRecoveryRequest = make(map[appstate.WAPatchName]time.Time)
)

// requestAppStateFatalRecovery asks our own primary device (the phone, if
// we're a companion) to send an unencrypted snapshot of an app-state
// collection whose server-side copy has itself failed LTHash verification.
// Gated by a 5-minute backoff separate from the regular resync counters,
// since this reaches the phone rather than the server and shouldn't be
// retried on the same tight loop.
func requestAppStateFatalRecovery(name appstate.WAPatchName) {
	appStateResyncMu.Lock()
	if wait := 5*time.Minute - time.Since(lastFatalRecoveryRequest[name]); wait > 0 {
		appStateResyncMu.Unlock()
		log.Printf("Fatal recovery for %s: skipping, still within 5m of the last request (%s left)", name, wait.Round(time.Second))
		return
	}
	lastFatalRecoveryRequest[name] = time.Now()
	appStateResyncMu.Unlock()

	if wa() == nil {
		return
	}
	log.Printf("Requesting fatal recovery snapshot for %s from our primary device.", name)
	req := whatsmeow.BuildAppStateRecoveryRequest(name)
	if _, err := wa().SendPeerMessage(context.Background(), req); err != nil {
		log.Printf("Fatal recovery request for %s failed to send: %v", name, err)
	}
}

func isDatabaseLocked(err error) bool {
	if err == nil {
		return false
	}
	msg := err.Error()
	return strings.Contains(msg, "database is locked") || strings.Contains(msg, "SQLITE_BUSY")
}

// retryAppStateAfterLock re-fetches an app-state collection whose sync failed
// on a database lock, backing off 15s, 30s, ... up to five times.
func retryAppStateAfterLock(name appstate.WAPatchName, attempt int) {
	if attempt >= 5 {
		log.Printf("App state %s: still hitting database locks after %d retries, giving up until next connection", name, attempt)
		return
	}
	delay := time.Duration(15*(attempt+1)) * time.Second
	log.Printf("App state %s hit a database lock; retrying in %s", name, delay)
	time.AfterFunc(delay, func() {
		if wa() == nil {
			return
		}
		err := wa().FetchAppState(context.Background(), name, false, false)
		switch {
		case err == nil:
			log.Printf("App state %s resync succeeded after database lock", name)
			appStateResyncMu.Lock()
			appStateResyncCount[name] = 0
			appStateResyncMu.Unlock()
			// Saved contact names land in the contact store; carry them
			// through to the chat list and contact picker.
			populateChatsFromStore()
			broadcastChats()
		case isDatabaseLocked(err):
			retryAppStateAfterLock(name, attempt+1)
		default:
			log.Printf("App state %s retry after database lock failed: %v", name, err)
		}
	})
}

func handleAppStateSyncError(v *events.AppStateSyncError) {
	appStateResyncMu.Lock()
	count := appStateResyncCount[v.Name]
	if count >= 3 {
		appStateResyncMu.Unlock()
		log.Printf("AppStateSyncError for %s: giving up after %d failed attempts. Will retry on next connection.", v.Name, count)
		return
	}
	lastTime := lastAppStateResync[v.Name]
	if wait := 60*time.Second - time.Since(lastTime); wait > 0 {
		appStateResyncMu.Unlock()
		log.Printf("AppStateSyncError for %s: skipping resync, still within 60s of the last attempt (%s left): %v", v.Name, wait.Round(time.Second), v.Error)
		return
	}
	appStateResyncCount[v.Name] = count + 1
	lastAppStateResync[v.Name] = time.Now()
	appStateResyncMu.Unlock()

	go func() {
		log.Printf("AppStateSyncError for %s (attempt %d/3): %v.", v.Name, count+1, v.Error)
		if wa() == nil {
			return
		}
		if v.Error != nil {
			errMsg := v.Error.Error()
			if strings.Contains(errMsg, "mismatching LTHash") || strings.Contains(errMsg, "failed to verify snapshot") {
				// This used to hand-write a fake version + all-zero hash
				// into whatsmeow_app_state_version and then do a plain
				// (non-full) resync. That never touched
				// whatsmeow_app_state_mutation_macs at all, so whatever
				// local mutation record was actually missing/wrong (the
				// real cause of the hash mismatch) stayed missing/wrong -
				// it just forced the version counter forward, which is why
				// the very next patch touching the same index failed the
				// exact same way again.
				//
				// A real full resync (fullSync=true) resets the version via
				// DeleteAppStateVersion *and* requests an actual snapshot
				// (see Client.fetchAppState: wantSnapshot := fullSync)
				// instead of replaying the patch stream that's missing the
				// record - the snapshot repopulates the current live
				// index->valueMAC mapping from scratch, which is what a
				// later patch actually needs.
				log.Printf("Mismatching LTHash for %s - forcing a full resync (fresh snapshot).", v.Name)
				if resyncErr := wa().FetchAppState(context.Background(), v.Name, true, false); resyncErr != nil {
					log.Printf("Full resync for %s failed: %v", v.Name, resyncErr)
					resyncErrMsg := resyncErr.Error()
					if strings.Contains(resyncErrMsg, "mismatching LTHash") || strings.Contains(resyncErrMsg, "failed to verify snapshot") {
						// A fresh snapshot straight from the server failing the
						// exact same verification means the server's own copy
						// of this collection has diverged, not just our local
						// mirror of it - no amount of re-asking the server can
						// fix that. The one source that's still authoritative
						// is our own primary device's local copy, fetched via
						// a COMPANION_SYNCD_SNAPSHOT_FATAL_RECOVERY peer
						// message; whatsmeow already parses the response in
						// handleAppStateRecovery (wired from message.go), it
						// just never had anything triggering the request.
						requestAppStateFatalRecovery(v.Name)
					}
				} else {
					log.Printf("Full resync for %s succeeded", v.Name)
					appStateResyncMu.Lock()
					appStateResyncCount[v.Name] = 0
					appStateResyncMu.Unlock()
				}
				return
			}
		}
		err := wa().FetchAppState(context.Background(), v.Name, false, false)
		if err != nil {
			if errors.Is(err, whatsmeow.ErrIQRateOverLimit) || strings.Contains(err.Error(), "429") || strings.Contains(err.Error(), "rate-overlimit") {
				log.Printf("Rate limit 429 during resync of %s. Scheduling retry in 90s...", v.Name)
				time.AfterFunc(90*time.Second, func() {
					appStateResyncMu.Lock()
					lastAppStateResync[v.Name] = time.Now()
					appStateResyncMu.Unlock()
					if wa() != nil {
						if retryErr := wa().FetchAppState(context.Background(), v.Name, false, false); retryErr != nil {
							log.Printf("Delayed retry for %s failed: %v", v.Name, retryErr)
						} else {
							log.Printf("Delayed resync for %s succeeded", v.Name)
							appStateResyncMu.Lock()
							appStateResyncCount[v.Name] = 0
							appStateResyncMu.Unlock()
						}
					}
				})
			} else if isDatabaseLocked(err) {
				// Typically a full history sync hammering the database right
				// after pairing. Retrying straight away just loses the same
				// race again; wait for things to calm down instead.
				// critical_unblock_low carries saved contact names, so giving
				// up here leaves every chat showing push names instead.
				retryAppStateAfterLock(v.Name, 0)
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
func isParticipantMe(p types.GroupParticipant) bool {
	if wa() == nil || wa().Store == nil || wa().Store.ID == nil {
		return false
	}
	// Each address is compared with ours of the same kind: a LID whose digits
	// happen to spell our phone number is somebody else.
	myJID := wa().Store.ID.ToNonAD()
	myLID := wa().Store.GetLID().ToNonAD()
	isMe := func(j types.JID) bool {
		switch j.Server {
		case types.DefaultUserServer:
			return j.User == myJID.User
		case types.HiddenUserServer:
			return !myLID.IsEmpty() && j.User == myLID.User
		}
		return false
	}
	if isMe(p.JID) || isMe(p.PhoneNumber) || isMe(p.LID) {
		return true
	}
	if p.JID.Server == types.HiddenUserServer {
		if pn, err := wa().Store.LIDs.GetPNForLID(context.Background(), p.JID); err == nil && !pn.IsEmpty() {
			return isMe(pn)
		}
	}
	return false
}

func isViewerAdmin(info *types.GroupInfo) bool {
	if info == nil {
		return false
	}
	for _, p := range info.Participants {
		if isParticipantMe(p) && (p.IsAdmin || p.IsSuperAdmin) {
			return true
		}
	}
	return false
}
