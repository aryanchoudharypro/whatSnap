package main

import (
	"encoding/binary"
	"math"
	"net"
	"sync"
	"github.com/purpshell/meowcaller"
)

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
	buf := make([]byte, meowcaller.FrameSamples*4)
	n, raddr, err := b.conn.ReadFromUDP(buf)
	if err != nil {
		return nil, err
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
