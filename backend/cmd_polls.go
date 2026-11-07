package main

import (
	"context"
	"encoding/json"
	"errors"
	"net"
	"strings"
	"time"
	"unicode/utf8"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
)

const (
	pollMaxQuestionLen = 255
	pollMaxOptionLen   = 100
	pollMinOptions     = 2
	pollMaxOptions     = 12
)

func (acc *Account) buildPoll(question string, options []string, multipleAnswers bool, correctAnswer string) (*waE2E.Message, error) {
	question = strings.TrimSpace(question)
	if question == "" {
		return nil, errors.New("the poll needs a question")
	}
	if utf8.RuneCountInString(question) > pollMaxQuestionLen {
		return nil, errors.New("the poll question is too long")
	}
	cleaned := make([]string, 0, len(options))
	seen := map[string]bool{}
	for _, opt := range options {
		opt = strings.TrimSpace(opt)
		if opt == "" {
			continue
		}
		if utf8.RuneCountInString(opt) > pollMaxOptionLen {
			return nil, errors.New("a poll option is too long")
		}
		if seen[opt] {
			return nil, errors.New("poll options must all be different")
		}
		seen[opt] = true
		cleaned = append(cleaned, opt)
	}
	if len(cleaned) < pollMinOptions {
		return nil, errors.New("a poll needs at least two options")
	}
	if len(cleaned) > pollMaxOptions {
		return nil, errors.New("a poll can have at most twelve options")
	}
	if correctAnswer = strings.TrimSpace(correctAnswer); correctAnswer != "" {
		for i, opt := range cleaned {
			if opt == correctAnswer {
				return acc.wa().BuildQuizPollCreation(question, cleaned, i)
			}
		}
		return nil, errors.New("the quiz's correct answer isn't one of its options")
	}
	selectable := 0
	if !multipleAnswers {
		selectable = 1
	}
	msg := acc.wa().BuildPollCreation(question, cleaned, selectable)
	if !multipleAnswers {
		msg.PollCreationMessageV3 = msg.PollCreationMessage
		msg.PollCreationMessage = nil
	}
	return msg, nil
}

func (acc *Account) handleSendPoll(conn net.Conn, raw json.RawMessage) {
	var data SendPollData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		acc.sendEvent(conn, "error", map[string]string{"message": "invalid jid"})
		return
	}
	msg, err := acc.buildPoll(data.Question, data.Options, data.MultipleAnswers, data.CorrectAnswer)
	if err != nil {
		acc.sendEvent(conn, "error", map[string]string{"message": err.Error()})
		return
	}
	go func() {
		ctxInfo, quotedID, quotedText, quotedSender := acc.buildAttachmentContext(data.ReplyToID, data.ReplySender)
		pollCreation(msg).ContextInfo = ctxInfo
		displayText := acc.extractTextRaw(msg)
		id := acc.wa().GenerateMessageID()
		acc.startSendingBuiltMessage(id, jid, msg, displayText, quotedID, quotedText, quotedSender)
		acc.finishSendingBuiltMessage(conn, id, jid, msg, displayText)
	}()
}

func (acc *Account) startSendingBuiltMessage(id string, jid types.JID, msg *waE2E.Message, displayText, quotedID, quotedText, quotedSender string) {
	senderJID := ""
	if acc.wa().Store.ID != nil {
		senderJID = acc.wa().Store.ID.ToNonAD().String()
	}
	ts := time.Now().Unix()
	rawBytes, _ := proto.Marshal(msg)
	acc.saveMessage(StoredMessage{
		ID: id, JID: jid.String(), SenderJID: senderJID, FromMe: true,
		Text: displayText, Timestamp: ts, Status: "pending", Raw: rawBytes,
		QuotedID: quotedID, QuotedText: quotedText, QuotedSender: quotedSender,
	})
	acc.db.Exec(`INSERT INTO chats (jid, timestamp, preview_snippet, preview_sender, preview_from_me) VALUES (?, ?, ?, ?, 1)
		ON CONFLICT(jid) DO UPDATE SET
			timestamp=MAX(timestamp, excluded.timestamp),
			preview_snippet=excluded.preview_snippet,
			preview_sender=excluded.preview_sender,
			preview_from_me=1,
			preview_is_reaction=0,
			preview_reaction_emoji=''`, jid.String(), ts, displayText, senderJID)
	acc.broadcastEvent("message", map[string]interface{}{
		"jid": jid.String(), "from": "me", "id": id, "text": displayText,
		"from_me":      true,
		"timestamp":    ts,
		"has_media":    false,
		"status":       "pending",
		"quotedId":     quotedID,
		"quotedText":   quotedText,
		"quotedSender": quotedSender,
		"pollResults":  newPollResults(msg),
	})
	acc.broadcastChats()
}

func (acc *Account) finishSendingBuiltMessage(conn net.Conn, id string, jid types.JID, msg *waE2E.Message, displayText string) {
	// The same in-flight claim the text path uses, so a manual retry and the
	// background sweep can never send this twice at once.
	if !acc.beginSendAttempt(id) {
		return
	}
	defer acc.endSendAttempt(id)
	var quotedID, quotedText, quotedSender string
	acc.db.QueryRow(`SELECT COALESCE(quoted_id,''), COALESCE(quoted_text,''), COALESCE(quoted_sender,'') FROM messages WHERE id = ?`, id).
		Scan(&quotedID, &quotedText, &quotedSender)
	msg = acc.applyDisappearingTimer(jid, msg)
	resp, err := acc.wa().SendMessage(context.Background(), jid, msg, whatsmeow.SendRequestExtra{ID: id, CheckPermissions: true})
	if err != nil {
		acc.db.Exec(`UPDATE messages SET status = 'failed' WHERE id = ?`, id)
		if conn != nil {
			acc.sendEvent(conn, "error", map[string]string{"message": sendErrorText(err)})
		}
		acc.broadcastEvent("message", map[string]interface{}{
			"jid": jid.String(), "from": "me", "id": id, "text": displayText,
			"from_me": true, "timestamp": time.Now().Unix(), "has_media": false, "status": "failed",
			"quotedId": quotedID, "quotedText": quotedText, "quotedSender": quotedSender,
			"pollResults": acc.pollResultsForMessage(id),
		})
		return
	}
	sentTs := resp.Timestamp.Unix()
	acc.db.Exec(`UPDATE messages SET status='sent', timestamp=?, sent_at=? WHERE id=?`, sentTs, sentTs, id)
	noteMessageExpiry(acc.db, id, sentTs, msg)
	acc.noteChannelPostSent(jid, id, resp)
	acc.broadcastEvent("message", map[string]interface{}{
		"jid": jid.String(), "from": "me", "id": id, "text": displayText,
		"from_me":      true,
		"timestamp":    sentTs,
		"has_media":    false,
		"status":       "sent",
		"sentHere":     true,
		"quotedId":     quotedID,
		"quotedText":   quotedText,
		"quotedSender": quotedSender,
		"pollResults":  acc.pollResultsForMessage(id),
	})
	acc.broadcastChats()
}
