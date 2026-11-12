package main

import (
	"fmt"
	"net/url"
	"regexp"
	"sort"
	"strconv"
	"strings"
)

// vCard parsing for received contact cards, the way WhatsApp Web reads them
// (WAWebVcardParsingUtils, WAWebVcardUtils, WAWebVcardGetNameFromParsed,
// WAWebVcardModalDetails.react).

const (
	vcardMaxBytes    = 5_000_000
	vcardMaxLines    = 100
	vcardMaxLineLen  = 16 * 1024
	vcardMaxPhysical = 4096
	vcardMaxPerMsg   = 257
)

type vcardField struct {
	index int
	typ   string
	props map[string][]string
	value []string
}

func (f vcardField) first() string {
	if len(f.value) == 0 {
		return ""
	}
	return f.value[0]
}

type parsedVcard map[string][]vcardField

type vcardRule struct {
	re    *regexp.Regexp
	parts int
}

var (
	vcardGrouped = map[string]vcardRule{
		"X-ABLabel": {regexp.MustCompile(`^X-ABLabel(?:;([^:]+))?:(.+)`), 1},
		"ADR":       {regexp.MustCompile(`^ADR(?:;([^:]+))?:;;(.*);(.*);(.*);(.*);(.*)`), 5},
		"X-ABADR":   {regexp.MustCompile(`^X-ABADR(?:;([^:]+))?:(.*)`), 1},
		"URL":       {regexp.MustCompile(`^URL(?:;([^:]+))?:(.*)`), 1},
		"EMAIL":     {regexp.MustCompile(`^EMAIL(?:;([^:]+))?:(.+)`), 1},
		"TEL":       {regexp.MustCompile(`^TEL(?:;([^:]+))?:(.+)`), 1},
	}
	vcardPlain = map[string]vcardRule{
		"N":                     {regexp.MustCompile(`^N(?:;([^:]+))?:([^;]*);([^;]*);([^;]*);([^;]*);(.*)`), 5},
		"FN":                    {regexp.MustCompile(`^FN(?:;([^:]+))?:(.*)`), 1},
		"NICKNAME":              {regexp.MustCompile(`^NICKNAME(?:;([^:]+))?:(.*)`), 1},
		"X-PHONETIC-FIRST-NAME": {regexp.MustCompile(`^X-PHONETIC-FIRST-NAME(?:;([^:]+))?:(.*)`), 1},
		"X-PHONETIC-LAST-NAME":  {regexp.MustCompile(`^X-PHONETIC-LAST-NAME(?:;([^:]+))?:(.*)`), 1},
		"ORG":                   {regexp.MustCompile(`^ORG(?:;([^:]+))?:([^;]*)(?:;(.*))?`), 2},
		"TITLE":                 {regexp.MustCompile(`^TITLE(?:;([^:]+))?:(.*)`), 1},
		"TEL":                   {regexp.MustCompile(`^TEL(?:;([^:]+))?:(.*)`), 1},
		"PHOTO":                 {regexp.MustCompile(`^PHOTO(?:;([^:]+))?:(.*)`), 1},
		"BDAY":                  {regexp.MustCompile(`^BDAY(?:;([^:]+))?:(.*)`), 1},
		"URL":                   {regexp.MustCompile(`^URL(?:;([^:]+))?:(.+)`), 1},
		"EMAIL":                 {regexp.MustCompile(`^EMAIL(?:;([^:]+))?:(.+)`), 1},
	}
	vcardItemRe    = regexp.MustCompile(`item(\d+)\.(.*)`)
	vcardNameRe    = regexp.MustCompile(`^([^:;]*)[:;]`)
	vcardServiceRe = regexp.MustCompile(`^X-([^;:]+)(?:;[^:]+)?:(.+)`)
	vcardLineBreak = regexp.MustCompile(`\r\n?|\n`)
	vcardAppleRe   = regexp.MustCompile(`^_\$!<([^>]*)>!\$_$`)
)

// vcardLines unfolds the card into logical lines with WA Web's limits.
func vcardLines(text string) []string {
	if len(text) > vcardMaxBytes {
		return nil
	}
	text = strings.TrimRight(text, " \t\r\n\v\f")
	last := max(strings.LastIndex(text, "\n"), strings.LastIndex(text, "\r")) + 1
	if len(text)-last > vcardMaxLineLen {
		return nil
	}
	var lines []string
	current := ""
	keep := func() {
		if len(lines) < vcardMaxLines-1 && len(current) <= vcardMaxLineLen && strings.TrimSpace(current) != "" {
			lines = append(lines, current)
		}
	}
	physical := vcardLineBreak.Split(text[:last], -1)
	if last > 0 {
		physical = physical[:len(physical)-1]
	}
	for i, line := range physical {
		if i == vcardMaxPhysical || len(lines) == vcardMaxLines-1 {
			return append(lines, text[last:])
		}
		if i > 0 && (strings.HasPrefix(line, " ") || strings.HasPrefix(line, "\t")) {
			if len(current) <= vcardMaxLineLen {
				current += line[1:]
			}
			continue
		}
		keep()
		current = line
	}
	keep()
	return append(lines, text[last:])
}

