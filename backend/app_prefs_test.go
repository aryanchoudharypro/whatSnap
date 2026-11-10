package main

import "testing"

func TestAutoDownloadFollowsWhatsAppWebLimits(t *testing.T) {
	cases := []struct {
		mediaType string
		size      int64
		gif       bool
		want      string
	}{
		{"image", 1 << 20, false, "auto_download_photos"},
		{"image", 33 << 20, false, ""},
		{"video", 10 << 20, false, "auto_download_videos"},
		{"video", 100 << 10, true, "auto_download_photos"},
		{"video", 2 << 20, true, "auto_download_videos"},
		{"document", 5 << 20, false, "auto_download_documents"},
		{"audio", 100 << 10, false, "auto_download_audio"},
		{"audio", 1 << 20, false, ""},
		{"voice", 10 << 10, false, ""},
	}
	for _, c := range cases {
		if got := autoDownloadPref(c.mediaType, c.size, c.gif); got != c.want {
			t.Errorf("autoDownloadPref(%q, %d, %v) = %q, want %q", c.mediaType, c.size, c.gif, got, c.want)
		}
	}
}

func TestLooksLikePhoneLabel(t *testing.T) {
	cases := map[string]bool{
		"+60123318658":     true,
		"+60 12-331 8658":  true,
		"12345@lid":        true,
		"迪豐 Whatsapp Only": false,
		"Amir":             false,
		"Room 101":         false,
		"":                 false,
	}
	for in, want := range cases {
		if got := looksLikePhoneLabel(in); got != want {
			t.Errorf("looksLikePhoneLabel(%q) = %v, want %v", in, got, want)
		}
	}
}
