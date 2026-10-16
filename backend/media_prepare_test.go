package main

import (
	"bytes"
	"image"
	"image/color"
	"image/jpeg"
	"image/png"
	"os"
	"path/filepath"
	"testing"
)

// withEXIFOrientation inserts an APP1 Exif segment carrying orientation o
// right after a JPEG's SOI marker.
func withEXIFOrientation(jpg []byte, o uint16) []byte {
	tiff := []byte{'M', 'M', 0, 42, 0, 0, 0, 8, 0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, byte(o >> 8), byte(o), 0, 0, 0, 0, 0, 0}
	payload := append([]byte("Exif\x00\x00"), tiff...)
	seg := []byte{0xFF, 0xE1, byte((len(payload) + 2) >> 8), byte(len(payload) + 2)}
	seg = append(seg, payload...)
	return append(append(append([]byte{}, jpg[:2]...), seg...), jpg[2:]...)
}

func TestPreparePhoto(t *testing.T) {
	dir := t.TempDir()
	// Prepared photos go to the media cache: point it at the test's folder.
	mediaDirOnce.Do(func() {})
	oldBase := mediaDirBase
	mediaDirBase = dir
	t.Cleanup(func() { mediaDirBase = oldBase })

	// A wide transparent PNG: resized to the 1600 px edge, flattened, JPEG.
	big := image.NewNRGBA(image.Rect(0, 0, 4000, 1000))
	big.Set(10, 10, color.NRGBA{255, 0, 0, 255})
	var pngBuf bytes.Buffer
	png.Encode(&pngBuf, big)
	src := filepath.Join(dir, "big.png")
	os.WriteFile(src, pngBuf.Bytes(), 0o644)
	p, err := preparePhoto("big", src)
	if err != nil {
		t.Fatal(err)
	}
	if p.width != 1600 || p.height != 400 {
		t.Fatalf("size %dx%d, want 1600x400", p.width, p.height)
	}
	out, _ := os.ReadFile(p.path)
	if _, format, err := image.Decode(bytes.NewReader(out)); err != nil || format != "jpeg" {
		t.Fatalf("output not a JPEG: %v %v", format, err)
	}
	if len(p.thumbnail) == 0 || len(p.thumbnail) > thumbMaxBytes {
		t.Fatalf("thumbnail is %d bytes", len(p.thumbnail))
	}
	tb, _, _ := image.DecodeConfig(bytes.NewReader(p.thumbnail))
	if tb.Width != 100 || tb.Height != 25 {
		t.Fatalf("thumbnail %dx%d, want 100x25", tb.Width, tb.Height)
	}

	// A portrait shot stored sideways (orientation 6) comes out upright.
	side := image.NewRGBA(image.Rect(0, 0, 300, 200))
	var jb bytes.Buffer
	jpeg.Encode(&jb, side, nil)
	src = filepath.Join(dir, "side.jpg")
	os.WriteFile(src, withEXIFOrientation(jb.Bytes(), 6), 0o644)
	p, err = preparePhoto("side", src)
	if err != nil {
		t.Fatal(err)
	}
	if p.width != 200 || p.height != 300 {
		t.Fatalf("rotated size %dx%d, want 200x300", p.width, p.height)
	}

	// A small photo isn't enlarged, and a prepared one isn't re-encoded.
	again, err := preparePhoto("side", p.path)
	if err != nil || again.path != p.path || again.width != 200 {
		t.Fatalf("re-preparing changed it: %+v %v", again, err)
	}
}
