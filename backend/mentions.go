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

type MentionRef struct {
	JID  string `json:"jid"`
	Name string `json:"name"`
}

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

func invokedBot(jids []types.JID) types.JID {
	for _, j := range jids {
		if j.IsBot() {
			return j
		}
	}
	return types.EmptyJID
}

func (acc *Account) storedMentionMessage(id string) (string, []types.JID) {
	var raw []byte
	acc.db.QueryRow(`SELECT raw FROM messages WHERE id = ?`, id).Scan(&raw)
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

func (acc *Account) editMentions(messageID string, picked []MentionRef) []MentionRef {
	var raw []byte
	acc.db.QueryRow(`SELECT raw FROM messages WHERE id = ?`, messageID).Scan(&raw)
	msg := &waE2E.Message{}
	if len(raw) == 0 || proto.Unmarshal(raw, msg) != nil {
		return picked
	}
	_, jids := mentionsInMessage(msg)
	out := make([]MentionRef, 0, len(jids)+len(picked))
	for _, jid := range jids {
		out = append(out, MentionRef{JID: jid.String(), Name: acc.resolveDisplayName(jid)})
	}
	return append(out, picked...)
}

func (acc *Account) chatLimitSharing(jid string) bool {
	var on int
	acc.db.QueryRow(`SELECT COALESCE(limit_sharing, 0) FROM chats WHERE jid = ?`, jid).Scan(&on)
	return on == 1
}

func (acc *Account) setChatLimitSharing(jid string, enabled bool, ts int64) (changed bool) {
	var res interface{ RowsAffected() (int64, error) }
	var err error
	if ts == 0 {
		res, err = acc.db.Exec(`UPDATE chats SET limit_sharing = ? WHERE jid = ? AND COALESCE(limit_sharing, 0) != ?`,
			boolToInt(enabled), jid, boolToInt(enabled))
	} else {
		res, err = acc.db.Exec(`UPDATE chats SET limit_sharing = ?, limit_sharing_ts = ? WHERE jid = ? AND COALESCE(limit_sharing_ts, 0) < ?`,
			boolToInt(enabled), ts, jid, ts)
	}
	if err != nil {
		return false
	}
	if n, _ := res.RowsAffected(); n > 0 {
		acc.broadcastEvent("chat_limit_sharing", map[string]interface{}{"jid": jid, "enabled": acc.chatSharingLimited(jid)})
		return true
	}
	return false
}

func (acc *Account) chatRestricted(jid string) bool {
	var on int
	acc.db.QueryRow(`SELECT COALESCE(restricted, 0) FROM chats WHERE jid = ?`, jid).Scan(&on)
	return on == 1
}

func (acc *Account) chatSharingLimited(jid string) bool {
	return acc.chatLimitSharing(jid) || acc.chatRestricted(jid)
}

func (acc *Account) setChatRestricted(jid string, enabled bool, ts int64) (changed bool) {
	res, err := acc.db.Exec(`UPDATE chats SET restricted = ?, restricted_ts = ? WHERE jid = ? AND COALESCE(restricted_ts, 0) < ?`,
		boolToInt(enabled), ts, jid, ts)
	if err != nil {
		return false
	}
	if n, _ := res.RowsAffected(); n > 0 {
		acc.broadcastEvent("chat_limit_sharing", map[string]interface{}{"jid": jid, "enabled": acc.chatSharingLimited(jid)})
		return true
	}
	return false
}

func (acc *Account) chatPrivacyName(jid string) string {
	if acc.chatRestricted(jid) {
		return "Restricted chat"
	}
	return "Advanced chat privacy"
}

func acp2InMessage(msg *waE2E.Message) (setting *waCommon.ACP2Setting, toggle bool) {
	if msg == nil {
		return nil, false
	}
	pm := msg.GetAcp2SettingMessage().GetMessage().GetProtocolMessage()
	if pm.GetAcp2Setting() == nil {
		pm = unwrapMessage(msg).GetProtocolMessage()
	}
	if pm.GetType() == waE2E.ProtocolMessage_ACP2_SETTING && pm.GetAcp2Setting() != nil {
		return pm.GetAcp2Setting(), true
	}
	if stamp := msg.GetMessageContextInfo().GetAcp2Setting(); stamp != nil {
		return stamp, false
	}
	return nil, false
}

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
func (acc *Account) handleGetMentionCandidates(conn net.Conn, raw json.RawMessage) {
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
		// The Meta AI to mention: in a group with private processing, Meta AI
		// is a member under its own ID, and WA Web mentions that member.
		metaAI := types.NewMetaAIJID
		if jid.Server == types.GroupServer {
			info, err := acc.wa().GetGroupInfo(context.Background(), jid)
			if err != nil {
				acc.sendEvent(conn, "error", map[string]string{"message": "Couldn't load the group's members to mention: " + err.Error()})
			} else {
				acc.setChatLimitSharing(jid.String(), info.IsLimitSharingEnabled, 0)
				bots := map[types.JID]bool{}
				for _, p := range info.Participants {
					if p.JID.IsBot() {
						bots[p.JID.ToNonAD()] = true
						continue
					}
					if acc.isParticipantMe(p) {
						continue
					}
					name := acc.resolveDisplayName(p.JID)
					if name == "" || strings.HasSuffix(name, "@"+types.HiddenUserServer) {
						name = p.DisplayName
					}
					if name == "" {
						continue
					}
					members = append(members, map[string]string{"jid": p.JID.ToNonAD().String(), "name": name})
				}
				if bots[types.TeeMetaAIJID] && !bots[types.NewMetaAIJID] {
					metaAI = types.TeeMetaAIJID
				}
			}
		}
		acc.sendEvent(conn, "mention_candidates", map[string]interface{}{
			"jid":           jid.String(),
			"members":       members,
			"limit_sharing": acc.chatSharingLimited(jid.String()),
			"meta_ai":       metaAI.String(),
		})
	}()
}
