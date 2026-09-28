package main

import (
	"context"
	"encoding/json"
	"net"
	"sort"
	"strings"
	"unicode"
	"unicode/utf8"

	"go.mau.fi/whatsmeow/proto/waCommon"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
)

// MentionRef is one @mention picked in the composer: the name it shows as in
// the typed text ("@Name") and the JID it stands for.
type MentionRef struct {
	JID  string `json:"jid"`
	Name string `json:"name"`
}

// applyMentions turns composer text into wire text. Each picked "@Name" still
// in the text becomes "@<JID user>" - what WhatsApp puts in the body, with the
// full JID in ContextInfo.MentionedJID - and the JIDs it found come back in
// the order they were picked. A mention whose "@Name" was since deleted or
// retyped is simply dropped, leaving whatever text is there as plain text.
func applyMentions(text string, mentions []MentionRef) (string, []types.JID) {
	type pick struct {
		token, wire string
		jid         types.JID
	}
	var picks []pick
	for _, m := range mentions {
		name := strings.TrimSpace(m.Name)
		jid, err := types.ParseJID(m.JID)
		if name == "" || err != nil || jid.User == "" {
			continue
		}
		picks = append(picks, pick{token: "@" + name, wire: "@" + jid.User, jid: jid.ToNonAD()})
	}
	// Longest name first, so "@Ana" can't eat the front of "@Ana Maria".
	byLength := make([]pick, len(picks))
	copy(byLength, picks)
	sort.SliceStable(byLength, func(i, j int) bool { return len(byLength[i].token) > len(byLength[j].token) })
	found := map[types.JID]bool{}
	for _, p := range byLength {
		var replaced bool
		text, replaced = replaceWholeToken(text, p.token, p.wire)
		if replaced {
			found[p.jid] = true
		}
	}
	var jids []types.JID
	for _, p := range picks {
		if found[p.jid] {
			jids = append(jids, p.jid)
			delete(found, p.jid)
		}
	}
	return text, jids
}

// replaceWholeToken replaces every occurrence of token that isn't followed
// straight away by a letter or digit ("@Ana" in "@Anabel" is someone else).
func replaceWholeToken(text, token, replacement string) (string, bool) {
	var b strings.Builder
	replaced := false
	for {
		i := strings.Index(text, token)
		if i == -1 {
			b.WriteString(text)
			break
		}
		end := i + len(token)
		if next, _ := utf8.DecodeRuneInString(text[end:]); end < len(text) && (unicode.IsLetter(next) || unicode.IsDigit(next)) {
			b.WriteString(text[:end])
			text = text[end:]
			continue
		}
		b.WriteString(text[:i])
		b.WriteString(replacement)
		text = text[end:]
		replaced = true
	}
	return b.String(), replaced
}

func jidStrings(jids []types.JID) []string {
	out := make([]string, len(jids))
	for i, j := range jids {
		out[i] = j.String()
	}
	return out
}

// invokedBot is the bot (Meta AI) a message mentions, if any. WhatsApp Web
// finds it the same way (mentionedJidList.find(isBot)); the message then
// also goes to the bot, which is how it answers in the chat.
func invokedBot(jids []types.JID) types.JID {
	for _, j := range jids {
		if j.IsBot() {
			return j
		}
	}
	return types.EmptyJID
}

// storedMentionMessage is the wire text and mentions a pending text message
// was stored with (see handleSend), so a retry sends the same mentions.
func storedMentionMessage(id string) (string, []types.JID) {
	var raw []byte
	db.QueryRow(`SELECT raw FROM messages WHERE id = ?`, id).Scan(&raw)
	if len(raw) == 0 {
		return "", nil
	}
	msg := &waE2E.Message{}
	if proto.Unmarshal(raw, msg) != nil {
		return "", nil
	}
	return mentionsInMessage(msg)
}

func mentionsInMessage(msg *waE2E.Message) (string, []types.JID) {
	ext := unwrapMessage(msg).GetExtendedTextMessage()
	var jids []types.JID
	for _, raw := range ext.GetContextInfo().GetMentionedJID() {
		if jid, err := types.ParseJID(raw); err == nil && jid.User != "" {
			jids = append(jids, jid)
		}
	}
	if len(jids) == 0 {
		return "", nil
	}
	return ext.GetText(), jids
}

