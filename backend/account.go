package main

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"log"
	"net"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"

	"go.mau.fi/whatsmeow/store/sqlstore"
	waLog "go.mau.fi/whatsmeow/util/log"
)

// One backend runs every linked account. Each account keeps its own database
// file and whatsmeow client; the app picks the account with the "account"
// field of each command, and every event carries the account it came from.

var (
	accountsMu sync.Mutex
	accounts   = map[string]*Account{}
	// openMu lets one account open at a time, without holding accountsMu
	// (and so every other account's commands) through its migrations.
	openMu sync.Mutex
)

// queuedCommand is a command waiting its turn on its account. Each account
// runs its commands in order on its own goroutine, so a slow command on one
// account never holds up another.
type queuedCommand struct {
	conn net.Conn
	cmd  Command
}

const accountCommandQueue = 256

func newAccount(id, dbPath string) *Account {
	acc := &Account{id: id, dbPath: dbPath}
	acc.ctx, acc.cancel = context.WithCancel(context.Background())
	acc.commands = make(chan queuedCommand, accountCommandQueue)
	acc.initAccountState()
	return acc
}

func lookupAccount(id string) *Account {
	accountsMu.Lock()
	defer accountsMu.Unlock()
	return accounts[id]
}

// openAccounts lists the accounts that are open, in a stable order.
func openAccounts() []*Account {
	accountsMu.Lock()
	list := make([]*Account, 0, len(accounts))
	for _, acc := range accounts {
		list = append(list, acc)
	}
	accountsMu.Unlock()
	sort.Slice(list, func(i, j int) bool { return list[i].id < list[j].id })
	return list
}

func openSQLite(dbPath string) (*sql.DB, error) {
	sqlitePath := dbPath
	if !strings.HasPrefix(sqlitePath, "file:") {
		sqlitePath = "file:" + sqlitePath
	}
	if !strings.Contains(sqlitePath, "?") {
		sqlitePath += "?_pragma=journal_mode(WAL)&_pragma=busy_timeout(10000)&_pragma=synchronous(NORMAL)&_pragma=foreign_keys(1)&_pragma=cache_size(-1000)&_txlock=immediate"
	}
	db, err := sql.Open("sqlite", sqlitePath)
	if err != nil {
		return nil, err
	}
	db.SetMaxOpenConns(8)
	db.SetMaxIdleConns(3)
	if _, err := db.Exec("PRAGMA foreign_keys = ON;"); err != nil {
		db.Close()
		return nil, err
	}
	return db, nil
}

// openAccount opens the account's database and client and starts connecting
// if it's linked. Opening an account that is already open does nothing.
func openAccount(id, dbPath string) (*Account, error) {
	if !validAccountID(id) {
		return nil, fmt.Errorf("invalid account id %q", id)
	}
	if !filepath.IsAbs(dbPath) {
		return nil, fmt.Errorf("account %s: database path must be absolute", id)
	}
	openMu.Lock()
	defer openMu.Unlock()
	for _, other := range openAccounts() {
		same := strings.EqualFold(filepath.Clean(other.dbPath), filepath.Clean(dbPath))
		switch {
		case other.id == id && same:
			return other, nil
		case other.id == id:
			return nil, fmt.Errorf("account %s is already open on another database", id)
		case same:
			return nil, fmt.Errorf("account %s already uses that database", other.id)
		}
	}
	if err := os.MkdirAll(filepath.Dir(dbPath), 0o700); err != nil {
		return nil, err
	}

	acc := newAccount(id, dbPath)
	db, err := openSQLite(dbPath)
	if err != nil {
		return nil, err
	}
	acc.db = db
	if err := acc.migrateDB(); err != nil {
		db.Close()
		return nil, fmt.Errorf("migrating database: %w", err)
	}
	acc.container = sqlstore.NewWithDB(db, "sqlite", waLog.Stdout("Database "+acc.logName(), waLogLevel(), true))
	if err := acc.container.Upgrade(context.Background()); err != nil {
		db.Close()
		return nil, fmt.Errorf("upgrading whatsmeow store: %w", err)
	}
	acc.removeOrphanedDeviceRows()
	deviceStore, err := acc.container.GetFirstDevice(context.Background())
	if err != nil {
		db.Close()
		return nil, err
	}
	if deviceStore == nil {
		deviceStore = acc.container.NewDevice()
	}
	acc.waClient.Store(acc.buildClient(deviceStore))
	acc.initCallClient()
	accountsMu.Lock()
	accounts[id] = acc
	accountsMu.Unlock()
	log.Printf("Opened account %s (%s)", id, dbPath)

	go acc.runCommands()
	go acc.runConnectionSupervisor()
	go acc.watchSocketLoss()
	go acc.runRetrySweep()
	go acc.runDisappearingSweep()
	go acc.fixupLegacyStickerTexts()
	go acc.mergeAllKnownLIDRows()
	if acc.wa().Store.ID != nil {
		acc.resetHistorySyncProgress()
		acc.requestConnect(0)
	}
	return acc, nil
}

