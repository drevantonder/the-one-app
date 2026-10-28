package mesh

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"net"
	"net/http"
	"slices"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/drevantonder/the-one-app/internal/core"
	"github.com/drevantonder/the-one-app/internal/store"
	"github.com/drevantonder/the-one-app/protocol"
)

const (
	pullWait  = 25 * time.Second
	pullLimit = 1000
)

type Mesh struct {
	node  *core.Node
	store *store.Store
	port  string
	log   *slog.Logger
	self  tsPeer
	who   whois

	mu      sync.Mutex
	peers   map[string]*peer  // by tailnet IP
	devices map[string]string // node device id → tailnet IP

	// OnPeer runs after each successful pull from a peer, with its base URL.
	OnPeer func(ctx context.Context, base string)
}

type peer struct {
	name     string
	ip       string
	device   string
	cancel   context.CancelFunc
	reached  bool
	lastSync time.Time
	lastErr  string
	ops      []core.Op
	opsAt    time.Time
}

func New(node *core.Node, st *store.Store, port string, log *slog.Logger) *Mesh {
	return &Mesh{node: node, store: st, port: port, log: log, who: whois{cache: map[string]whoisEntry{}}, peers: map[string]*peer{}, devices: map[string]string{}}
}

type pullRequest struct {
	Vector map[string]int64 `json:"vector"`
	WaitMs int              `json:"waitMs"`
}

type pullResponse struct {
	Device  string           `json:"device"`
	Entries []protocol.Entry `json:"entries"`
}

// Routes adds the sync endpoints to a mux.
func (m *Mesh) Routes(mux *http.ServeMux) {
	mux.HandleFunc("POST /v1/sync/pull", m.pull)
}

// pull long-polls: it answers with every entry the caller lacks, waiting up
// to waitMs for some to appear.
func (m *Mesh) pull(w http.ResponseWriter, r *http.Request) {
	var req pullRequest
	if err := json.NewDecoder(http.MaxBytesReader(w, r.Body, 1<<20)).Decode(&req); err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}
	wait := min(time.Duration(req.WaitMs)*time.Millisecond, 60*time.Second)
	deadline := time.After(wait)
	for {
		changed := m.store.Changed()
		entries, err := m.store.After(req.Vector, pullLimit)
		if err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}
		if len(entries) > 0 || wait == 0 {
			writeJSON(w, pullResponse{Device: m.store.Device(), Entries: entries})
			return
		}
		select {
		case <-changed:
		case <-deadline:
			wait = 0
		case <-r.Context().Done():
			return
		}
	}
}

// Start joins the tailnet side: it serves h on this machine's tailnet
// address (to the same Tailscale user only) and syncs with peer nodes. If
// Tailscale isn't running, the node simply stays local.
func (m *Mesh) Start(ctx context.Context, h http.Handler) error {
	st, err := status(ctx)
	if err != nil {
		m.log.Warn("tailnet unavailable; this node stays local", "err", err)
		return nil
	}
	m.self = st.Self
	ip := st.Self.ipv4()
	if ip == "" {
		return errors.New("this machine has no tailnet IPv4 address")
	}
	ln, err := net.Listen("tcp", net.JoinHostPort(ip, m.port))
	if err != nil {
		return err
	}
	srv := &http.Server{Handler: m.auth(h), ReadHeaderTimeout: 10 * time.Second}
	go func() {
		<-ctx.Done()
		sctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_ = srv.Shutdown(sctx)
	}()
	go func() {
		if err := srv.Serve(ln); err != nil && !errors.Is(err, http.ErrServerClosed) {
			m.log.Error("tailnet listener stopped", "err", err)
		}
	}()
	m.node.SetName(st.Self.Name())
	m.log.Info("on the tailnet", "name", st.Self.Name(), "url", fmt.Sprintf("http://%s:%s", strings.TrimSuffix(st.Self.DNSName, "."), m.port))
	m.node.AddCoreOp("peers", "List this user's other devices on the tailnet and whether their nodes are syncing.", core.SchemaFor[struct{}](), true,
		func(context.Context, json.RawMessage, protocol.Caller) (protocol.Result, error) {
			return m.peersResult(), nil
		})
	go m.discover(ctx)
	return nil
}

