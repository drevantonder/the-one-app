package mesh

import (
	"bytes"
	"context"
	_ "embed"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"net"
	"net/http"
	"runtime"
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
	known   []tsPeer          // this user's other machines, online or not
	nodes   map[string]string // tailnet name → version, of machines seen running a node

	// Version is this node's version, which peers show.
	Version string

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
	views    []protocol.Resource
	opsAt    time.Time
	version  string
}

const (
	nodesKey    = "mesh_nodes"
	devicesView = "ui://one/devices"
)

//go:embed devices.html
var devicesHTML string

func New(node *core.Node, st *store.Store, port string, log *slog.Logger) *Mesh {
	m := &Mesh{node: node, store: st, port: port, log: log, who: whois{cache: map[string]whoisEntry{}}, peers: map[string]*peer{}, devices: map[string]string{}, nodes: map[string]string{}}
	if raw, _ := st.Meta(nodesKey); raw != "" {
		_ = json.Unmarshal([]byte(raw), &m.nodes)
	}
	node.AddView(protocol.Resource{URI: devicesView, Name: "Devices", MIMEType: protocol.MIMEApp, Text: devicesHTML})
	node.AddCoreOp("devices", "List this user's devices that run a node, this one first: name, OS, version, and whether each is online.", core.SchemaFor[struct{}](), true,
		func(context.Context, json.RawMessage, protocol.Caller) (protocol.Result, error) {
			return m.devicesResult(), nil
		})
	node.SetUI("devices", &protocol.OperationUI{ResourceURI: devicesView})
	node.AddMenubarItem(protocol.MenubarItem{ID: "one.devices", Title: "Devices", Tool: "devices", Resource: devicesView})
	return m
}

type pullRequest struct {
	Vector map[string]int64 `json:"vector"`
	WaitMs int              `json:"waitMs"`
}

type pullResponse struct {
	Device  string           `json:"device"`
	Version string           `json:"version,omitempty"`
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
			writeJSON(w, pullResponse{Device: m.store.Device(), Version: m.Version, Entries: entries})
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
// Tailscale isn't running yet, as at boot, the node stays local and joins
// once it is.
func (m *Mesh) Start(ctx context.Context, h http.Handler) error {
	st, err := status(ctx)
	if err == nil {
		return m.join(ctx, h, st)
	}
	m.log.Warn("tailnet unavailable; this node stays local until it is", "err", err)
	go func() {
		t := time.NewTicker(10 * time.Second)
		defer t.Stop()
		for {
			select {
			case <-ctx.Done():
				return
			case <-t.C:
			}
			if st, err := status(ctx); err == nil {
				if err := m.join(ctx, h, st); err != nil {
					m.log.Error("joining the tailnet", "err", err)
				}
				return
			}
		}
	}()
	return nil
}

func (m *Mesh) join(ctx context.Context, h http.Handler, st tsStatus) error {
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
	m.node.AddCoreOp("peers", "List this user's other devices on the tailnet, online or not, and whether their nodes are syncing.", core.SchemaFor[struct{}](), true,
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
	var known []tsPeer
	for _, p := range st.Peer {
		if p.UserID != st.Self.UserID || p.ipv4() == "" {
			continue
		}
		known = append(known, p)
		if p.Online {
			want[p.ipv4()] = p
		}
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	m.known = known
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
	views, _ := m.peerViews(ctx, p)
	m.mu.Lock()
	p.ops, p.views, p.opsAt = ops, views, time.Now()
	m.mu.Unlock()
	m.publishRemote()
}

func (m *Mesh) peerViews(ctx context.Context, p *peer) ([]protocol.Resource, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, m.base(p)+"/v1/resources?local=1", nil)
	if err != nil {
		return nil, err
	}
	resp, err := client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	var views []protocol.Resource
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("%s", resp.Status)
	}
	return views, json.NewDecoder(resp.Body).Decode(&views)
}

// publishRemote gives the node every reachable peer's operations, in a
// stable order.
func (m *Mesh) publishRemote() {
	m.mu.Lock()
	var all []core.RemoteOp
	var views []protocol.Resource
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
		views = append(views, p.views...)
	}
	m.mu.Unlock()
	m.node.SetRemoteOps(all)
	m.node.SetRemoteResources(views)
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
		p.device, p.version = out.Device, out.Version
		if v, ok := m.nodes[p.name]; !ok || v != out.Version {
			m.nodes[p.name] = out.Version
			b, _ := json.Marshal(m.nodes)
			if err := m.store.SetMeta(nodesKey, string(b)); err != nil {
				m.log.Warn("saving the devices seen", "err", err)
			}
		}
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
		OS       string `json:"os,omitempty"`
		Online   bool   `json:"online"`
		Syncing  bool   `json:"syncing"`
		Version  string `json:"version,omitempty"`
		LastSeen string `json:"lastSeen,omitempty"`
		LastSync string `json:"lastSync,omitempty"`
		Error    string `json:"error,omitempty"`
	}
	m.mu.Lock()
	var rows []row
	for _, tp := range m.known {
		r := row{Name: tp.Name(), IP: tp.ipv4(), OS: tp.OS, Online: tp.Online}
		if !tp.Online && !tp.LastSeen.IsZero() {
			r.LastSeen = tp.LastSeen.Format(time.RFC3339)
		}
		if p := m.peers[r.IP]; p != nil {
			r.Syncing, r.Version, r.Error = p.reached, p.version, p.lastErr
			if !p.lastSync.IsZero() {
				r.LastSync = p.lastSync.Format(time.RFC3339)
			}
		}
		rows = append(rows, r)
	}
	m.mu.Unlock()
	sort.Slice(rows, func(i, j int) bool { return rows[i].Name < rows[j].Name })
	var lines []string
	for _, r := range rows {
		state := "offline"
		switch {
		case r.Syncing:
			state = "syncing"
		case r.Online:
			state = "no node reachable"
		}
		lines = append(lines, fmt.Sprintf("%-20s %-16s %s", r.Name, r.IP, state))
	}
	if len(lines) == 0 {
		lines = []string{"You have no other devices on the tailnet."}
	}
	return protocol.Result{Text: strings.Join(lines, "\n"), Data: rows}
}

