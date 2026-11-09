package main

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"net"
	"strings"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/proto/waWeb"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

// Scheduled messages, ported from WA Web: a scheduled message is sent right
// away as a ConditionalRevealMessage (its content encrypted with a reveal
// key) and the server posts it at the scheduled time. Until then WA Web keeps
// it in its reveal-key store (WAWebScheduledMsgRevealKeyStore) rather than in
// the chat; scheduled_messages is that store, and scheduled_orphan_keys is
// WAWebScheduledMsgOrphanRevealKeyStore (a reveal key that came before its
// message).

// The statuses WA Web gives a reveal-key record.
const (
	scheduledPending  = "PENDING"
	scheduledFailed   = "FAILED"
	scheduledRevealed = "REVEALED"
)

type scheduledRecord struct {
	MsgID       string
	ChatJID     string // the chat as the app shows it
	WireChat    string // the chat the message went to (LID for a LID chat)
	SenderJID   string // empty for our own
	FromMe      bool
	RevealKeyID string
	RevealKey   []byte // empty until a received message is revealed
	EncPayload  []byte
	EncIV       []byte
	ScheduledAt int64
	Status      string
	CreatedAt   int64
}

func (acc *Account) ensureScheduledMessages() error {
	if _, err := acc.db.Exec(`CREATE TABLE IF NOT EXISTS scheduled_messages (
		msg_id TEXT PRIMARY KEY,
		chat_jid TEXT NOT NULL,
		wire_chat TEXT NOT NULL DEFAULT '',
		sender_jid TEXT NOT NULL DEFAULT '',
		from_me INTEGER NOT NULL DEFAULT 0,
		reveal_key_id TEXT NOT NULL,
		reveal_key BLOB,
		enc_payload BLOB,
		enc_iv BLOB,
		scheduled_at INTEGER NOT NULL DEFAULT 0,
		status TEXT NOT NULL DEFAULT 'PENDING',
		created_at INTEGER NOT NULL DEFAULT 0
	)`); err != nil {
		return err
	}
	if _, err := acc.db.Exec(`CREATE INDEX IF NOT EXISTS idx_scheduled_messages_rkid ON scheduled_messages(reveal_key_id)`); err != nil {
		return err
	}
	if _, err := acc.db.Exec(`CREATE INDEX IF NOT EXISTS idx_scheduled_messages_chat ON scheduled_messages(chat_jid, scheduled_at)`); err != nil {
		return err
	}
	if _, err := acc.db.Exec(`CREATE TABLE IF NOT EXISTS scheduled_orphan_keys (
		reveal_key_id TEXT PRIMARY KEY,
		reveal_key BLOB NOT NULL,
		created_at INTEGER NOT NULL DEFAULT 0
	)`); err != nil {
		return err
	}
	// WA Web's cleanupExpiredRevealKeys / cleanupExpiredOrphanRevealKeys.
	cutoff := time.Now().Add(-whatsmeow.ScheduledMessageRevealKeyRetention).Unix()
	acc.db.Exec(`DELETE FROM scheduled_messages WHERE (CASE WHEN scheduled_at > 0 THEN scheduled_at ELSE created_at END) < ?`, cutoff)
	acc.db.Exec(`DELETE FROM scheduled_orphan_keys WHERE created_at < ?`, cutoff)
	return nil
}

const scheduledColumns = `msg_id, chat_jid, wire_chat, sender_jid, from_me, reveal_key_id, reveal_key, enc_payload, enc_iv, scheduled_at, status, created_at`

func scanScheduled(row interface{ Scan(...any) error }) (*scheduledRecord, error) {
	var rec scheduledRecord
	err := row.Scan(&rec.MsgID, &rec.ChatJID, &rec.WireChat, &rec.SenderJID, &rec.FromMe, &rec.RevealKeyID,
		&rec.RevealKey, &rec.EncPayload, &rec.EncIV, &rec.ScheduledAt, &rec.Status, &rec.CreatedAt)
	if err != nil {
		return nil, err
	}
	return &rec, nil
}

