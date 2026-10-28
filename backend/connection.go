package main

import (
	"context"
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

// connRequest asks the supervisor for a connect attempt. force means the
// user asked for it: drop whatever socket and reconnect wait whatsmeow has
// and dial a fresh connection even if the old socket still looks open.
type connRequest struct {
	delay time.Duration
	force bool
}

var (
	connRequests = make(chan connRequest, 8)
	connFailures atomic.Int32
	connProbing  atomic.Bool

	connStateMu  sync.Mutex
	connStateCur = map[string]interface{}{"state": connStateConnecting}
)

func init() {
	go runConnectionSupervisor()
	go watchNetworkChanges()
	go watchSocketLoss()
}

// requestConnect asks the supervisor to (re)connect after delay; 0 means now.
// A later request replaces any retry that's already scheduled.
func requestConnect(delay time.Duration) {
	select {
	case connRequests <- connRequest{delay: delay}:
	default:
	}
}

// reconnectNow is the network-change trigger: skip whatever backoff is
// pending (ours or whatsmeow's) and try immediately.
func reconnectNow() {
	connFailures.Store(0)
	requestConnect(0)
}

// forceReconnect is "Reconnect Now" when the connection isn't (really) up:
// like WA Web's Reconnect link, it throws away the current socket and any
// pending wait and dials a fresh connection right away.
func forceReconnect() {
	connFailures.Store(0)
	select {
	case connRequests <- connRequest{force: true}:
	default:
	}
}

// isOnline reports whether the socket is open and logged in. whatsmeow's
// IsLoggedIn alone stays true after the socket drops (it is only cleared by
// a stream error), so it can't tell a dead connection from a live one.
func isOnline(c *whatsmeow.Client) bool {
	return c != nil && c.IsConnected() && c.IsLoggedIn()
}

// handleReconnect is the frontend's "Reconnect Now" menu command. It always
// answers with a "connection_state", and only says online once a ping has
// actually come back over the socket.
func handleReconnect() {
	c := wa()
	if c == nil || c.Store.ID == nil {
		return
	}
	if !isOnline(c) || currentConnState()["state"] != connStateOnline {
		forceReconnect()
		return
	}
	// The socket looks up, but a half-open connection can look that way
	// for minutes before keepalives give up on it. Ask the server.
	if !connProbing.CompareAndSwap(false, true) {
		return
	}
	go func() {
		defer connProbing.Store(false)
		ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		ok, _, _ := c.DangerousInternals().SendKeepAlive(ctx)
		cancel()
		if c != wa() {
			return
		}
		if ok && isOnline(c) {
			setConnState(connStateOnline, "", 0)
			return
		}
		log.Println("Reconnect Now: ping got no answer; reconnecting")
		forceReconnect()
	}()
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
		force := false
		select {
		case req := <-connRequests:
			if req.delay > 0 {
				stopTimer()
				timer = time.NewTimer(req.delay)
				timerC = timer.C
				continue
			}
			force = req.force
			stopTimer()
		case <-timerC:
			timerC = nil
		}

		c := wa()
		// Unpaired: the QR / phone-number pairing flow owns Connect().
		if c == nil || c.Store.ID == nil {
			continue
		}
		if !force && c.IsConnected() {
			continue
		}

		setConnState(connStateConnecting, "", 0)
		// Close any half-dead socket, and cancel whatsmeow's own reconnect
		// loop if it is sitting out a backoff (Disconnect marks the drop as
		// expected, which wakes and ends that wait), so this attempt isn't
		// raced or followed by a second one when the old wait runs out.
		c.Disconnect()
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
		// Reconnect Now may already have a new socket up by the time this
		// older attempt reports its failure.
		if c == wa() && !c.IsConnected() {
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
		if c := wa(); c != nil && c.Store.ID != nil && !c.IsConnected() {
			log.Println("Network change detected; reconnecting now")
			reconnectNow()
		}
	}
}

// watchSocketLoss catches the socket going away without a Disconnected
// event, so the status bar never keeps saying online over a dead
// connection. whatsmeow sends no event when its keepalive gives up and
// reconnects (it treats that disconnect as expected), and neither does a
// failed reconnect that never got a socket.
func watchSocketLoss() {
	for range time.Tick(2 * time.Second) {
		c := wa()
		if c == nil || c.Store.ID == nil || c.IsConnected() {
			continue
		}
		if currentConnState()["state"] == connStateOnline {
			log.Println("Socket gone while marked online; showing connection lost")
			setConnState(connStateConnecting, "Connection lost.", 0)
		}
	}
}