// editMentions keeps the mentions an edited message already had: each one is
// shown in the stored text as "@<resolved name>" (resolveMentions), so while
// that is still in the new text it's still a mention. Mentions picked during
// the edit come after.
func editMentions(messageID string, picked []MentionRef) []MentionRef {
	var raw []byte
	db.QueryRow(`SELECT raw FROM messages WHERE id = ?`, messageID).Scan(&raw)
	msg := &waE2E.Message{}
	if len(raw) == 0 || proto.Unmarshal(raw, msg) != nil {
		return picked
	}
	_, jids := mentionsInMessage(msg)
	out := make([]MentionRef, 0, len(jids)+len(picked))
	for _, jid := range jids {
		out = append(out, MentionRef{JID: jid.String(), Name: resolveDisplayName(jid)})
	}
	return append(out, picked...)
}

// --- Advanced chat privacy (WhatsApp's "limit sharing") ---
//
// While it's on in a chat, nobody in it can mention Meta AI there (WA Web's
// isLimitSharingReceiverEnabledForUsers blocks the send). A group's setting
// comes with its metadata and a mex notification; a 1:1 chat's comes as a
// LIMIT_SHARING protocol message, a limitSharingV2 stamp on any message, or
// history sync.

func chatLimitSharing(jid string) bool {
	var on int
	db.QueryRow(`SELECT COALESCE(limit_sharing, 0) FROM chats WHERE jid = ?`, jid).Scan(&on)
	return on == 1
}

// setChatLimitSharing records a chat's setting. ts is the setting's own
// timestamp; a nonzero one is only applied when newer than what's stored
// (WAWebLimitSharingProtoUtils.updateChatWithLimitSharingIfNewer). Zero means
// authoritative (group metadata), applied as is.
func setChatLimitSharing(jid string, enabled bool, ts int64) {
	var res interface{ RowsAffected() (int64, error) }
	var err error
	if ts == 0 {
		res, err = db.Exec(`UPDATE chats SET limit_sharing = ? WHERE jid = ? AND COALESCE(limit_sharing, 0) != ?`,
			boolToInt(enabled), jid, boolToInt(enabled))
	} else {
		res, err = db.Exec(`UPDATE chats SET limit_sharing = ?, limit_sharing_ts = ? WHERE jid = ? AND COALESCE(limit_sharing_ts, 0) < ?`,
			boolToInt(enabled), ts, jid, ts)
	}
	if err != nil {
		return
	}
	if n, _ := res.RowsAffected(); n > 0 {
		broadcastEvent("chat_limit_sharing", map[string]interface{}{"jid": jid, "enabled": enabled})
	}
}

// limitSharingInMessage finds a chat-privacy setting a message carries: a
// LIMIT_SHARING protocol message (the toggle itself, possibly wrapped in
// limitSharingMessage) or the limitSharingV2 stamp WA Web reads off any
// message. toggle is true for the former, which has nothing else to show.
func limitSharingInMessage(msg *waE2E.Message) (setting *waCommon.LimitSharing, toggle bool) {
	if msg == nil {
		return nil, false
	}
	pm := msg.GetLimitSharingMessage().GetMessage().GetProtocolMessage()
	if pm.GetLimitSharing() == nil {
		pm = unwrapMessage(msg).GetProtocolMessage()
	}
	if pm.GetType() == waE2E.ProtocolMessage_LIMIT_SHARING && pm.GetLimitSharing() != nil {
		return pm.GetLimitSharing(), true
	}
	if v2 := msg.GetMessageContextInfo().GetLimitSharingV2(); v2 != nil {
		return v2, false
	}
	return nil, false
}

// handleGetMentionCandidates answers the composer's first "@" in a chat: the
// group's members (never us) and whether chat privacy is on.
func handleGetMentionCandidates(conn net.Conn, raw json.RawMessage) {
	var data JIDData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}
	go func() {
		members := []map[string]string{}
		if jid.Server == types.GroupServer {
			info, err := client.GetGroupInfo(context.Background(), jid)
			if err != nil {
				sendEvent(conn, "error", map[string]string{"message": "Couldn't load the group's members to mention: " + err.Error()})
			} else {
				setChatLimitSharing(jid.String(), info.IsLimitSharingEnabled, 0)
				for _, p := range info.Participants {
					if isParticipantMe(p) {
						continue
					}
					name := resolveDisplayName(p.JID)
					if name == "" || strings.HasSuffix(name, "@"+types.HiddenUserServer) {
						name = p.DisplayName
					}
					if name == "" {
						continue
					}
					members = append(members, map[string]string{"jid": p.JID.ToNonAD().String(), "name": name})
				}
			}
		}
		sendEvent(conn, "mention_candidates", map[string]interface{}{
			"jid":           jid.String(),
			"members":       members,
			"limit_sharing": chatLimitSharing(jid.String()),
		})
	}()
}
