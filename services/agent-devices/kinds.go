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
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"slices"
	"strconv"
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
	// pause freezes a running device, keeping its memory but using no CPU; resume carries on. A kind that can't
	// shuts it down instead (iPhones: Simulator can't pause).
	pause(ctx context.Context, v *vm) error
	resume(ctx context.Context, v *vm) error
	running(ctx context.Context, v *vm) bool
	remove(ctx context.Context, v *vm) error
	screen(ctx context.Context, v *vm) (image.Image, error)
	act(ctx context.Context, v *vm, a action) error
	open(ctx context.Context, v *vm, url string) error
	install(ctx context.Context, v *vm, app string) (string, error)
	run(ctx context.Context, v *vm, command string) (string, error)
	// reach lets v reach port on this machine's 127.0.0.1 at the same port on its own, until it's stopped.
	reach(ctx context.Context, v *vm, port int) (string, error)
}

// runtimes are every kind but linux, as this machine might run them.
var runtimes = []kind{chromeKind{}, windowsQEMU{}, androidEmulator{}, iosSimulator{}}

// A vm is a device of a kind other than linux, as its file in the service's folder says.
type vm struct {
	Name    string    `json:"name"`
	Kind    string    `json:"kind"`
	Of      string    `json:"of,omitempty"`      // the lasting device it's a throwaway copy of
	Base    string    `json:"base,omitempty"`    // windows: the copy's base, a folder of its lasting device's
	Dir     string    `json:"dir,omitempty"`     // its files
	Port    int       `json:"port,omitempty"`    // windows: its SSH; android: its console
	Display int       `json:"display,omitempty"` // windows: its VNC display
	ID      string    `json:"id,omitempty"`      // android: its AVD; iphone: its simulator
	CPUs    int       `json:"cpus,omitempty"`
	GB      int       `json:"gb,omitempty"` // memory
	Paused  bool      `json:"paused,omitempty"`
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
func (ds *devices) kindsHere() []map[string]any {
	var out []map[string]any
	lent := ds.loans()
	if rt != nil {
		all, _ := list(context.Background())
		out = append(out, map[string]any{"kind": linuxKind, "title": "Linux with a browser", "copies": true, "room": len(spares(all, lent)) > 0 || roomForOne()})
	}
	for _, k := range runtimes {
		if k.name() == chromeName {
			// Listed even where it can't run, with why: its owner may allow it.
			if why := ds.chromeWhyNot(context.Background()); why != "" {
				out = append(out, map[string]any{"kind": k.name(), "title": k.title(), "unavailable": why, "room": false})
			} else {
				out = append(out, map[string]any{"kind": k.name(), "title": k.title(), "input": k.limits(), "room": roomFor(chromeGB)})
			}
			continue
		}
		if k.can() == "" {
			unborrowed := 0
			for _, v := range ds.vms() {
				if _, ok := lent[v.Name]; v.Kind == k.name() && v.Of == "" && !ok {
					unborrowed++
				}
			}
			out = append(out, map[string]any{"kind": k.name(), "title": k.title(), "input": k.limits(), "room": roomFor(gbOf(nil)), "unborrowed": unborrowed})
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
		if v.Paused {
			return "paused"
		}
		return "running"
	}
	return "stopped"
}

// awake is a device running and not paused, started or resumed if it has to be; it counts as a call to it.
func (ds *devices) awake(ctx context.Context, v *vm) error {
	ds.touch(v.Name)
	k := v.kind()
	if !k.running(ctx, v) {
		if !roomFor(gbOf(v)) {
			return fmt.Errorf("%s is stopped, and this machine has no memory to spare to start it now", v.Name)
		}
		v.Paused = false
		ds.saveVM(v)
		if err := k.start(ctx, v); err != nil {
			return err
		}
		want(v.Name, true)
		return nil
	}
	if v.Paused {
		if err := k.resume(ctx, v); err != nil {
			return err
		}
		v.Paused = false
		return ds.saveVM(v)
	}
	return nil
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

// deleteVM deletes a device and its files.
func (ds *devices) deleteVM(ctx context.Context, v *vm) error {
	if err := v.kind().remove(ctx, v); err != nil {
		return err
	}
	ds.forgetVM(v.Name)
	unwant(v.Name)
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
			} else if b.State == loanLent && b.By == l.By && l.By != someone {
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
		ds.reserve(got.Name, l)
	} else if !ds.change(got.Name, func(b *loan) { b.For = l.For }) {
		return nil, fmt.Errorf("%s is being given back: borrow again", got.Name)
	}
	ds.touch(got.Name)
	if err := ds.awake(ctx, got); err != nil {
		if got.Of != "" || got.Name != name && strings.HasPrefix(got.Name, poolPrefix) {
			ds.deleteVM(context.WithoutCancel(ctx), got)
		}
		ds.endLoan(got.Name)
		return nil, err
	}
	ds.lend(got.Name)
	return got, nil
}

func gbOf(v *vm) int {
	if v != nil && v.GB > 0 {
		return v.GB
	}
	return 4
}

// roomFor reports whether this machine has gb of memory to spare, and 4 GB more for everything else it runs, as
// memory counts it (less the swap in use): a machine short of memory kills what it must, a device or a build or an
// agent.
func roomFor(gb int) bool {
	free, _ := memory()
	return free >= int64(gb+4)<<30
}

// giveBackVM ends a loan: a copy, or a device made for the loan, is deleted; a lasting one stays as it is.
func (ds *devices) giveBackVM(ctx context.Context, v *vm, why string) error {
	if _, err := ds.returning(v.Name); err != nil {
		return err
	}
	if v.Of != "" || strings.HasPrefix(v.Name, poolPrefix) {
		if err := ds.deleteVM(ctx, v); err != nil {
			ds.returnFailed(v.Name)
			return err
		}
	} else if v.Kind == "iphone" {
		// A booted simulator holds gigabytes of the Mac's memory.
		if v.kind().stop(ctx, v) == nil {
			want(v.Name, false)
		}
	}
	ds.endLoan(v.Name)
	log.Printf("%s: %s", v.Name, why)
	return nil
}

// A lasting device's loan ends after this long without a call, a look or a connection to it; a copy's after
// idleBorrowed, as a Linux device's.
const idleLasting = time.Hour

// A borrowed copy left stopped this long without a call is given back: a call starts it again before then.
const idleStoppedCopy = 2 * time.Minute

// A device nobody has borrowed, called or watched for idlePause is paused, and after idleStop shut down: each holds
// cores and gigabytes while it runs.
const (
	idlePauseVM = 15 * time.Minute
	idleStopVM  = time.Hour
)

// tendVMs gives back loans of devices left idle, and of devices that are gone, and pauses and then shuts down
// devices nobody is using.
func (ds *devices) tendVMs(ctx context.Context) {
	defer ds.restVMs(ctx)
	defer ds.pruneBases()
	ds.chromeOff(ctx, "Chrome isn't allowed here now")
	for name, l := range ds.loans() {
		v := ds.vmNamed(name)
		if v == nil || l.State == loanReturning || l.State == loanPreparing && l.Since.After(ds.started) {
			continue
		}
		limit := idleBorrowed
		if v.Of == "" && !strings.HasPrefix(v.Name, poolPrefix) {
			limit = idleLasting
		} else if v.state(ctx) == "stopped" {
			// A throwaway copy that stopped (its emulator killed for memory, or a reinstall mid-boot) holds its disk
			// and its loan for nothing: its borrower borrows another.
			limit = idleStoppedCopy
		}
		if ds.idle(name) < limit {
			continue
		}
		if ds.inUse(ctx, v) {
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

// screenWatched reports whether something other than this service is connected to a device's screen at socket.
func screenWatched(ctx context.Context, socket string) bool {
	out, err := exec.CommandContext(ctx, "ss", "-Hx", "state", "established").Output()
	return err == nil && strings.Contains(string(out), socket+" ")
}

// listVMs is this machine's devices of other kinds, as list shows them.
func (ds *devices) listVMs(ctx context.Context, me string, lent map[string]loan) []listed {
	var out []listed
	for _, v := range ds.vms() {
		l := listed{Name: v.Name, On: me, Kind: v.Kind, State: v.state(ctx), Of: v.Of, CPUs: float64(v.CPUs), MemoryLimit: int64(v.GB) << 30}
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
		"run_command {command}: runs it and gives its output (PowerShell on Windows, the shell on Android, a program in the simulator on an iPhone: it has no shell)",
		"Input here: " + k.limits(),
	}, "\n")
}

// useVM runs one of vmTools on a running device.
func (ds *devices) useVM(ctx context.Context, v *vm, tool string, args map[string]any) (any, error) {
	k := v.kind()
	if !k.running(ctx, v) {
		return nil, fmt.Errorf("%s is stopped: start it first", v.Name)
	}
	if err := ds.awake(ctx, v); err != nil {
		return nil, err
	}
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

// localOnly says whose port is, of what only this machine's own programs use on its 127.0.0.1: its node's (its local
// API, and its apps' runtime and their requests out), adb's server, which drives every phone here, Agent Devices'
// viewer, and each device's controls, screen and shell. "" if it's none of those.
func (ds *devices) localOnly(ctx context.Context, port int) string {
	if u, err := url.Parse(ds.node); err == nil {
		if np, _ := strconv.Atoi(u.Port()); np == port {
			return "this machine's node's"
		} else if node := listeners(ctx, np); len(node) > 0 {
			for _, pid := range listeners(ctx, port) {
				if slices.Contains(node, pid) || slices.Contains(node, parent(ctx, pid)) {
					return "this machine's node's"
				}
			}
		}
	}
	if port == adbPort {
		return "adb's"
	}
	if port == viewerPort {
		return "Agent Devices' viewer's"
	}
	for _, v := range ds.vms() {
		// Android: its console, and adb's port after it. Windows: its shell (SSH), and its screen (VNC).
		if port == v.Port || (v.Kind == "android" && port == v.Port+1) || (v.Display > 0 && port == 5900+v.Display) {
			return v.Name + "'s"
		}
	}
	all, _ := list(ctx)
	for _, d := range all {
		if d.port == port {
			return d.Name + "'s"
		}
	}
	return ""
}

// adbPort is where adb's server listens, which any program reaching it can drive every phone here with.
const adbPort = 5037

// listeners are the processes listening on port on this machine.
func listeners(ctx context.Context, port int) []int {
	var out []byte
	if runtime.GOOS == "linux" {
		out, _ = exec.CommandContext(ctx, "ss", "-Htlnp", fmt.Sprintf("( sport = :%d )", port)).Output()
	} else {
		out, _ = exec.CommandContext(ctx, "lsof", "-nP", fmt.Sprintf("-iTCP:%d", port), "-sTCP:LISTEN", "-Fp").Output()
	}
	// ss says users:(("one",pid=1,fd=9)), lsof -Fp a line p1 for each.
	var pids []int
	for _, f := range strings.FieldsFunc(string(out), func(r rune) bool { return r == ',' || r == '\n' || r == '(' || r == ')' }) {
		f, ok := strings.CutPrefix(f, "pid=")
		if !ok && runtime.GOOS != "linux" {
			f, ok = strings.CutPrefix(f, "p")
		}
		if pid, err := strconv.Atoi(f); ok && err == nil && pid > 0 && !slices.Contains(pids, pid) {
			pids = append(pids, pid)
		}
	}
	return pids
}

// parent is a process's parent.
func parent(ctx context.Context, pid int) int {
	out, _ := exec.CommandContext(ctx, "ps", "-o", "ppid=", "-p", strconv.Itoa(pid)).Output()
	ppid, _ := strconv.Atoi(strings.TrimSpace(string(out)))
	return ppid
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
		me, _, _ := ds.hosts(ctx)
		free, _ := memory()
		mine := map[string]any{"device": me, "kinds": ds.kindsHere(), "free_gb": free >> 30}
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
		out := map[string]any{"devices": all}
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
		if k.name() == chromeName && op == "create" {
			return nil, true, errors.New("chrome devices are only borrowed, each by one agent: borrow one with kind chrome")
		}
		if v == nil && !here {
			if r, done, err := ds.first(ctx, op, k.name(), name, in); done {
				return r, true, err
			}
		}
		if k.name() == chromeName {
			r, err := ds.borrowChrome(ctx, in)
			return r, true, err
		}
		if why := k.can(); why != "" {
			return nil, true, fmt.Errorf("none of your devices runs %s agent devices (here: %s)", k.title(), why)
		}
		if op == "create" {
			if name == "" {
				return nil, true, errors.New("name it: lowercase letters, digits and dashes")
			}
			v, err := ds.makeVM(ctx, k, name, nil)
			if err != nil {
				return nil, true, err
			}
			if err := ds.awake(ctx, v); err != nil {
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
		me, _, _ := ds.hosts(ctx)
		what := fmt.Sprintf("%s, the person's lasting %s device: leave it as you found it", got.Name, k.title())
		if got.Of != "" || strings.HasPrefix(got.Name, poolPrefix) {
			what = fmt.Sprintf("%s, a throwaway %s device of your own, deleted with everything in it when you give it back", got.Name, k.title())
			if got.Of != "" {
				what = fmt.Sprintf("%s, a throwaway copy of %s, deleted with everything in it when you give it back", got.Name, got.Of)
			}
		}
		msg := fmt.Sprintf("Borrowed %s. Pass device %q to computer_screen and computer_use (computer_tools says what it takes: %s). Give it back with give_back when done.",
			what, got.Name, k.limits())
		return map[string]any{"device": got.Name, "on": me, "kind": got.Kind, "copy": got.Of != "", "text": msg}, true, nil
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
	if v.Kind == chromeName && slices.Contains(browsing, op) {
		r, err := ds.browse(ctx, v, op, in)
		return r, true, err
	}
	minutes := func(def, most int) int {
		if m, ok := in["minutes"].(float64); ok {
			return min(max(int(m), 1), most)
		}
		return def
	}
	switch op {
	case "start":
		return "Started.", true, ds.awake(ctx, v)
	case "stop":
		v.Paused = false
		ds.saveVM(v)
		if err := k.stop(ctx, v); err != nil {
			return nil, true, err
		}
		want(v.Name, false)
		return fmt.Sprintf("Stopped %s. Its files stay; it stays stopped until started.", v.Name), true, nil
	case "delete":
		if err := ds.deleteVM(ctx, v); err != nil {
			return nil, true, err
		}
		ds.endLoan(v.Name)
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
		// Someone opening it means to use it: it starts, or carries on.
		if err := ds.awake(ctx, v); err != nil {
			return nil, true, err
		}
		m := minutes(60, 1440)
		url, err := vmViewURL(ctx, v, m)
		if err != nil {
			return nil, true, err
		}
		return fmt.Sprintf("%s\nThe link works for %d minutes, on the person's own devices.", url, m), true, nil
	case "screen":
		width := 640
		if w, ok := in["width"].(float64); ok {
			width = min(max(int(w), 160), 1920)
		}
		r, err := ds.screenNow(ctx, v.Name, width)
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
	case "reach":
		port, _ := in["port"].(float64)
		if port < 1 || port > 65535 {
			return nil, true, errors.New("reach which port? a number from 1 to 65535")
		}
		if mine := ds.localOnly(ctx, int(port)); mine != "" {
			return nil, true, fmt.Errorf("no device reaches port %d: it's %s, for this machine's own programs only", int(port), mine)
		}
		if !k.running(ctx, v) {
			return nil, true, fmt.Errorf("%s is stopped: start it first", v.Name)
		}
		r, err := k.reach(ctx, v, int(port))
		return r, true, err
	}
	return nil, true, fmt.Errorf("%s is a %s device: %s works on Linux devices only, for now", v.Name, k.title(), op)
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
	if v.GB == 0 && k.name() == "iphone" {
		v.GB = 2
	}
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

// outliving is a command run so that it outlives this service: on Linux in a scope of the user's own systemd, as a
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

// restVMs pauses the running devices nobody has used for idlePauseVM, and shuts down those idle for idleStopVM.
func (ds *devices) restVMs(ctx context.Context) {
	lent := ds.loans()
	for _, v := range ds.vms() {
		k := v.kind()
		if _, ok := lent[v.Name]; ok || k == nil || k.can() != "" || !k.running(ctx, v) {
			continue
		}
		if ds.inUse(ctx, v) {
			ds.touch(v.Name)
			continue
		}
		idle := ds.idle(v.Name)
		switch {
		case idle >= idleStopVM:
			log.Printf("stopping %s, idle for %s", v.Name, idleStopVM)
			if err := k.stop(ctx, v); err != nil {
				log.Printf("stopping %s: %v", v.Name, err)
			} else {
				want(v.Name, false) // it carries on when next called
			}
			v.Paused = false
			ds.saveVM(v)
		case idle >= idlePauseVM && !v.Paused:
			log.Printf("pausing %s, idle for %s", v.Name, idlePauseVM)
			if err := k.pause(ctx, v); err != nil {
				log.Printf("pausing %s: %v", v.Name, err)
				continue
			}
			v.Paused = k.running(ctx, v)
			if !v.Paused {
				want(v.Name, false) // pausing stopped it, as an iPhone's does
			}
			ds.saveVM(v)
		}
	}
}

var traffic = map[string]int64{} // each device's bytes through adb at the last look

// inUse reports whether something uses a device without calling this service: an SSH or VNC session into Windows,
// or more than adb's own chatter (512 KB in a look) to a phone.
func (ds *devices) inUse(ctx context.Context, v *vm) bool {
	switch v.Kind {
	case "windows":
		return connected(ctx, v.Port) || screenWatched(ctx, filepath.Join(v.Dir, "vnc.sock"))
	case "android":
		out, err := exec.CommandContext(ctx, "ss", "-tinH", "state", "established", fmt.Sprintf("( dport = :%d )", v.Port+1)).Output()
		if err != nil {
			return false
		}
		var n int64
		for _, f := range strings.Fields(string(out)) {
			if b, ok := strings.CutPrefix(f, "bytes_acked:"); ok {
				var x int64
				fmt.Sscan(b, &x)
				n += x
			}
		}
		was, seen := traffic[v.Name]
		traffic[v.Name] = n
		return seen && n-was > 512<<10
	}
	return false
}
