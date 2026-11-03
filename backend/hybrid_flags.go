package main

import "log"

// Server flags that move WhatsApp for Windows' calls onto WA Web's own
// calling engine (Hybrid+). Logged so a change on the server shows up in
// backend.log; meowcaller handles calls either way.
const (
	abPropWinHybridPlus               = 33753
	abPropWinHybridPlusIncomingPopout = 35379
)

func logHybridServerFlags() {
	if wa() == nil {
		return
	}
	props := wa().CachedABProps()
	value := func(code int) string {
		if v, ok := props[code]; ok {
			return v
		}
		return "not sent"
	}
	log.Printf("hybrid: server flags: wa_web_win_hybrid_plus_enabled (33753) = %s, wa_web_win_hybrid_plus_incoming_popout (35379) = %s (of %d flags)",
		value(abPropWinHybridPlus), value(abPropWinHybridPlusIncomingPopout), len(props))
}
