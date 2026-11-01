package main

import (
	"bytes"
	"encoding/binary"
	"os"
	"path/filepath"
	"testing"
)

func box(kind string, parts ...[]byte) []byte {
	body := bytes.Join(parts, nil)
	out := binary.BigEndian.AppendUint32(nil, uint32(8+len(body)))
	return append(append(out, kind...), body...)
}

func u32(v uint32) []byte { return binary.BigEndian.AppendUint32(nil, v) }

func testMovie(brand, codec string, w, h, seconds uint32, turned bool) []byte {
	mvhd := box("mvhd", u32(0), u32(0), u32(0), u32(1000), u32(seconds*1000), make([]byte, 80))
	matrix := []uint32{0x10000, 0, 0, 0, 0x10000, 0, 0, 0, 0x40000000}
	if turned {
		matrix = []uint32{0, 0x10000, 0, 0xFFFF0000, 0, 0, 0, 0, 0x40000000}
	}
	var m []byte
	for _, v := range matrix {
		m = append(m, u32(v)...)
	}
	tkhd := box("tkhd", u32(0), make([]byte, 20), make([]byte, 8), make([]byte, 8), m, u32(w<<16), u32(h<<16))
	stsd := box("stsd", u32(0), u32(1), box(codec, make([]byte, 70)))
	mdia := box("mdia", box("hdlr", u32(0), u32(0), []byte("vide"), make([]byte, 12)),
		box("minf", box("stbl", stsd)))
	return append(box("ftyp", []byte(brand), u32(0), []byte(brand)),
		append(box("mdat", make([]byte, 64)), box("moov", mvhd, box("trak", tkhd, mdia))...)...)
}

func TestReadVideoInfo(t *testing.T) {
	dir := t.TempDir()
	cases := []struct {
		name        string
		data        []byte
		w, h, secs  int
		brand, kind string
	}{
		{"landscape mp4", testMovie("isom", "avc1", 640, 360, 7, false), 640, 360, 7, "isom", "avc1"},
		{"portrait mov, stored turned", testMovie("qt  ", "avc1", 640, 360, 3, true), 360, 640, 3, "qt  ", "avc1"},
		{"hevc mov", testMovie("qt  ", "hvc1", 320, 240, 2, false), 320, 240, 2, "qt  ", "hvc1"},
	}
	for _, c := range cases {
		path := filepath.Join(dir, "v")
		os.WriteFile(path, c.data, 0o644)
		info, err := readVideoInfo(path)
		if err != nil {
			t.Fatalf("%s: %v", c.name, err)
		}
		if info.width != c.w || info.height != c.h || info.seconds != c.secs || info.brand != c.brand || info.codec != c.kind {
			t.Errorf("%s: got %+v", c.name, info)
		}
	}

	path := filepath.Join(dir, "turned.mov")
	before := testMovie("qt  ", "avc1", 640, 360, 3, true)
	os.WriteFile(path, before, 0o644)
	if err := mp4Brand(path); err != nil {
		t.Fatal(err)
	}
	after, _ := os.ReadFile(path)
	if len(after) != len(before) || !bytes.Equal(after[20:], before[20:]) {
		t.Fatal("rebranding changed more than the ftyp box")
	}
	if info, err := readVideoInfo(path); err != nil || info.brand != "mp42" || info.width != 360 {
		t.Fatalf("after rebranding: %+v %v", info, err)
	}
}
