package main

import (
	"fmt"
	"strconv"
	"strings"

	"go.mau.fi/whatsmeow/proto/waCompanionReg"
	"go.mau.fi/whatsmeow/proto/waWa6"
	"go.mau.fi/whatsmeow/store"
	"golang.org/x/sys/windows"
	"golang.org/x/sys/windows/registry"
	"google.golang.org/protobuf/proto"
)

// tools/update_wa_versions.py rewrites these two numbers.
const webClientRevision = 1053903555

const windowsAppBuild = 264101

const webViewFallbackMajor = 155

const webView2ClientKey = `Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}`

func applyClientIdentity() {
	store.SetWAVersion(store.WAVersionContainer{2, 3000, webClientRevision})
	build := strconv.Itoa(windowsAppBuild)
	agent := store.BaseClientPayload.UserAgent
	agent.AppVersion.Quaternary = proto.Uint32(windowsAppBuild)
	if len(build) == 6 {
		agent.Mcc = proto.String(build[:3])
		agent.Mnc = proto.String(build[3:])
	}

	if languages, err := windows.GetUserPreferredUILanguages(windows.MUI_LANGUAGE_NAME); err == nil && len(languages) > 0 {
		if language, country := parseLocale(languages[0]); language != "" {
			agent.LocaleLanguageIso6391 = proto.String(language)
			if country != "" {
				agent.LocaleCountryIso31661Alpha2 = proto.String(country)
			}
		}
	}

	store.DeviceProps.PlatformType = waCompanionReg.DeviceProps_UWP.Enum()
	store.DeviceProps.Os = proto.String("Windows")
	store.DeviceProps.Version = &waCompanionReg.DeviceProps_AppVersion{Primary: proto.Uint32(10)}

	// WIN_HYBRID is the sub-platform WhatsApp for Windows sends. It goes out
	// on every connect, so it needs no new link. Calls with it get native
	// relay blocks, which meowcaller dials over plain UDP like the real app.
	store.BaseClientPayload.WebInfo.WebSubPlatform = waWa6.ClientPayload_WebInfo_WIN_HYBRID.Enum()
}

// parseLocale splits a name like "en-GB" or "sr-Latn-RS" the way WhatsApp
// Web does for the language and country it sends at login.
func parseLocale(name string) (language, country string) {
	parts := strings.Split(name, "-")
	if !isLetters(parts[0], 2, 3) {
		return "", ""
	}
	next := 1
	if next < len(parts) && isLetters(parts[next], 4, 4) {
		next++
	}
	if next < len(parts) && (isLetters(parts[next], 2, 2) || isDigits(parts[next], 3)) {
		country = parts[next]
	}
	return parts[0], country
}

func isLetters(text string, shortest, longest int) bool {
	if len(text) < shortest || len(text) > longest {
		return false
	}
	for _, char := range text {
		if (char < 'a' || char > 'z') && (char < 'A' || char > 'Z') {
			return false
		}
	}
	return true
}

func isDigits(text string, length int) bool {
	if len(text) != length {
		return false
	}
	for _, char := range text {
		if char < '0' || char > '9' {
			return false
		}
	}
	return true
}

func webViewUserAgent() string {
	major := webViewFallbackMajor
	for _, root := range []struct {
		key  registry.Key
		path string
	}{
		{registry.LOCAL_MACHINE, `SOFTWARE\WOW6432Node\` + webView2ClientKey},
		{registry.LOCAL_MACHINE, `SOFTWARE\` + webView2ClientKey},
		{registry.CURRENT_USER, `Software\` + webView2ClientKey},
	} {
		if found := webViewMajorFrom(root.key, root.path); found > 0 {
			major = found
			break
		}
	}
	return fmt.Sprintf("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/%d.0.0.0 Safari/537.36 Edg/%d.0.0.0", major, major)
}

func webViewMajorFrom(root registry.Key, path string) int {
	key, err := registry.OpenKey(root, path, registry.QUERY_VALUE)
	if err != nil {
		return 0
	}
	defer key.Close()
	version, _, err := key.GetStringValue("pv")
	if err != nil {
		return 0
	}
	return parseWebViewMajor(version)
}

func parseWebViewMajor(version string) int {
	first, _, _ := strings.Cut(version, ".")
	major, err := strconv.Atoi(first)
	if err != nil || major < 100 {
		return 0
	}
	return major
}
