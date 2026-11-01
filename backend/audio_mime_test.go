package main

import "testing"

func TestAudioMimetypeMatchesWhatsAppWeb(t *testing.T) {
	opus := append([]byte("OggS\x00\x02"), make([]byte, 22)...)
	opus = append(opus, []byte("OpusHead\x01\x02")...)
	cases := []struct {
		name, sniffed, file string
		header              []byte
		want                string
	}{
		{"voice note sniffed as application/ogg", "application/ogg", "a.ogg", opus, "audio/ogg; codecs=opus"},
		{"ogg vorbis", "application/ogg", "a.ogg", []byte("OggS\x00\x02rest"), "audio/ogg"},
		{"mp3 with id3", "audio/mpeg", "a.mp3", []byte("ID3\x04"), "audio/mpeg"},
		{"mp3 frame", "application/octet-stream", "a.mp3", []byte{0xFF, 0xFB, 0x90}, "audio/mpeg"},
		{"adts aac", "audio/vnd.dlna.adts", "a.aac", []byte{0xFF, 0xF1, 0x50}, "audio/aac"},
		{"m4a sniffed as video", "video/mp4", "a.m4a", []byte("\x00\x00\x00\x20ftypM4A "), "audio/mp4"},
		{"wav", "audio/wave", "a.wav", []byte("RIFF\x00\x00\x00\x00WAVEfmt "), "audio/wav"},
		{"amr", "application/octet-stream", "a.amr", []byte("#!AMR\n"), "audio/amr"},
		{"opus by extension", "", "a.opus", nil, "audio/ogg; codecs=opus"},
		{"unknown kept", "audio/flac", "a.flac", []byte("fLaC"), "audio/flac"},
	}
	for _, c := range cases {
		if got := audioMimetype(c.sniffed, c.file, c.header); got != c.want {
			t.Errorf("%s: got %q, want %q", c.name, got, c.want)
		}
	}
}
