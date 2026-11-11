package main

import (
	"context"
	"encoding/json"
	"log"
	"net"
	"reflect"
	"time"
	"unsafe"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/appstate"
	waBinary "go.mau.fi/whatsmeow/binary"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/proto/waSyncAction"
	"go.mau.fi/whatsmeow/proto/waWeb"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

const statusPrivacyKey = "status_privacy"

type StatusTargetData struct {
	ID        string `json:"id"`
	PosterJID string `json:"poster_jid"`
	Emoji     string `json:"emoji"`
	Liked     bool   `json:"liked"`
}

type StatusMuteData struct {
	JID   string `json:"jid"`
	Muted bool   `json:"muted"`
}

type StatusPrivacyData struct {
	Mode string   `json:"mode"`
	JIDs []string `json:"jids"`
}

func (acc *Account) noteStatusViewers(v *events.Receipt) {
	if v.Type != types.ReceiptTypeRead {
		return
	}
	if acc.wa().GetPrivacySettings(context.Background()).ReadReceipts == types.PrivacySettingNone {
		return
	}
	viewer := v.Sender.ToNonAD()
	if pn := acc.storeTwin(viewer); viewer.Server == types.HiddenUserServer && !pn.IsEmpty() {
		viewer = pn.ToNonAD()
	}
	if viewer.IsEmpty() || acc.isOwnJID(viewer) {
		return
	}
	for _, id := range v.MessageIDs {
		res, err := acc.db.Exec(`INSERT INTO status_viewers (status_id, viewer, viewed_at) VALUES (?, ?, ?)
			ON CONFLICT(status_id, viewer) DO NOTHING`, string(id), viewer.String(), v.Timestamp.Unix())
		if err != nil {
			continue
		}
		if n, _ := res.RowsAffected(); n > 0 {
			acc.broadcastEvent("status_viewed", map[string]interface{}{"id": string(id), "viewer": acc.resolveDisplayName(viewer)})
		}
	}
}

// noteStatusSeenElsewhere marks statuses as seen when another of our own
// devices (the phone, WA Web) viewed them, as WA Web does for peer receipts.
func (acc *Account) noteStatusSeenElsewhere(v *events.Receipt) {
	for _, id := range v.MessageIDs {
		acc.db.Exec(`UPDATE messages SET viewed = 1 WHERE id = ? AND jid = 'status@broadcast'`, string(id))
	}
}

func (acc *Account) noteStatusReaction(v *events.Message, reaction *waE2E.ReactionMessage) {
	statusID := reaction.GetKey().GetID()
	if statusID == "" {
		return
	}
	viewer := "me"
	name := "You"
	if !v.Info.IsFromMe {
		_, sender := acc.resolveLIDChat(v)
		viewer, name = sender.ToNonAD().String(), acc.resolveDisplayName(sender)
	}
	ts := v.Info.Timestamp.Unix()
	acc.db.Exec(`INSERT INTO status_viewers (status_id, viewer, viewed_at, reaction) VALUES (?, ?, ?, ?)
		ON CONFLICT(status_id, viewer) DO UPDATE SET reaction = excluded.reaction`, statusID, viewer, ts, reaction.GetText())
	if !v.Info.IsFromMe && reaction.GetText() != "" {
		acc.broadcastEvent("status_reaction", map[string]interface{}{"id": statusID, "reactor": name, "emoji": reaction.GetText()})
	}
}

func (acc *Account) handleGetStatusViewers(conn net.Conn, raw json.RawMessage) {
	var data StatusTargetData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	rows, err := acc.db.Query(`SELECT viewer, viewed_at, COALESCE(reaction, '') FROM status_viewers
		WHERE status_id = ? AND viewer != 'me' ORDER BY viewed_at DESC`, data.ID)
	if err != nil {
		return
	}
	defer rows.Close()
	viewers := []map[string]interface{}{}
	for rows.Next() {
		var viewer, reaction string
		var ts int64
		if rows.Scan(&viewer, &ts, &reaction) != nil {
			continue
		}
		viewers = append(viewers, map[string]interface{}{"jid": viewer, "name": viewer, "timestamp": ts, "reaction": reaction})
	}
	rows.Close()
	// Names are looked up with the rows closed, as that runs queries of its own.
	for _, v := range viewers {
		if jid, err := types.ParseJID(v["jid"].(string)); err == nil {
			v["name"] = acc.resolveDisplayName(jid)
		}
	}
	acc.sendEvent(conn, "status_viewers", map[string]interface{}{"id": data.ID, "viewers": viewers})
}

func (acc *Account) statusPoster(id string) (types.JID, bool) {
	var sender string
	var fromMe bool
	if acc.db.QueryRow(`SELECT COALESCE(sender_jid, ''), from_me FROM messages WHERE id = ? AND jid = 'status@broadcast'`, id).
		Scan(&sender, &fromMe) != nil {
		return types.EmptyJID, false
	}
	jid, err := types.ParseJID(sender)
	return jid, err == nil && !fromMe
}

func (acc *Account) handleReactStatus(raw json.RawMessage) {
	var data StatusTargetData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	// The like is a real reaction, sent to the poster alone (WA Web's
	// sendStatusReaction); the other emoji are quick replies in their chat.
	if data.Emoji == whatsmeow.StatusLikeEmoji {
		acc.likeStatus(data.ID, true)
		return
	}
	if data.Emoji == "" && acc.myStatusReaction(data.ID) == whatsmeow.StatusLikeEmoji {
		acc.likeStatus(data.ID, false)
		return
	}
	go func() {
		poster, ok := acc.statusPoster(data.ID)
		if !ok {
			acc.broadcastEvent("error", map[string]string{"message": "That status update isn't here any more."})
			return
		}
		now := time.Now()
		quoted, _, _, qErr := acc.getMessageForQuote(data.ID)
		if qErr != nil {
			quoted = nil
		}
		ctxInfo := &waE2E.ContextInfo{
			StanzaID:      proto.String(data.ID),
			Participant:   proto.String(poster.String()),
			RemoteJID:     proto.String(types.StatusBroadcastJID.String()),
			QuotedMessage: quoted,
		}
		msg := &waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{Text: proto.String(data.Emoji), ContextInfo: ctxInfo}}
		if _, err := acc.wa().SendMessage(context.Background(), poster, msg, whatsmeow.SendRequestExtra{ID: acc.wa().GenerateMessageID()}); err != nil {
			acc.broadcastEvent("error", map[string]string{"message": "Couldn't react to the status: " + sendErrorText(err)})
			return
		}
		acc.db.Exec(`INSERT INTO status_viewers (status_id, viewer, viewed_at, reaction) VALUES (?, 'me', ?, ?)
			ON CONFLICT(status_id, viewer) DO UPDATE SET reaction = excluded.reaction`, data.ID, now.Unix(), data.Emoji)
		acc.broadcastEvent("status_reacted", map[string]interface{}{"id": data.ID, "emoji": data.Emoji})
	}()
}

