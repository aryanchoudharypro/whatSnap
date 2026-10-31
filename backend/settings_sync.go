package main

import (
	"context"
	"encoding/json"
	"log"
	"strconv"
	"strings"
	"time"

	"google.golang.org/protobuf/reflect/protoreflect"

	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waSyncAction"
	"go.mau.fi/whatsmeow/types/events"
)

// Settings sync: WhatsApp Web and its desktop apps share some settings
// between themselves through app state (WAWebSettingsSync, regular_low).
// Each setting is its own mutation, indexed by platform, setting and target
// ("app" or a chat), holding just that one field of SettingsSyncAction.
// WhatsApp for Windows writes HYBRID and takes WEB and HYBRID ones, so
// whatSnap, a Windows app, does the same. Per-chat ones (wallpapers, chat
// themes, tones) are left alone: whatSnap has none of those.
//
// WhatsApp only sends them once the phone lists settings_sync_enabled
// among its features and the server's flag of the same name is on. whatSnap
// doesn't wait for either: the phone's list is synced once at linking, so a
// whatSnap updated later never hears it, and devices with the feature off
// just ignore these mutations.

const (
	settingsSyncTimeKey = "settings_sync_ts:" // + the setting's key number
)

// The settings the backend keeps itself (app_prefs.go), by WhatsApp's name.
var syncedAppPrefs = map[string]string{
	"isPhotosAutodownloadEnabled":    "auto_download_photos",
	"isAudiosAutodownloadEnabled":    "auto_download_audio",
	"isVideosAutodownloadEnabled":    "auto_download_videos",
	"isDocumentsAutodownloadEnabled": "auto_download_documents",
	"mediaUploadQuality":             "hd_media",
}

// settingName is WhatsApp Web's name for a SettingsSyncAction field
// (WAWebSettingsSyncConst): the proto's, with "ID" written "Id".
func settingName(fd protoreflect.FieldDescriptor) string {
	name := fd.JSONName()
	if strings.HasSuffix(name, "ID") {
		name = name[:len(name)-2] + "Id"
	}
	return name
}

// settingField finds a setting's field. Each key's number is its field's.
func settingField(name string) protoreflect.FieldDescriptor {
	fields := (&waSyncAction.SettingsSyncAction{}).ProtoReflect().Descriptor().Fields()
	for i := range fields.Len() {
		if settingName(fields.Get(i)) == name {
			return fields.Get(i)
		}
	}
	return nil
}

func settingSyncedAt(key waSyncAction.SettingsSyncAction_SettingKey) int64 {
	var ms int64
	db.QueryRow(`SELECT CAST(value AS INTEGER) FROM app_settings WHERE key = ?`, settingsSyncTimeKey+strconv.Itoa(int(key))).Scan(&ms)
	return ms
}

func noteSettingSyncedAt(key waSyncAction.SettingsSyncAction_SettingKey, at time.Time) {
	db.Exec(`INSERT OR REPLACE INTO app_settings (key, value) VALUES (?, ?)`, settingsSyncTimeKey+strconv.Itoa(int(key)), at.UnixMilli())
}

