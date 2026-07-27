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

// mediaBaseDir returns (creating if needed) the local cache directory used
// for all downloaded media and avatars, placed alongside the SQLite db file
// so everything for one account lives together.
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

// extForMime maps a MIME type to a reasonable file extension for locally
// cached files. It's intentionally forgiving; an unrecognized mimetype just
// falls back to using its subtype, or .bin as a last resort.
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
			return "." + sub
		}
	}
	return ".bin"
}

// saveMediaBytes writes already-downloaded/decrypted media bytes to the
// local cache under the given category ("media", "avatars", or "outgoing"),
// keyed by id (typically a message ID, or a JID for avatars), and returns
// the absolute path so it can be handed straight to the frontend.
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

// readLocalFileOrDecode returns raw bytes from either a local file path or a
// base64 payload, whichever the frontend supplied. Callers pass both fields
// from the command payload; path wins when present since that's the primary
// delivery mechanism this backend uses.
func readLocalFileOrDecode(path string, base64Data string) ([]byte, error) {
	if path != "" {
		return os.ReadFile(path)
	}
	if base64Data != "" {
		return decodeBase64Loose(base64Data)
	}
	return nil, fmt.Errorf("no path or data provided")
}
