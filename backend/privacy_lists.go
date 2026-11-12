package main

import (
	"context"
	"encoding/json"
	"fmt"
	"net"

	"go.mau.fi/whatsmeow/types"
)

var privacyListCategories = map[string]bool{
	string(types.PrivacySettingTypeLastSeen): true,
	string(types.PrivacySettingTypeProfile):  true,
	string(types.PrivacySettingTypeStatus):   true,
	string(types.PrivacySettingTypeGroupAdd): true,
}

type PrivacyListData struct {
	Name string   `json:"name"`
	JIDs []string `json:"jids"`
}

type privacyListEntry struct {
	JID   string `json:"jid"`
	Name  string `json:"name"`
	Phone string `json:"phone"`
}

type privacyListPerson struct {
	LID types.JID
	PN  types.JID
}

func (p privacyListPerson) shownJID() types.JID {
	if !p.PN.IsEmpty() {
		return p.PN
	}
	return p.LID
}

func peopleFromList(list *types.PrivacyDisallowedList, twin jidTwin) []privacyListPerson {
	people := make([]privacyListPerson, 0, len(list.Users))
	for _, user := range list.Users {
		person := privacyListPerson{PN: user.PN.ToNonAD()}
		if user.JID.Server == types.HiddenUserServer {
			person.LID = user.JID.ToNonAD()
		} else if person.PN.IsEmpty() {
			person.PN = user.JID.ToNonAD()
		}
		if person.PN.IsEmpty() && !person.LID.IsEmpty() {
			person.PN = twin(person.LID).ToNonAD()
		}
		if person.LID.IsEmpty() && !person.PN.IsEmpty() {
			person.LID = twin(person.PN).ToNonAD()
		}
		people = append(people, person)
	}
	return people
}

func privacyListPayload(name string, people []privacyListPerson, nameFor func(types.JID) string) map[string]interface{} {
	entries := make([]privacyListEntry, 0, len(people))
	for _, person := range people {
		shown := person.shownJID()
		entry := privacyListEntry{JID: shown.String(), Name: nameFor(shown)}
		if !person.PN.IsEmpty() {
			entry.Phone = person.PN.User
		}
		entries = append(entries, entry)
	}
	return map[string]interface{}{"name": name, "entries": entries}
}

func samePerson(a, b privacyListPerson) bool {
	return (!a.LID.IsEmpty() && a.LID == b.LID) || (!a.PN.IsEmpty() && a.PN == b.PN)
}

func containsPerson(list []privacyListPerson, p privacyListPerson) bool {
	for _, other := range list {
		if samePerson(other, p) {
			return true
		}
	}
	return false
}

func privacyListChanges(current, wanted []privacyListPerson) []types.PrivacyListChange {
	change := func(action types.PrivacyListAction, p privacyListPerson) types.PrivacyListChange {
		jid, pn := p.LID, p.PN
		if jid.IsEmpty() {
			jid, pn = p.PN, types.EmptyJID
		}
		return types.PrivacyListChange{Action: action, JID: jid, PN: pn}
	}
	var changes []types.PrivacyListChange
	for _, person := range current {
		if !containsPerson(wanted, person) {
			changes = append(changes, change(types.PrivacyListActionRemove, person))
		}
	}
	var added []privacyListPerson
	for _, person := range wanted {
		if !containsPerson(current, person) && !containsPerson(added, person) {
			added = append(added, person)
			changes = append(changes, change(types.PrivacyListActionAdd, person))
		}
	}
	return changes
}

func listedPerson(list []privacyListPerson, jid types.JID) (privacyListPerson, bool) {
	jid = jid.ToNonAD()
	for _, person := range list {
		if jid == person.LID || jid == person.PN {
			return person, true
		}
	}
	return privacyListPerson{}, false
}

func (acc *Account) resolvePrivacyListPerson(ctx context.Context, jid types.JID) (privacyListPerson, error) {
	jid = jid.ToNonAD()
	person := privacyListPerson{}
	switch jid.Server {
	case types.HiddenUserServer:
		person.LID = jid
		person.PN = acc.storeTwin(jid).ToNonAD()
	case types.DefaultUserServer:
		person.PN = jid
		person.LID = acc.storeTwin(jid).ToNonAD()
		if person.LID.IsEmpty() {
			if info, err := acc.wa().GetUserInfo(ctx, []types.JID{jid}); err == nil {
				person.LID = info[jid].LID.ToNonAD()
			}
		}
	default:
		return person, fmt.Errorf("%s can't be on a privacy list", jid)
	}
	if person.LID.IsEmpty() {
		return person, fmt.Errorf("couldn't find %s on WhatsApp", acc.resolveDisplayName(jid))
	}
	return person, nil
}

func (acc *Account) handleGetPrivacyList(conn net.Conn, raw json.RawMessage) {
	var data PrivacyListData
	if err := json.Unmarshal(raw, &data); err != nil || !privacyListCategories[data.Name] {
		return
	}
	go func() {
		list, err := acc.wa().GetPrivacyDisallowedList(context.Background(), types.PrivacySettingType(data.Name))
		if err != nil {
			acc.sendEvent(conn, "privacy_list", map[string]interface{}{"name": data.Name, "error": err.Error()})
			return
		}
		acc.sendEvent(conn, "privacy_list", privacyListPayload(data.Name, peopleFromList(list, acc.storeTwin), acc.resolveDisplayName))
	}()
}

func (acc *Account) handleSetPrivacyList(raw json.RawMessage) {
	var data PrivacyListData
	if err := json.Unmarshal(raw, &data); err != nil || !privacyListCategories[data.Name] {
		return
	}
	go func() {
		ctx := context.Background()
		name := types.PrivacySettingType(data.Name)
		fail := func(err error) {
			acc.broadcastEvent("error", map[string]string{"message": "Couldn't save the excluded contacts: " + err.Error()})
		}
		list, err := acc.wa().GetPrivacyDisallowedList(ctx, name)
		if err != nil {
			fail(err)
			return
		}
		current := peopleFromList(list, acc.storeTwin)
		wanted := make([]privacyListPerson, 0, len(data.JIDs))
		for _, jidStr := range data.JIDs {
			jid, err := types.ParseJID(jidStr)
			if err != nil {
				continue
			}
			if person, ok := listedPerson(current, jid); ok {
				wanted = append(wanted, person)
				continue
			}
			person, err := acc.resolvePrivacyListPerson(ctx, jid)
			if err != nil {
				fail(err)
				return
			}
			wanted = append(wanted, person)
		}
		changes := privacyListChanges(current, wanted)
		if _, err := acc.wa().SetPrivacyDisallowedList(ctx, name, list.DHash, changes); err != nil {
			fail(err)
			return
		}
		if settings, err := acc.wa().TryFetchPrivacySettings(ctx, false); err == nil {
			acc.broadcastEvent("privacy_settings", privacySettingsToMap(settings))
		}
		if fresh, err := acc.wa().GetPrivacyDisallowedList(ctx, name); err == nil {
			acc.broadcastEvent("privacy_list", privacyListPayload(data.Name, peopleFromList(fresh, acc.storeTwin), acc.resolveDisplayName))
		}
	}()
}
