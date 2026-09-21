package main

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"log"
	"net"

	"go.mau.fi/whatsmeow/appstate"
)

// maxCommandBytes bounds a single JSON command line from the frontend.
const maxCommandBytes = 1024 * 1024

func handleConnection(conn net.Conn) {
	fmt.Println("New IPC connection!")
	defer conn.Close()

	wsMutex.Lock()
	wsClients[conn] = true
	wsMutex.Unlock()

	defer func() {
		wsMutex.Lock()
		delete(wsClients, conn)
		wsMutex.Unlock()
	}()

	if client.Store.ID != nil {
		log.Println("Sending initial connected event to WS")
		sendEvent(conn, "connected", map[string]string{"jid": client.Store.ID.String()})
		// "connected" here only means "logged in, show the chat list" - the
		// WhatsApp socket itself may still be down (no network at startup),
		// so follow up with its real state for the status bar.
		sendEvent(conn, "connection_state", currentConnState())
		go func() {
			populateChatsFromStore()
			broadcastChats()
		}()
	} else {
		// Not logged in: tell the client so up front. Without this the UI has
		// no way to know it should show the login screen at startup - main.go's
		// startup goroutine only ever broadcasts "connected" (and only when
		// already paired), and a broadcast would race the frontend's pipe
		// connect anyway. Sending it per-connection here is what makes the
		// login screen appear on a fresh, unpaired launch (and on reconnect).
		log.Println("Not logged in; sending needs_login to WS")
		sendEvent(conn, "needs_login", nil)
	}

	scanner := bufio.NewScanner(conn)
	buf := make([]byte, 0, 64*1024)
	scanner.Buffer(buf, maxCommandBytes)

	for scanner.Scan() {
		msg := scanner.Bytes()

		var cmd Command
		if err := json.Unmarshal(msg, &cmd); err != nil {
			continue
		}

		switch cmd.Action {
		// --- session / pairing ---
		case "get_qr":
			handleGetQR(conn)
		case "cancel_qr", "cancel_pairing":
			handleCancelPairing()
		case "pair":
			handlePair(cmd.Data)
		case "shutdown":
			handleShutdown()
		case "logout":
			handleLogout()
		case "reconnect":
			handleReconnect()

		// --- messaging ---
		case "send":
			go handleSend(conn, cmd.Data)
		case "forward_message":
			handleForwardMessage(conn, cmd.Data)
		case "react_message":
			handleReactMessage(cmd.Data)
		case "edit_message":
			handleEditMessage(cmd.Data)
		case "delete_message":
			handleDeleteMessage(cmd.Data)
		case "star_message":
			handleStarMessage(cmd.Data)
		case "retry_message":
			handleRetryMessage(conn, cmd.Data)
		case "send_typing":
			handleSendTyping(cmd.Data)
		case "subscribe_presence":
			handleSubscribePresence(cmd.Data)
		case "set_presence":
			handleSetPresence(cmd.Data)
		case "mark_read":
			handleMarkRead(cmd.Data)

		// --- contacts ---
		case "new_contact":
			handleNewContact(conn, cmd.Data)
		case "get_contacts":
			handleGetContacts(conn)
		case "get_avatar":
			handleGetAvatar(conn, cmd.Data)
		case "check_contacts":
			handleCheckContacts(conn, cmd.Data)
		case "get_user_info":
			handleGetUserInfo(conn, cmd.Data)
		case "get_business_profile":
			handleGetBusinessProfile(conn, cmd.Data)
		case "get_blocklist":
			handleGetBlocklist(conn)
		case "block_contact":
			handleBlockContact(cmd.Data)
		case "unblock_contact":
			handleUnblockContact(cmd.Data)
		case "set_about":
			handleSetAbout(cmd.Data)
		case "get_own_profile":
			handleGetOwnProfile(conn)
		case "set_push_name":
			handleSetPushName(cmd.Data)
		case "get_privacy_settings":
			handleGetPrivacySettings(conn)
		case "set_privacy_setting":
			handleSetPrivacySetting(cmd.Data)

		// --- groups ---
		case "create_group":
			handleCreateGroup(conn, cmd.Data)
		case "get_group_info":
			handleGetGroupInfo(conn, cmd.Data)
		case "get_community_subgroups":
			handleGetCommunitySubGroups(conn, cmd.Data)
		case "join_community_subgroup":
			handleJoinCommunitySubgroup(cmd.Data)
		case "leave_group":
			handleLeaveGroup(cmd.Data)
		case "get_group_invite_link":
			handleGetGroupInviteLink(conn, cmd.Data)
		case "update_group_participants":
			handleUpdateGroupParticipants(cmd.Data)
		case "get_group_invite_info":
			handleGetGroupInviteInfo(conn, cmd.Data)
		case "join_group_invite":
			handleJoinGroupInvite(cmd.Data)
		case "set_group_name":
			handleSetGroupName(cmd.Data)
		case "set_group_topic":
			handleSetGroupTopic(cmd.Data)
		case "set_group_photo":
			handleSetGroupPhoto(cmd.Data)
		case "set_group_announce":
			handleSetGroupAnnounce(cmd.Data)
		case "set_group_locked":
			handleSetGroupLocked(cmd.Data)

		// --- chats ---
		case "verify_chat_lock_code":
			handleVerifyChatLockCode(conn, cmd.Data)
		case "set_chat_locked":
			handleSetChatLocked(conn, cmd.Data)
		case "get_chats":
			handleGetChats(conn)
		case "get_messages":
			handleGetMessages(conn, cmd.Data)
		case "get_message_info":
			handleGetMessageInfo(conn, cmd.Data)
		case "search_messages":
			handleSearchMessages(conn, cmd.Data)
		case "get_starred_messages":
			handleGetStarredMessages(conn)
		case "export_chat":
			handleExportChat(conn, cmd.Data)
		case "export_chat_text":
			handleExportChatText(conn, cmd.Data)
		case "get_statuses":
			handleGetStatuses(conn)
		case "post_status":
			handlePostStatus(cmd.Data)
		case "post_media_status":
			handlePostMediaStatus(cmd.Data)
		case "mark_status_viewed":
			handleMarkStatusViewed(cmd.Data)
		case "set_chat_pinned":
			handleSetChatPinned(cmd.Data)
		case "set_chat_archived":
			handleSetChatArchived(cmd.Data)
		case "set_chat_muted":
			handleSetChatMuted(cmd.Data)
		case "set_chat_read":
			handleSetChatRead(cmd.Data)
		case "delete_chat":
			handleDeleteChat(cmd.Data)
		case "clear_chat":
			handleClearChat(cmd.Data)
		case "leave_and_delete_group":
			handleLeaveAndDeleteGroup(cmd.Data)
		case "request_history_sync":
			handleRequestHistorySync(cmd.Data)
		case "request_message_resend":
			handleRequestMessageResend(cmd.Data)
		case "place_call":
			handlePlaceCall(cmd.Data)
		case "answer_call":
			handleAnswerCall(cmd.Data)
		case "reject_call":
			handleRejectCall(cmd.Data)
		case "mute_call":
			handleMuteCall(cmd.Data)
		case "leave_call":
			handleLeaveCall(cmd.Data)
		case "add_participant":
			handleAddParticipant(cmd.Data)
		case "set_video":
			handleSetVideo(cmd.Data)

		// --- settings (unchanged) ---
		case "update_settings":
			fmt.Println("Received update_settings command:", string(cmd.Data))
			sendEvent(conn, "settings_updated", map[string]interface{}{"status": "success"})

		// --- media / attachments / voice messages ---
		case "send_attachment":
			handleSendAttachment(conn, cmd.Data)
		case "send_contacts":
			handleSendContacts(conn, cmd.Data)
		case "send_voice_message":
			handleSendVoiceMessage(conn, cmd.Data)
		case "download_media":
			handleDownloadMedia(conn, cmd.Data)
		case "cancel_download":
			handleCancelDownload(conn, cmd.Data)
		case "open_media":
			handleOpenMedia(conn, cmd.Data)
		case "save_media_as":
			handleSaveMediaAs(conn, cmd.Data)
		case "copy_album_to_clipboard":
			handleCopyAlbumToClipboard(conn, cmd.Data)
		case "play_voice_message":
			handlePlayVoiceMessage(conn, cmd.Data)
		// Recording itself happens client-side (mic capture); these are kept
		// as accepted no-ops so older frontend builds that still call them
		// don't error out.
		case "start_recording", "toggle_pause_recording", "discard_voice_message":
			fmt.Println(cmd.Action, "triggered (handled client-side; no backend action needed)")

		case "force_sync":
			go func() {
				appStateResyncMu.Lock()
				for k := range appStateResyncCount {
					delete(appStateResyncCount, k)
				}
				for k := range lastAppStateResync {
					delete(lastAppStateResync, k)
				}
				appStateResyncMu.Unlock()
				for _, name := range []appstate.WAPatchName{
					appstate.WAPatchRegularLow,
					appstate.WAPatchRegularHigh,
					appstate.WAPatchCriticalBlock,
					appstate.WAPatchCriticalUnblockLow,
				} {
					if err := client.FetchAppState(context.Background(), name, false, false); err != nil {
						fmt.Println("Error syncing AppState (", name, "):", err)
					}
				}
				populateChatsFromStore()
				broadcastChats()
				handleGetContacts(conn)
			}()
		}
	}

	if err := scanner.Err(); err != nil {
		if err == bufio.ErrTooLong {
			log.Printf("IPC connection dropped: command exceeded %d bytes", maxCommandBytes)
		} else {
			log.Println("IPC connection dropped:", err)
		}
	} else {
		log.Println("IPC connection closed by client")
	}
}