func (acc *Account) handleLikeStatus(raw json.RawMessage) {
	var data StatusTargetData
	if err := json.Unmarshal(raw, &data); err != nil || data.ID == "" {
		return
	}
	acc.likeStatus(data.ID, data.Liked)
}

// likeStatus likes a status update or takes the like back, as WA Web's like
// button does: a green heart reaction, or an empty one to unlike, sent only
// to the poster's devices and our own.
func (acc *Account) likeStatus(id string, liked bool) {
	go func() {
		poster, ok := acc.statusPoster(id)
		if !ok {
			acc.broadcastEvent("error", map[string]string{"message": "That status update isn't here any more."})
			return
		}
		emoji := ""
		if liked {
			emoji = whatsmeow.StatusLikeEmoji
		}
		resp, err := acc.wa().SendStatusReaction(context.Background(), poster, types.MessageID(id), emoji)
		if err != nil {
			acc.broadcastEvent("error", map[string]string{"message": "Couldn't like the status: " + sendErrorText(err)})
			return
		}
		ts := resp.Timestamp
		if ts.IsZero() {
			ts = time.Now()
		}
		acc.db.Exec(`INSERT INTO status_viewers (status_id, viewer, viewed_at, reaction) VALUES (?, 'me', ?, ?)
			ON CONFLICT(status_id, viewer) DO UPDATE SET reaction = excluded.reaction`, id, ts.Unix(), emoji)
		acc.broadcastEvent("status_reacted", map[string]interface{}{"id": id, "emoji": emoji, "like": true})
	}()
}

