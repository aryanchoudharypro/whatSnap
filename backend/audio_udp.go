package main

import (
	"encoding/binary"
	"github.com/purpshell/meowcaller"
	"log"
	"math"
	"net"
	"sync"
	"time"
)

// readTimeout bounds how long ReadFrame will wait for a UDP packet from the
// frontend before handing the engine a silence frame instead. meowcaller's
// engine pulls a frame every 60ms (FrameSamples cadence) and its own docs
// warn that "the call must keep sending to hold the relay bridge" - blocking
// that read loop indefinitely (e.g. while the frontend's audio streams are
// still spinning up, or briefly stall) risked the relay dropping the call
// entirely instead of just a moment of silence.
const readTimeout = 100 * time.Millisecond

const maxOpusPacket = 1275

type UDPBridge struct {
	conn       *net.UDPConn
	peerAddr   *net.UDPAddr
	peerAddrMu sync.Mutex
	opusFrame  []byte
	opusFrames [][]byte
	readBuf    []byte
	frameBuf   []float32
	writeBuf   []byte
}

var _ meowcaller.OpusFrameSource = (*UDPBridge)(nil)
var _ meowcaller.OpusFramesSource = (*UDPBridge)(nil)

func splitMicDatagram(datagram []byte) (pcm, opusFrame []byte) {
	const pcmBytes = meowcaller.FrameSamples * 4
	if len(datagram) <= pcmBytes {
		return datagram, nil
	}
	return datagram[:pcmBytes], datagram[pcmBytes:]
}

func (b *UDPBridge) OpusFrame() []byte {
	return b.opusFrame
}

// OpusFrames is the last frame as WhatsApp-sized 20 ms Opus packets.
func (b *UDPBridge) OpusFrames() [][]byte {
	return b.opusFrames
}

// splitOpusParts reads the packets after the PCM in a mic datagram: each is a
// 16-bit little-endian length and then the Opus packet.
func splitOpusParts(rest []byte) [][]byte {
	var parts [][]byte
	for len(rest) >= 2 {
		n := int(binary.LittleEndian.Uint16(rest))
		rest = rest[2:]
		if n > len(rest) {
			return nil
		}
		parts = append(parts, rest[:n])
		rest = rest[n:]
	}
	return parts
}

func NewUDPBridge() (*UDPBridge, int, error) {
	addr, err := net.ResolveUDPAddr("udp", "127.0.0.1:0")
	if err != nil {
		return nil, 0, err
	}
	conn, err := net.ListenUDP("udp", addr)
	if err != nil {
		return nil, 0, err
	}
	return &UDPBridge{
		conn:     conn,
		readBuf:  make([]byte, meowcaller.FrameSamples*4+maxOpusPacket),
		frameBuf: make([]float32, meowcaller.FrameSamples),
		writeBuf: make([]byte, meowcaller.FrameSamples*4),
	}, conn.LocalAddr().(*net.UDPAddr).Port, nil
}

func (b *UDPBridge) ReadFrame() ([]float32, error) {
	b.conn.SetReadDeadline(time.Now().Add(readTimeout))
	b.opusFrame = nil
	b.opusFrames = nil
	n, raddr, err := b.conn.ReadFromUDP(b.readBuf)
	if err != nil {
		if netErr, ok := err.(net.Error); !ok || !netErr.Timeout() {
			log.Printf("UDPBridge read error (continuing): %v", err)
		}
		for i := range b.frameBuf {
			b.frameBuf[i] = 0
		}
		return b.frameBuf, nil
	}

	b.peerAddrMu.Lock()
	if b.peerAddr == nil {
		b.peerAddr = raddr
	}
	b.peerAddrMu.Unlock()

	pcm, opusRest := splitMicDatagram(b.readBuf[:n])
	b.opusFrames = splitOpusParts(opusRest)
	b.opusFrame = nil
	if len(b.opusFrames) == 1 {
		b.opusFrame = b.opusFrames[0]
	}
	for i := range b.frameBuf {
		b.frameBuf[i] = 0
	}
	for i := 0; i < len(pcm)/4; i++ {
		b.frameBuf[i] = math.Float32frombits(binary.LittleEndian.Uint32(pcm[i*4:]))
	}
	return b.frameBuf, nil
}

func (b *UDPBridge) Close() error {
	return b.conn.Close()
}

func (b *UDPBridge) WriteFrame(frame []float32) error {
	b.peerAddrMu.Lock()
	paddr := b.peerAddr
	b.peerAddrMu.Unlock()

	if paddr == nil {
		// We don't know where to send yet, drop the frame
		return nil
	}

	buf := b.writeBuf
	if len(buf) < len(frame)*4 {
		buf = make([]byte, len(frame)*4)
		b.writeBuf = buf
	}
	buf = buf[:len(frame)*4]
	for i, f := range frame {
		binary.LittleEndian.PutUint32(buf[i*4:], math.Float32bits(f))
	}
	_, err := b.conn.WriteToUDP(buf, paddr)
	return err
}
