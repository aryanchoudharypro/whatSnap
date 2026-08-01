package main

import (
	"context"
	"database/sql"
	"flag"
	"fmt"
	"log"
	"net/http"
	"os"
	"strings"
	"sync"
	"time"

	_ "modernc.org/sqlite"

	"github.com/gorilla/websocket"
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
	db          *sql.DB
	wsClients   = make(map[*websocket.Conn]bool)
	wsMutex     sync.Mutex
	upgrader    = websocket.Upgrader{CheckOrigin: func(r *http.Request) bool { return true }}
	pairingCode      string
	dbPathFlag       = flag.String("db", "session.db", "SQLite db path")
)

func main() {
	flag.Parse()

	logFile, err := os.OpenFile("backend.log", os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0666)
	if err == nil {
		log.SetOutput(logFile)
		os.Stdout = logFile
		os.Stderr = logFile
	} else {
		fmt.Println("Failed to open backend.log:", err)
	}

	dbPath := *dbPathFlag
	sqlitePath := dbPath
	if !strings.HasPrefix(sqlitePath, "file:") {
		sqlitePath = "file:" + sqlitePath
	}
	dbLog := waLog.Stdout("Database", "DEBUG", true)
	if !strings.Contains(sqlitePath, "?") {
		sqlitePath += "?_pragma=journal_mode(WAL)&_pragma=busy_timeout(10000)"
	}
	db, err = sql.Open("sqlite", sqlitePath)
	if err != nil {
		panic(err)
	}
	if _, err := db.Exec("PRAGMA foreign_keys = ON;"); err != nil {
		panic(err)
	}

	if err := migrateDB(); err != nil {
		panic(err)
	}

	container := sqlstore.NewWithDB(db, "sqlite", dbLog)
	err = container.Upgrade(context.Background())
	if err != nil {
		panic(err)
	}

	store.DeviceProps.PlatformType = waCompanionReg.DeviceProps_DESKTOP.Enum()
	store.DeviceProps.Os = proto.String("Windows")
	store.DeviceProps.RequireFullSync = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.OnDemandReady = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.CompleteOnDemandReady = proto.Bool(true)
	store.DeviceProps.HistorySyncConfig.SupportAddOnHistorySyncMigration = proto.Bool(true)
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


	deviceStore, err := container.GetFirstDevice(context.Background())
	if err != nil {
		panic(err)
	}
	if deviceStore == nil {
		deviceStore = container.NewDevice()
	}

	clientLog := waLog.Stdout("Client", "DEBUG", true)
	client = whatsmeow.NewClient(deviceStore, clientLog)
	client.EnableAutoReconnect = true
	client.AutoTrustIdentity = true
	client.UseRetryMessageStore = true
	client.SetForceActiveDeliveryReceipts(true)
	client.AutomaticMessageRerequestFromPhone = true
	client.GetMessageForRetry = func(requester, to types.JID, id types.MessageID) *waE2E.Message {
		var rawBytes []byte
		err := db.QueryRow("SELECT raw FROM messages WHERE id = ?", id).Scan(&rawBytes)
		if err == nil && len(rawBytes) > 0 {
			msg := &waE2E.Message{}
			if proto.Unmarshal(rawBytes, msg) == nil {
				return msg
			}
		}
		return nil
	}
	client.AddEventHandler(eventHandler)

	go func() {
		if client.Store.ID != nil {
			err = client.Connect()
			if err != nil {
				log.Println("Connect error:", err)
				return
			}
			log.Println("Connected to WhatsApp!")
			broadcastEvent("connected", map[string]string{"jid": client.Store.ID.String()})
			go client.SendPresence(context.Background(), types.PresenceAvailable)

			go func() {
				time.Sleep(3 * time.Second)
				populateChatsFromStore()
				broadcastChats()
			}()
		}
	}()

	http.HandleFunc("/ws", wsHandler)
	http.HandleFunc("/shutdown", func(w http.ResponseWriter, r *http.Request) {
		log.Println("Shutting down gracefully...")
		if client != nil {
			client.Disconnect()
		}
		if db != nil {
			db.Close()
		}
		w.WriteHeader(http.StatusOK)
		go func() {
			time.Sleep(500 * time.Millisecond)
			os.Exit(0)
		}()
	})
	fmt.Println("Backend listening on :9000")
	http.ListenAndServe(":9000", nil)
}
