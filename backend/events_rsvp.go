package main

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"net"
	"strings"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/proto/waWeb"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

// Events and their RSVPs. An RSVP is sealed with the event's message secret
// like a poll vote, and each responder's latest answer replaces their
// earlier one, so event_responses keeps one row per event and responder. The
// tally is shown after the event the way a poll's results are (the
// pollResults line), and V on an event answers it.

// eventRSVP is one responder's answer.
type eventRSVP struct {
	response    waE2E.EventResponseMessage_EventResponseType
	extraGuests int
}

// eventResponseNames maps the RSVP names the frontend sends to their types.
var eventResponseNames = map[string]waE2E.EventResponseMessage_EventResponseType{
	"going":     waE2E.EventResponseMessage_GOING,
	"not_going": waE2E.EventResponseMessage_NOT_GOING,
	"maybe":     waE2E.EventResponseMessage_MAYBE,
}

func eventResponseName(r waE2E.EventResponseMessage_EventResponseType) string {
	for name, t := range eventResponseNames {
		if t == r {
			return name
		}
	}
	return ""
}

// loadEvent reads a stored event back from its message.
func loadEvent(eventID string) (*waE2E.EventMessage, bool) {
	var raw []byte
	if db.QueryRow(`SELECT raw FROM messages WHERE id = ?`, eventID).Scan(&raw) != nil || len(raw) == 0 {
		return nil, false
	}
	msg := &waE2E.Message{}
	if proto.Unmarshal(raw, msg) != nil {
		return nil, false
	}
	event := unwrapMessage(msg).GetEventMessage()
	return event, event != nil
}

func storeEventResponse(ex sqlExecer, eventID, responder string, resp *waE2E.EventResponseMessage, ts int64) {
	if eventID == "" || responder == "" || resp == nil {
		return
	}
	if ms := resp.GetTimestampMS(); ms > 0 {
		ts = ms / 1000
	}
	if _, err := ex.Exec(`INSERT INTO event_responses (event_id, responder, response, extra_guests, timestamp) VALUES (?, ?, ?, ?, ?)
		ON CONFLICT(event_id, responder) DO UPDATE SET response=excluded.response, extra_guests=excluded.extra_guests, timestamp=excluded.timestamp
		WHERE excluded.timestamp >= event_responses.timestamp`,
		eventID, responder, int32(resp.GetResponse()), resp.GetExtraGuestCount(), ts); err != nil {
		log.Printf("storing RSVP for event %s failed: %v", eventID, err)
	}
}

// eventResponses returns each responder's current answer.
func eventResponses(eventID string) map[string]eventRSVP {
	responses := map[string]eventRSVP{}
	rows, err := db.Query(`SELECT responder, response, extra_guests FROM event_responses WHERE event_id = ?`, eventID)
	if err != nil {
		return responses
	}
	defer rows.Close()
	for rows.Next() {
		var responder string
		var response int32
		var extra int
		if rows.Scan(&responder, &response, &extra) == nil {
			responses[responder] = eventRSVP{waE2E.EventResponseMessage_EventResponseType(response), extra}
		}
	}
	return responses
}

// eventResponsesText is the line shown after an event: how many are going
// (guests they bring included), maybe and not going, and your own answer.
func eventResponsesText(responses map[string]eventRSVP, me string) string {
	var going, guests, maybe, notGoing int
	for _, r := range responses {
		switch r.response {
		case waE2E.EventResponseMessage_GOING:
			going++
			guests += r.extraGuests
		case waE2E.EventResponseMessage_MAYBE:
			maybe++
		case waE2E.EventResponseMessage_NOT_GOING:
			notGoing++
		}
	}
	if going+maybe+notGoing == 0 {
		return "No responses yet."
	}
	goingText := fmt.Sprintf("%d going", going+guests)
	switch {
	case guests == 1:
		goingText += " (including 1 guest)"
	case guests > 1:
		goingText += fmt.Sprintf(" (including %d guests)", guests)
	}
	text := fmt.Sprintf("%s, %d maybe, %d can't go.", goingText, maybe, notGoing)
	switch responses[me].response {
	case waE2E.EventResponseMessage_GOING:
		text += " You're going."
	case waE2E.EventResponseMessage_MAYBE:
		text += " You said maybe."
	case waE2E.EventResponseMessage_NOT_GOING:
		text += " You can't go."
	}
	return text
}

// eventResultsForMessage is the RSVP line for a stored event ("" if id isn't
// one, or it's cancelled).
func eventResultsForMessage(id string) string {
	event, ok := loadEvent(id)
	if !ok || event.GetIsCanceled() {
		return ""
	}
	return eventResponsesText(eventResponses(id), ownVoterKey())
}

