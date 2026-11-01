package main

import (
	"testing"

	"go.mau.fi/whatsmeow/proto/waSyncAction"
)

func TestSettingNamesMatchWhatsAppWeb(t *testing.T) {
	for name, key := range map[string]waSyncAction.SettingsSyncAction_SettingKey{
		"isEnterToSendEnabled":           waSyncAction.SettingsSyncAction_IS_ENTER_TO_SEND_ENABLED,
		"defaultNotificationToneId":      waSyncAction.SettingsSyncAction_DEFAULT_NOTIFICATION_TONE_ID,
		"groupDefaultNotificationToneId": waSyncAction.SettingsSyncAction_GROUP_DEFAULT_NOTIFICATION_TONE_ID,
		"appTheme":                       waSyncAction.SettingsSyncAction_APP_THEME,
		"mediaUploadQuality":             waSyncAction.SettingsSyncAction_MEDIA_UPLOAD_QUALITY,
		"startAtLogin":                   waSyncAction.SettingsSyncAction_START_AT_LOGIN,
	} {
		fd := settingField(name)
		if fd == nil {
			t.Errorf("%s has no field", name)
			continue
		}
		if waSyncAction.SettingsSyncAction_SettingKey(fd.Number()) != key {
			t.Errorf("%s is field %d, want key %d", name, fd.Number(), key)
		}
	}
	if settingField("nope") != nil {
		t.Error("an unknown name found a field")
	}
}
