package main

import (
	"errors"
	"log"
	"net"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"go.mau.fi/whatsmeow"
	"golang.org/x/sys/windows"
)

// Connection supervisor. whatsmeow's own auto-reconnect only covers a socket
// that drops *after* a successful connect - if the very first Connect() fails
// (no network at boot, DNS not up yet, WhatsApp briefly unreachable) it just
// returns the error and nothing ever tries again, leaving the app silently
// dead until restarted. Several connect failures (unknown ConnectFailure
// reasons, CAT refresh errors, temp bans) also switch auto-reconnect off.
// This loop owns every connect attempt we start ourselves and keeps retrying
// with capped backoff, so the app always finds its way back online.

// connBackoffMax caps the wait between attempts, as WA Web does (900 s).
// A network change or "Reconnect Now" skips whatever wait is pending.
const connBackoffMax = 900 * time.Second

// Connection states reported to the frontend via "connection_state".
const (
	connStateOnline     = "online"
	connStateConnecting = "connecting"
	connStateOffline    = "offline" // an attempt failed; another is scheduled
	connStateStopped    = "stopped" // won't retry on its own (replaced, outdated)
)

var (
	connRequests = make(chan time.Duration, 8)
	connFailures atomic.Int32

	connStateMu  sync.Mutex
	connStateCur = map[string]interface{}{"state": connStateConnecting}
)

func init() {
	go runConnectionSupervisor()
	go watchNetworkChanges()
}

// requestConnect asks the supervisor to (re)connect after delay; 0 means now.
// A later request replaces any retry that's already scheduled.
func requestConnect(delay time.Duration) {
	select {
	case connRequests <- delay:
	default:
	}
}

// reconnectNow is the manual "Reconnect Now" command and the network-change
// trigger: skip whatever backoff is pending and try immediately.
func reconnectNow() {
	connFailures.Store(0)
	requestConnect(0)
}

// handleReconnect is the frontend's "Reconnect Now" menu command.
func handleReconnect() {
	if c := wa(); c != nil && c.IsLoggedIn() {
		setConnState(connStateOnline, "", 0) // still answer, so the UI can confirm
		return
	}
	reconnectNow()
}

// connBackoff is the wait after the given number of failed attempts in a row:
// WA Web's Fibonacci seconds (1, 1, 2, 3, 5, 8, ...) up to 15 minutes, the
// same schedule whatsmeow's own reconnect loop now follows. Retrying a server
// that keeps refusing the connection every minute forever is what a
// misbehaving client looks like.
func connBackoff(failures int32) time.Duration {
	if failures <= 0 {
		return 0
	}
	a, b := 1, 1
	for i := int32(1); i < failures && time.Duration(a)*time.Second < connBackoffMax; i++ {
		a, b = b, a+b
	}
	return min(time.Duration(a)*time.Second, connBackoffMax)
}

func runConnectionSupervisor() {
	var timer *time.Timer
	var timerC <-chan time.Time
	stopTimer := func() {
		if timer != nil {
			timer.Stop()
		}
		timerC = nil
	}

	for {
		select {
		case delay := <-connRequests:
			if delay > 0 {
				stopTimer()
				timer = time.NewTimer(delay)
				timerC = timer.C
				continue
			}
			stopTimer()
		case <-timerC:
			timerC = nil
		}

		c := wa()
		// Unpaired: the QR / phone-number pairing flow owns Connect().
		if c == nil || c.Store.ID == nil || c.IsLoggedIn() {
			continue
		}

		setConnState(connStateConnecting, "", 0)
		err := c.Connect()
		if err == nil || errors.Is(err, whatsmeow.ErrAlreadyConnected) {
			// Socket is up (or whatsmeow's own reconnect beat us to it); the
			// Connected / ConnectFailure events take it from here.
			continue
		}
		log.Println("Connect error:", err)
		n := connFailures.Add(1)
		delay := connBackoff(n)
		setConnState(connStateOffline, describeConnError(err), delay)
		stopTimer()
		timer = time.NewTimer(delay)
		timerC = timer.C
	}
}

// scheduleRetryAfterFailure is for failures whatsmeow won't recover from on
// its own (it has already marked the disconnect as expected).
func scheduleRetryAfterFailure(message string) {
	delay := connBackoff(connFailures.Add(1))
	setConnState(connStateOffline, message, delay)
	requestConnect(delay)
}

// autoReconnectHook runs inside whatsmeow's own reconnect loop after each
// failed attempt. Returning true keeps that loop going.
func autoReconnectHook(c *whatsmeow.Client) func(error) bool {
	return func(err error) bool {
		if c == wa() {
			setConnState(connStateOffline, describeConnError(err), connBackoff(int32(c.AutoReconnectErrors)))
		}
		return true
	}
}

func setConnState(state, message string, retryIn time.Duration) {
	data := map[string]interface{}{"state": state}
	if message != "" {
		data["message"] = message
	}
	if retryIn > 0 {
		data["retry_in"] = int(retryIn.Round(time.Second).Seconds())
	}
	connStateMu.Lock()
	connStateCur = data
	connStateMu.Unlock()
	broadcastEvent("connection_state", data)
}

func currentConnState() map[string]interface{} {
	connStateMu.Lock()
	defer connStateMu.Unlock()
	return connStateCur
}

// describeConnError turns a dial/handshake error into something a user can
// act on; the raw error still goes to the backend log.
func describeConnError(err error) string {
	var dnsErr *net.DNSError
	var opErr *net.OpError
	msg := strings.ToLower(err.Error())
	switch {
	case errors.As(err, &dnsErr), errors.As(err, &opErr),
		strings.Contains(msg, "timeout"), strings.Contains(msg, "connection refused"), strings.Contains(msg, "connection reset"):
		return "Can't reach WhatsApp. Check your internet connection."
	case strings.Contains(msg, "handshake"):
		return "Couldn't complete the secure connection to WhatsApp."
	default:
		return "Couldn't connect to WhatsApp."
	}
}

// watchNetworkChanges wakes the supervisor as soon as Windows reports an IP
// address change (Wi-Fi rejoined, cable plugged back in, VPN up), so coming
// back online doesn't mean waiting out a 30-60s backoff.
func watchNetworkChanges() {
	proc := windows.NewLazySystemDLL("iphlpapi.dll").NewProc("NotifyAddrChange")
	if proc.Find() != nil {
		return
	}
	for {
		// With both arguments NULL the call blocks until the next change.
		if r, _, _ := proc.Call(0, 0); r != 0 {
			log.Println("NotifyAddrChange failed:", r)
			return
		}
		// Addresses usually settle a moment before DNS/routes are ready.
		time.Sleep(2 * time.Second)
		// "stopped" (session replaced, client outdated) must stay manual -
		// auto-reconnecting after a replace would bounce the other session.
		if currentConnState()["state"] == connStateStopped {
			continue
		}
		if c := wa(); c != nil && c.Store.ID != nil && !c.IsLoggedIn() {
			log.Println("Network change detected; reconnecting now")
			reconnectNow()
		}
	}
}
