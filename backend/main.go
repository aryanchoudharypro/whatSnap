package main

import (
	"context"
	"database/sql"
	"encoding/json"
	"flag"
	"fmt"
	"net/http"
	"strings"
	"sync"
	"time"

	_ "modernc.org/sqlite"

	"github.com/gorilla/websocket"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waCompanionReg"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/store"
	"go.mau.fi/whatsmeow/store/sqlstore"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	waLog "go.mau.fi/whatsmeow/util/log"
	"google.golang.org/protobuf/proto"
)

var (
	client      *whatsmeow.Client
	db          *sql.DB
	wsClients   = make(map[*websocket.Conn]bool)
	wsMutex     sync.Mutex
	upgrader    = websocket.Upgrader{CheckOrigin: func(r *http.Request) bool { return true }}
	pairingCode string
	dbPathFlag  = flag.String("db", "session.db", "SQLite db path")
)

type Event struct {
	Type string          `json:"type"`
	Data json.RawMessage `json:"data,omitempty"`
}

type Command struct {
	Action string          `json:"action"`
	Data   json.RawMessage `json:"data,omitempty"`
}

type PairCommandData struct {
	Phone string `json:"phone"`
}

type SendCommandData struct {
	To   string `json:"to"`
	Text string `json:"text"`
}

type NewContactCommandData struct {
	Phone string `json:"phone"`
	Name  string `json:"name"`
}

