package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"os"
	"path/filepath"
	"runtime"
	"runtime/debug"
	"strings"
	"sync/atomic"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waAICommonDeprecated"
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

func (acc *Account) resolveLIDChat(v *events.Message) (chatJID, senderJID types.JID) {
	chatJID = v.Info.Chat.ToNonAD()
	senderJID = v.Info.Sender.ToNonAD()

	if senderJID.Server == "lid" {
		if !v.Info.SenderAlt.IsEmpty() {
			senderJID = v.Info.SenderAlt.ToNonAD()
		} else if pn, err := acc.wa().Store.LIDs.GetPNForLID(context.Background(), senderJID); err == nil && !pn.IsEmpty() {
			senderJID = pn.ToNonAD()
		} else if v.Info.IsFromMe && acc.wa().Store.ID != nil {
			senderJID = acc.wa().Store.ID.ToNonAD()
		}
	}

	if chatJID.Server == "lid" {
		if v.Info.IsFromMe {
			if !v.Info.RecipientAlt.IsEmpty() {
				chatJID = v.Info.RecipientAlt.ToNonAD()
			} else if pn, err := acc.wa().Store.LIDs.GetPNForLID(context.Background(), chatJID); err == nil && !pn.IsEmpty() {
				chatJID = pn.ToNonAD()
			}
		} else {
			chatJID = senderJID
		}
	} else if chatJID.Server == types.BroadcastServer && chatJID.User != "status" && !v.Info.IsFromMe {
		chatJID = senderJID
	}

	return
}

func (acc *Account) resolveSourcePN(src types.MessageSource) (chatJID, senderJID types.JID) {
	chatJID = src.Chat.ToNonAD()
	senderJID = src.Sender.ToNonAD()
	rawSender := senderJID
	if senderJID.Server == types.HiddenUserServer {
		if src.SenderAlt.Server == types.DefaultUserServer {
			senderJID = src.SenderAlt.ToNonAD()
		} else {
			senderJID = acc.preferPN(senderJID)
		}
	}
	if chatJID.Server == types.HiddenUserServer {
		switch {
		case chatJID == rawSender:
			chatJID = senderJID
		case src.RecipientAlt.Server == types.DefaultUserServer:
			chatJID = src.RecipientAlt.ToNonAD()
		default:
			chatJID = acc.preferPN(chatJID)
		}
	}
	return
}

// applyReaction stores an incoming reaction and updates the chat-list preview.
// It never creates a standalone chat message - if the reacted-to message isn't
// in the local DB, updateChatReactionPreview silently leaves the preview alone.
func (acc *Account) applyReaction(v *events.Message, reaction *waE2E.ReactionMessage) {
	if reaction.GetKey().GetRemoteJID() == types.StatusBroadcastJID.String() {
		acc.noteStatusReaction(v, reaction)
		return
	}
	targetID := reaction.GetKey().GetID()
	chatJID, senderJID := acc.resolveLIDChat(v)
	jid := chatJID.ToNonAD().String()
	emoji := reaction.GetText()

	reactorJID := senderJID.ToNonAD().String()
	reactorName := acc.resolveDisplayName(senderJID)
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
	acc.upsertReaction(targetID, jid, reactorJID, emoji, ts)
	acc.updateChatReactionPreview(jid, targetID, reactorJID, emoji, v.Info.IsFromMe)
	acc.broadcastChats()

	var reactedText string
	var isDeleted int
	if err := acc.db.QueryRow(`SELECT COALESCE(text, ''), is_deleted FROM messages WHERE id = ?`, targetID).
		Scan(&reactedText, &isDeleted); err != nil || isDeleted == 1 {
		reactedText = ""
	}

	acc.broadcastEvent("message_reaction", map[string]interface{}{
		"jid":          jid,
		"message_id":   targetID,
		"reactor":      reactorJID,
		"reactor_name": reactorName,
		"emoji":        emoji,
		"text":         reactedText,
	})
}

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

// sentMessageMovesChat says whether a message we sent takes its chat to the
// top. WA Web (WAWebMsgDataUtils eventTypeFromMsgType) gives a group invite we
// sent EventType.IGNORE: the chat gets the message and its preview but keeps
// its place in the list.
func sentMessageMovesChat(msg *waE2E.Message) bool {
	return unwrapMessage(msg).GetGroupInviteMessage() == nil
}

