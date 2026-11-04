package main

import (
	"context"
	"encoding/json"
	"net"
	"strconv"
	"strings"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
)

const (
	optionReply = "reply"
	optionURL   = "url"
	optionCall  = "call"
	optionCopy  = "copy"
	optionPhone = "phone"
)

type messageOption struct {
	Kind        string `json:"kind"`
	Label       string `json:"label"`
	Section     string `json:"section,omitempty"`
	Description string `json:"description,omitempty"`
	Value       string `json:"value,omitempty"`

	// What a reply needs; never sent to the UI.
	replyType string
	id        string
	index     uint32
}

func interactiveOf(msg *waE2E.Message) *waE2E.InteractiveMessage {
	if im := msg.GetInteractiveMessage(); im != nil {
		return im
	}
	return msg.GetTemplateMessage().GetInteractiveMessageTemplate()
}

func nativeFlowOption(name, paramsJSON string, index int) (messageOption, bool) {
	var params map[string]any
	_ = json.Unmarshal([]byte(paramsJSON), &params)
	str := func(key string) string {
		s, _ := params[key].(string)
		return strings.TrimSpace(s)
	}
	label := str("display_text")
	if label == "" {
		label = str("title")
	}
	switch name {
	case "quick_reply":
		if label == "" {
			return messageOption{}, false
		}
		return messageOption{Kind: optionReply, Label: label, replyType: "template", id: str("id"), index: uint32(index)}, true
	case "cta_url":
		if url := str("url"); url != "" {
			return messageOption{Kind: optionURL, Label: cmpOr(label, url), Value: url}, true
		}
	case "cta_call":
		if number := cmpOr(str("phone_number"), str("phoneNumber")); number != "" {
			return messageOption{Kind: optionCall, Label: cmpOr(label, number), Value: number}, true
		}
	case "cta_copy":
		if code := str("copy_code"); code != "" {
			return messageOption{Kind: optionCopy, Label: cmpOr(label, "Copy code"), Value: code}, true
		}
	}
	if label == "" {
		return messageOption{}, false
	}
	return messageOption{Kind: optionPhone, Label: label}, true
}

func cmpOr(a, b string) string {
	if a != "" {
		return a
	}
	return b
}

func messageOptions(msg *waE2E.Message) (heading string, options []messageOption) {
	msg = unwrapMessage(msg)
	if tpl := msg.GetTemplateMessage(); tpl != nil && tpl.GetInteractiveMessageTemplate() == nil {
		hydrated := tpl.GetHydratedTemplate()
		if hydrated == nil {
			hydrated = tpl.GetHydratedFourRowTemplate()
		}
		for i, button := range hydrated.GetHydratedButtons() {
			index := uint32(i)
			if button.Index != nil {
				index = button.GetIndex()
			}
			switch {
			case button.GetQuickReplyButton() != nil:
				qr := button.GetQuickReplyButton()
				if label := strings.TrimSpace(qr.GetDisplayText()); label != "" {
					options = append(options, messageOption{Kind: optionReply, Label: label, replyType: "template", id: qr.GetID(), index: index})
				}
			case button.GetUrlButton() != nil:
				u := button.GetUrlButton()
				if u.GetURL() != "" {
					options = append(options, messageOption{Kind: optionURL, Label: cmpOr(strings.TrimSpace(u.GetDisplayText()), u.GetURL()), Value: u.GetURL()})
				}
			case button.GetCallButton() != nil:
				c := button.GetCallButton()
				if c.GetPhoneNumber() != "" {
					options = append(options, messageOption{Kind: optionCall, Label: cmpOr(strings.TrimSpace(c.GetDisplayText()), c.GetPhoneNumber()), Value: c.GetPhoneNumber()})
				}
			}
		}
		return "", options
	}
	if im := interactiveOf(msg); im != nil {
		for i, button := range im.GetNativeFlowMessage().GetButtons() {
			if option, ok := nativeFlowOption(button.GetName(), button.GetButtonParamsJSON(), i); ok {
				options = append(options, option)
			}
		}
		return "", options
	}
	if bm := msg.GetButtonsMessage(); bm != nil {
		for i, button := range bm.GetButtons() {
			if info := button.GetNativeFlowInfo(); info != nil {
				if option, ok := nativeFlowOption(info.GetName(), info.GetParamsJSON(), i); ok {
					options = append(options, option)
				}
				continue
			}
			if label := strings.TrimSpace(button.GetButtonText().GetDisplayText()); label != "" {
				options = append(options, messageOption{Kind: optionReply, Label: label, replyType: "buttons", id: button.GetButtonID()})
			}
		}
		return "", options
	}
	if lm := msg.GetListMessage(); lm != nil {
		for _, section := range lm.GetSections() {
			for _, row := range section.GetRows() {
				if label := strings.TrimSpace(row.GetTitle()); label != "" {
					options = append(options, messageOption{
						Kind: optionReply, Label: label, Section: strings.TrimSpace(section.GetTitle()),
						Description: strings.TrimSpace(row.GetDescription()), replyType: "list", id: row.GetRowID(),
					})
				}
			}
		}
		return strings.TrimSpace(lm.GetButtonText()), options
	}
	return "", nil
}

