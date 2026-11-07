package main

import (
	"database/sql"
	"strings"
	"testing"

	"go.mau.fi/whatsmeow/proto/waSyncAction"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
	_ "modernc.org/sqlite"
)

func openCallRecordsDB(t *testing.T) {
	t.Helper()
	var err error
	acc.db, err = sql.Open("sqlite", "file::memory:")
	if err != nil {
		t.Fatal(err)
	}
	acc.db.SetMaxOpenConns(1)
	t.Cleanup(func() { acc.db.Close() })
	if err := acc.migrateDB(); err != nil {
		t.Fatal(err)
	}
}

func TestStoreCallRecordPhoneRecordWinsOverLiveRow(t *testing.T) {
	openCallRecordsDB(t)
	peer := "5511999990000@s.whatsapp.net"
	acc.storeCallRecord(callRecord{CallID: "c1", JID: peer, Result: "missed", Timestamp: 100, Text: "Missed voice call"})
	acc.storeCallRecord(callRecord{CallID: "c1", JID: peer, FromMe: true, IsVideo: true, Result: "connected", Duration: 30,
		Timestamp: 90, Text: "Outgoing video call, 30 seconds", Authoritative: true})
	// A live row arriving later must not undo the phone's record.
	acc.storeCallRecord(callRecord{CallID: "c1", JID: peer, Result: "missed", Timestamp: 100, Text: "Missed voice call"})

	var result string
	var video, fromMe int
	var dur int64
	if err := acc.db.QueryRow(`SELECT result, is_video, from_me, duration FROM call_records WHERE call_id='c1'`).
		Scan(&result, &video, &fromMe, &dur); err != nil {
		t.Fatal(err)
	}
	if result != "connected" || video != 1 || fromMe != 1 || dur != 30 {
		t.Errorf("got %s video=%d fromMe=%d dur=%d", result, video, fromMe, dur)
	}
}

func TestDeleteNewestCallRecord(t *testing.T) {
	openCallRecordsDB(t)
	peer := "5511999990000@s.whatsapp.net"
	acc.storeCallRecord(callRecord{CallID: "old", JID: peer, Timestamp: 10, Authoritative: true})
	acc.storeCallRecord(callRecord{CallID: "new", JID: peer, Timestamp: 20, Authoritative: true})
	acc.storeCallRecord(callRecord{CallID: "out", JID: peer, FromMe: true, Timestamp: 30, Authoritative: true})
	acc.storeCallRecord(callRecord{CallID: "later", JID: peer, Timestamp: 500, Authoritative: true})

	if id, ok := acc.deleteNewestCallRecord(peer, false, 100); !ok || id != "new" {
		t.Fatalf("got %q %v, want new", id, ok)
	}
	if id, ok := acc.deleteNewestCallRecord(peer, false, 100); !ok || id != "old" {
		t.Fatalf("got %q %v, want old", id, ok)
	}
	if _, ok := acc.deleteNewestCallRecord(peer, false, 100); ok {
		t.Fatal("a call after the deletion time, or in the other direction, must be kept")
	}
}

func TestMigrationCarriesOverCallMessages(t *testing.T) {
	openCallRecordsDB(t)
	acc.db.Exec(`INSERT INTO messages (id, jid, from_me, text, timestamp, media_type)
		VALUES ('call:x', 'a@s.whatsapp.net', 1, 'Outgoing video call', 5, 'call')`)
	if err := acc.ensureCallRecords(); err != nil {
		t.Fatal(err)
	}
	var video, fromMe int
	if err := acc.db.QueryRow(`SELECT is_video, from_me FROM call_records WHERE call_id='x'`).Scan(&video, &fromMe); err != nil {
		t.Fatal(err)
	}
	if video != 1 || fromMe != 1 {
		t.Errorf("video=%d fromMe=%d", video, fromMe)
	}
}

func TestCallResultName(t *testing.T) {
	if got := callResultName(waSyncAction.CallLogRecord_ONGOING); got != "connected" {
		t.Errorf("ongoing = %q", got)
	}
	if got := callResultName(waSyncAction.CallLogRecord_ACCEPTEDELSEWHERE); got != "accepted_elsewhere" {
		t.Errorf("elsewhere = %q", got)
	}
}

