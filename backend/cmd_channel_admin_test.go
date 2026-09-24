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

func TestChannelPictureJPEGIsSquareAndScaled(t *testing.T) {
	// A wide picture: red on the left third, green in the middle, blue on
	// the right. The square taken from it should be the green middle.
	src := image.NewRGBA(image.Rect(0, 0, 1500, 800))
	for y := 0; y < 800; y++ {
		for x := 0; x < 1500; x++ {
			c := color.RGBA{0, 255, 0, 255}
			if x < 350 {
				c = color.RGBA{255, 0, 0, 255}
			} else if x >= 1150 {
				c = color.RGBA{0, 0, 255, 255}
			}
			src.Set(x, y, c)
		}
	}
	var buf bytes.Buffer
	if err := png.Encode(&buf, src); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(t.TempDir(), "wide.png")
	if err := os.WriteFile(path, buf.Bytes(), 0o600); err != nil {
		t.Fatal(err)
	}
	out, err := channelPictureJPEG(path)
	if err != nil {
		t.Fatal(err)
	}
	img, err := jpeg.Decode(bytes.NewReader(out))
	if err != nil {
		t.Fatal(err)
	}
	if b := img.Bounds(); b.Dx() != channelPictureSize || b.Dy() != channelPictureSize {
		t.Fatalf("size = %v, want %dx%d", b, channelPictureSize, channelPictureSize)
	}
	for _, x := range []int{5, channelPictureSize / 2, channelPictureSize - 5} {
		r, g, b, _ := img.At(x, 100).RGBA()
		if g>>8 < 200 || r>>8 > 60 || b>>8 > 60 {
			t.Errorf("pixel %d = %d,%d,%d, want green", x, r>>8, g>>8, b>>8)
		}
	}
}

func TestChannelPictureJPEGRejectsNonImages(t *testing.T) {
	path := filepath.Join(t.TempDir(), "notes.txt")
	os.WriteFile(path, []byte("not a picture"), 0o600)
	if _, err := channelPictureJPEG(path); err == nil {
		t.Error("a text file was accepted as a picture")
	}
}
