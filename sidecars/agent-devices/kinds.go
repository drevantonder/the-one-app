package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"image"
	"io"
	"log"
	"maps"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"slices"
	"strings"
	"sync"
	"time"
)

// Kinds: what sort of device each one is. A Linux device with a browser (linux) is a container (runtime.go); the
// others are a virtual machine or an emulator, each run by the runtime for its kind that this machine has: Windows in
// QEMU on Linux with KVM (windows.go), Android in the Android SDK's emulator (android.go), an iPhone or iPad in
// Xcode's Simulator on a Mac (iphone.go). A machine runs the kinds whose runtime finds what it needs here. Another
// way to run a kind (Windows in a Windows machine's Hyper-V) is one more runtime in runtimes.
//
// A lasting device is the person's, borrowed by one agent at a time; a copy is a throwaway one made from a lasting
// device for one agent, deleted when it's given back.

const linuxKind = "linux"

// A kind is a runtime for one kind of device.
type kind interface {
	name() string   // as tools take it: windows, android, iphone
	title() string  // as a person reads it
	can() string    // why this machine can't run it, or ""
	limits() string // what input it takes, for tools' descriptions
	input() bool    // whether it takes clicks and typing
	// port is the first port its devices take one of, every step of them; 0 for none.
	port() (first, step int)
	// adopt checks that v, as adopt's input gives it, is something of this kind already here.
	adopt(ctx context.Context, v *vm) error
	// make makes v's files: as a copy of from, or with no from, a new lasting device.
	make(ctx context.Context, v *vm, from *vm) error
	start(ctx context.Context, v *vm) error
	stop(ctx context.Context, v *vm) error
	running(ctx context.Context, v *vm) bool
	remove(ctx context.Context, v *vm) error
	screen(ctx context.Context, v *vm) (image.Image, error)
	act(ctx context.Context, v *vm, a action) error
	open(ctx context.Context, v *vm, url string) error
	install(ctx context.Context, v *vm, app string) (string, error)
	run(ctx context.Context, v *vm, command string) (string, error)
}

// runtimes are every kind but linux, as this machine might run them.
var runtimes = []kind{windowsQEMU{}, androidEmulator{}}

// A vm is a device of a kind other than linux, as its file in the sidecar's folder says.
type vm struct {
	Name    string    `json:"name"`
	Kind    string    `json:"kind"`
	Of      string    `json:"of,omitempty"`      // the lasting device it's a throwaway copy of
	Dir     string    `json:"dir,omitempty"`     // its files
	Port    int       `json:"port,omitempty"`    // windows: its SSH; android: its console
	Display int       `json:"display,omitempty"` // windows: its VNC display
	ID      string    `json:"id,omitempty"`      // android: its AVD; iphone: its simulator
	CPUs    int       `json:"cpus,omitempty"`
	GB      int       `json:"gb,omitempty"` // memory
	Made    time.Time `json:"made"`
}

// An action is input to a device, in its screen's pixels.
type action struct {
	Do   string  `json:"do"` // click, double_click, right_click, type, key, scroll, swipe
	X    float64 `json:"x"`
	Y    float64 `json:"y"`
	ToX  float64 `json:"to_x"`
	ToY  float64 `json:"to_y"`
	Text string  `json:"text"`
	Key  string  `json:"key"`
}

func kindNamed(name string) (kind, error) {
	var names []string
	for _, k := range runtimes {
		if k.name() == name {
			return k, nil
		}
		names = append(names, k.name())
	}
	return nil, fmt.Errorf("no kind %q: there's %s and %s", name, linuxKind, strings.Join(names, ", "))
}

// kindsHere are the kinds this machine runs, each with its title and what input it takes.
func kindsHere() []map[string]any {
	var out []map[string]any
	if rt != nil {
		out = append(out, map[string]any{"kind": linuxKind, "title": "Linux with a browser", "copies": true})
	}
	for _, k := range runtimes {
		if k.can() == "" {
			out = append(out, map[string]any{"kind": k.name(), "title": k.title(), "input": k.limits()})
		}
	}
	return out
}

func (ds *devices) vmDir() string { return filepath.Join(ds.dir, "devices") }

var vmMu sync.Mutex

