package main

import (
	"context"
	"encoding/json"
	"net"
	"strings"
	"time"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
)

func (acc *Account) isSavedContact(jid types.JID) bool {
	if acc.wa() == nil || acc.wa().Store == nil || acc.wa().Store.Contacts == nil {
		return false
	}
	pn, lid := acc.contactForms(jid)
	for _, form := range []types.JID{pn, lid} {
		if form.IsEmpty() {
			continue
		}
		if c, err := acc.wa().Store.Contacts.GetContact(context.Background(), form); err == nil && (c.FullName != "" || c.FirstName != "") {
			return true
		}
	}
	return false
}

func (acc *Account) contactCardJSON(card parsedVcard) contactCardJSON {
	out := contactCardJSON{Name: vcardName(card), Org: vcardOrganization(card), Business: vcardIsBusiness(card), LID: vcardLID(card)}
	out.FirstName, out.LastName = vcardNames(card)
	out.Details = cardDetails(card)
	for _, tel := range vcardWhatsAppNumbers(card) {
		digits := tel.props["waid"][0]
		out.Numbers = append(out.Numbers, cardNumber{
			JID: types.NewJID(digits, types.DefaultUserServer).String(), Digits: digits, Label: vcardType(tel),
		})
	}
	var main types.JID
	switch {
	case out.LID != "":
		main = types.NewJID(out.LID, types.HiddenUserServer)
	case len(out.Numbers) > 0:
		main = types.NewJID(out.Numbers[0].Digits, types.DefaultUserServer)
	}
	if out.LID != "" {
		// Chats are kept under the phone number when it's known.
		out.LIDChat = acc.preferPN(main).String()
	}
	if !main.IsEmpty() {
		out.Saved = acc.isSavedContact(main)
		for _, n := range out.Numbers {
			out.Me = out.Me || acc.isOwnJID(types.NewJID(n.Digits, types.DefaultUserServer))
		}
		out.Me = out.Me || (out.LID != "" && acc.isOwnJID(main))
	}
	// The save form needs a phone number, as WA Web's does unless usernames are on.
	out.CanSave = len(out.Numbers) > 0 && !out.Me
	return out
}

// canAddToGroup is WA Web's isContactAddableToGroup: the open chat is a
// group I admin and the person isn't in it yet.
func (acc *Account) canAddToGroup(chat types.JID, card contactCardJSON) (bool, string) {
	if chat.Server != types.GroupServer || acc.wa() == nil {
		return false, ""
	}
	var isAdmin int
	var name string
	if acc.db.QueryRow(`SELECT COALESCE(is_admin, 0), COALESCE(name, '') FROM chats WHERE jid = ?`, chat.String()).Scan(&isAdmin, &name) != nil || isAdmin == 0 {
		return false, ""
	}
	var person types.JID
	switch {
	case card.LID != "":
		person = types.NewJID(card.LID, types.HiddenUserServer)
	case len(card.Numbers) > 0:
		person = types.NewJID(card.Numbers[0].Digits, types.DefaultUserServer)
	default:
		return false, ""
	}
	ctx, cancel := context.WithTimeout(context.Background(), 8*time.Second)
	defer cancel()
	info, err := acc.wa().GetGroupInfo(ctx, chat)
	if err != nil || !acc.isViewerAdmin(info) {
		return false, ""
	}
	pn, lid := acc.contactForms(person)
	for _, p := range info.Participants {
		for _, form := range []types.JID{p.JID, p.PhoneNumber, p.LID} {
			if !form.IsEmpty() && (form.ToNonAD() == pn || form.ToNonAD() == lid) {
				return false, ""
			}
		}
	}
	if info.GroupName.Name != "" {
		name = info.GroupName.Name
	}
	return true, name
}

type ContactCardRequest struct {
	JID       string `json:"jid"`
	MessageID string `json:"message_id"`
	Intent    string `json:"intent"`
}

func (acc *Account) handleGetContactCard(conn net.Conn, raw json.RawMessage) {
	var data ContactCardRequest
	if json.Unmarshal(raw, &data) != nil {
		return
	}
	// Checking whether the person can be added to the group asks the server.
	go acc.sendContactCard(conn, data)
}

func (acc *Account) sendContactCard(conn net.Conn, data ContactCardRequest) {
	msg, ok := acc.loadMessageProto(data.MessageID)
	if !ok {
		acc.sendEvent(conn, "error", map[string]string{"message": "This contact card can't be opened here."})
		return
	}
	var entries []*waE2E.ContactMessage
	multi := false
	title := ""
	switch {
	case msg.GetContactMessage() != nil:
		entries = []*waE2E.ContactMessage{msg.GetContactMessage()}
		title = strings.TrimSpace(msg.GetContactMessage().GetDisplayName())
	case msg.GetContactsArrayMessage() != nil:
		entries = msg.GetContactsArrayMessage().GetContacts()
		multi = true
	default:
		acc.sendEvent(conn, "error", map[string]string{"message": "This message isn't a contact card."})
		return
	}
	var texts, names []string
	var cards []contactCardJSON
	for _, entry := range entries {
		texts = append(texts, entry.GetVcard())
		names = append(names, entry.GetDisplayName())
		var parsed []parsedVcard
		if multi {
			parsed = splitVcards(entry.GetVcard())
		} else if card := parseVcard(entry.GetVcard()); card != nil {
			parsed = []parsedVcard{card}
		}
		for _, card := range parsed {
			if len(cards) == vcardMaxPerMsg {
				break
			}
			c := acc.contactCardJSON(card)
			if c.Name == "" {
				c.Name = entry.GetDisplayName()
			}
			cards = append(cards, c)
		}
	}
	if multi {
		title = multiCardName(names)
	} else if title == "" && len(cards) > 0 {
		title = cards[0].Name
	}
	groupName := ""
	// Only the one-card bubble offers Add to group, and only when it isn't
	// showing View business or Save contact instead.
	if !multi && len(cards) == 1 && data.Intent == "view" {
		c := &cards[0]
		if !c.Business && !(c.CanSave && !c.Saved) && !c.Me {
			if chat, err := types.ParseJID(data.JID); err == nil {
				c.AddToGroup, groupName = acc.canAddToGroup(chat.ToNonAD(), *c)
			}
		}
	}
	if cards == nil {
		cards = []contactCardJSON{}
	}
	acc.sendEvent(conn, "contact_card", map[string]interface{}{
		"jid": data.JID, "message_id": data.MessageID, "intent": data.Intent, "multi": multi,
		"title": title, "group_name": groupName, "cards": cards,
		"vcard": strings.Join(texts, "\n"), "file_name": vcardFileName(title),
	})
}
