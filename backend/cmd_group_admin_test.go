package main

import (
	"testing"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/types"
)

func TestReportedMessageEntry(t *testing.T) {
	openDisappearingTestDB(t)
	acc.invalidateDisplayNames()
	acc.db.Exec(`INSERT INTO messages (id, jid, sender_jid, from_me, text, timestamp) VALUES ('R1', 'g@g.us', '', 0, 'buy now', 1)`)
	hidden := types.NewJID("99", types.HiddenUserServer)
	entry := acc.reportedMessageEntry("g@g.us", whatsmeow.ReportedGroupMessage{
		MessageID: "R1",
		Reporters: []whatsmeow.GroupMessageReporter{{JID: hidden, Username: "sam", Timestamp: time.Unix(1, 0)}},
	})
	if entry["text"] != "buy now" || entry["stored"] != true {
		t.Fatalf("unexpected entry %v", entry)
	}
	if reporters := entry["reporters"].([]string); len(reporters) != 1 || reporters[0] == "" {
		t.Fatalf("each reporter gets a name, got %v", reporters)
	}
	missing := acc.reportedMessageEntry("g@g.us", whatsmeow.ReportedGroupMessage{MessageID: "GONE"})
	if missing["stored"] != false {
		t.Fatalf("a message not stored here is marked so, got %v", missing)
	}
}