// vms are this machine's devices of other kinds.
func (ds *devices) vms() []*vm {
	vmMu.Lock()
	defer vmMu.Unlock()
	files, _ := filepath.Glob(filepath.Join(ds.vmDir(), "*.json"))
	var out []*vm
	for _, f := range files {
		b, err := os.ReadFile(f)
		var v vm
		if err == nil && json.Unmarshal(b, &v) == nil {
			out = append(out, &v)
		}
	}
	return out
}

func (ds *devices) vmNamed(name string) *vm {
	for _, v := range ds.vms() {
		if v.Name == name {
			return v
		}
	}
	return nil
}

func (ds *devices) saveVM(v *vm) error {
	vmMu.Lock()
	defer vmMu.Unlock()
	os.MkdirAll(ds.vmDir(), 0o700)
	b, _ := json.MarshalIndent(v, "", "  ")
	return os.WriteFile(filepath.Join(ds.vmDir(), v.Name+".json"), b, 0o600)
}

func (ds *devices) forgetVM(name string) {
	vmMu.Lock()
	defer vmMu.Unlock()
	os.Remove(filepath.Join(ds.vmDir(), name+".json"))
}

// taken reports whether a device here has the name already.
func (ds *devices) taken(ctx context.Context, name string) bool {
	if ds.vmNamed(name) != nil {
		return true
	}
	all, _ := list(ctx)
	_, err := pick(all, name)
	return err == nil
}

func (v *vm) kind() kind {
	k, _ := kindNamed(v.Kind)
	return k
}

func (v *vm) state(ctx context.Context) string {
	if k := v.kind(); k != nil && k.can() == "" && k.running(ctx, v) {
		return "running"
	}
	return "stopped"
}

// makeVM makes a device of kind k here, named name: a copy of the lasting device from, or a new lasting one.
func (ds *devices) makeVM(ctx context.Context, k kind, name string, from *vm) (*vm, error) {
	if why := k.can(); why != "" {
		return nil, errors.New(why)
	}
	if !validName.MatchString(name) {
		return nil, errors.New("a device's name is lowercase letters, digits and dashes")
	}
	creating.Lock()
	defer creating.Unlock()
	if ds.taken(ctx, name) {
		return nil, fmt.Errorf("there's already a device named %s", name)
	}
	v := &vm{Name: name, Kind: k.name(), Made: time.Now().UTC()}
	if from != nil {
		v.Of = from.Name
	}
	if first, step := k.port(); first > 0 {
		v.Port = ds.freePort(first, step)
	}
	if err := k.make(ctx, v, from); err != nil {
		return nil, err
	}
	if err := ds.saveVM(v); err != nil {
		return nil, err
	}
	return v, nil
}

// startVM starts a device if it isn't running, and waits until it's ready.
func startVM(ctx context.Context, v *vm) error {
	k := v.kind()
	if k.running(ctx, v) {
		return nil
	}
	return k.start(ctx, v)
}

// deleteVM deletes a device and its files.
func (ds *devices) deleteVM(ctx context.Context, v *vm) error {
	if err := v.kind().remove(ctx, v); err != nil {
		return err
	}
	ds.forgetVM(v.Name)
	return nil
}

