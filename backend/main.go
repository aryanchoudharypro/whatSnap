// whaNTV backend -- WhatsApp protocol service for the whaNTV client.
// Copyright (C) 2026 Aryan Choudhary
//
// This program is free software: you can redistribute it and/or modify it
// under the terms of the GNU General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option)
// any later version. It is distributed WITHOUT ANY WARRANTY; without even the
// implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
// See the GNU General Public License in LICENSE for more details.
//
// GPLv3 applies because this binary links go.mau.fi/libsignal. See
// ../THIRD-PARTY-NOTICES.md.

package main

import (
	"context"
	"database/sql"
	"flag"
	"fmt"
	"log"
	"os"
	"runtime/debug"
	"strings"
	"sync"
	"time"

	_ "modernc.org/sqlite"

	"net"
	"github.com/Microsoft/go-winio"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waCompanionReg"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/store"
	"go.mau.fi/whatsmeow/store/sqlstore"
	"go.mau.fi/whatsmeow/types"
	waLog "go.mau.fi/whatsmeow/util/log"
	"google.golang.org/protobuf/proto"
)

var (
	client      *whatsmeow.Client
	container   *sqlstore.Container
	db          *sql.DB
	wsClients   = make(map[net.Conn]bool)
	wsMutex          sync.Mutex
	pairingCode      string
	dbPathFlag       = flag.String("db", "", "SQLite db path (default: %LOCALAPPDATA%\\whaNTV\\session.db)")
	pipeFlag         = flag.String("pipe", "", "Named pipe to listen on")
	verboseLogFlag   = flag.Bool("verbose-logging", false,
		"Log every WhatsApp protocol frame and SQL query (DEBUG level) instead of just warnings/errors")
)

// waLogLevel is whatsmeow's own client/database logger level: DEBUG logs
// every protocol frame and every SQL query the store runs, which is exactly
// the kind of continuous, high-volume logging worth being able to switch off
// - it's real allocation churn and unbounded log-file growth for something
// only needed while actively troubleshooting a connection issue, not during
// normal use. WARN keeps genuine problems visible either way.
func waLogLevel() string {
	if *verboseLogFlag {
		return "DEBUG"
	}
	return "WARN"
}

// buildClient wires up a *whatsmeow.Client with this app's fixed
// configuration (delivery receipts, auto-reconnect, media resend, etc.)
// around the given device store. Called once at startup and again by
// handleLogout to hand the app a fresh, unlinked session after a log out.
func buildClient(deviceStore *store.Device) *whatsmeow.Client {
	clientLog := waLog.Stdout("Client", waLogLevel(), true)
	c := whatsmeow.NewClient(deviceStore, clientLog)
	c.EnableAutoReconnect = true
	c.AutoReconnectHook = autoReconnectHook(c)
	c.AutoTrustIdentity = true
	// Undecryptable messages are worth one automatic ask to the phone -
	// without it a message we couldn't decrypt is simply lost.
	c.AutomaticMessageRerequestFromPhone = true
	c.UseRetryMessageStore = true
	// Keeps each decrypted message's plaintext until it has been handled.
	// Without it, closing whaNTV between decrypting a message (which advances
	// the Signal session for good) and acking it meant the server redelivered
	// it on the next connect, it no longer decrypted, and it was dropped.
	c.EnableDecryptedEventBuffer = true
	// Official clients attach a message secret and a <reporting> token to
	// every message they send (whatsapp-rust always does too), and receivers
	// validate it - WA Web reports a MISSING_MESSAGE_SECRET failure for one
	// without. whatsmeow leaves both off unless this is set.
	c.SendReportingTokens = true
	c.EmitAppStateEventsOnFullSync = true
	// Deliberately not SetForceActiveDeliveryReceipts: whatsmeow's default
	// sends ordinary delivery receipts only while we're marked available and
	// "inactive" ones otherwise - the same thing WhatsApp Web does when it
	// isn't in the foreground. Forcing active receipts from a backgrounded
	// companion is behaviour no official client has.
	c.GetMessageForRetry = func(requester, to types.JID, id types.MessageID) *waE2E.Message {
		// Only a message we sent ourselves may be re-sent in answer to a retry
		// receipt: looking up by ID alone let a crafted retry request pull
		// back someone else's message to us, from any chat.
		var rawBytes []byte
		err := db.QueryRow("SELECT raw FROM messages WHERE id = ? AND from_me = 1", id).Scan(&rawBytes)
		if err == nil && len(rawBytes) > 0 {
			msg := &waE2E.Message{}
			if proto.Unmarshal(rawBytes, msg) == nil {
				return msg
			}
		}
		return nil
	}
	c.AddEventHandler(eventHandler)
	return c
}

