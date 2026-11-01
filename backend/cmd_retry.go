package main

import (
	"encoding/json"
	"errors"
	"log"
	"net"
	"os"
	"strconv"
	"strings"
	"sync"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
)

const (
	maxAutoRetries        = 6
	retryBaseDelay        = 3 * time.Second
	retryMaxDelay         = 2 * time.Minute
	retrySweepPeriod      = 3 * time.Second
	rateLimitedRetryDelay = time.Minute
)

var (
	retryMu       sync.Mutex
	sendInFlight  = map[string]bool{}
	retryAttempts = map[string]int{}
	retryNextAt   = map[string]time.Time{}
)

func init() {
	go func() {
		ticker := time.NewTicker(retrySweepPeriod)
		defer ticker.Stop()
		for range ticker.C {
			if wa() == nil || !wa().IsConnected() {
				continue
			}
			retryPendingTextMessages()
		}
	}()
}

// beginSendAttempt claims the single in-flight slot for a message ID so the
// reconnect sweep, the periodic sweep, and a manual "Retry send" can never
// dispatch the same message to the network at the same time.
func beginSendAttempt(id string) bool {
	retryMu.Lock()
	defer retryMu.Unlock()
	if sendInFlight[id] {
		return false
	}
	sendInFlight[id] = true
	return true
}

func endSendAttempt(id string) {
	retryMu.Lock()
	delete(sendInFlight, id)
	retryMu.Unlock()
}

func clearRetryState(id string) {
	retryMu.Lock()
	delete(retryAttempts, id)
	delete(retryNextAt, id)
	retryMu.Unlock()
}

func recordSendFailure(id, jidStr, text, quotedID, quotedSender string, err error) {
	retryMu.Lock()
	attempts := retryAttempts[id] + 1
	retryAttempts[id] = attempts
	if attempts >= maxAutoRetries {
		retryMu.Unlock()
		markSendFailed(id, jidStr, text, quotedID, quotedSender)
		return
	}
	delay := retryBaseDelay * time.Duration(uint64(1)<<uint(attempts-1))
	if delay > retryMaxDelay {
		delay = retryMaxDelay
	}
	if sendServerErrorCode(err) == sendErrorRateLimited {
		delay = max(delay, rateLimitedRetryDelay)
	}
	retryNextAt[id] = time.Now().Add(delay)
	retryMu.Unlock()
}

// markSendFailed stops retrying a text message and marks it "failed", for
// when retries are used up or the send can never succeed.
func markSendFailed(id, jidStr, text, quotedID, quotedSender string) {
	clearRetryState(id)
	db.Exec(`UPDATE messages SET status = 'failed' WHERE id = ?`, id)
	broadcastEvent("message", map[string]interface{}{
		"jid": jidStr, "from": "me", "id": id, "text": text, "from_me": true,
		"timestamp": time.Now().Unix(), "has_media": false, "status": "failed",
		"quotedId": quotedID, "quotedSender": quotedSender,
	})
}

// Error codes in the server's ack that refuse a message for good. WA Web
// marks such a message failed and never resends it on its own; resending
// into a limit is the kind of behaviour that gets an account flagged.
const (
	sendErrorRateLimited   = 429
	sendErrorNoTrustToken  = 463
	sendErrorNewChatCapped = 475
)

func sendServerErrorCode(err error) int {
	if !errors.Is(err, whatsmeow.ErrServerReturnedError) {
		return 0
	}
	msg := err.Error()
	code, convErr := strconv.Atoi(strings.TrimSpace(msg[strings.LastIndex(msg, " ")+1:]))
	if convErr != nil {
		return 0
	}
	return code
}

func sendRefusedForGood(err error) bool {
	if errors.Is(err, whatsmeow.ErrNoPermissionInGroup) || errors.Is(err, whatsmeow.ErrRecipientNotOnWhatsApp) {
		return true
	}
	switch sendServerErrorCode(err) {
	case sendErrorNoTrustToken, sendErrorNewChatCapped:
		return true
	}
	return false
}