// borrowVM lends the lasting device named, or of the kind, or a throwaway copy of one with copy (or when every
// lasting one is borrowed and this machine has room for a copy).
func (ds *devices) borrowVM(ctx context.Context, k kind, name string, copy bool, l loan) (*vm, error) {
	if why := k.can(); why != "" {
		return nil, errors.New(why)
	}
	ds.take.Lock()
	defer ds.take.Unlock()
	lent := ds.loans()
	var lasting []*vm
	for _, v := range ds.vms() {
		if v.Kind == k.name() && v.Of == "" && (name == "" || v.Name == name) {
			lasting = append(lasting, v)
		}
	}
	if name != "" && len(lasting) == 0 {
		return nil, fmt.Errorf("there's no %s device named %s here", k.title(), name)
	}
	var got *vm
	if !copy {
		for _, v := range lasting {
			if b, ok := lent[v.Name]; !ok {
				got = v
				break
			} else if b.By == l.By {
				got = v // borrowed again by the same agent: its loan carries on
				l.Since = b.Since
				break
			}
		}
	}
	if got == nil {
		if len(lasting) == 0 && k.name() != "android" && k.name() != "iphone" {
			return nil, fmt.Errorf("there's no %s device here to borrow or copy: the person makes one with create", k.title())
		}
		var from *vm
		if len(lasting) > 0 {
			from = lasting[0]
		}
		if !copy && from != nil && !roomFor(gbOf(from)) {
			b := lent[from.Name]
			return nil, fmt.Errorf("%s is borrowed by %s for %s, since %s, and this machine has no room for a copy: ask them, or wait",
				from.Name, b.By, cmpOr(b.For, "something"), b.Since.Local().Format(time.Kitchen))
		}
		if from != nil && !roomFor(gbOf(from)) {
			return nil, fmt.Errorf("this machine has no memory to spare for a copy of %s", from.Name)
		}
		v, err := ds.makeVM(ctx, k, newPoolName(), from)
		if err != nil {
			return nil, err
		}
		got = v
	}
	if l.Since.IsZero() {
		l.Since = time.Now().UTC()
	}
	ds.setLoan(got.Name, &l)
	ds.touch(got.Name)
	if err := startVM(ctx, got); err != nil {
		if got.Of != "" || got.Name != name && strings.HasPrefix(got.Name, poolPrefix) {
			ds.deleteVM(context.WithoutCancel(ctx), got)
		}
		ds.setLoan(got.Name, nil)
		return nil, err
	}
	return got, nil
}

func gbOf(v *vm) int {
	if v != nil && v.GB > 0 {
		return v.GB
	}
	return 4
}

// roomFor reports whether this machine has gb of memory to spare, and 8 GB more for everything else it runs: a
// machine short of memory kills what it must, a device or a build or an agent.
func roomFor(gb int) bool {
	free, _ := memory()
	return free >= int64(gb+8)<<30
}

// giveBackVM ends a loan: a copy, or a device made for the loan, is deleted; a lasting one stays as it is.
func (ds *devices) giveBackVM(ctx context.Context, v *vm, why string) error {
	if v.Of != "" || strings.HasPrefix(v.Name, poolPrefix) {
		if err := ds.deleteVM(ctx, v); err != nil {
			return err
		}
	}
	ds.setLoan(v.Name, nil)
	log.Printf("%s: %s", v.Name, why)
	return nil
}

// A lasting device's loan ends after this long without a call, a look or a connection to it; a copy's after
// idleBorrowed, as a Linux device's.
const idleLasting = time.Hour

// tendVMs gives back loans of devices left idle, and of devices that are gone.
func (ds *devices) tendVMs(ctx context.Context) {
	for name, l := range ds.loans() {
		v := ds.vmNamed(name)
		if v == nil {
			continue
		}
		limit := idleBorrowed
		if v.Of == "" && !strings.HasPrefix(v.Name, poolPrefix) {
			limit = idleLasting
		}
		if ds.idle(name) < limit {
			continue
		}
		if v.Kind == "windows" && connected(ctx, v.Port, 5900+v.Display) {
			ds.touch(name)
			continue
		}
		if err := ds.giveBackVM(ctx, v, fmt.Sprintf("idle for %s (borrowed by %s for %s)", limit, l.By, l.For)); err != nil {
			log.Printf("giving back %s: %v", name, err)
		}
	}
}

// connected reports whether anything holds a connection to one of the ports here.
func connected(ctx context.Context, ports ...int) bool {
	for _, p := range ports {
		if p <= 0 {
			continue
		}
		out, err := exec.CommandContext(ctx, "ss", "-Htn", "state", "established", fmt.Sprintf("( sport = :%d )", p)).Output()
		if err == nil && strings.TrimSpace(string(out)) != "" {
			return true
		}
	}
	return false
}

// listVMs is this machine's devices of other kinds, as list shows them.
func (ds *devices) listVMs(ctx context.Context, me string, lent map[string]loan) []listed {
	var out []listed
	for _, v := range ds.vms() {
		l := listed{Name: v.Name, Server: me, Kind: v.Kind, State: v.state(ctx), Of: v.Of, CPUs: float64(v.CPUs), MemoryLimit: int64(v.GB) << 30}
		if b, ok := lent[v.Name]; ok {
			l.Borrowed = &b
		}
		out = append(out, l)
	}
	return out
}