func (acc *Account) myStatusReaction(id string) string {
	var reaction string
	acc.db.QueryRow(`SELECT COALESCE(reaction, '') FROM status_viewers WHERE status_id = ? AND viewer = 'me'`, id).Scan(&reaction)
	return reaction
}

// statusSetting names the status privacy for the <meta status_setting> of a
// status update, as WA Web's toStatusSettingMetaAttribute does.
func (acc *Account) statusSetting(ctx context.Context) string {
	mode, _ := acc.statusAudience(ctx)
	switch mode {
	case "only":
		return whatsmeow.StatusSettingAllowList
	case "except":
		return whatsmeow.StatusSettingDenyList
	}
	return whatsmeow.StatusSettingContacts
}

func (acc *Account) handleDeleteStatus(raw json.RawMessage) {
	var data StatusTargetData
	if err := json.Unmarshal(raw, &data); err != nil || data.ID == "" {
		return
	}
	go func() {
		msg := acc.wa().BuildRevoke(types.StatusBroadcastJID, types.EmptyJID, types.MessageID(data.ID))
		recipients := acc.storedStatusRecipients(data.ID)
		if len(recipients) == 0 {
			recipients = acc.statusRecipients(context.Background())
		}
		extra := whatsmeow.SendRequestExtra{ID: acc.wa().GenerateMessageID(), StatusRecipients: recipients}
		if _, err := acc.wa().SendMessage(context.Background(), types.StatusBroadcastJID, msg, extra); err != nil {
			acc.broadcastEvent("error", map[string]string{"message": "Couldn't delete the status: " + sendErrorText(err)})
			return
		}
		acc.removeStatus(data.ID)
	}()
}

// rememberStatusRecipients keeps who a status update was sent to, so a later
// delete is revoked for exactly those people, as WA Web does.
func (acc *Account) rememberStatusRecipients(id string, recipients []types.JID) {
	for _, jid := range recipients {
		acc.db.Exec(`INSERT OR IGNORE INTO status_recipients (status_id, jid) VALUES (?, ?)`, id, jid.ToNonAD().String())
	}
}

func (acc *Account) storedStatusRecipients(id string) []types.JID {
	rows, err := acc.db.Query(`SELECT jid FROM status_recipients WHERE status_id = ?`, id)
	if err != nil {
		return nil
	}
	var out []types.JID
	for rows.Next() {
		var s string
		if rows.Scan(&s) != nil {
			continue
		}
		if jid, err := types.ParseJID(s); err == nil {
			out = append(out, jid)
		}
	}
	rows.Close()
	return out
}

func (acc *Account) removeStatus(id string) {
	acc.db.Exec(`DELETE FROM status_recipients WHERE status_id = ?`, id)
	acc.purgeMessages(`id = ? AND jid = 'status@broadcast'`, id)
	acc.db.Exec(`DELETE FROM status_viewers WHERE status_id = ?`, id)
	acc.broadcastEvent("status_deleted", map[string]interface{}{"id": id})
}

func (acc *Account) noteStatusMute(v *events.UserStatusMute) {
	acc.setStatusMuted(v.JID, v.Action.GetMuted())
}

