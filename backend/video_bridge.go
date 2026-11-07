package main

import (
	"encoding/binary"
	"io"
	"log"
	"net"
	"sync"
	"time"
)

type VideoBridge struct {
	listener net.Listener
	send     func(accessUnit []byte, duration time.Duration) error
	closed   bool
	closedMu sync.Mutex
}

func NewVideoBridge(send func(accessUnit []byte, duration time.Duration) error) (*VideoBridge, int, error) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return nil, 0, err
	}
	port := listener.Addr().(*net.TCPAddr).Port
	return &VideoBridge{listener: listener, send: send}, port, nil
}

func (vb *VideoBridge) Run() {
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
		go vb.readFrames(conn)
	}
}

func (vb *VideoBridge) readFrames(conn net.Conn) {
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
		if err := vb.send(payload, time.Duration(durationMs)*time.Millisecond); err != nil {
			log.Printf("Failed to send video frame: %v\n", err)
		}
	}
}

func (vb *VideoBridge) Close() error {
	vb.closedMu.Lock()
	vb.closed = true
	vb.closedMu.Unlock()
	return vb.listener.Close()
}