// vmTools are what Computer Use's computer_use runs on a device of another kind.
func vmTools(k kind) string {
	return strings.Join([]string{
		"screenshot: the screen now, at its own size; x and y in the other tools are its pixels",
		"click {x, y}, double_click {x, y}, right_click {x, y} (on Android, a long press)",
		"type_text {text}: types it where the focus is",
		"press_key {key}: a key, like enter, escape, backspace, tab, home, back (Android), or a combination like ctrl-c",
		"scroll {x, y, to_x, to_y}, swipe {x, y, to_x, to_y}: drags from one point to the other",
		"open_url {url}: opens it in the browser",
		"install_app {app}: a file on this machine or a URL (an .apk on Android, an .msi, .msix or .exe on Windows, an .app on an iPhone), or a winget id on Windows",
		"run_command {command}: runs it and gives its output (PowerShell on Windows, the shell on Android, simctl spawn on an iPhone)",
		"Input here: " + k.limits(),
	}, "\n")
}

// useVM runs one of vmTools on a running device.
func (ds *devices) useVM(ctx context.Context, v *vm, tool string, args map[string]any) (any, error) {
	k := v.kind()
	if !k.running(ctx, v) {
		return nil, fmt.Errorf("%s is stopped: start it first", v.Name)
	}
	ds.touch(v.Name)
	text := func(key string) string { s, _ := args[key].(string); return s }
	f := func(key string) float64 { n, _ := args[key].(float64); return n }
	a := action{X: f("x"), Y: f("y"), ToX: f("to_x"), ToY: f("to_y"), Text: text("text"), Key: text("key")}
	switch tool {
	case "screenshot":
		return ds.screenVM(ctx, v, 0)
	case "click", "double_click", "right_click", "scroll", "swipe":
		a.Do = tool
	case "type_text":
		a.Do = "type"
	case "press_key":
		a.Do = "key"
	case "open_url":
		if text("url") == "" {
			return nil, errors.New("open which url?")
		}
		return "Opened.", k.open(ctx, v, text("url"))
	case "install_app":
		return k.install(ctx, v, cmpOr(text("app"), text("file")))
	case "run_command":
		return k.run(ctx, v, text("command"))
	default:
		return nil, fmt.Errorf("no tool %q on %s: there's\n%s", tool, v.Name, vmTools(k))
	}
	if err := k.act(ctx, v, a); err != nil {
		return nil, err
	}
	return "Done: take a screenshot to see what it did.", nil
}

// screenVM is a JPEG of the device's screen, width wide (0: its own size, at most 1600).
func (ds *devices) screenVM(ctx context.Context, v *vm, width int) (any, error) {
	img, err := v.kind().screen(ctx, v)
	if err != nil {
		return nil, err
	}
	b, w, h := jpegOf(img, width)
	return map[string]any{"content": []map[string]any{
		{"type": "image", "mimeType": "image/jpeg", "data": b64(b)},
		{"type": "text", "text": fmt.Sprintf("%s now, %dx%d; its own screen is %dx%d, the pixels x and y are in.", v.Name, w, h, img.Bounds().Dx(), img.Bounds().Dy())},
	}}, nil
}

// fetch is a file given as a path here or a URL, downloaded to a temporary file the caller removes.
func fetch(ctx context.Context, app string) (path string, temp bool, err error) {
	if !strings.HasPrefix(app, "http://") && !strings.HasPrefix(app, "https://") {
		if _, err := os.Stat(app); err != nil {
			return "", false, fmt.Errorf("no file %s on this machine", app)
		}
		return app, false, nil
	}
	req, _ := http.NewRequestWithContext(ctx, http.MethodGet, app, nil)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return "", false, err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return "", false, fmt.Errorf("downloading %s: %s", app, resp.Status)
	}
	name := filepath.Base(resp.Request.URL.Path)
	dir, err := os.MkdirTemp("", "agent-devices-app")
	if err != nil {
		return "", false, err
	}
	path = filepath.Join(dir, cmpOr(name, "app"))
	f, err := os.Create(path)
	if err != nil {
		return "", false, err
	}
	defer f.Close()
	_, err = io.Copy(f, resp.Body)
	return path, true, err
}

// run runs a command here, its error carrying what it said.
func run(ctx context.Context, name string, args ...string) (string, error) {
	return runIn(ctx, nil, name, args...)
}

