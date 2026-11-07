package main

import (
	"regexp"
	"strings"
)

// Meta AI writes its replies in CommonMark/Markdown.
// aiMarkdownToPlain transforms rich Markdown into WhatsApp-compatible formatting:
// headings (# Heading) become bold (*Heading*), **bold** becomes *bold*,
// code blocks and inline code are preserved, and list items are normalized to "- ".
// Quote markers are dropped and table cells joined with commas, which read well.
// Sighted users see the formatting in message bubbles; the frontend strips the
// markers from what a screen reader hears and from previews and notifications.

var (
	mdImage      = regexp.MustCompile(`!\[([^\]]*)\]\(([^)\s]+)[^)]*\)`)
	mdLink       = regexp.MustCompile(`\[([^\]]+)\]\(([^)\s]+)[^)]*\)`)
	mdBold       = regexp.MustCompile(`\*\*([^*\n]+?)\*\*`)
	mdInlineCode = regexp.MustCompile("`([^`\n]+)`")
	mdHeading    = regexp.MustCompile(`^\s{0,3}#{1,6}\s+`)
	mdBullet     = regexp.MustCompile(`^(\s*)[*+]\s+`)
	mdQuote      = regexp.MustCompile(`^\s*>\s?`)
	mdRule       = regexp.MustCompile(`^\s*([-*_])(\s*[-*_]){2,}\s*$`)
	mdTableSep   = regexp.MustCompile(`^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$`)
	mdEscape     = regexp.MustCompile(`\\([\\` + "`" + `*_{}\[\]()#+\-.!|>~])`)
)

func aiMarkdownToPlain(text string) string {
	if text == "" {
		return ""
	}
	lines := strings.Split(strings.ReplaceAll(text, "\r\n", "\n"), "\n")
	out := make([]string, 0, len(lines))
	inCode := false
	for _, line := range lines {
		trimmed := strings.TrimSpace(line)
		if strings.HasPrefix(trimmed, "```") {
			inCode = !inCode
			out = append(out, "```")
			continue
		}
		if inCode {
			out = append(out, line)
			continue
		}
		if mdRule.MatchString(line) || mdTableSep.MatchString(line) {
			continue
		}
		if mdHeading.MatchString(line) {
			heading := strings.TrimSpace(mdHeading.ReplaceAllString(line, ""))
			line = "*" + heading + "*"
		}
		line = mdQuote.ReplaceAllString(line, "")
		line = mdBullet.ReplaceAllString(line, "$1- ")
		if strings.HasPrefix(trimmed, "|") && strings.HasSuffix(trimmed, "|") && len(trimmed) > 1 {
			cells := strings.Split(trimmed[1:len(trimmed)-1], "|")
			for i := range cells {
				cells[i] = strings.TrimSpace(cells[i])
			}
			line = strings.Join(cells, ", ")
		}
		out = append(out, mdInline(line))
	}
	return strings.TrimSpace(strings.Join(out, "\n"))
}

func mdInline(line string) string {
	line = mdImage.ReplaceAllStringFunc(line, func(m string) string {
		parts := mdImage.FindStringSubmatch(m)
		return labeledLink(parts[1], parts[2])
	})
	line = mdLink.ReplaceAllStringFunc(line, func(m string) string {
		parts := mdLink.FindStringSubmatch(m)
		return labeledLink(parts[1], parts[2])
	})
	// Convert Markdown **bold** to WhatsApp *bold*
	line = mdBold.ReplaceAllString(line, "*$1*")
	// Inline code `code` is preserved for WhatsApp monospace
	return mdEscape.ReplaceAllString(line, "$1")
}

// labeledLink keeps a link's address in the text, so the message list can
// still find and open it, without repeating it when the label is the address.
func labeledLink(label, url string) string {
	label = strings.TrimSpace(label)
	if label == "" || label == url || strings.TrimPrefix(strings.TrimPrefix(url, "https://"), "http://") == label {
		return url
	}
	return label + " (" + url + ")"
}
