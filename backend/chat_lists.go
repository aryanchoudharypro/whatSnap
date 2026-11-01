package main

import (
	"context"
	"encoding/json"
	"net"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waSyncAction"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

type ChatListData struct {
	ID     string   `json:"id"`
	Name   string   `json:"name"`
	JID    string   `json:"jid"`
	JIDs   []string `json:"jids"`
	Member bool     `json:"member"`
}

type FavoriteData struct {
	JID      string `json:"jid"`
	Favorite bool   `json:"favorite"`
}

var listsMu sync.Mutex

var favoritesSendMu sync.Mutex

func shownListType(t waSyncAction.LabelEditAction_ListType) bool {
	switch t {
	case waSyncAction.LabelEditAction_CUSTOM, waSyncAction.LabelEditAction_NONE, waSyncAction.LabelEditAction_PREDEFINED:
		return true
	}
	return false
}

func noteLabelEdit(v *events.LabelEdit) {
	if v.Action == nil || v.LabelID == "" {
		return
	}
	listsMu.Lock()
	if v.Action.GetDeleted() {
		db.Exec(`DELETE FROM chat_lists WHERE id = ?`, v.LabelID)
		db.Exec(`DELETE FROM chat_list_members WHERE list_id = ?`, v.LabelID)
	} else if raw, err := proto.Marshal(v.Action); err == nil {
		db.Exec(`INSERT OR REPLACE INTO chat_lists (id, action) VALUES (?, ?)`, v.LabelID, raw)
	}
	listsMu.Unlock()
	broadcastChatLists()
}

func noteLabelChat(v *events.LabelAssociationChat) {
	if v.Action == nil || v.LabelID == "" || v.JID.IsEmpty() {
		return
	}
	setListMember(v.LabelID, appStateChatJID(v.JID.String()), v.Action.GetLabeled())
	broadcastChatLists()
}

func setListMember(id, jid string, member bool) {
	listsMu.Lock()
	defer listsMu.Unlock()
	if member {
		db.Exec(`INSERT OR IGNORE INTO chat_list_members (list_id, jid) VALUES (?, ?)`, id, jid)
	} else {
		db.Exec(`DELETE FROM chat_list_members WHERE list_id = ? AND jid = ?`, id, jid)
	}
}

func noteFavorites(v *events.Favorites) {
	if v.Action == nil {
		return
	}
	var jids []string
	for _, f := range v.Action.GetFavorites() {
		if f.GetID() != "" {
			jids = append(jids, appStateChatJID(f.GetID()))
		}
	}
	storeFavorites(jids)
	broadcastChatLists()
}

func storeFavorites(jids []string) {
	listsMu.Lock()
	defer listsMu.Unlock()
	tx, err := db.Begin()
	if err != nil {
		return
	}
	tx.Exec(`DELETE FROM chat_favorites`)
	for i, jid := range jids {
		tx.Exec(`INSERT OR IGNORE INTO chat_favorites (jid, position) VALUES (?, ?)`, jid, i)
	}
	tx.Commit()
}

func storedFavorites() []string {
	jids := []string{}
	rows, err := db.Query(`SELECT jid FROM chat_favorites ORDER BY position`)
	if err != nil {
		return jids
	}
	defer rows.Close()
	for rows.Next() {
		var jid string
		if rows.Scan(&jid) == nil {
			jids = append(jids, jid)
		}
	}
	return jids
}

type storedList struct {
	id     string
	action *waSyncAction.LabelEditAction
}

func storedLists() []storedList {
	var lists []storedList
	rows, err := db.Query(`SELECT id, action FROM chat_lists`)
	if err != nil {
		return nil
	}
	defer rows.Close()
	for rows.Next() {
		var id string
		var raw []byte
		if rows.Scan(&id, &raw) != nil {
			continue
		}
		var action waSyncAction.LabelEditAction
		if proto.Unmarshal(raw, &action) == nil {
			lists = append(lists, storedList{id, &action})
		}
	}
	return lists
}

func findList(id string) *waSyncAction.LabelEditAction {
	for _, l := range storedLists() {
		if l.id == id {
			return l.action
		}
	}
	return nil
}

func chatListsPayload() map[string]interface{} {
	members := map[string][]string{}
	if rows, err := db.Query(`SELECT list_id, jid FROM chat_list_members`); err == nil {
		for rows.Next() {
			var id, jid string
			if rows.Scan(&id, &jid) == nil {
				members[id] = append(members[id], jid)
			}
		}
		rows.Close()
	}
	type entry struct {
		order int64
		id    string
		item  map[string]interface{}
	}
	var entries []entry
	for _, l := range storedLists() {
		if !shownListType(l.action.GetType()) || strings.TrimSpace(l.action.GetName()) == "" {
			continue
		}
		num, _ := strconv.ParseInt(l.id, 10, 64)
		order := num
		if l.action.OrderIndex != nil {
			order = int64(l.action.GetOrderIndex())
		}
		jids := members[l.id]
		if jids == nil {
			jids = []string{}
		}
		entries = append(entries, entry{order, l.id, map[string]interface{}{
			"id": l.id, "name": l.action.GetName(), "jids": jids,
		}})
	}
	sort.Slice(entries, func(i, j int) bool {
		if entries[i].order != entries[j].order {
			return entries[i].order < entries[j].order
		}
		return entries[i].id < entries[j].id
	})
	lists := make([]map[string]interface{}, 0, len(entries))
	for _, e := range entries {
		lists = append(lists, e.item)
	}
	return map[string]interface{}{"favorites": storedFavorites(), "lists": lists}
}

func broadcastChatLists() {
	broadcastEvent("chat_lists", chatListsPayload())
}

func handleGetChatLists(conn net.Conn) {
	sendEvent(conn, "chat_lists", chatListsPayload())
}

func listError(what string, err error) {
	broadcastEvent("error", map[string]string{"message": "Couldn't " + what + ": " + err.Error()})
}

func handleSetFavorite(conn net.Conn, raw json.RawMessage) {
	var data FavoriteData
	if err := json.Unmarshal(raw, &data); err != nil || data.JID == "" {
		return
	}
	go func() {
		favoritesSendMu.Lock()
		defer favoritesSendMu.Unlock()
		var jids []string
		for _, jid := range storedFavorites() {
			if jid != data.JID {
				jids = append(jids, jid)
			}
		}
		if data.Favorite {
			jids = append(jids, data.JID)
		}
		wire := make([]types.JID, 0, len(jids))
		for _, s := range jids {
			if jid, err := types.ParseJID(s); err == nil {
				wire = append(wire, syncdChatJID(jid))
			}
		}
		if err := sendAppStateWithRetry(context.Background(), appstate.BuildFavorites(wire)); err != nil {
			listError("change your favorites", err)
			return
		}
		storeFavorites(jids)
		broadcastChatLists()
		sendEvent(conn, "favorite_saved", map[string]interface{}{"jid": data.JID, "favorite": data.Favorite})
	}()
}

func handleCreateList(conn net.Conn, raw json.RawMessage) {
	var data ChatListData
	if err := json.Unmarshal(raw, &data); err != nil || strings.TrimSpace(data.Name) == "" {
		return
	}
	go func() {
		var maxID, maxOrder int64
		for _, l := range storedLists() {
			if n, err := strconv.ParseInt(l.id, 10, 64); err == nil && n > maxID {
				maxID = n
			}
			if o := int64(l.action.GetOrderIndex()); o > maxOrder {
				maxOrder = o
			}
		}
		id := strconv.FormatInt(maxID+1, 10)
		action := &waSyncAction.LabelEditAction{
			Name:       proto.String(strings.TrimSpace(data.Name)),
			Deleted:    proto.Bool(false),
			OrderIndex: proto.Int32(int32(maxOrder + 1)),
			IsActive:   proto.Bool(true),
			Type:       waSyncAction.LabelEditAction_CUSTOM.Enum(),
		}
		if err := sendAppStateWithRetry(context.Background(), appstate.BuildLabelEditAction(id, action)); err != nil {
			listError("create the list", err)
			return
		}
		noteLabelEdit(&events.LabelEdit{Timestamp: time.Now(), LabelID: id, Action: action})
		for _, jid := range data.JIDs {
			sendListMember(id, jid, true)
		}
		broadcastChatLists()
		sendEvent(conn, "list_saved", map[string]interface{}{"id": id, "name": action.GetName(), "created": true})
	}()
}

func handleRenameList(conn net.Conn, raw json.RawMessage) {
	var data ChatListData
	if err := json.Unmarshal(raw, &data); err != nil || strings.TrimSpace(data.Name) == "" {
		return
	}
	go func() {
		old := findList(data.ID)
		if old == nil {
			return
		}
		action := proto.Clone(old).(*waSyncAction.LabelEditAction)
		action.Name = proto.String(strings.TrimSpace(data.Name))
		if err := sendAppStateWithRetry(context.Background(), appstate.BuildLabelEditAction(data.ID, action)); err != nil {
			listError("rename the list", err)
			return
		}
		noteLabelEdit(&events.LabelEdit{Timestamp: time.Now(), LabelID: data.ID, Action: action})
		sendEvent(conn, "list_saved", map[string]interface{}{"id": data.ID, "name": action.GetName()})
	}()
}

func handleDeleteList(conn net.Conn, raw json.RawMessage) {
	var data ChatListData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		old := findList(data.ID)
		if old == nil {
			return
		}
		action := proto.Clone(old).(*waSyncAction.LabelEditAction)
		action.Deleted = proto.Bool(true)
		if err := sendAppStateWithRetry(context.Background(), appstate.BuildLabelEditAction(data.ID, action)); err != nil {
			listError("delete the list", err)
			return
		}
		noteLabelEdit(&events.LabelEdit{Timestamp: time.Now(), LabelID: data.ID, Action: action})
		sendEvent(conn, "list_deleted", map[string]interface{}{"id": data.ID, "name": old.GetName()})
	}()
}

