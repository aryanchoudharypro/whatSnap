package main

import (
	"bytes"
	"context"
	"crypto/sha512"
	"encoding/json"
	"hash"
	"net"

	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waSyncAction"
	"go.mau.fi/whatsmeow/proto/waUserPassword"
	"go.mau.fi/whatsmeow/types"
	"golang.org/x/crypto/pbkdf2"
	"google.golang.org/protobuf/proto"
)

func handleVerifyChatLockCode(conn net.Conn, raw json.RawMessage) {
	var data struct {
		Code string `json:"code"`
	}
	if err := json.Unmarshal(raw, &data); err != nil {
		sendEvent(conn, "chat_lock_verify_result", map[string]interface{}{"success": false, "error": "invalid payload"})
		return
	}

	b, err := getChatLockSecretCode()
	if err != nil || b == nil {
		sendEvent(conn, "chat_lock_verify_result", map[string]interface{}{"success": false, "error": "no secret code configured"})
		return
	}

	var config waUserPassword.UserPassword
	if err := proto.Unmarshal(b, &config); err != nil {
		sendEvent(conn, "chat_lock_verify_result", map[string]interface{}{"success": false, "error": "failed to decode config"})
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
		sendEvent(conn, "chat_lock_verify_result", map[string]interface{}{"success": false, "error": "unsupported hash algorithm"})
		return
	}

	keyLen := len(config.GetTransformedData())
	if keyLen == 0 {
		sendEvent(conn, "chat_lock_verify_result", map[string]interface{}{"success": false, "error": "no hash to compare"})
		return
	}

	derived := pbkdf2.Key([]byte(data.Code), salt, iterations, keyLen, hashFunc)
	success := bytes.Equal(derived, config.GetTransformedData())

	sendEvent(conn, "chat_lock_verify_result", map[string]interface{}{
		"success": success,
	})
}

func buildLockChat(target types.JID, lock bool) appstate.PatchInfo {
	return appstate.PatchInfo{
		Type: appstate.WAPatchRegularLow,
		Mutations: []appstate.MutationInfo{
			{
				Index:   []string{"lock", target.String()},
				Version: 1,
				Value: &waSyncAction.SyncActionValue{
					LockChatAction: &waSyncAction.LockChatAction{
						Locked: proto.Bool(lock),
					},
				},
			},
		},
	}
}

func handleSetChatLocked(conn net.Conn, raw json.RawMessage) {
	var data ChatBoolData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	jid, err := types.ParseJID(data.JID)
	if err != nil {
		return
	}

	b, codeErr := getChatLockSecretCode()
	if codeErr != nil || len(b) == 0 {
		sendEvent(conn, "error", map[string]interface{}{"message": "You must configure a Secret Code for Chat Lock on your primary phone before you can lock or unlock chats from this device."})
		broadcastChats()
		return
	}

	go func() {
		if err := sendAppStateWithRetry(context.Background(), buildLockChat(jid, data.Value)); err != nil {
			broadcastChats()
			return
		}
		locked_int := 0
		if data.Value {
			locked_int = 1
		}
		db.Exec("UPDATE chats SET is_locked = ? WHERE jid = ?", locked_int, data.JID)
		broadcastChats()
	}()
}
