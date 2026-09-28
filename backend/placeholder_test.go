package main

import (
	"encoding/hex"
	"strings"
	"testing"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"google.golang.org/protobuf/proto"
)

// The raw bytes of a PayPal login code as a linked device receives it: only
// device-list metadata and a MASK_LINKED_DEVICES placeholder.
func TestPlaceholderMessageIsExplained(t *testing.T) {
	raw, _ := hex.DecodeString("9A02160A12420A1E59D3C64F3E1D6168DF48F6D3CFD50610028205020800")
	msg := &waE2E.Message{}
	if err := proto.Unmarshal(raw, msg); err != nil {
		t.Fatal(err)
	}
	if msg.GetPlaceholderMessage().GetType() != waE2E.PlaceholderMessage_MASK_LINKED_DEVICES {
		t.Fatalf("fixture isn't a masked placeholder: %v", msg)
	}
	if text := extractTextRaw(msg); !strings.Contains(text, "only be seen on your phone") {
		t.Fatalf("got %q", text)
	}
}
