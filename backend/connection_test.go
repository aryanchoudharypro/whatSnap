package main

import (
	"errors"
	"net"
	"testing"
	"time"
)

func TestConnBackoffDoublesThenCaps(t *testing.T) {
	want := []time.Duration{2, 4, 8, 16, 32, 60, 60, 60}
	for i, w := range want {
		if got := connBackoff(int32(i + 1)); got != w*time.Second {
			t.Errorf("connBackoff(%d) = %v, want %v", i+1, got, w*time.Second)
		}
	}
	if got := connBackoff(1000); got != connBackoffMax {
		t.Errorf("connBackoff(1000) = %v, want cap %v", got, connBackoffMax)
	}
}

func TestDescribeConnError(t *testing.T) {
	offline := "Can't reach WhatsApp. Check your internet connection."
	cases := []struct {
		err  error
		want string
	}{
		{&net.DNSError{Err: "no such host", Name: "web.whatsapp.com"}, offline},
		{&net.OpError{Op: "dial", Err: errors.New("unreachable")}, offline},
		{errors.New("i/o timeout"), offline},
		{errors.New("noise handshake failed: bad frame"), "Couldn't complete the secure connection to WhatsApp."},
		{errors.New("something else"), "Couldn't connect to WhatsApp."},
	}
	for _, c := range cases {
		if got := describeConnError(c.err); got != c.want {
			t.Errorf("describeConnError(%v) = %q, want %q", c.err, got, c.want)
		}
	}
}
