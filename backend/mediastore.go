package main

import (
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"
)

var (
	mediaDirOnce sync.Once
	mediaDirBase string
)

func mediaBaseDir() string {
	mediaDirOnce.Do(func() {
		dbPath := *dbPathFlag
		dir := filepath.Dir(dbPath)
		mediaDirBase = filepath.Join(dir, "media_cache")
		os.MkdirAll(filepath.Join(mediaDirBase, "media"), 0755)
		os.MkdirAll(filepath.Join(mediaDirBase, "avatars"), 0755)
		os.MkdirAll(filepath.Join(mediaDirBase, "outgoing"), 0755)
	})
	return mediaDirBase
}

func mediaDir() string  { return filepath.Join(mediaBaseDir(), "media") }
func avatarDir() string { return filepath.Join(mediaBaseDir(), "avatars") }

func extForMime(mime string) string {
	mime = strings.ToLower(strings.TrimSpace(mime))
	switch mime {
	case "image/jpeg", "image/jpg":
		return ".jpg"
	case "image/png":
		return ".png"
	case "image/webp":
		return ".webp"
	case "image/gif":
		return ".gif"
	case "video/mp4":
		return ".mp4"
	case "video/3gpp":
		return ".3gp"
	case "audio/ogg; codecs=opus", "audio/ogg":
		return ".ogg"
	case "audio/mpeg", "audio/mp3":
		return ".mp3"
	case "audio/mp4", "audio/m4a":
		return ".m4a"
	case "audio/aac":
		return ".aac"
	case "application/pdf":
		return ".pdf"
	case "application/vnd.openxmlformats-officedocument.wordprocessingml.document", "application/msword":
		return ".docx"
	case "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", "application/vnd.ms-excel":
		return ".xlsx"
	case "application/vnd.openxmlformats-officedocument.presentationml.presentation", "application/vnd.ms-powerpoint":
		return ".pptx"
	case "application/zip":
		return ".zip"
	case "application/x-rar-compressed", "application/vnd.rar":
		return ".rar"
	case "application/x-7z-compressed":
		return ".7z"
	}
	if idx := strings.Index(mime, "/"); idx != -1 {
		sub := mime[idx+1:]
		if semi := strings.Index(sub, ";"); semi != -1 {
			sub = sub[:semi]
		}
		sub = strings.TrimSpace(sub)
		if sub != "" && len(sub) <= 10 {
			// "text/x-python" and friends would otherwise land on disk as
			// ".x-python" - map them to their real extension here so the
			// frontend never has to rename a cached file behind the
			// database's back.
			return sanitizeFilename("." + sub)
		}
	}
	return ".bin"
}

func saveMediaBytes(category, id, mime string, data []byte) (string, error) {
	var dir string
	switch category {
	case "avatars":
		dir = avatarDir()
	case "outgoing":
		dir = filepath.Join(mediaBaseDir(), "outgoing")
	default:
		dir = mediaDir()
	}
	safeID := sanitizeFilenameComponent(id)
	path := filepath.Join(dir, safeID+extForMime(mime))
	if err := os.WriteFile(path, data, 0644); err != nil {
		return "", err
	}
	return path, nil
}

// downloadURLToFile fetches an unauthenticated direct-download URL (this is
// how WhatsApp serves profile pictures) and caches it locally. It's a plain
// HTTP GET since these CDN links don't require auth.
func downloadURLToFile(category, id, url string) (path string, mime string, err error) {
	httpClient := &http.Client{Timeout: 20 * time.Second}
	resp, err := httpClient.Get(url)
	if err != nil {
		return "", "", err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return "", "", fmt.Errorf("unexpected status %d fetching media", resp.StatusCode)
	}
	data, err := io.ReadAll(resp.Body)
	if err != nil {
		return "", "", err
	}
	mime = resp.Header.Get("Content-Type")
	if mime == "" {
		mime = "image/jpeg"
	}
	path, err = saveMediaBytes(category, id, mime, data)
	return path, mime, err
}