func (acc *Account) setStatusMuted(jid types.JID, muted bool) {
	if pn := acc.storeTwin(jid); jid.Server == types.HiddenUserServer && !pn.IsEmpty() {
		jid = pn
	}
	if muted {
		acc.db.Exec(`INSERT OR IGNORE INTO status_mutes (jid) VALUES (?)`, jid.ToNonAD().String())
	} else {
		acc.db.Exec(`DELETE FROM status_mutes WHERE jid = ?`, jid.ToNonAD().String())
	}
	acc.broadcastEvent("status_muted", map[string]interface{}{"jid": jid.ToNonAD().String(), "muted": muted})
}

func (acc *Account) statusMuted() map[string]bool {
	muted := map[string]bool{}
	rows, err := acc.db.Query(`SELECT jid FROM status_mutes`)
	if err != nil {
		return muted
	}
	defer rows.Close()
	for rows.Next() {
		var jid string
		if rows.Scan(&jid) == nil {
			muted[jid] = true
		}
	}
	return muted
}

func (acc *Account) statusMutedFor(muted map[string]bool, sender string) bool {
	if muted[sender] {
		return true
	}
	jid, err := types.ParseJID(sender)
	if err != nil {
		return false
	}
	twin := acc.storeTwin(jid)
	return !twin.IsEmpty() && muted[twin.ToNonAD().String()]
}

func (acc *Account) quoteStatus(ctxInfo *waE2E.ContextInfo, quotedID string) {
	var n int
	acc.db.QueryRow(`SELECT COUNT(*) FROM messages WHERE id = ? AND jid = 'status@broadcast'`, quotedID).Scan(&n)
	if n > 0 {
		ctxInfo.RemoteJID = proto.String(types.StatusBroadcastJID.String())
	}
}

func (acc *Account) handleSetStatusMute(raw json.RawMessage) {
	var data StatusMuteData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		targets := []types.JID{jid.ToNonAD()}
		if twin := acc.storeTwin(jid.ToNonAD()); !twin.IsEmpty() && twin.ToNonAD() != jid.ToNonAD() {
			targets = append(targets, twin.ToNonAD())
		}
		for _, target := range targets {
			if err := acc.sendAppStateWithRetry(context.Background(), appstate.BuildUserStatusMute(target, data.Muted)); err != nil {
				acc.broadcastEvent("error", map[string]string{"message": "Couldn't change muted status updates: " + err.Error()})
				return
			}
		}
		acc.setStatusMuted(jid, data.Muted)
	}()
}

func (acc *Account) noteStatusPrivacy(v *events.StatusPrivacy) {
	if v.Action == nil {
		return
	}
	// WA Web ignores Close Friends and custom list modes from the phone and
	// keeps the last contacts/except/only setting it knew.
	switch v.Action.GetMode() {
	case waSyncAction.StatusPrivacyAction_CLOSE_FRIENDS, waSyncAction.StatusPrivacyAction_CUSTOM_LIST:
		return
	}
	if raw, err := proto.Marshal(v.Action); err == nil {
		acc.db.Exec(`INSERT OR REPLACE INTO app_settings (key, value) VALUES (?, ?)`, statusPrivacyKey, raw)
	}
}

func (acc *Account) storedStatusPrivacy() *waSyncAction.StatusPrivacyAction {
	var raw []byte
	if acc.db.QueryRow(`SELECT value FROM app_settings WHERE key = ?`, statusPrivacyKey).Scan(&raw) != nil || len(raw) == 0 {
		return nil
	}
	var action waSyncAction.StatusPrivacyAction
	if proto.Unmarshal(raw, &action) != nil {
		return nil
	}
	return &action
}

