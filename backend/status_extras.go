package main

import (
	"context"
	"encoding/json"
	"log"
	"net"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waCommon"
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
}

type StatusMuteData struct {
	JID   string `json:"jid"`
	Muted bool   `json:"muted"`
}

type StatusPrivacyData struct {
	Mode string   `json:"mode"`
	JIDs []string `json:"jids"`
}

func noteStatusViewers(v *events.Receipt) {
	if v.Type != types.ReceiptTypeRead && v.Type != types.ReceiptTypePlayed {
		return
	}
	viewer := v.Sender.ToNonAD()
	if pn := storeTwin(viewer); viewer.Server == types.HiddenUserServer && !pn.IsEmpty() {
		viewer = pn.ToNonAD()
	}
	if viewer.IsEmpty() || isOwnJID(viewer) {
		return
	}
	for _, id := range v.MessageIDs {
		res, err := db.Exec(`INSERT INTO status_viewers (status_id, viewer, viewed_at) VALUES (?, ?, ?)
			ON CONFLICT(status_id, viewer) DO NOTHING`, string(id), viewer.String(), v.Timestamp.Unix())
		if err != nil {
			continue
		}
		if n, _ := res.RowsAffected(); n > 0 {
			broadcastEvent("status_viewed", map[string]interface{}{"id": string(id), "viewer": resolveDisplayName(viewer)})
		}
	}
}

func noteStatusReaction(v *events.Message, reaction *waE2E.ReactionMessage) {
	statusID := reaction.GetKey().GetID()
	if statusID == "" {
		return
	}
	viewer := "me"
	name := "You"
	if !v.Info.IsFromMe {
		_, sender := resolveLIDChat(v)
		viewer, name = sender.ToNonAD().String(), resolveDisplayName(sender)
	}
	ts := v.Info.Timestamp.Unix()
	db.Exec(`INSERT INTO status_viewers (status_id, viewer, viewed_at, reaction) VALUES (?, ?, ?, ?)
		ON CONFLICT(status_id, viewer) DO UPDATE SET reaction = excluded.reaction`, statusID, viewer, ts, reaction.GetText())
	if !v.Info.IsFromMe && reaction.GetText() != "" {
		broadcastEvent("status_reaction", map[string]interface{}{"id": statusID, "reactor": name, "emoji": reaction.GetText()})
	}
}

