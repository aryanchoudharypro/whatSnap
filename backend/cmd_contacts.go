package main

import (
	"context"
	"encoding/json"
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
		pushName := ""
		fullName := ""
		if contactInfo, cErr := wa().Store.Contacts.GetContact(context.Background(), jid); cErr == nil && contactInfo.Found {
			pushName = contactInfo.PushName
			fullName = contactInfo.FullName
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
		// fallback in case the server refuses -1.
		input := types.SetStatusInput{Text: &data.Text, Duration: jsontime.SInt(-1)}
		err := wa().SetStatusMessage(context.Background(), input)
		if err != nil {
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
		var digits []rune
		for _, r := range data.Phone {
			if r >= '0' && r <= '9' {
				digits = append(digits, r)
			}
		}
		phone := string(digits)
		resp, err := wa().IsOnWhatsApp(context.Background(), []string{phone})
		if err != nil || len(resp) == 0 || !resp[0].IsIn {
			sendEvent(conn, "new_contact_status", map[string]interface{}{"status": "error", "error": "Not on WhatsApp or query failed"})
			return
		}
		jid := resp[0].JID
		err = wa().Store.Contacts.PutContactName(context.Background(), jid, data.Name, data.Name)
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
			if syncErr := syncContactToPhone(pnJID, lidJID, data.Name); syncErr != nil {
				log.Printf("Contact sync (app-state) failed for %s: %v", jid, syncErr)
			}
		}
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
func syncContactToPhone(pnJID, lidJID types.JID, name string) error {
	if pnJID.IsEmpty() && lidJID.IsEmpty() {
		return fmt.Errorf("no known jid to sync contact under")
	}

	newMutation := func(index types.JID, includeLidField bool) appstate.MutationInfo {
		action := &waSyncAction.ContactAction{
			FullName:                 proto.String(name),
			FirstName:                proto.String(name),
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

// buildVCard renders a minimal WhatsApp-compatible vCard 3.0 for a contact,
// matching the shape official WhatsApp clients send in a ContactMessage.
func buildVCard(name, phone, jid string) string {
	displayName := name
	if displayName == "" {
		displayName = phone
	}
	waid := phone
	if waid == "" {
		waid = strings.SplitN(jid, "@", 2)[0]
	}
	telValue := waid
	if telValue != "" {
		telValue = "+" + telValue
	}
	return fmt.Sprintf("BEGIN:VCARD\nVERSION:3.0\nFN:%s\nTEL;type=CELL;type=VOICE;waid=%s:%s\nEND:VCARD",
		vCardEscape(displayName), waid, telValue)
}

// contactDisplayInfo resolves a contact jid to (name, phone) using the local
// contacts cache, falling back to the jid's own user part as the phone when
// nothing better is known.
func contactDisplayInfo(jidStr string) (name, phone string) {
	db.QueryRow(`SELECT COALESCE(name, ''), COALESCE(phone, '') FROM contacts_cache WHERE jid = ?`, jidStr).Scan(&name, &phone)
	if phone == "" {
		phone = strings.SplitN(jidStr, "@", 2)[0]
	}
	return name, phone
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
			name, phone := contactDisplayInfo(cjid)
			display := name
			if display == "" {
				display = phone
			}
			vcards = append(vcards, &waE2E.ContactMessage{
				DisplayName: proto.String(display),
				Vcard:       proto.String(buildVCard(name, phone, cjid)),
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
			arrayDisplayName := fmt.Sprintf("%d contacts", len(vcards))
			msg = &waE2E.Message{ContactsArrayMessage: &waE2E.ContactsArrayMessage{
				DisplayName: proto.String(arrayDisplayName),
				Contacts:    vcards,
				ContextInfo: ctxInfo,
			}}
			displayText = "[Contact] " + strings.Join(names, ", ")
		}

		startSendingBuiltMessage(id, jid, msg, displayText, quotedID, quotedText, quotedSender)
		finishSendingBuiltMessage(conn, id, jid, msg, displayText)
	}()
}
