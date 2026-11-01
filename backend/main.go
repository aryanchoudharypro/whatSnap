package main

import (
	"context"
	"database/sql"
	"flag"
	"fmt"
	"log"
	"os"
	"os/signal"
	"runtime/debug"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	_ "modernc.org/sqlite"

	"github.com/Microsoft/go-winio"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/store"
	"go.mau.fi/whatsmeow/store/sqlstore"
	"go.mau.fi/whatsmeow/types"
	waLog "go.mau.fi/whatsmeow/util/log"
	"google.golang.org/protobuf/proto"
	"net"
)

var (
	container      *sqlstore.Container
	db             *sql.DB
	wsClients      = make(map[net.Conn]bool)
	wsMutex        sync.Mutex
	pairingCode    string
	dbPathFlag     = flag.String("db", "", "SQLite db path (default: %LOCALAPPDATA%\\whatSnap\\session.db)")
	pipeFlag       = flag.String("pipe", "", "Named pipe to listen on")
	verboseLogFlag = flag.Bool("verbose-logging", false,
		"Log every WhatsApp protocol frame and SQL query (DEBUG level) instead of just warnings/errors")
)

var waClient atomic.Pointer[whatsmeow.Client]

func wa() *whatsmeow.Client { return waClient.Load() }

func waLogLevel() string {
	if *verboseLogFlag {
		return "DEBUG"
	}
	return "WARN"
}

func buildClient(deviceStore *store.Device) *whatsmeow.Client {
	clientLog := waLog.Stdout("Client", waLogLevel(), true)
	hookLIDStore(deviceStore)
	c := whatsmeow.NewClient(deviceStore, clientLog)
	c.UserAgent = webViewUserAgent()
	c.EnableAutoReconnect = true
	c.AutoReconnectHook = autoReconnectHook(c)
	c.AutoTrustIdentity = true
	c.AutomaticMessageRerequestFromPhone = true
	c.UseRetryMessageStore = true
	c.EnableDecryptedEventBuffer = true
	// The phone never re-uploads a history chunk it has a receipt for, so
	// the receipt waits until the chunk's blob has actually been downloaded.
	// A chunk whose download failed stays unacknowledged and comes again.
	c.AckHistorySyncAfterDispatch = true
	c.StreamHistorySyncConversations = true
	c.SendReportingTokens = true
	c.EmitAppStateEventsOnFullSync = true
	c.GetMessageForRetry = func(requester, to types.JID, id types.MessageID) *waE2E.Message {
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

func replaceWithFreshClient() {
	newDevice := container.NewDevice()
	waClient.Store(buildClient(newDevice))
	initCallClient()
}

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

	debug.SetGCPercent(50)

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
		sqlitePath += "?_pragma=journal_mode(WAL)&_pragma=busy_timeout(10000)&_pragma=synchronous(NORMAL)&_pragma=foreign_keys(1)&_pragma=cache_size(-1000)&_txlock=immediate"
	}
	db, err = sql.Open("sqlite", sqlitePath)
	if err != nil {
		panic(err)
	}
	db.SetMaxOpenConns(8)
	db.SetMaxIdleConns(3)
	if _, err := db.Exec("PRAGMA foreign_keys = ON;"); err != nil {
		panic(err)
	}

	if err := migrateDB(); err != nil {
		panic(err)
	}
	go fixupLegacyStickerTexts()

	interrupt := make(chan os.Signal, 1)
	signal.Notify(interrupt, os.Interrupt)
	go func() {
		<-interrupt
		handleShutdown()
	}()

	container = sqlstore.NewWithDB(db, "sqlite", dbLog)
	err = container.Upgrade(context.Background())
	if err != nil {
		panic(err)
	}
	removeOrphanedDeviceRows()

	// whatSnap presents itself as the Windows app (see applyClientIdentity) -
	// the same branch of WA Web that turns on RequireFullSync and the phone's
	// history-length choice below. These are only sent in the registration
	// payload, so they take effect on the next link, never on an existing
	// session.
	applyClientIdentity()
	store.DeviceProps.RequireFullSync = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.OnDemandReady = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.CompleteOnDemandReady = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.FullSyncDaysLimit = proto.Uint32(365)
	// Untried until the next link (2026-11-01): the "recent" first part
	// covered about a month, and the per-chat cap of the initial sync.
	store.DeviceProps.HistorySyncConfig.RecentSyncDaysLimit = proto.Uint32(365)
	store.DeviceProps.HistorySyncConfig.InitialSyncMaxMessagesPerChat = proto.Uint32(30000)
	store.DeviceProps.HistorySyncConfig.InlineInitialPayloadInE2EeMsg = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.SupportCallLogHistory = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.SupportBotUserAgentChatHistory = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.SupportCagReactionsAndPolls = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.SupportBizHostedMsg = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.SupportHostedGroupMsg = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.SupportMessageAssociation = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.SupportGroupHistory = proto.Bool(true)
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

	waClient.Store(buildClient(deviceStore))
	initCallClient()
	go mergeAllKnownLIDRows()

	if wa().Store.ID != nil {
		resetHistorySyncProgress()
		requestConnect(0)
	}

	pipePath := *pipeFlag
	if pipePath == "" {
		log.Fatal("Must provide --pipe argument for IPC")
	}

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