func handleGetStatusViewers(conn net.Conn, raw json.RawMessage) {
	var data StatusTargetData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	rows, err := db.Query(`SELECT viewer, viewed_at, COALESCE(reaction, '') FROM status_viewers
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
		name := viewer
		if jid, err := types.ParseJID(viewer); err == nil {
			name = resolveDisplayName(jid)
		}
		viewers = append(viewers, map[string]interface{}{"name": name, "timestamp": ts, "reaction": reaction})
	}
	sendEvent(conn, "status_viewers", map[string]interface{}{"id": data.ID, "viewers": viewers})
}

func statusPoster(id string) (types.JID, bool) {
	var sender string
	var fromMe bool
	if db.QueryRow(`SELECT COALESCE(sender_jid, ''), from_me FROM messages WHERE id = ? AND jid = 'status@broadcast'`, id).
		Scan(&sender, &fromMe) != nil {
		return types.EmptyJID, false
	}
	jid, err := types.ParseJID(sender)
	return jid, err == nil && !fromMe
}

func handleReactStatus(raw json.RawMessage) {
	var data StatusTargetData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		poster, ok := statusPoster(data.ID)
		if !ok {
			broadcastEvent("error", map[string]string{"message": "That status update isn't here any more."})
			return
		}
		now := time.Now()
		msg := &waE2E.Message{ReactionMessage: &waE2E.ReactionMessage{
			Key: &waCommon.MessageKey{
				RemoteJID:   proto.String(types.StatusBroadcastJID.String()),
				FromMe:      proto.Bool(false),
				ID:          proto.String(data.ID),
				Participant: proto.String(poster.String()),
			},
			Text:              proto.String(data.Emoji),
			SenderTimestampMS: proto.Int64(now.UnixMilli()),
		}}
		if _, err := wa().SendMessage(context.Background(), poster, msg, whatsmeow.SendRequestExtra{ID: wa().GenerateMessageID()}); err != nil {
			broadcastEvent("error", map[string]string{"message": "Couldn't react to the status: " + sendErrorText(err)})
			return
		}
		db.Exec(`INSERT INTO status_viewers (status_id, viewer, viewed_at, reaction) VALUES (?, 'me', ?, ?)
			ON CONFLICT(status_id, viewer) DO UPDATE SET reaction = excluded.reaction`, data.ID, now.Unix(), data.Emoji)
		broadcastEvent("status_reacted", map[string]interface{}{"id": data.ID, "emoji": data.Emoji})
	}()
}

func handleDeleteStatus(raw json.RawMessage) {
	var data StatusTargetData
	if err := json.Unmarshal(raw, &data); err != nil || data.ID == "" {
		return
	}
	go func() {
		msg := wa().BuildRevoke(types.StatusBroadcastJID, types.EmptyJID, types.MessageID(data.ID))
		extra := whatsmeow.SendRequestExtra{ID: wa().GenerateMessageID(), StatusRecipients: statusRecipients(context.Background())}
		if _, err := wa().SendMessage(context.Background(), types.StatusBroadcastJID, msg, extra); err != nil {
			broadcastEvent("error", map[string]string{"message": "Couldn't delete the status: " + sendErrorText(err)})
			return
		}
		removeStatus(data.ID)
	}()
}

func removeStatus(id string) {
	purgeMessages(`id = ? AND jid = 'status@broadcast'`, id)
	db.Exec(`DELETE FROM status_viewers WHERE status_id = ?`, id)
	broadcastEvent("status_deleted", map[string]interface{}{"id": id})
}

func noteStatusMute(v *events.UserStatusMute) {
	setStatusMuted(v.JID, v.Action.GetMuted())
}

func setStatusMuted(jid types.JID, muted bool) {
	if pn := storeTwin(jid); jid.Server == types.HiddenUserServer && !pn.IsEmpty() {
		jid = pn
	}
	if muted {
		db.Exec(`INSERT OR IGNORE INTO status_mutes (jid) VALUES (?)`, jid.ToNonAD().String())
	} else {
		db.Exec(`DELETE FROM status_mutes WHERE jid = ?`, jid.ToNonAD().String())
	}
	broadcastEvent("status_muted", map[string]interface{}{"jid": jid.ToNonAD().String(), "muted": muted})
}

func statusMuted() map[string]bool {
	muted := map[string]bool{}
	rows, err := db.Query(`SELECT jid FROM status_mutes`)
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

func statusMutedFor(muted map[string]bool, sender string) bool {
	if muted[sender] {
		return true
	}
	jid, err := types.ParseJID(sender)
	if err != nil {
		return false
	}
	twin := storeTwin(jid)
	return !twin.IsEmpty() && muted[twin.ToNonAD().String()]
}

func quoteStatus(ctxInfo *waE2E.ContextInfo, quotedID string) {
	var n int
	db.QueryRow(`SELECT COUNT(*) FROM messages WHERE id = ? AND jid = 'status@broadcast'`, quotedID).Scan(&n)
	if n > 0 {
		ctxInfo.RemoteJID = proto.String(types.StatusBroadcastJID.String())
	}
}

func handleSetStatusMute(raw json.RawMessage) {
	var data StatusMuteData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		if err := sendAppStateWithRetry(context.Background(), appstate.BuildUserStatusMute(jid.ToNonAD(), data.Muted)); err != nil {
			broadcastEvent("error", map[string]string{"message": "Couldn't change muted status updates: " + err.Error()})
			return
		}
		setStatusMuted(jid, data.Muted)
	}()
}

func noteStatusPrivacy(v *events.StatusPrivacy) {
	if v.Action == nil {
		return
	}
	if raw, err := proto.Marshal(v.Action); err == nil {
		db.Exec(`INSERT OR REPLACE INTO app_settings (key, value) VALUES (?, ?)`, statusPrivacyKey, raw)
	}
}

func storedStatusPrivacy() *waSyncAction.StatusPrivacyAction {
	var raw []byte
	if db.QueryRow(`SELECT value FROM app_settings WHERE key = ?`, statusPrivacyKey).Scan(&raw) != nil || len(raw) == 0 {
		return nil
	}
	var action waSyncAction.StatusPrivacyAction
	if proto.Unmarshal(raw, &action) != nil {
		return nil
	}
	return &action
}

func statusAudience(ctx context.Context) (mode string, list []types.JID) {
	if action := storedStatusPrivacy(); action != nil {
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
	if settings, err := wa().GetStatusPrivacy(ctx); err == nil && len(settings) > 0 {
		switch settings[0].Type {
		case types.StatusPrivacyTypeWhitelist:
			return "only", settings[0].List
		case types.StatusPrivacyTypeBlacklist:
			return "except", settings[0].List
		}
	}
	return "contacts", nil
}

func statusRecipients(ctx context.Context) []types.JID {
	action := storedStatusPrivacy()
	if action == nil {
		return nil
	}
	user := func(jid types.JID) types.JID {
		jid = jid.ToNonAD()
		if pn := storeTwin(jid); jid.Server == types.HiddenUserServer && !pn.IsEmpty() {
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
	contacts, err := wa().Store.Contacts.GetAllContacts(ctx)
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

func handleGetStatusPrivacy(conn net.Conn) {
	mode, list := statusAudience(context.Background())
	people := []map[string]string{}
	for _, jid := range list {
		people = append(people, map[string]string{"jid": jid.String(), "name": resolveDisplayName(jid)})
	}
	sendEvent(conn, "status_privacy", map[string]interface{}{"mode": mode, "people": people})
}

func handleSetStatusPrivacy(conn net.Conn, raw json.RawMessage) {
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
		if mode != waSyncAction.StatusPrivacyAction_CONTACTS {
			for _, s := range data.JIDs {
				if jid, err := types.ParseJID(s); err == nil {
					action.UserJID = append(action.UserJID, jid.ToNonAD().String())
				}
			}
		}
		if err := sendAppStateWithRetry(context.Background(), appstate.BuildStatusPrivacy(action)); err != nil {
			broadcastEvent("error", map[string]string{"message": "Couldn't change who sees your status: " + err.Error()})
			return
		}
		noteStatusPrivacy(&events.StatusPrivacy{Timestamp: time.Now(), Action: action})
		sendEvent(conn, "status_privacy_saved", map[string]interface{}{"mode": data.Mode})
	}()
}

// storeHistoryStatuses saves the status updates the phone sends at linking
// (the INITIAL_STATUS_V3 history sync chunk: statuses still up from the last
// 24 hours), which WA Web shows in its Updates tab right away. Before this
// they were dropped, so only statuses posted after linking showed.
func storeHistoryStatuses(statuses []*waWeb.WebMessageInfo) {
	cutoff := time.Now().Add(-24 * time.Hour)
	stored := 0
	for _, wmi := range statuses {
		evt, err := wa().ParseWebMessage(types.StatusBroadcastJID, wmi)
		if err != nil {
			log.Printf("Skipping history status %s: %v", wmi.GetKey().GetID(), err)
			continue
		}
		if evt.Info.Timestamp.Before(cutoff) || evt.Message == nil {
			continue
		}
		if evt.Info.Sender.IsEmpty() {
			continue
		}
		handleIncomingStatus(evt)
		stored++
	}
	log.Printf("History sync: %d of %d status updates stored", stored, len(statuses))
}
