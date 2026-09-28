package main

import "testing"

func TestApplyMentions(t *testing.T) {
	mentions := []MentionRef{
		{JID: "111@lid", Name: "Ana"},
		{JID: "222@s.whatsapp.net", Name: "Ana Maria"},
		{JID: "867051314767696@bot", Name: "Meta AI"},
		{JID: "333@lid", Name: "Gone"},
	}
	text, jids := applyMentions("hi @Ana Maria and @Ana, @Anabel stays. @Meta AI help", mentions)
	if want := "hi @222 and @111, @Anabel stays. @867051314767696 help"; text != want {
		t.Fatalf("text = %q, want %q", text, want)
	}
	if len(jids) != 3 || jids[0].String() != "111@lid" || jids[1].String() != "222@s.whatsapp.net" || !invokedBot(jids).IsBot() {
		t.Fatalf("jids = %v", jids)
	}
	if text, jids := applyMentions("no mentions", nil); text != "no mentions" || len(jids) != 0 {
		t.Fatalf("got %q %v", text, jids)
	}
}
