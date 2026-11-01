package main

import (
	"encoding/json"
	"net"
	"testing"
	"time"
)

func envelopeFor(t *testing.T, evtType string, data interface{}) []byte {
	t.Helper()
	server, client := net.Pipe()
	got := make(chan []byte, 1)
	go func() {
		buf := make([]byte, 1<<16)
		n, _ := client.Read(buf)
		got <- append([]byte(nil), buf[:n]...)
	}()
	go sendEvent(server, evtType, data)

	select {
	case b := <-got:
		return b
	case <-time.After(5 * time.Second):
		t.Fatal("sendEvent wrote nothing")
		return nil
	}
}

func TestSendEventMatchesMarshalledEnvelope(t *testing.T) {
	cases := []struct {
		name    string
		evtType string
		data    interface{}
	}{
		{"no data", "connected", nil},
		{"object payload", "conversation_messages", map[string]interface{}{
			"jid":      "a@s.whatsapp.net",
			"messages": []MessageJSON{{ID: "m1", Text: "hi", Timestamp: 3, Reactions: emptyReactions}},
		}},
		{"empty message list", "conversation_messages", map[string]interface{}{
			"jid": "a@s.whatsapp.net", "messages": []MessageJSON{},
		}},
		// Characters that must be escaped identically by both paths.
		{"type needing escapes", `we"ird\type`, map[string]interface{}{"k": "v"}},
		{"payload needing escapes", "message", map[string]interface{}{
			"text": "quote\" backslash " + `\` + " newline\n tab\t unicode—",
		}},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := envelopeFor(t, tc.evtType, tc.data)

			var raw json.RawMessage
			if tc.data != nil {
				b, err := json.Marshal(tc.data)
				if err != nil {
					t.Fatalf("marshal payload: %v", err)
				}
				raw = b
			}
			want, err := json.Marshal(Event{Type: tc.evtType, Data: raw})
			if err != nil {
				t.Fatalf("marshal event: %v", err)
			}
			want = append(want, '\n')

			if string(got) != string(want) {
				t.Errorf("envelope mismatch:\n got %s\nwant %s", got, want)
			}
		})
	}
}

// Every frame must be exactly one line: the UI reads the pipe with read_line.
func TestSendEventWritesOneNewlineTerminatedLine(t *testing.T) {
	got := envelopeFor(t, "conversation_messages", map[string]interface{}{
		"messages": []MessageJSON{{ID: "m1", Text: "line\nbreak", Reactions: emptyReactions}},
	})
	if n := len(got); n == 0 || got[n-1] != '\n' {
		t.Fatalf("frame is not newline terminated: %q", got)
	}
	if got := countNewlines(got); got != 1 {
		t.Errorf("frame contains %d newlines, want exactly 1 (embedded ones must be escaped)", got)
	}
}

func countNewlines(b []byte) int {
	n := 0
	for _, c := range b {
		if c == '\n' {
			n++
		}
	}
	return n
}