func sanitizeFilenameComponent(s string) string {
	replacer := strings.NewReplacer("/", "_", "\\", "_", ":", "_", "*", "_", "?", "_", "\"", "_", "<", "_", ">", "_", "|", "_")
	return replacer.Replace(s)
}

func readLocalFileOrDecode(path string, base64Data string) ([]byte, error) {
	if path != "" {
		return os.ReadFile(path)
	}
	if base64Data != "" {
		return decodeBase64Loose(base64Data)
	}
	return nil, fmt.Errorf("no path or data provided")
}

func isInTempDir(path string) bool {
	return isUnderDir(path, os.TempDir())
}

func isUnderDir(path, dir string) bool {
	absPath, err1 := filepath.Abs(path)
	absDir, err2 := filepath.Abs(dir)
	if err1 != nil || err2 != nil {
		return false
	}
	rel, err := filepath.Rel(strings.ToLower(absDir), strings.ToLower(absPath))
	return err == nil && rel != ".." && !strings.HasPrefix(rel, ".."+string(filepath.Separator))
}

// removeCachedMedia deletes a message's media file, but only when it lives in
// our own media cache. Outgoing attachments sent by path point at the user's
// original file, which must never be touched.
func removeCachedMedia(path string) {
	if path == "" || !isUnderDir(path, mediaBaseDir()) {
		return
	}
	os.Remove(path)
}

var scratchTempDirs = []string{"whantv_open", "whantv_clipboard"}

func cleanupScratchTempDirs(maxAge time.Duration) {
	cutoff := time.Now().Add(-maxAge)
	for _, name := range scratchTempDirs {
		dir := filepath.Join(os.TempDir(), name)
		entries, err := os.ReadDir(dir)
		if err != nil {
			continue
		}
		for _, e := range entries {
			info, err := e.Info()
			if err != nil || info.IsDir() || info.ModTime().After(cutoff) {
				continue
			}
			os.Remove(filepath.Join(dir, e.Name()))
		}
	}
}

func purgeMessages(where string, args ...interface{}) {
	rows, err := db.Query(`SELECT id, COALESCE(media_path, '') FROM messages WHERE `+where, args...)
	if err != nil {
		return
	}
	var ids, paths []string
	for rows.Next() {
		var id, path string
		if rows.Scan(&id, &path) == nil {
			ids = append(ids, id)
			if path != "" {
				paths = append(paths, path)
			}
		}
	}
	rows.Close()
	if len(ids) == 0 {
		return
	}

	writes := make([]func(tx sqlExecer), 0, len(ids))
	for _, id := range ids {
		writes = append(writes, func(tx sqlExecer) {
			tx.Exec(`DELETE FROM reactions WHERE message_id = ?`, id)
			tx.Exec(`DELETE FROM message_receipts WHERE message_id = ?`, id)
			tx.Exec(`DELETE FROM message_edits WHERE target_id = ?`, id)
			tx.Exec(`DELETE FROM poll_votes WHERE poll_id = ?`, id)
			tx.Exec(`DELETE FROM event_responses WHERE event_id = ?`, id)
			tx.Exec(`DELETE FROM messages WHERE id = ?`, id)
		})
	}
	writeBatch(writes)
	for _, p := range paths {
		removeCachedMedia(p)
	}
}

func purgeExpiredStatuses() {
	purgeMessages(`jid = 'status@broadcast' AND timestamp <= ?`, time.Now().Add(-24*time.Hour).Unix())
	db.Exec(`DELETE FROM status_viewers WHERE status_id NOT IN (SELECT id FROM messages WHERE jid = 'status@broadcast')`)
}

func sniffFileHeader(path string) []byte {
	f, err := os.Open(path)
	if err != nil {
		return nil
	}
	defer f.Close()
	buf := make([]byte, 512)
	n, _ := io.ReadFull(f, buf)
	return buf[:n]
}