// optionLinks gives each link button as a line of text with its URL, so the
// app's normal link handling finds it: Enter opens a single link straight away
// in the browser, and several links get the usual chooser.
func optionLinks(msg *waE2E.Message) []string {
	_, options := messageOptions(msg)
	var lines []string
	for _, o := range options {
		if o.Kind != optionURL || o.Value == "" {
			continue
		}
		if o.Label == "" || o.Label == o.Value {
			lines = append(lines, o.Value)
		} else {
			lines = append(lines, o.Label+": "+o.Value)
		}
	}
	return lines
}

func optionsSummary(msg *waE2E.Message) string {
	heading, options := messageOptions(msg)
	if msg = unwrapMessage(msg); msg.GetListMessage() != nil {
		if len(options) == 0 {
			return ""
		}
		choices := "1 choice"
		if len(options) != 1 {
			choices = strconv.Itoa(len(options)) + " choices"
		}
		return "Menu: " + cmpOr(heading, "Options") + ", " + choices
	}
	var labels []string
	for _, o := range options {
		if o.Kind == optionReply || o.Kind == optionCopy || o.Kind == optionPhone {
			labels = append(labels, o.Label)
		}
	}
	if len(labels) == 0 {
		return ""
	}
	return "Buttons: " + strings.Join(labels, ", ")
}

type MessageOptionData struct {
	JID       string `json:"jid"`
	MessageID string `json:"message_id"`
	Index     int    `json:"index"`
}

func loadMessageProto(id string) (*waE2E.Message, bool) {
	var raw []byte
	if db.QueryRow(`SELECT raw FROM messages WHERE id = ?`, id).Scan(&raw) != nil || len(raw) == 0 {
		return nil, false
	}
	msg := &waE2E.Message{}
	if proto.Unmarshal(raw, msg) != nil {
		return nil, false
	}
	return msg, true
}

func handleGetMessageOptions(conn net.Conn, raw json.RawMessage) {
	var data MessageOptionData
	if json.Unmarshal(raw, &data) != nil {
		return
	}
	msg, ok := loadMessageProto(data.MessageID)
	if !ok {
		sendEvent(conn, "error", map[string]string{"message": "This message's buttons can't be opened here."})
		return
	}
	heading, options := messageOptions(msg)
	if len(options) == 0 {
		sendEvent(conn, "error", map[string]string{"message": "This message has no buttons."})
		return
	}
	sendEvent(conn, "message_options", map[string]interface{}{
		"jid": data.JID, "message_id": data.MessageID, "heading": heading, "options": options,
	})
}

