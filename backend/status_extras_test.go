package main

import (
	"context"
	"database/sql"
	"testing"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waSyncAction"
	"go.mau.fi/whatsmeow/store/sqlstore"
	"go.mau.fi/whatsmeow/types"
	waLog "go.mau.fi/whatsmeow/util/log"
	"google.golang.org/protobuf/proto"
	_ "modernc.org/sqlite"
)

func setupTestAccount(t *testing.T) (*Account, func()) {
	t.Helper()
	db, err := sql.Open("sqlite", "file::memory:?cache=shared")
	if err != nil {
		t.Fatalf("failed to open in-memory sqlite: %v", err)
	}
	db.SetMaxOpenConns(1)

	testAcc := newAccount("test_acc", "file::memory:")
	testAcc.db = db
	if err := testAcc.migrateDB(); err != nil {
		db.Close()
		t.Fatalf("failed to migrate db: %v", err)
	}

	container := sqlstore.NewWithDB(db, "sqlite", waLog.Noop)
	if err := container.Upgrade(context.Background()); err != nil {
		db.Close()
		t.Fatalf("failed to upgrade whatsmeow store: %v", err)
	}
	deviceStore := container.NewDevice()
	cli := whatsmeow.NewClient(deviceStore, waLog.Noop)
	testAcc.waClient.Store(cli)

	cleanup := func() {
		db.Close()
	}
	return testAcc, cleanup
}

func storeStatusPrivacy(t *testing.T, acc *Account, action *waSyncAction.StatusPrivacyAction) {
	t.Helper()
	raw, err := proto.Marshal(action)
	if err != nil {
		t.Fatalf("failed to marshal status privacy: %v", err)
	}
	if _, err := acc.db.Exec(`INSERT OR REPLACE INTO app_settings (key, value) VALUES (?, ?)`, statusPrivacyKey, raw); err != nil {
		t.Fatalf("failed to store status privacy: %v", err)
	}
}

func TestStatusRecipientsNormalization(t *testing.T) {
	testAcc, cleanup := setupTestAccount(t)
	defer cleanup()

	ctx := context.Background()
	storeStatusPrivacy(t, testAcc, &waSyncAction.StatusPrivacyAction{Mode: waSyncAction.StatusPrivacyAction_CONTACTS.Enum()})

	pn1 := types.NewJID("6281111111", types.DefaultUserServer)
	pn2 := types.NewJID("6282222222", types.DefaultUserServer)
	lid2 := types.NewJID("1002222222", types.HiddenUserServer)
	lidUnmapped := types.NewJID("1003333333", types.HiddenUserServer)

	// Put LID mapping for lid2 -> pn2
	if err := testAcc.wa().Store.LIDs.PutLIDMapping(ctx, lid2, pn2); err != nil {
		t.Fatalf("failed to put lid mapping: %v", err)
	}

	// Put contacts in store:
	// - pn1 with full name (should be included)
	// - lid2 with full name (should be resolved to pn2)
	// - pn2 with full name (duplicate of lid2, should be deduplicated)
	// - lidUnmapped with full name (unresolvable LID, should be skipped to prevent 400 error)
	// - noName contact (empty FullName and FirstName, should be skipped)
	noNamePN := types.NewJID("6284444444", types.DefaultUserServer)

	if err := testAcc.wa().Store.Contacts.PutContactName(ctx, pn1, "User", "One"); err != nil {
		t.Fatalf("failed to put contact pn1: %v", err)
	}
	if err := testAcc.wa().Store.Contacts.PutContactName(ctx, lid2, "User", "Two"); err != nil {
		t.Fatalf("failed to put contact lid2: %v", err)
	}
	if err := testAcc.wa().Store.Contacts.PutContactName(ctx, pn2, "User", "Two"); err != nil {
		t.Fatalf("failed to put contact pn2: %v", err)
	}
	if err := testAcc.wa().Store.Contacts.PutContactName(ctx, lidUnmapped, "Unmapped", "LID"); err != nil {
		t.Fatalf("failed to put contact lidUnmapped: %v", err)
	}
	if err := testAcc.wa().Store.Contacts.PutContactName(ctx, noNamePN, "", ""); err != nil {
		t.Fatalf("failed to put contact noNamePN: %v", err)
	}

	recipients := testAcc.statusRecipients(ctx)
	if recipients == nil {
		t.Fatal("expected non-nil recipients")
	}

	for _, r := range recipients {
		if r.Server != types.DefaultUserServer {
			t.Errorf("expected only DefaultUserServer (s.whatsapp.net) recipients, got: %s", r.String())
		}
	}

	// Should contain pn1 and pn2 exactly once, and NOT contain lidUnmapped or noNamePN
	found := make(map[types.JID]int)
	for _, r := range recipients {
		found[r]++
	}

	if found[pn1] != 1 {
		t.Errorf("expected pn1 once, got %d", found[pn1])
	}
	if found[pn2] != 1 {
		t.Errorf("expected pn2 once, got %d", found[pn2])
	}
	if len(recipients) != 2 {
		t.Errorf("expected exactly 2 recipients, got %d: %+v", len(recipients), recipients)
	}
}

