package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

// GIF search, like WA Web's GIF panel, from Giphy with the user's own API
// key (WhatsApp's key is theirs). A GIF picked is fetched as Giphy's MP4 and
// sent the way the frontend already sends a GIF file: a looping, silent
// video (gifPlayback).

type GifSearchData struct {
	Key   string `json:"key"`
	Query string `json:"query"`
}

type GifFetchData struct {
	URL   string `json:"url"`
	ID    string `json:"id"`
	Title string `json:"title"`
	JID   string `json:"jid"` // the chat it's for, echoed back
}

// maxGifBytes keeps a picked GIF within what WhatsApp sends as one.
const maxGifBytes = 8 << 20

var gifHTTP = &http.Client{Timeout: 20 * time.Second}

type giphyRendition struct {
	MP4     string `json:"mp4"`
	MP4Size string `json:"mp4_size"`
}

type giphyResponse struct {
	Data []struct {
		ID     string `json:"id"`
		Title  string `json:"title"`
		Images struct {
			OriginalMP4 giphyRendition `json:"original_mp4"`
			FixedHeight giphyRendition `json:"fixed_height"`
		} `json:"images"`
	} `json:"data"`
	Meta struct {
		Status int    `json:"status"`
		Msg    string `json:"msg"`
	} `json:"meta"`
}

// handleSearchGifs searches Giphy, or lists what's trending for an empty
// query, as WA Web's panel opens on.
func handleSearchGifs(conn net.Conn, raw json.RawMessage) {
	var data GifSearchData
	if err := json.Unmarshal(raw, &data); err != nil || strings.TrimSpace(data.Key) == "" {
		return
	}
	go func() {
		query := strings.TrimSpace(data.Query)
		params := url.Values{"api_key": {strings.TrimSpace(data.Key)}, "limit": {"25"}, "rating": {"pg-13"}}
		endpoint := "https://api.giphy.com/v1/gifs/trending?"
		if query != "" {
			params.Set("q", query)
			endpoint = "https://api.giphy.com/v1/gifs/search?"
		}
		fail := func(message string) {
			sendEvent(conn, "gifs_failed", map[string]string{"query": query, "message": message})
		}
		resp, err := gifHTTP.Get(endpoint + params.Encode())
		if err != nil {
			// A *url.Error quotes the URL, API key and all.
			var urlErr *url.Error
			if errors.As(err, &urlErr) {
				err = urlErr.Err
			}
			fail("Couldn't reach Giphy: " + err.Error())
			return
		}
		defer resp.Body.Close()
		var parsed giphyResponse
		if err := json.NewDecoder(io.LimitReader(resp.Body, 4<<20)).Decode(&parsed); err != nil {
			fail(fmt.Sprintf("Giphy answered %s.", resp.Status))
			return
		}
		if resp.StatusCode == http.StatusUnauthorized || resp.StatusCode == http.StatusForbidden {
			fail("Giphy didn't accept the API key.")
			return
		}
		if resp.StatusCode != http.StatusOK {
			fail(fmt.Sprintf("Giphy answered %s %s.", resp.Status, parsed.Meta.Msg))
			return
		}
		results := []map[string]string{}
		for _, g := range parsed.Data {
			pick := g.Images.OriginalMP4
			if size, _ := strconv.Atoi(pick.MP4Size); pick.MP4 == "" || size > maxGifBytes/2 {
				pick = g.Images.FixedHeight
			}
			if pick.MP4 == "" {
				continue
			}
			results = append(results, map[string]string{"id": g.ID, "title": strings.TrimSpace(g.Title), "mp4": pick.MP4})
		}
		sendEvent(conn, "gifs", map[string]interface{}{"query": query, "results": results})
	}()
}

// handleFetchGif downloads a picked GIF's MP4 to a temporary file for the
// frontend to send.
func handleFetchGif(conn net.Conn, raw json.RawMessage) {
	var data GifFetchData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		fail := func(message string) {
			sendEvent(conn, "gifs_failed", map[string]string{"message": message})
		}
		parsed, err := url.Parse(data.URL)
		if err != nil || parsed.Scheme != "https" || !strings.HasSuffix(parsed.Hostname(), ".giphy.com") {
			fail("That isn't a Giphy GIF.")
			return
		}
		resp, err := gifHTTP.Get(data.URL)
		if err != nil {
			fail("Couldn't download the GIF: " + err.Error())
			return
		}
		defer resp.Body.Close()
		if resp.StatusCode != http.StatusOK {
			fail("Couldn't download the GIF: " + resp.Status)
			return
		}
		path := filepath.Join(os.TempDir(), "whantv_gif_"+sanitizeFilenameComponent(data.ID)+".mp4")
		f, err := os.Create(path)
		if err != nil {
			fail("Couldn't save the GIF: " + err.Error())
			return
		}
		n, err := io.Copy(f, io.LimitReader(resp.Body, maxGifBytes+1))
		f.Close()
		if err != nil || n > maxGifBytes {
			os.Remove(path)
			fail("That GIF is too big to send.")
			return
		}
		sendEvent(conn, "gif_ready", map[string]string{"path": path, "title": data.Title, "jid": data.JID})
	}()
}
