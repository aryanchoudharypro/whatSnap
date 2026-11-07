package main

import (
	"bytes"
	"encoding/binary"
	"image"
	"image/color"
	"image/draw"
	_ "image/gif"
	"image/jpeg"
	_ "image/png"
	"os"
	"path/filepath"
	"strings"

	_ "golang.org/x/image/webp"
)

const (
	abPropImageMaxEdge  = 3042
	abPropImageHDEdge   = 3204
	defaultImageHDEdge  = 2560
	abPropJPEGQuality   = 6619
	defaultImageMaxEdge = 1600
	defaultJPEGQuality  = 92
	thumbMaxEdge        = 100
	thumbMaxBytes       = 20000
	preparedPhotoSuffix = "_wa.jpg"
)

type preparedPhoto struct {
	path          string
	width, height int
	thumbnail     []byte
}

func (acc *Account) preparePhoto(id, path string, hd bool) (*preparedPhoto, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	img, format, err := image.Decode(bytes.NewReader(data))
	if err != nil {
		return nil, err
	}
	if format == "jpeg" {
		img = applyEXIFOrientation(img, jpegOrientation(data))
	}
	thumb := jpegThumbnail(img)
	if format == "gif" || strings.HasSuffix(path, preparedPhotoSuffix) {
		b := img.Bounds()
		return &preparedPhoto{path: path, width: b.Dx(), height: b.Dy(), thumbnail: thumb}, nil
	}

	maxEdge := acc.serverFlagInt(abPropImageMaxEdge, defaultImageMaxEdge)
	if hd {
		maxEdge = acc.serverFlagInt(abPropImageHDEdge, defaultImageHDEdge)
	}
	img = fitWithin(img, maxEdge, catmullRom)
	var buf bytes.Buffer
	if err := jpeg.Encode(&buf, onWhite(img), &jpeg.Options{Quality: acc.serverFlagInt(abPropJPEGQuality, defaultJPEGQuality)}); err != nil {
		return nil, err
	}
	out := filepath.Join(acc.mediaBaseDir(), "outgoing", sanitizeFilenameComponent(id)+preparedPhotoSuffix)
	if err := os.MkdirAll(filepath.Dir(out), 0o755); err != nil {
		return nil, err
	}
	if err := os.WriteFile(out, buf.Bytes(), 0o644); err != nil {
		return nil, err
	}
	b := img.Bounds()
	return &preparedPhoto{path: out, width: b.Dx(), height: b.Dy(), thumbnail: thumb}, nil
}

func fitWithin(img image.Image, maxEdge int, f scaleFilter) image.Image {
	b := img.Bounds()
	w, h := b.Dx(), b.Dy()
	if maxEdge <= 0 || (w <= maxEdge && h <= maxEdge) {
		return img
	}
	if w >= h {
		h = max(1, h*maxEdge/w)
		w = maxEdge
	} else {
		w = max(1, w*maxEdge/h)
		h = maxEdge
	}
	return scaleImage(img, b, w, h, f)
}

func onWhite(img image.Image) image.Image {
	b := img.Bounds()
	dst := image.NewRGBA(image.Rect(0, 0, b.Dx(), b.Dy()))
	draw.Draw(dst, dst.Bounds(), image.NewUniform(color.White), image.Point{}, draw.Src)
	draw.Draw(dst, dst.Bounds(), img, b.Min, draw.Over)
	return dst
}

func jpegThumbnail(img image.Image) []byte {
	small := onWhite(fitWithin(img, thumbMaxEdge, bilinear))
	var buf bytes.Buffer
	for quality := 80; quality >= 20; quality -= 15 {
		buf.Reset()
		if jpeg.Encode(&buf, small, &jpeg.Options{Quality: quality}) != nil {
			return nil
		}
		if buf.Len() <= thumbMaxBytes {
			break
		}
	}
	return bytes.Clone(buf.Bytes())
}

func jpegOrientation(data []byte) int {
	if len(data) < 4 || data[0] != 0xFF || data[1] != 0xD8 {
		return 1
	}
	for i := 2; i+4 <= len(data); {
		if data[i] != 0xFF {
			return 1
		}
		marker := data[i+1]
		if marker == 0xDA || marker == 0xD9 {
			return 1
		}
		size := int(binary.BigEndian.Uint16(data[i+2:]))
		if size < 2 {
			return 1
		}
		segment := data[i+4 : min(len(data), i+2+size)]
		if marker == 0xE1 && bytes.HasPrefix(segment, []byte("Exif\x00\x00")) {
			return tiffOrientation(segment[6:])
		}
		i += 2 + size
	}
	return 1
}

func tiffOrientation(tiff []byte) int {
	if len(tiff) < 8 {
		return 1
	}
	var order binary.ByteOrder
	switch string(tiff[:2]) {
	case "II":
		order = binary.LittleEndian
	case "MM":
		order = binary.BigEndian
	default:
		return 1
	}
	ifd := int(order.Uint32(tiff[4:]))
	if ifd+2 > len(tiff) {
		return 1
	}
	count := int(order.Uint16(tiff[ifd:]))
	for n := range count {
		entry := ifd + 2 + n*12
		if entry+12 > len(tiff) {
			return 1
		}
		if order.Uint16(tiff[entry:]) == 0x0112 {
			if o := int(order.Uint16(tiff[entry+8:])); o >= 1 && o <= 8 {
				return o
			}
			return 1
		}
	}
	return 1
}

func applyEXIFOrientation(img image.Image, orientation int) image.Image {
	if orientation <= 1 || orientation > 8 {
		return img
	}
	b := img.Bounds()
	w, h := b.Dx(), b.Dy()
	swap := orientation >= 5
	dw, dh := w, h
	if swap {
		dw, dh = h, w
	}
	dst := image.NewRGBA(image.Rect(0, 0, dw, dh))
	for y := range h {
		for x := range w {
			var dx, dy int
			switch orientation {
			case 2:
				dx, dy = w-1-x, y
			case 3:
				dx, dy = w-1-x, h-1-y
			case 4:
				dx, dy = x, h-1-y
			case 5:
				dx, dy = y, x
			case 6:
				dx, dy = h-1-y, x
			case 7:
				dx, dy = h-1-y, w-1-x
			case 8:
				dx, dy = y, w-1-x
			}
			dst.Set(dx, dy, img.At(b.Min.X+x, b.Min.Y+y))
		}
	}
	return dst
}