func (acc *Account) storeScheduled(rec scheduledRecord) error {
	if rec.CreatedAt == 0 {
		rec.CreatedAt = time.Now().Unix()
	}
	if rec.Status == "" {
		rec.Status = scheduledPending
	}
	_, err := acc.db.Exec(`INSERT OR REPLACE INTO scheduled_messages (`+scheduledColumns+`) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
		rec.MsgID, rec.ChatJID, rec.WireChat, rec.SenderJID, rec.FromMe, rec.RevealKeyID,
		rec.RevealKey, rec.EncPayload, rec.EncIV, rec.ScheduledAt, rec.Status, rec.CreatedAt)
	return err
}

func (acc *Account) scheduledBy(column, value string) *scheduledRecord {
	rec, err := scanScheduled(acc.db.QueryRow(`SELECT `+scheduledColumns+` FROM scheduled_messages WHERE `+column+` = ? LIMIT 1`, value))
	if err != nil {
		if !errors.Is(err, sql.ErrNoRows) {
			log.Printf("Couldn't read scheduled message by %s: %v", column, err)
		}
		return nil
	}
	return rec
}

func (acc *Account) deleteScheduled(msgID string) {
	if _, err := acc.db.Exec(`DELETE FROM scheduled_messages WHERE msg_id = ?`, msgID); err != nil {
		log.Printf("Couldn't delete scheduled message %s: %v", msgID, err)
	}
}

// pendingScheduledCount is WA Web's getScheduledMessageCount: what counts
// toward the 30-per-chat limit.
func (acc *Account) pendingScheduledCount(chat string) int {
	var n int
	acc.db.QueryRow(`SELECT COUNT(*) FROM scheduled_messages WHERE chat_jid = ? AND status = ? AND scheduled_at > 0`, chat, scheduledPending).Scan(&n)
	return n
}

// scheduledForChat is WA Web's getScheduledMessagesForChat, minus the ones
// it can't show (no reveal key yet: other people's).
func (acc *Account) scheduledForChat(chat string) []*scheduledRecord {
	rows, err := acc.db.Query(`SELECT `+scheduledColumns+` FROM scheduled_messages
		WHERE chat_jid = ? AND status IN (?, ?) AND scheduled_at > 0 AND length(reveal_key) > 0
		ORDER BY scheduled_at`, chat, scheduledPending, scheduledFailed)
	if err != nil {
		log.Printf("Couldn't list scheduled messages: %v", err)
		return nil
	}
	defer rows.Close()
	var out []*scheduledRecord
	for rows.Next() {
		if rec, err := scanScheduled(rows); err == nil {
			out = append(out, rec)
		}
	}
	return out
}

func (rec *scheduledRecord) decrypt() (*waE2E.Message, error) {
	if len(rec.RevealKey) == 0 {
		return nil, errors.New("no reveal key yet")
	}
	return whatsmeow.DecryptScheduledMessage(&waE2E.ConditionalRevealMessage{
		EncPayload:  rec.EncPayload,
		EncIV:       rec.EncIV,
		RevealKeyID: proto.String(rec.RevealKeyID),
	}, rec.RevealKey)
}

func (acc *Account) scheduledChanged(chat string) {
	acc.broadcastEvent("scheduled_messages", acc.scheduledListPayload(chat))
}

// noteScheduledCreated is WA Web's ScheduledMessageCreated system bubble.
func (acc *Account) noteScheduledCreated(chat string, msgID string) {
	now := time.Now()
	acc.insertGroupSystemMessage(chat, "system_scheduled_"+msgID, "You scheduled a message", now, false)
}

// scheduledWhen says when, like WA Web's list does with WAWebScheduledMsgDay:
// "today at 18:30", "tomorrow at 09:00" or "Wed, Nov 11 at 09:00".
func scheduledWhen(at, now time.Time) string {
	at, now = at.Local(), now.Local()
	clock := at.Format("15:04")
	y, m, d := now.Date()
	today := time.Date(y, m, d, 0, 0, 0, 0, time.Local)
	switch day := time.Date(at.Year(), at.Month(), at.Day(), 0, 0, 0, 0, time.Local); {
	case day.Equal(today):
		return "today at " + clock
	case day.Equal(today.AddDate(0, 0, 1)):
		return "tomorrow at " + clock
	default:
		return at.Format("Mon, Jan 2") + " at " + clock
	}
}

func schedulableChat(jid types.JID) bool {
	switch jid.Server {
	case types.DefaultUserServer, types.HiddenUserServer, types.GroupServer:
		return !jid.IsBot()
	}
	return false
}

// receiveConditionalReveal is WA Web's WAWebConditionalRevealPreProcessor
// for a message carrying a ConditionalRevealMessage. It reports whether the
// message is parked as scheduled (not shown in the chat). When a reveal key
// for it already came (an orphan key), the message is revealed instead.
func (acc *Account) receiveConditionalReveal(v *events.Message, crm *waE2E.ConditionalRevealMessage) bool {
	if crm.GetEncPayload() == nil || crm.GetEncIV() == nil || crm.GetRevealKeyID() == "" {
		log.Printf("Scheduled message %s is missing fields; showing it as is", v.Info.ID)
		return false
	}
	chat, sender := acc.resolveLIDChat(v)
	rkid := crm.GetRevealKeyID()
	rec := scheduledRecord{
		MsgID:       v.Info.ID,
		ChatJID:     chat.String(),
		WireChat:    v.Info.Chat.ToNonAD().String(),
		RevealKeyID: rkid,
		EncPayload:  crm.GetEncPayload(),
		EncIV:       crm.GetEncIV(),
	}
	// A message we scheduled on another device carries its key inline.
	if meta := v.Info.MsgMetaInfo.ScheduledMessage; meta != nil && meta.RevealKeyID == rkid && len(meta.RevealKey) > 0 {
		rec.FromMe = true
		rec.RevealKey = meta.RevealKey
		rec.ScheduledAt = meta.Time.Unix()
		if acc.pendingScheduledCount(rec.ChatJID) >= whatsmeow.ScheduledMessageMaxPerChat {
			log.Printf("Dropping scheduled message %s: the chat is at the scheduled message limit", v.Info.ID)
			return true
		}
		if err := acc.storeScheduled(rec); err != nil {
			log.Printf("Couldn't store scheduled message %s: %v", v.Info.ID, err)
			return false
		}
		acc.noteScheduledCreated(rec.ChatJID, rec.MsgID)
		acc.scheduledChanged(rec.ChatJID)
		return true
	}
	var orphanKey []byte
	if err := acc.db.QueryRow(`SELECT reveal_key FROM scheduled_orphan_keys WHERE reveal_key_id = ?`, rkid).Scan(&orphanKey); err == nil && len(orphanKey) > 0 {
		if inner, err := whatsmeow.DecryptScheduledMessage(crm, orphanKey); err == nil {
			acc.db.Exec(`DELETE FROM scheduled_orphan_keys WHERE reveal_key_id = ?`, rkid)
			revealed := &events.Message{Info: v.Info, RawMessage: inner}
			revealed.UnwrapRaw()
			acc.eventHandler(revealed)
			return true
		} else {
			log.Printf("Orphan reveal key didn't open scheduled message %s: %v", v.Info.ID, err)
		}
	}
	rec.FromMe = v.Info.IsFromMe
	if !v.Info.IsFromMe {
		rec.SenderJID = sender.String()
	}
	if meta := v.Info.MsgMetaInfo.ScheduledMessage; meta != nil && meta.RevealKeyID == rkid {
		rec.ScheduledAt = meta.Time.Unix()
	}
	if err := acc.storeScheduled(rec); err != nil {
		log.Printf("Couldn't store scheduled message %s: %v", v.Info.ID, err)
		return false
	}
	return true
}

// handleUnscheduleMessage is WA Web's WAWebScheduledMsgUnscheduleHandler: the
// scheduled copy goes, and a message that was already shown is deleted.
func (acc *Account) handleUnscheduleMessage(v *events.Message, pm *waE2E.ProtocolMessage) {
	targetID := pm.GetKey().GetID()
	if targetID == "" {
		return
	}
	chat, _ := acc.resolveLIDChat(v)
	rec := acc.scheduledBy("msg_id", targetID)
	if rec != nil {
		acc.deleteScheduled(rec.MsgID)
		acc.scheduledChanged(rec.ChatJID)
	}
	if _, _, _, _, err := acc.getMessageRaw(targetID); err == nil {
		acc.markMessageDeleted(targetID, chat.String())
		acc.broadcastEvent("message_deleted", map[string]interface{}{
			"jid": chat.String(), "message_id": targetID, "by": v.Info.Sender.String(),
		})
	}
}

// revealedMessageEvent builds the message a scheduled one turns into, to go
// through the normal message handling.
func (acc *Account) revealedMessageEvent(rec *scheduledRecord, inner *waE2E.Message, at time.Time, fromMe bool) *events.Message {
	chat, _ := types.ParseJID(rec.ChatJID)
	info := types.MessageInfo{
		MessageSource: types.MessageSource{Chat: chat, IsFromMe: fromMe, IsGroup: chat.Server == types.GroupServer},
		ID:            rec.MsgID,
		Timestamp:     at,
		Type:          "text",
	}
	switch {
	case fromMe:
		info.Sender = acc.wa().Store.GetJID().ToNonAD()
	case rec.SenderJID != "":
		info.Sender, _ = types.ParseJID(rec.SenderJID)
	default:
		info.Sender = chat
	}
	evt := &events.Message{Info: info, RawMessage: inner}
	return evt.UnwrapRaw()
}

// handleScheduledReveal is WA Web's WAWebScheduledMsgRevealNotificationHandler.
func (acc *Account) handleScheduledReveal(v *events.ScheduledMessageReveal) {
	rec := acc.scheduledBy("reveal_key_id", v.RevealKeyID)
	if rec == nil {
		if _, err := acc.db.Exec(`INSERT OR REPLACE INTO scheduled_orphan_keys (reveal_key_id, reveal_key, created_at) VALUES (?, ?, ?)`,
			v.RevealKeyID, v.RevealKey, time.Now().Unix()); err != nil {
			log.Printf("Couldn't store orphan reveal key: %v", err)
		}
		return
	}
	if !v.Chat.IsEmpty() && !acc.sameChat(v.Chat, rec) {
		log.Printf("Reveal key for scheduled message %s names another chat; ignoring it", rec.MsgID)
		return
	}
	if rec.FromMe || rec.SenderJID == "" {
		// Ours: the post notification handles it.
		return
	}
	if rec.Status == scheduledRevealed {
		acc.deleteScheduled(rec.MsgID)
		return
	}
	rec.RevealKey = v.RevealKey
	inner, err := rec.decrypt()
	if err != nil {
		log.Printf("Couldn't reveal scheduled message %s: %v", rec.MsgID, err)
		acc.deleteScheduled(rec.MsgID)
		return
	}
	at := v.Time
	if at.IsZero() && rec.ScheduledAt > 0 {
		at = time.Unix(rec.ScheduledAt, 0)
	} else if at.IsZero() {
		at = time.Now()
	}
	acc.eventHandler(acc.revealedMessageEvent(rec, inner, at, false))
	acc.deleteScheduled(rec.MsgID)
}

func (acc *Account) sameChat(notified types.JID, rec *scheduledRecord) bool {
	for _, known := range []string{rec.ChatJID, rec.WireChat} {
		if known == "" {
			continue
		}
		jid, err := types.ParseJID(known)
		if err != nil {
			continue
		}
		if jid.ToNonAD() == notified.ToNonAD() {
			return true
		}
		if notified.Server == types.HiddenUserServer && jid.Server == types.DefaultUserServer {
			if pn, err := acc.wa().Store.LIDs.GetPNForLID(context.Background(), notified); err == nil && pn.User == jid.User {
				return true
			}
		}
	}
	return false
}

// handleScheduledPost is WA Web's WAWebScheduledMsgPostNotificationHandler:
// one of ours was posted (it becomes a normal sent message) or failed.
func (acc *Account) handleScheduledPost(v *events.ScheduledMessagePost) {
	rec := acc.scheduledBy("reveal_key_id", v.RevealKeyID)
	if rec == nil {
		return
	}
	if !v.IsSuccess() {
		log.Printf("Scheduled message %s wasn't posted: %s", rec.MsgID, v.Status)
		acc.db.Exec(`UPDATE scheduled_messages SET status = ? WHERE msg_id = ?`, scheduledFailed, rec.MsgID)
		acc.scheduledChanged(rec.ChatJID)
		return
	}
	inner, err := rec.decrypt()
	if err != nil {
		log.Printf("Scheduled message %s was posted but can't be decrypted here: %v", rec.MsgID, err)
		return
	}
	at := time.Now()
	if rec.ScheduledAt > 0 {
		at = time.Unix(rec.ScheduledAt, 0)
	}
	acc.eventHandler(acc.revealedMessageEvent(rec, inner, at, true))
	acc.deleteScheduled(rec.MsgID)
	acc.scheduledChanged(rec.ChatJID)
}

// parkHistoryScheduled is what WA Web's parseWebMessageInfo does with a
// scheduled message in a history sync: ours (with its reveal key in
// scheduledMessageMetadata) goes back on the scheduled list, someone else's
// waits for its reveal key. Either way it isn't shown in the chat.
func (acc *Account) parkHistoryScheduled(chat types.JID, webMsg *waWeb.WebMessageInfo, evt *events.Message) bool {
	if meta := webMsg.GetScheduledMessageMetadata(); meta != nil && webMsg.GetKey().GetFromMe() {
		if meta.RevealKey == nil || meta.RevealKeyID == nil || meta.ScheduledTime == nil {
			return false
		}
		inner := historyScheduledInner(evt.Message)
		plaintext, err := proto.Marshal(inner)
		if err != nil {
			return false
		}
		iv, payload, err := whatsmeow.EncryptWithRevealKey(plaintext, meta.GetRevealKey())
		if err != nil {
			log.Printf("Couldn't re-encrypt scheduled message %s from history: %v", evt.Info.ID, err)
			return false
		}
		chatPN, _ := acc.resolveLIDChat(evt)
		err = acc.storeScheduled(scheduledRecord{
			MsgID: evt.Info.ID, ChatJID: chatPN.String(), WireChat: chat.ToNonAD().String(), FromMe: true,
			RevealKeyID: meta.GetRevealKeyID(), RevealKey: meta.GetRevealKey(), EncPayload: payload, EncIV: iv,
			ScheduledAt: int64(meta.GetScheduledTime()),
		})
		if err != nil {
			log.Printf("Couldn't store scheduled message %s from history: %v", evt.Info.ID, err)
		}
		return true
	}
	crm := unwrapMessage(evt.Message).GetConditionalRevealMessage()
	if crm == nil || webMsg.GetKey().GetFromMe() || crm.GetEncPayload() == nil || crm.GetEncIV() == nil || crm.GetRevealKeyID() == "" {
		return false
	}
	chatPN, sender := acc.resolveLIDChat(evt)
	err := acc.storeScheduled(scheduledRecord{
		MsgID: evt.Info.ID, ChatJID: chatPN.String(), WireChat: chat.ToNonAD().String(), SenderJID: sender.String(),
		RevealKeyID: crm.GetRevealKeyID(), EncPayload: crm.GetEncPayload(), EncIV: crm.GetEncIV(),
	})
	if err != nil {
		log.Printf("Couldn't park scheduled message %s from history: %v", evt.Info.ID, err)
	}
	return true
}

// historyScheduledInner is WA Web's buildHistorySyncInnerProto: the photo or
// video as it is, or the text (with the chat's disappearing timer).
func historyScheduledInner(msg *waE2E.Message) *waE2E.Message {
	msg = unwrapMessage(msg)
	if msg.GetImageMessage() != nil {
		return &waE2E.Message{ImageMessage: msg.GetImageMessage()}
	}
	if msg.GetVideoMessage() != nil {
		return &waE2E.Message{VideoMessage: msg.GetVideoMessage()}
	}
	text, _ := whatsmeow.ExtractScheduledMessageText(msg)
	if ctx := msg.GetExtendedTextMessage().GetContextInfo(); ctx.GetExpiration() > 0 || ctx.EphemeralSettingTimestamp != nil {
		return &waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{
			Text: proto.String(text),
			ContextInfo: &waE2E.ContextInfo{
				Expiration:                ctx.Expiration,
				EphemeralSettingTimestamp: ctx.EphemeralSettingTimestamp,
				DisappearingMode:          ctx.DisappearingMode,
			},
		}}
	}
	return &waE2E.Message{Conversation: proto.String(text)}
}

// scheduledMessagesEnabled is whether to offer scheduling: WA Web shows it
// when the server's scheduled_messages_sender_enabled flag is on. Until the
// flags have been read, or when the server doesn't send that one, it's
// offered anyway and the server's answer is shown if it refuses.
func (acc *Account) scheduledMessagesEnabled() bool {
	acc.serverFlagsMu.RLock()
	defer acc.serverFlagsMu.RUnlock()
	value, ok := acc.serverFlags[whatsmeow.ABPropScheduledMessagesSenderEnabled]
	if !ok {
		return true
	}
	return value == "1" || value == "true"
}

func (acc *Account) scheduledConfig() whatsmeow.ScheduledMessagesConfig {
	return acc.wa().ScheduledMessagesConfig()
}

type scheduledChatData struct {
	JID       string `json:"jid"`
	MessageID string `json:"message_id"`
}

func (acc *Account) scheduledResult(conn net.Conn, chat, action string, err error, done string) {
	payload := map[string]interface{}{"jid": chat, "action": action, "ok": err == nil}
	if err != nil {
		payload["message"] = err.Error()
	} else {
		payload["message"] = done
	}
	acc.sendEvent(conn, "scheduled_result", payload)
}

// plainScheduleError turns a send error into WA Web's words.
func plainScheduleError(err error) error {
	switch {
	case errors.Is(err, whatsmeow.ErrScheduledMessageLimitReached):
		return errors.New("You have reached the maximum number of messages you can schedule.")
	case errors.Is(err, whatsmeow.ErrScheduledMessageUnsupported):
		return fmt.Errorf("Your message can't be scheduled: %s.", strings.TrimPrefix(err.Error(), whatsmeow.ErrScheduledMessageUnsupported.Error()+": "))
	case errors.Is(err, whatsmeow.ErrServerReturnedError):
		return fmt.Errorf("Your message can't be scheduled. WhatsApp refused it (%v).", err)
	default:
		return fmt.Errorf("Your message can't be scheduled: %v.", err)
	}
}

func scheduleWindowError(cfg whatsmeow.ScheduledMessagesConfig) error {
	minutes := int(cfg.MinWindow / time.Minute)
	days := int(cfg.MaxWindow / (24 * time.Hour))
	return fmt.Errorf("Choose a time at least %d minutes from now and no more than %d days ahead.", minutes, days)
}

// handleGetScheduleOptions gives the schedule dialog what WA Web's
// WAWebScheduleTimePicker works out: the days it offers, the time it starts
// on and the window a time has to be in.
func (acc *Account) handleGetScheduleOptions(conn net.Conn, raw json.RawMessage) {
	var data scheduledChatData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	cfg := acc.scheduledConfig()
	now := time.Now()
	def := whatsmeow.DefaultScheduledMessageTime(now, cfg.MinWindow, cfg.MaxWindow, cfg.SmartDefaults).Local()
	minuteStart := now.Truncate(time.Minute)
	earliest := minuteStart.Add(cfg.MinWindow)
	if earliest.Truncate(time.Minute) != earliest {
		earliest = earliest.Truncate(time.Minute).Add(time.Minute)
	}
	latest := now.Add(cfg.MaxWindow).Truncate(time.Minute)
	type day struct {
		Value string `json:"value"`
		Label string `json:"label"`
	}
	var days []day
	y, m, d := now.Date()
	for i := 0; ; i++ {
		date := time.Date(y, m, d+i, 0, 0, 0, 0, time.Local)
		if date.After(latest) {
			break
		}
		label := date.Format("Monday 2 January")
		switch i {
		case 0:
			label = "Today, " + label
		case 1:
			label = "Tomorrow, " + label
		}
		days = append(days, day{Value: date.Format("2006-01-02"), Label: label})
	}
	jid, _ := types.ParseJID(data.JID)
	acc.sendEvent(conn, "schedule_options", map[string]interface{}{
		"jid":          data.JID,
		"enabled":      acc.scheduledMessagesEnabled(),
		"is_group":     jid.Server == types.GroupServer,
		"days":         days,
		"default_date": def.Format("2006-01-02"),
		"default_time": def.Format("15:04"),
		"earliest":     earliest.Local().Format("2006-01-02 15:04"),
		"latest":       latest.Local().Format("2006-01-02 15:04"),
		"window_error": scheduleWindowError(cfg).Error(),
		"pending":      acc.pendingScheduledCount(data.JID),
		"limit":        whatsmeow.ScheduledMessageMaxPerChat,
	})
}

// handleScheduleMessage is WA Web's sendScheduledTextMsgToChat.
func (acc *Account) handleScheduleMessage(conn net.Conn, raw json.RawMessage) {
	var data struct {
		JID      string       `json:"jid"`
		Text     string       `json:"text"`
		Date     string       `json:"date"`
		Time     string       `json:"time"`
		Mentions []MentionRef `json:"mentions"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	fail := func(err error) { acc.scheduledResult(conn, data.JID, "schedule", err, "") }
	jid, err := types.ParseJID(data.JID)
	if err != nil || !schedulableChat(jid) {
		fail(errors.New("Messages can't be scheduled in this chat."))
		return
	}
	if strings.TrimSpace(data.Text) == "" {
		fail(errors.New("Type a message to schedule."))
		return
	}
	at, err := time.ParseInLocation("2006-01-02 15:04", data.Date+" "+data.Time, time.Local)
	if err != nil {
		fail(errors.New("Choose a date and a time for the message."))
		return
	}
	cfg := acc.scheduledConfig()
	if !whatsmeow.IsScheduledTimeValid(at, time.Now(), cfg.MinWindow, cfg.MaxWindow) {
		fail(scheduleWindowError(cfg))
		return
	}
	if acc.pendingScheduledCount(data.JID) >= whatsmeow.ScheduledMessageMaxPerChat {
		fail(errors.New("You have reached the maximum number of messages you can schedule."))
		return
	}
	var msg *waE2E.Message
	if wireText, mentionJIDs := applyMentions(data.Text, data.Mentions); len(mentionJIDs) > 0 {
		msg = &waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{
			Text:        proto.String(wireText),
			ContextInfo: &waE2E.ContextInfo{MentionedJID: jidStrings(mentionJIDs)},
		}}
	} else {
		msg = &waE2E.Message{Conversation: proto.String(data.Text)}
	}
	msg = acc.applyDisappearingTimer(jid, msg)

	ctx, cancel := context.WithTimeout(context.Background(), 75*time.Second)
	defer cancel()
	resp, err := acc.wa().SendScheduledMessage(ctx, jid, msg, at)
	if err != nil {
		log.Printf("Scheduling a message in %s failed: %v", jid, err)
		fail(plainScheduleError(err))
		return
	}
	crm := resp.Wrapped.GetConditionalRevealMessage()
	err = acc.storeScheduled(scheduledRecord{
		MsgID: resp.ID, ChatJID: data.JID, WireChat: resp.Chat.ToNonAD().String(), FromMe: true,
		RevealKeyID: resp.RevealKeyID, RevealKey: resp.RevealKey,
		EncPayload: crm.GetEncPayload(), EncIV: crm.GetEncIV(), ScheduledAt: resp.Time.Unix(),
	})
	if err != nil {
		log.Printf("Couldn't store scheduled message %s: %v", resp.ID, err)
	}
	acc.noteScheduledCreated(data.JID, resp.ID)
	acc.scheduledResult(conn, data.JID, "schedule", nil, "Message scheduled for "+scheduledWhen(at, time.Now())+".")
	acc.scheduledChanged(data.JID)
}

