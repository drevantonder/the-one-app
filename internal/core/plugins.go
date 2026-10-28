package core

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"sort"
	"strings"
	"time"

	"github.com/google/jsonschema-go/jsonschema"

	"github.com/drevantonder/the-one-app/protocol"
	"github.com/drevantonder/the-one-app/rpc"
)

// Op is one operation as the surfaces (API, CLI, MCP) see it.
type Op struct {
	Plugin      string          `json:"plugin"`
	Name        string          `json:"name"`
	Description string          `json:"description"`
	InputSchema json.RawMessage `json:"inputSchema"`
	ReadOnly    bool            `json:"readOnly,omitempty"`
}

// ToolName is the operation's MCP tool name.
func (o Op) ToolName() string {
	if o.Plugin == CoreID {
		return o.Name
	}
	return o.Plugin + "_" + o.Name
}

type (
	noIn     struct{}
	pluginIn struct {
		ID string `json:"id" jsonschema:"the plugin id, as listed by plugins_list"`
	}
	addIn struct {
		ID   string          `json:"id" jsonschema:"the new plugin's id, e.g. gmail"`
		Spec json.RawMessage `json:"spec" jsonschema:"the declaration, e.g. {\"mcp\":{\"url\":...},\"oauth\":{...}}"`
	}
	linkIn struct {
		Dir string `json:"dir" jsonschema:"absolute path of the plugin's Go module on this device"`
	}
)

type coreOp struct {
	Op
	fn func(ctx context.Context, input json.RawMessage, caller protocol.Caller) (protocol.Result, error)
}

func schemaFor[T any]() json.RawMessage {
	s, err := jsonschema.For[T](nil)
	if err != nil {
		panic(err)
	}
	b, _ := json.Marshal(s)
	return b
}

func decode[T any](raw json.RawMessage) (T, error) {
	var v T
	if len(raw) > 0 && string(raw) != "null" {
		if err := json.Unmarshal(raw, &v); err != nil {
			return v, fmt.Errorf("bad input: %w", err)
		}
	}
	return v, nil
}