func (acc *Account) statusAudience(ctx context.Context) (mode string, list []types.JID) {
	if action := acc.storedStatusPrivacy(); action != nil {
		for _, s := range action.GetUserJID() {
			if jid, err := types.ParseJID(s); err == nil {
				list = append(list, jid)
			}
		}
		switch action.GetMode() {
		case waSyncAction.StatusPrivacyAction_ALLOW_LIST:
			return "only", list
		case waSyncAction.StatusPrivacyAction_DENY_LIST:
			return "except", list
		}
		return "contacts", nil
	}
	if settings, err := acc.wa().GetStatusPrivacy(ctx); err == nil && len(settings) > 0 {
		switch settings[0].Type {
		case types.StatusPrivacyTypeWhitelist:
			return "only", settings[0].List
		case types.StatusPrivacyTypeBlacklist:
			return "except", settings[0].List
		}
	}
	return "contacts", nil
}

// statusPostBlocker returns why a status update can't be posted right now, or
// an empty string: an empty audience is refused the way WA Web's privacy popup
// warns about it.
func (acc *Account) statusPostBlocker(ctx context.Context) string {
	action := acc.storedStatusPrivacy()
	if action == nil {
		return ""
	}
	switch action.GetMode() {
	case waSyncAction.StatusPrivacyAction_ALLOW_LIST, waSyncAction.StatusPrivacyAction_DENY_LIST:
		if recipients := acc.statusRecipients(ctx); recipients != nil && len(recipients) == 0 {
			return "This status update would be sent to 0 contacts. Check your status privacy."
		}
	}
	return ""
}

func (acc *Account) statusRecipients(ctx context.Context) []types.JID {
	action := acc.storedStatusPrivacy()
	if action == nil {
		return nil
	}
	user := func(jid types.JID) types.JID {
		jid = jid.ToNonAD()
		if pn := acc.storeTwin(jid); jid.Server == types.HiddenUserServer && !pn.IsEmpty() {
			return pn.ToNonAD()
		}
		return jid
	}
	listed := map[types.JID]bool{}
	for _, s := range action.GetUserJID() {
		if jid, err := types.ParseJID(s); err == nil {
			listed[user(jid)] = true
		}
	}
	out := []types.JID{}
	if action.GetMode() == waSyncAction.StatusPrivacyAction_ALLOW_LIST {
		for jid := range listed {
			out = append(out, jid)
		}
		return out
	}
	contacts, err := acc.wa().Store.Contacts.GetAllContacts(ctx)
	if err != nil {
		return nil
	}
	deny := action.GetMode() == waSyncAction.StatusPrivacyAction_DENY_LIST
	for jid, contact := range contacts {
		if contact.FullName == "" || (deny && listed[user(jid)]) {
			continue
		}
		out = append(out, jid)
	}
	return out
}

func (acc *Account) handleGetStatusPrivacy(conn net.Conn) {
	mode, list := acc.statusAudience(context.Background())
	people := []map[string]string{}
	for _, jid := range list {
		people = append(people, map[string]string{"jid": jid.String(), "name": acc.resolveDisplayName(jid)})
	}
	acc.sendEvent(conn, "status_privacy", map[string]interface{}{"mode": mode, "people": people})
}

func (acc *Account) handleSetStatusPrivacy(conn net.Conn, raw json.RawMessage) {
	var data StatusPrivacyData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		mode := waSyncAction.StatusPrivacyAction_CONTACTS
		switch data.Mode {
		case "only":
			mode = waSyncAction.StatusPrivacyAction_ALLOW_LIST
		case "except":
			mode = waSyncAction.StatusPrivacyAction_DENY_LIST
		}
		action := &waSyncAction.StatusPrivacyAction{Mode: mode.Enum()}
		if current := acc.storedStatusPrivacy(); current != nil {
			action.ShareToFb = current.ShareToFb
			action.ShareToIg = current.ShareToIg
		}
		if mode != waSyncAction.StatusPrivacyAction_CONTACTS {
			for _, s := range data.JIDs {
				if jid, err := types.ParseJID(s); err == nil {
					action.UserJID = append(action.UserJID, jid.ToNonAD().String())
				}
			}
		}
		if err := acc.sendAppStateWithRetry(context.Background(), appstate.BuildStatusPrivacy(action)); err != nil {
			acc.broadcastEvent("error", map[string]string{"message": "Couldn't change who sees your status: " + err.Error()})
			return
		}
		acc.noteStatusPrivacy(&events.StatusPrivacy{Timestamp: time.Now(), Action: action})
		acc.sendEvent(conn, "status_privacy_saved", map[string]interface{}{"mode": data.Mode})
	}()
}

