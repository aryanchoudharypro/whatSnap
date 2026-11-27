package main

import (
	"bytes"
	"context"
	"crypto/sha512"
	"encoding/json"
	"hash"
	"log"
	"net"
	"time"

	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waChatLockSettings"
	"go.mau.fi/whatsmeow/proto/waSyncAction"
	"go.mau.fi/whatsmeow/proto/waUserPassword"
	"go.mau.fi/whatsmeow/types"
	"golang.org/x/crypto/pbkdf2"
	"google.golang.org/protobuf/proto"
)

func (acc *Account) handleVerifyChatLockCode(conn net.Conn, raw json.RawMessage) {
	// Seq is set by the chat list's search box, which checks what is typed
	// there quietly; the answer carries it back so the app can tell it apart
	// from the unlock prompt's.
	var data struct {
		Code string `json:"code"`
		Seq  uint64 `json:"seq"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		acc.sendEvent(conn, "chat_lock_verify_result", map[string]interface{}{"success": false, "error": "invalid payload"})
		return
	}
	reply := func(result map[string]interface{}) {
		if data.Seq > 0 {
			result["seq"] = data.Seq
		}
		acc.sendEvent(conn, "chat_lock_verify_result", result)
	}

	b, err := acc.getChatLockSecretCode()
	if err != nil || b == nil {
		reply(map[string]interface{}{"success": false, "error": "no secret code configured"})
		return
	}

	var config waUserPassword.UserPassword
	if err := proto.Unmarshal(b, &config); err != nil {
		reply(map[string]interface{}{"success": false, "error": "failed to decode config"})
		return
	}

	var salt []byte
	var iterations int
	for _, arg := range config.GetTransformerArg() {
		if v := arg.GetValue().GetAsBlob(); len(v) > 0 {
			salt = v
		} else if v := arg.GetValue().GetAsUnsignedInteger(); v > 0 {
			iterations = int(v)
		}
	}

	var hashFunc func() hash.Hash
	if config.GetTransformer() == waUserPassword.UserPassword_PBKDF2_HMAC_SHA512 {
		hashFunc = sha512.New
	} else if config.GetTransformer() == waUserPassword.UserPassword_PBKDF2_HMAC_SHA384 {
		hashFunc = sha512.New384
	} else {
		reply(map[string]interface{}{"success": false, "error": "unsupported hash algorithm"})
		return
	}

	keyLen := len(config.GetTransformedData())
	if keyLen == 0 {
		reply(map[string]interface{}{"success": false, "error": "no hash to compare"})
		return
	}

	derived := pbkdf2.Key([]byte(data.Code), salt, iterations, keyLen, hashFunc)
	success := bytes.Equal(derived, config.GetTransformedData())

	reply(map[string]interface{}{"success": success})
}

// applySyncedChatLockSettings stores a chat_lock_settings mutation the way
// WA Web's WAWebChatLockSettingsSync reads it: hideLockedChats must be set,
// and a mutation without a secret code clears the stored one. A secret code
// that isn't PBKDF2-SHA512 with a salt and iteration count is malformed, so the
// whole mutation is dropped.
func (acc *Account) applySyncedChatLockSettings(cls *waChatLockSettings.ChatLockSettings) {
	if cls.HideLockedChats == nil {
		log.Println("Chat lock settings mutation without hideLockedChats, ignored")
		return
	}
	sc := cls.GetSecretCode()
	if sc != nil && !validChatLockSecretCode(sc) {
		log.Println("Chat lock settings mutation with a malformed secret code, ignored")
		return
	}
	acc.saveChatLockHide(cls.GetHideLockedChats())
	if sc == nil {
		acc.clearChatLockSecretCode()
	} else if b, err := proto.Marshal(sc); err == nil {
		acc.saveChatLockSecretCode(b)
	}
	acc.broadcastEvent("chat_lock_settings", acc.chatLockSettingsPayload())
}

// applyHistoryChatLockSettings stores what the history sync's global settings
// carry, without clearing anything that is missing there.
func (acc *Account) applyHistoryChatLockSettings(cls *waChatLockSettings.ChatLockSettings) {
	if cls.HideLockedChats != nil {
		acc.saveChatLockHide(cls.GetHideLockedChats())
	}
	if sc := cls.GetSecretCode(); sc != nil && validChatLockSecretCode(sc) {
		if b, err := proto.Marshal(sc); err == nil {
			acc.saveChatLockSecretCode(b)
		}
	}
	acc.broadcastEvent("chat_lock_settings", acc.chatLockSettingsPayload())
}

func validChatLockSecretCode(sc *waUserPassword.UserPassword) bool {
	if sc.Encoding == nil || sc.Transformer == nil || len(sc.GetTransformedData()) == 0 || len(sc.GetTransformerArg()) == 0 {
		return false
	}
	if sc.GetTransformer() != waUserPassword.UserPassword_PBKDF2_HMAC_SHA512 {
		return false
	}
	var salt, iterations bool
	for _, arg := range sc.GetTransformerArg() {
		switch arg.GetKey() {
		case "salt":
			salt = len(arg.GetValue().GetAsBlob()) > 0
		case "iterations":
			iterations = arg.GetValue().GetAsUnsignedInteger() > 0
		}
	}
	return salt && iterations
}

// chatLockSettingsPayload is what the app needs to know about chat lock: if
// the locked chats entry is hidden (then the secret code typed in the search
// box shows it) and if a secret code exists at all.
func (acc *Account) chatLockSettingsPayload() map[string]interface{} {
	b, _ := acc.getChatLockSecretCode()
	return map[string]interface{}{
		"hide_locked_chats": acc.getChatLockHide(),
		"has_secret_code":   len(b) > 0,
	}
}

// buildLockChat builds a lock or unlock like WA Web's
// WAWebLockChatSync.sendLockMutation: version 7, and locking first takes the
// chat out of the archive and unpins it, in the same patch. chat is the
// chat's own JID, for the archive's message range.
func (acc *Account) buildLockChat(target types.JID, chat string, lock bool) appstate.PatchInfo {
	patch := appstate.PatchInfo{Type: appstate.WAPatchRegularLow}
	if lock {
		lastTS, lastKey := acc.getLastMessageKeyForChat(chat)
		patch.Mutations = append(patch.Mutations, acc.withChatRange(appstate.BuildArchive(target, false, lastTS, lastKey), chat).Mutations...)
		patch.Mutations = append(patch.Mutations, appstate.BuildPin(target, false).Mutations...)
	}
	patch.Mutations = append(patch.Mutations, lockChatMutation(target, lock))
	return patch
}

func lockChatMutation(target types.JID, lock bool) appstate.MutationInfo {
	return appstate.MutationInfo{
		Index:   []string{"lock", target.String()},
		Version: 7,
		Value: &waSyncAction.SyncActionValue{
			LockChatAction: &waSyncAction.LockChatAction{
				Locked: proto.Bool(lock),
			},
		},
	}
}

func (acc *Account) handleSetChatLocked(conn net.Conn, raw json.RawMessage) {
	var data ChatBoolData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}

	b, codeErr := acc.getChatLockSecretCode()
	if codeErr != nil || len(b) == 0 {
		acc.sendEvent(conn, "error", map[string]interface{}{"message": "You must configure a Secret Code for Chat Lock on your primary phone before you can lock or unlock chats from this device."})
		acc.broadcastChats()
		return
	}

	go func() {
		if err := acc.sendAppStateWithRetry(context.Background(), acc.buildLockChat(acc.syncdChatJID(jid), data.JID, data.Value)); err != nil {
			acc.broadcastChats()
			return
		}
		locked_int := 0
		if data.Value {
			locked_int = 1
		}
		jids := acc.chatJIDList(data.JID)
		for _, j := range jids {
			res, _ := acc.db.Exec("UPDATE chats SET is_locked = ? WHERE jid = ?", locked_int, j)
			if rows, _ := res.RowsAffected(); rows == 0 && locked_int == 1 {
				acc.db.Exec("INSERT OR IGNORE INTO chats (jid, is_locked, timestamp) VALUES (?, 1, 0)", j)
			}
		}
		if data.Value {
			now := time.Now().Unix()
			acc.noteArchiveAction(data.JID, false, nil, now)
			acc.db.Exec("UPDATE chats SET archived = 0 WHERE jid = ?", data.JID)
			acc.applyPinChange(data.JID, false, now)
		}
		acc.broadcastChats()
	}()
}
