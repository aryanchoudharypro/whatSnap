package main

import (
	"strings"
	"testing"

	"go.mau.fi/whatsmeow/proto/waE2E"
)

func TestBuildPollUsesWhatsAppWebsFields(t *testing.T) {
	multi, err := acc.buildPoll(" Lunch? ", []string{"Pizza", " ", "Pasta"}, true, "")
	if err != nil {
		t.Fatal(err)
	}
	if multi.GetPollCreationMessage() == nil || multi.GetPollCreationMessageV3() != nil {
		t.Fatal("a multiple-answer poll belongs in pollCreationMessage")
	}
	p := multi.GetPollCreationMessage()
	if p.GetName() != "Lunch?" || len(p.GetOptions()) != 2 || p.GetSelectableOptionsCount() != 0 {
		t.Fatalf("unexpected poll: %v", p)
	}
	if len(multi.GetMessageContextInfo().GetMessageSecret()) != 32 {
		t.Fatal("a poll needs its 32-byte message secret for votes to decrypt")
	}

	single, err := acc.buildPoll("Lunch?", []string{"Pizza", "Pasta"}, false, "")
	if err != nil {
		t.Fatal(err)
	}
	if single.GetPollCreationMessageV3() == nil || single.GetPollCreationMessage() != nil {
		t.Fatal("a single-answer poll belongs in pollCreationMessageV3")
	}
	if single.GetPollCreationMessageV3().GetSelectableOptionsCount() != 1 {
		t.Fatal("a single-answer poll allows one selection")
	}
}

func TestBuildPollRejectsWhatWhatsAppWebWould(t *testing.T) {
	twelve := make([]string, 13)
	for i := range twelve {
		twelve[i] = strings.Repeat("x", i+1)
	}
	cases := map[string]struct {
		question string
		options  []string
	}{
		"no question":       {"  ", []string{"a", "b"}},
		"one option":        {"Q", []string{"a", ""}},
		"duplicate options": {"Q", []string{"a", "a"}},
		"too many options":  {"Q", twelve},
		"long question":     {strings.Repeat("q", 256), []string{"a", "b"}},
		"long option":       {"Q", []string{"a", strings.Repeat("o", 101)}},
	}
	for name, c := range cases {
		if _, err := acc.buildPoll(c.question, c.options, true, ""); err == nil {
			t.Errorf("%s: expected an error", name)
		}
	}
}

func TestAttachmentFieldsFromDisplayText(t *testing.T) {
	cases := []struct {
		mediaType, filename, text, caption string
		seconds                            int
	}{
		{"image", "", "[Image] beach day", "beach day", 0},
		{"image", "", "[Image]", "", 0},
		{"video", "", "[Video] clip", "clip", 0},
		{"document", "report.pdf", "[Document] report.pdf: see page 2", "see page 2", 0},
		{"document", "report.pdf", "[Document] report.pdf", "", 0},
		{"audio", "song.mp3", "[Audio] (3:07)", "", 187},
		{"sticker", "", "[Sticker]", "", 0},
	}
	for _, c := range cases {
		caption, seconds := attachmentFieldsFromDisplayText(c.mediaType, c.filename, c.text)
		if caption != c.caption || seconds != c.seconds {
			t.Errorf("%q: got (%q, %d), want (%q, %d)", c.text, caption, seconds, c.caption, c.seconds)
		}
		if c.mediaType != "sticker" {
			if back := attachmentDisplayText(c.mediaType, c.filename, caption, seconds); back != c.text {
				t.Errorf("%q: round trip gave %q", c.text, back)
			}
		}
	}
}

func TestPollResultsText(t *testing.T) {
	p := poll{question: "Lunch?", options: []string{"Yes", "No"}, multiple: false}
	if got := pollResultsText(p, nil, "me"); got != "Options: Yes, No. No votes yet." {
		t.Errorf("no votes: %q", got)
	}
	votes := map[string][]string{
		"me":    {optionHash("Yes")},
		"alice": {optionHash("No")},
	}
	want := "Yes 50%, 1 vote; No 50%, 1 vote. 2 people voted, you voted Yes."
	if got := pollResultsText(p, votes, "me"); got != want {
		t.Errorf("got %q, want %q", got, want)
	}
	multi := poll{options: []string{"A", "B", "C"}, multiple: true}
	votes = map[string][]string{
		"bob": {optionHash("A"), optionHash("B")},
		"eve": {optionHash("A")},
		"zed": {optionHash("C")},
	}
	want = "A 67%, 2 votes; B 33%, 1 vote; C 33%, 1 vote. 3 people voted."
	if got := pollResultsText(multi, votes, "me"); got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}

func TestBuildPollMakesAQuiz(t *testing.T) {
	msg, err := acc.buildPoll("Capital of France?", []string{"Lyon", "Paris", "Nice"}, true, " Paris ")
	if err != nil {
		t.Fatal(err)
	}
	pc := msg.GetPollCreationMessageV3()
	if pc == nil || pc.GetPollType() != waE2E.PollType_QUIZ || pc.GetSelectableOptionsCount() != 1 {
		t.Fatalf("a quiz is a single-answer V3 poll of type QUIZ, got %v", msg)
	}
	if pc.GetCorrectAnswer().GetOptionName() != "Paris" || pc.GetCorrectAnswer().GetOptionHash() != optionHash("Paris") {
		t.Fatalf("unexpected correct answer %v", pc.GetCorrectAnswer())
	}
	if acc.extractTextRaw(msg) != "[Quiz] Capital of France?" {
		t.Fatalf("got %q", acc.extractTextRaw(msg))
	}
	if _, err := acc.buildPoll("Q", []string{"a", "b"}, false, "c"); err == nil {
		t.Fatal("a correct answer that isn't an option must be refused")
	}
}

func TestQuizAnswerIsRevealedOnceAnswered(t *testing.T) {
	msg, _ := acc.buildPoll("Q", []string{"Right", "Wrong"}, false, "Right")
	p, ok := pollFromMessage(msg)
	if !ok || !p.quiz || p.multiple || p.correct != "Right" {
		t.Fatalf("unexpected quiz %+v", p)
	}
	if got := pollResultsText(p, nil, "me"); strings.Contains(got, "Right.") {
		t.Fatalf("the answer must stay hidden before answering: %q", got)
	}
	votes := map[string][]string{"me": {optionHash("Wrong")}}
	if got := pollResultsText(p, votes, "me"); !strings.HasSuffix(got, "The correct answer is Right.") {
		t.Fatalf("got %q", got)
	}
	votes["me"] = []string{optionHash("Right")}
	if got := pollResultsText(p, votes, "me"); !strings.HasSuffix(got, "You got it right: the answer is Right.") {
		t.Fatalf("got %q", got)
	}
	p.fromMe = true
	if got := pollResultsText(p, nil, "me"); !strings.HasSuffix(got, "The correct answer is Right.") {
		t.Fatalf("your own quiz shows its answer: %q", got)
	}
}