// chatTimeAfter is the SQL for a chat row's new timestamp on an upsert:
// raised to the message's time, or left alone when the message doesn't move
// the chat.
func chatTimeAfter(movesChat bool) string {
	if movesChat {
		return "MAX(timestamp, excluded.timestamp)"
	}
	return "timestamp"
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

func (acc *Account) extractText(msg *waE2E.Message) string {
	return acc.resolveMentions(acc.extractTextRaw(msg), getContextInfo(msg))
}

func (acc *Account) resolveMentions(text string, ctx *waE2E.ContextInfo) string {
	if text == "" || ctx == nil {
		return text
	}
	for _, raw := range ctx.GetMentionedJID() {
		jid, err := types.ParseJID(raw)
		if err != nil || jid.User == "" {
			continue
		}
		text = replaceMentionToken(text, "@"+jid.User, "@"+acc.resolveDisplayName(jid))
	}
	return text
}

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

func (acc *Account) extractTextRaw(msg *waE2E.Message) string {
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
		return videoText(msg.GetVideoMessage())
	}
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
	if msg.GetLimitSharingMessage() != nil || msg.GetAcp2SettingMessage() != nil {
		return "[Protocol Message]"
	}
	if notice := msg.GetMessageHistoryNotice(); notice != nil {
		return acc.messageHistoryNoticeText(notice, types.EmptyJID)
	}
	if msg.GetPinInChatMessage() != nil || msg.GetKeepInChatMessage() != nil ||
		msg.GetEncEventResponseMessage() != nil || msg.GetEncCommentMessage() != nil ||
		msg.GetStickerSyncRmrMessage() != nil || msg.GetScheduledCallEditMessage() != nil ||
		msg.GetCallLogMesssage() != nil || msg.GetMessageHistoryBundle() != nil {
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

// richResponseText turns a Meta AI reply into plain text. Current replies
// carry their content as JSON in unifiedResponse (sections of view models
// holding markdown, code, tables and so on); older ones used submessages.
func richResponseText(rich *waE2E.AIRichResponseMessage) string {
	if rich == nil {
		return ""
	}
	var parts []string
	if data := rich.GetUnifiedResponse().GetData(); len(data) > 0 {
		var parsed richresponse.RichResponse
		if err := json.Unmarshal(data, &parsed); err != nil {
			log.Printf("Failed to parse rich response JSON: %v", err)
		}
		for _, section := range parsed.Sections {
			if section.Model == nil {
				continue
			}
			if s := strings.TrimSpace(section.Model.String()); s != "" {
				parts = append(parts, s)
			}
		}
	}
	if len(parts) == 0 {
		parts = richResponseSubmessageText(rich.GetSubmessages())
	}
	return aiMarkdownToPlain(strings.Join(parts, "\n"))
}

func richResponseSubmessageText(subs []*waAICommonDeprecated.AIRichResponseSubMessage) []string {
	var parts []string
	add := func(s string) {
		if s = strings.TrimSpace(s); s != "" {
			parts = append(parts, s)
		}
	}
	for _, sub := range subs {
		switch {
		case sub.GetMessageText() != "":
			add(sub.GetMessageText())
		case sub.GetCodeMetadata() != nil:
			var code strings.Builder
			for _, block := range sub.GetCodeMetadata().GetCodeBlocks() {
				code.WriteString(block.GetCodeContent())
			}
			add(code.String())
		case sub.GetTableMetadata() != nil:
			table := sub.GetTableMetadata()
			add(table.GetTitle())
			for _, row := range table.GetRows() {
				add(strings.Join(row.GetItems(), ", "))
			}
		case sub.GetLatexMetadata() != nil:
			add(sub.GetLatexMetadata().GetText())
		}
	}
	return parts
}

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
	if summary := optionsSummary(msg); summary != "" {
		buttons = append(buttons, summary)
	}
	buttons = append(buttons, optionLinks(msg)...)
	text = strings.Join(blocks, "\n\n")
	if len(buttons) > 0 {
		if text != "" {
			text += "\n\n"
		}
		text += strings.Join(buttons, "\n")
	}
	return media, text, ok
}

func labeledValue(label, value string) string {
	label, value = strings.TrimSpace(label), strings.TrimSpace(value)
	if label == "" || value == "" {
		return label + value
	}
	return label + ": " + value
}

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

type pendingAlbum struct {
	id        string
	remaining int
	expires   time.Time
}

// albumWindow bounds how long an album header waits for its media. The media
// normally follows within a couple of seconds; the window only has to outlast
// a slow upload, and its real job is to stop an album whose media never
// arrived from swallowing unrelated photos sent later.
const albumWindow = 2 * time.Minute

func albumKey(chatJID, senderJID string) string {
	return chatJID + "|" + senderJID
}

func (acc *Account) registerAlbum(chatJID, senderJID, headerID string, album *waE2E.AlbumMessage) bool {
	expected := int(album.GetExpectedImageCount() + album.GetExpectedVideoCount())
	if expected <= 0 || headerID == "" {
		return false
	}
	acc.pendingAlbumsMu.Lock()
	defer acc.pendingAlbumsMu.Unlock()
	acc.pendingAlbums[albumKey(chatJID, senderJID)] = &pendingAlbum{
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
func (acc *Account) claimAlbumID(chatJID, senderJID string) string {
	acc.pendingAlbumsMu.Lock()
	defer acc.pendingAlbumsMu.Unlock()
	key := albumKey(chatJID, senderJID)
	album, ok := acc.pendingAlbums[key]
	if !ok {
		return ""
	}
	if time.Now().After(album.expires) {
		delete(acc.pendingAlbums, key)
		return ""
	}
	album.remaining--
	if album.remaining <= 0 {
		delete(acc.pendingAlbums, key)
	}
	return album.id
}

func (acc *Account) stampAlbumMembers(headerID, chatJID, senderJID string, headerTS int64, expected int) {
	if headerID == "" || expected <= 0 {
		return
	}
	acc.db.Exec(`UPDATE messages SET album_id = ? WHERE id IN (
			SELECT id FROM messages
			WHERE jid = ? AND sender_jid = ? AND media_type IN ('image', 'video')
				AND timestamp >= ? AND timestamp <= ?
				AND (album_id IS NULL OR album_id = '')
			ORDER BY timestamp ASC, id ASC LIMIT ?)`,
		headerID, chatJID, senderJID, headerTS-1, headerTS+int64(albumWindow/time.Second), expected)
}

func mediaCarrier(msg *waE2E.Message) *waE2E.Message {
	msg = unwrapMessage(msg)
	if msg == nil {
		return nil
	}
	if media, _, _ := templateParts(msg); media != nil {
		return media
	}
	if ptv := msg.GetPtvMessage(); ptv != nil {
		return &waE2E.Message{VideoMessage: ptv}
	}
	return msg
}

func (acc *Account) groupSize(jid types.JID) int {
	if jid.Server != types.GroupServer {
		return 0
	}
	acc.groupSizesMu.Lock()
	defer acc.groupSizesMu.Unlock()
	size, known := acc.groupSizes[jid]
	if !known {
		acc.groupSizes[jid] = 0
		go func() {
			if info, err := acc.wa().GetGroupInfo(context.Background(), jid); err == nil {
				acc.groupSizesMu.Lock()
				acc.groupSizes[jid] = len(info.Participants)
				acc.groupSizesMu.Unlock()
			}
		}()
	}
	return size
}

func (acc *Account) addressesMe(msg *waE2E.Message) bool {
	ctx := getContextInfo(msg)
	if ctx == nil || acc.wa() == nil || acc.wa().Store == nil {
		return false
	}
	if ctx.GetNonJIDMentions()&1 != 0 {
		return true
	}
	own, ownLID := acc.wa().Store.GetJID(), acc.wa().Store.GetLID()
	isMe := func(raw string) bool {
		jid, err := types.ParseJID(raw)
		return err == nil && jid.User != "" && (jid.User == own.User || (!ownLID.IsEmpty() && jid.User == ownLID.User))
	}
	for _, raw := range ctx.GetMentionedJID() {
		if isMe(raw) {
			return true
		}
	}
	return ctx.GetStanzaID() != "" && isMe(ctx.GetParticipant())
}

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

func (acc *Account) quotedPreview(msg *waE2E.Message) (quotedID, quotedText, quotedSender string) {
	if ctx := getContextInfo(msg); ctx != nil && ctx.GetStanzaID() != "" {
		quotedID = ctx.GetStanzaID()
		if participant := ctx.GetParticipant(); participant != "" {
			if parsed, err := types.ParseJID(participant); err == nil {
				quotedSender = acc.resolveDisplayName(parsed)
			} else {
				quotedSender = participant
			}
		}
		if qm := ctx.GetQuotedMessage(); qm != nil {
			quotedText = acc.extractText(qm)
		}
	}
	return
}

func (acc *Account) resolveQuotedSenderName(participant string) string {
	if participant == "" || participant == "me" {
		return participant
	}
	if parsed, err := types.ParseJID(participant); err == nil {
		return acc.resolveDisplayName(parsed)
	}
	return participant
}

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

func (acc *Account) resetHistorySyncProgress() {
	acc.historySyncMutex.Lock()
	acc.historySyncChatsDone = 0
	acc.historySyncMsgsDone = 0
	acc.historySyncMutex.Unlock()
	acc.resetRecentSyncProgress()
}

func (acc *Account) eventHandler(evt interface{}) {
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
		go acc.handleMediaRetryEvent(v)
	case *events.HistorySyncStatus:
		acc.handleHistorySyncStatus(v.Notification)
	case *events.HistorySyncChunkRetryResult:
		log.Printf("Phone answered history sync chunk retry for %s chunk %d: code %s (sent %v), can recover %v",
			v.SyncType, v.ChunkOrder, v.Code, v.HasCode, v.CanRecover)
	case *events.HistorySync:
		acc.historySyncPending.Add(1)
		// Run inline, not in a goroutine: whatsmeow sends the chunk's receipt
		// (AckHistorySyncAfterDispatch) once this returns, and the phone never
		// sends a chunk again after its receipt. Saving in the background let
		// the receipt go out first, so closing whatSnap mid-save lost that
		// part of the history for good. This only holds up whatsmeow's
		// history download loop (the next chunk waits), not other events: the
		// fork no longer holds its handler lock while a handler runs.
		func() {
			acc.historySyncMutex.Lock()
			defer acc.historySyncMutex.Unlock()
			defer acc.historySyncPending.Add(-1)
			chunkStart := time.Now()
			stallTimer := time.AfterFunc(historySyncStallThreshold, func() {
				dumpGoroutinesForStall("history sync chunk still writing after " + historySyncStallThreshold.String())
			})
			defer stallTimer.Stop()
			statsAtStart := acc.db.Stats()
			chunkMsgs := 0

			if v.Data == nil {
				return
			}
			if statuses := v.Data.GetStatusV3Messages(); len(statuses) > 0 {
				acc.storeHistoryStatuses(statuses)
			}

			// With StreamHistorySyncConversations on, the chats arrive still
			// encoded and are decoded one at a time below, so a big chunk
			// never sits in memory whole.
			convCount := len(v.Data.GetConversations())
			omittedFrom := -1
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
			// Chats the phone left out of the sync (too many, too big) come
			// as a separate list of plain chat records with no messages, sent
			// in batches. WA Web doesn't use this list at all, so it never
			// adds a chat here: it only fills in chats already listed (see
			// historyChatGate).
			if omitted := v.Data.GetOmittedConversationMetadata(); len(omitted.GetConversations()) > 0 {
				log.Printf("History sync: %d chats the phone left out (snapshot %s, batch %d of %d, complete %v)",
					len(omitted.GetConversations()), omitted.GetSnapshotID(), omitted.GetBatchIndex()+1, omitted.GetBatchCount(), omitted.GetComplete())
				convCount += len(omitted.GetConversations())
				synced := conversations
				conversations = func(yield func(int, *waHistorySync.Conversation) bool) {
					n := 0
					for i, conv := range synced {
						if !yield(i, conv) {
							return
						}
						n = i + 1
					}
					omittedFrom = n
					for i, conv := range omitted.GetConversations() {
						if !yield(n+i, conv) {
							return
						}
					}
				}
			}
			gate := acc.newHistoryChatGate(v.Data.GetSyncType())
			reportProgress := v.Data.GetSyncType() != waHistorySync.HistorySync_ON_DEMAND && convCount > 0
			if reportProgress {
				acc.broadcastEvent("history_sync", map[string]interface{}{
					"status":    "syncing",
					"chatsDone": acc.historySyncChatsDone,
					"msgsDone":  acc.historySyncMsgsDone,
				})
			}

			type syncedAlbum struct {
				id, jid, senderJID string
				timestamp          int64
				expected           int
			}
			var albumHeaders []syncedAlbum
			var syncedJIDs []string
			var jidsWithMessages []string
			var chunkListed, chunkSkipped, chunkTokenOnly, chunkDropped int
			var convLIDPairs []store.LIDMapping

			if gs := v.Data.GetGlobalSettings(); gs != nil {
				if cls := gs.GetChatLockSettings(); cls != nil {
					if sc := cls.GetSecretCode(); sc != nil {
						if b, err := proto.Marshal(sc); err == nil {
							acc.saveChatLockSecretCode(b)
						}
					}
				}
			}

			chunkPNs := historyChunkLIDMap(v.Data.GetPhoneNumberToLidMappings())
			resolvePN := func(j types.JID) types.JID {
				j = j.ToNonAD()
				if j.Server != types.HiddenUserServer {
					return j
				}
				if pn, ok := chunkPNs[j.User]; ok {
					return pn
				}
				if pn, err := acc.wa().Store.LIDs.GetPNForLID(context.Background(), j); err == nil && !pn.IsEmpty() {
					return pn.ToNonAD()
				}
				return j
			}
			acc.mergeLIDRowsIntoPN(chunkPNs)

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
				pushNameWrites = append(pushNameWrites, func(tx sqlExecer) {
					upsertContactCacheRowWith(tx, targetJID.String(), pushname, phone, false)
					tx.Exec(`UPDATE chats SET name = ? WHERE jid = ? AND (name IS NULL OR name = '' OR name LIKE '+%')`, pushname, targetJID.String())
				})
			}
			acc.writeBatch(pushNameWrites)
			if len(pushNameWrites) > 0 || len(chunkPNs) > 0 {
				acc.invalidateDisplayNames()
			}

			for convIdx, conv := range conversations {
				if reportProgress && convIdx > 0 && convIdx%5 == 0 {
					acc.broadcastEvent("history_sync", map[string]interface{}{
						"status":    "syncing",
						"chatsDone": acc.historySyncChatsDone + convIdx,
						"msgsDone":  acc.historySyncMsgsDone,
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
				if !gate.allows(jid, wireJID, omittedFrom >= 0 && convIdx >= omittedFrom) {
					chunkDropped++
					continue
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
					acc.requestGroupMetadata(chatJID)
				}
				if name == "" && chatJID.Server != types.GroupServer {
					if contact, err := acc.wa().Store.Contacts.GetContact(context.Background(), chatJID); err == nil && contact.Found {
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
				pinnedAt := int64(conv.GetPinned())
				if pinnedAt > 0 {
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
				mentions := conv.GetUnreadMentionCount()
				if mentions > unread {
					mentions = unread
				}
				if listed {
					writes = append(writes, func(tx sqlExecer) {
						_, err := tx.Exec(`INSERT INTO chats (jid, name, timestamp, unread_count, pinned, pinned_at, muted, archived, is_locked, marked_unread, wire_jid, unread_mentions) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
							ON CONFLICT(jid) DO UPDATE SET
							timestamp=MAX(timestamp, excluded.timestamp),
							name=CASE WHEN chats.name IS NULL OR chats.name = '' OR chats.name LIKE '+%' THEN COALESCE(NULLIF(excluded.name, ''), chats.name) ELSE chats.name END,
							unread_count=MAX(chats.unread_count, excluded.unread_count),
							unread_mentions=CASE WHEN excluded.unread_count >= chats.unread_count THEN MAX(COALESCE(chats.unread_mentions, 0), excluded.unread_mentions) ELSE chats.unread_mentions END,
							pinned=CASE WHEN COALESCE(chats.pin_changed_at, 0) > 0 THEN chats.pinned ELSE MAX(chats.pinned, excluded.pinned) END,
							pinned_at=CASE WHEN COALESCE(chats.pin_changed_at, 0) > 0 THEN chats.pinned_at ELSE MAX(COALESCE(chats.pinned_at, 0), excluded.pinned_at) END,
							muted=MAX(chats.muted, excluded.muted),
							archived=MAX(chats.archived, excluded.archived),
							is_locked=MAX(chats.is_locked, excluded.is_locked),
							marked_unread=MAX(COALESCE(chats.marked_unread, 0), excluded.marked_unread),
							wire_jid=COALESCE(NULLIF(chats.wire_jid, ''), excluded.wire_jid)`,
							jid, name, ts, unread, pinned, pinnedAt, muted, archived, locked, markedUnread, wireJID, mentions)
						if err != nil {
							fmt.Println("Error inserting chat from HistorySync:", err)
						}
					})
				}
				if conv.LimitSharing != nil {
					limitSharing, limitSharingTS := boolToInt(conv.GetLimitSharing()), conv.GetLimitSharingSettingTimestamp()
					writes = append(writes, func(tx sqlExecer) {
						tx.Exec(`UPDATE chats SET limit_sharing = ?, limit_sharing_ts = ? WHERE jid = ? AND COALESCE(limit_sharing_ts, 0) <= ?`,
							limitSharing, limitSharingTS, jid, limitSharingTS)
					})
				}
				if conv.Acp2Setting != nil {
					restricted, restrictedTS := boolToInt(conv.GetAcp2Setting().GetEnabled()), conv.GetAcp2Setting().GetSettingTimestamp()
					writes = append(writes, func(tx sqlExecer) {
						tx.Exec(`UPDATE chats SET restricted = ?, restricted_ts = ? WHERE jid = ? AND COALESCE(restricted_ts, 0) <= ?`,
							restricted, restrictedTS, jid, restrictedTS)
					})
				}
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
							stubSender := ""
							if !stubFromMe {
								if s := acc.historyStubActor(webMsg); !s.IsEmpty() {
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
					if webMsg.GetMessageStubType() != 0 && webMsg.GetMessage() == nil {
						if text := acc.historyStubText(webMsg); text != "" {
							stubTS := int64(webMsg.GetMessageTimestamp())
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
					evtMsg, parseEvtErr := acc.wa().ParseWebMessage(chatJID, webMsg)
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
						if reaction, err := acc.wa().DecryptReaction(context.Background(), evtMsg); err == nil {
							writes = append(writes, func(tx sqlExecer) { storeReactionTx(tx, jid, evtMsg, reaction) })
						} else {
							log.Printf("Failed to decrypt announcement-group reaction from HistorySync: %v", err)
						}
						continue
					}
					if update := unwrapMessage(evtMsg.Message).GetPollUpdateMessage(); update != nil {
						if vote, err := acc.decryptPollVote(evtMsg); err == nil {
							pollID, voter := update.GetPollCreationMessageKey().GetID(), acc.pollVoterKey(evtMsg.Info.Sender)
							ts := evtMsg.Info.Timestamp.Unix()
							writes = append(writes, func(tx sqlExecer) { storePollVote(tx, pollID, voter, vote.GetSelectedOptions(), ts) })
						}
						continue
					}
					if len(webMsg.GetPollUpdates()) > 0 {
						pollMsg := webMsg
						writes = append(writes, func(tx sqlExecer) { acc.historyPollVotes(tx, chatJID, pollMsg) })
					}
					if enc := unwrapMessage(evtMsg.Message).GetEncEventResponseMessage(); enc != nil {
						if resp, err := acc.decryptEventResponse(evtMsg); err == nil {
							eventID, responder := enc.GetEventCreationMessageKey().GetID(), acc.pollVoterKey(evtMsg.Info.Sender)
							ts := evtMsg.Info.Timestamp.Unix()
							writes = append(writes, func(tx sqlExecer) { storeEventResponse(tx, eventID, responder, resp, ts) })
						}
						continue
					}
					if len(webMsg.GetEventResponses()) > 0 {
						eventMsg := webMsg
						writes = append(writes, func(tx sqlExecer) { acc.historyEventResponses(tx, chatJID, eventMsg) })
					}

					if parentID := hdParentID(evtMsg.Message); parentID != "" {
						hdMsg := evtMsg.Message
						writes = append(writes, func(tx sqlExecer) { storeHDMedia(tx, parentID, hdMsg) })
						continue
					}
					if album := unwrapMessage(evtMsg.Message).GetAlbumMessage(); album != nil {
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

					msgText := acc.extractText(evtMsg.Message)
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
						quotedID, quotedText, quotedSender := acc.quotedPreview(evtMsg.Message)
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
								participant := acc.preferPN(userJID).String()
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

				acc.writeBatch(writes)
				acc.historySyncMsgsDone += len(conv.GetMessages())
				acc.historySyncChatsDone++
				if reportProgress && len(conv.GetMessages()) >= 200 {
					acc.broadcastEvent("history_sync", map[string]interface{}{
						"status":    "syncing",
						"chatsDone": acc.historySyncChatsDone,
						"msgsDone":  acc.historySyncMsgsDone,
					})
				}
			}
			for _, album := range albumHeaders {
				acc.stampAlbumMembers(album.id, album.jid, album.senderJID, album.timestamp, album.expected)
			}
			if len(convLIDPairs) > 0 {
				acc.wa().StoreObservedLIDMappings(context.Background(), convLIDPairs)
			}
			loopTook := time.Since(chunkStart)
			tailStart := time.Now()
			for _, j := range syncedJIDs {
				acc.UpdateChatSnippet(j)
			}
			if v.Data.GetSyncType() != waHistorySync.HistorySync_ON_DEMAND {
				acc.scheduleHistorySyncRefresh()
			}
			acc.broadcastChats()
			tailTook := time.Since(tailStart)
			// Before the chunk's last progress line, so that line already
			// carries the percent this chunk brought.
			acc.noteHistoryChunkProgress(v.Data.GetSyncType(), v.Data.GetChunkOrder(), v.Data.GetProgress())

			if reportProgress {
				acc.broadcastEvent("history_sync", map[string]interface{}{
					"status":    "syncing",
					"chatsDone": acc.historySyncChatsDone,
					"msgsDone":  acc.historySyncMsgsDone,
				})
			}
			// Sent for every chunk, on-demand ones included: an on-demand
			// chunk is the phone's answer to "load older messages", and the
			// UI waits on exactly this to page them in. It used to ride on the
			// progress event above, which on-demand chunks skip - so the answer
			// was written but never announced, the UI timed out and marked the
			// chat as having no more history until whatSnap was restarted.
			// Only chats that got messages: an empty answer to an on-demand
			// request (the phone has nothing older) must not look like new
			// history, or the UI would un-mark the chat and ask again.
			if v.Data.GetSyncType() == waHistorySync.HistorySync_ON_DEMAND {
				for _, j := range jidsWithMessages {
					acc.noteOnDemandAnswer(j)
				}
			}
			if len(jidsWithMessages) > 0 {
				acc.broadcastEvent("history_chats_synced", map[string]interface{}{
					"jids":     jidsWithMessages,
					"onDemand": v.Data.GetSyncType() == waHistorySync.HistorySync_ON_DEMAND,
				})
			}
			if convCount > 0 || v.Data.GetSyncType() == waHistorySync.HistorySync_FULL {
				stats := acc.db.Stats()
				log.Printf("History sync chunk written: type=%s chunk=%d progress=%d chats=%d (listed=%d skipped=%d tokenOnly=%d dropped=%d) msgs=%d took=%s (messages %s, snippets %s) queued=%d dbWait=%d/%s inUse=%d",
					v.Data.GetSyncType(), v.Data.GetChunkOrder(), v.Data.GetProgress(), convCount, chunkListed, chunkSkipped, chunkTokenOnly, chunkDropped, chunkMsgs,
					time.Since(chunkStart).Round(time.Millisecond), loopTook.Round(time.Millisecond), tailTook.Round(time.Millisecond),
					acc.historySyncPending.Load()-1,
					stats.WaitCount-statsAtStart.WaitCount, (stats.WaitDuration - statsAtStart.WaitDuration).Round(time.Millisecond),
					stats.InUse)
			}
			if v.Data.GetSyncType() == waHistorySync.HistorySync_FULL && v.Data.GetProgress() >= 100 {
				acc.scheduleHistorySyncCheckpoint()
			}
		}()

	case *events.Message:
		unwrappedMsg := unwrapMessage(v.Message)
		if v.Info.VerifiedName != nil && !v.Info.IsFromMe && !v.Info.IsGroup {
			go acc.markBusinessContact(v.Info.Chat)
		}
		if setting, toggle := limitSharingInMessage(v.Message); setting != nil {
			chat, _ := acc.resolveLIDChat(v)
			ts := setting.GetLimitSharingSettingTimestamp()
			if ts == 0 {
				ts = v.Info.Timestamp.UnixMilli()
			}
			go func() {
				if acc.setChatLimitSharing(chat.String(), setting.GetSharingLimited(), ts) && toggle {
					acc.noteChatPrivacyChange(chat.String(), v.Info.Sender, setting.GetSharingLimited(), time.UnixMilli(ts))
				}
			}()
			if toggle {
				return
			}
		}
		if setting, toggle := acp2InMessage(v.Message); setting != nil {
			chat, _ := acc.resolveLIDChat(v)
			ts := setting.GetSettingTimestamp()
			if ts == 0 {
				ts = v.Info.Timestamp.UnixMilli()
			}
			go func() {
				if acc.setChatRestricted(chat.String(), setting.GetEnabled(), ts) && toggle {
					acc.noteRestrictedChatChange(chat.String(), v.Info.Sender, setting.GetEnabled(), time.UnixMilli(ts))
				}
			}()
			if toggle {
				return
			}
		}
		if v.Info.Chat.Server == types.NewsletterServer {
			switch v.Info.Edit {
			case types.EditAttributeAdminRevoke:
				jid := v.Info.Chat.String()
				acc.markMessageDeleted(v.Info.ID, jid)
				acc.broadcastEvent("message_deleted", map[string]interface{}{
					"jid": jid, "message_id": v.Info.ID, "by": jid,
				})
				return
			case types.EditAttributeAdminEdit:
				jid, id := v.Info.Chat.String(), v.Info.ID
				editTime := v.Info.Timestamp.Unix()
				func() {
					shown, _ := acc.markMessageEdited(id, jid, v.Message, acc.extractText(v.Message), editTime)
					acc.broadcastEvent("message_edited", map[string]interface{}{
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
				decrypted, err := acc.wa().DecryptSecretEncryptedMessage(context.Background(), v)
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
			newText := acc.extractText(newContent)
			if newText == "" || newText == "[Unsupported Message]" || newText == "[Protocol Message]" {
				newText = acc.extractText(v.Message)
			}
			if newText == "[Protocol Message]" || newText == "[Unsupported Message]" {
				newText = acc.extractText(unwrappedMsg)
			}
			resolvedChat, _ := acc.resolveLIDChat(v)
			jid := resolvedChat.String()
			editTime := v.Info.Timestamp.Unix()
			if targetID != "" {
				func() {
					if v.Info.IsFromMe {
						acc.recordEditStanza(v.Info.ID, targetID, jid, editTime)
					}
					shown, _ := acc.markMessageEdited(targetID, jid, newContent, newText, editTime)
					acc.broadcastEvent("message_edited", map[string]interface{}{
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
			acc.applyEphemeralSettingMessage(v, pm)
			return
		}
		if pm := unwrappedMsg.GetProtocolMessage(); pm != nil && pm.GetType() == waE2E.ProtocolMessage_GROUP_MEMBER_LABEL_CHANGE {
			acc.applyMemberLabelChange(v)
			return
		}
		if unwrappedMsg != nil && unwrappedMsg.GetProtocolMessage() != nil && unwrappedMsg.GetProtocolMessage().GetType() == waE2E.ProtocolMessage_REVOKE {
			targetID := unwrappedMsg.GetProtocolMessage().GetKey().GetID()
			if v.Info.Chat == types.StatusBroadcastJID {
				go acc.removeStatus(targetID)
				return
			}
			resolvedChat, _ := acc.resolveLIDChat(v)
			jid := resolvedChat.String()
			acc.markMessageDeleted(targetID, jid)
			acc.broadcastEvent("message_deleted", map[string]interface{}{
				"jid":        jid,
				"message_id": targetID,
				"by":         v.Info.Sender.String(),
			})
			return
		}
		if reaction := unwrappedMsg.GetReactionMessage(); reaction != nil {
			acc.applyReaction(v, reaction)
			return
		}
		if pin := unwrappedMsg.GetPinInChatMessage(); pin != nil {
			acc.applyPinInChat(v, pin)
			return
		}
		if keep := unwrappedMsg.GetKeepInChatMessage(); keep != nil {
			acc.applyKeepInChat(v, keep)
			return
		}
		if unwrappedMsg.GetPollUpdateMessage() != nil {
			acc.applyPollVote(v)
			return
		}
		if unwrappedMsg.GetEncEventResponseMessage() != nil {
			acc.applyEventResponse(v)
			return
		}
		if unwrappedMsg.GetEncReactionMessage() != nil {
			reaction, err := acc.wa().DecryptReaction(context.Background(), v)
			if err != nil {
				log.Printf("Failed to decrypt announcement-group reaction from %s: %v", v.Info.Sender, err)
				return
			}
			acc.applyReaction(v, reaction)
			return
		}
		if v.Info.Chat.User == "status" && v.Info.Chat.Server == types.BroadcastServer {
			acc.handleIncomingStatus(v)
			return
		}
		if parentID := hdParentID(v.Message); parentID != "" {
			acc.noteLiveHDMedia(parentID, v.Message)
			return
		}

		if album := unwrappedMsg.GetAlbumMessage(); album != nil {
			albumChat, albumSender := acc.resolveLIDChat(v)
			if acc.registerAlbum(albumChat.String(), albumSender.String(), v.Info.ID, album) {
				log.Printf("Album header %s from %s: expecting %d media", v.Info.ID, albumSender,
					album.GetExpectedImageCount()+album.GetExpectedVideoCount())
			}
			return
		}

		if notice := unwrappedMsg.GetMessageHistoryNotice(); notice != nil {
			chat, _ := acc.resolveLIDChat(v)
			text := acc.messageHistoryNoticeText(notice, v.Info.Sender)
			acc.insertGroupSystemMessage(chat.String(), v.Info.ID, text, v.Info.Timestamp, true)
			return
		}

		if unwrappedMsg.GetMessageHistoryBundle() != nil {
			return
		}

		if isKeyDistributionOnly(unwrappedMsg) {
			return
		}

		text := acc.extractText(v.Message)
		if text == "[Protocol Message]" {
			return
		}
		if v.IsViewOnce {
			// Already opened here: a redelivered copy must not bring it back.
			if acc.viewOnceState(v.Info.ID) == viewOnceOpened {
				return
			}
			text = viewOnceText(text)
		}
		if text == "[Unsupported Message]" && v.Info.Sender.IsBot() {
			if *verboseLogFlag {
				log.Printf("Unrecognized bot message content from %s: %s", v.Info.Sender, unwrapMessage(v.Message).String())
			} else {
				log.Printf("Unrecognized bot message content from %s (enable verbose logging to capture it)", v.Info.Sender)
			}
		}
		if (v.Info.MsgBotInfo.EditType == types.EditTypeInner || v.Info.MsgBotInfo.EditType == types.EditTypeLast) &&
			v.Info.MsgBotInfo.EditTargetID != "" {
			resolvedChat, _ := acc.resolveLIDChat(v)
			jid := resolvedChat.String()
			targetID := v.Info.MsgBotInfo.EditTargetID
			editTime := v.Info.Timestamp.Unix()
			acc.markMessageEdited(targetID, jid, unwrapMessage(v.Message), text, editTime)
			acc.broadcastEvent("message_edited", map[string]interface{}{
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

		albumID := ""
		if mt, _, _, _ := mediaInfoFromMessage(v.Message); mt == "image" || mt == "video" {
			albumChat, albumSender := acc.resolveLIDChat(v)
			albumID = acc.claimAlbumID(albumChat.String(), albumSender.String())
		}

		func() {
			chatJID, senderJID := acc.resolveLIDChat(v)

			jid := chatJID.String()

			if isIgnoredChatJID(jid) && !acc.isFollowedChannelPost(jid) {
				return
			}
			ts := v.Info.Timestamp.Unix()
			increment := 1
			if v.Info.IsFromMe {
				increment = 0
			}
			if increment > 0 && v.Info.ID != "" {
				var alreadyExists int
				_ = acc.db.QueryRow(`SELECT COUNT(*) FROM messages WHERE id = ?`, v.Info.ID).Scan(&alreadyExists)
				if alreadyExists > 0 {
					// Redelivered/duplicate event for a message we've already
					// counted (e.g. reconnect resync) - don't double-count it.
					increment = 0
				}
			}
			mentions := 0
			if increment > 0 && chatJID.Server == types.GroupServer && acc.addressesMe(v.Message) {
				mentions = 1
			}
			movesChat := !v.Info.IsFromMe || sentMessageMovesChat(v.Message)
			_, err := acc.db.Exec(`INSERT INTO chats (jid, timestamp, unread_count, preview_snippet, preview_sender, preview_from_me, wire_jid, unread_mentions) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
				ON CONFLICT(jid) DO UPDATE SET
					timestamp=`+chatTimeAfter(movesChat)+`,
					unread_count=chats.unread_count + excluded.unread_count,
					unread_mentions=COALESCE(chats.unread_mentions, 0) + excluded.unread_mentions,
					preview_snippet=COALESCE(NULLIF(excluded.preview_snippet, ''), chats.preview_snippet),
					preview_sender=excluded.preview_sender,
					preview_from_me=excluded.preview_from_me,
					preview_is_reaction=0,
					preview_reaction_emoji='',
					wire_jid=COALESCE(NULLIF(excluded.wire_jid, ''), chats.wire_jid)`,
				jid, ts, increment, text, senderJID.String(), v.Info.IsFromMe, wireChat(v.Info.Chat, chatJID), mentions)
			if err != nil {
				fmt.Println("Error updating chat timestamp:", err)
			}
			if chatJID.Server == types.GroupServer {
				var name string
				acc.db.QueryRow(`SELECT COALESCE(name, '') FROM chats WHERE jid = ?`, jid).Scan(&name)
				if name == "" {
					acc.requestGroupMetadata(chatJID)
				}
			}
			if v.Info.ID != "" {
				mediaType, mimetype, filename, mediaSize := mediaInfoFromMessage(v.Message)
				quotedID, quotedText, quotedSender := acc.quotedPreview(v.Message)
				status := ""
				if v.Info.IsFromMe {
					status = "sent"
				}
				rawBytes, _ := proto.Marshal(v.Message)
				err := acc.saveMessage(StoredMessage{
					ID: v.Info.ID, JID: jid, SenderJID: senderJID.String(), WireSender: wireSender(chatJID, v.Info.Sender, senderJID),
					FromMe: v.Info.IsFromMe, Text: text, Timestamp: ts, Raw: rawBytes, Status: status,
					MediaType: mediaType, MediaMimetype: mimetype, MediaFilename: filename, MediaSize: mediaSize,
					QuotedID: quotedID, QuotedText: quotedText, QuotedSender: quotedSender,
					AlbumID: albumID, ServerID: int64(v.Info.ServerID),
				})
				if err != nil {
					fmt.Println("Error inserting message from live event:", err)
				} else {
					if v.Info.ServerStoreTime > 0 {
						acc.db.Exec(`UPDATE messages SET server_sts = ? WHERE id = ?`, v.Info.ServerStoreTime, v.Info.ID)
					}
					noteMessageExpiry(acc.db, v.Info.ID, ts, v.Message)
					acc.adoptChatDisappearing(chatJID, v.Message)
					if v.IsViewOnce {
						markViewOnce(acc.db, v.Info.ID)
					}
					if mediaType == "voice" || mediaType == "sticker" {
						acc.autoDownloadVoiceNote(v.Info.ID, jid)
					} else if mediaType != "" && !v.IsViewOnce {
						acc.maybeAutoDownload(v.Message, v.Info, jid, mediaType, mediaSize)
					}
				}
			}
			acc.broadcastChats()
		}()
		if text != "" {
			chatJID, senderJID := acc.resolveLIDChat(v)
			if isIgnoredChatJID(chatJID.String()) && !acc.isFollowedChannelPost(chatJID.String()) {
				return
			}
			liveMediaType, _, liveMediaFilename, liveMediaSize := mediaInfoFromMessage(v.Message)

			quotedID, quotedText, quotedSender := acc.quotedPreview(v.Message)
			senderName := ""
			if !v.Info.IsFromMe {
				senderName = acc.resolveDisplayName(senderJID)
			}
			acc.broadcastEvent("message", map[string]interface{}{
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
				"addressesMe":   chatJID.Server == types.GroupServer && acc.addressesMe(v.Message),
				"groupSize":     acc.groupSize(chatJID),
			})
		}
	case *events.UndecryptableMessage:
		go acc.storeUndecryptablePlaceholder(v)
	case *events.IdentityChange:
		go acc.noteSecurityCodeChange(v)
	case *events.PushName:
		acc.invalidateDisplayNames()
		go func() {
			// whatsmeow has already stored the name. A sender named only by
			// LID is cached under their phone JID when that's known, and a
			// LID's digits are never cached as a phone number.
			target := acc.preferPN(v.JID)
			if target.Server == types.HiddenUserServer && v.JIDAlt.Server == types.DefaultUserServer {
				target = v.JIDAlt.ToNonAD()
			}
			jidStr := target.String()
			if v.NewPushName != "" {
				acc.upsertContactCacheRow(jidStr, v.NewPushName, cachePhone(target), false)
				acc.db.Exec(`UPDATE chats SET name = ? WHERE jid = ? AND (name IS NULL OR name = '' OR name LIKE '+%')`, v.NewPushName, jidStr)
				acc.broadcastChats()
			}
		}()
	case *events.Contact:
		acc.invalidateDisplayNames()
		if v.FromFullSync && !v.Removed {
			return
		}
		go func() {
			jidStr := acc.preferPN(v.JID).String()
			if v.Removed {
				acc.deleteContactCacheRow(jidStr)
				acc.db.Exec(`UPDATE chats SET name = '' WHERE jid = ?`, jidStr)
				acc.populateChatsFromStore()
				acc.broadcastChats()
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
				phone := ""
				if pj, err := types.ParseJID(jidStr); err == nil && pj.Server == types.DefaultUserServer {
					phone = pj.User
				}
				acc.upsertContactCacheRow(jidStr, fullName, phone, false)
				acc.db.Exec(`UPDATE chats SET name = ? WHERE jid = ?`, fullName, jidStr)
				acc.broadcastChats()
			}
		}()
	case *events.BusinessName:
		acc.invalidateDisplayNames()
		go func() {
			target := acc.preferPN(v.JID)
			jidStr := target.String()
			if v.NewBusinessName != "" {
				acc.upsertContactCacheRow(jidStr, v.NewBusinessName, cachePhone(target), true)
				acc.db.Exec(`UPDATE chats SET name = ? WHERE jid = ? AND (name IS NULL OR name = '' OR name LIKE '+%')`, v.NewBusinessName, jidStr)
				acc.broadcastChats()
			}
		}()
	case *events.OfflineSyncPreview, *events.OfflineSyncProgress:
		acc.handleOfflineSyncProgress(v)
	case *events.OfflineSyncCompleted:
		log.Printf("OfflineSyncCompleted event received. Count: %d", v.Count)
		acc.handleOfflineSyncProgress(v)
		go func() {
			acc.UpdateAllChatSnippets()
			acc.populateChatsFromStore()
			acc.broadcastChats()
		}()
	case *events.DeleteForMe:
		// Synced from another of our own linked devices (or replayed on
		// first link / history resync) - always a local-only hide, never a
		// tombstone. See deleteMessageForMe's doc comment.
		jid := v.ChatJID.String()
		go acc.deleteMessageForMe(v.MessageID, jid)
		go acc.applySyncedCallDelete(v.MessageID)
		acc.broadcastEvent("message_deleted", map[string]interface{}{
			"jid":          jid,
			"message_id":   v.MessageID,
			"by":           v.SenderJID.String(),
			"for_everyone": false,
		})
	case *events.Connected:
		acc.forgetPresenceSubscriptions()
		acc.forgetChannelRefreshes()
		acc.convertStoredVotesOnce.Do(func() { go acc.runOnce("convert_stored_poll_votes_v1", acc.convertStoredPollVotes) })
		acc.recheckChatLIDsOnce.Do(func() { go acc.recheckChatLIDs() })
		acc.learnKeepArchivedOnce.Do(func() {
			go func() {
				time.Sleep(10 * time.Second)
				acc.learnKeepChatsArchived()
			}()
		})
		go acc.refreshServerFlags()
		acc.appStateResyncMu.Lock()
		for k := range acc.appStateResyncCount {
			delete(acc.appStateResyncCount, k)
		}
		for k := range acc.lastAppStateResync {
			delete(acc.lastAppStateResync, k)
		}
		acc.appStateResyncMu.Unlock()
		acc.connFailures.Store(0)
		if acc.wa().Store.ID != nil {
			acc.setConnState(connStateOnline, "", 0)
			acc.broadcastEvent("connected", map[string]string{"jid": acc.wa().Store.ID.String()})
			go acc.sendDesiredPresence(true)
			go acc.retryPendingTextMessages()
			go acc.flushPendingResends()
			go acc.refreshBlocklistOnConnect()
			go func() {
				time.Sleep(3 * time.Second)
				acc.populateChatsFromStore()
				acc.broadcastChats()
			}()
		}
	case *events.PairSuccess:
		log.Printf("Successfully paired with %s", v.ID.String())
		acc.broadcastEvent("pair_success", map[string]string{"jid": v.ID.String()})
		go acc.sendDesiredPresence(true)
		go func() {
			time.Sleep(5 * time.Second)
			acc.populateChatsFromStore()
			acc.broadcastChats()
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
			acc.replaceWithFreshClient()
			acc.broadcastEvent("needs_login", map[string]string{"reason": reason})
		}()
	case *events.StreamReplaced:
		log.Println("Session replaced by another connection")
		msg := "whatSnap was disconnected because this WhatsApp account connected from another session. Use Reconnect Now in the Account menu to take the session back."
		acc.broadcastEvent("error", map[string]string{"message": msg})
		acc.setConnState(connStateStopped, "Disconnected: this account connected from another session.", 0)
	case *events.TemporaryBan:
		log.Println("Temporary ban:", v.String())
		acc.broadcastEvent("error", map[string]string{"message": v.String()})
		if v.Expire > 0 {
			acc.setConnState(connStateOffline, "Temporarily banned by WhatsApp.", v.Expire)
			acc.requestConnect(v.Expire + 5*time.Second)
		} else {
			acc.scheduleRetryAfterFailure("Temporarily banned by WhatsApp.")
		}
	case *events.ClientOutdated:
		log.Println("WhatsApp rejected this client as outdated")
		acc.broadcastEvent("error", map[string]string{
			"message": "WhatsApp has rejected this version of whatSnap as outdated. Please update whatSnap to reconnect.",
		})
		acc.setConnState(connStateStopped, "This version of whatSnap is outdated. Please update it.", 0)
	case *events.CATRefreshError:
		log.Println("CAT refresh error:", v.Error)
		acc.scheduleRetryAfterFailure("Couldn't refresh WhatsApp's connection token.")
	case *events.ConnectFailure:
		log.Println("Unhandled connect failure:", v.Reason.String(), v.Message)
		acc.scheduleRetryAfterFailure("WhatsApp refused the connection (" + v.Reason.String() + ").")
	case *events.StreamError:
		log.Println("Unknown stream error, code:", v.Code)
	case *events.Disconnected:
		acc.setConnState(connStateConnecting, "Connection lost.", 0)
	case *events.KeepAliveTimeout:
		log.Printf("Keepalive timed out (%d in a row, last answer %s ago)", v.ErrorCount, time.Since(v.LastSuccess).Round(time.Second))
		if acc.currentConnState()["state"] == connStateOnline {
			acc.setConnState(connStateConnecting, "Not responding.", 0)
		}
	case *events.KeepAliveRestored:
		if isOnline(acc.wa()) && acc.currentConnState()["state"] == connStateConnecting {
			acc.setConnState(connStateOnline, "", 0)
		}
	case *events.ChatPresence:
		isComposing := v.State == types.ChatPresenceComposing
		isRecording := isComposing && v.Media == types.ChatPresenceMediaAudio
		isTyping := isComposing && !isRecording

		chatJID, senderJID := acc.resolveSourcePN(v.MessageSource)

		acc.broadcastEvent("typing", map[string]interface{}{
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
			if pn, err := acc.wa().Store.LIDs.GetPNForLID(context.Background(), targetJID); err == nil && !pn.IsEmpty() {
				targetJID = pn.ToNonAD()
			}
		}

		acc.broadcastEvent("presence", map[string]interface{}{
			"jid":         targetJID.String(),
			"unavailable": v.Unavailable,
			"last_seen":   lastSeenUnix,
		})
	case *events.Receipt:
		if v.MessageSource.Chat == types.StatusBroadcastJID && !v.MessageSource.IsFromMe {
			go acc.noteStatusViewers(v)
		}
		chatJID, senderJID := acc.resolveSourcePN(v.MessageSource)

		acc.broadcastEvent("receipt", map[string]interface{}{
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
			updatedByStatus := map[string][]string{}
			if chatJID.Server == "g.us" && !v.MessageSource.IsFromMe && (status == "delivered" || status == "read" || status == "played") {
				acc.updateParticipantReceiptIfNewer(ids, senderJID.String(), status, receiptTS)
				updatedByStatus = acc.updateGroupMessageStatuses(ids, chatJID.String(), receiptTS)
			} else if updated := acc.updateMessageStatusIfNewer(ids, status, receiptTS); len(updated) > 0 {
				updatedByStatus[status] = updated
			}
			if len(updatedByStatus) > 0 {
				acc.broadcastChats()
			}
			for newStatus, updated := range updatedByStatus {
				acc.broadcastEvent("message_status", map[string]interface{}{
					"jid":         resolvedChatJID.String(),
					"message_ids": updated,
					"status":      newStatus,
					"timestamp":   receiptTS,
				})
			}
			isReadSelf := v.Type == types.ReceiptTypeReadSelf || v.Type == types.ReceiptTypePlayedSelf || ((v.Type == types.ReceiptTypeRead || v.Type == types.ReceiptTypePlayed) && v.MessageSource.IsFromMe)
			if isReadSelf {
				acc.db.Exec("UPDATE chats SET unread_count = 0 WHERE jid = ?", resolvedChatJID.String())
				for _, mid := range v.MessageIDs {
					acc.db.Exec("UPDATE chats SET unread_count = 0 WHERE jid = (SELECT jid FROM messages WHERE id = ?)", string(mid))
				}
				acc.broadcastChats()
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
						acc.saveChatLockSecretCode(b)
					}
				}
			}
			if lca := v.GetLockChatAction(); lca != nil {
				if len(v.Index) > 1 {
					jid := acc.appStateChatJID(v.Index[1])
					isLockedInt := 0
					if lca.GetLocked() {
						isLockedInt = 1
					}
					acc.db.Exec("UPDATE chats SET is_locked = ? WHERE jid = ?", isLockedInt, jid)
					acc.broadcastChats()
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
		}()
	case *events.AppStateSyncComplete:
		if len(v.Name) > 0 {
			go func() {
				acc.populateChatsFromStore()
				acc.broadcastChats()
			}()
		}
	case *events.AppStateSyncError:
		acc.handleAppStateSyncError(v)
	case *events.CallLogSync:
		go acc.handleCallLogSync(v)
	case *events.CallLogHistory:
		go acc.handleCallLogHistory(v)
	case *events.DeleteIndividualCallLog:
		go acc.handleDeleteIndividualCallLog(v)
	case *events.Archive:
		go func() {
			if v.Action == nil {
				return
			}
			jid := acc.preferPN(v.JID).String()
			acc.noteArchiveAction(jid, v.Action.GetArchived(), v.Action.GetMessageRange(), v.Timestamp.Unix())
			acc.db.Exec("UPDATE chats SET archived = ? WHERE jid = ?", boolToInt(v.Action.GetArchived()), jid)
			acc.broadcastChats()
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
			acc.db.Exec("UPDATE chats SET muted = ? WHERE jid = ?", mutedInt, acc.preferPN(v.JID).String())
			acc.broadcastChats()
		}()
	case *events.Pin:
		// Applied right here, in the order whatsmeow hands the mutations over,
		// not in a goroutine per event: a full sync can carry an old unpin under
		// the phone number and a newer pin under the LID for the same chat, and
		// both land on one row. Like WA Web (WAWebPinChatSync) the newest
		// mutation wins, and pinning takes the chat out of the archive.
		if v.Action != nil {
			acc.applyPinChange(acc.preferPN(v.JID).String(), v.Action.GetPinned(), v.Timestamp.Unix())
		}
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
			acc.db.Exec("UPDATE chats SET is_locked = ? WHERE jid = ?", lockedInt, acc.preferPN(v.JID).String())
			acc.broadcastChats()
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
			jidStr := acc.preferPN(v.JID).String()
			// Like WA Web (WAWebMarkChatAsReadSync): when messages have come
			// in here since the other device marked the chat, a read only
			// clears a "marked as unread" mark and an unread does nothing.
			covers := acc.rangeCoversChat(jidStr, v.Action.GetMessageRange())
			if !covers {
				if v.Action.GetRead() {
					acc.db.Exec("UPDATE chats SET unread_count = 0, marked_unread = 0 WHERE jid = ? AND COALESCE(marked_unread, 0) = 1", jidStr)
				}
			} else if v.Action.GetRead() {
				acc.db.Exec("UPDATE chats SET unread_count = 0 WHERE jid = ?", jidStr)
			} else {
				// "Mark as unread" only needs the chat to show as unread - it
				// must not replace a real count history sync already gave it
				// (a full app-state sync replays this for every such chat).
				acc.db.Exec("UPDATE chats SET unread_count = MAX(unread_count, 1), marked_unread = 1 WHERE jid = ?", jidStr)
			}
			acc.broadcastChats()
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
			acc.db.Exec("UPDATE messages SET starred = ? WHERE id = ?", starredInt, v.MessageID)
			acc.broadcastEvent("message_starred", map[string]interface{}{
				"jid": acc.preferPN(v.ChatJID).String(), "message_id": v.MessageID, "starred": v.Action.GetStarred(),
			})
		}()
	case *events.ClearChat:
		go func() {
			jidStr := acc.preferPN(v.JID).String()
			if acc.purgeChatRange(jidStr, v.Action.GetMessageRange()) {
				acc.clearChatPreview(jidStr)
			} else {
				acc.UpdateChatSnippet(jidStr)
			}
			acc.broadcastChats()
			acc.broadcastEvent("chat_cleared", map[string]interface{}{"jid": jidStr})
		}()
	case *events.DeleteChat:
		// Previously nothing here at all - handleDeleteChat only ever
		// deleted a chat locally and never told WhatsApp (see its own doc
		// comment), so this case could never have fired from this app's own
		// actions; it only ever meant the phone or another linked device
		// deleted the chat.
		go func() {
			jidStr := acc.preferPN(v.JID).String()
			if acc.purgeChatRange(jidStr, v.Action.GetMessageRange()) {
				acc.db.Exec("DELETE FROM chats WHERE jid = ?", jidStr)
			} else {
				acc.UpdateChatSnippet(jidStr)
				acc.broadcastEvent("chat_cleared", map[string]interface{}{"jid": jidStr})
			}
			acc.broadcastChats()
		}()
	case *events.Picture:
		go func() {
			jidStr := v.JID.String()
			if v.Remove {
				acc.updateContactAvatarCache(jidStr, "", "")
			}
			acc.forgetAvatarPreview(jidStr)
			if pn := acc.preferPN(v.JID).String(); pn != jidStr {
				acc.forgetAvatarPreview(pn)
				acc.broadcastEvent("avatar_updated", map[string]interface{}{"jid": pn, "removed": v.Remove})
			}
			if v.JID.Server == types.GroupServer && !v.Timestamp.IsZero() {
				acc.insertGroupSystemMessage(jidStr, fmt.Sprintf("system_%s_%d_picture", v.JID.User, v.Timestamp.UnixNano()),
					acc.groupPictureChangeText(v.Author, v.Remove), v.Timestamp, false)
			}
			acc.broadcastEvent("avatar_updated", map[string]interface{}{
				"jid":     jidStr,
				"removed": v.Remove,
			})
		}()
	case *events.UserAbout:
		go acc.updateContactAboutCache(v.JID.String(), v.Status)
		acc.broadcastEvent("about_updated", map[string]interface{}{
			"jid":    v.JID.String(),
			"status": v.Status,
		})
	case *events.DisableLinkPreviews:
		go acc.noteDisableLinkPreviews(v)
	case *events.UnarchiveChatsSetting:
		go acc.noteUnarchiveChatsSetting(v)
	case *events.SettingsSync:
		go acc.noteSettingsSync(v)
	case *events.FavoriteSticker:
		go acc.noteFavoriteSticker(v)
	case *events.UserStatusMute:
		go acc.noteStatusMute(v)
	case *events.StatusPrivacy:
		go acc.noteStatusPrivacy(v)
	case *events.DefaultDisappearingTimer:
		acc.broadcastEvent("default_timer", map[string]interface{}{"seconds": int(v.Timer.Seconds())})
	case *events.LabelEdit:
		go acc.noteLabelEdit(v)
	case *events.LabelAssociationChat:
		go acc.noteLabelChat(v)
	case *events.Favorites:
		go acc.noteFavorites(v)
	case *events.GroupLimitSharingUpdate:
		go func() {
			if acc.setChatLimitSharing(v.JID.String(), v.Enabled(), 0) {
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
				acc.noteChatPrivacyChange(v.JID.String(), by, v.Enabled(), ts)
			}
		}()
	case *events.GroupInfo:
		acc.invalidateJoinedGroups()
		if _, deleting := acc.pendingGroupDeletes.Load(v.JID.String()); deleting {
			return
		}
		go func() {
			if v.Name != nil {
				acc.db.Exec(`INSERT INTO chats (jid, name, timestamp) VALUES (?, ?, 0) ON CONFLICT(jid) DO UPDATE SET name=excluded.name`, v.JID.String(), v.Name.Name)
			}
			if v.Ephemeral != nil {
				seconds := uint32(0)
				if v.Ephemeral.IsEphemeral {
					seconds = v.Ephemeral.DisappearingTimer
				}
				acc.setChatDisappearing(v.JID.String(), seconds, v.Timestamp.Unix())
			}

			sendRightsChanged := false
			if v.Announce != nil {
				acc.db.Exec("UPDATE chats SET admins_only = ? WHERE jid = ?", boolToInt(v.Announce.IsAnnounce), v.JID.String())
				sendRightsChanged = true
			}
			isAdminChanged := false
			for _, j := range v.Promote {
				if acc.isParticipantMe(types.GroupParticipant{JID: j}) {
					acc.db.Exec("UPDATE chats SET is_admin = 1 WHERE jid = ?", v.JID.String())
					isAdminChanged = true
					break
				}
			}
			for _, j := range v.Demote {
				if acc.isParticipantMe(types.GroupParticipant{JID: j}) {
					acc.db.Exec("UPDATE chats SET is_admin = 0 WHERE jid = ?", v.JID.String())
					isAdminChanged = true
					break
				}
			}

			for _, j := range v.Join {
				if acc.isParticipantMe(types.GroupParticipant{JID: j}) {
					acc.markGroupJoined(v.JID.String(), true)
					sendRightsChanged = true
					acc.requestGroupMetadata(v.JID)
				}
			}
			for _, j := range v.Leave {
				if acc.isParticipantMe(types.GroupParticipant{JID: j}) {
					acc.markGroupJoined(v.JID.String(), false)
					sendRightsChanged = true
				}
			}
			if v.Link != nil {
				acc.applyCommunityLinkChange(v, v.Link, true)
			}
			if v.Unlink != nil {
				acc.applyCommunityLinkChange(v, v.Unlink, false)
			}

			if delta := len(v.Join) - len(v.Leave); delta != 0 {
				acc.db.Exec(`UPDATE chats SET participant_count = MAX(0, participant_count + ?) WHERE jid = ? AND participant_count > 0`, delta, v.JID.String())
			}

			for i, line := range acc.groupChangeMessages(v) {
				acc.insertGroupSystemMessage(v.JID.String(), fmt.Sprintf("system_%s_%d_%d", v.JID.User, v.Timestamp.UnixNano(), i), line.Text, v.Timestamp, line.MovesChat)
			}

			if isAdminChanged || sendRightsChanged {
				acc.broadcastChats()
			}
		}()
		acc.broadcastEvent("group_updated", map[string]interface{}{
			"jid": v.JID.String(),
		})
	case *events.NewsletterJoin, *events.NewsletterLeave, *events.NewsletterMuteChange, *events.NewsletterLiveUpdate:
		acc.handleChannelEvent(v)
	case *events.GroupsDirty:
		log.Printf("Server marked %d groups as out of date; refreshing", len(v.Groups))
		go func() {
			acc.invalidateJoinedGroups()
			acc.populateChatsFromStore()
			acc.broadcastChats()
		}()
	case *events.DirtyState:
		switch v.Type {
		case whatsmeow.DirtyTypeGroups:
			log.Printf("Server marked the group list as out of date; refreshing")
			go func() {
				acc.invalidateJoinedGroups()
				acc.populateChatsFromStore()
				acc.broadcastChats()
			}()
		case whatsmeow.DirtyTypeAccountSync:
			go acc.refreshBlocklist()
		}
	case *events.JoinedGroup:
		acc.invalidateJoinedGroups()
		go func() {
			if v.GroupName.Name != "" {
				acc.db.Exec(`INSERT INTO chats (jid, name, timestamp) VALUES (?, ?, 0) ON CONFLICT(jid) DO UPDATE SET name=excluded.name`, v.JID.String(), v.GroupName.Name)
			} else {
				acc.requestGroupMetadata(v.JID)
			}
			acc.recordGroupCommunity(&v.GroupInfo)
			acc.markGroupJoined(v.JID.ToNonAD().String(), true)
			actor := ""
			if v.Sender != nil && !acc.isParticipantMe(types.GroupParticipant{JID: *v.Sender}) {
				actor = acc.nameToken(*v.Sender)
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
			acc.insertGroupSystemMessage(v.JID.String(), fmt.Sprintf("system_%s_%d", v.JID.User, time.Now().UnixNano()), msg, time.Now(), true)

			acc.invalidateJoinedGroups()
			acc.populateChatsFromStore()
			acc.broadcastChats()
		}()
		acc.broadcastEvent("group_updated", map[string]interface{}{
			"jid": v.JID.String(),
		})
	case *events.Blocklist:
		go acc.refreshBlocklist()
	case *events.ABPropsRefreshed:
		go acc.refreshServerFlags()
	}
}

func (acc *Account) joinedNamesList(jids []types.JID) string {
	names := make([]string, 0, len(jids))
	for _, j := range jids {
		names = append(names, acc.nameToken(j))
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

// groupChangeLine is one system line for a group change, and whether WA Web
// moves the chat to the top for it (see insertGroupSystemMessage).
type groupChangeLine struct {
	Text      string
	MovesChat bool
}

func (acc *Account) groupChangeMessages(v *events.GroupInfo) []groupChangeLine {
	var lines []groupChangeLine
	quiet := func(text string) { lines = append(lines, groupChangeLine{Text: text}) }
	includesMe := func(jids []types.JID) bool {
		for _, j := range jids {
			if acc.isParticipantMe(types.GroupParticipant{JID: j}) {
				return true
			}
		}
		return false
	}
	actor := ""
	if v.Sender != nil {
		actor = acc.nameToken(*v.Sender)
	}
	// "Someone" never reads out a raw JID: settings changes can come with no
	// sender (the server applying them, or an old notification).
	who := actor
	if who == "" {
		who = "Someone"
	}

	if v.Name != nil && v.Name.Name != "" {
		lines = append(lines, groupChangeLine{Text: fmt.Sprintf("%s changed the group name to \"%s\"", who, v.Name.Name), MovesChat: true})
	}
	if v.Topic != nil {
		if v.Topic.TopicDeleted || v.Topic.Topic == "" {
			quiet(fmt.Sprintf("%s deleted the group description", who))
		} else {
			quiet(fmt.Sprintf("%s changed the group description", who))
		}
	}
	if v.Announce != nil {
		if v.Announce.IsAnnounce {
			quiet(fmt.Sprintf("%s changed this group's settings to allow only admins to send messages to this group", who))
		} else {
			quiet(fmt.Sprintf("%s changed this group's settings to allow all members to send messages to this group", who))
		}
	}
	if v.Locked != nil {
		if v.Locked.IsLocked {
			quiet(fmt.Sprintf("%s changed this group's settings to allow only admins to edit this group's info", who))
		} else {
			quiet(fmt.Sprintf("%s changed this group's settings to allow all members to edit this group's info", who))
		}
	}
	if v.Ephemeral != nil {
		quiet(disappearingChangeText(who, v.Ephemeral.IsEphemeral, v.Ephemeral.DisappearingTimer))
	}
	if v.MembershipApprovalMode != nil {
		if v.MembershipApprovalMode.IsJoinApprovalRequired {
			quiet(fmt.Sprintf("%s turned on admin approval to join this group", who))
		} else {
			quiet(fmt.Sprintf("%s turned off admin approval to join this group", who))
		}
	}
	if v.MemberAddMode != nil {
		switch *v.MemberAddMode {
		case types.GroupMemberAddModeAdmin:
			quiet(fmt.Sprintf("%s changed this group's settings to allow only admins to add others to this group", who))
		case types.GroupMemberAddModeAllMember:
			quiet(fmt.Sprintf("%s changed this group's settings to allow all members to add others to this group", who))
		}
	}
	if v.Delete != nil && v.Delete.Deleted {
		quiet(fmt.Sprintf("%s deleted this group", who))
	}
	if v.NewInviteLink != nil && actor != "" {
		quiet(fmt.Sprintf("%s reset this group's invite link", actor))
	}

	if len(v.Join) > 0 {
		names := acc.joinedNamesList(v.Join)
		// Only a join that brings you in moves the chat.
		moves := includesMe(v.Join)
		switch {
		case actor != "" && actor != names:
			lines = append(lines, groupChangeLine{Text: fmt.Sprintf("%s added %s", actor, names), MovesChat: moves})
		case v.JoinReason == "invite":
			lines = append(lines, groupChangeLine{Text: fmt.Sprintf("%s joined using this group's invite link", names), MovesChat: moves})
		default:
			lines = append(lines, groupChangeLine{Text: fmt.Sprintf("%s joined", names), MovesChat: moves})
		}
	}

	if len(v.Leave) > 0 {
		names := acc.joinedNamesList(v.Leave)
		if actor != "" && actor != names {
			// Being removed yourself moves the chat; anyone else, or
			// leaving (even you), doesn't.
			moves := includesMe(v.Leave) && !acc.isParticipantMe(types.GroupParticipant{JID: *v.Sender})
			lines = append(lines, groupChangeLine{Text: fmt.Sprintf("%s removed %s", actor, names), MovesChat: moves})
		} else {
			quiet(fmt.Sprintf("%s left", names))
		}
	}

	if len(v.Promote) > 0 {
		names := acc.joinedNamesList(v.Promote)
		if actor != "" {
			quiet(fmt.Sprintf("%s made %s a group admin", actor, names))
		} else {
			quiet(fmt.Sprintf("%s is now a group admin", names))
		}
	}

	for _, j := range v.Modify {
		quiet(fmt.Sprintf("%s changed their phone number", acc.nameToken(j)))
	}

	if len(v.Demote) > 0 {
		names := acc.joinedNamesList(v.Demote)
		if actor != "" {
			quiet(fmt.Sprintf("%s removed %s as group admin", actor, names))
		} else {
			quiet(fmt.Sprintf("%s is no longer a group admin", names))
		}
	}

	return lines
}

const historySyncStallThreshold = 8 * time.Second

var lastStallDump atomic.Int64

func dumpGoroutinesForStall(reason string) {
	now := time.Now()
	if last := lastStallDump.Load(); last != 0 && now.Sub(time.Unix(0, last)) < time.Minute {
		return
	}
	lastStallDump.Store(now.UnixNano())
	path := filepath.Join(logsDir(), "stall-"+now.Format("20060102-150405")+".txt")
	f, err := os.Create(path)
	if err != nil {
		log.Println("Failed to write stall dump:", err)
		return
	}
	defer f.Close()
	fmt.Fprintf(f, "%s\n%s\n\n", now.Format(time.RFC3339Nano), reason)
	buf := make([]byte, 1<<20)
	for {
		n := runtime.Stack(buf, true)
		if n < len(buf) {
			buf = buf[:n]
			break
		}
		buf = make([]byte, 2*len(buf))
	}
	f.Write(buf)
	log.Printf("Stall detected (%s); goroutine dump written to %s", reason, path)
}

const historySyncRefreshDelay = 10 * time.Second

func (acc *Account) scheduleHistorySyncRefresh() {
	acc.historySyncRefreshMu.Lock()
	defer acc.historySyncRefreshMu.Unlock()
	if acc.historySyncRefreshTimer != nil {
		acc.historySyncRefreshTimer.Stop()
	}
	acc.historySyncRefreshTimer = time.AfterFunc(historySyncRefreshDelay, func() {
		if acc.historySyncPending.Load() > 0 {
			acc.scheduleHistorySyncRefresh()
			return
		}
		start := time.Now()
		acc.UpdateAllChatSnippets()
		acc.populateChatsFromStore()
		log.Printf("History sync refresh (snippets, contacts, groups) took %s", time.Since(start).Round(time.Millisecond))
		acc.lookUpBusinessNames()
	})
}

const historySyncCheckpointDelay = 5 * time.Second

func (acc *Account) scheduleHistorySyncCheckpoint() {
	acc.historySyncCheckpointMu.Lock()
	acc.historySyncCheckpointGen++
	gen := acc.historySyncCheckpointGen
	acc.historySyncCheckpointMu.Unlock()
	acc.runHistorySyncCheckpointAfter(gen, 0)
}

func (acc *Account) runHistorySyncCheckpointAfter(gen, attempt int) {
	time.AfterFunc(historySyncCheckpointDelay, func() {
		acc.historySyncCheckpointMu.Lock()
		stale := gen != acc.historySyncCheckpointGen
		acc.historySyncCheckpointMu.Unlock()
		if stale {
			return
		}
		if acc.historySyncPending.Load() > 0 {
			acc.runHistorySyncCheckpointAfter(gen, attempt)
			return
		}
		acc.historySyncMutex.Lock()
		defer acc.historySyncMutex.Unlock()
		start := time.Now()
		var busy, walFrames, checkpointed int
		if err := acc.db.QueryRow("PRAGMA wal_checkpoint(TRUNCATE)").Scan(&busy, &walFrames, &checkpointed); err != nil {
			log.Println("History sync WAL checkpoint failed:", err)
			return
		}
		log.Printf("History sync WAL checkpoint: busy=%d took=%s", busy, time.Since(start).Round(time.Millisecond))
		if busy != 0 && attempt < 3 {
			acc.runHistorySyncCheckpointAfter(gen, attempt+1)
		}
	})
}

func (acc *Account) appStateChatJID(raw string) string {
	j, err := types.ParseJID(raw)
	if err != nil {
		return raw
	}
	return acc.preferPN(j).String()
}

func (acc *Account) preferPN(jid types.JID) types.JID {
	jid = jid.ToNonAD()
	if jid.Server == types.HiddenUserServer && acc.wa() != nil {
		if pn, err := acc.wa().Store.LIDs.GetPNForLID(context.Background(), jid); err == nil && !pn.IsEmpty() {
			return pn.ToNonAD()
		}
	}
	return jid
}

func (acc *Account) historyStubActor(webMsg *waWeb.WebMessageInfo) types.JID {
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
	return acc.preferPN(jid)
}

func (acc *Account) historyStubJIDs(params []string) []types.JID {
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
			jids = append(jids, acc.preferPN(jid))
		}
	}
	return jids
}

func (acc *Account) historyStubText(webMsg *waWeb.WebMessageInfo) string {
	actor := ""
	if a := acc.historyStubActor(webMsg); !a.IsEmpty() {
		actor = acc.nameToken(a)
	}
	params := webMsg.GetMessageStubParameters()
	names := acc.joinedNamesList(acc.historyStubJIDs(params))
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
	case waWeb.WebMessageInfo_POLL_OPTIONS_REMOVED:
		if actor != "" {
			return fmt.Sprintf("%s removed options from a poll", actor)
		}
		return "Options were removed from a poll"
	case waWeb.WebMessageInfo_POLL_ADDED_OPTION_REMOVED:
		if actor != "" {
			return fmt.Sprintf("%s removed an option that was added to a poll", actor)
		}
		return "An option added to a poll was removed"
	}
	return ""
}

func (acc *Account) messageHistoryNoticeText(notice *waE2E.MessageHistoryNotice, sender types.JID) string {
	if notice == nil {
		return ""
	}
	var receiverJIDs []types.JID
	if meta := notice.GetMessageHistoryMetadata(); meta != nil {
		for _, r := range meta.GetHistoryReceivers() {
			if jid, err := types.ParseJID(r); err == nil && jid.User != "" {
				receiverJIDs = append(receiverJIDs, acc.preferPN(jid))
			}
		}
	}
	receivers := acc.joinedNamesList(receiverJIDs)
	actor := ""
	if !sender.IsEmpty() {
		actor = acc.nameToken(acc.preferPN(sender))
	}
	switch {
	case actor != "" && receivers != "":
		return fmt.Sprintf("%s shared recent message history with %s", actor, receivers)
	case receivers != "":
		return fmt.Sprintf("Recent message history was shared with %s", receivers)
	case actor != "":
		return fmt.Sprintf("%s shared recent message history", actor)
	default:
		return "Recent message history was shared"
	}
}

// insertGroupSystemMessage stores a system line and makes it the chat's
// preview. movesChat says whether the line also raises the chat's time and so
// takes it to the top of the list. WA Web (WAWebMsgDataUtils
// eventTypeFromMsgType) only moves a chat for a few of these lines: a new
// group name, the group being created, you being added or removed, linking to
// a community, and pins and history notices. Everything else (other people
// joining, leaving or being removed, admin changes, description, icon,
// settings, disappearing messages, chat privacy) gets EventType.IGNORE: the
// chat keeps its place, though the line still becomes its preview.
func (acc *Account) insertGroupSystemMessage(jid, id, text string, ts time.Time, movesChat bool) {
	if text == "" {
		return
	}
	if isIgnoredChatJID(jid) {
		return
	}
	tsUnix := ts.Unix()
	err := acc.saveMessage(StoredMessage{
		ID: id, JID: jid, Text: text, Timestamp: tsUnix, Status: "sent", MediaType: "system",
	})
	if err != nil {
		fmt.Println("Error inserting group system message:", err)
		return
	}
	newTime := "timestamp"
	if movesChat {
		newTime = "MAX(timestamp, excluded.timestamp)"
	}
	acc.db.Exec(`INSERT INTO chats (jid, timestamp, unread_count, preview_snippet, preview_sender, preview_from_me) VALUES (?, ?, 0, ?, '', 0)
		ON CONFLICT(jid) DO UPDATE SET
			timestamp=`+newTime+`,
			preview_snippet=excluded.preview_snippet,
			preview_sender='',
			preview_from_me=0,
			preview_is_reaction=0,
			preview_reaction_emoji=''`,
		jid, tsUnix, text)
	var chatName string
	acc.db.QueryRow(`SELECT name FROM chats WHERE jid = ?`, jid).Scan(&chatName)

	acc.broadcastChats()
	acc.broadcastEvent("message", map[string]interface{}{
		"jid":       jid,
		"from":      "",
		"id":        id,
		"text":      acc.renderNames(text, nil),
		"from_me":   false,
		"timestamp": tsUnix,
		"has_media": false,
		"mediaType": "system",
		"chatName":  chatName,
	})
}

func (acc *Account) populateChatsFromStore() {
	acc.populateChatsMu.Lock()
	if acc.populateChatsRunning {
		acc.populateChatsPending = true
		acc.populateChatsMu.Unlock()
		return
	}
	acc.populateChatsRunning = true
	acc.populateChatsMu.Unlock()

	for {
		acc.populateChatsFromStoreOnce()
		acc.broadcastChats()

		acc.populateChatsMu.Lock()
		if acc.populateChatsPending {
			acc.populateChatsPending = false
			acc.populateChatsMu.Unlock()
			continue
		}
		acc.populateChatsRunning = false
		acc.populateChatsMu.Unlock()
		break
	}
}

func (acc *Account) populateChatsFromStoreOnce() {
	if acc.wa() == nil || acc.wa().Store == nil || acc.wa().Store.Contacts == nil {
		return
	}

	allContacts, err := acc.wa().Store.Contacts.GetAllContacts(context.Background())
	if err != nil {
		fmt.Println("Error getting contacts from store:", err)
		return
	}

	type mergedContact struct {
		name       string
		nameRank   int
		phone      string
		isBusiness bool
	}
	merged := make(map[string]*mergedContact, len(allContacts))
	var order []string
	var resolvedLIDJIDs []string
	for jid, contact := range allContacts {
		targetJID := jid.ToNonAD()
		if targetJID.Server == "lid" {
			if pn, pnErr := acc.wa().Store.LIDs.GetPNForLID(context.Background(), targetJID); pnErr == nil && !pn.IsEmpty() {
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
	acc.writeBatch(contactWrites)
	acc.invalidateDisplayNames()
	acc.healStaleUnreadCounts()

	groups, err := acc.serverGroupList()
	var joinedGroupsSnapshot map[string]bool
	if err == nil {
		var groupWrites []func(tx sqlExecer)
		joined := make(map[string]bool, len(groups))
		joinedGroupsSnapshot = joined
		for _, g := range groups {
			joined[g.JID.ToNonAD().String()] = true
			groupWrites = append(groupWrites, acc.groupInfoWrite(g))
		}
		acc.writeBatch(groupWrites)
		acc.setJoinedGroups(joined)
	}

	rows, err := acc.db.Query(`SELECT jid FROM chats WHERE name IS NULL OR name = '' OR name LIKE '+%' OR jid LIKE '%@lid'`)
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
			if pj.Server == types.GroupServer {
				if joinedGroupsSnapshot == nil {
					acc.requestGroupMetadata(pj)
				}
				continue
			}
			resName := acc.resolveDisplayName(pj)
			if resName != "" && resName != jStr && !strings.HasPrefix(resName, "+") {
				acc.db.Exec(`UPDATE chats SET name = ? WHERE jid = ?`, resName, jStr)
			}
		}
	}

	rowsBroad, errBroad := acc.db.Query(`SELECT DISTINCT sender_jid FROM messages
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
				pn, err := acc.wa().Store.LIDs.GetPNForLID(context.Background(), target)
				if err != nil || pn.IsEmpty() {
					continue
				}
				target = pn.ToNonAD()
			}
			if target.Server != types.DefaultUserServer {
				continue
			}
			acc.db.Exec(`INSERT INTO chats (jid, timestamp)
				SELECT ?, MAX(timestamp) FROM messages
				WHERE jid LIKE '%@broadcast' AND from_me = 0 AND sender_jid = ?
				ON CONFLICT(jid) DO UPDATE SET timestamp = MAX(chats.timestamp, excluded.timestamp)`,
				target.String(), s)
			acc.db.Exec(`UPDATE messages SET jid = ?, sender_jid = ?
				WHERE jid LIKE '%@broadcast' AND from_me = 0 AND sender_jid = ?`,
				target.String(), target.String(), s)
			acc.UpdateChatSnippet(target.String())
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
	// the group couldn't be found in whatSnap at all. Being in the group is
	// reason enough to list it (at the bottom, having no activity to sort
	// by). The same can't be done for people: the phone also sends an empty
	// entry for contacts who were never chatted with, and those must go.
	const emptyChat = `timestamp = 0 AND COALESCE(is_community, 0) = 0 AND COALESCE(is_channel, 0) = 0 AND jid NOT IN (SELECT DISTINCT jid FROM messages) AND jid NOT IN (SELECT DISTINCT jid FROM chats WHERE pinned = 1 OR muted = 1 OR archived = 1)`
	acc.db.Exec("DELETE FROM chats WHERE jid NOT LIKE '%@g.us' AND " + emptyChat)
	if joinedGroupsSnapshot != nil {
		if emptyGroups, qErr := acc.db.Query("SELECT jid FROM chats WHERE jid LIKE '%@g.us' AND " + emptyChat); qErr == nil {
			var left []string
			for emptyGroups.Next() {
				var j string
				if emptyGroups.Scan(&j) == nil && !joinedGroupsSnapshot[j] {
					left = append(left, j)
				}
			}
			emptyGroups.Close()
			for _, j := range left {
				acc.db.Exec("DELETE FROM chats WHERE jid = ?", j)
			}
		}
	}
	acc.db.Exec("DELETE FROM chats WHERE jid = 'status@broadcast' OR (jid LIKE '%@newsletter' AND COALESCE(is_channel, 0) = 0) OR jid LIKE '%@broadcast'")
	go acc.syncFollowedChannelsThrottled()
}

func (acc *Account) handleIncomingStatus(v *events.Message) {
	text := acc.extractText(v.Message)
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
	if err := acc.saveMessage(StoredMessage{
		ID: v.Info.ID, JID: "status@broadcast", SenderJID: senderJID.String(),
		FromMe: v.Info.IsFromMe, Text: text, Timestamp: ts, Raw: rawBytes, Status: status,
		MediaType: mediaType, MediaMimetype: mimetype, MediaFilename: filename, MediaSize: mediaSize,
	}); err != nil {
		log.Println("Error saving status update:", err)
		return
	}
	go acc.purgeExpiredStatuses()
	background, textColor := textStatusColors(rawBytes)
	acc.broadcastEvent("status_update", map[string]interface{}{
		"id": v.Info.ID, "posterJid": senderJID.String(), "fromMe": v.Info.IsFromMe,
		"text": text, "mediaType": mediaType, "timestamp": ts,
		"background": background, "textColor": textColor,
	})
}

func (acc *Account) requestAppStateFatalRecovery(name appstate.WAPatchName) {
	acc.appStateResyncMu.Lock()
	if wait := 5*time.Minute - time.Since(acc.lastFatalRecoveryRequest[name]); wait > 0 {
		acc.appStateResyncMu.Unlock()
		log.Printf("Fatal recovery for %s: skipping, still within 5m of the last request (%s left)", name, wait.Round(time.Second))
		return
	}
	acc.lastFatalRecoveryRequest[name] = time.Now()
	acc.appStateResyncMu.Unlock()

	if acc.wa() == nil {
		return
	}
	log.Printf("Requesting fatal recovery snapshot for %s from our primary device.", name)
	req := whatsmeow.BuildAppStateRecoveryRequest(name)
	if _, err := acc.wa().SendPeerMessage(context.Background(), req); err != nil {
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

func (acc *Account) retryAppStateAfterLock(name appstate.WAPatchName, attempt int) {
	if attempt >= 5 {
		log.Printf("App state %s: still hitting database locks after %d retries, giving up until next connection", name, attempt)
		return
	}
	delay := time.Duration(15*(attempt+1)) * time.Second
	log.Printf("App state %s hit a database lock; retrying in %s", name, delay)
	time.AfterFunc(delay, func() {
		if acc.wa() == nil {
			return
		}
		err := acc.wa().FetchAppState(context.Background(), name, false, false)
		switch {
		case err == nil:
			log.Printf("App state %s resync succeeded after database lock", name)
			acc.appStateResyncMu.Lock()
			acc.appStateResyncCount[name] = 0
			acc.appStateResyncMu.Unlock()
			acc.populateChatsFromStore()
			acc.broadcastChats()
		case isDatabaseLocked(err):
			acc.retryAppStateAfterLock(name, attempt+1)
		default:
			log.Printf("App state %s retry after database lock failed: %v", name, err)
		}
	})
}

func (acc *Account) handleAppStateSyncError(v *events.AppStateSyncError) {
	acc.appStateResyncMu.Lock()
	count := acc.appStateResyncCount[v.Name]
	if count >= 3 {
		acc.appStateResyncMu.Unlock()
		log.Printf("AppStateSyncError for %s: giving up after %d failed attempts. Will retry on next connection.", v.Name, count)
		return
	}
	lastTime := acc.lastAppStateResync[v.Name]
	if wait := 60*time.Second - time.Since(lastTime); wait > 0 {
		acc.appStateResyncMu.Unlock()
		log.Printf("AppStateSyncError for %s: skipping resync, still within 60s of the last attempt (%s left): %v", v.Name, wait.Round(time.Second), v.Error)
		return
	}
	acc.appStateResyncCount[v.Name] = count + 1
	acc.lastAppStateResync[v.Name] = time.Now()
	acc.appStateResyncMu.Unlock()

	go func() {
		log.Printf("AppStateSyncError for %s (attempt %d/3): %v.", v.Name, count+1, v.Error)
		if acc.wa() == nil {
			return
		}
		if v.Error != nil {
			errMsg := v.Error.Error()
			if strings.Contains(errMsg, "mismatching LTHash") || strings.Contains(errMsg, "failed to verify snapshot") || strings.Contains(errMsg, "terminal patch") {
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
				if resyncErr := acc.wa().FetchAppState(context.Background(), v.Name, true, false); resyncErr != nil {
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
						acc.requestAppStateFatalRecovery(v.Name)
					}
				} else {
					log.Printf("Full resync for %s succeeded", v.Name)
					acc.appStateResyncMu.Lock()
					acc.appStateResyncCount[v.Name] = 0
					acc.appStateResyncMu.Unlock()
				}
				return
			}
		}
		err := acc.wa().FetchAppState(context.Background(), v.Name, false, false)
		if err != nil {
			if errors.Is(err, whatsmeow.ErrIQRateOverLimit) || strings.Contains(err.Error(), "429") || strings.Contains(err.Error(), "rate-overlimit") {
				log.Printf("Rate limit 429 during resync of %s. Scheduling retry in 90s...", v.Name)
				time.AfterFunc(90*time.Second, func() {
					acc.appStateResyncMu.Lock()
					acc.lastAppStateResync[v.Name] = time.Now()
					acc.appStateResyncMu.Unlock()
					if acc.wa() != nil {
						if retryErr := acc.wa().FetchAppState(context.Background(), v.Name, false, false); retryErr != nil {
							log.Printf("Delayed retry for %s failed: %v", v.Name, retryErr)
						} else {
							log.Printf("Delayed resync for %s succeeded", v.Name)
							acc.appStateResyncMu.Lock()
							acc.appStateResyncCount[v.Name] = 0
							acc.appStateResyncMu.Unlock()
						}
					}
				})
			} else if isDatabaseLocked(err) {
				// Typically a full history sync hammering the database right
				// after pairing. Retrying straight away just loses the same
				// race again; wait for things to calm down instead.
				// critical_unblock_low carries saved contact names, so giving
				// up here leaves every chat showing push names instead.
				acc.retryAppStateAfterLock(v.Name, 0)
			} else {
				log.Printf("Failed to resync app state %s: %v", v.Name, err)
			}
		} else {
			log.Printf("App state %s resync succeeded", v.Name)
			acc.appStateResyncMu.Lock()
			acc.appStateResyncCount[v.Name] = 0
			acc.appStateResyncMu.Unlock()
		}
	}()
}
func (acc *Account) isParticipantMe(p types.GroupParticipant) bool {
	if acc.wa() == nil || acc.wa().Store == nil || acc.wa().Store.ID == nil {
		return false
	}
	myJID := acc.wa().Store.ID.ToNonAD()
	myLID := acc.wa().Store.GetLID().ToNonAD()
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
		if pn, err := acc.wa().Store.LIDs.GetPNForLID(context.Background(), p.JID); err == nil && !pn.IsEmpty() {
			return isMe(pn)
		}
	}
	return false
}

func (acc *Account) isViewerAdmin(info *types.GroupInfo) bool {
	if info == nil {
		return false
	}
	for _, p := range info.Participants {
		if acc.isParticipantMe(p) && (p.IsAdmin || p.IsSuperAdmin) {
			return true
		}
	}
	return false
}

func videoText(video *waE2E.VideoMessage) string {
	text := "[Video]"
	if video.GetGifPlayback() {
		text += " GIF"
		if label := strings.TrimSpace(video.GetAccessibilityLabel()); label != "" {
			text += ": " + strings.TrimSuffix(label, ".")
		}
		if caption := video.GetCaption(); caption != "" {
			text += ". " + caption
		}
		return text
	}
	if caption := video.GetCaption(); caption != "" {
		text += " " + caption
	}
	return text
}

// applyPinChange stores a pin or unpin made at changedAt (unix seconds) unless
// a newer one is already stored. The pin time is what orders the pinned chats,
// newest pin first, as WA Web orders them by chat.pin.
func (acc *Account) applyPinChange(jid string, pinned bool, changedAt int64) {
	pinnedAt := int64(0)
	if pinned {
		pinnedAt = max(changedAt, 1)
	}
	acc.db.Exec(`UPDATE chats SET pinned = ?, pinned_at = ?, pin_changed_at = ?,
			archived = CASE WHEN ? = 1 THEN 0 ELSE archived END
		WHERE jid = ? AND COALESCE(pin_changed_at, 0) <= ?`,
		boolToInt(pinned), pinnedAt, changedAt, boolToInt(pinned), jid, changedAt)
	if pinned {
		acc.db.Exec(`UPDATE archive_actions SET archived = 0, changed_at = MAX(changed_at, ?) WHERE jid = ?`, changedAt, jid)
	}
	acc.broadcastChats()
}