// validAccountID matches the ids the app gives accounts: "default" for the
// first one and 32 lowercase hex characters for the rest.
func validAccountID(id string) bool {
	if id == "default" {
		return true
	}
	if len(id) != 32 {
		return false
	}
	for _, c := range id {
		if !(c >= '0' && c <= '9' || c >= 'a' && c <= 'f') {
			return false
		}
	}
	return true
}

func (acc *Account) runCommands() {
	for {
		select {
		case <-acc.ctx.Done():
			return
		case q := <-acc.commands:
			acc.dispatchCommand(q.conn, q.cmd)
		}
	}
}

// close disconnects the account and closes its database, so its files can
// be deleted. Its goroutines stop with acc.ctx; anything still running on
// the account afterwards just sees a closed database.
func (acc *Account) close() {
	if !acc.closing.CompareAndSwap(false, true) {
		return
	}
	accountsMu.Lock()
	if accounts[acc.id] == acc {
		delete(accounts, acc.id)
	}
	accountsMu.Unlock()
	acc.cancel()
	acc.endPairingAttempt()
	if c := acc.wa(); c != nil {
		c.Disconnect()
	}
	if acc.db != nil {
		if _, err := acc.db.Exec("PRAGMA wal_checkpoint(TRUNCATE)"); err != nil {
			log.Printf("Account %s: WAL checkpoint failed: %v", acc.id, err)
		}
		if err := acc.db.Close(); err != nil {
			log.Printf("Account %s: error closing database: %v", acc.id, err)
		}
	}
	log.Printf("Closed account %s", acc.id)
}

// handleOpenAccount answers "open_account" with the account's greeting, or
// an "account_error" if it can't be opened.
func handleOpenAccount(conn net.Conn, id string, raw json.RawMessage) {
	var data struct {
		DB string `json:"db"`
	}
	json.Unmarshal(raw, &data)
	acc, err := openAccount(id, data.DB)
	if err != nil {
		log.Printf("Couldn't open account %s: %v", id, err)
		sendAccountEvent(conn, id, "account_error", map[string]string{"message": err.Error()})
		return
	}
	acc.commands <- queuedCommand{conn: conn, cmd: Command{Action: "hello"}}
}

// handleCloseAccount closes the account and tells the app once its files
// are free to delete.
func handleCloseAccount(conn net.Conn, id string) {
	go func() {
		if acc := lookupAccount(id); acc != nil {
			acc.close()
		}
		sendAccountEvent(conn, id, "account_closed", nil)
	}()
}

// sendAccountEvent sends an event for an account that may not be open.
func sendAccountEvent(conn net.Conn, id, evtType string, data interface{}) {
	(&Account{id: id}).sendEvent(conn, evtType, data)
}

// logName tells this account's lines apart in the shared backend log.
func (acc *Account) logName() string {
	if len(acc.id) > 8 {
		return acc.id[:8]
	}
	return acc.id
}
