package main

import (
	"archive/zip"
	"bytes"
	"encoding/binary"
	"testing"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"google.golang.org/protobuf/proto"
)

func TestStickerDescription(t *testing.T) {
	cases := []struct {
		name     string
		sticker  *waE2E.StickerMessage
		expected string
	}{
		{
			name:     "accessibility label only",
			sticker:  &waE2E.StickerMessage{AccessibilityLabel: proto.String("Happy Cat")},
			expected: "Happy Cat",
		},
		{
			name:     "emojis only",
			sticker:  &waE2E.StickerMessage{Emojis: proto.String("😂 🎉")},
			expected: "😂 🎉",
		},
		{
			name:     "avatar with label",
			sticker:  &waE2E.StickerMessage{IsAvatar: proto.Bool(true), AccessibilityLabel: proto.String("Celebrating")},
			expected: "Avatar: Celebrating",
		},
		{
			name:     "avatar without label",
			sticker:  &waE2E.StickerMessage{IsAvatar: proto.Bool(true)},
			expected: "Avatar sticker",
		},
		{
			name:     "ai sticker with label",
			sticker:  &waE2E.StickerMessage{IsAiSticker: proto.Bool(true), AccessibilityLabel: proto.String("Space Dog")},
			expected: "AI sticker: Space Dog",
		},
		{
			name:     "animated emoji",
			sticker:  &waE2E.StickerMessage{Emojis: proto.String("🔥"), IsAnimated: proto.Bool(true)},
			expected: "🔥, animated",
		},
		{
			name:     "fallback plain sticker",
			sticker:  &waE2E.StickerMessage{},
			expected: "Sticker",
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := stickerDescription(tc.sticker)
			if got != tc.expected {
				t.Errorf("stickerDescription() = %q, want %q", got, tc.expected)
			}
		})
	}
}

func TestParseWebPStickerMeta(t *testing.T) {
	// Construct a mock WebP RIFF file with an EXIF chunk
	jsonPayload := []byte(`{"sticker-pack-name":"Funny Pack","emojis":["🐱","❤️"],"accessibility-text":"Cute cat"}`)
	exifPayload := append([]byte("Exif\x00\x00"), jsonPayload...)

	buf := new(bytes.Buffer)
	buf.WriteString("RIFF")
	binary.Write(buf, binary.LittleEndian, uint32(4+8+len(exifPayload)))
	buf.WriteString("WEBP")
	buf.WriteString("EXIF")
	binary.Write(buf, binary.LittleEndian, uint32(len(exifPayload)))
	buf.Write(exifPayload)

	meta := parseStickerMeta(buf.Bytes())
	if meta == nil {
		t.Fatal("expected non-nil meta")
	}
	if meta.PackName != "Funny Pack" {
		t.Errorf("PackName = %q, want %q", meta.PackName, "Funny Pack")
	}
	if meta.Accessibility != "Cute cat" {
		t.Errorf("Accessibility = %q, want %q", meta.Accessibility, "Cute cat")
	}
	if len(meta.Emojis) != 2 || meta.Emojis[0] != "🐱" || meta.Emojis[1] != "❤️" {
		t.Errorf("Emojis = %v, want ['🐱', '❤️']", meta.Emojis)
	}
}

func TestParseLottieStickerMeta(t *testing.T) {
	// A Lottie sticker is a zip whose animation.json carries the metadata.
	buf := new(bytes.Buffer)
	zw := zip.NewWriter(buf)
	w, _ := zw.Create("animation/animation.json")
	w.Write([]byte(`{"v":"5.7.4","layers":[],"metadata":{"customProps":{"sticker-pack-id":"abc","is-first-party-sticker":1,"emojis":["👋"],"accessibility-text":"Cuppy waving hello"}}}`))
	zw.Close()

	meta := parseStickerMeta(buf.Bytes())
	if meta == nil {
		t.Fatal("expected non-nil meta")
	}
	if meta.Accessibility != "Cuppy waving hello" || meta.PackID != "abc" || meta.FirstParty != 1 {
		t.Errorf("meta = %+v", meta)
	}
	st := &waE2E.StickerMessage{Emojis: proto.String("👋")}
	labelStickerFromFile(st, buf.Bytes())
	if got := stickerDescription(st); got != "Cuppy waving hello" {
		t.Errorf("stickerDescription() = %q, want the file's accessibility text", got)
	}
}

func TestStickerMetaKeepsFieldsAroundBadOnes(t *testing.T) {
	// A field of an unexpected type mustn't lose the label.
	meta := decodeStickerMeta([]byte(`{"is-first-party-sticker":"1","accessibility-text":"Thumbs up"}`))
	if meta == nil || meta.Accessibility != "Thumbs up" {
		t.Errorf("meta = %+v", meta)
	}
}

func TestLabelStickerFromFileFallsBackToPackName(t *testing.T) {
	buf := new(bytes.Buffer)
	zw := zip.NewWriter(buf)
	w, _ := zw.Create("animation/animation.json")
	w.Write([]byte(`{"metadata":{"customProps":{"sticker-pack-name":"Party Pack"}}}`))
	zw.Close()
	st := &waE2E.StickerMessage{}
	labelStickerFromFile(st, buf.Bytes())
	if st.GetAccessibilityLabel() != "Party Pack" {
		t.Errorf("label = %q", st.GetAccessibilityLabel())
	}
	// A label the message carries is kept.
	st = &waE2E.StickerMessage{AccessibilityLabel: proto.String("Sender's label")}
	labelStickerFromFile(st, buf.Bytes())
	if st.GetAccessibilityLabel() != "Sender's label" {
		t.Errorf("label = %q", st.GetAccessibilityLabel())
	}
}
