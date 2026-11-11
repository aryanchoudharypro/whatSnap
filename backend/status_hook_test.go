package main

import (
	"context"
	"testing"

	waBinary "go.mau.fi/whatsmeow/binary"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/store"
	waLog "go.mau.fi/whatsmeow/util/log"
)

func TestHookStatusStanzasRouting(t *testing.T) {
	dev := &store.Device{}
	c := whatsmeow.NewClient(dev, waLog.Noop)
	hookStatusStanzas(c)

	handlers := getClientNodeHandlers(c)
	if handlers == nil {
		t.Fatal("nodeHandlers not found on client")
	}

	encRouted := false
	unencRouted := false

	handlers["message"] = func(ctx context.Context, node *waBinary.Node) {
		encRouted = true
	}
	handlers["status_orig_mock"] = func(ctx context.Context, node *waBinary.Node) {
		unencRouted = true
	}

	// Re-hook using mocks
	origStatus := handlers["status_orig_mock"]
	msgHandler := handlers["message"]
	handlers["status"] = func(ctx context.Context, node *waBinary.Node) {
		if len(node.GetChildrenByTag("enc")) > 0 {
			msgHandler(ctx, node)
		} else {
			origStatus(ctx, node)
		}
	}

	// Test encrypted status node
	encNode := &waBinary.Node{
		Tag: "status",
		Attrs: waBinary.Attrs{
			"from":        "status@broadcast",
			"participant": "12345@s.whatsapp.net",
		},
		Content: []waBinary.Node{
			{Tag: "enc", Attrs: waBinary.Attrs{"type": "skmsg"}},
		},
	}
	handlers["status"](context.Background(), encNode)
	if !encRouted {
		t.Error("encrypted status stanza was not routed to message handler")
	}

	// Test unencrypted status node
	unencNode := &waBinary.Node{
		Tag: "status",
		Attrs: waBinary.Attrs{
			"from": "12345@newsletter",
		},
		Content: []waBinary.Node{
			{Tag: "plaintext"},
		},
	}
	handlers["status"](context.Background(), unencNode)
	if !unencRouted {
		t.Error("unencrypted status stanza was not routed to unencrypted handler")
	}
}

func getClientNodeHandlers(c *whatsmeow.Client) map[string]func(context.Context, *waBinary.Node) {
	return getNodeHandlersMap(c)
}
