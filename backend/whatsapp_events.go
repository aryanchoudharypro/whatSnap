package main

import (
	"context"
	"fmt"
	"log"
	"strings"
	"sync"
	"time"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/proto/waWeb"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

func resolveLIDChat(v *events.Message) (chatJID, senderJID types.JID) {
	chatJID = v.Info.Chat.ToNonAD()
	senderJID = v.Info.Sender.ToNonAD()

	if senderJID.Server == "lid" {
		if !v.Info.SenderAlt.IsEmpty() {
			senderJID = v.Info.SenderAlt.ToNonAD()
		} else if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), senderJID); err == nil && !pn.IsEmpty() {
			senderJID = pn.ToNonAD()
		} else if v.Info.IsFromMe && client.Store.ID != nil {
			senderJID = client.Store.ID.ToNonAD()
		}
	}

	if chatJID.Server == "lid" {
		if v.Info.IsFromMe {
			if !v.Info.RecipientAlt.IsEmpty() {
				chatJID = v.Info.RecipientAlt.ToNonAD()
			} else if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), chatJID); err == nil && !pn.IsEmpty() {
				chatJID = pn.ToNonAD()
			}
		} else {
			chatJID = senderJID
		}
	} else if chatJID.Server == types.BroadcastServer && chatJID.User != "status" && !v.Info.IsFromMe {
		// Incoming broadcast-list messages belong in the sender's 1:1 chat,
		// like official WhatsApp, not in a chat keyed by the broadcast JID.
		chatJID = senderJID
	}

	return
}

