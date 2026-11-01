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
}

var _ meowcaller.OpusFrameSource = (*UDPBridge)(nil)

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

func NewUDPBridge() (*UDPBridge, int, error) {
	addr, err := net.ResolveUDPAddr("udp", "127.0.0.1:0")
	if err != nil {
		return nil, 0, err
	}
	conn, err := net.ListenUDP("udp", addr)
	if err != nil {
		return nil, 0, err
	}
	return &UDPBridge{conn: conn}, conn.LocalAddr().(*net.UDPAddr).Port, nil
}

func (b *UDPBridge) ReadFrame() ([]float32, error) {
	b.conn.SetReadDeadline(time.Now().Add(readTimeout))
	b.opusFrame = nil
	buf := make([]byte, meowcaller.FrameSamples*4+maxOpusPacket)
	n, raddr, err := b.conn.ReadFromUDP(buf)
	if err != nil {
		if netErr, ok := err.(net.Error); !ok || !netErr.Timeout() {
			log.Printf("UDPBridge read error (continuing): %v", err)
		}
		return make([]float32, meowcaller.FrameSamples), nil
	}

	b.peerAddrMu.Lock()
	if b.peerAddr == nil {
		b.peerAddr = raddr
	}
	b.peerAddrMu.Unlock()

	pcm, opusFrame := splitMicDatagram(buf[:n])
	b.opusFrame = opusFrame
	frame := make([]float32, meowcaller.FrameSamples)
	for i := 0; i < len(pcm)/4; i++ {
		frame[i] = math.Float32frombits(binary.LittleEndian.Uint32(pcm[i*4:]))
	}
	return frame, nil
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

	buf := make([]byte, len(frame)*4)
	for i, f := range frame {
		binary.LittleEndian.PutUint32(buf[i*4:], math.Float32bits(f))
	}
	_, err := b.conn.WriteToUDP(buf, paddr)
	return err
}
