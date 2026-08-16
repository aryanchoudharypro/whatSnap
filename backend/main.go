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
	db          *sql.DB
	wsClients   = make(map[net.Conn]bool)
	wsMutex          sync.Mutex
	pairingCode      string
	dbPathFlag       = flag.String("db", "", "SQLite db path (default: %LOCALAPPDATA%\\whaNTV\\session.db)")
	pipeFlag         = flag.String("pipe", "", "Named pipe to listen on")
)

func main() {
	flag.Parse()

	if *dbPathFlag == "" {
		*dbPathFlag = defaultDBPath()
	}

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
	client.EmitAppStateEventsOnFullSync = true
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
	initCallClient()

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
