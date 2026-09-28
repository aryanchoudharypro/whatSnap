package main

import (
	"testing"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/types"
)

func TestBuildSpamReport(t *testing.T) {
	contact := types.NewJID("15550001111", types.DefaultUserServer)
	m := reportableMessage{id: "M1", sender: contact.String(), mediaType: "voice", ts: 1700000000, raw: []byte{1}}
	report := buildSpamReport(contact, m, false, "")
	if report.From != contact || report.Flow != whatsmeow.SpamFlowMessageMenu || report.MediaType != "ptt" || !report.Group.IsEmpty() {
		t.Fatalf("unexpected 1:1 message report %+v", report)
	}
	if buildSpamReport(contact, m, true, "").Flow != whatsmeow.SpamFlowContactInfo {
		t.Fatal("reporting a contact's chat uses the contact info flow")
	}

	group := types.NewJID("120363000000000000", types.GroupServer)
	report = buildSpamReport(group, m, true, "Neighbours")
	if report.Group != group || report.GroupSubject != "Neighbours" || report.Participant != contact ||
		report.Flow != whatsmeow.SpamFlowGroupInfoReport {
		t.Fatalf("unexpected group report %+v", report)
	}
}

func TestReportableMessages(t *testing.T) {
	openDisappearingTestDB(t)
	db.Exec(`INSERT INTO messages (id, jid, sender_jid, from_me, text, timestamp, media_type) VALUES
		('THEIRS', 'c@s.whatsapp.net', 'c@s.whatsapp.net', 0, 'hi', 10, ''),
		('MINE', 'c@s.whatsapp.net', '', 1, 'hello', 20, ''),
		('CALL', 'c@s.whatsapp.net', 'c@s.whatsapp.net', 0, 'Missed call', 30, 'call')`)
	if m, ok := latestReportable("c@s.whatsapp.net"); !ok || m.id != "THEIRS" {
		t.Fatalf("a chat report names their latest real message, got %+v %v", m, ok)
	}
	if _, ok := loadReportable("c@s.whatsapp.net", "MINE"); ok {
		t.Fatal("your own message can't be reported")
	}
	if _, ok := latestReportable("empty@s.whatsapp.net"); ok {
		t.Fatal("nothing to report in an empty chat")
	}
}
