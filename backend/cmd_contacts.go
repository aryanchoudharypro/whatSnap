package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"net"
	"strings"
	"time"

	"go.mau.fi/util/jsontime"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/proto/waSyncAction"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
)

func (acc *Account) handleGetContacts(conn net.Conn) {
	go func() {
		rows, err := acc.db.Query(`SELECT jid, name, phone, is_business, about, avatar_path FROM contacts_cache WHERE name != '' OR phone != '' ORDER BY name COLLATE NOCASE ASC`)

		if err != nil {
			acc.sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}
		defer rows.Close()
		saved := acc.savedContactJIDs()
		contacts := []map[string]interface{}{}
		for rows.Next() {
			var jid, name, phone, about, avatarPath string
			var isBusiness int
			if err := rows.Scan(&jid, &name, &phone, &isBusiness, &about, &avatarPath); err == nil {
				if saved != nil && !saved[jid] {
					continue
				}
				contacts = append(contacts, map[string]interface{}{
					"jid": jid, "name": name, "phone": phone, "saved": saved != nil,
					"isBusiness": isBusiness == 1, "about": about,
					"avatarPath": avatarPath,
				})
			}
		}
		acc.sendEvent(conn, "contacts", contacts)
	}()
}

// savedContactJIDs is everyone in the phone's address book: contacts the
// phone synced with a name (ContactAction), under their phone number JID.
// WA Web's contact lists (new chat, new call, add to group) show only these
// (isAddressBookContact); people only known by their push name, or whom the
// phone has since removed, stay out. Nil when the store has no names yet;
// then everyone is listed, marked unsaved.
func (acc *Account) savedContactJIDs() map[string]bool {
	if acc.wa() == nil || acc.wa().Store == nil || acc.wa().Store.Contacts == nil {
		return nil
	}
	all, err := acc.wa().Store.Contacts.GetAllContacts(context.Background())
	if err != nil {
		return nil
	}
	saved := make(map[string]bool, len(all))
	for jid, contact := range all {
		if contact.FullName == "" && contact.FirstName == "" {
			continue
		}
		saved[jid.ToNonAD().String()] = true
		saved[acc.preferPN(jid.ToNonAD()).String()] = true
	}
	if len(saved) == 0 {
		return nil
	}
	return saved
}

func (acc *Account) handleGetAvatar(conn net.Conn, raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		info, err := acc.wa().GetProfilePictureInfo(context.Background(), jid, &whatsmeow.GetProfilePictureParams{})
		if err != nil || info == nil {
			acc.sendEvent(conn, "avatar", map[string]interface{}{"jid": data.JID, "path": ""})
			return
		}
		path, _, dErr := acc.downloadURLToFile("avatars", data.JID, info.URL)
		if dErr != nil {
			acc.sendEvent(conn, "avatar", map[string]interface{}{"jid": data.JID, "path": ""})
			return
		}
		acc.updateContactAvatarCache(data.JID, path, info.ID)
		acc.sendEvent(conn, "avatar", map[string]interface{}{"jid": data.JID, "path": path, "pictureId": info.ID})
	}()
}

func (acc *Account) handleCheckContacts(conn net.Conn, raw json.RawMessage) {
	var data CheckContactsData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		var cleaned []string
		for _, p := range data.Phones {
			var digits []rune
			for _, r := range p {
				if r >= '0' && r <= '9' {
					digits = append(digits, r)
				}
			}
			if len(digits) > 0 {
				cleaned = append(cleaned, string(digits))
			}
		}
		if len(cleaned) == 0 {
			acc.sendEvent(conn, "contacts_checked", map[string]interface{}{"results": []interface{}{}})
			return
		}
		resp, err := acc.wa().IsOnWhatsApp(context.Background(), cleaned)
		if err != nil {
			acc.sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}
		results := []map[string]interface{}{}
		for _, r := range resp {
			results = append(results, map[string]interface{}{
				"query": r.Query, "isOnWhatsApp": r.IsIn, "jid": r.JID.String(),
			})
		}
		acc.sendEvent(conn, "contacts_checked", map[string]interface{}{"results": results})
	}()
}

