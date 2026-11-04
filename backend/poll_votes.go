package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"log"
	"math"
	"net"
	"strings"
	"sync"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/proto/waWeb"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

type poll struct {
	question string
	options  []string
	multiple bool
	quiz     bool
	correct  string
	fromMe   bool
}

func loadPoll(pollID string) (poll, bool) {
	var raw []byte
	var fromMe bool
	if db.QueryRow(`SELECT raw, from_me FROM messages WHERE id = ?`, pollID).Scan(&raw, &fromMe) != nil || len(raw) == 0 {
		return poll{}, false
	}
	msg := &waE2E.Message{}
	if proto.Unmarshal(raw, msg) != nil {
		return poll{}, false
	}
	p, ok := pollFromMessage(unwrapMessage(msg))
	p.fromMe = fromMe
	return p, ok
}

func pollFromMessage(msg *waE2E.Message) (poll, bool) {
	pc := pollCreation(msg)
	if pc == nil {
		return poll{}, false
	}
	p := poll{question: pc.GetName(), multiple: pc.GetSelectableOptionsCount() != 1}
	for _, opt := range pc.GetOptions() {
		p.options = append(p.options, opt.GetOptionName())
	}
	if pc.GetPollType() == waE2E.PollType_QUIZ {
		p.quiz, p.multiple = true, false
		p.correct = pc.GetCorrectAnswer().GetOptionName()
		if hash := pc.GetCorrectAnswer().GetOptionHash(); p.correct == "" && hash != "" {
			for _, name := range p.options {
				if optionHash(name) == hash {
					p.correct = name
				}
			}
		}
	}
	return p, true
}

func quizAnswerText(p poll, mine []string) string {
	if !p.quiz || p.correct == "" || (!p.fromMe && len(mine) == 0) {
		return ""
	}
	if !p.fromMe {
		for _, h := range mine {
			if h == optionHash(p.correct) {
				return " You got it right: the answer is " + p.correct + "."
			}
		}
	}
	return " The correct answer is " + p.correct + "."
}

func optionHash(name string) string {
	sum := sha256.Sum256([]byte(name))
	return hex.EncodeToString(sum[:])
}

func pollVoterKey(jid types.JID) string {
	return preferPN(jid.ToNonAD()).String()
}

func ownVoterKey() string {
	if wa() == nil || wa().Store.ID == nil {
		return ""
	}
	return wa().Store.ID.ToNonAD().String()
}

func storePollVote(exec sqlExecer, pollID, voter string, selected [][]byte, ts int64) {
	if pollID == "" || voter == "" {
		return
	}
	hashes := make([]string, 0, len(selected))
	for _, h := range selected {
		hashes = append(hashes, hex.EncodeToString(h))
	}
	if _, err := exec.Exec(`INSERT INTO poll_votes (poll_id, voter, selected, timestamp) VALUES (?, ?, ?, ?)
		ON CONFLICT(poll_id, voter) DO UPDATE SET selected=excluded.selected, timestamp=excluded.timestamp
		WHERE excluded.timestamp >= poll_votes.timestamp`,
		pollID, voter, strings.Join(hashes, ","), ts); err != nil {
		log.Printf("storing poll vote for %s failed: %v", pollID, err)
	}
}

func pollVotes(pollID string) map[string][]string {
	votes := map[string][]string{}
	rows, err := db.Query(`SELECT voter, selected FROM poll_votes WHERE poll_id = ?`, pollID)
	if err != nil {
		return votes
	}
	defer rows.Close()
	for rows.Next() {
		var voter, selected string
		if rows.Scan(&voter, &selected) == nil && selected != "" {
			votes[voter] = strings.Split(selected, ",")
		}
	}
	return votes
}

