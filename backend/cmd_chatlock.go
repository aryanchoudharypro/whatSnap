package main

import (
	"bytes"
	"context"
	"crypto/sha512"
	"encoding/json"
	"hash"
	"net"
	"time"

	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waSyncAction"
	"go.mau.fi/whatsmeow/proto/waUserPassword"
	"go.mau.fi/whatsmeow/types"
	"golang.org/x/crypto/pbkdf2"
	"google.golang.org/protobuf/proto"
)

func (acc *Account) handleVerifyChatLockCode(conn net.Conn, raw json.RawMessage) {
	var data struct {
		Code string `json:"code"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		acc.sendEvent(conn, "chat_lock_verify_result", map[string]interface{}{"success": false, "error": "invalid payload"})
		return
	}

	b, err := acc.getChatLockSecretCode()
	if err != nil || b == nil {
		acc.sendEvent(conn, "chat_lock_verify_result", map[string]interface{}{"success": false, "error": "no secret code configured"})
		return
	}

	var config waUserPassword.UserPassword
	if err := proto.Unmarshal(b, &config); err != nil {
		acc.sendEvent(conn, "chat_lock_verify_result", map[string]interface{}{"success": false, "error": "failed to decode config"})
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
		acc.sendEvent(conn, "chat_lock_verify_result", map[string]interface{}{"success": false, "error": "unsupported hash algorithm"})
		return
	}

	keyLen := len(config.GetTransformedData())
	if keyLen == 0 {
		acc.sendEvent(conn, "chat_lock_verify_result", map[string]interface{}{"success": false, "error": "no hash to compare"})
		return
	}

	derived := pbkdf2.Key([]byte(data.Code), salt, iterations, keyLen, hashFunc)
	success := bytes.Equal(derived, config.GetTransformedData())

	acc.sendEvent(conn, "chat_lock_verify_result", map[string]interface{}{
		"success": success,
	})
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
		acc.db.Exec("UPDATE chats SET is_locked = ? WHERE jid = ?", locked_int, data.JID)
		if data.Value {
			now := time.Now().Unix()
			acc.noteArchiveAction(data.JID, false, nil, now)
			acc.db.Exec("UPDATE chats SET archived = 0 WHERE jid = ?", data.JID)
			acc.applyPinChange(data.JID, false, now)
		}
		acc.broadcastChats()
	}()
}