func (acc *Account) handleGetUserInfo(conn net.Conn, raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		info, err := acc.wa().GetUserInfo(context.Background(), []types.JID{jid})
		if err != nil {
			acc.sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}
		userInfo, ok := info[jid]
		if !ok {
			acc.sendEvent(conn, "error", map[string]string{"message": "user info not found"})
			return
		}
		pnJID, lidJID := acc.contactForms(jid)
		var pushName, fullName, firstName string
		for _, form := range []types.JID{jid, pnJID, lidJID} {
			if form.IsEmpty() {
				continue
			}
			if c, cErr := acc.wa().Store.Contacts.GetContact(context.Background(), form); cErr == nil && c.Found {
				if pushName == "" {
					pushName = c.PushName
				}
				if fullName == "" && firstName == "" {
					fullName, firstName = c.FullName, c.FirstName
				}
			}
		}
		status := userInfo.Status
		if fresh, sErr := acc.wa().GetTextStatus(context.Background(), jid); sErr == nil && fresh != nil {
			status = fresh.Text
		}
		go acc.updateContactAboutCache(data.JID, status)
		acc.sendEvent(conn, "user_info", map[string]interface{}{
			"jid": data.JID, "status": status, "pictureId": userInfo.PictureID,
			"deviceCount": len(userInfo.Devices),
			"pushName":    pushName,
			"fullName":    fullName,
			"firstName":   firstName,
			"saved":       fullName != "" || firstName != "",
			"phone":       pnJID.User,
		})
	}()
}

func (acc *Account) handleGetBusinessProfile(conn net.Conn, raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		profile, err := acc.wa().GetBusinessProfile(context.Background(), jid)
		if err != nil {
			acc.sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}
		acc.sendEvent(conn, "business_profile", profile)
	}()
}

func (acc *Account) handleGetBlocklist(conn net.Conn) {
	go func() {
		acc.sendBlocklist(conn)
		acc.refreshBlocklist()
	}()
}

func (acc *Account) handleSetBlocked(raw json.RawMessage, block bool) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		result := map[string]interface{}{"jid": data.JID, "name": acc.resolveDisplayName(jid), "blocked": block}
		if jid.Server != types.DefaultUserServer && jid.Server != types.HiddenUserServer {
			result["error"] = "Only contacts can be blocked."
		} else if err := acc.setBlocked(jid, block); errors.Is(err, whatsmeow.ErrRecipientNotOnWhatsApp) {
			result["error"] = notOnWhatsAppText
		} else if err != nil {
			result["error"] = err.Error()
		}
		acc.broadcastEvent("block_result", result)
	}()
}

func (acc *Account) handleSetAbout(raw json.RawMessage) {
	var data SetAboutData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		// ephemeral_duration_sec -1 is an About that never expires
		// (WAWebTextStatusUtils.TEXT_STATUS_DURATION_INFINITE). 0 is not: the
		// server takes it as "default" and stores 24h, which is what made
		// every About set from here vanish after a day. 24h stays only as a
		// fallback in case the server refuses -1. Anything else is one of
		// the times WhatsApp offers (text_status_ttl_seconds_allowlist).
		duration := -1
		if data.Duration != nil && *data.Duration > 0 {
			duration = *data.Duration
		}
		input := types.SetStatusInput{Text: &data.Text, Duration: jsontime.SInt(duration)}
		err := acc.wa().SetStatusMessage(context.Background(), input)
		if err != nil && duration == -1 {
			log.Printf("Setting a permanent About failed (%v), retrying with a 24h expiry", err)
			input.Duration = jsontime.S(24 * time.Hour)
			err = acc.wa().SetStatusMessage(context.Background(), input)
		}
		if err != nil {
			acc.broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		if acc.wa().Store.ID != nil {
			if st, gerr := acc.wa().GetTextStatus(context.Background(), acc.wa().Store.ID.ToNonAD()); gerr == nil && st != nil {
				log.Printf("About updated, server reports expiry %ds (-1 = never expires)", st.EphemeralDurationSec)
			}
		}
	}()
}