// auth admits only requests from this node's own Tailscale user.
func (m *Mesh) auth(h http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		user, login, err := m.who.lookup(r.Context(), r.RemoteAddr)
		if err != nil || user != m.self.UserID {
			http.Error(w, "forbidden: not this node's owner", http.StatusForbidden)
			return
		}
		r.Header.Set("X-One-Actor", login)
		h.ServeHTTP(w, r)
	})
}

func (m *Mesh) discover(ctx context.Context) {
	t := time.NewTicker(15 * time.Second)
	defer t.Stop()
	for {
		m.refresh(ctx)
		select {
		case <-ctx.Done():
			return
		case <-t.C:
		}
	}
}

func (m *Mesh) refresh(ctx context.Context) {
	st, err := status(ctx)
	if err != nil {
		m.log.Warn("tailscale status", "err", err)
		return
	}
	want := map[string]tsPeer{}
	for _, p := range st.Peer {
		if p.UserID == st.Self.UserID && p.Online && p.ipv4() != "" {
			want[p.ipv4()] = p
		}
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	for ip, p := range m.peers {
		if _, ok := want[ip]; !ok {
			p.cancel()
			delete(m.peers, ip)
			go m.publishRemote() // runs once refresh releases the lock
		}
	}
	for ip, tp := range want {
		if _, ok := m.peers[ip]; ok {
			continue
		}
		pctx, cancel := context.WithCancel(ctx)
		p := &peer{name: tp.Name(), ip: ip, cancel: cancel}
		m.peers[ip] = p
		go m.follow(pctx, p)
	}
}

var client = &http.Client{Timeout: pullWait + 15*time.Second}

// follow keeps pulling a peer's entries for as long as it is online.
func (m *Mesh) follow(ctx context.Context, p *peer) {
	backoff := 5 * time.Second
	wait := time.Duration(0) // the first pull answers at once, so status is accurate
	for ctx.Err() == nil {
		n, err := m.pullOnce(ctx, p, wait)
		wait = pullWait
		if err != nil {
			wait = 0
		}
		lost := false
		m.mu.Lock()
		if err != nil {
			lost = p.reached
			p.reached, p.lastErr = false, err.Error()
		} else {
			p.reached, p.lastErr, p.lastSync = true, "", time.Now()
		}
		m.mu.Unlock()
		if lost {
			m.publishRemote() // its operations are gone until it is back
		}
		switch {
		case err != nil:
			select {
			case <-ctx.Done():
			case <-time.After(backoff):
			}
			backoff = min(backoff*2, time.Minute)
		case n > 0:
			backoff = 5 * time.Second
			m.peerOps(ctx, p)
			if m.OnPeer != nil {
				m.OnPeer(ctx, m.base(p))
			}
			m.log.Info("synced", "peer", p.name, "entries", n)
		default:
			backoff = 5 * time.Second
			m.peerOps(ctx, p)
			if m.OnPeer != nil {
				m.OnPeer(ctx, m.base(p))
			}
		}
	}
}

// peerOps refreshes, at most every 30s, which operations a peer runs
// locally, and gives the node the union of every peer's.
func (m *Mesh) peerOps(ctx context.Context, p *peer) {
	m.mu.Lock()
	fresh := time.Since(p.opsAt) < 30*time.Second
	m.mu.Unlock()
	if fresh {
		return
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, m.base(p)+"/v1/operations?local=1", nil)
	if err != nil {
		return
	}
	resp, err := client.Do(req)
	if err != nil {
		return
	}
	defer resp.Body.Close()
	var ops []core.Op
	if resp.StatusCode != http.StatusOK || json.NewDecoder(resp.Body).Decode(&ops) != nil {
		return
	}
	m.mu.Lock()
	p.ops, p.opsAt = ops, time.Now()
	m.mu.Unlock()
	m.publishRemote()
}

// publishRemote gives the node every reachable peer's operations, in a
// stable order.
func (m *Mesh) publishRemote() {
	m.mu.Lock()
	var all []core.RemoteOp
	ips := make([]string, 0, len(m.peers))
	for ip := range m.peers {
		ips = append(ips, ip)
	}
	sort.Strings(ips)
	for _, ip := range ips {
		p := m.peers[ip]
		if !p.reached || p.device == "" {
			continue
		}
		for _, o := range p.ops {
			all = append(all, core.RemoteOp{Op: o, Device: p.device, DeviceName: p.name})
		}
	}
	m.mu.Unlock()
	m.node.SetRemoteOps(all)
}

func (m *Mesh) base(p *peer) string { return "http://" + net.JoinHostPort(p.ip, m.port) }

func (m *Mesh) pullOnce(ctx context.Context, p *peer, wait time.Duration) (int, error) {
	vec, err := m.store.Vector()
	if err != nil {
		return 0, err
	}
	body, _ := json.Marshal(pullRequest{Vector: vec, WaitMs: int(wait / time.Millisecond)})
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, m.base(p)+"/v1/sync/pull", bytes.NewReader(body))
	if err != nil {
		return 0, err
	}
	resp, err := client.Do(req)
	if err != nil {
		return 0, err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return 0, fmt.Errorf("%s", resp.Status)
	}
	var out pullResponse
	if err := json.NewDecoder(resp.Body).Decode(&out); err != nil {
		return 0, err
	}
	if out.Device != "" {
		m.mu.Lock()
		m.devices[out.Device] = p.ip
		p.device = out.Device
		m.mu.Unlock()
	}
	return m.node.Ingest(ctx, out.Entries)
}