// handleUnscheduleMessage is WA Web's unscheduleMsgAction.
func (acc *Account) handleUnscheduleAction(conn net.Conn, raw json.RawMessage) {
	var data scheduledChatData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	rec := acc.scheduledBy("msg_id", data.MessageID)
	if rec == nil {
		acc.scheduledResult(conn, data.JID, "unschedule", errors.New("That scheduled message is already gone."), "")
		acc.scheduledChanged(data.JID)
		return
	}
	if err := acc.unscheduleRecord(rec); err != nil {
		acc.scheduledResult(conn, data.JID, "unschedule", err, "")
		return
	}
	acc.scheduledResult(conn, data.JID, "unschedule", nil, "Scheduled message deleted.")
	acc.scheduledChanged(rec.ChatJID)
}

func (acc *Account) unscheduleRecord(rec *scheduledRecord) error {
	wire := rec.WireChat
	if wire == "" {
		wire = rec.ChatJID
	}
	chat, err := types.ParseJID(wire)
	if err != nil {
		return errors.New("Couldn't delete scheduled message.")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 75*time.Second)
	defer cancel()
	if _, err = acc.wa().UnscheduleMessage(ctx, chat, rec.MsgID, rec.RevealKeyID); err != nil {
		log.Printf("Unscheduling %s failed: %v", rec.MsgID, err)
		return errors.New("Couldn't delete scheduled message. Connect to the internet and try again.")
	}
	acc.deleteScheduled(rec.MsgID)
	return nil
}

