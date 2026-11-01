package main

import (
	"bytes"
	"context"
	"html"
	"image"
	"io"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"time"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

const (
	linkPreviewTimeout   = 5 * time.Second
	linkPreviewPageBytes = 512 << 10
	linkPreviewImageMax  = 4 << 20
	linkPreviewCacheSize = 32
	linkPreviewsOffKey   = "link_previews_disabled"
)

type linkPreview struct {
	title, description string
	thumbnail          []byte
}

var (
	linkPreviewMu    sync.Mutex
	linkPreviewCache = map[string]*linkPreview{}
	linkPreviewOrder []string
)

func noteDisableLinkPreviews(v *events.DisableLinkPreviews) {
	db.Exec(`INSERT OR REPLACE INTO app_settings (key, value) VALUES (?, ?)`, linkPreviewsOffKey, boolToInt(v.Disabled))
}

func linkPreviewsDisabled() bool {
	var off int
	db.QueryRow(`SELECT CAST(value AS INTEGER) FROM app_settings WHERE key = ?`, linkPreviewsOffKey).Scan(&off)
	return off == 1
}

func firstWebLink(text string) (matched, fetch string) {
	for _, word := range strings.Fields(text) {
		lower := strings.ToLower(word)
		start := strings.Index(lower, "https://")
		if start < 0 {
			start = strings.Index(lower, "http://")
		}
		if start < 0 {
			start = strings.Index(lower, "www.")
		}
		if start < 0 {
			continue
		}
		word = strings.TrimRight(word[start:], ".,;:!?)]}'\"")
		fetch = word
		if strings.HasPrefix(strings.ToLower(word), "www.") {
			fetch = "https://" + word
		}
		if u, err := url.Parse(fetch); err == nil && u.Host != "" && strings.Contains(u.Host, ".") {
			return word, fetch
		}
	}
	return "", ""
}

func withLinkPreview(msg *waE2E.Message) *waE2E.Message {
	text := msg.GetConversation()
	if text == "" {
		text = msg.GetExtendedTextMessage().GetText()
	}
	matched, fetch := firstWebLink(text)
	if matched == "" || linkPreviewsDisabled() {
		return msg
	}
	preview := fetchLinkPreview(fetch)
	if preview == nil {
		return msg
	}
	if msg.Conversation != nil {
		msg = &waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{Text: proto.String(text)}}
	}
	ext := msg.GetExtendedTextMessage()
	ext.MatchedText = proto.String(matched)
	ext.Title = proto.String(preview.title)
	if preview.description != "" {
		ext.Description = proto.String(preview.description)
	}
	if len(preview.thumbnail) > 0 {
		ext.JPEGThumbnail = preview.thumbnail
	}
	ext.PreviewType = waE2E.ExtendedTextMessage_NONE.Enum()
	return msg
}

func fetchLinkPreview(pageURL string) *linkPreview {
	linkPreviewMu.Lock()
	if cached, ok := linkPreviewCache[pageURL]; ok {
		linkPreviewMu.Unlock()
		return cached
	}
	linkPreviewMu.Unlock()

	ctx, cancel := context.WithTimeout(context.Background(), linkPreviewTimeout)
	defer cancel()
	page, finalURL := httpGetLimited(ctx, pageURL, linkPreviewPageBytes, "text/html")
	var preview *linkPreview
	if page != nil {
		tags := pageMetaTags(page)
		title := firstNonEmpty(tags["og:title"], tags["twitter:title"], tags["<title>"])
		if title != "" {
			preview = &linkPreview{
				title:       clip(title, 200),
				description: clip(firstNonEmpty(tags["og:description"], tags["twitter:description"], tags["description"]), 300),
			}
			if img := firstNonEmpty(tags["og:image"], tags["og:image:url"], tags["twitter:image"]); img != "" {
				if base, err := url.Parse(finalURL); err == nil {
					if ref, err := base.Parse(img); err == nil && (ref.Scheme == "https" || ref.Scheme == "http") {
						if data, _ := httpGetLimited(ctx, ref.String(), linkPreviewImageMax, "image/"); data != nil {
							if decoded, _, err := image.Decode(bytes.NewReader(data)); err == nil {
								preview.thumbnail = jpegThumbnail(decoded)
							}
						}
					}
				}
			}
		}
	}

	linkPreviewMu.Lock()
	defer linkPreviewMu.Unlock()
	if _, ok := linkPreviewCache[pageURL]; !ok {
		linkPreviewCache[pageURL] = preview
		linkPreviewOrder = append(linkPreviewOrder, pageURL)
		if len(linkPreviewOrder) > linkPreviewCacheSize {
			delete(linkPreviewCache, linkPreviewOrder[0])
			linkPreviewOrder = linkPreviewOrder[1:]
		}
	}
	return preview
}