// devicesResult lists this device, then every other machine of this user's
// that has been seen running a node, online or not.
func (m *Mesh) devicesResult() protocol.Result {
	type row struct {
		Name     string `json:"name"`
		OS       string `json:"os"`
		Self     bool   `json:"self,omitempty"`
		Online   bool   `json:"online"`
		Version  string `json:"version,omitempty"`
		LastSeen string `json:"lastSeen,omitempty"`
	}
	m.mu.Lock()
	self := row{Name: m.node.Name(), OS: m.self.OS, Self: true, Online: true, Version: m.Version}
	if self.OS == "" {
		self.OS = map[string]string{"darwin": "macOS"}[runtime.GOOS]
		if self.OS == "" {
			self.OS = runtime.GOOS
		}
	}
	var rows []row
	for _, tp := range m.known {
		version, isNode := m.nodes[tp.Name()]
		p := m.peers[tp.ipv4()]
		if p != nil && p.reached {
			version, isNode = p.version, true
		}
		if !isNode {
			continue
		}
		r := row{Name: tp.Name(), OS: tp.OS, Online: p != nil && p.reached, Version: version}
		switch {
		case r.Online:
		case !tp.Online && !tp.LastSeen.IsZero():
			r.LastSeen = tp.LastSeen.Format(time.RFC3339)
		case p != nil && !p.lastSync.IsZero():
			r.LastSeen = p.lastSync.Format(time.RFC3339)
		}
		rows = append(rows, r)
	}
	m.mu.Unlock()
	sort.Slice(rows, func(i, j int) bool { return rows[i].Name < rows[j].Name })
	rows = append([]row{self}, rows...)
	lines := make([]string, 0, len(rows))
	for _, r := range rows {
		state := "online"
		switch {
		case r.Self:
			state = "this device"
		case !r.Online && r.LastSeen != "":
			state = "offline, last seen " + r.LastSeen
		case !r.Online:
			state = "offline"
		}
		lines = append(lines, fmt.Sprintf("%-20s %-8s %-12s %s", r.Name, r.OS, r.Version, state))
	}
	return protocol.Result{Text: strings.Join(lines, "\n"), Data: map[string]any{"devices": rows}}
}

func writeJSON(w http.ResponseWriter, v any) {
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(v)
}
