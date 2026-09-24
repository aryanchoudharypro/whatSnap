package main

import "testing"

func TestFormatChannelStats(t *testing.T) {
	cases := []struct {
		views     int
		reactions map[string]int
		want      string
	}{
		{0, nil, ""},
		{1, nil, "1 view"},
		{1204, map[string]int{"❤️": 12, "👍": 31, "😮": 0}, "1,204 views, 👍 31, ❤️ 12"},
		{0, map[string]int{"🙏": 1500000}, "🙏 1,500,000"},
	}
	for _, c := range cases {
		if got := formatChannelStats(c.views, c.reactions); got != c.want {
			t.Errorf("formatChannelStats(%d, %v) = %q, want %q", c.views, c.reactions, got, c.want)
		}
	}
	if got := channelStatsFromColumns(3, `{"👍":2}`, false); got != "3 views, 👍 2" {
		t.Errorf("channelStatsFromColumns = %q", got)
	}
	if got := channelStatsFromColumns(3, "", true); got != "Pinned, 3 views" {
		t.Errorf("pinned channelStatsFromColumns = %q", got)
	}
	if got := channelStatsFromColumns(0, "", true); got != "Pinned" {
		t.Errorf("pinned channelStatsFromColumns with no counts = %q", got)
	}
}

func TestChannelPollResultsText(t *testing.T) {
	p := poll{question: "Lunch?", options: []string{"Pizza", "Salad"}}
	counts := map[string]int{optionHash("Pizza"): 3, optionHash("Salad"): 1}
	if got, want := channelPollResultsText(p, counts, nil), "Pizza 75%, 3 votes; Salad 25%, 1 vote. 4 votes in all."; got != want {
		t.Errorf("got %q, want %q", got, want)
	}
	if got, want := channelPollResultsText(p, counts, []string{optionHash("Salad")}), "Pizza 75%, 3 votes; Salad 25%, 1 vote. 4 votes in all, you voted Salad."; got != want {
		t.Errorf("got %q, want %q", got, want)
	}
	if got, want := channelPollResultsText(p, nil, nil), "Options: Pizza, Salad. No votes yet."; got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}

func TestChannelPollVotesJSON(t *testing.T) {
	var hash [32]byte
	hash[0] = 0xab
	got := channelPollVotesJSON(map[[32]byte]int{hash: 5})
	want := `{"ab00000000000000000000000000000000000000000000000000000000000000":5}`
	if got != want {
		t.Errorf("got %s", got)
	}
}

func TestRegionFromPhone(t *testing.T) {
	for number, want := range map[string]string{
		"+919800000004": "IN", "+14155550100": "US", "+447700900123": "GB", "+14165550100": "CA",
		"+17875550100": "PR", "+77015550100": "KZ", "+74955550100": "RU", "+358181234567": "AX",
		"+971501234567": "AE", "+2348031234567": "NG", "+8005550100": "", "": "",
	} {
		if got := regionForPhone(number); got != want {
			t.Errorf("%s: got %q, want %s", number, got, want)
		}
	}
}