func (acc *Account) handleNewContact(conn net.Conn, raw json.RawMessage) {
	var data NewContactCommandData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		var candidates []string
		for _, c := range phoneCandidates(data.Phone, acc.ownCallingCode()) {
			candidates = append(candidates, "+"+c)
		}
		if len(candidates) == 0 {
			acc.sendEvent(conn, "new_contact_status", map[string]interface{}{"status": "error", "error": "Not on WhatsApp or query failed"})
			return
		}
		results, err := acc.wa().IsOnWhatsApp(context.Background(), candidates)
		var found *types.IsOnWhatsAppResponse
		for _, candidate := range candidates {
			for i := range results {
				if results[i].IsIn && results[i].Query == candidate {
					found = &results[i]
					break
				}
			}
			if found != nil {
				break
			}
		}
		if err != nil || found == nil {
			acc.sendEvent(conn, "new_contact_status", map[string]interface{}{"status": "error", "error": "Not on WhatsApp or query failed"})
			return
		}
		resp := []types.IsOnWhatsAppResponse{*found}
		phone := strings.TrimPrefix(resp[0].Query, "+")
		jid := resp[0].JID
		firstName := data.FirstName
		if firstName == "" {
			firstName = data.Name
		}
		err = acc.wa().Store.Contacts.PutContactName(context.Background(), jid, firstName, data.Name)
		if err != nil {
			acc.sendEvent(conn, "new_contact_status", map[string]interface{}{"status": "error", "error": err.Error()})
			return
		}
		acc.sendEvent(conn, "new_contact_status", map[string]interface{}{"status": "success", "jid": jid.String(), "name": data.Name})
		acc.upsertContactCacheRow(jid.String(), data.Name, phone, false)

		pnJID, lidJID := resp[0].PhoneNumber, types.JID{}
		if jid.Server == types.HiddenUserServer {
			lidJID = jid
		} else {
			pnJID = jid
			if resolved, lerr := acc.wa().Store.LIDs.GetLIDForPN(context.Background(), jid); lerr == nil {
				lidJID = resolved
			}
		}

		if !pnJID.IsEmpty() {
			acc.db.Exec(`UPDATE chats SET name = ? WHERE jid = ?`, data.Name, pnJID.String())
		}
		if !lidJID.IsEmpty() {
			acc.db.Exec(`UPDATE chats SET name = ? WHERE jid = ?`, data.Name, lidJID.String())
		}

		acc.populateChatsFromStore()
		acc.broadcastChats()

		if data.Sync {
			if syncErr := acc.syncContactToPhone(pnJID, lidJID, data.Name, firstName); syncErr != nil {
				log.Printf("Contact sync (app-state) failed for %s: %v", jid, syncErr)
			}
		}
	}()
}

func (acc *Account) contactForms(jid types.JID) (pn, lid types.JID) {
	jid = jid.ToNonAD()
	ctx := context.Background()
	if jid.Server == types.HiddenUserServer {
		lid = jid
		if resolved, err := acc.wa().Store.LIDs.GetPNForLID(ctx, jid); err == nil {
			pn = resolved
		}
	} else {
		pn = jid
		if resolved, err := acc.wa().Store.LIDs.GetLIDForPN(ctx, jid); err == nil {
			lid = resolved
		}
	}
	return pn, lid
}

func (acc *Account) handleDeleteContact(conn net.Conn, raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		pn, lid := acc.contactForms(jid)
		var forms []types.JID
		for _, form := range []types.JID{pn, lid} {
			if !form.IsEmpty() {
				forms = append(forms, form)
			}
		}
		// Nothing to send means it was only ever saved here, never synced.
		if err := acc.wa().SendAppState(context.Background(), appstate.BuildRemoveContact(forms...)); err != nil && !errors.Is(err, appstate.ErrNothingToSend) {
			acc.sendEvent(conn, "error", map[string]string{"message": "Couldn't delete the contact: " + err.Error()})
			return
		}
		for _, form := range forms {
			acc.wa().Store.Contacts.PutContactName(context.Background(), form, "", "")
			acc.deleteContactCacheRow(form.String())
			acc.db.Exec(`UPDATE chats SET name = '' WHERE jid = ?`, form.String())
		}
		acc.invalidateDisplayNames()
		acc.populateChatsFromStore()
		acc.broadcastChats()
		acc.sendEvent(conn, "contact_deleted", map[string]string{"jid": data.JID})
	}()
}

