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

func handleGetContacts(conn net.Conn) {
	go func() {
		// Straight from the cache. contacts_cache is already kept current by
		// contact/push-name events and the refreshes after connect and history
		// sync; rebuilding it here (every contact, plus a network fetch of all
		// our groups) on every contact-picker open was pure overhead.
		rows, err := db.Query(`SELECT jid, name, phone, is_business, about, avatar_path FROM contacts_cache WHERE name != '' OR phone != '' ORDER BY name COLLATE NOCASE ASC`)

		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}
		defer rows.Close()
		contacts := []map[string]interface{}{}
		for rows.Next() {
			var jid, name, phone, about, avatarPath string
			var isBusiness int
			if err := rows.Scan(&jid, &name, &phone, &isBusiness, &about, &avatarPath); err == nil {
				contacts = append(contacts, map[string]interface{}{
					"jid": jid, "name": name, "phone": phone,
					"isBusiness": isBusiness == 1, "about": about,
					"avatarPath": avatarPath,
				})
			}
		}
		sendEvent(conn, "contacts", contacts)
	}()
}

func handleGetAvatar(conn net.Conn, raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		info, err := wa().GetProfilePictureInfo(context.Background(), jid, &whatsmeow.GetProfilePictureParams{})
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}
		if info == nil {
			sendEvent(conn, "avatar", map[string]interface{}{"jid": data.JID, "path": ""})
			return
		}
		path, _, dErr := downloadURLToFile("avatars", data.JID, info.URL)
		if dErr != nil {
			sendEvent(conn, "error", map[string]string{"message": dErr.Error()})
			return
		}
		updateContactAvatarCache(data.JID, path, info.ID)
		sendEvent(conn, "avatar", map[string]interface{}{"jid": data.JID, "path": path, "pictureId": info.ID})
	}()
}

func handleCheckContacts(conn net.Conn, raw json.RawMessage) {
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
			sendEvent(conn, "contacts_checked", map[string]interface{}{"results": []interface{}{}})
			return
		}
		resp, err := wa().IsOnWhatsApp(context.Background(), cleaned)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}
		results := []map[string]interface{}{}
		for _, r := range resp {
			results = append(results, map[string]interface{}{
				"query": r.Query, "isOnWhatsApp": r.IsIn, "jid": r.JID.String(),
			})
		}
		sendEvent(conn, "contacts_checked", map[string]interface{}{"results": results})
	}()
}

func handleGetUserInfo(conn net.Conn, raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		info, err := wa().GetUserInfo(context.Background(), []types.JID{jid})
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}
		userInfo, ok := info[jid]
		if !ok {
			sendEvent(conn, "error", map[string]string{"message": "user info not found"})
			return
		}
		pnJID, lidJID := contactForms(jid)
		var pushName, fullName, firstName string
		for _, form := range []types.JID{jid, pnJID, lidJID} {
			if form.IsEmpty() {
				continue
			}
			if c, cErr := wa().Store.Contacts.GetContact(context.Background(), form); cErr == nil && c.Found {
				if pushName == "" {
					pushName = c.PushName
				}
				if fullName == "" && firstName == "" {
					fullName, firstName = c.FullName, c.FirstName
				}
			}
		}
		// userInfo.Status comes from the legacy usync <status/> node, which
		// the server stops keeping in sync once a contact updates their
		// About via the newer mex mutation. GetTextStatus is the read-side
		// counterpart of that mutation and gives a fresh value; fall back to
		// the usync value if the fresh fetch fails.
		status := userInfo.Status
		if fresh, sErr := wa().GetTextStatus(context.Background(), jid); sErr == nil && fresh != nil {
			status = fresh.Text
		}
		go updateContactAboutCache(data.JID, status)
		sendEvent(conn, "user_info", map[string]interface{}{
			"jid": data.JID, "status": status, "pictureId": userInfo.PictureID,
			"deviceCount": len(userInfo.Devices),
			"pushName":    pushName,
			"fullName":    fullName,
			"firstName":   firstName,
			// In the address book, so the info page offers Edit and Delete
			// rather than Add.
			"saved": fullName != "" || firstName != "",
			"phone": pnJID.User,
		})
	}()
}

