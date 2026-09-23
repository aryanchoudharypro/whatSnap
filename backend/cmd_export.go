package main

import (
	"archive/zip"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net"
	"os"
	"strings"
	"time"

	"go.mau.fi/whatsmeow/types"
)

type ExportChatData struct {
	JID      string `json:"jid"`
	DestPath string `json:"dest_path"` // .zip file to write: transcript.txt + media/
}

// downloadMediaForExportSync blocks until a message's media is on disk (using
// whatever's already cached) or gives up. It mirrors handleDownloadMedia's
// "URL expired -> ask the phone to re-upload -> wait" retry, but synchronous
// and silent: export just needs a yes/no per message, not progress events.
func downloadMediaForExportSync(messageID string) (path string, ok bool) {
	var cached string
	db.QueryRow("SELECT COALESCE(media_path, '') FROM messages WHERE id = ?", messageID).Scan(&cached)
	if cached != "" {
		if _, statErr := os.Stat(cached); statErr == nil {
			return cached, true
		}
	}

	ctx := context.Background()
	for attempt := 0; attempt < 2; attempt++ {
		p, _, _, _, err := downloadMessageMediaToFile(ctx, messageID, nil)
		if err == nil {
			return p, true
		}

		expired := strings.Contains(err.Error(), "403") || strings.Contains(err.Error(), "404") || strings.Contains(err.Error(), "410")
		if attempt != 0 || !expired {
			return "", false
		}

		if retryErr := requestMediaRetry(messageID); retryErr != nil {
			return "", false
		}
		ch := make(chan bool, 1)
		retryWaitersMu.Lock()
		retryWaiters[messageID] = ch
		retryWaitersMu.Unlock()
		select {
		case success := <-ch:
			if !success {
				return "", false
			}
			// loop again to download the now-fresh media
		case <-time.After(45 * time.Second):
			retryWaitersMu.Lock()
			delete(retryWaiters, messageID)
			retryWaitersMu.Unlock()
			return "", false
		}
	}
	return "", false
}

func handleExportChat(conn net.Conn, raw json.RawMessage) {
	var data ExportChatData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}

	go func() {
		var chatName string
		db.QueryRow("SELECT COALESCE(name, '') FROM chats WHERE jid = ?", data.JID).Scan(&chatName)
		if chatName == "" {
			chatName = resolveDisplayName(jid)
		}

		rows, queryErr := db.Query(`SELECT id, COALESCE(text, ''), timestamp, from_me, COALESCE(sender_jid, ''),
				COALESCE(media_type, ''), COALESCE(is_deleted, 0)
			FROM messages WHERE jid = ? ORDER BY timestamp ASC, id ASC`, data.JID)
		if queryErr != nil {
			broadcastEvent("error", map[string]string{"message": "Export failed: " + queryErr.Error()})
			return
		}
		type row struct {
			id, text, senderJID, mediaType string
			ts                             int64
			fromMe, deleted                bool
		}
		var msgRows []row
		for rows.Next() {
			var r row
			var deletedInt int
			if scanErr := rows.Scan(&r.id, &r.text, &r.ts, &r.fromMe, &r.senderJID, &r.mediaType, &deletedInt); scanErr == nil {
				r.deleted = deletedInt == 1
				msgRows = append(msgRows, r)
			}
		}
		rows.Close()

		total := len(msgRows)
		zipFile, err := os.Create(data.DestPath)
		if err != nil {
			broadcastEvent("error", map[string]string{"message": "Export failed: " + err.Error()})
			return
		}
		defer zipFile.Close()
		zw := zip.NewWriter(zipFile)

		var transcript strings.Builder
		fmt.Fprintf(&transcript, "Chat export: %s\n", chatName)
		fmt.Fprintf(&transcript, "Exported: %s\n\n", time.Now().Format("2006-01-02 15:04:05"))

		mediaExported, mediaSkipped := 0, 0
		for i, r := range msgRows {
			if i%25 == 0 || i == total-1 {
				broadcastEvent("export_progress", map[string]interface{}{
					"jid": data.JID, "current": i + 1, "total": total,
				})
			}

			tsStr := time.Unix(r.ts, 0).Format("2006-01-02 15:04")
			sender := "You"
			if !r.fromMe {
				if r.senderJID != "" {
					if parsed, perr := types.ParseJID(r.senderJID); perr == nil {
						sender = resolveDisplayName(parsed)
					} else {
						sender = chatName
					}
				} else {
					sender = chatName
				}
			}

			if r.deleted {
				fmt.Fprintf(&transcript, "[%s] %s: This message was deleted\n", tsStr, sender)
				continue
			}

			body := r.text
			if body == "" {
				body = "[Media]"
			}

			if r.mediaType != "" {
				if path, ok := downloadMediaForExportSync(r.id); ok {
					// Streamed into the archive rather than read whole: a chat's
					// videos would otherwise each sit fully in memory in turn.
					if f, openErr := os.Open(path); openErr == nil {
						entryName := "media/" + sanitizeFilenameComponent(r.id) + extForFilePath(path)
						if w, zerr := zw.Create(entryName); zerr == nil {
							if _, werr := io.Copy(w, f); werr == nil {
								body = fmt.Sprintf("%s (%s)", body, entryName)
								mediaExported++
							}
						}
						f.Close()
					} else {
						mediaSkipped++
					}
				} else {
					mediaSkipped++
					body = fmt.Sprintf("%s (media unavailable)", body)
				}
			}

			fmt.Fprintf(&transcript, "[%s] %s: %s\n", tsStr, sender, body)
		}

		if w, zerr := zw.Create("transcript.txt"); zerr == nil {
			io.WriteString(w, transcript.String())
		}
		if err := zw.Close(); err != nil {
			log.Println("Error finalizing export zip:", err)
			broadcastEvent("error", map[string]string{"message": "Export failed while finalizing archive: " + err.Error()})
			return
		}

		broadcastEvent("chat_exported", map[string]interface{}{
			"jid": data.JID, "path": data.DestPath, "message_count": total,
			"media_exported": mediaExported, "media_skipped": mediaSkipped,
		})
	}()
}

