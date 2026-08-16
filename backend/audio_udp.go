package main

import (
	"encoding/binary"
	"log"
	"math"
	"net"
	"sync"
	"time"
	"github.com/purpshell/meowcaller"
)

// readTimeout bounds how long ReadFrame will wait for a UDP packet from the
// frontend before handing the engine a silence frame instead. meowcaller's
// engine pulls a frame every 60ms (FrameSamples cadence) and its own docs
// warn that "the call must keep sending to hold the relay bridge" - blocking
// that read loop indefinitely (e.g. while the frontend's audio streams are
// still spinning up, or briefly stall) risked the relay dropping the call
// entirely instead of just a moment of silence.
const readTimeout = 100 * time.Millisecond

type UDPBridge struct {
	conn       *net.UDPConn
	peerAddr   *net.UDPAddr
	peerAddrMu sync.Mutex
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
	buf := make([]byte, meowcaller.FrameSamples*4)
	n, raddr, err := b.conn.ReadFromUDP(buf)
	if err != nil {
		// meowcaller's Player treats any non-nil ReadFrame error as fatal
		// and permanently ends outbound audio for the rest of the call
		// (see player.go's nextFrame). A live loopback UDP bridge has no
		// legitimate "end of stream", so there's no error here that should
		// ever be terminal - notably including WSAECONNRESET, which Windows
		// can hand back on a UDP socket after an unrelated ICMP Port
		// Unreachable, with nothing to do with whether the call is still
		// up (the same quirk hit the frontend's receive side; see
		// call_audio.rs). Treat every read error as transient.
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

	frame := make([]float32, meowcaller.FrameSamples)
	for i := 0; i < n/4; i++ {
		if i < meowcaller.FrameSamples {
			frame[i] = math.Float32frombits(binary.LittleEndian.Uint32(buf[i*4:]))
		}
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
