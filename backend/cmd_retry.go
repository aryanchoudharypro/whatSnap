package main

import (
	"encoding/json"
	"log"
	"net"
	"os"
	"strconv"
	"strings"
	"time"

	"go.mau.fi/whatsmeow/types"
)

// retryPendingTextMessages resends every text message still marked "pending"
// (never confirmed sent, whether because we were offline or the app closed
// mid-send) once the WhatsApp connection comes back up. It's called from the
// events.Connected handler, so it runs both at startup and after any
// reconnect.
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
	for _, p := range pending {
		log.Printf("Retrying pending message %s after reconnect", p.id)
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
		sendUploadedVoiceMessage(conn, id, jid, mediaPath, mediaMimetype, duration, ctxInfo, finalQuotedID, finalQuotedText, finalQuotedSender)
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