func pollResultsText(p poll, votes map[string][]string, me string) string {
	if len(p.options) == 0 {
		return ""
	}
	if len(votes) == 0 {
		return fmt.Sprintf("Options: %s. No votes yet.", strings.Join(p.options, ", ")) + quizAnswerText(p, nil)
	}
	counts := make([]int, len(p.options))
	index := map[string]int{}
	for i, name := range p.options {
		index[optionHash(name)] = i
	}
	for _, hashes := range votes {
		for _, h := range hashes {
			if i, ok := index[h]; ok {
				counts[i]++
			}
		}
	}
	parts := make([]string, len(p.options))
	for i, name := range p.options {
		pct := int(math.Round(float64(counts[i]) * 100 / float64(len(votes))))
		noun := "votes"
		if counts[i] == 1 {
			noun = "vote"
		}
		parts[i] = fmt.Sprintf("%s %d%%, %d %s", name, pct, counts[i], noun)
	}
	voted := fmt.Sprintf("%d people voted", len(votes))
	if len(votes) == 1 {
		voted = "1 person voted"
	}
	text := strings.Join(parts, "; ") + ". " + voted
	if mine := votes[me]; len(mine) > 0 {
		var names []string
		for _, h := range mine {
			if i, ok := index[h]; ok {
				names = append(names, p.options[i])
			}
		}
		if len(names) > 0 {
			text += ", you voted " + strings.Join(names, " and ")
		}
	}
	return text + "." + quizAnswerText(p, votes[me])
}

func newPollResults(msg *waE2E.Message) string {
	if p, ok := pollFromMessage(msg); ok {
		return pollResultsText(p, nil, "")
	}
	if event := unwrapMessage(msg).GetEventMessage(); event != nil && !event.GetIsCanceled() {
		return eventResponsesText(nil, "")
	}
	return ""
}

func pollResultsForMessage(pollID string) string {
	p, ok := loadPoll(pollID)
	if !ok {
		return eventResultsForMessage(pollID)
	}
	if counts, isChannel := channelPollCounts(pollID); isChannel {
		return channelPollResultsText(p, counts, pollVotes(pollID)[ownVoterKey()])
	}
	return pollResultsText(p, pollVotes(pollID), ownVoterKey())
}

func channelPollCounts(pollID string) (map[string]int, bool) {
	var jid, raw string
	db.QueryRow(`SELECT jid, COALESCE(channel_poll_votes, '') FROM messages WHERE id = ?`, pollID).Scan(&jid, &raw)
	if !isChannelJID(jid) {
		return nil, false
	}
	counts := map[string]int{}
	if raw != "" {
		json.Unmarshal([]byte(raw), &counts)
	}
	return counts, true
}

func channelPollResultsText(p poll, counts map[string]int, mine []string) string {
	if len(p.options) == 0 {
		return ""
	}
	total := 0
	for _, name := range p.options {
		total += counts[optionHash(name)]
	}
	var text string
	if total == 0 {
		text = fmt.Sprintf("Options: %s. No votes yet", strings.Join(p.options, ", "))
	} else {
		parts := make([]string, len(p.options))
		for i, name := range p.options {
			n := counts[optionHash(name)]
			noun := "votes"
			if n == 1 {
				noun = "vote"
			}
			parts[i] = fmt.Sprintf("%s %d%%, %d %s", name, int(math.Round(float64(n)*100/float64(total))), n, noun)
		}
		text = strings.Join(parts, "; ") + ". " + fmt.Sprintf("%d votes in all", total)
		if total == 1 {
			text = strings.Join(parts, "; ") + ". 1 vote in all"
		}
	}
	var names []string
	for _, h := range mine {
		for _, name := range p.options {
			if optionHash(name) == h {
				names = append(names, name)
			}
		}
	}
	if len(names) > 0 {
		text += ", you voted " + strings.Join(names, " and ")
	}
	return text + "." + quizAnswerText(p, mine)
}

func broadcastPollResults(chatJID, pollID string) string {
	results := pollResultsForMessage(pollID)
	if results != "" {
		broadcastEvent("poll_updated", map[string]interface{}{
			"jid":         chatJID,
			"message_id":  pollID,
			"pollResults": results,
		})
	}
	return results
}