// syncContactToPhone sends the "contact" app-state patch(es) WhatsApp Web
// itself sends when adding a contact: collection critical_unblock_low, with
// SaveOnPrimaryAddressbook set so the linked phone actually saves it to its
// address book rather than whatSnap's cache staying the only place that knows
// about it.
//
// Captured from a real WhatsApp Web session's own patch (via temporary debug
// logging on the receiving events.Contact handler): a modern (LID-migrated)
// account writes the contact under BOTH the phone-number JID index (with
// LidJID pointing at the contact's LID) AND a second, separate mutation
// indexed by the LID itself (LidJID left empty there, since the index is
// already the LID). Sending only one of the two is silently accepted by the
// server (no error, version still advances) but never actually applied to
// the phone's contact list.
//
// pnJID and lidJID are the two forms of the same contact as returned by
// IsOnWhatsApp (queried with addressing_mode=lid, its primary .JID is
// typically already the LID form, not the phone number - .PhoneNumber holds
// the actual PN JID). Either may be the zero JID if unknown.
func (acc *Account) syncContactToPhone(pnJID, lidJID types.JID, name, firstName string) error {
	if pnJID.IsEmpty() && lidJID.IsEmpty() {
		return fmt.Errorf("no known jid to sync contact under")
	}

	newMutation := func(index types.JID, includeLidField bool) appstate.MutationInfo {
		action := &waSyncAction.ContactAction{
			FullName:                 proto.String(name),
			FirstName:                proto.String(firstName),
			SaveOnPrimaryAddressbook: proto.Bool(true),
		}
		if includeLidField && !lidJID.IsEmpty() {
			action.LidJID = proto.String(lidJID.String())
		}
		return appstate.MutationInfo{
			Index:   []string{appstate.IndexContact, index.String()},
			Version: 2,
			Value:   &waSyncAction.SyncActionValue{ContactAction: action},
		}
	}

	var mutations []appstate.MutationInfo
	if !pnJID.IsEmpty() {
		mutations = append(mutations, newMutation(pnJID, true))
	}
	if !lidJID.IsEmpty() {
		mutations = append(mutations, newMutation(lidJID, false))
	}

	patch := appstate.PatchInfo{
		Type:      appstate.WAPatchCriticalUnblockLow,
		Mutations: mutations,
	}
	return acc.wa().SendAppState(context.Background(), patch)
}

