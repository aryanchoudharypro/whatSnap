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
	if got := channelStatsFromColumns(3, `{"👍":2}`); got != "3 views, 👍 2" {
		t.Errorf("channelStatsFromColumns = %q", got)
	}
}

func TestRegionFromPhone(t *testing.T) {
	for number, want := range map[string]string{"+919800000004": "IN", "+14155550100": "US", "+447700900123": "GB"} {
		if got := regionForPhone(number); got != want {
			t.Errorf("%s: got %q, want %s", number, got, want)
		}
	}
}