func TestStatusRecipientsOnlyMode(t *testing.T) {
	testAcc, cleanup := setupTestAccount(t)
	defer cleanup()

	ctx := context.Background()

	pn1 := types.NewJID("6281111111", types.DefaultUserServer)
	pn2 := types.NewJID("6282222222", types.DefaultUserServer)
	lid2 := types.NewJID("1002222222", types.HiddenUserServer)
	lidUnmapped := types.NewJID("1009999999", types.HiddenUserServer)

	_ = testAcc.wa().Store.LIDs.PutLIDMapping(ctx, lid2, pn2)

	// Save ALLOW_LIST status privacy
	storeStatusPrivacy(t, testAcc, &waSyncAction.StatusPrivacyAction{
		Mode:    waSyncAction.StatusPrivacyAction_ALLOW_LIST.Enum(),
		UserJID: []string{pn1.String(), lid2.String(), lidUnmapped.String()},
	})

	recipients := testAcc.statusRecipients(ctx)
	if len(recipients) != 2 {
		t.Fatalf("expected 2 recipients (unresolvable LID skipped), got %d: %+v", len(recipients), recipients)
	}

	for _, r := range recipients {
		if r.Server != types.DefaultUserServer {
			t.Errorf("got non-PN recipient: %s", r.String())
		}
	}
}

func TestWithStatusViewersFiltersLID(t *testing.T) {
	testAcc, cleanup := setupTestAccount(t)
	defer cleanup()

	ctx := context.Background()

	pn1 := types.NewJID("6281111111", types.DefaultUserServer)
	pn2 := types.NewJID("6282222222", types.DefaultUserServer)
	lid2 := types.NewJID("1002222222", types.HiddenUserServer)
	lidUnmapped := types.NewJID("1008888888", types.HiddenUserServer)

	_ = testAcc.wa().Store.LIDs.PutLIDMapping(ctx, lid2, pn2)

	// Insert into status_viewers:
	// - pn1 (already in recipients)
	// - lid2 (resolves to pn2, not in recipients)
	// - lidUnmapped (cannot resolve to PN, must be skipped)
	// - 'me' (must be ignored)
	_, _ = testAcc.db.Exec(`INSERT INTO status_viewers (status_id, viewer, viewed_at) VALUES ('stat1', ?, 123)`, pn1.String())
	_, _ = testAcc.db.Exec(`INSERT INTO status_viewers (status_id, viewer, viewed_at) VALUES ('stat1', ?, 124)`, lid2.String())
	_, _ = testAcc.db.Exec(`INSERT INTO status_viewers (status_id, viewer, viewed_at) VALUES ('stat1', ?, 125)`, lidUnmapped.String())
	_, _ = testAcc.db.Exec(`INSERT INTO status_viewers (status_id, viewer, viewed_at) VALUES ('stat1', 'me', 126)`)

	initial := []types.JID{pn1}
	viewers := testAcc.withStatusViewers("stat1", initial)

	if len(viewers) != 2 {
		t.Fatalf("expected exactly 2 viewers, got %d: %+v", len(viewers), viewers)
	}
	for _, v := range viewers {
		if v.Server != types.DefaultUserServer {
			t.Errorf("expected only PN viewer, got: %s", v.String())
		}
	}
}

func TestStatusPostBlocker(t *testing.T) {
	testAcc, cleanup := setupTestAccount(t)
	defer cleanup()

	ctx := context.Background()

	// No stored privacy and no connection to ask the server: refuse rather
	// than fall back to all contacts.
	if reason := testAcc.statusPostBlocker(ctx); reason == "" {
		t.Error("unknown status privacy should block status posting")
	}
	if recipients := testAcc.statusRecipients(ctx); recipients != nil {
		t.Errorf("unknown status privacy should give nil recipients, got %+v", recipients)
	}

	// Contacts mode with 0 contacts
	storeStatusPrivacy(t, testAcc, &waSyncAction.StatusPrivacyAction{Mode: waSyncAction.StatusPrivacyAction_CONTACTS.Enum()})
	if reason := testAcc.statusPostBlocker(ctx); reason != "" {
		t.Errorf("contacts mode shouldn't block: %s", reason)
	}

	// Empty allow list
	storeStatusPrivacy(t, testAcc, &waSyncAction.StatusPrivacyAction{Mode: waSyncAction.StatusPrivacyAction_ALLOW_LIST.Enum()})
	if reason := testAcc.statusPostBlocker(ctx); reason == "" {
		t.Error("empty allow list should block status posting")
	}
}
