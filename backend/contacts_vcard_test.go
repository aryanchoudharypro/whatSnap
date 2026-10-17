package main

import (
	"slices"
	"testing"
)

func TestPhoneCandidates(t *testing.T) {
	cases := []struct {
		typed, own string
		want       []string
	}{
		{"98765 43210", "91", []string{"919876543210", "9876543210"}}, // your own country first
		{"+91 98765 43210", "91", []string{"919876543210"}},           // + means international
		{"0044 7700 900123", "91", []string{"447700900123"}},          // so does 00
		{"07700 900123", "44", []string{"447700900123", "07700900123"}},
		{"919876543210", "91", []string{"919876543210"}}, // already has the country code
		{"abc", "91", nil},
	}
	for _, c := range cases {
		if got := phoneCandidates(c.typed, c.own); !slices.Equal(got, c.want) {
			t.Errorf("phoneCandidates(%q, %q) = %q, want %q", c.typed, c.own, got, c.want)
		}
	}
}

func TestBuildVCardMatchesWhatsAppWeb(t *testing.T) {
	got := buildVCard(sharedContact{name: "Asha Verma", phone: "919876543210"})
	want := "BEGIN:VCARD\nVERSION:3.0\nN:Verma;Asha;;;\nFN:Asha Verma\nTEL;type=CELL;waid=919876543210:+91 9876543210\nEND:VCARD"
	if got != want {
		t.Errorf("person:\n%s\nwant\n%s", got, want)
	}
	got = buildVCard(sharedContact{name: "Shop; Co", phone: "14155550100", isBusiness: true})
	want = "BEGIN:VCARD\nVERSION:3.0\nN:Co;Shop\\;;;;\nFN:Shop\\; Co\nTEL;type=CELL;waid=14155550100:+1 4155550100\nX-WA-BIZ-NAME:Shop\\; Co\nEND:VCARD"
	if got != want {
		t.Errorf("business:\n%s\nwant\n%s", got, want)
	}
	// Someone whose number is hidden from us is shared by their LID, never
	// with the LID passed off as a phone number.
	got = buildVCard(sharedContact{name: "Ravi", lid: "100000012345678"})
	want = "BEGIN:VCARD\nVERSION:3.0\nN:;Ravi;;;\nFN:Ravi\nX-WA-LID:100000012345678\nEND:VCARD"
	if got != want {
		t.Errorf("hidden number:\n%s\nwant\n%s", got, want)
	}
}