func handleSetListMember(conn net.Conn, raw json.RawMessage) {
	var data ChatListData
	if err := json.Unmarshal(raw, &data); err != nil || data.ID == "" {
		return
	}
	jids := data.JIDs
	if data.JID != "" {
		jids = append(jids, data.JID)
	}
	if len(jids) == 0 {
		return
	}
	go func() {
		var changed []string
		for _, jid := range jids {
			if sendListMember(data.ID, jid, data.Member) {
				changed = append(changed, jid)
			}
		}
		if len(changed) == 0 {
			return
		}
		broadcastChatLists()
		name := ""
		if l := findList(data.ID); l != nil {
			name = l.GetName()
		}
		sendEvent(conn, "list_member_saved", map[string]interface{}{
			"id": data.ID, "name": name, "jid": changed[0], "count": len(changed), "member": data.Member,
		})
	}()
}

func sendListMember(id, chat string, member bool) bool {
	jid, err := types.ParseJID(chat)
	if err != nil {
		return false
	}
	if err := sendAppStateWithRetry(context.Background(), appstate.BuildLabelChat(syncdChatJID(jid), id, member)); err != nil {
		listError("change the list", err)
		return false
	}
	setListMember(id, chat, member)
	return true
}

func handleGetCommonGroups(conn net.Conn, raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	person, err := types.ParseJID(data.JID)
	if err != nil || person.Server == types.GroupServer {
		return
	}
	person = person.ToNonAD()
	twin := storeTwin(person).ToNonAD()
	groups, err := serverGroupList()
	if err != nil {
		sendEvent(conn, "error", map[string]string{"message": "Couldn't load the groups in common: " + err.Error()})
		return
	}
	same := func(j types.JID) bool {
		j = j.ToNonAD()
		return !j.IsEmpty() && (j == person || (!twin.IsEmpty() && j == twin))
	}
	list := []map[string]string{}
	for _, g := range groups {
		if g.IsParent {
			continue
		}
		for _, p := range g.Participants {
			if same(p.JID) || same(p.PhoneNumber) || same(p.LID) {
				list = append(list, map[string]string{"jid": g.JID.String(), "name": g.Name})
				break
			}
		}
	}
	sendEvent(conn, "common_groups", map[string]interface{}{"jid": data.JID, "groups": list})
}
