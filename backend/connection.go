package main

import (
	"context"
	"errors"
	"log"
	"net"
	"strings"
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

func init() {
	go watchNetworkChanges()
}

func (acc *Account) requestConnect(delay time.Duration) {
	select {
	case acc.connRequests <- connRequest{delay: delay}:
	default:
	}
}

func (acc *Account) reconnectNow() {
	acc.connFailures.Store(0)
	acc.requestConnect(0)
}

func (acc *Account) forceReconnect() {
	acc.connFailures.Store(0)
	select {
	case acc.connRequests <- connRequest{force: true}:
	default:
	}
}

func isOnline(c *whatsmeow.Client) bool {
	return c != nil && c.IsConnected() && c.IsLoggedIn()
}

func (acc *Account) handleReconnect() {
	c := acc.wa()
	if c == nil || c.Store.ID == nil {
		return
	}
	if !isOnline(c) || acc.currentConnState()["state"] != connStateOnline {
		acc.forceReconnect()
		return
	}
	if !acc.connProbing.CompareAndSwap(false, true) {
		return
	}
	go func() {
		defer acc.connProbing.Store(false)
		ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		ok, _, _ := c.DangerousInternals().SendKeepAlive(ctx)
		cancel()
		if c != acc.wa() {
			return
		}
		if ok && isOnline(c) {
			acc.setConnState(connStateOnline, "", 0)
			return
		}
		log.Println("Reconnect Now: ping got no answer; reconnecting")
		acc.forceReconnect()
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

func (acc *Account) runConnectionSupervisor() {
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
		case <-acc.ctx.Done():
			stopTimer()
			return
		case req := <-acc.connRequests:
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

		c := acc.wa()
		if c == nil || c.Store.ID == nil {
			continue
		}
		if !force && c.IsConnected() {
			continue
		}

		acc.setConnState(connStateConnecting, "", 0)
		c.Disconnect()
		err := c.Connect()
		if err == nil || errors.Is(err, whatsmeow.ErrAlreadyConnected) {
			continue
		}
		log.Println("Connect error:", err)
		n := acc.connFailures.Add(1)
		delay := connBackoff(n)
		acc.setConnState(connStateOffline, describeConnError(err), delay)
		stopTimer()
		timer = time.NewTimer(delay)
		timerC = timer.C
	}
}

func (acc *Account) scheduleRetryAfterFailure(message string) {
	delay := connBackoff(acc.connFailures.Add(1))
	acc.setConnState(connStateOffline, message, delay)
	acc.requestConnect(delay)
}

func (acc *Account) autoReconnectHook(c *whatsmeow.Client) func(error) bool {
	return func(err error) bool {
		if c == acc.wa() && !c.IsConnected() {
			acc.setConnState(connStateOffline, describeConnError(err), connBackoff(int32(c.AutoReconnectErrors)))
		}
		return true
	}
}

func (acc *Account) setConnState(state, message string, retryIn time.Duration) {
	data := map[string]interface{}{"state": state}
	if message != "" {
		data["message"] = message
	}
	if retryIn > 0 {
		data["retry_in"] = int(retryIn.Round(time.Second).Seconds())
	}
	acc.connStateMu.Lock()
	acc.connStateCur = data
	acc.connStateMu.Unlock()
	acc.broadcastEvent("connection_state", data)
}

func (acc *Account) currentConnState() map[string]interface{} {
	acc.connStateMu.Lock()
	defer acc.connStateMu.Unlock()
	return acc.connStateCur
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

// watchNetworkChanges reconnects every account that lost its socket as soon
// as Windows reports an address change, instead of waiting out its backoff.
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
		for _, acc := range openAccounts() {
			// "stopped" (session replaced, client outdated) must stay manual -
			// auto-reconnecting after a replace would bounce the other session.
			if acc.currentConnState()["state"] == connStateStopped {
				continue
			}
			if c := acc.wa(); c != nil && c.Store.ID != nil && !c.IsConnected() {
				log.Printf("Network change detected; reconnecting account %s now", acc.id)
				acc.reconnectNow()
			}
		}
	}
}

// watchSocketLoss catches the socket going away without a Disconnected
// event, so the status bar never keeps saying online over a dead
// connection. whatsmeow sends no event when its keepalive gives up and
// reconnects (it treats that disconnect as expected), and neither does a
// failed reconnect that never got a socket.
func (acc *Account) watchSocketLoss() {
	ticker := time.NewTicker(2 * time.Second)
	defer ticker.Stop()
	for {
		select {
		case <-acc.ctx.Done():
			return
		case <-ticker.C:
		}
		c := acc.wa()
		if c == nil || c.Store.ID == nil || c.IsConnected() {
			continue
		}
		if acc.currentConnState()["state"] == connStateOnline {
			log.Println("Socket gone while marked online; showing connection lost")
			acc.setConnState(connStateConnecting, "Connection lost.", 0)
		}
	}
}