func vCardEscape(s string) string {
	r := strings.NewReplacer(`\`, `\\`, `,`, `\,`, `;`, `\;`, "\n", `\n`)
	return r.Replace(s)
}

func callingCodeOf(digits string) string {
	for n := 3; n >= 1; n-- {
		if len(digits) > n {
			if _, ok := callingCodeRegions[digits[:n]]; ok {
				return digits[:n]
			}
		}
	}
	return ""
}

func (acc *Account) ownCallingCode() string {
	if acc.wa().Store.ID == nil {
		return ""
	}
	return callingCodeOf(acc.wa().Store.ID.User)
}

func phoneCandidates(typed, ownCode string) []string {
	typed = strings.TrimSpace(typed)
	var digits strings.Builder
	for _, r := range typed {
		if r >= '0' && r <= '9' {
			digits.WriteRune(r)
		}
	}
	d := digits.String()
	if d == "" {
		return nil
	}
	switch {
	case strings.HasPrefix(typed, "+"):
		return []string{d}
	case strings.HasPrefix(d, "00"):
		return []string{d[2:]}
	}
	var out []string
	if ownCode != "" {
		local := strings.TrimLeft(d, "0")
		if !strings.HasPrefix(d, ownCode) || len(d) <= len(ownCode)+7 {
			out = append(out, ownCode+local)
		}
	}
	if len(out) == 0 || out[0] != d {
		out = append(out, d)
	}
	return out
}

func formatPhone(digits string) string {
	if code := callingCodeOf(digits); code != "" {
		return "+" + code + " " + digits[len(code):]
	}
	return "+" + digits
}

type sharedContact struct {
	name       string
	phone      string
	lid        string
	isBusiness bool
}

func buildVCard(c sharedContact) string {
	display := c.name
	if display == "" {
		display = formatPhone(c.phone)
	}
	first, last := display, ""
	if i := strings.IndexByte(display, ' '); i > 0 {
		first, last = display[:i], strings.TrimSpace(display[i+1:])
	}
	lines := []string{"BEGIN:VCARD", "VERSION:3.0", "N:" + vCardEscape(last) + ";" + vCardEscape(first) + ";;;", "FN:" + vCardEscape(display)}
	switch {
	case c.phone != "":
		lines = append(lines, "TEL;type=CELL;waid="+c.phone+":"+formatPhone(c.phone))
	case c.lid != "":
		lines = append(lines, "X-WA-LID:"+c.lid)
	}
	if c.isBusiness {
		lines = append(lines, "X-WA-BIZ-NAME:"+vCardEscape(display))
	}
	return strings.Join(append(lines, "END:VCARD"), "\n")
}

func (acc *Account) contactToShare(jidStr string) sharedContact {
	var c sharedContact
	jid, err := types.ParseJID(jidStr)
	if err != nil {
		return c
	}
	pn, lid := jid, types.JID{}
	if jid.Server == types.HiddenUserServer {
		lid, pn = jid, types.JID{}
		if mapped, err := acc.wa().Store.LIDs.GetPNForLID(context.Background(), jid); err == nil {
			pn = mapped
		}
	}
	keys := []string{jid.ToNonAD().String()}
	if !pn.IsEmpty() && pn.ToNonAD() != jid.ToNonAD() {
		keys = append(keys, pn.ToNonAD().String())
	}
	for _, key := range keys {
		var name, phone string
		var business bool
		if acc.db.QueryRow(`SELECT COALESCE(name, ''), COALESCE(phone, ''), COALESCE(is_business, 0) FROM contacts_cache WHERE jid = ?`, key).
			Scan(&name, &phone, &business) == nil {
			if c.name == "" {
				c.name = name
			}
			if c.phone == "" {
				c.phone = strings.TrimPrefix(phone, "+")
			}
			c.isBusiness = c.isBusiness || business
		}
	}
	if c.phone == "" && !pn.IsEmpty() {
		c.phone = pn.User
	}
	if c.phone == "" && !lid.IsEmpty() {
		c.lid = lid.User
	}
	return c
}

func (acc *Account) handleSendContacts(conn net.Conn, raw json.RawMessage) {
	var data SendContactsData
	if err := json.Unmarshal(raw, &data); err != nil || len(data.ContactJIDs) == 0 {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		acc.sendEvent(conn, "error", map[string]string{"message": "invalid jid"})
		return
	}
	go func() {
		id := acc.wa().GenerateMessageID()

		vcards := make([]*waE2E.ContactMessage, 0, len(data.ContactJIDs))
		names := make([]string, 0, len(data.ContactJIDs))
		for _, cjid := range data.ContactJIDs {
			c := acc.contactToShare(cjid)
			display := c.name
			if display == "" {
				display = formatPhone(c.phone)
			}
			vcards = append(vcards, &waE2E.ContactMessage{
				DisplayName: proto.String(display),
				Vcard:       proto.String(buildVCard(c)),
			})
			names = append(names, display)
		}

		ctxInfo, quotedID, quotedText, quotedSender := acc.buildAttachmentContext(data.ReplyToID, data.ReplySender)

		var msg *waE2E.Message
		var displayText string
		if len(vcards) == 1 {
			vcards[0].ContextInfo = ctxInfo
			msg = &waE2E.Message{ContactMessage: vcards[0]}
			displayText = "[Contact] " + names[0]
		} else {
			msg = &waE2E.Message{ContactsArrayMessage: &waE2E.ContactsArrayMessage{
				Contacts:    vcards,
				ContextInfo: ctxInfo,
			}}
			displayText = "[Contact] " + strings.Join(names, ", ")
		}

		acc.startSendingBuiltMessage(id, jid, msg, displayText, quotedID, quotedText, quotedSender)
		acc.finishSendingBuiltMessage(conn, id, jid, msg, displayText)
	}()
}
