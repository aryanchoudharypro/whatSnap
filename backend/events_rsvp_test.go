package main

import (
	"strings"
	"testing"
	"time"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"google.golang.org/protobuf/proto"
)

func rsvp(r waE2E.EventResponseMessage_EventResponseType, extra int32, ms int64) *waE2E.EventResponseMessage {
	return &waE2E.EventResponseMessage{Response: r.Enum(), ExtraGuestCount: proto.Int32(extra), TimestampMS: proto.Int64(ms)}
}

func TestEventResponsesKeepEachResponderLatestAnswer(t *testing.T) {
	openDisappearingTestDB(t)
	const event, alice, bob = "EVT1", "1@s.whatsapp.net", "2@s.whatsapp.net"
	storeEventResponse(db, event, alice, rsvp(waE2E.EventResponseMessage_GOING, 0, 2000), 0)
	storeEventResponse(db, event, alice, rsvp(waE2E.EventResponseMessage_NOT_GOING, 0, 1000), 0)
	storeEventResponse(db, event, bob, rsvp(waE2E.EventResponseMessage_MAYBE, 0, 1000), 0)
	storeEventResponse(db, event, bob, rsvp(waE2E.EventResponseMessage_GOING, 2, 3000), 0)

	got := eventResponses(event)
	if got[alice].response != waE2E.EventResponseMessage_GOING {
		t.Fatalf("alice: got %v, want going", got[alice].response)
	}
	if got[bob].response != waE2E.EventResponseMessage_GOING || got[bob].extraGuests != 2 {
		t.Fatalf("bob: got %+v, want going with 2 guests", got[bob])
	}
	if text := eventResponsesText(got, bob); text != "4 going (including 2 guests), 0 maybe, 0 can't go. You're going." {
		t.Fatalf("unexpected tally %q", text)
	}
}

func TestEventResponsesText(t *testing.T) {
	if got := eventResponsesText(nil, "me"); got != "No responses yet." {
		t.Fatalf("got %q", got)
	}
	responses := map[string]eventRSVP{
		"a":  {response: waE2E.EventResponseMessage_GOING},
		"b":  {response: waE2E.EventResponseMessage_NOT_GOING},
		"me": {response: waE2E.EventResponseMessage_MAYBE},
	}
	if got := eventResponsesText(responses, "me"); got != "1 going, 1 maybe, 1 can't go. You said maybe." {
		t.Fatalf("got %q", got)
	}
}

func TestEventResultsForStoredEvent(t *testing.T) {
	openDisappearingTestDB(t)
	raw, _ := proto.Marshal(&waE2E.Message{EventMessage: &waE2E.EventMessage{Name: proto.String("Picnic")}})
	cancelled, _ := proto.Marshal(&waE2E.Message{EventMessage: &waE2E.EventMessage{Name: proto.String("Off"), IsCanceled: proto.Bool(true)}})
	db.Exec(`INSERT INTO messages (id, jid, text, timestamp, raw) VALUES ('E1', 'g@g.us', '[Event] Picnic', 1, ?), ('E2', 'g@g.us', '[Event cancelled] Off', 1, ?)`, raw, cancelled)
	storeEventResponse(db, "E1", "1@s.whatsapp.net", rsvp(waE2E.EventResponseMessage_GOING, 0, 1000), 0)
	if got := pollResultsForMessage("E1"); got != "1 going, 0 maybe, 0 can't go." {
		t.Fatalf("got %q", got)
	}
	if got := pollResultsForMessage("E2"); got != "" {
		t.Fatalf("a cancelled event shows no tally, got %q", got)
	}
	purgeMessages("id = ?", "E1")
	if len(eventResponses("E1")) != 0 {
		t.Fatal("deleting an event must delete its RSVPs")
	}
}

func TestParseEventStart(t *testing.T) {
	loc := time.FixedZone("test", 5*3600+1800)
	got, err := parseEventStart("2026-10-05", "18:30", loc)
	if err != nil || !got.Equal(time.Date(2026, 10, 5, 18, 30, 0, 0, loc)) {
		t.Fatalf("got %v, %v", got, err)
	}
	got, err = parseEventStart("5/10/2026", "6:30 pm", loc)
	if err != nil || !got.Equal(time.Date(2026, 10, 5, 18, 30, 0, 0, loc)) {
		t.Fatalf("day-first date and 12-hour time: got %v, %v", got, err)
	}
	if got, err = parseEventStart("", "", loc); err != nil || !got.IsZero() {
		t.Fatalf("no date means no start time: got %v, %v", got, err)
	}
	for _, bad := range [][2]string{{"", "18:30"}, {"tomorrow", ""}, {"2026-10-05", "later"}} {
		if _, err := parseEventStart(bad[0], bad[1], loc); err == nil || !strings.Contains(err.Error(), "type") {
			t.Errorf("%q %q: expected an error, got %v", bad[0], bad[1], err)
		}
	}
}