// runIn runs a command here with stdin.
func runIn(ctx context.Context, stdin io.Reader, name string, args ...string) (string, error) {
	cmd := exec.CommandContext(ctx, name, args...)
	cmd.Stdin = stdin
	var out bytes.Buffer
	cmd.Stdout, cmd.Stderr = &out, &out
	err := cmd.Run()
	if err != nil {
		return out.String(), fmt.Errorf("%s %s: %v: %s", filepath.Base(name), strings.Join(args[:min(len(args), 2)], " "), err, strings.TrimSpace(out.String()))
	}
	return out.String(), nil
}

// usedPorts are the ports this machine's devices of other kinds use.
func (ds *devices) usedPorts() []int {
	var ports []int
	for _, v := range ds.vms() {
		ports = append(ports, v.Port)
	}
	return ports
}

// freePort is the first port from first, every step, no device here uses and nothing listens on.
func (ds *devices) freePort(first, step int) int {
	used := ds.usedPorts()
	for p := first; ; p += step {
		if !slices.Contains(used, p) && !listening(p) {
			return p
		}
	}
}

// runKinds runs the operations that pick a kind, or act on a device of a kind other than linux; done is false for the
// rest, which the Linux devices' code runs.
func (ds *devices) runKinds(ctx context.Context, op, name string, here bool, in map[string]any) (any, bool, error) {
	text := func(k string) string { s, _ := in[k].(string); return strings.TrimSpace(s) }
	kindName := text("kind")
	if kindName == linuxKind {
		kindName = ""
	}
	var v *vm
	if name != "" {
		v = ds.vmNamed(name)
	}
	switch op {
	case "kinds":
		me, _, _ := ds.servers(ctx)
		free, _ := memory()
		mine := map[string]any{"server": me, "kinds": kindsHere(), "free_gb": free >> 30}
		if here {
			return mine, true, nil
		}
		got, failed := ds.everywhere(ctx, "kinds", nil)
		all := []any{mine}
		for _, s := range slices.Sorted(maps.Keys(got)) {
			var m map[string]any
			json.Unmarshal(got[s], &m)
			all = append(all, m)
		}
		out := map[string]any{"servers": all}
		if len(failed) > 0 {
			out["unreachable"] = failed
		}
		return out, true, nil
	case "adopt":
		r, err := ds.adopt(ctx, name, kindName, in)
		return r, true, err
	case "create", "borrow":
		if v != nil {
			kindName = v.Kind
		}
		if kindName == "" {
			return nil, false, nil
		}
		k, err := kindNamed(kindName)
		if err != nil {
			return nil, true, err
		}
		if why := k.can(); why != "" {
			if here {
				return nil, true, errors.New(why)
			}
			r, err := ds.onAnother(ctx, op, k, in, name)
			return r, true, err
		}
		if op == "create" {
			if name == "" {
				return nil, true, errors.New("name it: lowercase letters, digits and dashes")
			}
			v, err := ds.makeVM(ctx, k, name, nil)
			if err != nil {
				return nil, true, err
			}
			if err := startVM(ctx, v); err != nil {
				return nil, true, fmt.Errorf("made %s, but it didn't start: %w", v.Name, err)
			}
			return fmt.Sprintf("Made %s, a lasting %s device, and started it.", v.Name, k.title()), true, nil
		}
		why := text("for")
		if why == "" {
			return nil, true, errors.New("say what it's for, in a few words: the person sees it")
		}
		copy, _ := in["copy"].(bool)
		got, err := ds.borrowVM(ctx, k, name, copy, loan{By: cmpOr(text("by"), caller(ctx)), For: why})
		if err != nil {
			return nil, true, err
		}
		me, _, _ := ds.servers(ctx)
		what := fmt.Sprintf("%s, the person's lasting %s device: leave it as you found it", got.Name, k.title())
		if got.Of != "" || strings.HasPrefix(got.Name, poolPrefix) {
			what = fmt.Sprintf("%s, a throwaway %s device of your own, deleted with everything in it when you give it back", got.Name, k.title())
			if got.Of != "" {
				what = fmt.Sprintf("%s, a throwaway copy of %s, deleted with everything in it when you give it back", got.Name, got.Of)
			}
		}
		msg := fmt.Sprintf("Borrowed %s. Pass device %q to computer_screen and computer_use (computer_tools says what it takes: %s). Give it back with give_back when done.",
			what, got.Name, k.limits())
		return map[string]any{"device": got.Name, "server": me, "kind": got.Kind, "copy": got.Of != "", "text": msg}, true, nil
	}
	if v == nil {
		if slices.Contains([]string{"open", "install", "run"}, op) {
			r, err := ds.runLinux(ctx, op, name, in)
			return r, true, err
		}
		return nil, false, nil
	}
	k := v.kind()
	if k == nil || k.can() != "" {
		return nil, true, fmt.Errorf("%s is a %s device, which this machine can't run now", v.Name, v.Kind)
	}
	minutes := func(def, most int) int {
		if m, ok := in["minutes"].(float64); ok {
			return min(max(int(m), 1), most)
		}
		return def
	}
	switch op {
	case "start":
		ds.touch(v.Name)
		return "Started.", true, startVM(ctx, v)
	case "stop":
		return fmt.Sprintf("Stopped %s. Its files stay; it stays stopped until started.", v.Name), true, k.stop(ctx, v)
	case "delete":
		if err := ds.deleteVM(ctx, v); err != nil {
			return nil, true, err
		}
		ds.setLoan(v.Name, nil)
		return fmt.Sprintf("Deleted %s and everything in it.", v.Name), true, nil
	case "give_back":
		if _, ok := ds.loans()[v.Name]; !ok {
			return nil, true, fmt.Errorf("%s isn't borrowed", v.Name)
		}
		if err := ds.giveBackVM(ctx, v, "given back by "+caller(ctx)); err != nil {
			return nil, true, err
		}
		return fmt.Sprintf("Gave back %s.", v.Name), true, nil
	case "view":
		if !k.running(ctx, v) {
			return nil, true, fmt.Errorf("%s is stopped: start it first", v.Name)
		}
		m := minutes(60, 1440)
		url, err := vmViewURL(ctx, v, m)
		if err != nil {
			return nil, true, err
		}
		return fmt.Sprintf("%s\nThe link works for %d minutes, on the person's own devices.", url, m), true, nil
	case "screen":
		if !k.running(ctx, v) {
			return nil, true, fmt.Errorf("%s is stopped: start it first", v.Name)
		}
		width := 640
		if w, ok := in["width"].(float64); ok {
			width = min(max(int(w), 160), 1920)
		}
		r, err := ds.screenVM(ctx, v, width)
		return r, true, err
	case "tools":
		return vmTools(k), true, nil
	case "use":
		tool, _ := in["tool"].(string)
		args, _ := in["args"].(map[string]any)
		r, err := ds.useVM(ctx, v, tool, args)
		return r, true, err
	case "open":
		r, err := ds.useVM(ctx, v, "open_url", in)
		return r, true, err
	case "install":
		r, err := ds.useVM(ctx, v, "install_app", in)
		return r, true, err
	case "run":
		r, err := ds.useVM(ctx, v, "run_command", in)
		return r, true, err
	}
	return nil, true, fmt.Errorf("%s is a %s device: %s works on Linux devices only, for now", v.Name, k.title(), op)
}