func (n *Node) coreOps() []coreOp {
	n.mu.RLock()
	extra := slices.Clone(n.extraOps)
	n.mu.RUnlock()
	return append([]coreOp{
		{Op{CoreID, "plugins_list", "List plugins, whether each is enabled and running, and any error.", schemaFor[noIn](), true},
			func(context.Context, json.RawMessage, protocol.Caller) (protocol.Result, error) {
				return n.listPlugins(), nil
			}},
		{Op{CoreID, "plugins_enable", "Enable a plugin on every device.", schemaFor[pluginIn](), false},
			func(ctx context.Context, raw json.RawMessage, c protocol.Caller) (protocol.Result, error) {
				in, err := decode[pluginIn](raw)
				if err != nil {
					return protocol.Result{}, err
				}
				return n.setEnabled(ctx, in.ID, true, c.Actor)
			}},
		{Op{CoreID, "plugins_disable", "Disable a plugin on every device. Its data stays in the log, so enabling it again restores everything.", schemaFor[pluginIn](), false},
			func(ctx context.Context, raw json.RawMessage, c protocol.Caller) (protocol.Result, error) {
				in, err := decode[pluginIn](raw)
				if err != nil {
					return protocol.Result{}, err
				}
				return n.setEnabled(ctx, in.ID, false, c.Actor)
			}},
		{Op{CoreID, "plugins_reload", "Restart a plugin as a new generation, rebuilding it first if it is linked from source.", schemaFor[pluginIn](), false},
			func(ctx context.Context, raw json.RawMessage, _ protocol.Caller) (protocol.Result, error) {
				in, err := decode[pluginIn](raw)
				if err != nil {
					return protocol.Result{}, err
				}
				return n.reload(ctx, in.ID)
			}},
		{Op{CoreID, "plugins_publish", "Build a linked plugin for every platform and install it on all of this user's devices.", schemaFor[pluginIn](), false},
			func(ctx context.Context, raw json.RawMessage, c protocol.Caller) (protocol.Result, error) {
				in, err := decode[pluginIn](raw)
				if err != nil {
					return protocol.Result{}, err
				}
				return n.publish(ctx, in.ID, c.Actor)
			}},
		{Op{CoreID, "plugins_add", "Add a plugin from a declaration, with no code: e.g. a remote MCP server and its sign-in. It is added on every device.", schemaFor[addIn](), false},
			func(ctx context.Context, raw json.RawMessage, c protocol.Caller) (protocol.Result, error) {
				in, err := decode[addIn](raw)
				if err != nil {
					return protocol.Result{}, err
				}
				return n.add(ctx, in, c.Actor)
			}},
		{Op{CoreID, "plugins_remove", "Remove an added or published plugin from every device. Its log entries are kept.", schemaFor[pluginIn](), false},
			func(ctx context.Context, raw json.RawMessage, c protocol.Caller) (protocol.Result, error) {
				in, err := decode[pluginIn](raw)
				if err != nil {
					return protocol.Result{}, err
				}
				n.mu.RLock()
				src := n.sources[in.ID]
				n.mu.RUnlock()
				if src == nil || (src.declared == nil && src.published == nil) {
					return protocol.Result{}, fmt.Errorf("%q is not an added or published plugin", in.ID)
				}
				payload, _ := json.Marshal(map[string]string{"id": in.ID})
				if _, err := n.store.Append(CoreID, "plugin.removed", payload, c.Actor); err != nil {
					return protocol.Result{}, err
				}
				n.reconcile(ctx)
				return protocol.Result{Text: "Removed " + in.ID + "."}, nil
			}},
		{Op{CoreID, "plugins_link", "Load a plugin from a Go module on this device. It is rebuilt and hot-reloaded whenever its files change.", schemaFor[linkIn](), false},
			func(ctx context.Context, raw json.RawMessage, c protocol.Caller) (protocol.Result, error) {
				in, err := decode[linkIn](raw)
				if err != nil {
					return protocol.Result{}, err
				}
				return n.link(ctx, in.Dir, c.Actor)
			}},
	}, extra...)
}

// AddCoreOp adds an operation to the core namespace, for parts of the node
// outside this package (the mesh, the bucket).
func (n *Node) AddCoreOp(name, description string, input json.RawMessage, readOnly bool, fn func(ctx context.Context, input json.RawMessage, caller protocol.Caller) (protocol.Result, error)) {
	n.mu.Lock()
	n.extraOps = append(n.extraOps, coreOp{Op{CoreID, name, description, input, readOnly}, fn})
	n.mu.Unlock()
	n.changed()
}

// SchemaFor derives an input schema from a Go type.
func SchemaFor[T any]() json.RawMessage { return schemaFor[T]() }

// Operations lists every operation: core first, then every device's
// plugins, each with the devices it is available on.
func (n *Node) Operations() []Op {
	var out []Op
	for _, c := range n.coreOps() {
		out = append(out, c.Op)
	}
	ops, devices := n.availability()
	for _, o := range ops {
		out = append(out, withDevices(o, devices[o.Plugin+"/"+o.Name]))
	}
	return out
}

// LocalOperations lists the operations of plugins running on this device.
// This is what a node offers its peers.
func (n *Node) LocalOperations() []Op {
	var out []Op
	for _, g := range n.running() {
		for _, o := range g.manifest.Operations {
			out = append(out, Op{Plugin: g.id, Name: o.Name, Description: o.Description, InputSchema: o.InputSchema, ReadOnly: o.ReadOnly})
		}
	}
	return out
}

// RemoteOp is an operation of a plugin running on another device.
type RemoteOp struct {
	Op
	Device     string `json:"device"`     // its node's device id
	DeviceName string `json:"deviceName"` // its tailnet name
}

// SetRemoteOps replaces what other devices offer.
func (n *Node) SetRemoteOps(ops []RemoteOp) {
	n.mu.Lock()
	same := len(ops) == len(n.remote)
	for i := 0; same && i < len(ops); i++ {
		same = ops[i].Plugin == n.remote[i].Plugin && ops[i].Name == n.remote[i].Name && ops[i].Device == n.remote[i].Device && ops[i].DeviceName == n.remote[i].DeviceName
	}
	n.remote = ops
	n.mu.Unlock()
	if !same {
		n.changed()
	}
}