// handleSendScheduledNow is WA Web's sendScheduledMsgNow: the text goes out
// as a normal message and the scheduled copy is dropped. WA Web offers it
// for one that failed; for one still waiting, it's unscheduled first so it
// doesn't arrive twice.
func (acc *Account) handleSendScheduledNow(conn net.Conn, raw json.RawMessage) {
	var data scheduledChatData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	rec := acc.scheduledBy("msg_id", data.MessageID)
	if rec == nil {
		acc.scheduledResult(conn, data.JID, "send_now", errors.New("That scheduled message is already gone."), "")
		return
	}
	inner, err := rec.decrypt()
	text := ""
	if err == nil {
		text, _ = whatsmeow.ExtractScheduledMessageText(inner)
	}
	if text == "" {
		acc.scheduledResult(conn, data.JID, "send_now", errors.New("Couldn't recover this message's text to send it now."), "")
		return
	}
	if rec.Status == scheduledPending && rec.ScheduledAt > time.Now().Unix() {
		if err := acc.unscheduleRecord(rec); err != nil {
			acc.scheduledResult(conn, data.JID, "send_now", err, "")
			return
		}
	} else {
		acc.deleteScheduled(rec.MsgID)
	}
	sendData, _ := json.Marshal(SendCommandData{To: rec.ChatJID, Text: text})
	acc.handleSend(conn, sendData)
	acc.scheduledResult(conn, data.JID, "send_now", nil, "Message sent.")
	acc.scheduledChanged(rec.ChatJID)
}