func handleGetBusinessProfile(conn net.Conn, raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		profile, err := wa().GetBusinessProfile(context.Background(), jid)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}
		sendEvent(conn, "business_profile", profile)
	}()
}

func handleGetBlocklist(conn net.Conn) {
	go func() {
		// What's stored answers at once (and is all there is offline); the
		// server's copy follows as a broadcast.
		sendEvent(conn, "blocklist", blocklistPayload())
		refreshBlocklist()
	}()
}

// handleSetBlocked blocks or unblocks a contact and reports how it went.
func handleSetBlocked(raw json.RawMessage, block bool) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		result := map[string]interface{}{"jid": data.JID, "name": resolveDisplayName(jid), "blocked": block}
		if jid.Server != types.DefaultUserServer && jid.Server != types.HiddenUserServer {
			result["error"] = "Only contacts can be blocked."
		} else if err := setBlocked(jid, block); err != nil {
			result["error"] = err.Error()
		}
		broadcastEvent("block_result", result)
	}()
}

func handleSetAbout(raw json.RawMessage) {
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
		err := wa().SetStatusMessage(context.Background(), input)
		if err != nil && duration == -1 {
			log.Printf("Setting a permanent About failed (%v), retrying with a 24h expiry", err)
			input.Duration = jsontime.S(24 * time.Hour)
			err = wa().SetStatusMessage(context.Background(), input)
		}
		if err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		if wa().Store.ID != nil {
			if st, gerr := wa().GetTextStatus(context.Background(), wa().Store.ID.ToNonAD()); gerr == nil && st != nil {
				log.Printf("About updated, server reports expiry %ds (-1 = never expires)", st.EphemeralDurationSec)
			}
		}
	}()
}

// handleNewContact is the original add-a-contact flow, preserved, with the
// contacts_cache kept in sync and the previous "chats" event (which was
// actually sending a raw, differently-shaped contacts map mislabeled as
// chats) replaced with a proper chat-list refresh.
func handleNewContact(conn net.Conn, raw json.RawMessage) {
	var data NewContactCommandData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		var candidates []string
		for _, c := range phoneCandidates(data.Phone, ownCallingCode()) {
			candidates = append(candidates, "+"+c)
		}
		if len(candidates) == 0 {
			sendEvent(conn, "new_contact_status", map[string]interface{}{"status": "error", "error": "Not on WhatsApp or query failed"})
			return
		}
		results, err := wa().IsOnWhatsApp(context.Background(), candidates)
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
			sendEvent(conn, "new_contact_status", map[string]interface{}{"status": "error", "error": "Not on WhatsApp or query failed"})
			return
		}
		resp := []types.IsOnWhatsAppResponse{*found}
		phone := strings.TrimPrefix(resp[0].Query, "+")
		jid := resp[0].JID
		firstName := data.FirstName
		if firstName == "" {
			firstName = data.Name
		}
		err = wa().Store.Contacts.PutContactName(context.Background(), jid, firstName, data.Name)
		if err != nil {
			sendEvent(conn, "new_contact_status", map[string]interface{}{"status": "error", "error": err.Error()})
			return
		}
		sendEvent(conn, "new_contact_status", map[string]interface{}{"status": "success", "jid": jid.String(), "name": data.Name})
		upsertContactCacheRow(jid.String(), data.Name, phone, false)

		// IsOnWhatsApp is queried with addressing_mode=lid, so resp[0].JID is
		// typically already the contact's LID rather than their phone-number
		// JID (that's resp[0].PhoneNumber instead) - resolve both forms since
		// an existing chat with this person may be keyed by either one.
		pnJID, lidJID := resp[0].PhoneNumber, types.JID{}
		if jid.Server == types.HiddenUserServer {
			lidJID = jid
		} else {
			pnJID = jid
			if resolved, lerr := wa().Store.LIDs.GetLIDForPN(context.Background(), jid); lerr == nil {
				lidJID = resolved
			}
		}

		// Rename any existing chat with this person immediately, rather than
		// waiting on populateChatsFromStore's full contact rescan below.
		if !pnJID.IsEmpty() {
			db.Exec(`UPDATE chats SET name = ? WHERE jid = ?`, data.Name, pnJID.String())
		}
		if !lidJID.IsEmpty() {
			db.Exec(`UPDATE chats SET name = ? WHERE jid = ?`, data.Name, lidJID.String())
		}

		populateChatsFromStore()
		broadcastChats()

		// Push the contact to WhatsApp's own contact-sync (the same app-state
		// patch official WhatsApp Web sends when you add a contact), so it
		// actually shows up in the linked phone's WhatsApp/address book
		// instead of only being cached locally by whaNTV. Best-effort: the
		// contact was already created successfully above either way, so a
		// failure here is only logged, not surfaced as an error to the user.
		if data.Sync {
			if syncErr := syncContactToPhone(pnJID, lidJID, data.Name, firstName); syncErr != nil {
				log.Printf("Contact sync (app-state) failed for %s: %v", jid, syncErr)
			}
		}
	}()
}