// noteSettingsSync applies a setting another WhatsApp desktop device
// changed, unless a newer one for it was already applied or sent from here
// (WA Web keeps only the newest of each in a batch the same way).
func noteSettingsSync(v *events.SettingsSync) {
	if v.Target != appstate.SettingsSyncAppTarget || (v.Platform != waSyncAction.SettingsSyncAction_WEB && v.Platform != waSyncAction.SettingsSyncAction_HYBRID) {
		return
	}
	msg := v.Action.ProtoReflect()
	fd := msg.Descriptor().Fields().ByNumber(protoreflect.FieldNumber(v.Key))
	if fd == nil || !msg.Has(fd) {
		log.Printf("Skipping settings sync for key %d: no value", v.Key)
		return
	}
	if v.Timestamp.UnixMilli() <= settingSyncedAt(v.Key) {
		return
	}
	noteSettingSyncedAt(v.Key, v.Timestamp)
	name := settingName(fd)
	var value interface{}
	switch val := msg.Get(fd); fd.Kind() {
	case protoreflect.BoolKind:
		value = val.Bool()
	case protoreflect.EnumKind:
		value = int(val.Enum())
	case protoreflect.Int32Kind:
		value = val.Int()
	case protoreflect.StringKind:
		value = val.String()
	default:
		return
	}
	if pref, ok := syncedAppPrefs[name]; ok {
		on, isBool := value.(bool)
		if name == "mediaUploadQuality" {
			switch waSyncAction.SettingsSyncAction_MediaQualitySetting(value.(int)) {
			case waSyncAction.SettingsSyncAction_HD:
				on, isBool = true, true
			case waSyncAction.SettingsSyncAction_STANDARD:
				on, isBool = false, true
			}
		}
		if !isBool {
			return
		}
		db.Exec(`INSERT OR REPLACE INTO app_settings (key, value) VALUES (?, ?)`, "pref:"+pref, boolToInt(on))
		broadcastEvent("app_prefs", appPrefsPayload())
		return
	}
	broadcastEvent("synced_setting", map[string]interface{}{"name": name, "value": value})
}

// syncSettings shares settings changed here with the other WhatsApp desktop
// devices, by WhatsApp's name and in its form (see Settings::synced).
func syncSettings(changes map[string]interface{}) {
	if len(changes) == 0 {
		return
	}
	now := time.Now()
	patch := appstate.PatchInfo{Type: appstate.WAPatchRegularLow}
	var keys []waSyncAction.SettingsSyncAction_SettingKey
	for name, value := range changes {
		fd := settingField(name)
		if fd == nil {
			continue
		}
		var val protoreflect.Value
		switch v := value.(type) {
		case bool:
			if fd.Kind() != protoreflect.BoolKind {
				continue
			}
			val = protoreflect.ValueOfBool(v)
		case float64: // a JSON number
			switch fd.Kind() {
			case protoreflect.Int32Kind:
				val = protoreflect.ValueOfInt32(int32(v))
			case protoreflect.EnumKind:
				val = protoreflect.ValueOfEnum(protoreflect.EnumNumber(v))
			default:
				continue
			}
		default:
			continue
		}
		action := &waSyncAction.SettingsSyncAction{}
		action.ProtoReflect().Set(fd, val)
		key := waSyncAction.SettingsSyncAction_SettingKey(fd.Number())
		one := appstate.BuildSettingsSync(waSyncAction.SettingsSyncAction_HYBRID, key, appstate.SettingsSyncAppTarget, action)
		patch.Mutations = append(patch.Mutations, one.Mutations...)
		keys = append(keys, key)
	}
	if len(patch.Mutations) == 0 {
		return
	}
	for _, key := range keys {
		noteSettingSyncedAt(key, now)
	}
	go func() {
		if err := sendAppStateWithRetry(context.Background(), patch); err != nil {
			log.Printf("Syncing %d settings to the other devices failed: %v", len(patch.Mutations), err)
		}
	}()
}

// handleSyncSettings takes the app's own settings that changed.
func handleSyncSettings(raw json.RawMessage) {
	var changes map[string]interface{}
	if json.Unmarshal(raw, &changes) == nil {
		syncSettings(changes)
	}
}

// syncAppPrefs shares the backend's own synced prefs that changed.
func syncAppPrefs(changed map[string]bool) {
	changes := map[string]interface{}{}
	for name, pref := range syncedAppPrefs {
		v, ok := changed[pref]
		if !ok {
			continue
		}
		if name == "mediaUploadQuality" {
			quality := waSyncAction.SettingsSyncAction_STANDARD
			if v {
				quality = waSyncAction.SettingsSyncAction_HD
			}
			changes[name] = float64(quality)
		} else {
			changes[name] = v
		}
	}
	syncSettings(changes)
}