// onAnother runs op (create or borrow) of a kind this machine can't run on another server that can, with room.
func (ds *devices) onAnother(ctx context.Context, op string, k kind, in map[string]any, name string) (any, error) {
	got, _ := ds.everywhere(ctx, "kinds", nil)
	var tried []string
	for _, s := range slices.Sorted(maps.Keys(got)) {
		var theirs struct {
			Kinds []struct{ Kind string }
			Free  int `json:"free_gb"`
		}
		json.Unmarshal(got[s], &theirs)
		if !slices.ContainsFunc(theirs.Kinds, func(x struct{ Kind string }) bool { return x.Kind == k.name() }) {
			continue
		}
		pass := map[string]any{"here": true}
		maps.Copy(pass, in)
		if name != "" {
			pass["device"] = name
		}
		r, err := ds.callNode(ctx, s, op, pass)
		if err == nil {
			return r, nil
		}
		tried = append(tried, s+": "+err.Error())
	}
	if len(tried) > 0 {
		return nil, errors.New(strings.Join(tried, "; "))
	}
	return nil, fmt.Errorf("none of your servers runs %s devices (here: %s)", k.title(), k.can())
}

// adopt makes something already on this machine a lasting device: a Windows VM's folder (its disk.qcow2,
// OVMF_VARS.fd, tpm and monitor.sock, made as windows.go makes them), an Android Virtual Device, or a simulator.
func (ds *devices) adopt(ctx context.Context, name, kindName string, in map[string]any) (any, error) {
	text := func(k string) string { s, _ := in[k].(string); return strings.TrimSpace(s) }
	num := func(k string) int { n, _ := in[k].(float64); return int(n) }
	k, err := kindNamed(kindName)
	if err != nil {
		return nil, err
	}
	if why := k.can(); why != "" {
		return nil, errors.New(why)
	}
	if !validName.MatchString(name) {
		return nil, errors.New("a device's name is lowercase letters, digits and dashes")
	}
	if ds.taken(ctx, name) {
		return nil, fmt.Errorf("there's already a device named %s", name)
	}
	v := &vm{Name: name, Kind: k.name(), Made: time.Now().UTC(), CPUs: num("cpus"), GB: num("gb"), Port: num("port"), Display: num("display"), ID: text("id")}
	v.Dir = text("dir")
	if err := k.adopt(ctx, v); err != nil {
		return nil, err
	}
	if err := ds.saveVM(v); err != nil {
		return nil, err
	}
	return fmt.Sprintf("%s is a lasting %s device now, %s.", v.Name, k.title(), v.state(ctx)), nil
}

