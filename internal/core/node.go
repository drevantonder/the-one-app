// Package core runs a node: it owns the log, starts plugin processes, swaps
// plugin generations on reload, and builds shared state (operations, wake,
// MCP instructions) by running plugin transforms in order from empty.
package core

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"slices"
	"strings"
	"sync"
	"time"

	"github.com/drevantonder/the-one-app/internal/store"
	"github.com/drevantonder/the-one-app/protocol"
	"github.com/drevantonder/the-one-app/rpc"
)

// CoreID is the namespace of the core's own entries and operations.
const CoreID = "one"

type source struct {
	id        string
	builtin   bool
	dir       string          // linked Go module, rebuilt on change
	binary    string          // last successful build of a linked plugin
	published *published      // published from another device, fetched from the bucket
	declared  json.RawMessage // a declaration, run by the declared-plugin host
}

type generation struct {
	id       string
	n        int
	manifest protocol.Manifest
	last     orderKey // the last entry applied, in log order
	conn     *rpc.Conn
	cmd      *exec.Cmd
	stdin    interface{ Close() error }
	inflight sync.WaitGroup
}

type Node struct {
	store    *store.Store
	builtins []string // bundled plugins, run as `one plugin-host <id>`, in order
	dataDir  string
	exe      string
	log      *slog.Logger

	mu       sync.RWMutex
	sources  map[string]*source
	gens     map[string]*generation
	errs     map[string]string
	genCount int
	watchers []func()
	extraOps []coreOp
	coord    Coordinator
	blobs    Blobs
	declHost []string   // command that runs a declared plugin
	remote   []RemoteOp // operations of plugins running on other devices
	name     string     // this device's name; see Name
}

// SetDeclaredHost sets the command that runs declared plugins. The core
// passes the declaration in ONE_DECLARATION and never looks inside it.
func (n *Node) SetDeclaredHost(argv []string) {
	n.mu.Lock()
	n.declHost = argv
	n.mu.Unlock()
}

// New creates a node. builtins are the plugins bundled with the binary; they
// run first, in the order given.
func New(st *store.Store, dataDir string, builtins []string, log *slog.Logger) (*Node, error) {
	exe, err := os.Executable()
	if err != nil {
		return nil, err
	}
	n := &Node{
		store:    st,
		builtins: builtins,
		dataDir:  dataDir,
		exe:      exe,
		log:      log,
		sources:  map[string]*source{},
		gens:     map[string]*generation{},
		errs:     map[string]string{},
	}
	for _, id := range builtins {
		n.sources[id] = &source{id: id, builtin: true}
	}
	if err := n.loadLinks(); err != nil {
		return nil, err
	}
	return n, nil
}

func (n *Node) Device() string { return n.store.Device() }

// Start launches every enabled plugin.
func (n *Node) Start(ctx context.Context) {
	n.refreshPublished(ctx)
	enabled := n.enabledState()
	for _, id := range n.sourceOrder() {
		if enabled[id] {
			if err := n.startPlugin(ctx, id); err != nil {
				n.log.Error("plugin failed to start", "plugin", id, "err", err)
			}
		}
	}
	go n.watchLinks(ctx)
	go n.runSchedules(ctx)
}

// Stop shuts down every plugin.
func (n *Node) Stop() {
	n.mu.Lock()
	gens := n.gens
	n.gens = map[string]*generation{}
	n.mu.Unlock()
	for _, g := range gens {
		n.retire(g, 5*time.Second)
	}
}

// OnChange registers a callback for when operations or instructions change.
func (n *Node) OnChange(fn func()) {
	n.mu.Lock()
	n.watchers = append(n.watchers, fn)
	n.mu.Unlock()
}

func (n *Node) changed() {
	n.mu.RLock()
	ws := slices.Clone(n.watchers)
	n.mu.RUnlock()
	for _, fn := range ws {
		fn()
	}
}

// sourceOrder is the deterministic plugin order: built-ins first, then the
// rest by id. Transforms run in this order.
func (n *Node) sourceOrder() []string {
	n.mu.RLock()
	defer n.mu.RUnlock()
	out := slices.Clone(n.builtins)
	var rest []string
	for id := range n.sources {
		if !slices.Contains(n.builtins, id) {
			rest = append(rest, id)
		}
	}
	slices.Sort(rest)
	return append(out, rest...)
}

// running returns live generations in plugin order.
func (n *Node) running() []*generation {
	order := n.sourceOrder()
	n.mu.RLock()
	defer n.mu.RUnlock()
	var out []*generation
	for _, id := range order {
		if g := n.gens[id]; g != nil {
			out = append(out, g)
		}
	}
	return out
}

