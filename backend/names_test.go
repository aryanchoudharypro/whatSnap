package main

import "testing"

func TestRenderNamesFillsInEachPerson(t *testing.T) {
	names := map[string]string{"1@s.whatsapp.net": "Asha", "2@lid": "+91∙∙∙∙∙∙∙∙62"}
	nameFor := func(jid string) string { return names[jid] }
	cases := map[string]string{
		"no people here": "no people here",
		nameMarker + "1@s.whatsapp.net" + nameMarker + " added " + nameMarker + "2@lid" + nameMarker: "Asha added +91∙∙∙∙∙∙∙∙62",
		"stray " + nameMarker + "end": "stray " + nameMarker + "end",
	}
	for in, want := range cases {
		if got := renderNames(in, nameFor); got != want {
			t.Errorf("renderNames(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestBakedSystemLinePatterns(t *testing.T) {
	line := "Asha added 10000000000001@lid and +919800000004"
	got := bakedPhonePattern.ReplaceAllString(bakedLIDPattern.ReplaceAllString(line, nameMarker+"$1@lid"+nameMarker), nameMarker+"$1@s.whatsapp.net"+nameMarker)
	want := "Asha added " + nameMarker + "10000000000001@lid" + nameMarker + " and " + nameMarker + "919800000004@s.whatsapp.net" + nameMarker
	if got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}