const notOnWhatsAppText = "This number isn't on WhatsApp."

func sendErrorText(err error) string {
	if errors.Is(err, whatsmeow.ErrNoPermissionInGroup) {
		return "Only admins can send messages to this group."
	}
	if errors.Is(err, whatsmeow.ErrRecipientNotOnWhatsApp) {
		return notOnWhatsAppText
	}
	switch sendServerErrorCode(err) {
	case sendErrorRateLimited:
		return "WhatsApp says you're sending too fast. Wait a little before sending more."
	case sendErrorNewChatCapped:
		return "WhatsApp is limiting messages to new chats on this account for now. Try again later."
	case sendErrorNoTrustToken:
		return "WhatsApp won't deliver this message to this contact right now."
	}
	return err.Error()
}

func retryPendingTextMessages() {
	rows, err := db.Query(`SELECT id, jid, COALESCE(text,''), COALESCE(quoted_id,''), COALESCE(quoted_sender,'')
		FROM messages
		WHERE from_me = 1 AND status = 'pending' AND (media_type IS NULL OR media_type = '')
		ORDER BY timestamp ASC`)
	if err != nil {
		return
	}
	type pendingMsg struct{ id, jid, text, quotedID, quotedSender string }
	var pending []pendingMsg
	for rows.Next() {
		var p pendingMsg
		if scanErr := rows.Scan(&p.id, &p.jid, &p.text, &p.quotedID, &p.quotedSender); scanErr == nil {
			pending = append(pending, p)
		}
	}
	rows.Close()

	now := time.Now()
	for _, p := range pending {
		retryMu.Lock()
		inFlight := sendInFlight[p.id]
		next, hasNext := retryNextAt[p.id]
		retryMu.Unlock()
		if inFlight || (hasNext && now.Before(next)) {
			continue
		}
		log.Printf("Retrying pending message %s", p.id)
		if resendStoredMessage(nil, p.id, p.jid, p.text) {
			continue
		}
		sendPendingTextMessage(nil, p.id, p.jid, p.text, p.quotedID, p.quotedSender)
	}
}

func handleRetryMessage(conn net.Conn, raw json.RawMessage) {
	var data MessageRefData
	if err := json.Unmarshal(raw, &data); err != nil || data.MessageID == "" {
		return
	}
	go retryOneMessage(conn, data.MessageID)
}

func retryOneMessage(conn net.Conn, id string) {
	var jidStr, text, status, mediaType, mediaMimetype, mediaFilename, mediaPath, quotedID, quotedSender string
	err := db.QueryRow(`SELECT jid, COALESCE(text,''), COALESCE(status,''),
			COALESCE(media_type,''), COALESCE(media_mimetype,''), COALESCE(media_filename,''), COALESCE(media_path,''),
			COALESCE(quoted_id,''), COALESCE(quoted_sender,'')
		FROM messages WHERE id = ? AND from_me = 1`, id).
		Scan(&jidStr, &text, &status, &mediaType, &mediaMimetype, &mediaFilename, &mediaPath, &quotedID, &quotedSender)
	if err != nil {
		if conn != nil {
			sendEvent(conn, "error", map[string]string{"message": "Message not found for retry"})
		}
		return
	}
	if status != "pending" && status != "failed" {
		return
	}

	if mediaType == "" && resendStoredMessage(conn, id, jidStr, text) {
		return
	}

	if mediaType == "" {
		clearRetryState(id)
		if status != "pending" {
			db.Exec(`UPDATE messages SET status = 'pending' WHERE id = ?`, id)
		}
		sendPendingTextMessage(conn, id, jidStr, text, quotedID, quotedSender)
		return
	}

	if mediaPath == "" {
		if conn != nil {
			sendEvent(conn, "error", map[string]string{"message": "The original file is no longer available to retry."})
		}
		return
	}
	if _, statErr := os.Stat(mediaPath); statErr != nil {
		if conn != nil {
			sendEvent(conn, "error", map[string]string{"message": "The original file is no longer available to retry."})
		}
		return
	}
	jid, parseErr := types.ParseJID(jidStr)
	if parseErr != nil {
		return
	}

	if !beginSendAttempt(id) {
		return
	}
	defer endSendAttempt(id)

	db.Exec(`UPDATE messages SET status = 'uploading' WHERE id = ?`, id)
	broadcastEvent("message", map[string]interface{}{
		"jid": jidStr, "from": "me", "id": id, "text": text, "from_me": true,
		"timestamp": time.Now().Unix(), "has_media": true, "mediaType": mediaType,
		"mediaFilename": mediaFilename, "mediaPath": mediaPath, "status": "uploading",
	})

	ctxInfo, finalQuotedID, finalQuotedText, finalQuotedSender := buildAttachmentContext(quotedID, quotedSender)

	if mediaType == "voice" {
		duration := parseVoiceDurationSeconds(text)
		var waveform []byte
		db.QueryRow(`SELECT voice_waveform FROM messages WHERE id = ?`, id).Scan(&waveform)
		sendUploadedVoiceMessage(conn, id, jid, mediaPath, mediaMimetype, duration, waveform, ctxInfo, finalQuotedID, finalQuotedText, finalQuotedSender)
	} else {
		caption, seconds := attachmentFieldsFromDisplayText(mediaType, mediaFilename, text)
		sendUploadedAttachment(conn, id, jid, mediaType, mediaPath, mediaMimetype, mediaFilename, caption, seconds, nil, false, ctxInfo, finalQuotedID, finalQuotedText, finalQuotedSender)
	}
}