func (n *Node) remoteOps() []RemoteOp {
	n.mu.RLock()
	defer n.mu.RUnlock()
	return slices.Clone(n.remote)
}

// remoteFor finds a device running the plugin's operation.
func (n *Node) remoteFor(plugin, op string) (string, bool) {
	for _, r := range n.remoteOps() {
		if r.Plugin == plugin && r.Name == op {
			return r.Device, true
		}
	}
	return "", false
}

func (n *Node) callCore(ctx context.Context, op string, input json.RawMessage, caller protocol.Caller) (protocol.Result, error) {
	for _, c := range n.coreOps() {
		if c.Name == op {
			return c.fn(ctx, input, caller)
		}
	}
	return protocol.Result{}, fmt.Errorf("no core operation %q", op)
}

type pluginState struct {
	ID      string `json:"id"`
	Enabled bool   `json:"enabled"`
}

// enabledState folds the core's plugin.state entries; the latest wins.
// Built-ins are enabled until disabled.
func (n *Node) enabledState() map[string]bool {
	out := map[string]bool{}
	for _, id := range n.builtins {
		out[id] = true
	}
	entries, err := n.store.Entries(CoreID)
	if err != nil {
		n.log.Error("read plugin state", "err", err)
		return out
	}
	for _, e := range entries {
		if e.Type != "plugin.state" {
			continue
		}
		var s pluginState
		if json.Unmarshal(e.Payload, &s) == nil {
			out[s.ID] = s.Enabled
		}
	}
	return out
}

func (n *Node) setEnabled(ctx context.Context, id string, enabled bool, actor string) (protocol.Result, error) {
	n.mu.RLock()
	_, known := n.sources[id]
	n.mu.RUnlock()
	if !known {
		return protocol.Result{}, fmt.Errorf("unknown plugin %q", id)
	}
	payload, _ := json.Marshal(pluginState{ID: id, Enabled: enabled})
	if _, err := n.store.Append(CoreID, "plugin.state", payload, actor); err != nil {
		return protocol.Result{}, err
	}
	if !enabled {
		n.stopPlugin(id)
		return protocol.Result{Text: fmt.Sprintf("Disabled %s. Its data is kept; enable it to bring everything back.", id)}, nil
	}
	if err := n.startPlugin(ctx, id); err != nil {
		return protocol.Result{}, err
	}
	return protocol.Result{Text: "Enabled " + id + "."}, nil
}

func (n *Node) reload(ctx context.Context, id string) (protocol.Result, error) {
	n.mu.RLock()
	src := n.sources[id]
	n.mu.RUnlock()
	if src == nil {
		return protocol.Result{}, fmt.Errorf("unknown plugin %q", id)
	}
	if !n.enabledState()[id] {
		return protocol.Result{}, fmt.Errorf("plugin %q is disabled", id)
	}
	if !src.builtin {
		if err := n.build(src); err != nil {
			n.setErr(id, err)
			return protocol.Result{}, err
		}
	}
	if err := n.startPlugin(ctx, id); err != nil {
		return protocol.Result{}, err
	}
	return protocol.Result{Text: "Reloaded " + id + "."}, nil
}

