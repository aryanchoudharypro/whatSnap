package main

import (
	"database/sql"
	"regexp"
	"strings"

	"go.mau.fi/whatsmeow/types"
)

// Group-event lines ("X added Y", "X left") are stored as message rows. They
// used to be stored with the names already filled in, so a person whose name
// or phone number wasn't known yet stayed "36920176427140@lid" forever, even
// after WhatsApp told us who that was. Now each person is stored as a marker
// around their JID and named when the line is shown.
const nameMarker = "\x1f"

// nameToken is the stored form of a person inside a group-event line.
func nameToken(jid types.JID) string {
	return nameMarker + preferPN(jid).String() + nameMarker
}

// renderNames fills in the people in a stored group-event line. nameFor
// names one JID; pass nil to use resolveDisplayName.
func renderNames(text string, nameFor func(string) string) string {
	if !strings.Contains(text, nameMarker) {
		return text
	}
	if nameFor == nil {
		nameFor = displayNameForJIDString
	}
	parts := strings.Split(text, nameMarker)
	var b strings.Builder
	for i, part := range parts {
		// Odd parts sit between a pair of markers. A stray unpaired marker
		// at the end is left as plain text.
		switch {
		case i%2 == 1 && i < len(parts)-1:
			b.WriteString(nameFor(part))
		case i%2 == 1:
			b.WriteString(nameMarker + part)
		default:
			b.WriteString(part)
		}
	}
	return b.String()
}

func displayNameForJIDString(jidStr string) string {
	parsed, err := types.ParseJID(jidStr)
	if err != nil {
		return jidStr
	}
	return resolveDisplayName(parsed)
}

var (
	bakedLIDPattern   = regexp.MustCompile(`\b(\d+)@lid\b`)
	bakedPhonePattern = regexp.MustCompile(`\+(\d{8,15})\b`)
)

// tokenizeBakedSystemLines turns the raw LIDs and "+<number>" fallbacks that
// older builds wrote into group-event lines into name markers, so those
// people get named once we know who they are. Lines with a quote in them
// quote a group name, which may contain a number of its own, so they're
// left alone.
func tokenizeBakedSystemLines() {
	rows, err := db.Query(`SELECT id, text FROM messages WHERE media_type = 'system' AND (text LIKE '%@lid%' OR text LIKE '%+%')`)
	if err != nil {
		return
	}
	type row struct{ id, text string }
	var updates []row
	for rows.Next() {
		var r row
		var text sql.NullString
		if rows.Scan(&r.id, &text) != nil || strings.Contains(text.String, nameMarker) || strings.Contains(text.String, `"`) {
			continue
		}
		r.text = bakedLIDPattern.ReplaceAllString(text.String, nameMarker+"$1@lid"+nameMarker)
		r.text = bakedPhonePattern.ReplaceAllString(r.text, nameMarker+"$1@"+types.DefaultUserServer+nameMarker)
		if r.text != text.String {
			updates = append(updates, r)
		}
	}
	rows.Close()
	writes := make([]func(tx sqlExecer), 0, len(updates))
	for _, u := range updates {
		writes = append(writes, func(tx sqlExecer) {
			tx.Exec(`UPDATE messages SET text = ? WHERE id = ?`, u.text, u.id)
		})
	}
	writeBatch(writes)
}