func buildOptionReply(option messageOption, ctx *waE2E.ContextInfo) *waE2E.Message {
	switch option.replyType {
	case "buttons":
		return &waE2E.Message{ButtonsResponseMessage: &waE2E.ButtonsResponseMessage{
			SelectedButtonID: proto.String(option.id),
			Response:         &waE2E.ButtonsResponseMessage_SelectedDisplayText{SelectedDisplayText: option.Label},
			Type:             waE2E.ButtonsResponseMessage_DISPLAY_TEXT.Enum(),
			ContextInfo:      ctx,
		}}
	case "list":
		reply := &waE2E.ListResponseMessage{
			Title:             proto.String(option.Label),
			ListType:          waE2E.ListResponseMessage_SINGLE_SELECT.Enum(),
			SingleSelectReply: &waE2E.ListResponseMessage_SingleSelectReply{SelectedRowID: proto.String(option.id)},
		}
		if option.Description != "" {
			reply.Description = proto.String(option.Description)
		}
		return &waE2E.Message{ListResponseMessage: reply}
	default:
		return &waE2E.Message{TemplateButtonReplyMessage: &waE2E.TemplateButtonReplyMessage{
			SelectedID:          proto.String(option.id),
			SelectedDisplayText: proto.String(option.Label),
			SelectedIndex:       proto.Uint32(option.index),
			ContextInfo:         ctx,
		}}
	}
}

func handleSendMessageOption(conn net.Conn, raw json.RawMessage) {
	var data MessageOptionData
	if json.Unmarshal(raw, &data) != nil {
		return
	}
	go func() {
		fail := func(msg string) { sendEvent(conn, "error", map[string]string{"message": msg}) }
		msg, ok := loadMessageProto(data.MessageID)
		if !ok {
			fail("This message's buttons can't be used here.")
			return
		}
		_, options := messageOptions(msg)
		if data.Index < 0 || data.Index >= len(options) || options[data.Index].Kind != optionReply {
			fail("That option can't be sent from here.")
			return
		}
		option := options[data.Index]
		chat, err := types.ParseJID(data.JID)
		if err != nil {
			return
		}
		quoted, storedSender, preview, err := getMessageForQuote(data.MessageID)
		if err != nil {
			fail("This message's buttons can't be used here.")
			return
		}
		// A list reply goes out without quoting the list. WA Web's list
		// reply carries only the chat's own context (disappearing timer),
		// and quoting the list makes WhatsApp reject the stanza with 479.
		var ctx *waE2E.ContextInfo
		if option.replyType != "list" {
			ctx = &waE2E.ContextInfo{StanzaID: proto.String(data.MessageID), QuotedMessage: quoted}
			if participant := wireParticipantFor(data.MessageID, storedSender); participant != "" && participant != "me" {
				ctx.Participant = proto.String(participant)
			}
		}
		reply := applyDisappearingTimer(chat, buildOptionReply(option, ctx))
		id := wa().GenerateMessageID()
		resp, err := wa().SendMessage(context.Background(), chat, reply, whatsmeow.SendRequestExtra{ID: id})
		if err != nil {
			fail(sendErrorText(err))
			return
		}
		ts := resp.Timestamp.Unix()
		if ts <= 0 {
			ts = time.Now().Unix()
		}
		senderJID := ""
		if wa().Store.ID != nil {
			senderJID = wa().Store.ID.ToNonAD().String()
		}
		rawBytes, _ := proto.Marshal(reply)
		quotedSender := resolveQuotedSenderName(storedSender)
		saveMessage(StoredMessage{
			ID: id, JID: chat.String(), SenderJID: senderJID, FromMe: true,
			Text: option.Label, Timestamp: ts, Status: "sent", Raw: rawBytes,
			QuotedID: data.MessageID, QuotedText: preview, QuotedSender: quotedSender,
		})
		noteMessageExpiry(db, id, ts, reply)
		db.Exec(`INSERT INTO chats (jid, timestamp, preview_snippet, preview_sender, preview_from_me) VALUES (?, ?, ?, ?, 1)
			ON CONFLICT(jid) DO UPDATE SET
			timestamp=MAX(timestamp, excluded.timestamp),
			preview_snippet=excluded.preview_snippet,
			preview_sender=excluded.preview_sender,
			preview_from_me=1,
			preview_is_reaction=0,
			preview_reaction_emoji=''`,
			chat.String(), ts, option.Label, senderJID)
		broadcastEvent("message", map[string]interface{}{
			"jid": chat.String(), "from": "me", "id": id, "text": option.Label, "from_me": true,
			"timestamp": ts, "has_media": false, "status": "sent", "sentHere": true,
			"quotedId": data.MessageID, "quotedText": preview, "quotedSender": quotedSender,
		})
		broadcastChats()
		sendEvent(conn, "message_option_sent", map[string]interface{}{"label": option.Label})
	}()
}