func extractText(msg *waE2E.Message) string {
	if msg == nil {
		return ""
	}
	if msg.GetExtendedTextMessage() != nil {
		return msg.GetExtendedTextMessage().GetText()
	}
	if msg.GetDeviceSentMessage() != nil && msg.GetDeviceSentMessage().GetMessage() != nil {
		return extractText(msg.GetDeviceSentMessage().GetMessage())
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
		caption := msg.GetDocumentMessage().GetCaption()
		title := msg.GetDocumentMessage().GetTitle()
		if title == "" {
			title = msg.GetDocumentMessage().GetFileName()
		}
		if caption != "" {
			if title != "" {
				return "[Document] " + title + ": " + caption
			}
			return "[Document] " + caption
		}
		if title != "" {
			return "[Document] " + title
		}
		return "[Document]"
	}
	if msg.GetAudioMessage() != nil {
		audio := msg.GetAudioMessage()
		sec := audio.GetSeconds()
		durationStr := ""
		if sec > 0 {
			m := sec / 60
			s := sec % 60
			durationStr = fmt.Sprintf(" (%d:%02d)", m, s)
		}
		if audio.GetPTT() {
			return "[Voice Message]" + durationStr
		}
		return "[Audio]" + durationStr
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

// getContextInfo pulls the ContextInfo (which carries reply/quote data) out
// of whichever message type actually has it populated.
func getContextInfo(msg *waE2E.Message) *waE2E.ContextInfo {
	if msg == nil {
		return nil
	}
	switch {
	case msg.GetExtendedTextMessage() != nil:
		return msg.GetExtendedTextMessage().GetContextInfo()
	case msg.GetImageMessage() != nil:
		return msg.GetImageMessage().GetContextInfo()
	case msg.GetVideoMessage() != nil:
		return msg.GetVideoMessage().GetContextInfo()
	case msg.GetDocumentMessage() != nil:
		return msg.GetDocumentMessage().GetContextInfo()
	case msg.GetAudioMessage() != nil:
		return msg.GetAudioMessage().GetContextInfo()
	case msg.GetStickerMessage() != nil:
		return msg.GetStickerMessage().GetContextInfo()
	}
	return nil
}

// quotedPreview produces the small denormalized snapshot we store alongside
// a message that replies to another one, so the frontend can render
// "replying to: ..." without a second lookup.
func quotedPreview(msg *waE2E.Message) (quotedID, quotedText, quotedSender string) {
	if ctx := getContextInfo(msg); ctx != nil && ctx.GetStanzaID() != "" {
		quotedID = ctx.GetStanzaID()
		if participant := ctx.GetParticipant(); participant != "" {
			if parsed, err := types.ParseJID(participant); err == nil {
				quotedSender = resolveDisplayName(parsed)
			} else {
				quotedSender = participant
			}
		}
		if qm := ctx.GetQuotedMessage(); qm != nil {
			quotedText = extractText(qm)
		}
	}
	return
}

// mediaInfoFromMessage extracts a normalized (mediaType, mimetype, filename)
// triple for the media-bearing message kinds, so we can tag rows in the
// messages table for the frontend without it having to inspect raw protobuf.
func mediaInfoFromMessage(msg *waE2E.Message) (mediaType, mimetype, filename string) {
	if msg == nil {
		return "", "", ""
	}
	switch {
	case msg.GetImageMessage() != nil:
		return "image", msg.GetImageMessage().GetMimetype(), ""
	case msg.GetVideoMessage() != nil:
		return "video", msg.GetVideoMessage().GetMimetype(), ""
	case msg.GetAudioMessage() != nil:
		if msg.GetAudioMessage().GetPTT() {
			return "voice", msg.GetAudioMessage().GetMimetype(), ""
		}
		return "audio", msg.GetAudioMessage().GetMimetype(), ""
	case msg.GetDocumentMessage() != nil:
		name := msg.GetDocumentMessage().GetFileName()
		if name == "" {
			name = msg.GetDocumentMessage().GetTitle()
		}
		return "document", msg.GetDocumentMessage().GetMimetype(), sanitizeFilename(name)
	case msg.GetStickerMessage() != nil:
		return "sticker", msg.GetStickerMessage().GetMimetype(), ""
	}
	return "", "", ""
}

func isIgnoredChatJID(jid string) bool {
	return jid == "status@broadcast" || strings.HasSuffix(jid, "@broadcast") || strings.HasSuffix(jid, "@newsletter")
}

var historySyncMutex sync.Mutex

func eventHandler(evt interface{}) {
	switch evt.(type) {
	case *events.Message, *events.Receipt, *events.HistorySync:
		log.Printf("Event received in eventHandler: %T", evt)
	}

	switch v := evt.(type) {
	case *events.MediaRetry:
		go handleMediaRetryEvent(v)
	case *events.HistorySync:
		go func() {
			historySyncMutex.Lock()
			defer historySyncMutex.Unlock()

			if v.Data == nil {
				return
			}

			for _, mapping := range v.Data.GetPhoneNumberToLidMappings() {
				if mapping == nil {
					continue
				}
				pnJID, err1 := types.ParseJID(mapping.GetPnJID())
				lidJID, err2 := types.ParseJID(mapping.GetLidJID())
				if err1 == nil && err2 == nil && !pnJID.IsEmpty() && !lidJID.IsEmpty() {
					client.Store.LIDs.PutLIDMapping(context.Background(), lidJID, pnJID)
					db.Exec(`UPDATE chats SET jid = ? WHERE jid = ?`, pnJID.String(), lidJID.String())
					db.Exec(`UPDATE messages SET jid = ? WHERE jid = ?`, pnJID.String(), lidJID.String())
					db.Exec(`UPDATE messages SET sender_jid = ? WHERE sender_jid = ?`, pnJID.String(), lidJID.String())
				}
			}

			for _, p := range v.Data.GetPushnames() {
				if p == nil {
					continue
				}
				idStr := p.GetID()
				pushname := p.GetPushname()
				if idStr == "" || pushname == "" {
					continue
				}
				parsedJID, err := types.ParseJID(idStr)
				if err != nil {
					continue
				}
				targetJID := parsedJID.ToNonAD()
				if targetJID.Server == "lid" {
					if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), targetJID); err == nil && !pn.IsEmpty() {
						targetJID = pn.ToNonAD()
					}
				}
				client.Store.Contacts.PutPushName(context.Background(), targetJID, pushname)
				upsertContactCacheRow(targetJID.String(), pushname, targetJID.User, false)
				db.Exec(`UPDATE chats SET name = ? WHERE jid = ? AND (name IS NULL OR name = '' OR name LIKE '+%')`, pushname, targetJID.String())
			}

			for _, conv := range v.Data.GetConversations() {
				tx, txErr := db.Begin()
				if txErr != nil {
					fmt.Println("Error starting transaction for HistorySync:", txErr)
					continue
				}

				jid := conv.GetID()
				if isIgnoredChatJID(jid) {
					tx.Rollback()
					continue
				}
				chatJID, parseErr := types.ParseJID(jid)
				if parseErr != nil {
					tx.Rollback()
					continue
				}
				chatJID = chatJID.ToNonAD()
				if chatJID.Server == "lid" {
					if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), chatJID); err == nil && !pn.IsEmpty() {
						chatJID = pn.ToNonAD()
					}
				}
				jid = chatJID.String()

				name := conv.GetName()
				if name == "" {
					if contact, err := client.Store.Contacts.GetContact(context.Background(), chatJID); err == nil && contact.Found {
						if contact.FullName != "" {
							name = contact.FullName
						} else if contact.PushName != "" {
							name = contact.PushName
						}
					}
				}

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

				for i, historyMsg := range conv.GetMessages() {
					if i > 0 && i%200 == 0 {
						tx.Commit()
						time.Sleep(5 * time.Millisecond)
						tx, _ = db.Begin()
					}
					webMsg := historyMsg.GetMessage()
					if webMsg == nil {
						continue
					}
					if webMsg.GetMessageStubType() == waWeb.WebMessageInfo_REVOKE {
						stubID := webMsg.GetKey().GetID()
						if stubID != "" {
							stubFromMe := webMsg.GetKey().GetFromMe()
							stubTS := int64(webMsg.GetMessageTimestamp())
							if _, err := tx.Exec(`INSERT INTO messages (id, jid, from_me, text, timestamp, is_deleted)
									VALUES (?, ?, ?, '', ?, 1)
									ON CONFLICT(id) DO UPDATE SET is_deleted=1, text=''`,
								stubID, jid, stubFromMe, stubTS); err != nil {
								log.Println("Error storing revoked stub from HistorySync:", err)
							}
						}
						continue
					}
					evtMsg, parseEvtErr := client.ParseWebMessage(chatJID, webMsg)
					if parseEvtErr != nil || evtMsg == nil {
						msgID := webMsg.GetKey().GetID()
						if msgID != "" {
							msgTS := int64(webMsg.GetMessageTimestamp())
							msgFromMe := webMsg.GetKey().GetFromMe()
							if _, err := tx.Exec(`INSERT INTO messages (id, jid, from_me, text, timestamp)
									VALUES (?, ?, ?, '[Unsupported Message]', ?)
									ON CONFLICT(id) DO UPDATE SET text='[Unsupported Message]'`,
								msgID, jid, msgFromMe, msgTS); err != nil {
								log.Println("Error storing fallback unsupported message:", err)
							}
						}
						continue
					}
					if pm := evtMsg.Message.GetProtocolMessage(); pm != nil && pm.GetType() == waE2E.ProtocolMessage_REVOKE {
						if targetID := pm.GetKey().GetID(); targetID != "" {
							if _, err := tx.Exec(`INSERT INTO messages (id, jid, from_me, text, timestamp, is_deleted)
									VALUES (?, ?, ?, '', ?, 1)
									ON CONFLICT(id) DO UPDATE SET is_deleted=1, text=''`,
								targetID, jid, evtMsg.Info.IsFromMe, evtMsg.Info.Timestamp.Unix()); err != nil {
								log.Println("Error storing revoke from HistorySync:", err)
							}
						}
						continue
					}
					if reaction := evtMsg.Message.GetReactionMessage(); reaction != nil {
						targetID := reaction.GetKey().GetID()
						emoji := reaction.GetText()
						reactorJID := evtMsg.Info.Sender.ToNonAD().String()
						if evtMsg.Info.IsFromMe {
							reactorJID = "me"
						}
						ts := reaction.GetSenderTimestampMS() / 1000
						if ts == 0 {
							ts = evtMsg.Info.Timestamp.Unix()
						}
						if emoji == "" {
							tx.Exec(`DELETE FROM reactions WHERE message_id = ? AND (reactor_jid = ? OR reactor_jid = 'me')`, targetID, reactorJID)
						} else {
							tx.Exec(`INSERT INTO reactions (message_id, chat_jid, reactor_jid, emoji, timestamp) VALUES (?, ?, ?, ?, ?)
								ON CONFLICT(message_id, reactor_jid) DO UPDATE SET emoji = excluded.emoji, timestamp = excluded.timestamp`,
								targetID, jid, reactorJID, emoji, ts)
						}
						continue
					}

					msgText := extractText(evtMsg.Message)
					if msgText == "[Protocol Message]" {
						continue
					}
					msgID := evtMsg.Info.ID
					if msgID == "" && webMsg.Key != nil {
						msgID = webMsg.GetKey().GetID()
					}
					if msgID != "" {
						msgJID := evtMsg.Info.Chat.ToNonAD().String()
						if msgJID == "" || strings.HasSuffix(msgJID, "@lid") {
							msgJID = jid
						}
						msgFromMe := evtMsg.Info.IsFromMe
						msgTS := evtMsg.Info.Timestamp.Unix()
						if msgTS <= 0 && webMsg.MessageTimestamp != nil {
							msgTS = int64(webMsg.GetMessageTimestamp())
						}
						senderJID := evtMsg.Info.Sender.ToNonAD()
						if senderJID.Server == "lid" {
							if !evtMsg.Info.SenderAlt.IsEmpty() {
								senderJID = evtMsg.Info.SenderAlt.ToNonAD()
							} else if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), senderJID); err == nil && !pn.IsEmpty() {
								senderJID = pn.ToNonAD()
							}
						}
						rawBytes, _ := proto.Marshal(evtMsg.Message)
						mediaType, mimetype, filename := mediaInfoFromMessage(evtMsg.Message)
						quotedID, quotedText, quotedSender := quotedPreview(evtMsg.Message)
						status := ""
						var sentAt, deliveredAt, readAt, playedAt int64
						if msgFromMe {
							sentAt = msgTS
							for _, ur := range webMsg.GetUserReceipt() {
								if ts := ur.GetReceiptTimestamp(); ts > deliveredAt {
									deliveredAt = ts
								}
								if ts := ur.GetReadTimestamp(); ts > readAt {
									readAt = ts
								}
								if ts := ur.GetPlayedTimestamp(); ts > playedAt {
									playedAt = ts
								}
							}
							stStr := strings.ToLower(webMsg.GetStatus().String())
							switch {
							case strings.Contains(stStr, "played"):
								status = "played"
							case strings.Contains(stStr, "read"):
								status = "read"
							case strings.Contains(stStr, "delivery") || strings.Contains(stStr, "delivered"):
								status = "delivered"
							case strings.Contains(stStr, "server") || strings.Contains(stStr, "sent"):
								status = "sent"
							default:
								if webMsg.GetStatus() > 0 {
									status = "sent"
								}
							}
						}
						_, err := tx.Exec(`INSERT INTO messages
								(id, jid, from_me, text, timestamp, sender_jid, raw, status, media_type, media_mimetype, media_filename, quoted_id, quoted_text, quoted_sender, sent_at, delivered_at, read_at, played_at)
							VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
							ON CONFLICT(id) DO UPDATE SET text=excluded.text, timestamp=excluded.timestamp, sender_jid=excluded.sender_jid, raw=excluded.raw,
								status=CASE WHEN excluded.status != '' THEN excluded.status ELSE messages.status END,
								media_type=excluded.media_type, media_mimetype=excluded.media_mimetype, media_filename=excluded.media_filename,
								quoted_id=excluded.quoted_id, quoted_text=excluded.quoted_text, quoted_sender=excluded.quoted_sender,
								sent_at=CASE WHEN excluded.sent_at != 0 THEN excluded.sent_at ELSE messages.sent_at END,
								delivered_at=CASE WHEN messages.delivered_at != 0 THEN messages.delivered_at ELSE excluded.delivered_at END,
								read_at=CASE WHEN messages.read_at != 0 THEN messages.read_at ELSE excluded.read_at END,
								played_at=CASE WHEN messages.played_at != 0 THEN messages.played_at ELSE excluded.played_at END`,
							msgID, msgJID, msgFromMe, msgText, msgTS, senderJID.String(), rawBytes, status,
							mediaType, mimetype, filename, quotedID, quotedText, quotedSender, sentAt, deliveredAt, readAt, playedAt)
						if err != nil {
							log.Println("Error inserting message from HistorySync:", err)
						}
					}
				}
				if err := tx.Commit(); err != nil {
					log.Println("Error committing HistorySync transaction for conv:", err)
				}
			}
			UpdateAllChatSnippets()
			populateChatsFromStore()
			broadcastChats()
		}()

	case *events.Message:
		if v.IsEdit {
			targetID := v.Message.GetProtocolMessage().GetKey().GetID()
			newContent := v.Message.GetProtocolMessage().GetEditedMessage()
			newText := extractText(newContent)
			resolvedChat, _ := resolveLIDChat(v)
			jid := resolvedChat.String()
			editTime := v.Info.Timestamp.Unix()
			go func() {
				rawBytes, _ := proto.Marshal(newContent)
				markMessageEdited(targetID, jid, newText, rawBytes, editTime)
			}()
			broadcastEvent("message_edited", map[string]interface{}{
				"jid":        jid,
				"message_id": targetID,
				"text":       newText,
				"edited_at":  editTime,
			})
			return
		}
		if v.Message.GetProtocolMessage() != nil && v.Message.GetProtocolMessage().GetType() == waE2E.ProtocolMessage_REVOKE {
			targetID := v.Message.GetProtocolMessage().GetKey().GetID()
			resolvedChat, _ := resolveLIDChat(v)
			jid := resolvedChat.String()
			go markMessageDeleted(targetID, jid)
			broadcastEvent("message_deleted", map[string]interface{}{
				"jid":        jid,
				"message_id": targetID,
				"by":         v.Info.Sender.String(),
			})
			return
		}
		if reaction := v.Message.GetReactionMessage(); reaction != nil {
			targetID := reaction.GetKey().GetID()
			chatJID, senderJID := resolveLIDChat(v)
			jid := chatJID.ToNonAD().String()
			emoji := reaction.GetText()

			reactorJID := senderJID.ToNonAD().String()
			reactorName := resolveDisplayName(senderJID)
			if v.Info.IsFromMe {
				reactorJID = "me"
				reactorName = "You"
			}

			ts := reaction.GetSenderTimestampMS() / 1000
			if ts == 0 {
				ts = v.Info.Timestamp.Unix()
			}

			go upsertReaction(targetID, jid, reactorJID, emoji, ts)
			broadcastEvent("message_reaction", map[string]interface{}{
				"jid":          jid,
				"message_id":   targetID,
				"reactor":      reactorJID,
				"reactor_name": reactorName,
				"emoji":        emoji,
			})
			return
		}

		text := extractText(v.Message)
		if text == "[Protocol Message]" {
			return
		}
		log.Printf("Received events.Message. ID: %s, Sender: %s, Chat: %s, text length: %d", v.Info.ID, v.Info.Sender.String(), v.Info.Chat.String(), len(text))

		go func() {
			chatJID, senderJID := resolveLIDChat(v)

			jid := chatJID.String()

			if isIgnoredChatJID(jid) {
				return
			}
			ts := v.Info.Timestamp.Unix()
			increment := 1
			if v.Info.IsFromMe {
				increment = 0
			}
			_, err := db.Exec(`INSERT INTO chats (jid, timestamp, unread_count, preview_snippet, preview_sender, preview_from_me) VALUES (?, ?, ?, ?, ?, ?)
				ON CONFLICT(jid) DO UPDATE SET
					timestamp=MAX(timestamp, excluded.timestamp),
					unread_count=chats.unread_count + excluded.unread_count,
					preview_snippet=COALESCE(NULLIF(excluded.preview_snippet, ''), chats.preview_snippet),
					preview_sender=excluded.preview_sender,
					preview_from_me=excluded.preview_from_me`,
				jid, ts, increment, text, senderJID.String(), v.Info.IsFromMe)
			if err != nil {
				fmt.Println("Error updating chat timestamp:", err)
			}
			if v.Info.ID != "" {
				mediaType, mimetype, filename := mediaInfoFromMessage(v.Message)
				quotedID, quotedText, quotedSender := quotedPreview(v.Message)
				status := ""
				if v.Info.IsFromMe {
					status = "sent"
				}
				rawBytes, _ := proto.Marshal(v.Message)
				err := saveMessage(StoredMessage{
					ID: v.Info.ID, JID: jid, SenderJID: senderJID.String(),
					FromMe: v.Info.IsFromMe, Text: text, Timestamp: ts, Raw: rawBytes, Status: status,
					MediaType: mediaType, MediaMimetype: mimetype, MediaFilename: filename,
					QuotedID: quotedID, QuotedText: quotedText, QuotedSender: quotedSender,
				})
				if err != nil {
					fmt.Println("Error inserting message from live event:", err)
				}
			}
			broadcastChats()
		}()
		if text != "" {
			chatJID, senderJID := resolveLIDChat(v)
			liveMediaType, _, liveMediaFilename := mediaInfoFromMessage(v.Message)

			quotedID, quotedText, quotedSender := quotedPreview(v.Message)
			broadcastEvent("message", map[string]interface{}{
				"jid":           chatJID.String(),
				"from":          senderJID.String(),
				"id":            v.Info.ID,
				"text":          text,
				"from_me":       v.Info.IsFromMe,
				"timestamp":     v.Info.Timestamp.Unix(),
				"has_media":     v.Message.GetImageMessage() != nil || v.Message.GetVideoMessage() != nil || v.Message.GetAudioMessage() != nil || v.Message.GetDocumentMessage() != nil || v.Message.GetStickerMessage() != nil,
				"mediaType":     liveMediaType,
				"mediaFilename": liveMediaFilename,
				"quotedId":      quotedID,
				"quotedText":    quotedText,
				"quotedSender":  quotedSender,
			})
		}
	case *events.PushName:
		go func() {
			jidStr := v.JID.ToNonAD().String()
			if v.NewPushName != "" {
				client.Store.Contacts.PutPushName(context.Background(), v.JID.ToNonAD(), v.NewPushName)
				upsertContactCacheRow(jidStr, v.NewPushName, v.JID.User, false)
				db.Exec(`UPDATE chats SET name = ? WHERE jid = ? AND (name IS NULL OR name = '' OR name LIKE '+%')`, v.NewPushName, jidStr)
				broadcastChats()
			}
		}()
	case *events.Contact:
		go func() {
			jidStr := v.JID.ToNonAD().String()
			fullName := ""
			if v.Action != nil {
				fullName = v.Action.GetFullName()
				if fullName == "" {
					fullName = v.Action.GetFirstName()
				}
			}
			if fullName != "" {
				client.Store.Contacts.PutContactName(context.Background(), v.JID.ToNonAD(), fullName, fullName)
				upsertContactCacheRow(jidStr, fullName, v.JID.User, false)
				db.Exec(`UPDATE chats SET name = ? WHERE jid = ?`, fullName, jidStr)
				broadcastChats()
			}
		}()
	case *events.BusinessName:
		go func() {
			jidStr := v.JID.ToNonAD().String()
			if v.NewBusinessName != "" {
				upsertContactCacheRow(jidStr, v.NewBusinessName, v.JID.User, true)
				db.Exec(`UPDATE chats SET name = ? WHERE jid = ? AND (name IS NULL OR name = '' OR name LIKE '+%')`, v.NewBusinessName, jidStr)
				broadcastChats()
			}
		}()
	case *events.OfflineSyncCompleted:
		log.Printf("OfflineSyncCompleted event received. Count: %d", v.Count)
		go func() {
			UpdateAllChatSnippets()
			populateChatsFromStore()
			broadcastChats()
		}()
	case *events.DeleteForMe:
		jid := v.ChatJID.String()
		go markMessageDeleted(v.MessageID, jid)
		broadcastEvent("message_deleted", map[string]interface{}{
			"jid":        jid,
			"message_id": v.MessageID,
			"by":         v.SenderJID.String(),
		})
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
		log.Printf("Successfully paired with %s", v.ID.String())
		broadcastEvent("pair_success", map[string]string{"jid": v.ID.String()})
		go client.SendPresence(context.Background(), types.PresenceAvailable)
		go func() {
			time.Sleep(5 * time.Second)
			populateChatsFromStore()
			broadcastChats()
		}()
	case *events.LoggedOut:
		broadcastEvent("disconnected", nil)
	case *events.ChatPresence:
		isComposing := v.State == types.ChatPresenceComposing
		isRecording := isComposing && v.Media == types.ChatPresenceMediaAudio
		isTyping := isComposing && !isRecording

		chatJID := v.MessageSource.Chat.ToNonAD()
		senderJID := v.MessageSource.Sender.ToNonAD()

		if chatJID.Server == "lid" {
			if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), chatJID); err == nil && !pn.IsEmpty() {
				chatJID = pn.ToNonAD()
			}
		}
		if senderJID.Server == "lid" {
			if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), senderJID); err == nil && !pn.IsEmpty() {
				senderJID = pn.ToNonAD()
			}
		}

		broadcastEvent("typing", map[string]interface{}{
			"chat_jid":   chatJID.String(),
			"sender_jid": senderJID.String(),
			"state":      string(v.State),
			"media":      string(v.Media),
			"typing":     isTyping,
			"recording":  isRecording,
		})
	case *events.Presence:
		lastSeenUnix := int64(0)
		if !v.LastSeen.IsZero() {
			lastSeenUnix = v.LastSeen.Unix()
		}

		targetJID := v.From.ToNonAD()
		if targetJID.Server == "lid" {
			if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), targetJID); err == nil && !pn.IsEmpty() {
				targetJID = pn.ToNonAD()
			}
		}

		broadcastEvent("presence", map[string]interface{}{
			"jid":         targetJID.String(),
			"unavailable": v.Unavailable,
			"last_seen":   lastSeenUnix,
		})
	case *events.Receipt:
		chatJID := v.MessageSource.Chat.ToNonAD()
		senderJID := v.MessageSource.Sender.ToNonAD()

		if chatJID.Server == "lid" {
			if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), chatJID); err == nil && !pn.IsEmpty() {
				chatJID = pn.ToNonAD()
			}
		}
		if senderJID.Server == "lid" {
			if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), senderJID); err == nil && !pn.IsEmpty() {
				senderJID = pn.ToNonAD()
			}
		}

		broadcastEvent("receipt", map[string]interface{}{
			"chat_jid":    chatJID.String(),
			"sender_jid":  senderJID.String(),
			"message_ids": v.MessageIDs,
			"type":        string(v.Type),
			"timestamp":   v.Timestamp.Unix(),
		})
		log.Printf("Received Receipt event - Chat: %s, Sender: %s, Type: %s, MessageIDs: %v", chatJID.String(), senderJID.String(), string(v.Type), v.MessageIDs)
		status := "delivered"
		if v.Type == types.ReceiptTypeRead || v.Type == types.ReceiptTypeReadSelf {
			status = "read"
		} else if v.Type == types.ReceiptTypePlayed {
			status = "played"
		} else if string(v.Type) == "sender" {
			status = "sent"
		} else if string(v.Type) == "inactive" {
			status = "delivered"
		} else if string(v.Type) != "" {
			status = string(v.Type)
		}
		go func() {
			ids := make([]string, len(v.MessageIDs))
			for i, id := range v.MessageIDs {
				ids[i] = string(id)
			}
			receiptTS := v.Timestamp.Unix()
			updated := updateMessageStatusIfNewer(ids, status, receiptTS)
			resolvedChatJID := v.MessageSource.Chat.ToNonAD()
			if resolvedChatJID.Server == "lid" {
				if pn, err := client.Store.LIDs.GetPNForLID(context.Background(), resolvedChatJID); err == nil && !pn.IsEmpty() {
					resolvedChatJID = pn.ToNonAD()
				}
			}
			if len(updated) > 0 {
				broadcastEvent("message_status", map[string]interface{}{
					"jid":         resolvedChatJID.String(),
					"message_ids": updated,
					"status":      status,
					"timestamp":   receiptTS,
				})
			}
			if v.Type == types.ReceiptTypeReadSelf {
				db.Exec("UPDATE chats SET unread_count = 0 WHERE jid = ?", resolvedChatJID.String())
				broadcastChats()
			}
		}()
	case *events.AppStateSyncComplete:
		if len(v.Name) > 0 {
			go func() {
				populateChatsFromStore()
				broadcastChats()
			}()
		}
	case *events.Picture:
		go func() {
			jidStr := v.JID.String()
			if v.Remove {
				updateContactAvatarCache(jidStr, "", "")
			}
			broadcastEvent("avatar_updated", map[string]interface{}{
				"jid":     jidStr,
				"removed": v.Remove,
			})
		}()
	case *events.UserAbout:
		go updateContactAboutCache(v.JID.String(), v.Status)
		broadcastEvent("about_updated", map[string]interface{}{
			"jid":    v.JID.String(),
			"status": v.Status,
		})
	case *events.GroupInfo:
		go func() {
			if v.Name != nil {
				db.Exec(`UPDATE chats SET name = ? WHERE jid = ?`, v.Name.Name, v.JID.String())
			}
		}()
		broadcastEvent("group_updated", map[string]interface{}{
			"jid": v.JID.String(),
		})
	case *events.JoinedGroup:
		go func() {
			populateChatsFromStore()
			broadcastChats()
		}()
		broadcastEvent("group_updated", map[string]interface{}{
			"jid": v.JID.String(),
		})
	case *events.Blocklist:
		go func() {
			for _, change := range v.Changes {
				setContactBlocked(change.JID.String(), change.Action == events.BlocklistChangeActionBlock)
			}
		}()
		broadcastEvent("blocklist_updated", nil)
	}
}

