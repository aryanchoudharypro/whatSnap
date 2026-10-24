package main

import (
	"fmt"
	"strconv"
	"strings"

	"go.mau.fi/whatsmeow/proto/waCompanionReg"
	"go.mau.fi/whatsmeow/store"
	"golang.org/x/sys/windows/registry"
	"google.golang.org/protobuf/proto"
)

// windowsAppBuild is the build number of the WhatsApp Windows app whatSnap
// presents itself as. The app hands it to WhatsApp Web as the windowsBuild
// query parameter: Store version 2.2639.100.0 is 263900, 2.2638.102.0 was
// 263802 (minor version, then the third part minus 100 as two digits).
//
// To refresh it, install the current Store app and read the number from the
// URL in its WebView history
// (%LOCALAPPDATA%\Packages\5319275A.WhatsAppDesktop_*\LocalCache\EBWebView\Default\History).
// A real install stays on one build until the Store updates it, so being a
// release or two behind is ordinary.
const windowsAppBuild = 263900

// webViewFallbackMajor is the WebView2 major version used in the User-Agent
// when this machine has no WebView2 runtime to read it from.
const webViewFallbackMajor = 155

// webView2ClientKey is where the WebView2 runtime records its version.
const webView2ClientKey = `Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}`

// applyClientIdentity makes what whatSnap tells WhatsApp about what it is
// describe the Windows app, which is WhatsApp Web in a WebView
// (WAWebEnvironment.isWindows), as far as calls allow. WA Web builds exactly
// two identities (WAWebClientPayload): a browser, or the Windows app with
// platform UWP in the device props together with sub-platform WIN_HYBRID, a
// four-part version and the build's digits as mcc/mnc in the login payload.
// whatSnap sends the second except for the sub-platform.
//
// The device props only go out when linking; the login payload on every
// connect.
func applyClientIdentity() {
	build := strconv.Itoa(windowsAppBuild)
	agent := store.BaseClientPayload.UserAgent
	agent.AppVersion.Quaternary = proto.Uint32(windowsAppBuild)
	if len(build) == 6 {
		agent.Mcc = proto.String(build[:3])
		agent.Mnc = proto.String(build[3:])
	}
	// The sub-platform stays a browser's. Logged in as WIN_HYBRID, the call
	// relays stop answering the DTLS/SCTP media transport meowcaller speaks
	// (WA Web's own), so no call ever connects.

	store.DeviceProps.PlatformType = waCompanionReg.DeviceProps_UWP.Enum()
	store.DeviceProps.Os = proto.String("Windows")
	// WA Web sends the OS version its user-agent parser reports, which for
	// every Windows since 10 is just "10".
	store.DeviceProps.Version = &waCompanionReg.DeviceProps_AppVersion{Primary: proto.Uint32(10)}
}

// webViewUserAgent is the User-Agent of the WebView2 the Windows app runs
// WhatsApp Web in, for the websocket handshake and media transfers. Without
// one, Go's own ("Go-http-client/1.1") goes out on a connection that claims
// to be a browser. It follows this machine's WebView2 runtime so it moves on
// with updates the way a real install's does.
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

// parseWebViewMajor reads the major version out of a runtime version such as
// "155.0.4283.45". An uninstalled runtime leaves "0.0.0.0" behind.
func parseWebViewMajor(version string) int {
	first, _, _ := strings.Cut(version, ".")
	major, err := strconv.Atoi(first)
	if err != nil || major < 100 {
		return 0
	}
	return major
}