func (acc *Account) handleGetScheduledMessages(conn net.Conn, raw json.RawMessage) {
	var data scheduledChatData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	acc.sendEvent(conn, "scheduled_messages", acc.scheduledListPayload(data.JID))
}

func (acc *Account) scheduledListPayload(chat string) map[string]interface{} {
	now := time.Now()
	entries := []map[string]interface{}{}
	for _, rec := range acc.scheduledForChat(chat) {
		inner, err := rec.decrypt()
		if err != nil {
			continue
		}
		kind, text := "text", ""
		switch {
		case inner.GetImageMessage() != nil:
			kind, text = "image", inner.GetImageMessage().GetCaption()
		case inner.GetVideoMessage() != nil:
			kind, text = "video", inner.GetVideoMessage().GetCaption()
		default:
			var ok bool
			if text, ok = whatsmeow.ExtractScheduledMessageText(inner); !ok {
				continue
			}
		}
		at := time.Unix(rec.ScheduledAt, 0)
		entries = append(entries, map[string]interface{}{
			"id":           rec.MsgID,
			"text":         text,
			"kind":         kind,
			"scheduled_at": rec.ScheduledAt,
			"when":         scheduledWhen(at, now),
			"failed":       rec.Status == scheduledFailed,
			"past":         at.Before(now),
		})
	}
	return map[string]interface{}{
		"jid":      chat,
		"messages": entries,
		"pending":  acc.pendingScheduledCount(chat),
		"limit":    whatsmeow.ScheduledMessageMaxPerChat,
	}
}