// contactForms are the phone number and LID forms of a person, either of
// which may be empty when unknown.
func contactForms(jid types.JID) (pn, lid types.JID) {
	jid = jid.ToNonAD()
	ctx := context.Background()
	if jid.Server == types.HiddenUserServer {
		lid = jid
		if resolved, err := wa().Store.LIDs.GetPNForLID(ctx, jid); err == nil {
			pn = resolved
		}
	} else {
		pn = jid
		if resolved, err := wa().Store.LIDs.GetLIDForPN(ctx, jid); err == nil {
			lid = resolved
		}
	}
	return pn, lid
}

// handleDeleteContact deletes a saved contact, as WA Web's "Delete contact"
// does: a REMOVE of the contact's app-state record, which the phone takes out
// of its address book too. The chat stays and goes back to showing the
// number and WhatsApp name.
func handleDeleteContact(conn net.Conn, raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		pn, lid := contactForms(jid)
		var forms []types.JID
		for _, form := range []types.JID{pn, lid} {
			if !form.IsEmpty() {
				forms = append(forms, form)
			}
		}
		// Nothing to send means it was only ever saved here, never synced.
		if err := wa().SendAppState(context.Background(), appstate.BuildRemoveContact(forms...)); err != nil && !errors.Is(err, appstate.ErrNothingToSend) {
			sendEvent(conn, "error", map[string]string{"message": "Couldn't delete the contact: " + err.Error()})
			return
		}
		for _, form := range forms {
			wa().Store.Contacts.PutContactName(context.Background(), form, "", "")
			deleteContactCacheRow(form.String())
			db.Exec(`UPDATE chats SET name = '' WHERE jid = ?`, form.String())
		}
		invalidateDisplayNames()
		populateChatsFromStore()
		broadcastChats()
		sendEvent(conn, "contact_deleted", map[string]string{"jid": data.JID})
	}()
}

// syncContactToPhone sends the "contact" app-state patch(es) WhatsApp Web
// itself sends when adding a contact: collection critical_unblock_low, with
// SaveOnPrimaryAddressbook set so the linked phone actually saves it to its
// address book rather than whaNTV's cache staying the only place that knows
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
func syncContactToPhone(pnJID, lidJID types.JID, name, firstName string) error {
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
	return wa().SendAppState(context.Background(), patch)
}