func applyPollVote(v *events.Message) {
	update := unwrapMessage(v.Message).GetPollUpdateMessage()
	pollID := update.GetPollCreationMessageKey().GetID()
	vote, err := decryptPollVote(v)
	if err != nil {
		log.Printf("poll vote on %s couldn't be decrypted: %v", pollID, err)
		return
	}
	ts := v.Info.Timestamp.Unix()
	if ms := update.GetSenderTimestampMS(); ms > 0 {
		ts = ms / 1000
	}
	storePollVote(db, pollID, pollVoterKey(v.Info.Sender), vote.GetSelectedOptions(), ts)
	chat, _ := resolveLIDChat(v)
	broadcastPollResults(chat.String(), pollID)
}

func decryptPollVote(v *events.Message) (*waE2E.PollVoteMessage, error) {
	evt := *v
	evt.Message = unwrapMessage(v.Message)
	return wa().DecryptPollVote(context.Background(), &evt)
}

func historyPollVotes(exec sqlExecer, chat types.JID, webMsg *waWeb.WebMessageInfo) {
	pollID := webMsg.GetKey().GetID()
	for _, pu := range webMsg.GetPollUpdates() {
		key := pu.GetPollUpdateMessageKey()
		var voter string
		switch {
		case key.GetFromMe():
			voter = ownVoterKey()
		case key.GetParticipant() != "":
			if j, err := types.ParseJID(key.GetParticipant()); err == nil {
				voter = pollVoterKey(j)
			}
		default:
			if j, err := types.ParseJID(key.GetRemoteJID()); err == nil {
				voter = pollVoterKey(j)
			} else {
				voter = pollVoterKey(chat)
			}
		}
		storePollVote(exec, pollID, voter, pu.GetVote().GetSelectedOptions(), pu.GetSenderTimestampMS()/1000)
	}
}

type PollRefData struct {
	JID       string   `json:"jid"`
	MessageID string   `json:"message_id"`
	Options   []string `json:"options"`
}

func handleGetPoll(conn net.Conn, raw json.RawMessage) {
	var data PollRefData
	if json.Unmarshal(raw, &data) != nil {
		return
	}
	p, ok := loadPoll(data.MessageID)
	if !ok {
		sendEvent(conn, "error", map[string]string{"message": "This poll can't be opened here."})
		return
	}
	selected := []string{}
	for _, h := range pollVotes(data.MessageID)[ownVoterKey()] {
		for _, name := range p.options {
			if optionHash(name) == h {
				selected = append(selected, name)
			}
		}
	}
	sendEvent(conn, "poll_details", map[string]interface{}{
		"jid":        data.JID,
		"message_id": data.MessageID,
		"question":   p.question,
		"options":    p.options,
		"multiple":   p.multiple,
		"selected":   selected,
		"quiz":       p.quiz,
	})
}

func handleSendPollVote(conn net.Conn, raw json.RawMessage) {
	var data PollRefData
	if json.Unmarshal(raw, &data) != nil {
		return
	}
	go func() {
		fail := func(msg string) { sendEvent(conn, "error", map[string]string{"message": msg}) }
		p, ok := loadPoll(data.MessageID)
		if !ok {
			fail("This poll can't be voted in here.")
			return
		}
		valid := map[string]bool{}
		for _, name := range p.options {
			valid[name] = true
		}
		for _, name := range data.Options {
			if !valid[name] {
				fail("That option isn't in this poll.")
				return
			}
		}
		if !p.multiple && len(data.Options) > 1 {
			fail("This poll takes only one answer.")
			return
		}
		if p.quiz && len(pollVotes(data.MessageID)[ownVoterKey()]) > 0 {
			fail("You've already answered this quiz.")
			return
		}
		if p.quiz && len(data.Options) == 0 {
			fail("Choose an answer.")
			return
		}
		if isChannelJID(data.JID) {
			sendChannelPollVote(conn, data, fail)
			return
		}
		chat, err := types.ParseJID(data.JID)
		if err != nil {
			fail("invalid jid")
			return
		}
		info := secretMessageInfo(chat, data.MessageID)
		msg, err := wa().BuildPollVote(context.Background(), info, data.Options)
		if err != nil {
			fail("Couldn't vote: " + err.Error())
			return
		}
		msg.PollUpdateMessage.SenderTimestampMS = proto.Int64(time.Now().UnixMilli())
		if _, err := wa().SendMessage(context.Background(), chat, msg, whatsmeow.SendRequestExtra{}); err != nil {
			fail("Couldn't send the vote: " + err.Error())
			return
		}
		storePollVote(db, data.MessageID, ownVoterKey(), whatsmeow.HashPollOptions(data.Options), time.Now().Unix())
		results := broadcastPollResults(data.JID, data.MessageID)
		sendEvent(conn, "poll_voted", map[string]interface{}{
			"message_id":  data.MessageID,
			"pollResults": results,
			"retracted":   len(data.Options) == 0,
		})
	}()
}