func main() {
	flag.Parse()
	dbPath := *dbPathFlag
	if !strings.HasPrefix(dbPath, "file:") {
		dbPath = "file:" + dbPath
	}
	dbLog := waLog.Stdout("Database", "DEBUG", true)
	var err error
	db, err = sql.Open("sqlite", dbPath)
	if err != nil {
		panic(err)
	}
	if _, err := db.Exec("PRAGMA foreign_keys = ON;"); err != nil {
		panic(err)
	}

	if _, err := db.Exec(`CREATE TABLE IF NOT EXISTS chats (
		jid TEXT PRIMARY KEY,
		name TEXT,
		timestamp INTEGER
	)`); err != nil {
		panic(err)
	}
	db.Exec("ALTER TABLE chats ADD COLUMN unread_count INTEGER DEFAULT 0")
	db.Exec("ALTER TABLE chats ADD COLUMN pinned INTEGER DEFAULT 0")
	db.Exec("ALTER TABLE chats ADD COLUMN muted INTEGER DEFAULT 0")
	db.Exec("ALTER TABLE chats ADD COLUMN preview_snippet TEXT DEFAULT ''")
	db.Exec("ALTER TABLE chats ADD COLUMN archived INTEGER DEFAULT 0")

	if _, err := db.Exec(`CREATE TABLE IF NOT EXISTS messages (
		id TEXT PRIMARY KEY,
		jid TEXT,
		from_me BOOLEAN,
		text TEXT,
		timestamp INTEGER,
		sender_jid TEXT
	)`); err != nil {
		panic(err)
	}
	db.Exec("ALTER TABLE messages ADD COLUMN sender_jid TEXT DEFAULT ''")
	if _, err := db.Exec(`CREATE INDEX IF NOT EXISTS idx_messages_jid_timestamp ON messages(jid, timestamp DESC);`); err != nil {
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
	store.DeviceProps.HistorySyncConfig.FullSyncDaysLimit = proto.Uint32(1095)
	store.DeviceProps.HistorySyncConfig.StorageQuotaMb = proto.Uint32(50000)

	deviceStore, err := container.GetFirstDevice(context.Background())
	if err != nil {
		panic(err)
	}
	if deviceStore == nil {
		deviceStore = container.NewDevice()
	}

	clientLog := waLog.Stdout("Client", "DEBUG", true)
	client = whatsmeow.NewClient(deviceStore, clientLog)
	client.AddEventHandler(eventHandler)

	go func() {
		if client.Store.ID != nil {
			err = client.Connect()
			if err != nil {
				fmt.Println(err)
				return
			}
			broadcastEvent("connected", map[string]string{"jid": client.Store.ID.String()})
			go client.SendPresence(context.Background(), types.PresenceAvailable)
			
			go func() {
				time.Sleep(3 * time.Second)

				for _, patch := range appstate.AllPatchNames {
					client.FetchAppState(context.Background(), patch, false, false)
				}

				populateChatsFromStore()
				broadcastChats()
			}()
		}
	}()

	http.HandleFunc("/ws", wsHandler)
	fmt.Println("Backend listening on :9000")
	http.ListenAndServe(":9000", nil)
}

func extractText(msg *waE2E.Message) string {
	if msg == nil {
		return ""
	}
	if msg.GetExtendedTextMessage() != nil {
		return msg.GetExtendedTextMessage().GetText()
	}
	if msg.GetConversation() != "" {
		return msg.GetConversation()
	}
	if msg.GetImageMessage() != nil {
		caption := msg.GetImageMessage().GetCaption()
		if caption != "" {
			return "[Image] " + caption
		}
		return "[Image]"
	}
	if msg.GetVideoMessage() != nil {
		caption := msg.GetVideoMessage().GetCaption()
		if caption != "" {
			return "[Video] " + caption
		}
		return "[Video]"
	}
	if msg.GetDocumentMessage() != nil {
		title := msg.GetDocumentMessage().GetTitle()
		if title == "" {
			title = msg.GetDocumentMessage().GetFileName()
		}
		if title != "" {
			return "[Document] " + title
		}
		return "[Document]"
	}
	if msg.GetAudioMessage() != nil {
		if msg.GetAudioMessage().GetPTT() {
			return "[Voice Message]"
		}
		return "[Audio]"
	}
	if msg.GetLocationMessage() != nil {
		return "[Location]"
	}
	if msg.GetContactMessage() != nil {
		name := msg.GetContactMessage().GetDisplayName()
		if name != "" {
			return "[Contact] " + name
		}
		return "[Contact]"
	}
	if msg.GetStickerMessage() != nil {
		return "[Sticker]"
	}
	if msg.GetReactionMessage() != nil {
		return "[Reaction]"
	}
	if msg.GetPollCreationMessage() != nil {
		return "[Poll] " + msg.GetPollCreationMessage().GetName()
	}
	if msg.GetProtocolMessage() != nil {
		if msg.GetProtocolMessage().GetType() == waE2E.ProtocolMessage_REVOKE {
			return "[Message deleted]"
		}
		return "[Protocol Message]"
	}
	return "[Unsupported Message]"
}

func eventHandler(evt interface{}) {
	switch v := evt.(type) {
	case *events.HistorySync:
		go func() {
			tx, txErr := db.Begin()
			if txErr != nil {
				fmt.Println("Error starting transaction for HistorySync:", txErr)
				return
			}
			for _, conv := range v.Data.GetConversations() {
				jid := conv.GetID()
				if jid == "status@broadcast" {
					continue
				}
				name := conv.GetName()
				ts := conv.GetConversationTimestamp()
				if ts == 0 {
					ts = conv.GetLastMsgTimestamp()
				}
				if ts == 0 && len(conv.GetMessages()) > 0 {
					for _, historyMsg := range conv.GetMessages() {
						msg := historyMsg.GetMessage()
						if msg != nil && msg.MessageTimestamp != nil {
							mTS := uint64(msg.GetMessageTimestamp())
							if mTS > ts {
								ts = mTS
							}
						}
					}
				}
				unread := conv.GetUnreadCount()
				pinned := 0
				if conv.GetPinned() > 0 {
					pinned = 1
				}
				muted := 0
				if conv.GetMuteEndTime() > 0 {
					muted = 1
				}
				archived := 0
				if conv.GetArchived() {
					archived = 1
				}
				_, err := tx.Exec(`INSERT INTO chats (jid, name, timestamp, unread_count, pinned, muted, archived) VALUES (?, ?, ?, ?, ?, ?, ?)
					ON CONFLICT(jid) DO UPDATE SET 
					timestamp=MAX(timestamp, excluded.timestamp), 
					name=COALESCE(NULLIF(excluded.name, ''), chats.name),
					unread_count=excluded.unread_count,
					pinned=excluded.pinned,
					muted=excluded.muted,
					archived=excluded.archived`,
					jid, name, ts, unread, pinned, muted, archived)
				if err != nil {
					fmt.Println("Error inserting chat from HistorySync:", err)
				}

				chatJID, parseErr := types.ParseJID(jid)
				if parseErr == nil {
					for _, historyMsg := range conv.GetMessages() {
						webMsg := historyMsg.GetMessage()
						if webMsg == nil {
							continue
						}
						evtMsg, parseEvtErr := client.ParseWebMessage(chatJID, webMsg)
						if parseEvtErr == nil && evtMsg != nil {
							msgText := extractText(evtMsg.Message)
							msgID := evtMsg.Info.ID
							if msgID == "" && webMsg.Key != nil {
								msgID = webMsg.GetKey().GetID()
							}
							if msgID != "" {
								msgJID := evtMsg.Info.Chat.String()
								if msgJID == "" {
									msgJID = chatJID.String()
								}
								msgFromMe := evtMsg.Info.IsFromMe
								msgTS := evtMsg.Info.Timestamp.Unix()
								if msgTS <= 0 && webMsg.MessageTimestamp != nil {
									msgTS = int64(webMsg.GetMessageTimestamp())
								}
								senderJID := evtMsg.Info.Sender.String()
								_, err := tx.Exec(`INSERT INTO messages (id, jid, from_me, text, timestamp, sender_jid) VALUES (?, ?, ?, ?, ?, ?)
									ON CONFLICT(id) DO UPDATE SET text=excluded.text, timestamp=excluded.timestamp, sender_jid=excluded.sender_jid`,
									msgID, msgJID, msgFromMe, msgText, msgTS, senderJID)
								if err != nil {
									fmt.Println("Error inserting message from HistorySync:", err)
								}
							}
						}
					}
				}
			}
			if err := tx.Commit(); err != nil {
				fmt.Println("Error committing HistorySync transaction:", err)
			}
			broadcastChats()
		}()
	case *events.Message:
		text := extractText(v.Message)

		go func() {
			jid := v.Info.Chat.String()
			if jid == "status@broadcast" {
				return
			}
			ts := v.Info.Timestamp.Unix()
			increment := 1
			if v.Info.IsFromMe {
				increment = 0
			}
			_, err := db.Exec(`INSERT INTO chats (jid, timestamp, unread_count, preview_snippet) VALUES (?, ?, ?, ?)
				ON CONFLICT(jid) DO UPDATE SET 
					timestamp=MAX(timestamp, excluded.timestamp), 
					unread_count=chats.unread_count + excluded.unread_count,
					preview_snippet=COALESCE(NULLIF(excluded.preview_snippet, ''), chats.preview_snippet)`,
				jid, ts, increment, text)
			if err != nil {
				fmt.Println("Error updating chat timestamp:", err)
			}
			if v.Info.ID != "" {
				senderJID := v.Info.Sender.String()
				_, err := db.Exec(`INSERT INTO messages (id, jid, from_me, text, timestamp, sender_jid) VALUES (?, ?, ?, ?, ?, ?)
					ON CONFLICT(id) DO UPDATE SET text=excluded.text, timestamp=excluded.timestamp, sender_jid=excluded.sender_jid`,
					v.Info.ID, jid, v.Info.IsFromMe, text, ts, senderJID)
				if err != nil {
					fmt.Println("Error inserting message from live event:", err)
				}
			}
			broadcastChats()
		}()
		if text != "" {
			broadcastEvent("message", map[string]string{
				"jid":  v.Info.Chat.String(),
				"from": v.Info.Sender.String(),
				"text": text,
			})
		}
	case *events.Connected:
		if client.Store.ID != nil {
			broadcastEvent("connected", map[string]string{"jid": client.Store.ID.String()})
			go client.SendPresence(context.Background(), types.PresenceAvailable)
			go func() {
				time.Sleep(3 * time.Second)
				populateChatsFromStore()
				broadcastChats()
			}()
		}
	case *events.PairSuccess:
		broadcastEvent("connected", map[string]string{"jid": v.ID.String()})
		go client.SendPresence(context.Background(), types.PresenceAvailable)
		go func() {
			time.Sleep(5 * time.Second)
			for _, patch := range appstate.AllPatchNames {
				client.FetchAppState(context.Background(), patch, false, false)
			}
			populateChatsFromStore()
			broadcastChats()
		}()
	case *events.LoggedOut:
		broadcastEvent("disconnected", nil)
	case *events.ChatPresence:
		isTyping := v.State == types.ChatPresenceComposing
		isRecording := v.Media == types.ChatPresenceMediaAudio
		broadcastEvent("typing", map[string]interface{}{
			"chat_jid":   v.MessageSource.Chat.String(),
			"sender_jid": v.MessageSource.Sender.String(),
			"state":      string(v.State),
			"media":      string(v.Media),
			"typing":     isTyping,
			"recording":  isRecording,
		})
	case *events.Presence:
		broadcastEvent("presence", map[string]interface{}{
			"jid":         v.From.String(),
			"unavailable": v.Unavailable,
			"last_seen":   v.LastSeen.Unix(),
		})
	case *events.Receipt:
		broadcastEvent("receipt", map[string]interface{}{
			"chat_jid":    v.MessageSource.Chat.String(),
			"sender_jid":  v.MessageSource.Sender.String(),
			"message_ids": v.MessageIDs,
			"type":        string(v.Type),
			"timestamp":   v.Timestamp.Unix(),
		})
	case *events.AppStateSyncComplete:
		if len(v.Name) > 0 {
			go func() {
				populateChatsFromStore()
				broadcastChats()
			}()
		}
	}
}

func populateChatsFromStore() {
	allContacts, err := client.Store.Contacts.GetAllContacts(context.Background())
	if err != nil {
		fmt.Println("Error getting contacts from store:", err)
		return
	}

	for jid, contact := range allContacts {
		jidStr := jid.String()
		if jidStr == "status@broadcast" {
			continue
		}

		name := contact.FullName
		if name == "" {
			name = contact.PushName
		}
		if name == "" {
			name = contact.BusinessName
		}

		if jid.Server == "lid" {
			if pn, pnErr := client.Store.LIDs.GetPNForLID(context.Background(), jid); pnErr == nil && !pn.IsEmpty() {
				pnContact, pnContactErr := client.Store.Contacts.GetContact(context.Background(), pn)
				if pnContactErr == nil && pnContact.Found {
					if pnContact.FullName != "" {
						name = pnContact.FullName
					} else if pnContact.PushName != "" {
						name = pnContact.PushName
					}
				}
				if name == "" {
					name = "+" + pn.User
				}
			}
		}

		if name == "" {
			continue
		}

		db.Exec(`INSERT INTO chats (jid, name, timestamp) VALUES (?, ?, 0)
			ON CONFLICT(jid) DO UPDATE SET
			name=COALESCE(NULLIF(excluded.name, ''), chats.name)`,
			jidStr, name)
	}

	groups, err := client.GetJoinedGroups(context.Background())
	if err == nil {
		for _, g := range groups {
			jidStr := g.JID.String()
			db.Exec(`INSERT INTO chats (jid, name, timestamp) VALUES (?, ?, 0)
				ON CONFLICT(jid) DO UPDATE SET
				name=COALESCE(NULLIF(excluded.name, ''), chats.name)`,
				jidStr, g.GroupName.Name)
		}
	}

	db.Exec("DELETE FROM chats WHERE jid = 'status@broadcast'")
}

func wsHandler(w http.ResponseWriter, r *http.Request) {
	conn, err := upgrader.Upgrade(w, r, nil)
	if err != nil {
		fmt.Println("WS Upgrade error:", err)
		return
	}
	fmt.Println("New WS connection!")
	defer conn.Close()

	wsMutex.Lock()
	wsClients[conn] = true
	wsMutex.Unlock()

	defer func() {
		wsMutex.Lock()
		delete(wsClients, conn)
		wsMutex.Unlock()
	}()

	if client.Store.ID != nil && client.IsConnected() {
		sendEvent(conn, "connected", map[string]string{"jid": client.Store.ID.String()})
		go func() {
			populateChatsFromStore()
			broadcastChats()
		}()
	}

	for {
		_, msg, err := conn.ReadMessage()
		if err != nil {
			break
		}

		var cmd Command
		if err := json.Unmarshal(msg, &cmd); err != nil {
			continue
		}

		switch cmd.Action {
		case "get_qr":
			go func() {
				if client.Store.ID == nil {
					if client != nil && client.IsConnected() {
						client.Disconnect()
					}
					qrChan, err := client.GetQRChannel(context.Background())
					if err != nil {
						sendEvent(conn, "error", map[string]string{"message": err.Error()})
						return
					}
					err = client.Connect()
					if err != nil {
						sendEvent(conn, "error", map[string]string{"message": err.Error()})
						return
					}
					for qr := range qrChan {
						if qr.Event == "code" {
							sendEvent(conn, "qr", map[string]string{"code": qr.Code})
						}
					}
				}
			}()
		case "cancel_qr", "cancel_pairing":
			fmt.Println("Received cancel request")
			if client.Store.ID == nil {
				client.Disconnect()
			}
		case "pair":
			fmt.Println("Received pair request for:", string(cmd.Data))
			var data PairCommandData
			if err := json.Unmarshal(cmd.Data, &data); err == nil {
				fmt.Println("Parsed phone:", data.Phone)
				go func() {
					if client.Store.ID == nil {
						if !client.IsConnected() {
							qrChan, _ := client.GetQRChannel(context.Background())
							err := client.Connect()
							if err != nil {
								broadcastEvent("error", map[string]string{"message": err.Error()})
								return
							}
							if qrChan != nil {
								<-qrChan // Wait for the first QR event so connection is fully established
								go func() {
									for range qrChan {
										// Consume channel to avoid leaks, but don't call PairPhone again
									}
								}()
							}
						}
						code, err := client.PairPhone(context.Background(), data.Phone, true, whatsmeow.PairClientFirefox, "Firefox (Windows)")
						if err != nil {
							broadcastEvent("error", map[string]string{"message": err.Error()})
							return
						}
						pairingCode = code
						broadcastEvent("pairing_code", map[string]string{"code": code})
					}
				}()
			}
		case "send":
			var data SendCommandData
			if err := json.Unmarshal(cmd.Data, &data); err == nil {
				jid, err := types.ParseJID(data.To)
				if err != nil {
					continue
				}
				msg := &waE2E.Message{
					Conversation: &data.Text,
				}
				_, err = client.SendMessage(context.Background(), jid, msg)
				if err != nil {
					sendEvent(conn, "error", map[string]string{"message": err.Error()})
				}
			}
		case "new_contact":
			var data NewContactCommandData
			if err := json.Unmarshal(cmd.Data, &data); err == nil {
				go func() {
					var digits []rune
					for _, r := range data.Phone {
						if r >= '0' && r <= '9' {
							digits = append(digits, r)
						}
					}
					phone := string(digits)
					resp, err := client.IsOnWhatsApp(context.Background(), []string{phone})
					if err != nil || len(resp) == 0 || !resp[0].IsIn {
						sendEvent(conn, "new_contact_status", map[string]interface{}{"status": "error", "error": "Not on WhatsApp or query failed"})
						return
					}
					jid := resp[0].JID
					err = client.Store.Contacts.PutContactName(context.Background(), jid, data.Name, data.Name)
					if err != nil {
						sendEvent(conn, "new_contact_status", map[string]interface{}{"status": "error", "error": err.Error()})
						return
					}
					sendEvent(conn, "new_contact_status", map[string]interface{}{"status": "success", "jid": jid.String(), "name": data.Name})
					chats, err := client.Store.Contacts.GetAllContacts(context.Background())
					if err == nil {
						sendEvent(conn, "chats", chats)
					}
				}()
			}
		case "create_group":
			var group_data struct {
				Name         string   `json:"name"`
				Participants []string `json:"participants"`
			}
			if err := json.Unmarshal(cmd.Data, &group_data); err == nil {
				go func() {
					seen_jids := make(map[string]bool)
					var participants_jids []types.JID
					for _, participant := range group_data.Participants {
						if parsed_jid, parse_err := types.ParseJID(participant); parse_err == nil && parsed_jid.User != "" {
							if !seen_jids[parsed_jid.String()] {
								seen_jids[parsed_jid.String()] = true
								participants_jids = append(participants_jids, parsed_jid)
							}
						} else {
							var phone_digits []rune
							for _, char := range participant {
								if char >= '0' && char <= '9' {
									phone_digits = append(phone_digits, char)
								}
							}
							phone_str := string(phone_digits)
							check_resp, check_err := client.IsOnWhatsApp(context.Background(), []string{phone_str})
							if check_err != nil || len(check_resp) == 0 || !check_resp[0].IsIn {
								sendEvent(conn, "create_group_status", map[string]interface{}{"status": "error", "error": fmt.Sprintf("Participant %s is not on WhatsApp", participant)})
								return
							}
							resolved_jid := check_resp[0].JID
							if !seen_jids[resolved_jid.String()] {
								seen_jids[resolved_jid.String()] = true
								participants_jids = append(participants_jids, resolved_jid)
							}
						}
					}
					create_resp, create_err := client.CreateGroup(context.Background(), whatsmeow.ReqCreateGroup{
						Name:         group_data.Name,
						Participants: participants_jids,
					})
					if create_err != nil {
						sendEvent(conn, "create_group_status", map[string]interface{}{"status": "error", "error": create_err.Error()})
						return
					}
					sendEvent(conn, "create_group_status", map[string]interface{}{"status": "success", "jid": create_resp.JID.String()})
					all_chats, all_chats_err := client.Store.Contacts.GetAllContacts(context.Background())
					if all_chats_err == nil {
						sendEvent(conn, "chats", all_chats)
					}
				}()
			}

		case "get_chats":
			go func() {
				rows, err := db.Query(`SELECT jid, name, timestamp, unread_count, pinned, muted, preview_snippet, archived FROM chats ORDER BY pinned DESC, timestamp DESC, name ASC`)
				if err != nil {
					fmt.Println("Error querying chats:", err)
					return
				}
				defer rows.Close()

				var chats []map[string]interface{}
				for rows.Next() {
					var jid string
					var name sql.NullString
					var ts int64
					var unread int
					var pinned int
					var muted int
					var preview sql.NullString
					var archived int

					if err := rows.Scan(&jid, &name, &ts, &unread, &pinned, &muted, &preview, &archived); err == nil {
						n := jid
						if name.Valid && name.String != "" {
							n = name.String
						}
						
						parsedJID, errJid := types.ParseJID(jid)
						if errJid == nil {
							contactJID := parsedJID
							if parsedJID.Server == "lid" {
								if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), parsedJID); err == nil && !pn.IsEmpty() {
									contactJID = pn
								}
							}
							
							contactInfo, errContact := client.Store.Contacts.GetContact(context.Background(), contactJID)
							if errContact == nil && contactInfo.Found {
								if contactInfo.FullName != "" {
									n = contactInfo.FullName
								} else if contactInfo.PushName != "" {
									n = contactInfo.PushName
								} else if contactInfo.BusinessName != "" {
									n = contactInfo.BusinessName
								}
							} else if parsedJID.Server != "g.us" && parsedJID.Server != "lid" {
								if n == jid {
									n = "+" + parsedJID.User
								}
							}
							
							chatSettings, errSettings := client.Store.ChatSettings.GetChatSettings(context.Background(), parsedJID)
							if errSettings == nil && chatSettings.Found {
								if chatSettings.Archived {
									archived = 1
								}
								if chatSettings.Pinned {
									pinned = 1
								}
								if !chatSettings.MutedUntil.IsZero() && chatSettings.MutedUntil.After(time.Now()) {
									muted = 1
								}
							}
						}

						p := ""
						if preview.Valid {
							p = preview.String
						}
						chats = append(chats, map[string]interface{}{
							"JID":            jid,
							"FullName":       n,
							"Timestamp":      ts,
							"UnreadCount":    unread,
							"Pinned":         pinned == 1,
							"Muted":          muted == 1,
							"PreviewSnippet": p,
							"Archived":       archived == 1,
						})
					}
				}
				sendEvent(conn, "chats", chats)
			}()
		case "get_messages":
			var data struct {
				JID             string `json:"jid"`
				Limit           int    `json:"limit"`
				BeforeTimestamp int64  `json:"before_timestamp"`
				BeforeID        string `json:"before_id"`
			}
			if err := json.Unmarshal(cmd.Data, &data); err == nil {
				go func() {
					db.Exec("UPDATE chats SET unread_count = 0 WHERE jid = ?", data.JID)
					broadcastChats()

					limit := data.Limit
					if limit <= 0 {
						limit = 50
					}

					var rows *sql.Rows
					var queryErr error
					if data.BeforeTimestamp > 0 && data.BeforeID != "" {
						rows, queryErr = db.Query(`SELECT id, text, timestamp, from_me, COALESCE(sender_jid, '') FROM messages WHERE jid = ? AND (timestamp < ? OR (timestamp = ? AND id < ?)) ORDER BY timestamp DESC, id DESC LIMIT ?`, data.JID, data.BeforeTimestamp, data.BeforeTimestamp, data.BeforeID, limit)
					} else if data.BeforeTimestamp > 0 {
						rows, queryErr = db.Query(`SELECT id, text, timestamp, from_me, COALESCE(sender_jid, '') FROM messages WHERE jid = ? AND timestamp < ? ORDER BY timestamp DESC LIMIT ?`, data.JID, data.BeforeTimestamp, limit)
					} else {
						rows, queryErr = db.Query(`SELECT id, text, timestamp, from_me, COALESCE(sender_jid, '') FROM messages WHERE jid = ? ORDER BY timestamp DESC LIMIT ?`, data.JID, limit)
					}

					var msgs []map[string]interface{}
					if queryErr == nil {
						defer rows.Close()
						for rows.Next() {
							var id string
							var text string
							var ts int64
							var fromMe bool
							var senderJID string
							if err := rows.Scan(&id, &text, &ts, &fromMe, &senderJID); err == nil {
								senderName := ""
								if senderJID != "" {
									if parsedSender, errP := types.ParseJID(senderJID); errP == nil {
										if client != nil && client.Store != nil && client.Store.Contacts != nil {
											if c, errC := client.Store.Contacts.GetContact(context.Background(), parsedSender); errC == nil && c.Found {
												if c.FullName != "" {
													senderName = c.FullName
												} else if c.PushName != "" {
													senderName = c.PushName
												} else if c.BusinessName != "" {
													senderName = c.BusinessName
												}
											}
										}
										if senderName == "" {
											senderName = "+" + parsedSender.User
										}
									}
								}
								msgs = append(msgs, map[string]interface{}{
									"id":               id,
									"text":             text,
									"timestamp":        ts,
									"fromMe":           fromMe,
									"from_me":          fromMe,
									"senderJid":        senderJID,
									"sender_jid":       senderJID,
									"senderName":       senderName,
									"sender_name":      senderName,
								})
							}
						}
					}

					for i, j := 0, len(msgs)-1; i < j; i, j = i+1, j-1 {
						msgs[i], msgs[j] = msgs[j], msgs[i]
					}

					if msgs == nil {
						msgs = []map[string]interface{}{}
					}

					sendEvent(conn, "conversation_messages", map[string]interface{}{
						"jid":              data.JID,
						"before_timestamp": data.BeforeTimestamp,
						"messages":         msgs,
					})
				}()
			}
		case "search_messages":
			var data struct {
				JID   string `json:"jid"`
				Query string `json:"query"`
			}
			if err := json.Unmarshal(cmd.Data, &data); err == nil {
				go func() {
					// TODO: Implement actual search
					var results []map[string]interface{}
					results = append(results, map[string]interface{}{"from": "System", "text": fmt.Sprintf("Search for '%s' not yet implemented.", data.Query)})
					sendEvent(conn, "search_results", map[string]interface{}{"results": results})
				}()
			}
		case "update_settings":
			fmt.Println("Received update_settings command:", string(cmd.Data))
			sendEvent(conn, "settings_updated", map[string]interface{}{"status": "success"})
		case "start_recording":
			fmt.Println("start_recording triggered")
		case "toggle_pause_recording":
			fmt.Println("toggle_pause_recording triggered")
		case "send_voice_message":
			fmt.Println("send_voice_message triggered")
		case "discard_voice_message":
			fmt.Println("discard_voice_message triggered")
		case "send_attachment":
			fmt.Println("send_attachment triggered")
		case "download_media":
			fmt.Println("download_media triggered")
			go func() {
				// Stub: simulate a successful media download
				sendEvent(conn, "media_downloaded", map[string]interface{}{"path": "C:\\Downloads\\media_file.jpg"})
			}()
		case "open_media":
			fmt.Println("open_media triggered")
		case "save_media_as":
			fmt.Println("save_media_as triggered")
		case "send_typing":
			var data struct {
				JID   string `json:"jid"`
				State string `json:"state"`
				Media string `json:"media"`
			}
			if err := json.Unmarshal(cmd.Data, &data); err == nil {
				jid, err := types.ParseJID(data.JID)
				if err == nil {
					pState := types.ChatPresenceComposing
					if data.State == "paused" {
						pState = types.ChatPresencePaused
					}
					go client.SendChatPresence(context.Background(), jid, pState, types.ChatPresenceMedia(data.Media))
				}
			}
		case "subscribe_presence":
			var data struct {
				JID string `json:"jid"`
			}
			if err := json.Unmarshal(cmd.Data, &data); err == nil {
				jid, err := types.ParseJID(data.JID)
				if err == nil {
					go client.SubscribePresence(context.Background(), jid)
				}
			}
		case "mark_read":
			var data struct {
				JID        string   `json:"jid"`
				MessageIDs []string `json:"message_ids"`
			}
			if err := json.Unmarshal(cmd.Data, &data); err == nil {
				chatJID, err := types.ParseJID(data.JID)
				if err == nil {
					var ids []types.MessageID
					for _, id := range data.MessageIDs {
						ids = append(ids, types.MessageID(id))
					}
					if len(ids) > 0 {
						go client.MarkRead(context.Background(), ids, time.Now(), chatJID, types.EmptyJID)
					}
				}
			}
		}
	}
}

