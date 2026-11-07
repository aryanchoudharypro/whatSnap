package main

import (
	"bytes"
	"encoding/binary"
	"math"
	"net"
	"testing"

	"github.com/purpshell/meowcaller"
)

func TestUDPBridgeHandsOverTheOpusPacketThatCameWithTheFrame(t *testing.T) {
	bridge, port, err := NewUDPBridge()
	if err != nil {
		t.Fatalf("NewUDPBridge: %v", err)
	}
	defer bridge.Close()
	frontend, err := net.DialUDP("udp", nil, &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: port})
	if err != nil {
		t.Fatalf("dial bridge: %v", err)
	}
	defer frontend.Close()

	pcm := make([]byte, meowcaller.FrameSamples*4)
	for i := 0; i < meowcaller.FrameSamples; i++ {
		binary.LittleEndian.PutUint32(pcm[i*4:], math.Float32bits(float32(i)/1000))
	}
	opusFrame := []byte{0x58, 1, 2, 3, 4}

	if _, err = frontend.Write(append(append([]byte(nil), pcm...), opusFrame...)); err != nil {
		t.Fatalf("send frame: %v", err)
	}
	frame, err := bridge.ReadFrame()
	if err != nil || len(frame) != meowcaller.FrameSamples || frame[500] != 0.5 {
		t.Fatalf("ReadFrame = %d samples, err %v", len(frame), err)
	}
	if got := bridge.OpusFrame(); !bytes.Equal(got, opusFrame) {
		t.Fatalf("OpusFrame = %x, want %x", got, opusFrame)
	}

	if _, err = frontend.Write(pcm); err != nil {
		t.Fatalf("send PCM frame: %v", err)
	}
	if frame, err = bridge.ReadFrame(); err != nil || frame[500] != 0.5 {
		t.Fatalf("ReadFrame (PCM only) err %v", err)
	}
	if got := bridge.OpusFrame(); got != nil {
		t.Fatalf("OpusFrame after a PCM-only frame = %x, want none", got)
	}

	if _, err = frontend.Write(append(append([]byte(nil), pcm...), opusFrame...)); err != nil {
		t.Fatalf("send frame: %v", err)
	}
	if _, err = bridge.ReadFrame(); err != nil {
		t.Fatalf("ReadFrame: %v", err)
	}
	frame, err = bridge.ReadFrame()
	if err != nil || len(frame) != meowcaller.FrameSamples || frame[500] != 0 {
		t.Fatalf("ReadFrame on timeout = %d samples, err %v", len(frame), err)
	}
	if got := bridge.OpusFrame(); got != nil {
		t.Fatalf("OpusFrame for a silence frame = %x, want none", got)
	}

	if pcmPart, opusPart := splitMicDatagram(make([]byte, 4)); len(pcmPart) != 4 || opusPart != nil {
		t.Fatalf("hello datagram split into %d PCM bytes and %d Opus bytes", len(pcmPart), len(opusPart))
	}
}

func TestSplitOpusPartsReadsLengthPrefixedPackets(t *testing.T) {
	rest := []byte{2, 0, 0x48, 1, 3, 0, 0x48, 2, 3, 1, 0, 0x48}
	parts := splitOpusParts(rest)
	if len(parts) != 3 || len(parts[0]) != 2 || len(parts[1]) != 3 || len(parts[2]) != 1 {
		t.Fatalf("parts = %v", parts)
	}
	if splitOpusParts([]byte{9, 0, 1}) != nil {
		t.Fatal("a length past the end of the datagram must give no packets")
	}
}