// decryptEventResponse decrypts an RSVP even when it arrived wrapped
// (ephemeral, device-sent...): DecryptEventResponse only looks at the
// top-level message.
func decryptEventResponse(v *events.Message) (*waE2E.EventResponseMessage, error) {
	evt := *v
	evt.Message = unwrapMessage(v.Message)
	return wa().DecryptEventResponse(context.Background(), &evt)
}

// applyEventResponse handles a live RSVP: decrypt it with the event's
// secret, record it, and refresh the event's tally.
func applyEventResponse(v *events.Message) {
	eventID := unwrapMessage(v.Message).GetEncEventResponseMessage().GetEventCreationMessageKey().GetID()
	resp, err := decryptEventResponse(v)
	if err != nil {
		log.Printf("RSVP to event %s couldn't be decrypted: %v", eventID, err)
		return
	}
	storeEventResponse(db, eventID, pollVoterKey(v.Info.Sender), resp, v.Info.Timestamp.Unix())
	chat, _ := resolveLIDChat(v)
	broadcastPollResults(chat.String(), eventID)
}

// historyEventResponses records the RSVPs history sync delivers with an
// event - already decrypted, one per responder.
func historyEventResponses(exec sqlExecer, chat types.JID, webMsg *waWeb.WebMessageInfo) {
	eventID := webMsg.GetKey().GetID()
	for _, er := range webMsg.GetEventResponses() {
		key := er.GetEventResponseMessageKey()
		var responder string
		switch {
		case key.GetFromMe():
			responder = ownVoterKey()
		case key.GetParticipant() != "":
			if j, err := types.ParseJID(key.GetParticipant()); err == nil {
				responder = pollVoterKey(j)
			}
		default:
			if j, err := types.ParseJID(key.GetRemoteJID()); err == nil {
				responder = pollVoterKey(j)
			} else {
				responder = pollVoterKey(chat)
			}
		}
		storeEventResponse(exec, eventID, responder, er.GetEventResponseMessage(), er.GetTimestampMS()/1000)
	}
}

type EventRefData struct {
	JID         string `json:"jid"`
	MessageID   string `json:"message_id"`
	Response    string `json:"response"`
	ExtraGuests int    `json:"extra_guests"`
}

// handleGetEvent sends the frontend an event and our current answer, for
// the RSVP dialog.
func handleGetEvent(conn net.Conn, raw json.RawMessage) {
	var data EventRefData
	if json.Unmarshal(raw, &data) != nil {
		return
	}
	event, ok := loadEvent(data.MessageID)
	if !ok {
		sendEvent(conn, "error", map[string]string{"message": "This event can't be opened here."})
		return
	}
	if event.GetIsCanceled() {
		sendEvent(conn, "error", map[string]string{"message": "This event was cancelled."})
		return
	}
	mine := eventResponses(data.MessageID)[ownVoterKey()]
	when := ""
	if start := event.GetStartTime(); start > 0 {
		when = time.Unix(start, 0).Format("Monday 2 January 2006, 15:04")
	}
	sendEvent(conn, "event_details", map[string]interface{}{
		"jid":                  data.JID,
		"message_id":           data.MessageID,
		"name":                 strings.TrimSpace(event.GetName()),
		"when":                 when,
		"location":             strings.TrimSpace(event.GetLocation().GetName()),
		"response":             eventResponseName(mine.response),
		"extra_guests":         mine.extraGuests,
		"extra_guests_allowed": event.GetExtraGuestsAllowed(),
		"results":              eventResultsForMessage(data.MessageID),
	})
}

