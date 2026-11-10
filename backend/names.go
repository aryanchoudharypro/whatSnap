package main

import (
	"database/sql"
	"regexp"
	"strings"

	"go.mau.fi/whatsmeow/types"
)

const nameMarker = "\x1f"

func (acc *Account) nameToken(jid types.JID) string {
	return nameMarker + acc.preferPN(jid).String() + nameMarker
}

func (acc *Account) renderNames(text string, nameFor func(string) string) string {
	if !strings.Contains(text, nameMarker) {
		return text
	}
	if nameFor == nil {
		nameFor = acc.displayNameForJIDString
	}
	parts := strings.Split(text, nameMarker)
	var b strings.Builder
	for i, part := range parts {
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

const unknownMemberName = "Unknown member"

// looksLikePhoneLabel reports whether a display name is only a phone number or
// raw address, which is what resolveDisplayName falls back to when it knows no
// name at all.
func looksLikePhoneLabel(name string) bool {
	if name == "" || strings.Contains(name, "@") {
		return name != ""
	}
	digits := 0
	for _, r := range strings.TrimPrefix(name, "+") {
		switch {
		case r >= '0' && r <= '9':
			digits++
		case r == ' ' || r == '-' || r == '(' || r == ')' || r == '\u2011' || r == '\u00a0':
		default:
			return false
		}
	}
	return digits >= 5
}

// hidePhoneInGroup swaps a bare phone number for a spoken-friendly label when
// the "hide phone numbers" preference is on and the name comes from a group
// chat. Saved names and profile names pass through unchanged.
func (acc *Account) hidePhoneInGroup(chatJID, name string) string {
	if !strings.HasSuffix(chatJID, "@"+types.GroupServer) || !looksLikePhoneLabel(name) || !acc.appPref("hide_phone_numbers") {
		return name
	}
	return unknownMemberName
}

func (acc *Account) displayNameForJIDString(jidStr string) string {
	parsed, err := types.ParseJID(jidStr)
	if err != nil {
		return jidStr
	}
	return acc.resolveDisplayName(parsed)
}

var (
	bakedLIDPattern   = regexp.MustCompile(`\b(\d+)@lid\b`)
	bakedPhonePattern = regexp.MustCompile(`\+(\d{8,15})\b`)
)

func (acc *Account) tokenizeBakedSystemLines() {
	rows, err := acc.db.Query(`SELECT id, text FROM messages WHERE media_type = 'system' AND (text LIKE '%@lid%' OR text LIKE '%+%')`)
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
	acc.writeBatch(writes)
}
