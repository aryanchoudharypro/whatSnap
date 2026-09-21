package main

import (
	"encoding/binary"
	"io"
	"log"
	"net"
	"sync"
	"time"

	"github.com/purpshell/meowcaller"
)

// VideoBridge carries outbound H.264 access units from the Rust frontend's
// camera encoder to meowcaller over a TCP loopback socket. UDP (as used for
// audio, see audio_udp.go) can't be reused here: a single datagram is capped
// at 65,507 bytes and an IDR/keyframe access unit routinely exceeds that.
//
// Wire format per frame, written by the frontend and read here:
//
//	uint32 BE payload length
//	uint32 BE frame duration in milliseconds (0 = use meowcaller's fallback step)
//	payload bytes (one Annex-B H.264 access unit)
//
// The frontend only connects while the local camera is on; between calls (or
// while the camera is off) nothing is connected and ReadLoop simply waits on
// Accept again once a connection closes.
type VideoBridge struct {
	listener net.Listener
	closed   bool
	closedMu sync.Mutex
}

// NewVideoBridge opens a TCP listener on an ephemeral loopback port and
// returns it along with the port number to hand to the frontend.
func NewVideoBridge() (*VideoBridge, int, error) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return nil, 0, err
	}
	port := listener.Addr().(*net.TCPAddr).Port
	return &VideoBridge{listener: listener}, port, nil
}

// Run accepts connections from the frontend's camera encoder for the
// lifetime of the call, forwarding every frame it reads to call.
// It blocks until the bridge is closed, so callers should run it in its own
// goroutine.
func (vb *VideoBridge) Run(call *meowcaller.Call) {
	for {
		conn, err := vb.listener.Accept()
		if err != nil {
			vb.closedMu.Lock()
			closed := vb.closed
			vb.closedMu.Unlock()
			if closed {
				return
			}
			log.Printf("VideoBridge accept error (continuing): %v\n", err)
			time.Sleep(50 * time.Millisecond)
			continue
		}
		vb.readFrames(conn, call)
	}
}

func (vb *VideoBridge) readFrames(conn net.Conn, call *meowcaller.Call) {
	defer conn.Close()
	header := make([]byte, 8)
	for {
		if _, err := io.ReadFull(conn, header); err != nil {
			if err != io.EOF {
				log.Printf("VideoBridge header read error: %v\n", err)
			}
			return
		}
		payloadLen := binary.BigEndian.Uint32(header[0:4])
		durationMs := binary.BigEndian.Uint32(header[4:8])
		// A malformed or wildly oversized length would otherwise make
		// ReadFull allocate and block on an attacker-sized buffer, but this
		// is a loopback-only socket fed by our own frontend, so a generous
		// sanity cap (4 MiB) is just a guard against a framing bug, not a
		// security boundary.
		const maxPayload = 4 << 20
		if payloadLen > maxPayload {
			log.Printf("VideoBridge frame too large (%d bytes), dropping connection\n", payloadLen)
			return
		}
		payload := make([]byte, payloadLen)
		if _, err := io.ReadFull(conn, payload); err != nil {
			log.Printf("VideoBridge payload read error: %v\n", err)
			return
		}
		if err := call.SendVideoWithDuration(payload, time.Duration(durationMs)*time.Millisecond); err != nil {
			log.Printf("Failed to send video frame: %v\n", err)
		}
	}
}

// Close stops accepting new connections and unblocks Run.
func (vb *VideoBridge) Close() error {
	vb.closedMu.Lock()
	vb.closed = true
	vb.closedMu.Unlock()
	return vb.listener.Close()
}
