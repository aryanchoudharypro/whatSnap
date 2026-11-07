package main

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"log"
	"net"
	"runtime/debug"
	"time"

	"go.mau.fi/whatsmeow/appstate"
)

const maxCommandBytes = 1024 * 1024

// handleConnection reads the app's commands. Each one names the account it
// is for; "open_account", "close_account" and "shutdown" are for the backend
// as a whole.
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

	scanner := bufio.NewScanner(conn)
	buf := make([]byte, 0, 64*1024)
	scanner.Buffer(buf, maxCommandBytes)

	for scanner.Scan() {
		var cmd Command
		if err := json.Unmarshal(scanner.Bytes(), &cmd); err != nil {
			continue
		}
		switch cmd.Action {
		case "open_account":
			handleOpenAccount(conn, cmd.Account, cmd.Data)
			continue
		case "close_account":
			handleCloseAccount(conn, cmd.Account)
			continue
		case "shutdown":
			shutdownBackend()
			continue
		}
		acc := lookupAccount(cmd.Account)
		if acc == nil {
			log.Printf("Dropping %q for account %q, which isn't open", cmd.Action, cmd.Account)
			continue
		}
		select {
		case acc.commands <- queuedCommand{conn: conn, cmd: cmd}:
		case <-acc.ctx.Done():
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

// sendGreeting tells a newly connected app where this backend stands: linked
// (with the state the app needs first) or waiting to be linked. The app asks
// for it again with "hello" when it switches to this backend's account.
func (acc *Account) sendGreeting(conn net.Conn) {
	if acc.wa().Store.ID != nil {
		log.Println("Sending initial connected event to WS")
		acc.sendEvent(conn, "connected", map[string]string{"jid": acc.wa().Store.ID.String()})
		acc.sendEvent(conn, "connection_state", acc.currentConnState())
		acc.sendEvent(conn, "server_flags", acc.serverFlagsPayload())
		acc.sendEvent(conn, "blocklist", acc.blocklistPayload())
		go func() {
			acc.populateChatsFromStore()
			acc.broadcastChats()
		}()
	} else {
		// Not logged in: tell the client so up front. Without this the UI has
		// no way to know it should show the login screen at startup - main.go's
		// startup goroutine only ever broadcasts "connected" (and only when
		// already paired), and a broadcast would race the frontend's pipe
		// connect anyway. Sending it per-connection here is what makes the
		// login screen appear on a fresh, unpaired launch (and on reconnect).
		log.Println("Not logged in; sending needs_login to WS")
		acc.sendEvent(conn, "needs_login", nil)
	}
}

const commandStallThreshold = 10 * time.Second

func (acc *Account) dispatchCommand(conn net.Conn, cmd Command) {
	stalled := time.AfterFunc(commandStallThreshold, func() {
		log.Printf("Command %q still running after %s; later commands are waiting on it", cmd.Action, commandStallThreshold)
		dumpGoroutinesForStall(fmt.Sprintf("command %q still running after %s", cmd.Action, commandStallThreshold))
	})
	start := time.Now()
	defer func() {
		if stalled.Stop() {
			return
		}
		log.Printf("Command %q finished after %s", cmd.Action, time.Since(start).Round(time.Millisecond))
	}()
	defer func() {
		if r := recover(); r != nil {
			log.Printf("PANIC handling command %q: %v\n%s", cmd.Action, r, debug.Stack())
		}
	}()
	switch cmd.Action {
	case "send", "send_attachment", "send_sticker", "send_contacts", "send_poll", "send_event", "send_voice_message":
		acc.markReadOnSend(cmd.Data)
	}
	switch cmd.Action {
	case "hello":
		acc.sendGreeting(conn)
	case "get_qr":
		acc.handleGetQR(conn)
	case "cancel_qr", "cancel_pairing":
		acc.handleCancelPairing()
	case "pair":
		acc.handlePair(cmd.Data)
	case "logout":
		acc.handleLogout()
	case "reconnect":
		acc.handleReconnect()

	case "send":
		go acc.handleSend(conn, cmd.Data)
	case "forward_message":
		acc.handleForwardMessage(conn, cmd.Data)
	case "react_message":
		acc.handleReactMessage(cmd.Data)
	case "get_mention_candidates":
		acc.handleGetMentionCandidates(conn, cmd.Data)
	case "edit_message":
		acc.handleEditMessage(cmd.Data)
	case "delete_message":
		acc.handleDeleteMessage(cmd.Data)
	case "delete_messages":
		acc.handleDeleteMessages(cmd.Data)
	case "star_message":
		acc.handleStarMessage(cmd.Data)
	case "pin_message":
		acc.handlePinMessage(cmd.Data)
	case "get_pinned_messages":
		go acc.handleGetPinnedMessages(conn, cmd.Data)
	case "keep_message":
		acc.handleKeepMessage(cmd.Data)
	case "get_stickers":
		go acc.handleGetStickers(conn)
	case "get_sticker_store":
		go acc.handleGetStickerStore(conn)
	case "get_sticker_pack":
		go acc.handleGetStickerPack(conn, cmd.Data)
	case "send_sticker":
		acc.handleSendSticker(conn, cmd.Data)
	case "set_sticker_favorite":
		acc.handleSetStickerFavorite(conn, cmd.Data)
	case "get_chat_privacy":
		acc.handleGetChatPrivacy(conn, cmd.Data)
	case "set_chat_privacy":
		acc.handleSetChatPrivacy(cmd.Data)
	case "retry_message":
		acc.handleRetryMessage(conn, cmd.Data)
	case "send_typing":
		acc.handleSendTyping(cmd.Data)
	case "subscribe_presence":
		go acc.handleSubscribePresence(cmd.Data)
	case "set_presence":
		acc.handleSetPresence(cmd.Data)
	case "mark_read":
		acc.handleMarkRead(cmd.Data)

	case "new_contact":
		acc.handleNewContact(conn, cmd.Data)
	case "get_contacts":
		acc.handleGetContacts(conn)
	case "get_avatar":
		acc.handleGetAvatar(conn, cmd.Data)
	case "get_avatar_previews":
		acc.handleGetAvatarPreviews(conn, cmd.Data)
	case "get_thumbnails":
		acc.handleGetThumbnails(conn, cmd.Data)
	case "get_waveforms":
		acc.handleGetWaveforms(conn, cmd.Data)
	case "check_contacts":
		acc.handleCheckContacts(conn, cmd.Data)
	case "delete_contact":
		acc.handleDeleteContact(conn, cmd.Data)
	case "get_user_info":
		acc.handleGetUserInfo(conn, cmd.Data)
	case "get_business_profile":
		acc.handleGetBusinessProfile(conn, cmd.Data)
	case "get_blocklist":
		acc.handleGetBlocklist(conn)
	case "report":
		acc.handleReport(conn, cmd.Data)
	case "block_contact":
		acc.handleSetBlocked(cmd.Data, true)
	case "unblock_contact":
		acc.handleSetBlocked(cmd.Data, false)
	case "set_about":
		acc.handleSetAbout(cmd.Data)
	case "set_own_photo":
		acc.handleSetOwnPhoto(conn, cmd.Data)
	case "get_default_timer":
		go acc.handleGetDefaultTimer(conn)
	case "set_default_timer":
		acc.handleSetDefaultTimer(conn, cmd.Data)
	case "get_own_profile":
		acc.handleGetOwnProfile(conn)
	case "set_push_name":
		acc.handleSetPushName(cmd.Data)
	case "get_app_prefs":
		acc.handleGetAppPrefs(conn)
	case "set_app_prefs":
		acc.handleSetAppPrefs(cmd.Data)
	case "sync_settings":
		acc.handleSyncSettings(cmd.Data)
	case "sync_chat_tone":
		acc.handleSyncChatTone(cmd.Data)
	case "get_privacy_settings":
		acc.handleGetPrivacySettings(conn)
	case "set_privacy_setting":
		acc.handleSetPrivacySetting(cmd.Data)
	case "get_privacy_list":
		acc.handleGetPrivacyList(conn, cmd.Data)
	case "set_privacy_list":
		acc.handleSetPrivacyList(cmd.Data)

	case "create_group":
		acc.handleCreateGroup(conn, cmd.Data)
	case "get_group_info":
		acc.handleGetGroupInfo(conn, cmd.Data)
	case "send_group_history":
		acc.handleSendGroupHistory(conn, cmd.Data)
	case "get_security_code":
		acc.handleGetSecurityCode(conn, cmd.Data)
	case "get_community_subgroups":
		acc.handleGetCommunitySubGroups(conn, cmd.Data)
	case "get_join_requests":
		go acc.handleGetJoinRequests(conn, cmd.Data)
	case "answer_join_requests":
		acc.handleAnswerJoinRequests(conn, cmd.Data)
	case "create_community":
		acc.handleCreateCommunity(conn, cmd.Data)
	case "get_linkable_groups":
		go acc.handleGetLinkableGroups(conn, cmd.Data)
	case "link_community_group":
		acc.handleLinkCommunityGroup(conn, cmd.Data, true)
	case "unlink_community_group":
		acc.handleLinkCommunityGroup(conn, cmd.Data, false)
	case "deactivate_community":
		acc.handleDeactivateCommunity(conn, cmd.Data)
	case "join_community_subgroup":
		acc.handleJoinCommunitySubgroup(cmd.Data)
	case "leave_group":
		acc.handleLeaveGroup(cmd.Data)
	case "get_group_invite_link":
		acc.handleGetGroupInviteLink(conn, cmd.Data)
	case "update_group_participants":
		acc.handleUpdateGroupParticipants(cmd.Data)
	case "get_group_invite_info":
		acc.handleGetGroupInviteInfo(conn, cmd.Data)
	case "join_group_invite":
		acc.handleJoinGroupInvite(cmd.Data)
	case "channel_directory":
		acc.handleChannelDirectory(conn, cmd.Data)
	case "get_channel_info":
		acc.handleGetChannelInfo(conn, cmd.Data)
	case "follow_channel":
		acc.handleFollowChannel(cmd.Data, true)
	case "unfollow_channel":
		acc.handleFollowChannel(cmd.Data, false)
	case "create_channel":
		acc.handleCreateChannel(conn, cmd.Data)
	case "update_channel":
		acc.handleUpdateChannel(conn, cmd.Data)
	case "delete_channel":
		acc.handleDeleteChannel(conn, cmd.Data)
	case "get_channel_followers":
		acc.handleGetChannelFollowers(conn, cmd.Data)
	case "change_channel_owner":
		acc.handleChannelAdminChange(conn, cmd.Data, true)
	case "demote_channel_admin":
		acc.handleChannelAdminChange(conn, cmd.Data, false)
	case "invite_channel_admins":
		acc.handleInviteChannelAdmins(conn, cmd.Data)
	case "get_channel_admin_invite":
		acc.handleGetChannelAdminInvite(conn, cmd.Data)
	case "accept_channel_admin_invite":
		acc.handleChannelAdminInviteAnswer(conn, cmd.Data, true)
	case "revoke_channel_admin_invite":
		acc.handleChannelAdminInviteAnswer(conn, cmd.Data, false)
	case "pin_channel_post":
		acc.handlePinChannelPost(conn, cmd.Data)
	case "get_channel_reactions":
		acc.handleGetChannelReactions(conn, cmd.Data)
	case "get_channel_poll_voters":
		acc.handleGetChannelPollVoters(conn, cmd.Data)
	case "get_channel_pending_invites":
		acc.handleGetChannelPendingInvites(conn, cmd.Data)
	case "get_similar_channels":
		acc.handleGetSimilarChannels(conn, cmd.Data)
	case "get_group_link_info":
		acc.handleGetGroupLinkInfo(conn, cmd.Data)
	case "join_group_link":
		acc.handleJoinGroupLink(cmd.Data)
	case "set_group_name":
		acc.handleSetGroupName(cmd.Data)
	case "set_group_topic":
		acc.handleSetGroupTopic(cmd.Data)
	case "set_group_photo":
		acc.handleSetGroupPhoto(cmd.Data)
	case "set_group_announce":
		acc.handleSetGroupAnnounce(cmd.Data)
	case "set_group_locked":
		acc.handleSetGroupLocked(cmd.Data)
	case "set_member_label":
		acc.handleSetMemberLabel(conn, cmd.Data)
	case "set_group_moderation":
		acc.handleSetGroupModeration(cmd.Data)
	case "report_to_admins":
		acc.handleReportToAdmins(conn, cmd.Data)
	case "get_member_changes":
		acc.handleGetMemberChanges(conn, cmd.Data)
	case "get_reported_messages":
		acc.handleGetReportedMessages(conn, cmd.Data)

	case "verify_chat_lock_code":
		// PBKDF2 with the server's iteration count can take a moment -
		// never on the goroutine that reads every other command.
		go acc.handleVerifyChatLockCode(conn, cmd.Data)
	case "set_chat_locked":
		acc.handleSetChatLocked(conn, cmd.Data)
	case "get_chats":
		acc.handleGetChats(conn)
	case "get_messages":
		acc.handleGetMessages(conn, cmd.Data)
	case "get_chat_media":
		acc.handleGetChatMedia(conn, cmd.Data)
	case "get_messages_around":
		acc.handleGetMessagesAround(conn, cmd.Data)
	case "get_message_info":
		acc.handleGetMessageInfo(conn, cmd.Data)
	case "search_messages":
		acc.handleSearchMessages(conn, cmd.Data)
	case "get_starred_messages":
		acc.handleGetStarredMessages(conn)
	case "export_chat":
		acc.handleExportChat(conn, cmd.Data)
	case "export_chat_text":
		acc.handleExportChatText(conn, cmd.Data)
	case "get_statuses":
		acc.handleGetStatuses(conn)
	case "post_status":
		acc.handlePostStatus(cmd.Data)
	case "post_media_status":
		acc.handlePostMediaStatus(cmd.Data)
	case "get_status_viewers":
		go acc.handleGetStatusViewers(conn, cmd.Data)
	case "react_status":
		acc.handleReactStatus(cmd.Data)
	case "delete_status":
		acc.handleDeleteStatus(cmd.Data)
	case "set_status_mute":
		acc.handleSetStatusMute(cmd.Data)
	case "get_chat_lists":
		go acc.handleGetChatLists(conn)
	case "set_favorite":
		acc.handleSetFavorite(conn, cmd.Data)
	case "create_list":
		acc.handleCreateList(conn, cmd.Data)
	case "rename_list":
		acc.handleRenameList(conn, cmd.Data)
	case "delete_list":
		acc.handleDeleteList(conn, cmd.Data)
	case "set_list_member":
		acc.handleSetListMember(conn, cmd.Data)
	case "get_common_groups":
		go acc.handleGetCommonGroups(conn, cmd.Data)
	case "get_status_privacy":
		go acc.handleGetStatusPrivacy(conn)
	case "set_status_privacy":
		acc.handleSetStatusPrivacy(conn, cmd.Data)
	case "mark_status_viewed":
		acc.handleMarkStatusViewed(cmd.Data)
	case "set_chat_pinned":
		acc.handleSetChatPinned(cmd.Data)
	case "set_chat_archived":
		acc.handleSetChatArchived(cmd.Data)
	case "set_disappearing_timer":
		acc.handleSetDisappearingTimer(conn, cmd.Data)
	case "get_disappearing_timer":
		acc.handleGetDisappearingTimer(conn, cmd.Data)
	case "set_chat_muted":
		acc.handleSetChatMuted(cmd.Data)
	case "set_chat_read":
		acc.handleSetChatRead(cmd.Data)
	case "delete_chat":
		acc.handleDeleteChat(cmd.Data)
	case "clear_chat":
		acc.handleClearChat(cmd.Data)
	case "leave_and_delete_group":
		acc.handleLeaveAndDeleteGroup(cmd.Data)
	case "request_history_sync":
		acc.handleRequestHistorySync(cmd.Data)
	case "request_message_resend":
		acc.handleRequestMessageResend(cmd.Data)
	case "create_call_link":
		acc.handleCreateCallLink(conn, cmd.Data)
	case "join_call_link":
		acc.handleJoinCallLink(cmd.Data)
	case "join_group_call":
		acc.handleJoinGroupCall(cmd.Data)
	case "get_joinable_calls":
		acc.handleGetJoinableCalls()
	case "raise_hand":
		acc.handleCallToggle(conn, cmd.Data, "hand")
	case "call_reaction":
		acc.handleCallToggle(conn, cmd.Data, "reaction")
	case "admit_call_participant":
		acc.handleCallToggle(conn, cmd.Data, "admit")
	case "mute_call_participant":
		acc.handleCallToggle(conn, cmd.Data, "mute_person")
	case "remove_call_participant":
		acc.handleCallToggle(conn, cmd.Data, "remove_person")
	case "set_call_approval":
		acc.handleCallToggle(conn, cmd.Data, "approval")
	case "place_call":
		acc.handlePlaceCall(cmd.Data)
	case "get_call_logs":
		acc.handleGetCallLogs(conn)
	case "delete_call_logs":
		acc.handleDeleteCallLogs(cmd.Data)
	case "get_call_targets":
		acc.handleGetCallTargets(conn)
	case "answer_call":
		acc.handleAnswerCall(cmd.Data)
	case "reject_call":
		acc.handleRejectCall(cmd.Data)
	case "mute_call":
		acc.handleMuteCall(cmd.Data)
	case "leave_call":
		acc.handleLeaveCall(cmd.Data)
	case "add_participant":
		acc.handleAddParticipant(cmd.Data)
	case "set_video":
		acc.handleSetVideo(cmd.Data)
	case "set_screen_share":
		acc.handleSetScreenShare(cmd.Data)
	case "set_call_options":
		acc.handleSetCallOptions(cmd.Data)

	case "update_settings":
		fmt.Println("Received update_settings command:", string(cmd.Data))
		acc.sendEvent(conn, "settings_updated", map[string]interface{}{"status": "success"})

	case "send_attachment":
		acc.handleSendAttachment(conn, cmd.Data)
	case "send_contacts":
		acc.handleSendContacts(conn, cmd.Data)
	case "send_poll":
		acc.handleSendPoll(conn, cmd.Data)
	case "get_poll":
		acc.handleGetPoll(conn, cmd.Data)
	case "send_poll_vote":
		acc.handleSendPollVote(conn, cmd.Data)
	case "send_event":
		acc.handleSendEvent(conn, cmd.Data)
	case "get_event":
		acc.handleGetEvent(conn, cmd.Data)
	case "get_message_options":
		acc.handleGetMessageOptions(conn, cmd.Data)
	case "send_message_option":
		acc.handleSendMessageOption(conn, cmd.Data)
	case "get_catalog":
		acc.handleGetCatalog(conn, cmd.Data)
	case "get_order":
		acc.handleGetOrder(conn, cmd.Data)
	case "send_event_response":
		acc.handleSendEventResponse(conn, cmd.Data)
	case "send_voice_message":
		acc.handleSendVoiceMessage(conn, cmd.Data)
	case "download_media":
		acc.handleDownloadMedia(conn, cmd.Data)
	case "cancel_download":
		acc.handleCancelDownload(conn, cmd.Data)
	case "cancel_upload":
		acc.handleCancelUpload(cmd.Data)
	case "open_media":
		acc.handleOpenMedia(conn, cmd.Data)
	case "prepare_media":
		acc.handlePrepareMedia(conn, cmd.Data)
	case "save_media_as":
		acc.handleSaveMediaAs(conn, cmd.Data)
	case "copy_album_to_clipboard":
		acc.handleCopyAlbumToClipboard(conn, cmd.Data)
	case "play_voice_message":
		acc.handlePlayVoiceMessage(conn, cmd.Data)
	// Recording itself happens client-side (mic capture); these are kept
	// as accepted no-ops so older frontend builds that still call them
	// don't error out.
	case "start_recording", "toggle_pause_recording", "discard_voice_message":
		fmt.Println(cmd.Action, "triggered (handled client-side; no backend action needed)")

	case "force_sync":
		go func() {
			acc.appStateResyncMu.Lock()
			for k := range acc.appStateResyncCount {
				delete(acc.appStateResyncCount, k)
			}
			for k := range acc.lastAppStateResync {
				delete(acc.lastAppStateResync, k)
			}
			acc.appStateResyncMu.Unlock()
			for _, name := range []appstate.WAPatchName{
				appstate.WAPatchRegular,
				appstate.WAPatchRegularLow,
				appstate.WAPatchRegularHigh,
				appstate.WAPatchCriticalBlock,
				appstate.WAPatchCriticalUnblockLow,
			} {
				if err := acc.wa().FetchAppState(context.Background(), name, false, false); err != nil {
					fmt.Println("Error syncing AppState (", name, "):", err)
				}
			}
			acc.populateChatsFromStore()
			acc.broadcastChats()
			acc.handleGetContacts(conn)
		}()
	}
}
