package main

import (
	"context"
	"encoding/json"

	"github.com/gorilla/websocket"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
)

func handleGetContacts(conn *websocket.Conn) {
	go func() {
		populateChatsFromStore()
		rows, err := db.Query(`SELECT jid, name, phone, is_business, about, avatar_path, is_blocked FROM contacts_cache WHERE name != '' OR phone != '' ORDER BY name COLLATE NOCASE ASC`)

		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}
		defer rows.Close()
		contacts := []map[string]interface{}{}
		for rows.Next() {
			var jid, name, phone, about, avatarPath string
			var isBusiness, isBlocked int
			if err := rows.Scan(&jid, &name, &phone, &isBusiness, &about, &avatarPath, &isBlocked); err == nil {
				contacts = append(contacts, map[string]interface{}{
					"jid": jid, "name": name, "phone": phone,
					"isBusiness": isBusiness == 1, "about": about,
					"avatarPath": avatarPath, "isBlocked": isBlocked == 1,
				})
			}
		}
		sendEvent(conn, "contacts", contacts)
	}()
}

func handleGetAvatar(conn *websocket.Conn, raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		info, err := client.GetProfilePictureInfo(context.Background(), jid, &whatsmeow.GetProfilePictureParams{})
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

func handleCheckContacts(conn *websocket.Conn, raw json.RawMessage) {
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
		resp, err := client.IsOnWhatsApp(context.Background(), cleaned)
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

func handleGetUserInfo(conn *websocket.Conn, raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		info, err := client.GetUserInfo(context.Background(), []types.JID{jid})
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
		if contactInfo, cErr := client.Store.Contacts.GetContact(context.Background(), jid); cErr == nil && contactInfo.Found {
			pushName = contactInfo.PushName
			fullName = contactInfo.FullName
		}
		go updateContactAboutCache(data.JID, userInfo.Status)
		sendEvent(conn, "user_info", map[string]interface{}{
			"jid": data.JID, "status": userInfo.Status, "pictureId": userInfo.PictureID,
			"deviceCount": len(userInfo.Devices),
			"pushName": pushName,
			"fullName": fullName,
		})
	}()
}

func handleGetBusinessProfile(conn *websocket.Conn, raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		profile, err := client.GetBusinessProfile(context.Background(), jid)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}
		sendEvent(conn, "business_profile", profile)
	}()
}

func handleGetBlocklist(conn *websocket.Conn) {
	go func() {
		bl, err := client.GetBlocklist(context.Background())
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}
		jids := make([]string, len(bl.JIDs))
		for i, j := range bl.JIDs {
			jids[i] = j.String()
		}
		sendEvent(conn, "blocklist", map[string]interface{}{"jids": jids})
	}()
}

func handleBlockContact(raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		if _, err := client.UpdateBlocklist(context.Background(), jid, events.BlocklistChangeActionBlock); err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		setContactBlocked(data.JID, true)
		broadcastEvent("blocklist_updated", nil)
	}()
}

func handleUnblockContact(raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		if _, err := client.UpdateBlocklist(context.Background(), jid, events.BlocklistChangeActionUnblock); err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		setContactBlocked(data.JID, false)
		broadcastEvent("blocklist_updated", nil)
	}()
}

func handleSetAbout(raw json.RawMessage) {
	var data SetAboutData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		if err := client.SetStatusMessage(context.Background(), types.SetStatusInput{Text: &data.Text}); err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
		}
	}()
}

// handleNewContact is the original add-a-contact flow, preserved, with the
// contacts_cache kept in sync and the previous "chats" event (which was
// actually sending a raw, differently-shaped contacts map mislabeled as
// chats) replaced with a proper chat-list refresh.
func handleNewContact(conn *websocket.Conn, raw json.RawMessage) {
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
		resp, err := client.IsOnWhatsApp(context.Background(), []string{phone})
		if err != nil || len(resp) == 0 || !resp[0].IsIn {
			sendEvent(conn, "new_contact_status", map[string]interface{}{"status": "error", "error": "Not on WhatsApp or query failed"})
			return
		}
		jid := resp[0].JID
		err = client.Store.Contacts.PutContactName(context.Background(), jid, data.Name, data.Name)
		if err != nil {
			sendEvent(conn, "new_contact_status", map[string]interface{}{"status": "error", "error": err.Error()})
			return
		}
		sendEvent(conn, "new_contact_status", map[string]interface{}{"status": "success", "jid": jid.String(), "name": data.Name})
		upsertContactCacheRow(jid.String(), data.Name, phone, false)
		populateChatsFromStore()
		broadcastChats()
	}()
}
