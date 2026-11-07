package main

import "testing"

func TestAIMarkdownToPlain(t *testing.T) {
	cases := map[string]string{
		"# Hello there\n- Guinea pig duty accepted.":                         "*Hello there*\n- Guinea pig duty accepted.",
		"- Went to [Accessible Diabetes Tracker](https://www.jjhof.com/adt)": "- Went to Accessible Diabetes Tracker (https://www.jjhof.com/adt)",
		"[https://x.com](https://x.com)":                                     "https://x.com",
		"* **bold** and `code`":                                              "- *bold* and `code`",
		"| a | b |\n|---|---|\n| 1 | 2 |":                                    "a, b\n1, 2",
		"```go\nx := 1\n```":                                                 "```\nx := 1\n```",
		"a\n---\nb \\*c\\*":                                                  "a\nb *c*",
	}
	for in, want := range cases {
		if got := aiMarkdownToPlain(in); got != want {
			t.Errorf("aiMarkdownToPlain(%q) = %q, want %q", in, got, want)
		}
	}
}