// vCardEscape escapes the characters vCard 3.0 (RFC 2426) requires escaped
// inside a property value: backslash, comma, semicolon and newline.
func vCardEscape(s string) string {
	r := strings.NewReplacer(`\`, `\\`, `,`, `\,`, `;`, `\;`, "\n", `\n`)
	return r.Replace(s)
}

// callingCodeOf is the calling code a number in international digits starts
// with ("44" for "447700900123"), or "" when none matches.
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

// ownCallingCode is the calling code of our own number, which a number typed
// without one is taken to be from.
func ownCallingCode() string {
	if wa().Store.ID == nil {
		return ""
	}
	return callingCodeOf(wa().Store.ID.User)
}

// phoneCandidates are the ways a typed number can be read, most likely first,
// as WhatsApp's own add-contact form reads it with the country picker on your
// own country. A + or 00 prefix means the number is already international;
// otherwise it's your country's number (its national trunk 0 dropped) or, as
// before, the digits as typed. WhatsApp itself says which one is registered.
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

// formatPhone shows international digits as a contact card does: the
// calling code apart from the rest ("+44 7700900123").
func formatPhone(digits string) string {
	if code := callingCodeOf(digits); code != "" {
		return "+" + code + " " + digits[len(code):]
	}
	return "+" + digits
}

// sharedContact is what a shared contact card says about someone.
type sharedContact struct {
	name       string
	phone      string // international digits, "" when their number is hidden from us
	lid        string // their LID's user part, used only when there's no number
	isBusiness bool
}

// buildVCard renders a contact card the way WhatsApp Web does
// (WAWebFrontendVcardUtils vcardFromContactModel): N and FN, the number as a
// waid TEL, or the LID when the number isn't known, and the business name
// for a business.
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

// contactToShare resolves a contact's name, number and kind from the
// local contacts cache. A contact known by their LID gets their number
// from the LID mapping, as WhatsApp Web does: the LID itself is no phone
// number.
func contactToShare(jidStr string) sharedContact {
	var c sharedContact
	jid, err := types.ParseJID(jidStr)
	if err != nil {
		return c
	}
	pn, lid := jid, types.JID{}
	if jid.Server == types.HiddenUserServer {
		lid, pn = jid, types.JID{}
		if mapped, err := wa().Store.LIDs.GetPNForLID(context.Background(), jid); err == nil {
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
		if db.QueryRow(`SELECT COALESCE(name, ''), COALESCE(phone, ''), COALESCE(is_business, 0) FROM contacts_cache WHERE jid = ?`, key).
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

func handleSendContacts(conn net.Conn, raw json.RawMessage) {
	var data SendContactsData
	if err := json.Unmarshal(raw, &data); err != nil || len(data.ContactJIDs) == 0 {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		sendEvent(conn, "error", map[string]string{"message": "invalid jid"})
		return
	}
	go func() {
		id := wa().GenerateMessageID()

		vcards := make([]*waE2E.ContactMessage, 0, len(data.ContactJIDs))
		names := make([]string, 0, len(data.ContactJIDs))
		for _, cjid := range data.ContactJIDs {
			c := contactToShare(cjid)
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

		ctxInfo, quotedID, quotedText, quotedSender := buildAttachmentContext(data.ReplyToID, data.ReplySender)

		var msg *waE2E.Message
		var displayText string
		if len(vcards) == 1 {
			vcards[0].ContextInfo = ctxInfo
			msg = &waE2E.Message{ContactMessage: vcards[0]}
			displayText = "[Contact] " + names[0]
		} else {
			// WhatsApp Web sends only the cards (WAWebGenerateMultiVcardMessageProto);
			// receivers title them themselves.
			msg = &waE2E.Message{ContactsArrayMessage: &waE2E.ContactsArrayMessage{
				Contacts:    vcards,
				ContextInfo: ctxInfo,
			}}
			displayText = "[Contact] " + strings.Join(names, ", ")
		}

		startSendingBuiltMessage(id, jid, msg, displayText, quotedID, quotedText, quotedSender)
		finishSendingBuiltMessage(conn, id, jid, msg, displayText)
	}()
}
