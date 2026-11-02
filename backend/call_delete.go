package main

import (
	"context"
	"encoding/json"
	"log"
	"strings"
	"time"

	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/types"
)

// Deleting calls from the Calls tab works the way WA Web's does
// (WAWebVoipCallsTabClearCallLogAction, WAWebCallLogEntryCell): a call is a
// call log message in its chat, keyed by the call id, and deleting it is a
// plain Delete for me of that message - one deleteMessageForMe mutation per
// call in regular_high, media kept. There is no call specific action: WA Web
// never sends delete_individual_call_log.

// deleteForMeBatch caps the mutations in one patch, so clearing a long call
// history doesn't build one huge patch.
const deleteForMeBatch = 100

type DeleteCallLogsData struct {
	CallIDs []string `json:"call_ids"`
	All     bool     `json:"all"`
}

type callDeleteKey struct {
	callID  string
	jid     string
	keyChat string
	creator string
	fromMe  bool
	isGroup bool
	ts      int64
}

func handleDeleteCallLogs(raw json.RawMessage) {
	var data DeleteCallLogsData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		keys := loadCallDeleteKeys(data.CallIDs, data.All)
		if len(keys) == 0 {
			broadcastEvent("call_logs_changed", map[string]interface{}{})
			return
		}
		if wa() != nil {
			sendCallLogDeletes(keys)
		}
		for _, k := range keys {
			removeCallLocally(k.callID, k.jid)
		}
		broadcastChats()
		broadcastEvent("call_logs_changed", map[string]interface{}{})
	}()
}

func loadCallDeleteKeys(ids []string, all bool) []callDeleteKey {
	q := `SELECT call_id, jid, COALESCE(key_chat, ''), COALESCE(creator, ''), from_me, is_group, COALESCE(timestamp, 0) FROM call_records`
	var args []interface{}
	if !all {
		if len(ids) == 0 {
			return nil
		}
		q += ` WHERE call_id IN (?` + strings.Repeat(`, ?`, len(ids)-1) + `)`
		for _, id := range ids {
			args = append(args, id)
		}
	}
	rows, err := db.Query(q, args...)
	if err != nil {
		log.Printf("delete call logs: query failed: %v", err)
		return nil
	}
	defer rows.Close()
	var keys []callDeleteKey
	for rows.Next() {
		var k callDeleteKey
		var fromMe, group int
		if rows.Scan(&k.callID, &k.jid, &k.keyChat, &k.creator, &fromMe, &group, &k.ts) != nil {
			continue
		}
		k.fromMe, k.isGroup = fromMe == 1, group == 1
		keys = append(keys, k)
	}
	return keys
}

// callDeleteMutation builds the Delete for me of one call log message, or
// reports false when the call's key isn't known well enough to name it (rows
// stored before keys were kept, calls joined later, a group call someone else
// placed whose creator never reached us). Those are only removed here.
func callDeleteMutation(k callDeleteKey) (types.JID, appstate.MutationInfo, bool) {
	keyChat := k.keyChat
	if keyChat == "" && !k.isGroup {
		// A one to one call stored before keys were kept is filed under the
		// other person, which is also where WA Web keys it.
		keyChat = k.jid
	}
	if keyChat == "" {
		return types.EmptyJID, appstate.MutationInfo{}, false
	}
	chat, err := types.ParseJID(keyChat)
	if err != nil || chat.IsEmpty() {
		return types.EmptyJID, appstate.MutationInfo{}, false
	}
	sender := types.EmptyJID
	if chat.Server == types.GroupServer && !k.fromMe {
		if k.creator == "" {
			return types.EmptyJID, appstate.MutationInfo{}, false
		}
		if sender, err = types.ParseJID(k.creator); err != nil || sender.IsEmpty() {
			return types.EmptyJID, appstate.MutationInfo{}, false
		}
	}
	chat = callKeyChatWire(chat)
	ts := time.Now()
	if k.ts > 0 {
		ts = time.Unix(k.ts, 0)
	}
	patch := appstate.BuildDeleteMessageForMe(chat, sender, types.MessageID(k.callID), k.fromMe, false, ts)
	if len(patch.Mutations) != 1 {
		return types.EmptyJID, appstate.MutationInfo{}, false
	}
	return chat, patch.Mutations[0], true
}

// callKeyChatWire puts a one to one chat in the addressing WA Web's syncd
// index uses for it: the LID once the account has migrated, the phone number
// before that.
func callKeyChatWire(chat types.JID) types.JID {
	chat = chat.ToNonAD()
	if chat.Server == types.HiddenUserServer && wa() != nil && wa().Store != nil && wa().Store.LIDMigrationTimestamp == 0 {
		if pn, err := wa().Store.LIDs.GetPNForLID(context.Background(), chat); err == nil && !pn.IsEmpty() {
			return pn.ToNonAD()
		}
		return chat
	}
	return syncdChatJID(chat)
}

// sendCallLogDeletes sends the mutations a chat at a time, the way WA Web's
// sendDeleteMsgs does, in batches of deleteForMeBatch.
func sendCallLogDeletes(keys []callDeleteKey) {
	byChat := map[types.JID][]appstate.MutationInfo{}
	var order []types.JID
	for _, k := range keys {
		chat, m, ok := callDeleteMutation(k)
		if !ok {
			log.Printf("delete call logs: %s has no known key, removing it here only", k.callID)
			continue
		}
		if _, seen := byChat[chat]; !seen {
			order = append(order, chat)
		}
		byChat[chat] = append(byChat[chat], m)
	}
	for _, chat := range order {
		muts := byChat[chat]
		for len(muts) > 0 {
			n := min(len(muts), deleteForMeBatch)
			patch := appstate.PatchInfo{Type: appstate.WAPatchRegularHigh, Mutations: muts[:n]}
			if err := sendAppStateWithRetry(context.Background(), patch); err != nil {
				log.Printf("delete call logs: app-state sync for %s failed: %v", chat, err)
			}
			muts = muts[n:]
		}
	}
}

// removeCallLocally drops a call from the Calls tab and its chat, and
// remembers it so a later sync of the same call doesn't bring it back.
func removeCallLocally(callID, jid string) {
	db.Exec(`INSERT OR IGNORE INTO deleted_calls (call_id) VALUES (?)`, callID)
	db.Exec(`DELETE FROM call_records WHERE call_id = ?`, callID)
	purgeMessages(`id = ?`, "call:"+callID)
	if jid != "" {
		UpdateChatSnippet(jid)
		broadcastEvent("message_deleted", map[string]interface{}{
			"jid": jid, "message_id": "call:" + callID, "by": "me", "for_everyone": false,
		})
	}
}

// applySyncedCallDelete handles a Delete for me from another device that names
// a call: WA Web's Calls tab sends exactly that. The message id is the bare
// call id, while whatSnap stores the call as "call:<id>", possibly under the
// other addressing of the chat, so the ordinary delete never finds it.
func applySyncedCallDelete(messageID string) {
	if messageID == "" || db == nil {
		return
	}
	var jid string
	if db.QueryRow(`SELECT jid FROM call_records WHERE call_id = ?`, messageID).Scan(&jid) != nil {
		return
	}
	removeCallLocally(messageID, jid)
	broadcastChats()
	broadcastEvent("call_logs_changed", map[string]interface{}{})
}

func ownCallCreator() types.JID {
	if wa() == nil || wa().Store == nil || wa().Store.ID == nil {
		return types.EmptyJID
	}
	return wa().Store.ID.ToNonAD()
}