func secretMessageInfo(chat types.JID, messageID string) *types.MessageInfo {
	var fromMe bool
	var senderStr string
	db.QueryRow(`SELECT from_me, COALESCE(sender_jid,'') FROM messages WHERE id = ?`, messageID).Scan(&fromMe, &senderStr)
	sender := chat
	if fromMe && wa().Store.ID != nil {
		sender = wa().Store.ID.ToNonAD()
	} else if s, perr := types.ParseJID(senderStr); perr == nil && !s.IsEmpty() {
		sender = s
	}
	if _, realSender, serr := wa().Store.MsgSecrets.GetMessageSecret(context.Background(), chat, sender, messageID); serr == nil && !realSender.IsEmpty() {
		sender = realSender
	}
	return &types.MessageInfo{
		MessageSource: types.MessageSource{Chat: chat, Sender: sender, IsFromMe: fromMe, IsGroup: chat.Server == types.GroupServer},
		ID:            messageID,
	}
}

func sendChannelPollVote(conn net.Conn, data PollRefData, fail func(string)) {
	var serverID int64
	db.QueryRow(`SELECT COALESCE(server_id, 0) FROM messages WHERE id = ?`, data.MessageID).Scan(&serverID)
	chat, err := types.ParseJID(data.JID)
	if err != nil || serverID == 0 {
		fail("Can't vote in this poll: it hasn't finished loading from the channel.")
		return
	}
	if err := wa().NewsletterSendPollVote(context.Background(), chat, types.MessageServerID(serverID), data.Options); err != nil {
		fail("Couldn't send the vote: " + err.Error())
		return
	}
	if counts, ok := channelPollCounts(data.MessageID); ok {
		for _, h := range pollVotes(data.MessageID)[ownVoterKey()] {
			if counts[h] > 0 {
				counts[h]--
			}
		}
		for _, name := range data.Options {
			counts[optionHash(name)]++
		}
		if out, err := json.Marshal(counts); err == nil {
			db.Exec(`UPDATE messages SET channel_poll_votes = ? WHERE id = ?`, string(out), data.MessageID)
		}
	}
	storePollVote(db, data.MessageID, ownVoterKey(), whatsmeow.HashPollOptions(data.Options), time.Now().Unix())
	results := broadcastPollResults(data.JID, data.MessageID)
	sendEvent(conn, "poll_voted", map[string]interface{}{
		"message_id":  data.MessageID,
		"pollResults": results,
		"retracted":   len(data.Options) == 0,
	})
}