type ExportChatTextData struct {
	JID      string `json:"jid"`
	DestPath string `json:"dest_path"` // .md file to write
}

// handleExportChatText writes a text-only Markdown transcript: no media is
// downloaded (fast, works offline), each calendar date is an H2 heading, and
// every message is one line "**Sender**, HH:MM — content". Deleted messages
// render "This message was deleted"; media collapses to a placeholder
// ([ptt] for voice, [file] for documents, [media] for everything else), with
// any caption text kept after it.
func handleExportChatText(conn net.Conn, raw json.RawMessage) {
	var data ExportChatTextData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}

	go func() {
		var chatName string
		db.QueryRow("SELECT COALESCE(name, '') FROM chats WHERE jid = ?", data.JID).Scan(&chatName)
		if chatName == "" {
			chatName = resolveDisplayName(jid)
		}

		rows, queryErr := db.Query(`SELECT COALESCE(text, ''), timestamp, from_me, COALESCE(sender_jid, ''),
				COALESCE(media_type, ''), COALESCE(is_deleted, 0)
			FROM messages WHERE jid = ? ORDER BY timestamp ASC, id ASC`, data.JID)
		if queryErr != nil {
			broadcastEvent("error", map[string]string{"message": "Export failed: " + queryErr.Error()})
			return
		}

		var md strings.Builder

		lastDate := ""
		total := 0
		for rows.Next() {
			var text, senderJID, mediaType string
			var ts int64
			var fromMe bool
			var deletedInt int
			if scanErr := rows.Scan(&text, &ts, &fromMe, &senderJID, &mediaType, &deletedInt); scanErr != nil {
				continue
			}

			t := time.Unix(ts, 0)

			var content string
			switch {
			case deletedInt == 1:
				content = "This message was deleted"
			case mediaType != "":
				placeholder := "[media]"
				switch mediaType {
				case "voice":
					placeholder = "[ptt]"
				case "document":
					placeholder = "[file]"
				}
				if strings.TrimSpace(text) != "" {
					content = placeholder + " " + text
				} else {
					content = placeholder
				}
			default:
				content = text
			}

			// Nothing worth a line (e.g. a protocol/placeholder message with no
			// text and no media) - skip it rather than emit an empty entry.
			if strings.TrimSpace(content) == "" {
				continue
			}
			// Keep one message per line so the "Sender, time — text" shape holds.
			content = strings.ReplaceAll(content, "\r\n", " ")
			content = strings.ReplaceAll(content, "\n", " ")

			if date := t.Format("01/02/06"); date != lastDate {
				// First date has no leading blank line so the file opens directly
				// on the heading; later dates get a separating blank line.
				if lastDate == "" {
					fmt.Fprintf(&md, "# %s\n\n", date)
				} else {
					fmt.Fprintf(&md, "\n# %s\n\n", date)
				}
				lastDate = date
			}

			sender := "You"
			if !fromMe {
				if senderJID != "" {
					if parsed, perr := types.ParseJID(senderJID); perr == nil {
						sender = resolveDisplayName(parsed)
					} else {
						sender = chatName
					}
				} else {
					sender = chatName
				}
			}

			fmt.Fprintf(&md, "%s, %s — %s\n", sender, t.Format("15:04"), content)
			total++
		}
		rows.Close()

		if err := os.WriteFile(data.DestPath, []byte(md.String()), 0644); err != nil {
			broadcastEvent("error", map[string]string{"message": "Export failed: " + err.Error()})
			return
		}

		broadcastEvent("chat_exported", map[string]interface{}{
			"jid": data.JID, "path": data.DestPath, "message_count": total, "text_only": true,
		})
	}()
}

// extForFilePath pulls the extension straight off a cached media file's own
// path rather than re-deriving it from mimetype, since that's exactly the
// extension ensureMediaCached/downloadMessageMediaToFile already chose for it.
func extForFilePath(path string) string {
	for i := len(path) - 1; i >= 0; i-- {
		if path[i] == '.' {
			return path[i:]
		}
		if path[i] == '/' || path[i] == '\\' {
			break
		}
	}
	return ""
}
