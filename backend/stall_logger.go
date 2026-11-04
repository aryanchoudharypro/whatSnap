package main

import (
	"fmt"
	"strings"

	waLog "go.mau.fi/whatsmeow/util/log"
)

// stallLogger passes everything to the real logger, and writes a goroutine
// dump the first time whatsmeow warns that handling one stanza is taking
// long. whatsmeow handles stanzas one at a time, so one stuck event handler
// holds up everything after it (2026-11-04: an offline voice note stuck for
// minutes after waking the PC, leaving the sync at 40%); the dump shows
// where it's stuck.
type stallLogger struct {
	waLog.Logger
}

func (l stallLogger) Warnf(msg string, args ...any) {
	l.Logger.Warnf(msg, args...)
	if strings.HasPrefix(msg, "Node handling is taking long") {
		reason := fmt.Sprintf(msg, args...)
		if len(reason) > 400 {
			reason = reason[:400] + "..."
		}
		go dumpGoroutinesForStall(reason)
	}
}

func (l stallLogger) Sub(module string) waLog.Logger {
	return stallLogger{l.Logger.Sub(module)}
}