// acquire returns a plugin's live generation with an in-flight slot held.
func (n *Node) acquire(id string) (*generation, func(), error) {
	n.mu.RLock()
	defer n.mu.RUnlock()
	g := n.gens[id]
	if g == nil {
		return nil, nil, fmt.Errorf("plugin %q is not running", id)
	}
	g.inflight.Add(1)
	return g, g.inflight.Done, nil
}

func (n *Node) command(src *source) []string {
	if src.declared != nil {
		n.mu.RLock()
		defer n.mu.RUnlock()
		return n.declHost
	}
	if src.builtin {
		return []string{n.exe, "plugin-host", src.id}
	}
	n.mu.RLock()
	defer n.mu.RUnlock()
	return []string{src.binary}
}

// startPlugin starts a new generation and swaps it in. The previous
// generation keeps serving calls it already accepted, then stops.
func (n *Node) startPlugin(ctx context.Context, id string) error {
	n.mu.RLock()
	src := n.sources[id]
	n.mu.RUnlock()
	if src == nil {
		return fmt.Errorf("unknown plugin %q", id)
	}
	n.mu.RLock()
	pub := src.published
	n.mu.RUnlock()
	switch {
	case pub != nil:
		path, err := n.fetchPublished(ctx, *pub)
		if err != nil {
			n.setErr(id, err)
			return err
		}
		n.mu.Lock()
		src.binary = path
		n.mu.Unlock()
	case src.declared != nil:
		if len(n.command(src)) == 0 {
			err := errors.New("this node can't run declared plugins")
			n.setErr(id, err)
			return err
		}
	case !src.builtin && n.command(src)[0] == "":
		if err := n.build(src); err != nil {
			n.setErr(id, err)
			return err
		}
	}
	g, err := n.spawn(ctx, src)
	if err != nil {
		n.setErr(id, err)
		return err
	}
	n.mu.Lock()
	old := n.gens[id]
	n.gens[id] = g
	delete(n.errs, id)
	n.mu.Unlock()
	n.log.Info("plugin started", "plugin", id, "version", g.manifest.Version, "generation", g.n)
	// Only now is this the plugin's live generation, so anything it does on
	// ready (such as changing its operations) reaches the right one.
	_ = g.conn.Notify(protocol.MethodReady, struct{}{})
	if old != nil {
		go n.retire(old, 30*time.Second)
	}
	n.changed()
	return nil
}

func (n *Node) stopPlugin(id string) {
	n.mu.Lock()
	g := n.gens[id]
	delete(n.gens, id)
	n.mu.Unlock()
	if g != nil {
		go n.retire(g, 30*time.Second)
		n.changed()
	}
}

func (n *Node) setErr(id string, err error) {
	n.mu.Lock()
	n.errs[id] = err.Error()
	n.mu.Unlock()
}

func (n *Node) spawn(ctx context.Context, src *source) (*generation, error) {
	args := n.command(src)
	cmd := exec.Command(args[0], args[1:]...)
	cmd.Env = append(os.Environ(), "ONE_PLUGIN="+src.id, "ONE_PLUGIN_DIR="+filepath.Join(n.dataDir, "plugins", src.id, "data"))
	if src.declared != nil {
		cmd.Env = append(cmd.Env, "ONE_DECLARATION="+string(src.declared))
	}
	stdin, err := cmd.StdinPipe()
	if err != nil {
		return nil, err
	}
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return nil, err
	}
	cmd.Stderr = &logWriter{log: n.log, plugin: src.id}
	if err := cmd.Start(); err != nil {
		return nil, err
	}
	n.mu.Lock()
	n.genCount++
	g := &generation{id: src.id, n: n.genCount, cmd: cmd, stdin: stdin}
	n.mu.Unlock()
	g.conn = rpc.New(stdout, stdin, n.pluginHandler(src.id), false)
	go func() { _ = g.conn.Run(context.Background()) }()

	fail := func(err error) (*generation, error) {
		_ = stdin.Close()
		_ = cmd.Process.Kill()
		_ = cmd.Wait()
		return nil, err
	}
	sctx, cancel := context.WithTimeout(ctx, 30*time.Second)
	defer cancel()
	if err := g.conn.Call(sctx, protocol.MethodDescribe, struct{}{}, &g.manifest); err != nil {
		return fail(fmt.Errorf("describe: %w", err))
	}
	if g.manifest.Protocol != protocol.Version {
		return fail(fmt.Errorf("plugin speaks protocol %d, core speaks %d", g.manifest.Protocol, protocol.Version))
	}
	if g.manifest.ID != src.id {
		return fail(fmt.Errorf("plugin says its id is %q, expected %q", g.manifest.ID, src.id))
	}
	if p := g.manifest.Platforms; len(p) > 0 && !slices.Contains(p, runtime.GOOS) {
		return fail(fmt.Errorf("%w: runs on %s", ErrPlatform, strings.Join(p, ", ")))
	}
	if err := g.conn.Call(sctx, protocol.MethodReset, struct{}{}, nil); err != nil {
		return fail(fmt.Errorf("reset: %w", err))
	}
	entries, err := n.store.Entries(src.id)
	if err != nil {
		return fail(err)
	}
	if len(entries) > 0 {
		g.last = keyOf(entries[len(entries)-1])
	}
	for start := 0; start < len(entries); start += 500 {
		batch := entries[start:min(start+500, len(entries))]
		if err := g.conn.Call(sctx, protocol.MethodApply, protocol.ApplyParams{Entries: batch}, nil); err != nil {
			return fail(fmt.Errorf("replay: %w", err))
		}
	}
	return g, nil
}