func attachmentFieldsFromDisplayText(mediaType, filename, text string) (caption string, seconds int) {
	switch mediaType {
	case "audio":
		return "", parseVoiceDurationSeconds(text)
	case "sticker":
		return "", 0
	case "image", "video":
		tag := "[Image]"
		if mediaType == "video" {
			tag = "[Video]"
		}
		return strings.TrimPrefix(strings.TrimPrefix(text, tag), " "), 0
	}
	rest := strings.TrimPrefix(strings.TrimPrefix(text, "[Document]"), " ")
	if filename != "" {
		if rest == filename {
			return "", 0
		}
		if after, ok := strings.CutPrefix(rest, filename+": "); ok {
			return after, 0
		}
	}
	return rest, 0
}

func parseVoiceDurationSeconds(text string) int {
	start := strings.LastIndex(text, "(")
	end := strings.LastIndex(text, ")")
	if start == -1 || end == -1 || end <= start+1 {
		return 0
	}
	parts := strings.SplitN(text[start+1:end], ":", 2)
	if len(parts) != 2 {
		return 0
	}
	minutes, mErr := strconv.Atoi(strings.TrimSpace(parts[0]))
	seconds, sErr := strconv.Atoi(strings.TrimSpace(parts[1]))
	if mErr != nil || sErr != nil {
		return 0
	}
	return minutes*60 + seconds
}

func resendStoredMessage(conn net.Conn, id, jidStr, text string) bool {
	var raw []byte
	db.QueryRow(`SELECT raw FROM messages WHERE id = ?`, id).Scan(&raw)
	if len(raw) == 0 {
		return false
	}
	msg := &waE2E.Message{}
	if proto.Unmarshal(raw, msg) != nil || !storesBuiltMessage(msg) {
		return false
	}
	jid, err := types.ParseJID(jidStr)
	if err != nil {
		return true
	}
	clearRetryState(id)
	db.Exec(`UPDATE messages SET status = 'pending' WHERE id = ?`, id)
	broadcastEvent("message", map[string]interface{}{
		"jid": jidStr, "from": "me", "id": id, "text": text, "from_me": true,
		"timestamp": time.Now().Unix(), "has_media": false, "status": "pending",
	})
	finishSendingBuiltMessage(conn, id, jid, msg, text)
	return true
}

func storesBuiltMessage(msg *waE2E.Message) bool {
	return pollCreation(msg) != nil || msg.GetContactMessage() != nil || msg.GetContactsArrayMessage() != nil
}