func populateChatsFromStore() {
	if client == nil || client.Store == nil || client.Store.Contacts == nil {
		return
	}

	allContacts, err := client.Store.Contacts.GetAllContacts(context.Background())
	if err != nil {
		fmt.Println("Error getting contacts from store:", err)
		return
	}

	for jid, contact := range allContacts {
		targetJID := jid.ToNonAD()
		if targetJID.Server == "lid" {
			if pn, pnErr := client.Store.LIDs.GetPNForLID(context.Background(), targetJID); pnErr == nil && !pn.IsEmpty() {
				targetJID = pn.ToNonAD()
			}
		}

		jidStr := targetJID.String()
		if isIgnoredChatJID(jidStr) {
			continue
		}

		name := contact.FullName
		if name == "" {
			name = contact.PushName
		}
		if name == "" {
			name = contact.BusinessName
		}

		if name == "" && targetJID.Server != "g.us" && targetJID.Server != "lid" {
			name = "+" + targetJID.User
		}

		phone := ""
		if targetJID.Server != "g.us" && targetJID.Server != "lid" {
			phone = targetJID.User
		}

		if name != "" || phone != "" {
			upsertContactCacheRow(jidStr, name, phone, contact.BusinessName != "")
		}

		if name != "" {
			db.Exec(`INSERT INTO chats (jid, name, timestamp) VALUES (?, ?, 0)
				ON CONFLICT(jid) DO UPDATE SET
				name=COALESCE(NULLIF(excluded.name, ''), chats.name)`,
				jidStr, name)
		}
	}

	groups, err := client.GetJoinedGroups(context.Background())
	if err == nil {
		for _, g := range groups {
			jidStr := g.JID.ToNonAD().String()
			db.Exec(`INSERT INTO chats (jid, name, timestamp) VALUES (?, ?, 0)
				ON CONFLICT(jid) DO UPDATE SET
				name=COALESCE(NULLIF(excluded.name, ''), chats.name)`,
				jidStr, g.GroupName.Name)
		}
	}

	rows, err := db.Query(`SELECT jid FROM chats WHERE name IS NULL OR name = '' OR name LIKE '+%' OR jid LIKE '%@lid'`)
	if err == nil {
		var jidsToResolve []string
		for rows.Next() {
			var j string
			if rows.Scan(&j) == nil {
				jidsToResolve = append(jidsToResolve, j)
			}
		}
		rows.Close()

		for _, jStr := range jidsToResolve {
			pj, err := types.ParseJID(jStr)
			if err != nil {
				continue
			}
			resName := resolveDisplayName(pj)
			if resName != "" && resName != jStr && !strings.HasPrefix(resName, "+") {
				db.Exec(`UPDATE chats SET name = ? WHERE jid = ?`, resName, jStr)
			}
		}
	}

	rowsBroad, errBroad := db.Query(`SELECT DISTINCT sender_jid FROM messages
		WHERE jid LIKE '%@broadcast' AND jid != 'status@broadcast' AND from_me = 0 AND sender_jid != ''`)
	if errBroad == nil {
		var senders []string
		for rowsBroad.Next() {
			var s string
			if rowsBroad.Scan(&s) == nil {
				senders = append(senders, s)
			}
		}
		rowsBroad.Close()
		for _, s := range senders {
			parsed, err := types.ParseJID(s)
			if err != nil {
				continue
			}
			target := parsed.ToNonAD()
			if target.Server == "lid" {
				pn, err := client.Store.LIDs.GetPNForLID(context.Background(), target)
				if err != nil || pn.IsEmpty() {
					continue
				}
				target = pn.ToNonAD()
			}
			if target.Server != types.DefaultUserServer {
				continue
			}
			db.Exec(`INSERT INTO chats (jid, timestamp)
				SELECT ?, MAX(timestamp) FROM messages
				WHERE jid LIKE '%@broadcast' AND from_me = 0 AND sender_jid = ?
				ON CONFLICT(jid) DO UPDATE SET timestamp = MAX(chats.timestamp, excluded.timestamp)`,
				target.String(), s)
			db.Exec(`UPDATE messages SET jid = ?, sender_jid = ?
				WHERE jid LIKE '%@broadcast' AND from_me = 0 AND sender_jid = ?`,
				target.String(), target.String(), s)
			UpdateChatSnippet(target.String())
		}
	}
	db.Exec("DELETE FROM chats WHERE jid = 'status@broadcast' OR jid LIKE '%@newsletter' OR jid LIKE '%@broadcast'")
}