func (n *Node) retire(g *generation, grace time.Duration) {
	done := make(chan struct{})
	go func() { g.inflight.Wait(); close(done) }()
	select {
	case <-done:
	case <-time.After(grace):
	}
	_ = g.stdin.Close()
	exited := make(chan struct{})
	go func() { _ = g.cmd.Wait(); close(exited) }()
	select {
	case <-exited:
	case <-time.After(3 * time.Second):
		_ = g.cmd.Process.Kill()
		<-exited
	}
}

func (n *Node) pluginHandler(id string) rpc.Handler {
	return func(ctx context.Context, method string, params json.RawMessage) (any, error) {
		switch method {
		case protocol.MethodAppend:
			var in protocol.AppendParams
			if err := json.Unmarshal(params, &in); err != nil {
				return nil, err
			}
			if in.Type == "" {
				return nil, errors.New("append: missing type")
			}
			if err := n.checkConsistent(ctx, id, in.Type); err != nil {
				return nil, err
			}
			e, err := n.store.Append(id, in.Type, in.Payload, in.Actor)
			if err == nil && n.coordinator() != nil {
				n.mu.RLock()
				consistent := n.gens[id] != nil && modeOf(n.gens[id].manifest, in.Type) == protocol.Consistent
				n.mu.RUnlock()
				if consistent {
					// Durable before acknowledged, so a device taking over the
					// lease sees this entry when it catches up from the bucket.
					if ferr := n.coordinator().Flush(ctx); ferr != nil {
						n.log.Warn("consistent entry not yet in the bucket", "plugin", id, "err", ferr)
					}
				}
			}
			if err == nil {
				n.mu.Lock()
				if g := n.gens[id]; g != nil {
					g.last = keyOf(e)
				}
				n.mu.Unlock()
			}
			return e, err
		case protocol.MethodChanged:
			go n.redescribe(id)
			return nil, nil
		case protocol.MethodInvoke:
			var in protocol.InvokeParams
			if err := json.Unmarshal(params, &in); err != nil {
				return nil, err
			}
			return n.Call(ctx, in.Plugin, in.Operation, in.Input, protocol.Caller{Actor: "plugin " + id})
		case protocol.MethodFiles:
			var in protocol.FilesParams
			if err := json.Unmarshal(params, &in); err != nil {
				return nil, err
			}
			return n.files(ctx, id, in)
		case protocol.MethodLog:
			var in protocol.LogParams
			_ = json.Unmarshal(params, &in)
			n.log.Info(in.Message, "plugin", id)
			return struct{}{}, nil
		}
		return nil, &rpc.Error{Code: -32601, Message: "unknown method " + method}
	}
}

// ErrPlatform means a plugin doesn't run on this device's platform.
var ErrPlatform = errors.New("not available on this platform")

// redescribe refreshes a running plugin's manifest after it says its
// operations changed.
func (n *Node) redescribe(id string) {
	g, release, err := n.acquire(id)
	if err != nil {
		return
	}
	defer release()
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	var m protocol.Manifest
	if err := g.conn.Call(ctx, protocol.MethodDescribe, struct{}{}, &m); err != nil {
		n.log.Warn("redescribe failed", "plugin", id, "err", err)
		return
	}
	n.mu.Lock()
	g.manifest = m
	n.mu.Unlock()
	n.log.Info("plugin operations changed", "plugin", id, "operations", len(m.Operations))
	n.changed()
}

// Hook passes an HTTP request to a plugin's hook.
func (n *Node) Hook(ctx context.Context, plugin string, req protocol.HookRequest) (protocol.HookResponse, error) {
	g, release, err := n.acquire(plugin)
	if err != nil {
		return protocol.HookResponse{}, err
	}
	defer release()
	var res protocol.HookResponse
	err = g.conn.Call(ctx, protocol.MethodHook, req, &res)
	return res, err
}