func vcardParams(raw string) map[string][]string {
	props := map[string][]string{}
	if raw == "" {
		return props
	}
	for _, part := range strings.Split(raw, ";") {
		kv := strings.Split(part, "=")
		switch len(kv) {
		case 1:
			props[strings.ToLower(kv[0])] = append(props[strings.ToLower(kv[0])], "true")
		case 2:
			props[strings.ToLower(kv[0])] = append(props[strings.ToLower(kv[0])], strings.ToLower(kv[1]))
		}
	}
	return props
}

// parseVcard returns nil when the text isn't a vCard 2.1, 3.0 or 4.0.
func parseVcard(text string) parsedVcard {
	lines := vcardLines(text)
	if len(lines) < 3 {
		return nil
	}
	head := strings.ToUpper(strings.TrimSpace(lines[0]))
	version := strings.ToUpper(strings.TrimSpace(lines[1]))
	foot := strings.ToUpper(strings.TrimSpace(lines[len(lines)-1]))
	if head != "BEGIN:VCARD" || foot != "END:VCARD" ||
		(version != "VERSION:2.1" && version != "VERSION:3.0" && version != "VERSION:4.0") {
		return nil
	}
	type group struct {
		vcardField
		key    string
		fields []vcardField
	}
	card := parsedVcard{}
	groups := map[string]*group{}
	var groupOrder []string
	for i := 2; i < len(lines)-1; i++ {
		line := lines[i]
		item := vcardItemRe.FindStringSubmatch(line)
		body := line
		if item != nil {
			body = item[2]
		}
		name := vcardNameRe.FindStringSubmatch(body)
		if name == nil {
			continue
		}
		rules := vcardPlain
		if item != nil {
			rules = vcardGrouped
		}
		typ, key := name[1], name[1]
		props := map[string][]string{}
		var value []string
		if rule, ok := rules[typ]; ok {
			if m := rule.re.FindStringSubmatch(body); m != nil {
				props = vcardParams(m[1])
				end := min(2+rule.parts, len(m))
				value = append(value, m[2:end]...)
			}
		} else if at := strings.Index(body, "X-"); at >= 0 {
			if m := vcardServiceRe.FindStringSubmatch(body[at:]); m != nil {
				key, typ, value = "SERVICE", m[1], []string{m[2]}
			}
		}
		if !slicesAny(value, func(v string) bool { return strings.TrimSpace(v) != "" }) {
			continue
		}
		field := vcardField{index: i, typ: typ, props: props, value: value}
		if item == nil {
			card[key] = append(card[key], field)
			continue
		}
		if g, ok := groups[item[1]]; ok {
			g.fields = append(g.fields, vcardField{typ: typ, props: props, value: value})
		} else {
			groups[item[1]] = &group{vcardField: field, key: key}
			groupOrder = append(groupOrder, item[1])
		}
	}
	for _, id := range groupOrder {
		g := groups[id]
		for _, f := range g.fields {
			g.props[f.typ] = []string{strings.Join(f.value, ";")}
		}
		card[g.key] = append(card[g.key], g.vcardField)
	}
	for key := range card {
		sort.SliceStable(card[key], func(a, b int) bool { return card[key][a].index < card[key][b].index })
	}
	return card
}

func slicesAny(values []string, ok func(string) bool) bool {
	for _, v := range values {
		if ok(v) {
			return true
		}
	}
	return false
}

// splitVcards finds every BEGIN:VCARD ... END:VCARD block, as WA Web does
// for each entry of a multi-contact message.
func splitVcards(text string) []parsedVcard {
	if len(text) > vcardMaxBytes {
		return nil
	}
	var cards []parsedVcard
	start := strings.Index(text, "BEGIN:VCARD")
	for start >= 0 && len(cards) < vcardMaxPerMsg {
		end := strings.Index(text[start+len("BEGIN:VCARD"):], "END:VCARD")
		if end < 0 {
			break
		}
		end += start + len("BEGIN:VCARD") + len("END:VCARD")
		if card := parseVcard(text[start:end]); card != nil {
			cards = append(cards, card)
		}
		next := strings.Index(text[end:], "BEGIN:VCARD")
		if next < 0 {
			break
		}
		start = end + next
	}
	return cards
}

