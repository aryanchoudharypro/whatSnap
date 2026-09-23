package main

import (
	"encoding/json"
	"log"
	"net"
	"os"
	"strconv"
	"strings"
	"sync"
	"time"

	"go.mau.fi/whatsmeow/types"
)

// Auto-retry policy for pending text sends, mirroring official WhatsApp:
// keep quietly retrying with backoff while offline/erroring, and only
// surface a "failed to send" state (requiring a manual tap) once retries
// are exhausted.
const (
	maxAutoRetries   = 6
	retryBaseDelay   = 3 * time.Second
	retryMaxDelay    = 2 * time.Minute
	retrySweepPeriod = 3 * time.Second
)

var (
	retryMu       sync.Mutex
	sendInFlight  = map[string]bool{}
	retryAttempts = map[string]int{}
	retryNextAt   = map[string]time.Time{}
)

func init() {
	// Single periodic sweep, independent of the reconnect-triggered one in
	// whatsapp_events.go, so messages that fail while the socket stays up
	// (not just offline) still get retried. beginSendAttempt's in-flight
	// guard makes the two triggers safe to overlap instead of double-sending.
	go func() {
		ticker := time.NewTicker(retrySweepPeriod)
		defer ticker.Stop()
		for range ticker.C {
			if client == nil || !client.IsConnected() {
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

// clearRetryState resets a message's backoff bookkeeping. Called on a
// successful send, and on a manual retry so a user-initiated attempt always
// gets a fresh run of automatic backoff rather than inheriting whatever was
// left over from before.
func clearRetryState(id string) {
	retryMu.Lock()
	delete(retryAttempts, id)
	delete(retryNextAt, id)
	retryMu.Unlock()
}

// recordSendFailure schedules the next backoff attempt, or - once
// maxAutoRetries is exhausted - marks the message "failed" so the UI stops
// silently retrying and asks the user to retry manually instead.
func recordSendFailure(id, jidStr, text, quotedID, quotedSender string) {
	retryMu.Lock()
	attempts := retryAttempts[id] + 1
	retryAttempts[id] = attempts
	if attempts >= maxAutoRetries {
		delete(retryAttempts, id)
		delete(retryNextAt, id)
		retryMu.Unlock()

		db.Exec(`UPDATE messages SET status = 'failed' WHERE id = ?`, id)
		broadcastEvent("message", map[string]interface{}{
			"jid": jidStr, "from": "me", "id": id, "text": text, "from_me": true,
			"timestamp": time.Now().Unix(), "has_media": false, "status": "failed",
			"quotedId": quotedID, "quotedSender": quotedSender,
		})
		return
	}
	delay := retryBaseDelay * time.Duration(uint64(1)<<uint(attempts-1))
	if delay > retryMaxDelay {
		delay = retryMaxDelay
	}
	retryNextAt[id] = time.Now().Add(delay)
	retryMu.Unlock()
}

// retryPendingTextMessages resends every text message still marked "pending"
// that isn't already in flight and whose backoff delay has elapsed. It's
// called from the events.Connected handler (for an immediate attempt right
// on reconnect) and from the periodic sweep started in init() above.
//
// Media is deliberately excluded here: a stalled upload is surfaced as
// "failed" instead of "pending", and is only retried when the user
// explicitly asks via the message context menu — silently re-uploading a
// possibly-large file in the background isn't something that should just
// happen without the user noticing.
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
		sendPendingTextMessage(nil, p.id, p.jid, p.text, p.quotedID, p.quotedSender)
	}
}

// handleRetryMessage lets the frontend explicitly re-attempt a message that's
// stuck "pending" or "failed" (context menu "Retry send"), for both text and
// media.
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
		return // already sent (or something else entirely) - nothing to do
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

	// Unlike the text path (sendPendingTextMessage), nothing claimed an
	// in-flight slot for media before this point - two overlapping retries
	// (e.g. a double-tapped "Retry send" on a slow upload) would otherwise
	// both pass the pending/failed status check above and both upload the
	// same file concurrently.
	if !beginSendAttempt(id) {
		return
	}
	defer endSendAttempt(id)

	db.Exec(`UPDATE messages SET status = 'uploading' WHERE id = ?`, id)
	broadcastEvent("message", map[string]interface{}{
		"jid": jidStr, "from": "me", "id": id, "text": text, "from_me": true,
		"timestamp": time.Now().Unix(), "has_media": true, "mediaType": mediaType,
		"mediaFilename": mediaFilename, "status": "uploading",
	})

	// Re-derive the ContextInfo from the still-referenced quoted message
	// (rather than trusting a possibly-stale quoted_text snapshot); if the
	// quoted message is gone this just drops the quote, which is fine.
	ctxInfo, finalQuotedID, finalQuotedText, finalQuotedSender := buildAttachmentContext(quotedID, quotedSender)

	if mediaType == "voice" {
		duration := parseVoiceDurationSeconds(text)
		var waveform []byte
		db.QueryRow(`SELECT voice_waveform FROM messages WHERE id = ?`, id).Scan(&waveform)
		sendUploadedVoiceMessage(conn, id, jid, mediaPath, mediaMimetype, duration, waveform, ctxInfo, finalQuotedID, finalQuotedText, finalQuotedSender)
	} else {
		sendUploadedAttachment(conn, id, jid, mediaType, mediaPath, mediaMimetype, mediaFilename, text, ctxInfo, finalQuotedID, finalQuotedText, finalQuotedSender)
	}
}

// parseVoiceDurationSeconds pulls the "(m:ss)" duration back out of a stored
// voice message's display text (e.g. "[Voice Message] (1:23)"), since that's
// the only place the original recording length is kept.
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
