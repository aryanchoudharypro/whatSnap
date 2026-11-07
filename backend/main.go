package main

import (
	"flag"
	"fmt"
	"log"
	"os"
	"os/signal"
	"runtime/debug"
	"sync"
	"time"

	_ "modernc.org/sqlite"

	"net"

	"github.com/Microsoft/go-winio"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/store"
	"go.mau.fi/whatsmeow/types"
	waLog "go.mau.fi/whatsmeow/util/log"
	"google.golang.org/protobuf/proto"
)

var (
	wsClients = make(map[net.Conn]bool)
	wsMutex   sync.Mutex

	pipeFlag       = flag.String("pipe", "", "Named pipe to listen on")
	verboseLogFlag = flag.Bool("verbose-logging", false,
		"Log every WhatsApp protocol frame and SQL query (DEBUG level) instead of just warnings/errors")
)

func (acc *Account) wa() *whatsmeow.Client { return acc.waClient.Load() }

func waLogLevel() string {
	if *verboseLogFlag {
		return "DEBUG"
	}
	return "WARN"
}

func (acc *Account) buildClient(deviceStore *store.Device) *whatsmeow.Client {
	clientLog := stallLogger{waLog.Stdout("Client "+acc.logName(), waLogLevel(), true)}
	acc.hookLIDStore(deviceStore)
	c := whatsmeow.NewClient(deviceStore, clientLog)
	c.UserAgent = webViewUserAgent()
	c.EnableAutoReconnect = true
	c.AutoReconnectHook = acc.autoReconnectHook(c)
	c.AutoTrustIdentity = true
	c.AutomaticMessageRerequestFromPhone = true
	c.UseRetryMessageStore = true
	c.EnableDecryptedEventBuffer = true
	// The phone never re-uploads a history chunk it has a receipt for, so
	// the receipt waits until the chunk's blob has actually been downloaded.
	// A chunk whose download failed stays unacknowledged and comes again.
	c.AckHistorySyncAfterDispatch = true
	// A chunk whose download fails (blob expired or broken) is asked for
	// again with HISTORY_SYNC_CHUNK_RETRY, up to three times.
	c.RetryFailedHistorySyncChunks = true
	c.StreamHistorySyncConversations = true
	c.SendReportingTokens = true
	c.EmitAppStateEventsOnFullSync = true
	c.GetMessageForRetry = func(requester, to types.JID, id types.MessageID) *waE2E.Message {
		var rawBytes []byte
		err := acc.db.QueryRow("SELECT raw FROM messages WHERE id = ? AND from_me = 1", id).Scan(&rawBytes)
		if err == nil && len(rawBytes) > 0 {
			msg := &waE2E.Message{}
			if proto.Unmarshal(rawBytes, msg) == nil {
				return msg
			}
		}
		return nil
	}
	c.AddEventHandler(acc.eventHandler)
	return c
}

func (acc *Account) replaceWithFreshClient() {
	newDevice := acc.container.NewDevice()
	acc.waClient.Store(acc.buildClient(newDevice))
	acc.initCallClient()
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

	housekeepLogs()
	logFile, err := os.OpenFile(logFilePath(), os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0o600)
	if err == nil {
		log.SetOutput(logFile)
		os.Stdout = logFile
		os.Stderr = logFile
	} else {
		fmt.Println("Failed to open backend log:", err)
	}

	interrupt := make(chan os.Signal, 1)
	signal.Notify(interrupt, os.Interrupt)
	go func() {
		<-interrupt
		shutdownBackend()
	}()

	// whatSnap presents itself as the Windows app (see applyClientIdentity) -
	// the same branch of WA Web that turns on RequireFullSync and the phone's
	// history-length choice below. These are only sent in the registration
	// payload, so they take effect on the next link, never on an existing
	// session. They are process-wide, the same for every account.
	applyClientIdentity()
	store.DeviceProps.RequireFullSync = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.OnDemandReady = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.CompleteOnDemandReady = proto.Bool(true)
	// Ten years of full history (aryan, 2026-11-02: it pulls in a lot). The
	// Windows app (WA Web's debug_1_year_history_sync branch) sends no size
	// limit and no storage quota, so neither do we. The phone has the final say.
	store.DeviceProps.HistorySyncConfig.FullSyncDaysLimit = proto.Uint32(3650)
	store.DeviceProps.HistorySyncConfig.FullSyncSizeMbLimit = nil
	store.DeviceProps.HistorySyncConfig.StorageQuotaMb = nil
	// One year and 50k messages per chat of recent history (2026-11-01).
	// Only sent at linking. 365 days brought about a month of "recent" history.
	store.DeviceProps.HistorySyncConfig.RecentSyncDaysLimit = proto.Uint32(365)
	store.DeviceProps.HistorySyncConfig.InitialSyncMaxMessagesPerChat = proto.Uint32(50000)
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

	// The app opens each account with "open_account" once it's connected.
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