func pollInfoForMessage(pollID string) map[string]interface{} {
	p, ok := loadPoll(pollID)
	if !ok {
		return nil
	}
	nameOf := map[string]string{}
	for _, name := range p.options {
		nameOf[optionHash(name)] = name
	}
	me := ownVoterKey()
	voters := []map[string]interface{}{}
	rows, err := db.Query(`SELECT voter, selected, timestamp FROM poll_votes WHERE poll_id = ? AND selected != '' ORDER BY timestamp DESC`, pollID)
	if err == nil {
		defer rows.Close()
		for rows.Next() {
			var voter, selected string
			var ts int64
			if rows.Scan(&voter, &selected, &ts) != nil {
				continue
			}
			var picked []string
			for _, h := range strings.Split(selected, ",") {
				if name, ok := nameOf[h]; ok {
					picked = append(picked, name)
				}
			}
			if len(picked) == 0 {
				continue
			}
			name := "You"
			if voter != me {
				name = voter
			}
			voters = append(voters, map[string]interface{}{"name": name, "options": picked, "timestamp": ts, "me": voter == me})
		}
		rows.Close()
		// Names are looked up with the rows closed, as that runs queries of its own.
		for _, v := range voters {
			if !v["me"].(bool) {
				if jid, perr := types.ParseJID(v["name"].(string)); perr == nil {
					v["name"] = resolveDisplayName(jid)
				}
			}
		}
	}
	_, isChannel := channelPollCounts(pollID)
	return map[string]interface{}{
		"question": p.question,
		"results":  pollResultsForMessage(pollID),
		"voters":   voters,
		"channel":  isChannel,
	}
}

var convertStoredVotesOnce sync.Once

func convertStoredPollVotes() {
	type storedRow struct {
		id, jid, sender string
		fromMe          bool
		ts              int64
		raw             []byte
	}
	rows, err := db.Query(`SELECT id, jid, from_me, COALESCE(sender_jid,''), COALESCE(timestamp,0), raw FROM messages
		WHERE text = '[Unsupported Message]' AND raw IS NOT NULL AND length(raw) > 0`)
	if err != nil {
		return
	}
	var found []storedRow
	for rows.Next() {
		var r storedRow
		if rows.Scan(&r.id, &r.jid, &r.fromMe, &r.sender, &r.ts, &r.raw) == nil {
			found = append(found, r)
		}
	}
	rows.Close()

	polls := map[string]string{}
	converted := 0
	for _, r := range found {
		msg := &waE2E.Message{}
		if proto.Unmarshal(r.raw, msg) != nil || unwrapMessage(msg).GetPollUpdateMessage() == nil {
			continue
		}
		chat, err := types.ParseJID(r.jid)
		if err != nil {
			continue
		}
		sender := chat
		if r.fromMe && wa().Store.ID != nil {
			sender = wa().Store.ID.ToNonAD()
		} else if s, perr := types.ParseJID(r.sender); perr == nil && !s.IsEmpty() {
			sender = s
		}
		evt := &events.Message{Message: msg, Info: types.MessageInfo{
			MessageSource: types.MessageSource{Chat: chat, Sender: sender, IsFromMe: r.fromMe, IsGroup: chat.Server == types.GroupServer},
			ID:            r.id,
			Timestamp:     time.Unix(r.ts, 0),
		}}
		vote, verr := decryptPollVote(evt)
		if verr != nil && sender.Server == types.DefaultUserServer {
			if lid, lerr := wa().Store.LIDs.GetLIDForPN(context.Background(), sender); lerr == nil && !lid.IsEmpty() {
				evt.Info.Sender = lid
				vote, verr = decryptPollVote(evt)
			}
		}
		if verr == nil {
			pollID := unwrapMessage(msg).GetPollUpdateMessage().GetPollCreationMessageKey().GetID()
			storePollVote(db, pollID, pollVoterKey(sender), vote.GetSelectedOptions(), r.ts)
			polls[pollID] = r.jid
		} else {
			log.Printf("stored poll vote %s couldn't be decrypted, removing it anyway: %v", r.id, verr)
		}
		purgeMessages("id = ?", r.id)
		converted++
	}
	for pollID, chat := range polls {
		broadcastPollResults(chat, pollID)
	}
	if converted > 0 {
		log.Printf("Turned %d stored poll votes back into votes", converted)
		broadcastChats()
	}
}
