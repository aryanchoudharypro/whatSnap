package main

import (
	"database/sql"
	"log"
	"strings"

	"go.mau.fi/whatsmeow/proto/waHistorySync"
)

// historyChatGate decides which history-sync conversations may touch the
// chat list, the way WA Web does it.
//
// WA Web creates chats only from the INITIAL_BOOTSTRAP chunk, which holds
// the phone's real chat list. RECENT, FULL and ON_DEMAND chunks only add
// messages to chats that already exist: a conversation with no chat yet is
// dropped (WAWebHandleHistorySyncChunk logs it as "dropped N chats, null
// boundary"). The FULL chunk also carries every thread the phone still
// keeps a stub for, such as chats deleted on the phone and groups left and
// deleted there, each with a single leftover message. Listing those
// brought back hundreds of deleted chats on a fresh link. The list of
// conversations the phone left out (omittedConversationMetadata) isn't
// used by WA Web at all, so it only fills in chats we already have.
//
// If no bootstrap chunk ever arrived for this device, nothing is gated, so
// a phone that skips it still gets its chats listed.
type historyChatGate struct {
	gated  bool
	joined map[string]bool
}

func historyBootstrapKey() string {
	cli := wa()
	if cli == nil || cli.Store == nil || cli.Store.ID == nil {
		return ""
	}
	// The device ID changes with every link, so a relink starts over.
	return "history_bootstrap:" + cli.Store.ID.String()
}

func newHistoryChatGate(syncType waHistorySync.HistorySync_HistorySyncType) *historyChatGate {
	key := historyBootstrapKey()
	if syncType == waHistorySync.HistorySync_INITIAL_BOOTSTRAP {
		if key != "" {
			db.Exec(`INSERT OR REPLACE INTO app_settings (key, value) VALUES (?, '1')`, key)
		}
		return &historyChatGate{}
	}
	var seen int
	if key != "" {
		db.QueryRow(`SELECT COUNT(*) FROM app_settings WHERE key = ?`, key).Scan(&seen)
	}
	return &historyChatGate{gated: seen > 0}
}

// allows reports whether the conversation for jid (and its LID form,
// wireJID, if it came as one) may be written. omitted marks a conversation
// from the omitted-chats list, which never adds a chat.
func (g *historyChatGate) allows(jid, wireJID string, omitted bool) bool {
	if !g.gated && !omitted {
		return true
	}
	if historyChatExists(jid, wireJID) {
		return true
	}
	// A group we're in is listed by populateChatsFromStore anyway; its row
	// may just not be written yet this soon after linking.
	if strings.HasSuffix(jid, "@g.us") {
		return g.isJoined(jid)
	}
	return false
}

func (g *historyChatGate) isJoined(jid string) bool {
	if g.joined == nil {
		g.joined = map[string]bool{}
		groups, err := serverGroupList()
		if err != nil {
			log.Println("History sync: couldn't fetch joined groups:", err)
			return false
		}
		for _, gi := range groups {
			g.joined[gi.JID.ToNonAD().String()] = true
		}
	}
	return g.joined[jid]
}

func historyChatExists(jid, wireJID string) bool {
	var n int
	err := db.QueryRow(`SELECT COUNT(*) FROM chats WHERE jid = ? OR (? != '' AND (jid = ? OR wire_jid = ?))`,
		jid, wireJID, wireJID, wireJID).Scan(&n)
	if err != nil && err != sql.ErrNoRows {
		return false
	}
	return n > 0
}