func (n *Node) listPlugins() protocol.Result {
	enabled := n.enabledState()
	type row struct {
		ID         string `json:"id"`
		Kind       string `json:"kind"`
		Enabled    bool   `json:"enabled"`
		Running    bool   `json:"running"`
		Version    string `json:"version,omitempty"`
		Generation int    `json:"generation,omitempty"`
		Operations int    `json:"operations,omitempty"`
		Dir        string `json:"dir,omitempty"`
		Error      string `json:"error,omitempty"`
	}
	var rows []row
	var lines []string
	for _, id := range n.sourceOrder() {
		n.mu.RLock()
		src, g, errText := n.sources[id], n.gens[id], n.errs[id]
		n.mu.RUnlock()
		r := row{ID: id, Kind: "builtin", Enabled: enabled[id], Running: g != nil, Error: errText}
		switch {
		case src.declared != nil:
			r.Kind = "added"
		case src.published != nil:
			r.Kind = "published"
		case !src.builtin:
			r.Kind, r.Dir = "linked", src.dir
		}
		if g != nil {
			r.Version, r.Generation, r.Operations = g.manifest.Version, g.n, len(g.manifest.Operations)
		}
		rows = append(rows, r)
		state := "disabled"
		switch {
		case g != nil:
			state = fmt.Sprintf("running v%s, generation %d, %d operations", r.Version, r.Generation, r.Operations)
		case enabled[id]:
			state = "enabled, not running"
		}
		line := fmt.Sprintf("%-14s %-8s %s", id, r.Kind, state)
		if errText != "" {
			line += "\n  error: " + errText
		}
		lines = append(lines, line)
	}
	return protocol.Result{Text: strings.Join(lines, "\n"), Data: rows}
}

// ---- linked plugins: built from a Go module on this device -----------------

const linksKey = "linked_plugins"

func (n *Node) loadLinks() error {
	raw, err := n.store.Meta(linksKey)
	if err != nil || raw == "" {
		return err
	}
	links := map[string]string{}
	if err := json.Unmarshal([]byte(raw), &links); err != nil {
		return err
	}
	for id, dir := range links {
		if !slices.Contains(n.builtins, id) {
			n.sources[id] = &source{id: id, dir: dir}
		}
	}
	return nil
}

func (n *Node) saveLinks() error {
	links := map[string]string{}
	n.mu.RLock()
	for id, s := range n.sources {
		if !s.builtin {
			links[id] = s.dir
		}
	}
	n.mu.RUnlock()
	b, _ := json.Marshal(links)
	return n.store.SetMeta(linksKey, string(b))
}

func (n *Node) link(ctx context.Context, dir, actor string) (protocol.Result, error) {
	dir, err := filepath.Abs(dir)
	if err != nil {
		return protocol.Result{}, err
	}
	if info, err := os.Stat(dir); err != nil || !info.IsDir() {
		return protocol.Result{}, fmt.Errorf("%s is not a directory", dir)
	}
	probe := &source{dir: dir, id: "link-" + shortHash(dir)}
	if err := n.build(probe); err != nil {
		return protocol.Result{}, err
	}
	m, err := describeBinary(ctx, probe.binary)
	if err != nil {
		return protocol.Result{}, err
	}
	if m.ID == CoreID || slices.Contains(n.builtins, m.ID) {
		return protocol.Result{}, fmt.Errorf("plugin id %q is reserved", m.ID)
	}
	n.mu.Lock()
	n.sources[m.ID] = &source{id: m.ID, dir: dir, binary: probe.binary}
	n.mu.Unlock()
	if err := n.saveLinks(); err != nil {
		return protocol.Result{}, err
	}
	res, err := n.setEnabled(ctx, m.ID, true, actor)
	if err != nil {
		return protocol.Result{}, err
	}
	res.Text = fmt.Sprintf("Linked %s from %s. %s It reloads whenever its files change.", m.ID, dir, res.Text)
	return res, nil
}

// build compiles a linked plugin into the data directory.
func (n *Node) build(src *source) error {
	out := filepath.Join(n.dataDir, "plugins", src.id, fmt.Sprintf("%d", time.Now().UnixNano()), "plugin")
	if err := os.MkdirAll(filepath.Dir(out), 0o755); err != nil {
		return err
	}
	cmd := exec.Command("go", "build", "-o", out, ".")
	cmd.Dir = src.dir
	if b, err := cmd.CombinedOutput(); err != nil {
		return fmt.Errorf("build failed:\n%s", strings.TrimSpace(string(b)))
	}
	n.mu.Lock()
	prev := src.binary
	src.binary = out
	n.mu.Unlock()
	if prev != "" {
		_ = os.RemoveAll(filepath.Dir(prev)) // a running process keeps its open file
	}
	return nil
}

