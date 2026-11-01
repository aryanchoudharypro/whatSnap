package main

import (
	"strconv"
	"strings"
	"testing"

	"go.mau.fi/whatsmeow/proto/waCompanionReg"
	"go.mau.fi/whatsmeow/proto/waWa6"
	"go.mau.fi/whatsmeow/store"
)

func TestParseWebViewMajor(t *testing.T) {
	for version, want := range map[string]int{
		"155.0.4283.45": 155,
		"0.0.0.0":       0,
		"":              0,
		"garbage":       0,
	} {
		if got := parseWebViewMajor(version); got != want {
			t.Errorf("parseWebViewMajor(%q) = %d, want %d", version, got, want)
		}
	}
}

func TestWebViewUserAgentIsAnEdgeWebView(t *testing.T) {
	ua := webViewUserAgent()
	if !strings.HasPrefix(ua, "Mozilla/5.0 (Windows NT 10.0; Win64; x64)") || !strings.Contains(ua, " Edg/") {
		t.Fatalf("unexpected user agent %q", ua)
	}
}

func TestClientIdentityIsTheWindowsAppExceptSubPlatform(t *testing.T) {
	applyClientIdentity()
	agent := store.BaseClientPayload.GetUserAgent()
	if got := store.BaseClientPayload.GetWebInfo().GetWebSubPlatform(); got == waWa6.ClientPayload_WebInfo_WIN_HYBRID {
		t.Errorf("sub-platform %v breaks calls", got)
	}
	if got := store.DeviceProps.GetPlatformType(); got != waCompanionReg.DeviceProps_UWP {
		t.Errorf("platform type %v", got)
	}
	if got := agent.GetAppVersion().GetQuaternary(); got != windowsAppBuild {
		t.Errorf("fourth version part %d", got)
	}
	if got := agent.GetMcc() + agent.GetMnc(); got != strconv.Itoa(windowsAppBuild) || len(agent.GetMcc()) != 3 {
		t.Errorf("mcc+mnc %q isn't the build's six digits", got)
	}
	// The registration build hash is over the three-part web version, so the
	// first three parts must stay whatever the library pinned.
	web := store.GetWAVersion()
	if agent.GetAppVersion().GetPrimary() != web[0] || agent.GetAppVersion().GetSecondary() != web[1] || agent.GetAppVersion().GetTertiary() != web[2] {
		t.Errorf("web version changed: %v", agent.GetAppVersion())
	}
	version := store.DeviceProps.GetVersion()
	if version.GetPrimary() != 10 || version.Secondary != nil || version.Tertiary != nil {
		t.Errorf("device props version %v, want just 10", version)
	}
}