func httpGetLimited(ctx context.Context, rawURL string, limit int64, wantType string) ([]byte, string) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, rawURL, nil)
	if err != nil {
		return nil, ""
	}
	req.Header.Set("User-Agent", "WhatsApp/2.2412.54 W")
	req.Header.Set("Accept-Language", "en")
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return nil, ""
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK || !strings.HasPrefix(strings.ToLower(resp.Header.Get("Content-Type")), wantType) {
		return nil, ""
	}
	data, err := io.ReadAll(io.LimitReader(resp.Body, limit))
	if err != nil && len(data) == 0 {
		return nil, ""
	}
	return data, resp.Request.URL.String()
}

func pageMetaTags(page []byte) map[string]string {
	doc := string(page)
	lower := strings.ToLower(doc)
	if end := strings.Index(lower, "</head>"); end > 0 {
		doc, lower = doc[:end], lower[:end]
	}
	tags := map[string]string{}
	for pos := 0; ; {
		i := strings.Index(lower[pos:], "<meta")
		if i < 0 {
			break
		}
		start := pos + i
		end := strings.IndexByte(lower[start:], '>')
		if end < 0 {
			break
		}
		attrs := tagAttributes(doc[start+5 : start+end])
		key := strings.ToLower(firstNonEmpty(attrs["property"], attrs["name"]))
		if key != "" && attrs["content"] != "" {
			if _, seen := tags[key]; !seen {
				tags[key] = strings.TrimSpace(html.UnescapeString(attrs["content"]))
			}
		}
		pos = start + end
	}
	if i := strings.Index(lower, "<title"); i >= 0 {
		if open := strings.IndexByte(lower[i:], '>'); open >= 0 {
			rest := doc[i+open+1:]
			if close := strings.Index(strings.ToLower(rest), "</title>"); close >= 0 {
				tags["<title>"] = strings.Join(strings.Fields(html.UnescapeString(rest[:close])), " ")
			}
		}
	}
	return tags
}

func tagAttributes(s string) map[string]string {
	attrs := map[string]string{}
	for {
		s = strings.TrimLeft(s, " \t\r\n/")
		eq := strings.IndexByte(s, '=')
		if eq <= 0 {
			return attrs
		}
		name := strings.ToLower(strings.TrimSpace(s[:eq]))
		s = strings.TrimLeft(s[eq+1:], " \t\r\n")
		var value string
		if s != "" && (s[0] == '"' || s[0] == '\'') {
			end := strings.IndexByte(s[1:], s[0])
			if end < 0 {
				return attrs
			}
			value, s = s[1:end+1], s[end+2:]
		} else {
			end := strings.IndexAny(s, " \t\r\n")
			if end < 0 {
				end = len(s)
			}
			value, s = s[:end], s[end:]
		}
		if fields := strings.Fields(name); len(fields) > 0 {
			attrs[fields[len(fields)-1]] = value
		}
	}
}

func firstNonEmpty(values ...string) string {
	for _, v := range values {
		if v = strings.TrimSpace(v); v != "" {
			return v
		}
	}
	return ""
}

func clip(s string, n int) string {
	if r := []rune(s); len(r) > n {
		return strings.TrimSpace(string(r[:n-1])) + "…"
	}
	return s
}