func describeBinary(ctx context.Context, binary string) (protocol.Manifest, error) {
	var m protocol.Manifest
	cmd := exec.Command(binary)
	stdin, _ := cmd.StdinPipe()
	stdout, _ := cmd.StdoutPipe()
	if err := cmd.Start(); err != nil {
		return m, err
	}
	defer func() { _ = stdin.Close(); _ = cmd.Process.Kill(); _ = cmd.Wait() }()
	conn := rpc.New(stdout, stdin, func(context.Context, string, json.RawMessage) (any, error) { return struct{}{}, nil }, false)
	go func() { _ = conn.Run(context.Background()) }()
	cctx, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()
	if err := conn.Call(cctx, protocol.MethodDescribe, struct{}{}, &m); err != nil {
		return m, fmt.Errorf("describe: %w", err)
	}
	if m.ID == "" {
		return m, fmt.Errorf("plugin has no id")
	}
	return m, nil
}

// watchLinks rebuilds and hot-reloads linked plugins when their files change.
func (n *Node) watchLinks(ctx context.Context) {
	seen := map[string]string{}
	t := time.NewTicker(time.Second)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-t.C:
		}
		n.mu.RLock()
		var linked []*source
		for _, s := range n.sources {
			if !s.builtin {
				linked = append(linked, s)
			}
		}
		n.mu.RUnlock()
		enabled := n.enabledState()
		for _, s := range linked {
			fp := fingerprint(s.dir)
			prev, ok := seen[s.id]
			seen[s.id] = fp
			if !ok || prev == fp || !enabled[s.id] {
				continue
			}
			n.log.Info("plugin source changed, reloading", "plugin", s.id)
			if _, err := n.reload(ctx, s.id); err != nil {
				n.log.Error("hot reload failed; previous generation keeps running", "plugin", s.id, "err", err)
			}
		}
	}
}

func fingerprint(dir string) string {
	var parts []string
	_ = filepath.WalkDir(dir, func(p string, d fs.DirEntry, err error) error {
		if err != nil {
			return nil
		}
		if d.IsDir() && strings.HasPrefix(d.Name(), ".") && p != dir {
			return filepath.SkipDir
		}
		name := d.Name()
		if d.IsDir() || !(strings.HasSuffix(name, ".go") || name == "go.mod" || name == "go.sum") {
			return nil
		}
		if info, err := d.Info(); err == nil {
			parts = append(parts, fmt.Sprintf("%s:%d:%d", p, info.Size(), info.ModTime().UnixNano()))
		}
		return nil
	})
	sort.Strings(parts)
	return shortHash(strings.Join(parts, "\n"))
}

func shortHash(s string) string {
	h := sha256.Sum256([]byte(s))
	return hex.EncodeToString(h[:6])
}

func (n *Node) add(ctx context.Context, in addIn, actor string) (protocol.Result, error) {
	if in.ID == "" || in.ID == CoreID || slices.Contains(n.builtins, in.ID) {
		return protocol.Result{}, fmt.Errorf("plugin id %q is reserved or empty", in.ID)
	}
	if !json.Valid(in.Spec) || len(in.Spec) == 0 {
		return protocol.Result{}, fmt.Errorf("spec must be a JSON object")
	}
	n.mu.RLock()
	src := n.sources[in.ID]
	n.mu.RUnlock()
	if src != nil && src.declared == nil {
		return protocol.Result{}, fmt.Errorf("a plugin called %q already exists", in.ID)
	}
	payload, _ := json.Marshal(map[string]any{"id": in.ID, "spec": in.Spec})
	if _, err := n.store.Append(CoreID, "plugin.declared", payload, actor); err != nil {
		return protocol.Result{}, err
	}
	state, _ := json.Marshal(pluginState{ID: in.ID, Enabled: true})
	if _, err := n.store.Append(CoreID, "plugin.state", state, actor); err != nil {
		return protocol.Result{}, err
	}
	n.reconcile(ctx)
	return n.listPlugins(), nil
}