// Forward runs an operation on another device's node over the tailnet.
func (m *Mesh) Forward(ctx context.Context, device, plugin, op string, input json.RawMessage, _ protocol.Caller) (protocol.Result, error) {
	m.mu.Lock()
	ip, ok := m.devices[device]
	m.mu.Unlock()
	if !ok {
		return protocol.Result{}, fmt.Errorf("device %s isn't reachable right now", device)
	}
	return m.call(ctx, ip, plugin, op, input)
}

func (m *Mesh) call(ctx context.Context, ip, plugin, op string, input json.RawMessage) (protocol.Result, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, "http://"+net.JoinHostPort(ip, m.port)+"/v1/call/"+plugin+"/"+op, bytes.NewReader(input))
	if err != nil {
		return protocol.Result{}, err
	}
	req.Header.Set("Content-Type", "application/json")
	resp, err := client.Do(req)
	if err != nil {
		return protocol.Result{}, fmt.Errorf("reaching %s: %w", ip, err)
	}
	defer resp.Body.Close()
	var out struct {
		protocol.Result
		Error string `json:"error"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&out); err != nil {
		return protocol.Result{}, err
	}
	if out.Error != "" {
		return protocol.Result{}, errors.New(out.Error)
	}
	return out.Result, nil
}

func (m *Mesh) peersResult() protocol.Result {
	type row struct {
		Name     string `json:"name"`
		IP       string `json:"ip"`
		Syncing  bool   `json:"syncing"`
		LastSync string `json:"lastSync,omitempty"`
		Error    string `json:"error,omitempty"`
	}
	m.mu.Lock()
	var rows []row
	for _, p := range m.peers {
		r := row{Name: p.name, IP: p.ip, Syncing: p.reached, Error: p.lastErr}
		if !p.lastSync.IsZero() {
			r.LastSync = p.lastSync.Format(time.RFC3339)
		}
		rows = append(rows, r)
	}
	m.mu.Unlock()
	sort.Slice(rows, func(i, j int) bool { return rows[i].Name < rows[j].Name })
	var lines []string
	for _, r := range slices.Clone(rows) {
		state := "no node reachable"
		if r.Syncing {
			state = "syncing"
		}
		lines = append(lines, fmt.Sprintf("%-20s %-16s %s", r.Name, r.IP, state))
	}
	if len(lines) == 0 {
		lines = []string{"No other devices of yours are online on the tailnet."}
	}
	return protocol.Result{Text: strings.Join(lines, "\n"), Data: rows}
}

func writeJSON(w http.ResponseWriter, v any) {
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(v)
}