func vcardClean(label string) string {
	label = vcardAppleRe.ReplaceAllString(label, "$1")
	label = strings.Replace(label, `\,`, ",", 1)
	label = strings.Replace(label, `\;`, ";", 1)
	return strings.Replace(label, `\\`, `\`, 1)
}

var vcardServiceNames = map[string]string{
	"jabber": "Jabber", "skype-username": "Skype", "skype": "Skype", "msn": "Windows Live", "aim": "AIM",
	"yahoo": "Yahoo", "icq": "ICQ", "wa-biz-description": "Description", "wa-biz-name": "Business name",
}

var vcardTypeNames = map[string][]struct {
	needs []string
	label string
}{
	"TEL": {
		{[]string{"cell"}, "Mobile"}, {[]string{"iphone"}, "iPhone"}, {[]string{"main"}, "Main"},
		{[]string{"home", "fax"}, "Home fax"}, {[]string{"work", "fax"}, "Work fax"}, {[]string{"home"}, "Home"},
		{[]string{"work"}, "Work"}, {[]string{"pager"}, "Pager"}, {nil, "Home"},
	},
	"EMAIL": {{[]string{"home"}, "Home"}, {[]string{"work"}, "Work"}, {[]string{"internet"}, "Email"}},
	"URL":   {{[]string{"home"}, "Home"}, {[]string{"work"}, "Work"}},
	"ADR":   {{[]string{"home"}, "Home"}, {[]string{"work"}, "Work"}},
}

// vcardType is the small label WA Web shows under a field (Mobile, Work, ...).
func vcardType(f vcardField) string {
	if label := f.props["X-ABLabel"]; len(label) > 0 && label[0] != "" {
		return vcardClean(label[0])
	}
	if name, ok := vcardServiceNames[strings.ToLower(f.typ)]; ok {
		return name
	}
	have := map[string]bool{}
	for _, t := range f.props["type"] {
		have[strings.ToLower(t)] = true
	}
	for _, rule := range vcardTypeNames[f.typ] {
		all := true
		for _, need := range rule.needs {
			all = all && have[need]
		}
		if all {
			return rule.label
		}
	}
	return f.typ
}

// vcardWhatsAppNumbers are the TEL entries carrying waid, the numbers WA Web
// treats as on WhatsApp without asking the server.
func vcardWhatsAppNumbers(card parsedVcard) []vcardField {
	var out []vcardField
	for _, tel := range card["TEL"] {
		if len(tel.props["waid"]) > 0 {
			out = append(out, tel)
		}
	}
	return out
}

func vcardLID(card parsedVcard) string {
	if len(card["TEL"]) > 0 {
		return ""
	}
	for _, s := range card["SERVICE"] {
		if strings.ToLower(s.typ) == "wa-lid" && s.first() != "" {
			return s.first()
		}
	}
	return ""
}

func vcardIsBusiness(card parsedVcard) bool {
	for _, s := range card["SERVICE"] {
		if strings.ToLower(s.typ) == "wa-biz-name" && s.first() != "" {
			return true
		}
	}
	return false
}

func vcardName(card parsedVcard) string {
	switch {
	case len(card["FN"]) > 0:
		return card["FN"][0].first()
	case len(card["NICKNAME"]) > 0:
		return card["NICKNAME"][0].first()
	case len(card["TEL"]) > 0:
		tel := card["TEL"][0]
		if waid := tel.props["waid"]; len(waid) > 0 {
			return formatPhone(waid[0])
		}
		return tel.first()
	}
	return ""
}

func vcardOrganization(card parsedVcard) string {
	if len(card["ORG"]) == 0 || len(card["ORG"][0].value) == 0 {
		return ""
	}
	parts := card["ORG"][0].value
	ordered := append(append([]string{}, parts[1:]...), parts[0])
	var kept []string
	for _, p := range ordered {
		if p = strings.TrimSpace(p); p != "" {
			kept = append(kept, p)
		}
	}
	title := ""
	if len(card["TITLE"]) > 0 {
		title = strings.TrimSpace(card["TITLE"][0].first())
	}
	org := strings.Join(kept, ", ")
	if title != "" && org != "" {
		return title + " - " + org
	}
	return title + org
}

// vcardNames splits the card's name the way WA Web fills its save form:
// N's given and family parts, else FN's first and second words.
func vcardNames(card parsedVcard) (first, last string) {
	var n, fn []string
	if len(card["N"]) > 0 {
		n = card["N"][0].value
	}
	if len(card["FN"]) > 0 {
		fn = strings.Split(card["FN"][0].first(), " ")
	}
	switch {
	case len(n) > 1:
		first = n[1]
	case len(fn) > 0:
		first = fn[0]
	}
	switch {
	case len(n) > 0:
		last = n[0]
	case len(fn) > 1:
		last = fn[1]
	}
	return first, last
}

var vcardDateRes = []*regexp.Regexp{regexp.MustCompile(`-([^-]*)-([^-]*)-([^-]*)`), regexp.MustCompile(`^([^-]*)-([^-]*)-([^-]*)`)}

var vcardMonths = []string{"January", "February", "March", "April", "May", "June", "July", "August",
	"September", "October", "November", "December"}

// vcardDate reads a BDAY as year-month-day, with no year when it's missing
// or the 1604 placeholder Apple uses.
func vcardDate(value string) string {
	for _, re := range vcardDateRes {
		m := re.FindStringSubmatch(value)
		if m == nil {
			continue
		}
		month, merr := strconv.Atoi(m[2])
		day, derr := strconv.Atoi(strings.SplitN(m[3], "T", 2)[0])
		if merr != nil || derr != nil || month < 1 || month > 12 || day < 1 || day > 31 {
			return value
		}
		if m[1] == "" || m[1] == "1604" {
			return fmt.Sprintf("%d %s", day, vcardMonths[month-1])
		}
		return fmt.Sprintf("%d %s %s", day, vcardMonths[month-1], m[1])
	}
	return value
}

type cardNumber struct {
	JID    string `json:"jid"`
	Digits string `json:"digits"`
	Label  string `json:"label"`
}

type cardDetail struct {
	Kind  string `json:"kind"`
	Value string `json:"value"`
	Label string `json:"label"`
	Link  string `json:"link,omitempty"`
}

type contactCardJSON struct {
	Name       string       `json:"name"`
	Org        string       `json:"org"`
	FirstName  string       `json:"first_name"`
	LastName   string       `json:"last_name"`
	Numbers    []cardNumber `json:"numbers"`
	LID        string       `json:"lid"`
	LIDChat    string       `json:"lid_chat,omitempty"`
	Details    []cardDetail `json:"details"`
	Saved      bool         `json:"saved"`
	Me         bool         `json:"me"`
	Business   bool         `json:"business"`
	CanSave    bool         `json:"can_save"`
	AddToGroup bool         `json:"add_to_group"`
}

// cardDetails lists a card's rows in WA Web's contact viewer order.
func cardDetails(card parsedVcard) []cardDetail {
	var rows []cardDetail
	for _, e := range card["EMAIL"] {
		rows = append(rows, cardDetail{Kind: "email", Value: e.first(), Label: vcardType(e), Link: "mailto:" + e.first()})
	}
	for _, a := range card["ADR"] {
		var shown, linked []string
		for _, part := range a.value {
			shown = append(shown, part)
			if part != "" {
				linked = append(linked, url.PathEscape(part))
			}
		}
		rows = append(rows, cardDetail{
			Kind: "address", Value: strings.TrimSpace(strings.Join(shown, " ")), Label: vcardType(a),
			Link: "https://maps.google.com/maps/search/" + strings.Join(linked, ", "),
		})
	}
	for _, tel := range card["TEL"] {
		if len(tel.props["waid"]) == 0 {
			rows = append(rows, cardDetail{Kind: "phone", Value: tel.first(), Label: vcardType(tel)})
		}
	}
	for _, b := range card["BDAY"] {
		rows = append(rows, cardDetail{Kind: "birthday", Value: vcardDate(b.first()), Label: "Birthday"})
	}
	for _, n := range card["NICKNAME"] {
		rows = append(rows, cardDetail{Kind: "nickname", Value: n.first(), Label: "Nickname"})
	}
	for _, s := range card["SERVICE"] {
		if s.typ == "WA-LID" || s.typ == "WA-BIZ-AUTOMATED-TYPE" {
			continue
		}
		rows = append(rows, cardDetail{Kind: "service", Value: s.first(), Label: vcardType(s)})
	}
	return rows
}

// multiCardName is WA Web's name for a multi-contact message:
// "Asha and 2 other contacts".
func multiCardName(names []string) string {
	first := ""
	for _, n := range names {
		if n != "" {
			first = n
			break
		}
	}
	switch {
	case len(names) == 0:
		return "No contacts"
	case first == "" && len(names) == 1:
		return "1 contact"
	case first == "":
		return fmt.Sprintf("%d contacts", len(names))
	case len(names) == 1:
		return first
	case len(names) == 2:
		return first + " and 1 other contact"
	}
	return fmt.Sprintf("%s and %d other contacts", first, len(names)-1)
}

// vcardFileName drops the characters Windows won't take in a file name.
func vcardFileName(title string) string {
	name := strings.Map(func(r rune) rune {
		switch {
		case r < 0x20, r == 0x7f, r >= 0x202a && r <= 0x202e, r >= 0x2066 && r <= 0x2069, strings.ContainsRune(`<>:"/\|?*`, r):
			return -1
		}
		return r
	}, strings.TrimSpace(title))
	if name == "" {
		name = "Contact"
	}
	if r := []rune(name); len(r) > 200 {
		name = string(r[:200])
	}
	return name + ".vcf"
}
