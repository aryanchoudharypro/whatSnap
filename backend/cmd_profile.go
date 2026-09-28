package main

import (
	"context"
	"encoding/json"
	"net"

	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/types"
)

// handleGetOwnProfile returns the local user's push name and about/status
// text, for populating the privacy tab's profile fields.
func handleGetOwnProfile(conn net.Conn) {
	go func() {
		if wa().Store.ID == nil {
			sendEvent(conn, "error", map[string]string{"message": "not logged in"})
			return
		}
		myJID := wa().Store.ID.ToNonAD()
		about := ""
		// GetUserInfo's Status comes from the legacy usync <status/> node,
		// which the server stops keeping in sync with SetStatusMessage's
		// newer mex mutation - GetTextStatus is the read-side counterpart
		// of that mutation and is the only reliable way to get a fresh
		// value.
		if status, err := wa().GetTextStatus(context.Background(), myJID); err == nil && status != nil {
			about = status.Text
		}
		sendEvent(conn, "own_profile", map[string]interface{}{
			"jid":      myJID.String(),
			"pushName": wa().Store.PushName,
			"about":    about,
		})
	}()
}

func handleSetPushName(raw json.RawMessage) {
	var data SetPushNameData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		ctx := context.Background()
		if err := wa().SendAppState(ctx, appstate.BuildSettingPushName(data.Name)); err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		wa().Store.PushName = data.Name
		if err := wa().Store.Save(ctx); err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		broadcastEvent("own_profile", map[string]interface{}{
			"jid":      wa().Store.ID.ToNonAD().String(),
			"pushName": wa().Store.PushName,
		})
	}()
}

func handleGetPrivacySettings(conn net.Conn) {
	go func() {
		settings, err := wa().TryFetchPrivacySettings(context.Background(), true)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": err.Error()})
			return
		}
		sendEvent(conn, "privacy_settings", privacySettingsToMap(settings))
	}()
}

func handleSetPrivacySetting(raw json.RawMessage) {
	var data SetPrivacySettingData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		settings, err := wa().SetPrivacySetting(
			context.Background(),
			types.PrivacySettingType(data.Name),
			types.PrivacySetting(data.Value),
		)
		if err != nil {
			broadcastEvent("error", map[string]string{"message": err.Error()})
			return
		}
		broadcastEvent("privacy_settings", privacySettingsToMap(&settings))
	}()
}

func privacySettingsToMap(settings *types.PrivacySettings) map[string]interface{} {
	return map[string]interface{}{
		"groupadd":     string(settings.GroupAdd),
		"last":         string(settings.LastSeen),
		"status":       string(settings.Status),
		"profile":      string(settings.Profile),
		"readreceipts": string(settings.ReadReceipts),
		"online":       string(settings.Online),
		"calladd":      string(settings.CallAdd),
	}
}
