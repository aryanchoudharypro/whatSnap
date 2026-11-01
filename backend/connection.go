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

const connBackoffMax = 900 * time.Second

const (
	connStateOnline     = "online"
	connStateConnecting = "connecting"
	connStateOffline    = "offline"
	connStateStopped    = "stopped"
)

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

func requestConnect(delay time.Duration) {
	select {
	case connRequests <- connRequest{delay: delay}:
	default:
	}
}

func reconnectNow() {
	connFailures.Store(0)
	requestConnect(0)
}

func forceReconnect() {
	connFailures.Store(0)
	select {
	case connRequests <- connRequest{force: true}:
	default:
	}
}

func isOnline(c *whatsmeow.Client) bool {
	return c != nil && c.IsConnected() && c.IsLoggedIn()
}

func handleReconnect() {
	c := wa()
	if c == nil || c.Store.ID == nil {
		return
	}
	if !isOnline(c) || currentConnState()["state"] != connStateOnline {
		forceReconnect()
		return
	}
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
		if c == nil || c.Store.ID == nil {
			continue
		}
		if !force && c.IsConnected() {
			continue
		}

		setConnState(connStateConnecting, "", 0)
		c.Disconnect()
		err := c.Connect()
		if err == nil || errors.Is(err, whatsmeow.ErrAlreadyConnected) {
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

func scheduleRetryAfterFailure(message string) {
	delay := connBackoff(connFailures.Add(1))
	setConnState(connStateOffline, message, delay)
	requestConnect(delay)
}

func autoReconnectHook(c *whatsmeow.Client) func(error) bool {
	return func(err error) bool {
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

func watchNetworkChanges() {
	proc := windows.NewLazySystemDLL("iphlpapi.dll").NewProc("NotifyAddrChange")
	if proc.Find() != nil {
		return
	}
	for {
		if r, _, _ := proc.Call(0, 0); r != 0 {
			log.Println("NotifyAddrChange failed:", r)
			return
		}
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
