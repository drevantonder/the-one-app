// Package mesh keeps a user's nodes in step over their tailnet. It relies on
// the Tailscale already running on each device: peers are the user's own
// online machines, and every request is checked against the caller's
// Tailscale identity.
package mesh

import (
	"context"
	"encoding/json"
	"errors"
	"net"
	"os/exec"
	"strings"
	"sync"
	"time"
)

type tsStatus struct {
	BackendState string
	Self         tsPeer
	Peer         map[string]tsPeer
}

type tsPeer struct {
	HostName     string
	DNSName      string
	TailscaleIPs []string
	UserID       int64
	Online       bool
	OS           string
	LastSeen     time.Time
}

func (p tsPeer) ipv4() string {
	for _, ip := range p.TailscaleIPs {
		if !strings.Contains(ip, ":") {
			return ip
		}
	}
	return ""
}

// Name is the machine's short MagicDNS name.
func (p tsPeer) Name() string { return strings.SplitN(p.DNSName, ".", 2)[0] }

func tailscale(ctx context.Context, args ...string) ([]byte, error) {
	ctx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	return exec.CommandContext(ctx, "tailscale", args...).Output()
}

func status(ctx context.Context) (tsStatus, error) {
	var st tsStatus
	out, err := tailscale(ctx, "status", "--json")
	if err != nil {
		return st, err
	}
	if err := json.Unmarshal(out, &st); err != nil {
		return st, err
	}
	if st.BackendState != "Running" {
		return st, errors.New("tailscale is " + st.BackendState)
	}
	return st, nil
}

// whois resolves a caller's Tailscale user, cached briefly per address.
type whois struct {
	mu    sync.Mutex
	cache map[string]whoisEntry
}

type whoisEntry struct {
	user    int64
	login   string
	expires time.Time
}

func (w *whois) lookup(ctx context.Context, addr string) (int64, string, error) {
	host, _, err := net.SplitHostPort(addr)
	if err != nil {
		host = addr
	}
	w.mu.Lock()
	if e, ok := w.cache[host]; ok && time.Now().Before(e.expires) {
		w.mu.Unlock()
		return e.user, e.login, nil
	}
	w.mu.Unlock()
	out, err := tailscale(ctx, "whois", "--json", host)
	if err != nil {
		return 0, "", err
	}
	var r struct {
		UserProfile struct {
			ID        int64
			LoginName string
		}
	}
	if err := json.Unmarshal(out, &r); err != nil {
		return 0, "", err
	}
	w.mu.Lock()
	w.cache[host] = whoisEntry{r.UserProfile.ID, r.UserProfile.LoginName, time.Now().Add(time.Minute)}
	w.mu.Unlock()
	return r.UserProfile.ID, r.UserProfile.LoginName, nil
}