// storeHistoryStatuses saves the status updates the phone sends at linking
// (the INITIAL_STATUS_V3 history sync chunk: statuses still up from the last
// 24 hours), which WA Web shows in its Updates tab right away. Before this
// they were dropped, so only statuses posted after linking showed.
func (acc *Account) storeHistoryStatuses(statuses []*waWeb.WebMessageInfo) {
	cutoff := time.Now().Add(-24 * time.Hour)
	stored := 0
	for _, wmi := range statuses {
		if wmi == nil || wmi.GetMessage() == nil {
			continue
		}
		ts := time.Unix(int64(wmi.GetMessageTimestamp()), 0)
		if ts.Before(cutoff) {
			continue
		}
		evt, err := acc.wa().ParseWebMessage(types.StatusBroadcastJID, wmi)
		if err != nil || evt == nil || evt.Info.Sender.IsEmpty() {
			var sender types.JID
			fromMe := wmi.GetKey().GetFromMe()
			if fromMe && acc.wa() != nil && acc.wa().Store != nil && acc.wa().Store.ID != nil {
				sender = acc.wa().Store.ID.ToNonAD()
			} else if p := wmi.GetParticipant(); p != "" {
				sender, _ = types.ParseJID(p)
			} else if p := wmi.GetKey().GetParticipant(); p != "" {
				sender, _ = types.ParseJID(p)
			} else if r := wmi.GetKey().GetRemoteJID(); r != "" && r != "status@broadcast" {
				sender, _ = types.ParseJID(r)
			}
			if sender.IsEmpty() {
				log.Printf("Skipping history status %s: cannot determine sender", wmi.GetKey().GetID())
				continue
			}
			evt = &events.Message{
				Info: types.MessageInfo{
					MessageSource: types.MessageSource{
						Chat:     types.StatusBroadcastJID,
						Sender:   sender,
						IsFromMe: fromMe,
						IsGroup:  true,
					},
					ID:        types.MessageID(wmi.GetKey().GetID()),
					PushName:  wmi.GetPushName(),
					Timestamp: ts,
				},
				RawMessage: wmi.GetMessage(),
				Message:    unwrapMessage(wmi.GetMessage()),
			}
		}
		acc.handleIncomingStatus(evt)
		stored++
	}
	log.Printf("History sync: %d of %d status updates stored", stored, len(statuses))
}

// Route encrypted contact status stanzas to handleEncryptedMessage.
// whatsmeow routes <status> to handleUnencryptedMessage, dropping non-newsletters (tulir/whatsmeow#1323).
func getNodeHandlersMap(c *whatsmeow.Client) map[string]func(context.Context, *waBinary.Node) {
	if c == nil {
		return nil
	}
	v := reflect.ValueOf(c).Elem()
	field := v.FieldByName("nodeHandlers")
	if !field.IsValid() {
		return nil
	}
	type nodeHandler = func(context.Context, *waBinary.Node)
	ptr := (*map[string]nodeHandler)(unsafe.Pointer(field.UnsafeAddr()))
	if ptr == nil || *ptr == nil {
		return nil
	}
	return *ptr
}

func hookStatusStanzas(c *whatsmeow.Client) {
	handlers := getNodeHandlersMap(c)
	if handlers == nil {
		return
	}
	origStatus, okStatus := handlers["status"]
	msgHandler, okMsg := handlers["message"]
	if !okStatus || !okMsg {
		return
	}
	handlers["status"] = func(ctx context.Context, node *waBinary.Node) {
		if len(node.GetChildrenByTag("enc")) > 0 {
			msgHandler(ctx, node)
		} else {
			origStatus(ctx, node)
		}
	}
}