func TestCallLogKeyChatFollowsWAWeb(t *testing.T) {
	ownPN := types.NewJID("911111111111", types.DefaultUserServer)
	ownLID := types.NewJID("55555", types.HiddenUserServer)
	peer := types.NewJID("77777", types.HiddenUserServer)
	other := types.NewJID("88888", types.HiddenUserServer)
	part := func(j types.JID) *waSyncAction.CallLogRecord_ParticipantInfo {
		return &waSyncAction.CallLogRecord_ParticipantInfo{UserJID: proto.String(j.String())}
	}
	cases := []struct {
		name    string
		rec     *waSyncAction.CallLogRecord
		creator types.JID
		fromMe  bool
		want    string
	}{
		{"group", &waSyncAction.CallLogRecord{GroupJID: proto.String("123@g.us")}, peer, false, "123@g.us"},
		{"incoming", &waSyncAction.CallLogRecord{}, peer, false, peer.String()},
		{"outgoing one person", &waSyncAction.CallLogRecord{Participants: []*waSyncAction.CallLogRecord_ParticipantInfo{part(ownLID), part(peer)}}, ownLID, true, peer.String()},
		{"outgoing ad hoc group", &waSyncAction.CallLogRecord{Participants: []*waSyncAction.CallLogRecord_ParticipantInfo{part(peer), part(other)}}, ownLID, true, ownLID.String()},
		{"outgoing no participants", &waSyncAction.CallLogRecord{}, ownLID, true, ownLID.String()},
		{"own call link", &waSyncAction.CallLogRecord{IsCallLink: proto.Bool(true), Participants: []*waSyncAction.CallLogRecord_ParticipantInfo{part(peer)}}, ownLID, true, ownLID.String()},
		{"no creator", &waSyncAction.CallLogRecord{}, types.EmptyJID, false, ""},
	}
	for _, c := range cases {
		if got := callLogKeyChat(c.rec, c.creator, c.fromMe, ownPN, ownLID); got != c.want {
			t.Errorf("%s: got %q, want %q", c.name, got, c.want)
		}
	}
}

func TestCallDeleteMutationIndex(t *testing.T) {
	index := func(k callDeleteKey) []string {
		t.Helper()
		_, m, ok := acc.callDeleteMutation(k)
		if !ok {
			return nil
		}
		return m.Index
	}
	got := index(callDeleteKey{callID: "C1", jid: "5511@s.whatsapp.net", keyChat: "77777@lid", creator: "77777@lid", ts: 100})
	want := []string{"deleteMessageForMe", "77777@lid", "C1", "0", "0"}
	if strings.Join(got, "|") != strings.Join(want, "|") {
		t.Errorf("incoming: got %v, want %v", got, want)
	}
	got = index(callDeleteKey{callID: "C2", jid: "123@g.us", keyChat: "123@g.us", creator: "77777@lid", isGroup: true, ts: 100})
	want = []string{"deleteMessageForMe", "123@g.us", "C2", "0", "77777@lid"}
	if strings.Join(got, "|") != strings.Join(want, "|") {
		t.Errorf("group: got %v, want %v", got, want)
	}
	got = index(callDeleteKey{callID: "C3", jid: "123@g.us", keyChat: "123@g.us", creator: "55555@lid", fromMe: true, isGroup: true, ts: 100})
	want = []string{"deleteMessageForMe", "123@g.us", "C3", "1", "0"}
	if strings.Join(got, "|") != strings.Join(want, "|") {
		t.Errorf("own group call: got %v, want %v", got, want)
	}
	if got := index(callDeleteKey{callID: "C4", jid: "123@g.us", isGroup: true}); got != nil {
		t.Errorf("a group call without a key must stay local, got %v", got)
	}
	_, m, _ := acc.callDeleteMutation(callDeleteKey{callID: "C1", jid: "x@lid", keyChat: "77777@lid", ts: 100})
	act := m.Value.GetDeleteMessageForMeAction()
	if act.GetDeleteMedia() || act.GetMessageTimestamp() != 100 {
		t.Errorf("delete media %v, timestamp %d", act.GetDeleteMedia(), act.GetMessageTimestamp())
	}
}

func TestDeletedCallStaysDeleted(t *testing.T) {
	openCallRecordsDB(t)
	peer := "5511999990000@s.whatsapp.net"
	acc.storeCallRecord(callRecord{CallID: "c1", JID: peer, Timestamp: 10, Authoritative: true})
	acc.db.Exec(`INSERT INTO messages (id, jid, text, timestamp, media_type) VALUES ('call:c1', ?, 'Missed voice call', 10, 'call')`, peer)
	acc.applySyncedCallDelete("c1")
	var n int
	acc.db.QueryRow(`SELECT COUNT(*) FROM call_records WHERE call_id = 'c1'`).Scan(&n)
	var m int
	acc.db.QueryRow(`SELECT COUNT(*) FROM messages WHERE id = 'call:c1'`).Scan(&m)
	if n != 0 || m != 0 {
		t.Fatalf("records=%d messages=%d after delete", n, m)
	}
	acc.storeCallRecord(callRecord{CallID: "c1", JID: peer, Timestamp: 10, Authoritative: true})
	acc.db.QueryRow(`SELECT COUNT(*) FROM call_records WHERE call_id = 'c1'`).Scan(&n)
	if n != 0 {
		t.Fatal("a deleted call came back from a later sync")
	}
}