func broadcastChats() {
	rows, err := db.Query(`SELECT jid, name, timestamp, unread_count, pinned, muted, preview_snippet, archived FROM chats ORDER BY pinned DESC, timestamp DESC, name ASC`)
	if err != nil {
		fmt.Println("Error querying chats:", err)
		return
	}
	defer rows.Close()

	var chats []map[string]interface{}
	for rows.Next() {
		var jid string
		var name sql.NullString
		var ts int64
		var unread int
		var pinned int
		var muted int
		var preview sql.NullString
		var archived int

		if err := rows.Scan(&jid, &name, &ts, &unread, &pinned, &muted, &preview, &archived); err == nil {
			n := jid
			if name.Valid && name.String != "" {
				n = name.String
			}
			
			parsedJID, errJid := types.ParseJID(jid)
			if errJid == nil {
				contactJID := parsedJID
				if parsedJID.Server == "lid" {
					if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), parsedJID); err == nil && !pn.IsEmpty() {
						contactJID = pn
					}
				}
				
				contactInfo, errContact := client.Store.Contacts.GetContact(context.Background(), contactJID)
				if errContact == nil && contactInfo.Found {
					if contactInfo.FullName != "" {
						n = contactInfo.FullName
					} else if contactInfo.PushName != "" {
						n = contactInfo.PushName
					} else if contactInfo.BusinessName != "" {
						n = contactInfo.BusinessName
					}
				} else if parsedJID.Server != "g.us" && parsedJID.Server != "lid" {
					if n == jid {
						n = "+" + parsedJID.User
					}
				}
				
				chatSettings, errSettings := client.Store.ChatSettings.GetChatSettings(context.Background(), parsedJID)
				if errSettings == nil && chatSettings.Found {
					if chatSettings.Archived {
						archived = 1
					}
					if chatSettings.Pinned {
						pinned = 1
					}
					if !chatSettings.MutedUntil.IsZero() && chatSettings.MutedUntil.After(time.Now()) {
						muted = 1
					}
				}
			}

			p := ""
			if preview.Valid {
				p = preview.String
			}
			chats = append(chats, map[string]interface{}{
				"JID":            jid,
				"FullName":       n,
				"Timestamp":      ts,
				"UnreadCount":    unread,
				"Pinned":         pinned == 1,
				"Muted":          muted == 1,
				"PreviewSnippet": p,
				"Archived":       archived == 1,
			})
		}
	}
	broadcastEvent("chats", chats)
}

func broadcastEvent(evtType string, data interface{}) {
	wsMutex.Lock()
	defer wsMutex.Unlock()

	var raw json.RawMessage
	if data != nil {
		bytes, err := json.Marshal(data)
		if err == nil {
			raw = bytes
		}
	}

	evt := Event{
		Type: evtType,
		Data: raw,
	}

	bytes, err := json.Marshal(evt)
	if err != nil {
		return
	}

	for conn := range wsClients {
		conn.WriteMessage(websocket.TextMessage, bytes)
	}
}

func sendEvent(conn *websocket.Conn, evtType string, data interface{}) {
	wsMutex.Lock()
	defer wsMutex.Unlock()
	var raw json.RawMessage
	if data != nil {
		bytes, err := json.Marshal(data)
		if err == nil {
			raw = bytes
		}
	}

	evt := Event{
		Type: evtType,
		Data: raw,
	}

	bytes, err := json.Marshal(evt)
	if err != nil {
		return
	}

	conn.WriteMessage(websocket.TextMessage, bytes)
}
