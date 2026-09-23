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

// Poll votes. A vote names its options by their SHA-256 hashes, and each
// voter's latest vote replaces their earlier one (an empty vote takes it
// back), so poll_votes keeps one row per poll and voter.

// poll is a stored poll's definition, read back from its message.
type poll struct {
	question string
	options  []string
	multiple bool
}

func loadPoll(pollID string) (poll, bool) {
	var raw []byte
	if db.QueryRow(`SELECT raw FROM messages WHERE id = ?`, pollID).Scan(&raw) != nil || len(raw) == 0 {
		return poll{}, false
	}
	msg := &waE2E.Message{}
	if proto.Unmarshal(raw, msg) != nil {
		return poll{}, false
	}
	return pollFromMessage(unwrapMessage(msg))
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
	return p, true
}

func optionHash(name string) string {
	sum := sha256.Sum256([]byte(name))
	return hex.EncodeToString(sum[:])
}

// pollVoterKey names a voter the same way however their vote reached us
// (phone number or LID, any device).
func pollVoterKey(jid types.JID) string {
	return preferPN(jid.ToNonAD()).String()
}

func ownVoterKey() string {
	if client == nil || client.Store.ID == nil {
		return ""
	}
	return client.Store.ID.ToNonAD().String()
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

// pollVotes returns each voter's current selection (option hashes).
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

// pollResultsText is the line shown after a poll's question: each option's
// share of the voters, like WhatsApp's bars, plus how you voted.
func pollResultsText(p poll, votes map[string][]string, me string) string {
	if len(p.options) == 0 {
		return ""
	}
	if len(votes) == 0 {
		return fmt.Sprintf("Options: %s. No votes yet.", strings.Join(p.options, ", "))
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
	return text + "."
}

// newPollResults is the results line for a poll that has just arrived or
// been sent - no votes yet, so it lists the options ("" if msg isn't a poll).
func newPollResults(msg *waE2E.Message) string {
	if p, ok := pollFromMessage(msg); ok {
		return pollResultsText(p, nil, "")
	}
	return ""
}

// pollResultsForMessage is the results line for a stored poll ("" if pollID
// isn't one).
func pollResultsForMessage(pollID string) string {
	p, ok := loadPoll(pollID)
	if !ok {
		return ""
	}
	return pollResultsText(p, pollVotes(pollID), ownVoterKey())
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

// applyPollVote handles a live vote: decrypt it with the poll's secret,
// record it, and refresh the poll's results.
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

// decryptPollVote decrypts a vote even when it arrived wrapped (ephemeral,
// device-sent...): DecryptPollVote only looks at the top-level message.
func decryptPollVote(v *events.Message) (*waE2E.PollVoteMessage, error) {
	evt := *v
	evt.Message = unwrapMessage(v.Message)
	return client.DecryptPollVote(context.Background(), &evt)
}

// historyPollVotes records the votes history sync delivers with a poll -
// already decrypted, one per voter.
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

// handleGetPoll sends the frontend a poll's options, whether it takes
// several answers, and what we voted last, for the voting dialog.
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
	})
}

// handleSendPollVote votes in a poll (an empty selection takes the vote
// back, as in WhatsApp) and reports the new results.
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
		chat, err := types.ParseJID(data.JID)
		if err != nil {
			fail("invalid jid")
			return
		}
		var fromMe bool
		var senderStr string
		db.QueryRow(`SELECT from_me, COALESCE(sender_jid,'') FROM messages WHERE id = ?`, data.MessageID).Scan(&fromMe, &senderStr)
		sender := chat
		if fromMe && client.Store.ID != nil {
			sender = client.Store.ID.ToNonAD()
		} else if s, perr := types.ParseJID(senderStr); perr == nil && !s.IsEmpty() {
			sender = s
		}
		// Vote in the addressing the poll's secret was saved under: whatsmeow
		// picks our own LID or phone number from the sender's server, and the
		// other side decrypts with whichever the poll used.
		if _, realSender, serr := client.Store.MsgSecrets.GetMessageSecret(context.Background(), chat, sender, data.MessageID); serr == nil && !realSender.IsEmpty() {
			sender = realSender
		}
		info := &types.MessageInfo{
			MessageSource: types.MessageSource{Chat: chat, Sender: sender, IsFromMe: fromMe, IsGroup: chat.Server == types.GroupServer},
			ID:            data.MessageID,
		}
		msg, err := client.BuildPollVote(context.Background(), info, data.Options)
		if err != nil {
			fail("Couldn't vote: " + err.Error())
			return
		}
		msg.PollUpdateMessage.SenderTimestampMS = proto.Int64(time.Now().UnixMilli())
		if _, err := client.SendMessage(context.Background(), chat, msg, whatsmeow.SendRequestExtra{}); err != nil {
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

var convertStoredVotesOnce sync.Once

// convertStoredPollVotes cleans up after older builds, which didn't know
// votes and stored each one in its chat as an "[Unsupported Message]" row:
// every such row that is really a vote becomes that vote on its poll, and
// the row goes. Runs once, after connecting (decrypting needs the client).
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

	polls := map[string]string{} // poll id -> chat
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
		if r.fromMe && client.Store.ID != nil {
			sender = client.Store.ID.ToNonAD()
		} else if s, perr := types.ParseJID(r.sender); perr == nil && !s.IsEmpty() {
			sender = s
		}
		evt := &events.Message{Message: msg, Info: types.MessageInfo{
			MessageSource: types.MessageSource{Chat: chat, Sender: sender, IsFromMe: r.fromMe, IsGroup: chat.Server == types.GroupServer},
			ID:            r.id,
			Timestamp:     time.Unix(r.ts, 0),
		}}
		vote, verr := decryptPollVote(evt)
		// The row keeps the voter's phone number, but the vote may have been
		// sealed to their LID - try that too.
		if verr != nil && sender.Server == types.DefaultUserServer {
			if lid, lerr := client.Store.LIDs.GetLIDForPN(context.Background(), sender); lerr == nil && !lid.IsEmpty() {
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