// runLinux runs open, install and run on a Linux device.
func (ds *devices) runLinux(ctx context.Context, op, name string, input map[string]any) (any, error) {
	text := func(k string) string { s, _ := input[k].(string); return strings.TrimSpace(s) }
	d, err := ds.running(ctx, name)
	if err != nil {
		return nil, err
	}
	script := text("command")
	switch op {
	case "open":
		if text("url") == "" {
			return nil, errors.New("open which url?")
		}
		script = `runuser -u cua -- env DISPLAY=:1 HOME=` + home + ` setsid google-chrome --no-first-run --no-default-browser-check --password-store=basic ` + quote(text("url")) + ` >/dev/null 2>&1 </dev/null &`
	case "install":
		app := cmpOr(text("app"), text("file"))
		switch {
		case app == "":
			return nil, errors.New("install what? a package's name, or a .deb's URL")
		case strings.HasPrefix(app, "http://") || strings.HasPrefix(app, "https://"):
			script = `set -e; curl -fsSLo /tmp/app.deb ` + quote(app) + ` && apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq /tmp/app.deb; rm -f /tmp/app.deb`
		default:
			script = `set -e; apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq ` + quote(app)
		}
	}
	if script == "" {
		return nil, errors.New("run what? pass command")
	}
	var out bytes.Buffer
	err = in(ctx, d, strings.NewReader(script), &out, "sh", "-s")
	if err != nil {
		return nil, fmt.Errorf("%v\n%s", err, out.String())
	}
	if op == "open" {
		return "Opened.", nil
	}
	return out.String(), nil
}

// outliving is a command run so that it outlives this sidecar: on Linux in a scope of the user's own systemd, as a
// service's restart stops everything in its own; on a Mac, in a session of its own.
func outliving(name string, args ...string) (string, []string) {
	if runtime.GOOS == "linux" {
		if _, err := exec.LookPath("systemd-run"); err == nil {
			return "systemd-run", append([]string{"--user", "--scope", "--quiet", "--collect", "--", name}, args...)
		}
	}
	return name, args
}

// userSystemd points this program at the user's own systemd, which a service has no login session for; lingering
// keeps it running.
func userSystemd() {
	if runtime.GOOS != "linux" || os.Getenv("XDG_RUNTIME_DIR") != "" {
		return
	}
	dir := fmt.Sprintf("/run/user/%d", os.Getuid())
	if _, err := os.Stat(dir); err == nil {
		os.Setenv("XDG_RUNTIME_DIR", dir)
		os.Setenv("DBUS_SESSION_BUS_ADDRESS", "unix:path="+dir+"/bus")
	}
}

func runOutliving(ctx context.Context, name string, args ...string) (string, error) {
	n, a := outliving(name, args...)
	return run(ctx, n, a...)
}