// Call runs a plugin operation.
func (n *Node) Call(ctx context.Context, plugin, op string, input json.RawMessage, caller protocol.Caller) (protocol.Result, error) {
	if plugin == CoreID {
		return n.callCore(ctx, op, input, caller)
	}
	input, device, err := n.takeDevice(plugin, op, input)
	if err != nil {
		return protocol.Result{}, err
	}
	if device != "" && !n.isSelf(device) {
		id, err := n.remoteOn(plugin, op, device)
		if err != nil {
			return protocol.Result{}, err
		}
		return n.forward(ctx, id, plugin, op, input, caller)
	}
	g, release, err := n.acquire(plugin)
	if err != nil {
		// Not running here: run it on a device that has it, unless asked
		// for this one.
		if device != "" {
			return protocol.Result{}, n.notOn(plugin, op, device)
		}
		if id, ok := n.remoteFor(plugin, op); ok {
			return n.forward(ctx, id, plugin, op, input, caller)
		}
		return protocol.Result{}, err
	}
	if !slices.ContainsFunc(g.manifest.Operations, func(o protocol.Operation) bool { return o.Name == op }) {
		release()
		// The plugin runs here without this operation, which another
		// device's has (operations can depend on the device).
		if device == "" {
			if id, ok := n.remoteFor(plugin, op); ok {
				return n.forward(ctx, id, plugin, op, input, caller)
			}
		}
		if device != "" {
			return protocol.Result{}, n.notOn(plugin, op, device)
		}
		return protocol.Result{}, fmt.Errorf("plugin %q has no operation %q", plugin, op)
	}
	defer release()
	if caller.Device == "" {
		caller.Device = n.store.Device()
	}
	var res protocol.Result
	err = g.conn.Call(ctx, protocol.MethodCall, protocol.CallParams{Operation: op, Input: input, Caller: caller}, &res)
	if holder, ok := notHolder(err); ok {
		// Consistent data has one writer: run the operation on its device.
		return n.forward(ctx, holder, plugin, op, input, caller)
	}
	return res, err
}

// forward runs an operation on another device, by its node's device id.
func (n *Node) forward(ctx context.Context, device, plugin, op string, input json.RawMessage, caller protocol.Caller) (protocol.Result, error) {
	c := n.coordinator()
	if c == nil {
		return protocol.Result{}, fmt.Errorf("device %s can't be reached without the mesh", device)
	}
	return c.Forward(ctx, device, plugin, op, input, caller)
}

// transform threads doc through every running plugin that registered name.
func (n *Node) transform(ctx context.Context, name string, doc any, args any) (json.RawMessage, error) {
	raw, err := json.Marshal(doc)
	if err != nil {
		return nil, err
	}
	argRaw, _ := json.Marshal(args)
	for _, r := range n.running() {
		g, release, err := n.acquire(r.id)
		if err != nil {
			continue // stopped meanwhile
		}
		if !slices.Contains(g.manifest.Transforms, name) {
			release()
			continue
		}
		var next json.RawMessage
		err = g.conn.Call(ctx, protocol.MethodTransform, protocol.TransformParams{Name: name, Doc: raw, Args: argRaw}, &next)
		release()
		if err != nil {
			return nil, fmt.Errorf("%s %s transform: %w", g.id, name, err)
		}
		raw = next
	}
	return raw, nil
}

const baseInstructions = `TheOneApp is this user's personal hub, shared across their devices. Its tools come from plugins, and every tool is also a CLI command (` + "`one <plugin> <operation>`" + `) and an HTTP API.`

// Instructions builds the MCP server instructions: the base text, then each
// plugin's transform in order.
func (n *Node) Instructions(ctx context.Context) string {
	base := baseInstructions
	raw, err := n.transform(ctx, protocol.TransformInstructions, protocol.InstructionsDoc{Text: base}, nil)
	if err != nil {
		n.log.Error("building MCP instructions", "err", err)
		return base
	}
	var doc protocol.InstructionsDoc
	if err := json.Unmarshal(raw, &doc); err != nil {
		return base
	}
	return doc.Text
}

type logWriter struct {
	log    *slog.Logger
	plugin string
}

func (w *logWriter) Write(p []byte) (int, error) {
	for _, line := range strings.Split(strings.TrimRight(string(p), "\n"), "\n") {
		if line != "" {
			w.log.Warn(line, "plugin", w.plugin, "stream", "stderr")
		}
	}
	return len(p), nil
}