// handleSendEventResponse answers an event: going, not going or maybe,
// optionally bringing guests when the event allows it.
func handleSendEventResponse(conn net.Conn, raw json.RawMessage) {
	var data EventRefData
	if json.Unmarshal(raw, &data) != nil {
		return
	}
	go func() {
		fail := func(msg string) { sendEvent(conn, "error", map[string]string{"message": msg}) }
		response, known := eventResponseNames[data.Response]
		if !known {
			fail("Choose going, maybe or not going.")
			return
		}
		event, ok := loadEvent(data.MessageID)
		if !ok {
			fail("This event can't be answered here.")
			return
		}
		if event.GetIsCanceled() {
			fail("This event was cancelled.")
			return
		}
		extra := data.ExtraGuests
		if response != waE2E.EventResponseMessage_GOING || !event.GetExtraGuestsAllowed() || extra < 0 {
			extra = 0
		}
		chat, err := types.ParseJID(data.JID)
		if err != nil {
			fail("invalid jid")
			return
		}
		msg, err := wa().BuildEventResponse(context.Background(), secretMessageInfo(chat, data.MessageID), response, extra)
		if err != nil {
			fail("Couldn't answer the event: " + err.Error())
			return
		}
		if _, err := wa().SendMessage(context.Background(), chat, msg, whatsmeow.SendRequestExtra{}); err != nil {
			fail("Couldn't send your answer: " + err.Error())
			return
		}
		storeEventResponse(db, data.MessageID, ownVoterKey(), &waE2E.EventResponseMessage{
			Response:        response.Enum(),
			TimestampMS:     proto.Int64(time.Now().UnixMilli()),
			ExtraGuestCount: proto.Int32(int32(extra)),
		}, time.Now().Unix())
		results := broadcastPollResults(data.JID, data.MessageID)
		sendEvent(conn, "event_answered", map[string]interface{}{
			"message_id":  data.MessageID,
			"response":    data.Response,
			"pollResults": results,
		})
	}()
}

type SendEventData struct {
	JID         string `json:"jid"`
	Name        string `json:"name"`
	Description string `json:"description"`
	// Date and Time are as typed: "2026-10-05" and "18:30", local time. An
	// event with no date has no start time; a date alone starts at midnight.
	Date        string `json:"date"`
	Time        string `json:"time"`
	Location    string `json:"location"`
	ExtraGuests bool   `json:"extra_guests_allowed"`
	ReplyToID   string `json:"reply_to_id"`
	ReplySender string `json:"reply_to_sender"`
}

// parseEventStart reads the date and time typed for an event, in local time.
func parseEventStart(date, clock string, loc *time.Location) (time.Time, error) {
	date, clock = strings.TrimSpace(date), strings.TrimSpace(clock)
	if date == "" {
		if clock != "" {
			return time.Time{}, fmt.Errorf("type the date the event starts on, as well as the time")
		}
		return time.Time{}, nil
	}
	var day time.Time
	var err error
	for _, layout := range []string{"2006-01-02", "2/1/2006", "02/01/2006", "2.1.2006", "2 Jan 2006", "2 January 2006"} {
		if day, err = time.ParseInLocation(layout, date, loc); err == nil {
			break
		}
	}
	if err != nil {
		return time.Time{}, fmt.Errorf("the date isn't one I can read; type it like 2026-10-05")
	}
	if clock == "" {
		return day, nil
	}
	var at time.Time
	for _, layout := range []string{"15:04", "15.04", "3:04pm", "3:04 pm", "3pm", "3 pm", "3:04PM", "3:04 PM", "3PM", "3 PM"} {
		if at, err = time.ParseInLocation(layout, clock, loc); err == nil {
			break
		}
	}
	if err != nil {
		return time.Time{}, fmt.Errorf("the time isn't one I can read; type it like 18:30")
	}
	return time.Date(day.Year(), day.Month(), day.Day(), at.Hour(), at.Minute(), 0, 0, loc), nil
}

// handleSendEvent creates an event in a chat.
func handleSendEvent(conn net.Conn, raw json.RawMessage) {
	var data SendEventData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	fail := func(msg string) { sendEvent(conn, "error", map[string]string{"message": msg}) }
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		fail("invalid jid")
		return
	}
	if isChannelJID(data.JID) {
		fail("Events can't be posted in channels.")
		return
	}
	start, err := parseEventStart(data.Date, data.Time, time.Local)
	if err != nil {
		fail("Can't create the event: " + err.Error() + ".")
		return
	}
	creation := whatsmeow.EventCreation{
		Name:               strings.TrimSpace(data.Name),
		Description:        strings.TrimSpace(data.Description),
		StartTime:          start,
		ExtraGuestsAllowed: data.ExtraGuests,
	}
	if place := strings.TrimSpace(data.Location); place != "" {
		creation.Location = &waE2E.LocationMessage{Name: proto.String(place)}
	}
	msg, err := wa().BuildEventCreation(creation)
	if err != nil {
		fail("The event needs a name.")
		return
	}
	go func() {
		ctxInfo, quotedID, quotedText, quotedSender := buildAttachmentContext(data.ReplyToID, data.ReplySender)
		msg.EventMessage.ContextInfo = ctxInfo
		displayText := extractTextRaw(msg)
		id := wa().GenerateMessageID()
		startSendingBuiltMessage(id, jid, msg, displayText, quotedID, quotedText, quotedSender)
		finishSendingBuiltMessage(conn, id, jid, msg, displayText)
	}()
}
