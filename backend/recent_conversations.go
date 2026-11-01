package main

import (
	"context"
	"log"

	"google.golang.org/protobuf/encoding/prototext"
	"google.golang.org/protobuf/proto"

	"go.mau.fi/whatsmeow/proto/waE2E"
)

// Recent conversations on demand: a peer request (any time, not only at
// linking) asking the phone for its N most recent chats with up to M
// messages each. WA Web's own code says it doesn't support this, so
// whether a phone answers a Windows-app companion is an experiment; an
// answer arrives as a history sync of type RECENT_CONVERSATIONS_ON_DEMAND
// and is stored like any other.
const (
	recentConversationsCount   = 200
	recentConversationsPerChat = 30000
)

func handleRequestRecentConversations() {
	c := wa()
	if c == nil || c.Store.ID == nil {
		return
	}
	req := &waE2E.Message{
		ProtocolMessage: &waE2E.ProtocolMessage{
			Type: waE2E.ProtocolMessage_PEER_DATA_OPERATION_REQUEST_MESSAGE.Enum(),
			PeerDataOperationRequestMessage: &waE2E.PeerDataOperationRequestMessage{
				PeerDataOperationRequestType: waE2E.PeerDataOperationRequestType_RECENT_CONVERSATIONS_ON_DEMAND_REQUEST.Enum(),
				RecentConversationsOnDemandRequest: &waE2E.PeerDataOperationRequestMessage_RecentConversationsOnDemandRequest{
					ConversationCount:       proto.Uint32(recentConversationsCount),
					MessagesPerConversation: proto.Uint32(recentConversationsPerChat),
				},
			},
		},
	}
	go func() {
		historyRequestMu.Lock()
		resp, err := c.SendPeerMessage(context.Background(), req)
		historyRequestMu.Unlock()
		if err != nil {
			log.Printf("Recent chats request failed: %v", err)
			broadcastEvent("error", map[string]string{"message": "Couldn't ask your phone for recent chats: " + err.Error()})
			return
		}
		log.Printf("Asked the phone for its %d most recent chats, %d messages each (request %s)", recentConversationsCount, recentConversationsPerChat, resp.ID)
	}()
}

// logPeerDataResponse notes what the phone answered to a recent chats
// request, so a log shows whether it is supported at all.
func logPeerDataResponse(msg *waE2E.Message) {
	resp := msg.GetProtocolMessage().GetPeerDataOperationRequestResponseMessage()
	if resp.GetPeerDataOperationRequestType() != waE2E.PeerDataOperationRequestType_RECENT_CONVERSATIONS_ON_DEMAND_REQUEST {
		return
	}
	log.Printf("Phone answered the recent chats request %s: %s", resp.GetStanzaID(), prototext.Format(resp))
}
