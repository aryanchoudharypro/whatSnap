package main

import (
	"slices"
	"testing"
)

const iphoneCard = "BEGIN:VCARD\r\nVERSION:3.0\r\nN:Verma;Asha;;;\r\nFN:Asha Verma\r\nORG:Example Ltd;Sales\r\nTITLE:Manager\r\n" +
	"item1.TEL;waid=919876543210:+91 98765 43210\r\nitem1.X-ABLabel:Mobile\r\n" +
	"TEL;type=WORK;type=VOICE:+1 415 555 0100\r\n" +
	"item2.EMAIL;type=INTERNET:asha@example.com\r\nitem2.X-ABLabel:_$!<Work>!$_\r\n" +
	"item3.ADR;type=HOME:;;12 Park Street;Kolkata;WB;700016;India\r\nitem3.X-ABADR:in\r\n" +
	"BDAY;value=date:1604-03-05\r\nNICKNAME:Ash\r\nX-SKYPE-USERNAME:asha.v\r\nEND:VCARD"

func TestParseVcardLikeWhatsAppWeb(t *testing.T) {
	card := parseVcard(iphoneCard)
	if card == nil {
		t.Fatal("card didn't parse")
	}
	if got := vcardName(card); got != "Asha Verma" {
		t.Errorf("name = %q", got)
	}
	if first, last := vcardNames(card); first != "Asha" || last != "Verma" {
		t.Errorf("names = %q %q", first, last)
	}
	if got := vcardOrganization(card); got != "Manager - Sales, Example Ltd" {
		t.Errorf("org = %q", got)
	}
	numbers := vcardWhatsAppNumbers(card)
	if len(numbers) != 1 || numbers[0].props["waid"][0] != "919876543210" || vcardType(numbers[0]) != "Mobile" {
		t.Fatalf("whatsapp numbers = %+v", numbers)
	}
	want := []cardDetail{
		{Kind: "email", Value: "asha@example.com", Label: "Work", Link: "mailto:asha@example.com"},
		{Kind: "address", Value: "12 Park Street Kolkata WB 700016 India", Label: "Home",
			Link: "https://maps.google.com/maps/search/12%20Park%20Street, Kolkata, WB, 700016, India"},
		{Kind: "phone", Value: "+1 415 555 0100", Label: "Work"},
		{Kind: "birthday", Value: "5 March", Label: "Birthday"},
		{Kind: "nickname", Value: "Ash", Label: "Nickname"},
		{Kind: "service", Value: "asha.v", Label: "Skype"},
	}
	if got := cardDetails(card); !slices.Equal(got, want) {
		t.Errorf("details:\n%+v\nwant\n%+v", got, want)
	}
}

func TestParseVcardRejectsAndFolds(t *testing.T) {
	if parseVcard("BEGIN:VCARD\nFN:No version\nEND:VCARD") != nil {
		t.Error("a card without VERSION must not parse")
	}
	card := parseVcard("BEGIN:VCARD\nVERSION:2.1\nFN:Long\n  Name\nTEL;CELL:+44 7700 900123\nEND:VCARD")
	if card == nil || vcardName(card) != "Long Name" {
		t.Fatalf("folded name = %+v", card)
	}
	// A bare 2.1 parameter isn't a type, so WA Web falls through to Home.
	if got := vcardType(card["TEL"][0]); got != "Home" {
		t.Errorf("bare CELL label = %q", got)
	}
	if len(vcardWhatsAppNumbers(card)) != 0 {
		t.Error("a number without waid isn't a WhatsApp number")
	}
}

func TestVcardLIDAndBusiness(t *testing.T) {
	ours := buildVCard(sharedContact{name: "Ravi", lid: "100000012345678"})
	card := parseVcard(ours)
	if vcardLID(card) != "100000012345678" || vcardIsBusiness(card) {
		t.Errorf("lid card = %+v", card)
	}
	if len(cardDetails(card)) != 0 {
		t.Errorf("the LID isn't shown as a detail: %+v", cardDetails(card))
	}
	biz := parseVcard(buildVCard(sharedContact{name: "Shop", phone: "14155550100", isBusiness: true}))
	if !vcardIsBusiness(biz) || vcardLID(biz) != "" {
		t.Errorf("business card = %+v", biz)
	}
	if got := cardDetails(biz); len(got) != 1 || got[0].Label != "Business name" {
		t.Errorf("business details = %+v", got)
	}
}

func TestSplitVcardsAndNames(t *testing.T) {
	two := "BEGIN:VCARD\nVERSION:3.0\nFN:A\nEND:VCARD\nBEGIN:VCARD\nVERSION:3.0\nFN:B\nEND:VCARD"
	if cards := splitVcards(two); len(cards) != 2 || vcardName(cards[1]) != "B" {
		t.Errorf("split = %+v", cards)
	}
	cases := map[string][]string{
		"Asha":                      {"Asha"},
		"Asha and 1 other contact":  {"Asha", "Ravi"},
		"Asha and 2 other contacts": {"", "Asha", "Ravi"},
		"3 contacts":                {"", "", ""},
		"1 contact":                 {""},
	}
	for want, names := range cases {
		if got := multiCardName(names); got != want {
			t.Errorf("multiCardName(%q) = %q, want %q", names, got, want)
		}
	}
	if got := vcardFileName(` a/b:c? `); got != "abc.vcf" {
		t.Errorf("file name = %q", got)
	}
}
