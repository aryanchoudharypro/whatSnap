package main

import (
	"bytes"
	"encoding/binary"
	"errors"
	"io"
	"math"
	"os"
)

type videoInfo struct {
	seconds       int
	width, height int
	brand         string
	codec         string
}

const maxMoovBytes = 64 << 20

func readVideoInfo(path string) (*videoInfo, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	info := &videoInfo{}
	var moov []byte
	var offset int64
	for moov == nil {
		size, kind, headerLen, err := readBoxHeader(f, offset)
		if err != nil {
			return nil, err
		}
		switch kind {
		case "ftyp":
			brand := make([]byte, 4)
			if _, err := f.ReadAt(brand, offset+headerLen); err == nil {
				info.brand = string(brand)
			}
		case "moov":
			if size-headerLen > maxMoovBytes {
				return nil, errors.New("moov box too large")
			}
			moov = make([]byte, size-headerLen)
			if _, err := f.ReadAt(moov, offset+headerLen); err != nil {
				return nil, err
			}
		}
		if size == 0 {
			break
		}
		offset += size
	}
	if moov == nil {
		return nil, errors.New("no moov box")
	}
	if err := info.parseMoov(moov); err != nil {
		return nil, err
	}
	return info, nil
}

func readBoxHeader(r io.ReaderAt, offset int64) (size int64, kind string, headerLen int64, err error) {
	var h [16]byte
	if _, err = r.ReadAt(h[:8], offset); err != nil {
		return
	}
	size, kind, headerLen = int64(binary.BigEndian.Uint32(h[:4])), string(h[4:8]), 8
	if size == 1 {
		if _, err = r.ReadAt(h[8:16], offset+8); err != nil {
			return
		}
		size, headerLen = int64(binary.BigEndian.Uint64(h[8:16])), 16
	}
	if size != 0 && size < headerLen {
		err = errors.New("malformed box")
	}
	return
}

func eachBox(data []byte, fn func(kind string, body []byte)) {
	for len(data) >= 8 {
		size, headerLen := uint64(binary.BigEndian.Uint32(data)), uint64(8)
		kind := string(data[4:8])
		if size == 1 && len(data) >= 16 {
			size, headerLen = binary.BigEndian.Uint64(data[8:]), 16
		} else if size == 0 {
			size = uint64(len(data))
		}
		if size < headerLen || size > uint64(len(data)) {
			return
		}
		fn(kind, data[headerLen:size])
		data = data[size:]
	}
}

func (info *videoInfo) parseMoov(moov []byte) error {
	eachBox(moov, func(kind string, body []byte) {
		switch kind {
		case "mvhd":
			info.seconds = mvhdSeconds(body)
		case "trak":
			info.parseTrak(body)
		}
	})
	if info.width == 0 || info.height == 0 {
		return errors.New("no video track")
	}
	return nil
}

func mvhdSeconds(body []byte) int {
	if len(body) < 1 {
		return 0
	}
	var timescale uint32
	var duration uint64
	if body[0] == 1 && len(body) >= 32 {
		timescale, duration = binary.BigEndian.Uint32(body[20:]), binary.BigEndian.Uint64(body[24:])
	} else if len(body) >= 20 {
		timescale, duration = binary.BigEndian.Uint32(body[12:]), uint64(binary.BigEndian.Uint32(body[16:]))
	}
	if timescale == 0 || duration == math.MaxUint32 || duration == math.MaxUint64 {
		return 0
	}
	return int(math.Round(float64(duration) / float64(timescale)))
}

func (info *videoInfo) parseTrak(trak []byte) {
	if info.width != 0 {
		return
	}
	var tkhd, mdia []byte
	eachBox(trak, func(kind string, body []byte) {
		switch kind {
		case "tkhd":
			tkhd = body
		case "mdia":
			mdia = body
		}
	})
	isVideo, codec := false, ""
	eachBox(mdia, func(kind string, body []byte) {
		switch kind {
		case "hdlr":
			isVideo = len(body) >= 12 && string(body[8:12]) == "vide"
		case "minf":
			eachBox(body, func(kind string, body []byte) {
				if kind == "stbl" {
					eachBox(body, func(kind string, body []byte) {
						if kind == "stsd" && len(body) >= 16 {
							codec = string(body[12:16])
						}
					})
				}
			})
		}
	})
	if !isVideo || len(tkhd) < 84 {
		return
	}
	n := len(tkhd)
	w := int(binary.BigEndian.Uint32(tkhd[n-8:]) >> 16)
	h := int(binary.BigEndian.Uint32(tkhd[n-4:]) >> 16)
	matrix := tkhd[n-44 : n-8]
	a, b := int32(binary.BigEndian.Uint32(matrix[0:])), int32(binary.BigEndian.Uint32(matrix[4:]))
	if a == 0 && b != 0 {
		w, h = h, w
	}
	info.width, info.height, info.codec = w, h, codec
}

func mp4Brand(path string) error {
	f, err := os.OpenFile(path, os.O_RDWR, 0)
	if err != nil {
		return err
	}
	defer f.Close()
	size, kind, headerLen, err := readBoxHeader(f, 0)
	if err != nil || kind != "ftyp" || size < headerLen+8 {
		return errors.New("no ftyp box")
	}
	body := make([]byte, size-headerLen)
	if _, err := f.ReadAt(body, headerLen); err != nil {
		return err
	}
	copy(body[0:4], "mp42")
	binary.BigEndian.PutUint32(body[4:8], 0)
	for i := 8; i+4 <= len(body); i += 4 {
		switch i {
		case 8:
			copy(body[i:], "isom")
		case 12:
			copy(body[i:], "mp42")
		default:
			if bytes.Equal(body[i:i+4], []byte("qt  ")) {
				copy(body[i:], "isom")
			}
		}
	}
	_, err = f.WriteAt(body, headerLen)
	return err
}