// replaceWithFreshClient swaps the global whatsmeow client for a brand-new,
// unpaired one so the app can be paired again without a restart. Used both
// after an explicit "Log Out" and whenever whatsmeow itself reports the
// session was invalidated server-side (events.LoggedOut) - in both cases the
// old device's credentials are already gone, so there's nothing to reuse.
func replaceWithFreshClient() {
	newDevice := container.NewDevice()
	client = buildClient(newDevice)
	initCallClient()
}

// scavengeIdleMemory periodically forces the Go runtime to return unused
// heap spans back to the OS. The runtime's own scavenger does this too, but
// lazily and on its own schedule; after a burst (history sync, opening a
// huge chat) that can leave the process sitting on pages it no longer needs
// for minutes. This just nudges that along on a period short enough that
// Task Manager's number actually reflects what the app is using right now.
func scavengeIdleMemory() {
	go func() {
		ticker := time.NewTicker(2 * time.Minute)
		defer ticker.Stop()
		for range ticker.C {
			debug.FreeOSMemory()
		}
	}()
}

func main() {
	scavengeIdleMemory()

	// This process spends almost all its life idle in the background, so
	// trading a bit of GC CPU for a smaller steady-state heap is a clear win.
	// The default GOGC=100 lets live heap double before collecting; 50 keeps
	// roughly half that resident. SetMemoryLimit is a backstop for the real
	// spikes (parsing tens of thousands of protobuf messages during a history
	// sync) so the collector works harder to stay under it instead of just
	// doubling whatever the spike's live set happened to be.
	debug.SetGCPercent(50)
	debug.SetMemoryLimit(128 << 20)

	flag.Parse()

	if *dbPathFlag == "" {
		*dbPathFlag = defaultDBPath()
	}

	housekeepLogs()
	logFile, err := os.OpenFile(logFilePath(), os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0o600)
	if err == nil {
		log.SetOutput(logFile)
		os.Stdout = logFile
		os.Stderr = logFile
	} else {
		fmt.Println("Failed to open backend log:", err)
	}

	dbPath := *dbPathFlag
	sqlitePath := dbPath
	if !strings.HasPrefix(sqlitePath, "file:") {
		sqlitePath = "file:" + sqlitePath
	}
	dbLog := waLog.Stdout("Database", waLogLevel(), true)
	if !strings.Contains(sqlitePath, "?") {
		// _txlock=immediate: every transaction (ours and whatsmeow's store)
		// takes SQLite's write lock at BEGIN, waiting under busy_timeout,
		// instead of starting as a reader and upgrading at its first write -
		// an upgrade that can't wait for the lock and so fails outright with
		// "database is locked" if another connection committed in between,
		// and lets read-then-write transactions tangle with each other.
		// cache_size(-1000): 1MB of page cache per connection instead of
		// SQLite's own default of -2000 (2MB). Each idle pooled connection
		// keeps its cache resident for as long as it's pooled, so this and
		// the smaller idle pool below directly bound how much RAM sits there
		// doing nothing between queries.
		sqlitePath += "?_pragma=journal_mode(WAL)&_pragma=busy_timeout(10000)&_pragma=synchronous(NORMAL)&_pragma=foreign_keys(1)&_pragma=cache_size(-1000)&_txlock=immediate"
	}
	db, err = sql.Open("sqlite", sqlitePath)
	if err != nil {
		panic(err)
	}
	// SQLite only ever allows one writer at a time no matter how many
	// connections Go opens; leaving the pool unbounded just means bursts of
	// concurrent writers (live events, history sync, call state all firing
	// from their own goroutines) pile up as separate connections each
	// independently burning into the busy_timeout above. Capping it keeps
	// that contention bounded while still leaving room for WAL's concurrent
	// reads (chat/message list queries) alongside the one active writer.
	//
	// MaxIdleConns is intentionally lower than MaxOpenConns: a burst can
	// still open up to 8, but only a few stay pooled (and keep their page
	// cache resident) once things go quiet again - the rest are closed back
	// to the OS instead of sitting idle for the app's whole lifetime.
	db.SetMaxOpenConns(8)
	db.SetMaxIdleConns(3)
	// Also set per connection via _pragma=foreign_keys(1) above - this only
	// covers whichever pooled connection runs it. With just this, whatsmeow's
	// ON DELETE CASCADE cleanup (e.g. dropping a logged-out device's message
	// secrets and contacts) only happened if the delete landed on that one
	// connection, so old pairings' rows piled up in the database.
	if _, err := db.Exec("PRAGMA foreign_keys = ON;"); err != nil {
		panic(err)
	}

	if err := migrateDB(); err != nil {
		panic(err)
	}

	container = sqlstore.NewWithDB(db, "sqlite", dbLog)
	err = container.Upgrade(context.Background())
	if err != nil {
		panic(err)
	}
	removeOrphanedDeviceRows()

	// UWP is what WA Web itself sends when it runs as the Windows app (the
	// WebView2 wrapper, WAWebEnvironment.isWindows) - the same branch that
	// turns on RequireFullSync and the phone's history-length choice below.
	// Only sent in the registration payload, so it takes effect on the next
	// link, never on an existing session.
	store.DeviceProps.PlatformType = waCompanionReg.DeviceProps_UWP.Enum()
	store.DeviceProps.Os = proto.String("Windows")
	store.DeviceProps.RequireFullSync = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.OnDemandReady = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.CompleteOnDemandReady = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.FullSyncDaysLimit = proto.Uint32(3650)
	store.DeviceProps.HistorySyncConfig.FullSyncSizeMbLimit = proto.Uint32(50000)
	store.DeviceProps.HistorySyncConfig.StorageQuotaMb = proto.Uint32(102400)
	store.DeviceProps.HistorySyncConfig.InlineInitialPayloadInE2EeMsg = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.SupportCallLogHistory = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.SupportBotUserAgentChatHistory = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.SupportCagReactionsAndPolls = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.SupportBizHostedMsg = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.SupportHostedGroupMsg = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.SupportMessageAssociation = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.SupportGroupHistory = proto.Bool(true)
	// The rest of WA Web's static capability claims (WAWebClientPayload, per
	// whatsapp-rust's default_history_sync_config). It does not claim
	// SupportAddOnHistorySyncMigration, so neither do we.
	store.DeviceProps.HistorySyncConfig.SupportRecentSyncChunkMessageCountTuning = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.SupportFbidBotChatHistory = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.SupportManusHistory = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.SupportHatchHistory = proto.Bool(true)


	deviceStore, err := container.GetFirstDevice(context.Background())
	if err != nil {
		panic(err)
	}
	if deviceStore == nil {
		deviceStore = container.NewDevice()
	}

	client = buildClient(deviceStore)
	initCallClient()

	if client.Store.ID != nil {
		// First connect of the session: start history-sync progress from
		// zero (events.Connected deliberately doesn't, see there). The
		// supervisor in connection.go keeps retrying if this attempt fails,
		// and events.Connected does the "connected" broadcast once it lands.
		resetHistorySyncProgress()
		requestConnect(0)
	}

	pipePath := *pipeFlag
	if pipePath == "" {
		log.Fatal("Must provide --pipe argument for IPC")
	}

	// A nil config falls back to the token's default DACL, which is broader
	// than we need for a pipe that can drive the WhatsApp session.
	pipeCfg := &winio.PipeConfig{}
	if sddl, err := currentUserPipeSDDL(); err != nil {
		log.Println("Falling back to default pipe ACL:", err)
	} else {
		pipeCfg.SecurityDescriptor = sddl
	}

	listener, err := winio.ListenPipe(pipePath, pipeCfg)
	if err != nil {
		log.Fatal("Failed to create named pipe:", err)
	}
	defer listener.Close()

	fmt.Println("Backend listening on Named Pipe:", pipePath)

	for {
		conn, err := listener.Accept()
		if err != nil {
			log.Println("Pipe accept error:", err)
			continue
		}

		go handleConnection(conn)
	}
}
