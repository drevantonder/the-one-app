package main

import (
	"archive/tar"
	"bufio"
	"bytes"
	"compress/gzip"
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"image"
	"io"
	"log"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"sync/atomic"
	"syscall"
	"time"
)

// chrome devices: the machine's own Google Chrome, headless, one for each borrow, with a folder of its own that's
// deleted when it's given back. No container: it starts in a second and reaches what this machine reaches, so a
// machine runs them only while its owner allows it (chromeSwitch). This service drives it over its DevTools pipe
// (--remote-debugging-pipe: no port, nothing else can reach it) and answers the browser tools itself (browse.go).
// Chrome's own sandbox stays on.

const chromeName = "chrome"

type chromeKind struct{}

func (chromeKind) name() string  { return chromeName }
func (chromeKind) title() string { return "Chrome" }
func (chromeKind) limits() string {
	return "the browser tools (computer_browser_*) read and act on its pages; clicks, typing, keys and scrolling in the page; open_url. It's only a browser: no install_app or run_command"
}
func (chromeKind) input() bool      { return true }
func (chromeKind) port() (int, int) { return 0, 0 }

func (chromeKind) can() string {
	if chromePath() == "" {
		return "Google Chrome isn't installed here"
	}
	return ""
}

// chromePath is this machine's Chrome, or "".
func chromePath() string {
	if runtime.GOOS == "darwin" {
		home, _ := os.UserHomeDir()
		for _, p := range []string{"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", filepath.Join(home, "Applications/Google Chrome.app/Contents/MacOS/Google Chrome")} {
			if _, err := os.Stat(p); err == nil {
				return p
			}
		}
		return ""
	}
	for _, name := range []string{"google-chrome", "google-chrome-stable", "chromium", "chromium-browser"} {
		if p, err := exec.LookPath(name); err == nil {
			return p
		}
	}
	return ""
}

func (chromeKind) adopt(context.Context, *vm) error {
	return errors.New("chrome devices are only borrowed: there's nothing to adopt")
}

// make makes the device's folder, its Chrome's user data.
func (chromeKind) make(_ context.Context, v *vm, _ *vm) error {
	dir, err := os.MkdirTemp("", "agent-devices-"+v.Name+"-")
	if err != nil {
		return err
	}
	v.Dir, v.GB = dir, 1
	return nil
}

func (chromeKind) start(ctx context.Context, v *vm) error {
	_, err := launch(ctx, v)
	return err
}

func (chromeKind) stop(_ context.Context, v *vm) error {
	if b := browserOf(v.Name); b != nil {
		b.close()
	}
	return nil
}

// Chrome can't pause: a borrowed one runs until it's given back.
func (chromeKind) pause(context.Context, *vm) error  { return nil }
func (chromeKind) resume(context.Context, *vm) error { return nil }

func (chromeKind) running(_ context.Context, v *vm) bool {
	b := browserOf(v.Name)
	return b != nil && b.alive()
}

func (k chromeKind) remove(ctx context.Context, v *vm) error {
	k.stop(ctx, v)
	if v.Dir != "" && strings.Contains(filepath.Base(v.Dir), "agent-devices-") {
		// A Chrome this program started before it restarted.
		exec.Command("pkill", "-f", "--", "--user-data-dir="+v.Dir).Run()
		return os.RemoveAll(v.Dir)
	}
	return nil
}

func (chromeKind) screen(ctx context.Context, v *vm) (image.Image, error) {
	b, err := liveBrowser(v)
	if err != nil {
		return nil, err
	}
	p, err := b.currentPage(ctx)
	if err != nil {
		return nil, err
	}
	raw, err := b.call(ctx, p.session, "Page.captureScreenshot", map[string]any{"format": "jpeg", "quality": 80})
	if err != nil {
		return nil, err
	}
	var shot struct{ Data string }
	json.Unmarshal(raw, &shot)
	data, err := base64.StdEncoding.DecodeString(shot.Data)
	if err != nil {
		return nil, err
	}
	img, _, err := image.Decode(bytes.NewReader(data))
	return img, err
}

// act is the viewer's and computer_use's input, in the page's CSS pixels.
func (chromeKind) act(ctx context.Context, v *vm, a action) error {
	b, err := liveBrowser(v)
	if err != nil {
		return err
	}
	p, err := b.currentPage(ctx)
	if err != nil {
		return err
	}
	switch a.Do {
	case "click", "double_click", "right_click":
		button, count := "left", 1
		if a.Do == "right_click" {
			button = "right"
		}
		if a.Do == "double_click" {
			count = 2
		}
		_, err = b.click(ctx, p, a.X, a.Y, button, count)
	case "type":
		_, err = b.input(ctx, p, "Input.insertText", map[string]any{"text": a.Text})
	case "key":
		_, err = b.key(ctx, p, a.Key)
	case "scroll":
		_, err = b.input(ctx, p, "Input.dispatchMouseEvent", map[string]any{"type": "mouseWheel", "x": a.X, "y": a.Y, "deltaX": a.X - a.ToX, "deltaY": a.Y - a.ToY})
	case "swipe":
		_, err = b.drag(ctx, p, a.X, a.Y, a.ToX, a.ToY)
	default:
		return fmt.Errorf("no action %q", a.Do)
	}
	return err
}

func (chromeKind) open(ctx context.Context, v *vm, url string) error {
	b, err := liveBrowser(v)
	if err != nil {
		return err
	}
	p, err := b.currentPage(ctx)
	if err != nil {
		return err
	}
	return b.navigate(ctx, p, url)
}

func (chromeKind) install(context.Context, *vm, string) (string, error) {
	return "", errors.New("a chrome device is only a browser: nothing installs on it; borrow a linux one for that")
}

func (chromeKind) run(context.Context, *vm, string) (string, error) {
	return "", errors.New("a chrome device is only a browser: it runs no commands; borrow a linux one for that")
}

func (chromeKind) reach(context.Context, *vm, int) (string, error) {
	return "", errors.New("a chrome device already reaches what this machine reaches")
}

// unpackProfile puts a browser profile's last save in a chrome device's folder: its files and folders only.
func (ds *devices) unpackProfile(ctx context.Context, v *vm, p *profile) error {
	if p.Blob == "" {
		return nil
	}
	sealed, err := ds.getBlob(ctx, p.Blob)
	if err != nil {
		return fmt.Errorf("fetching the %s profile: %w", p.Name, err)
	}
	plain, err := unseal(p.Key, p.Salt, sealed)
	if err != nil {
		return err
	}
	z, err := gzip.NewReader(bytes.NewReader(plain))
	if err != nil {
		return fmt.Errorf("unpacking the %s profile: %w", p.Name, err)
	}
	t := tar.NewReader(z)
	for {
		h, err := t.Next()
		if err == io.EOF {
			return nil
		}
		if err != nil {
			return fmt.Errorf("unpacking the %s profile: %w", p.Name, err)
		}
		name := filepath.Clean(h.Name)
		if name == "." || filepath.IsAbs(name) || name == ".." || strings.HasPrefix(name, "../") {
			continue
		}
		to := filepath.Join(v.Dir, name)
		switch h.Typeflag {
		case tar.TypeDir:
			err = os.MkdirAll(to, 0o700)
		case tar.TypeReg:
			os.MkdirAll(filepath.Dir(to), 0o700)
			var f *os.File
			if f, err = os.OpenFile(to, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0o600); err == nil {
				_, err = io.Copy(f, t)
				f.Close()
			}
		}
		if err != nil {
			return fmt.Errorf("unpacking the %s profile: %w", p.Name, err)
		}
	}
}

// A browser is a chrome device's Chrome, and this program's DevTools connection to it: the browser's own, with a
// session for each page (flat sessions, Target.setAutoAttach).
type browser struct {
	name string
	cmd  *exec.Cmd
	w    io.WriteCloser
	wMu  sync.Mutex
	next atomic.Int64
	done chan struct{} // closed once Chrome is gone

	mu       sync.Mutex
	waiting  map[int64]chan reply_
	pages    map[string]*page // by session
	current  string           // the page the person sees and computer_use acts on, by target
	ua       string           // Chrome's user agent, without "Headless"
	snaps    int              // snapshots taken, for their ids
	dialogs  int              // dialogs seen, for their ids
	launched time.Time
}

// A page is a tab, as one session of the connection sees it.
type page struct {
	target, session string
	refs            map[string]int64 // backend DOM node ids by ref, since its last navigation
	outline         []string         // its last snapshot's lines, for a continuation
	snapshot        string
	dialog          *dialog
	opened          chan struct{} // a dialog opened
	loaded          chan struct{} // its load event
}

type dialog struct {
	ID, Kind, Message, Default, URL string
}

type reply_ struct {
	result json.RawMessage
	err    error
}

var (
	browsersMu sync.Mutex
	browsers   = map[string]*browser{}
)

func browserOf(name string) *browser {
	browsersMu.Lock()
	defer browsersMu.Unlock()
	return browsers[name]
}

func liveBrowser(v *vm) (*browser, error) {
	if b := browserOf(v.Name); b != nil && b.alive() {
		return b, nil
	}
	return nil, fmt.Errorf("%s's Chrome isn't running: borrow another", v.Name)
}

func (b *browser) alive() bool {
	select {
	case <-b.done:
		return false
	default:
		return true
	}
}

// chromeArgs are what Chrome starts with, as the Linux devices' Chrome does where it makes sense: no first-run
// screens, sign-ins kept in the profile's own store (basic on Linux, as a profile made in a Linux device was; on a
// Mac, never the person's keychain), and headless at the size of a Linux device's screen.
func chromeArgs(dir string) []string {
	args := []string{"--headless=new", "--remote-debugging-pipe", "--user-data-dir=" + dir, "--window-size=1280,800",
		"--no-first-run", "--no-default-browser-check", "--disable-search-engine-choice-screen", "--hide-crash-restore-bubble"}
	if runtime.GOOS == "darwin" {
		args = append(args, "--use-mock-keychain")
	} else {
		args = append(args, "--password-store=basic")
	}
	return append(args, "about:blank")
}

// launch starts a chrome device's Chrome, its DevTools on fds 3 (to it) and 4 (from it).
func launch(ctx context.Context, v *vm) (*browser, error) {
	path := chromePath()
	if path == "" {
		return nil, errors.New(chromeKind{}.can())
	}
	if b := browserOf(v.Name); b != nil {
		b.close()
	}
	toR, toW, err := os.Pipe()
	if err != nil {
		return nil, err
	}
	fromR, fromW, err := os.Pipe()
	if err != nil {
		return nil, err
	}
	cmd := exec.Command(path, chromeArgs(v.Dir)...)
	cmd.ExtraFiles = []*os.File{toR, fromW}
	var stderr bytes.Buffer
	cmd.Stderr = &limited{&stderr, 8 << 10}
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	if err := cmd.Start(); err != nil {
		return nil, err
	}
	toR.Close()
	fromW.Close()
	b := &browser{name: v.Name, cmd: cmd, w: toW, done: make(chan struct{}), waiting: map[int64]chan reply_{}, pages: map[string]*page{}, launched: time.Now()}
	go func() {
		b.read(fromR)
		cmd.Wait()
		close(b.done)
		b.mu.Lock()
		for id, c := range b.waiting {
			c <- reply_{err: errors.New("Chrome closed")}
			delete(b.waiting, id)
		}
		b.mu.Unlock()
	}()
	browsersMu.Lock()
	browsers[v.Name] = b
	browsersMu.Unlock()
	ctx, cancel := context.WithTimeout(ctx, 30*time.Second)
	defer cancel()
	raw, err := b.call(ctx, "", "Browser.getVersion", nil)
	if err != nil {
		b.close()
		return nil, fmt.Errorf("Chrome didn't start: %v: %s", err, strings.TrimSpace(stderr.String()))
	}
	var ver struct{ UserAgent string }
	json.Unmarshal(raw, &ver)
	b.ua = strings.Replace(ver.UserAgent, "HeadlessChrome", "Chrome", 1)
	if _, err := b.call(ctx, "", "Target.setAutoAttach", map[string]any{"autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true}); err != nil {
		b.close()
		return nil, err
	}
	if _, err := b.currentPage(ctx); err != nil {
		b.close()
		return nil, err
	}
	log.Printf("%s: Chrome started in %s", v.Name, time.Since(b.launched).Round(time.Millisecond))
	return b, nil
}

// limited keeps the first n bytes written to it.
type limited struct {
	b *bytes.Buffer
	n int
}

func (l *limited) Write(p []byte) (int, error) {
	if room := l.n - l.b.Len(); room > 0 {
		l.b.Write(p[:min(len(p), room)])
	}
	return len(p), nil
}

// close ends Chrome, and everything it started.
func (b *browser) close() {
	if b.cmd.Process != nil && b.alive() {
		syscall.Kill(-b.cmd.Process.Pid, syscall.SIGKILL)
	}
	b.w.Close()
	select {
	case <-b.done:
	case <-time.After(5 * time.Second):
	}
	browsersMu.Lock()
	if browsers[b.name] == b {
		delete(browsers, b.name)
	}
	browsersMu.Unlock()
}

// read takes Chrome's messages, each JSON ended by a NUL: answers go to whoever waits for them, events are handled.
func (b *browser) read(r io.Reader) {
	in := bufio.NewReaderSize(r, 1<<20)
	for {
		msg, err := in.ReadBytes(0)
		if err != nil {
			return
		}
		var m struct {
			ID     int64           `json:"id"`
			Method string          `json:"method"`
			Params json.RawMessage `json:"params"`
			Result json.RawMessage `json:"result"`
			Error  *struct {
				Message string `json:"message"`
			} `json:"error"`
			Session string `json:"sessionId"`
		}
		if json.Unmarshal(msg[:len(msg)-1], &m) != nil {
			continue
		}
		if m.ID != 0 {
			b.mu.Lock()
			c := b.waiting[m.ID]
			delete(b.waiting, m.ID)
			b.mu.Unlock()
			if c != nil {
				r := reply_{result: m.Result}
				if m.Error != nil {
					r.err = errors.New(m.Error.Message)
				}
				c <- r
			}
			continue
		}
		b.event(m.Method, m.Session, m.Params)
	}
}

func (b *browser) event(method, session string, params json.RawMessage) {
	switch method {
	case "Target.attachedToTarget":
		var e struct {
			SessionID  string `json:"sessionId"`
			TargetInfo struct {
				TargetID string `json:"targetId"`
				Type     string `json:"type"`
			} `json:"targetInfo"`
		}
		json.Unmarshal(params, &e)
		if e.TargetInfo.Type == "page" {
			b.mu.Lock()
			b.pages[e.SessionID] = &page{target: e.TargetInfo.TargetID, session: e.SessionID, refs: map[string]int64{}, opened: make(chan struct{}, 1), loaded: make(chan struct{}, 1)}
			if b.current == "" {
				b.current = e.TargetInfo.TargetID
			}
			b.mu.Unlock()
		}
		go b.ready(e.SessionID, e.TargetInfo.Type == "page")
	case "Target.detachedFromTarget":
		var e struct {
			SessionID string `json:"sessionId"`
		}
		json.Unmarshal(params, &e)
		b.mu.Lock()
		if p := b.pages[e.SessionID]; p != nil && p.target == b.current {
			b.current = ""
		}
		delete(b.pages, e.SessionID)
		b.mu.Unlock()
	case "Page.javascriptDialogOpening":
		var e struct{ URL, Message, Type, DefaultPrompt string }
		json.Unmarshal(params, &e)
		b.mu.Lock()
		if p := b.pages[session]; p != nil {
			b.dialogs++
			p.dialog = &dialog{ID: fmt.Sprintf("dialog-%d", b.dialogs), Kind: e.Type, Message: e.Message, Default: e.DefaultPrompt, URL: e.URL}
			signal(p.opened)
		}
		b.mu.Unlock()
	case "Page.javascriptDialogClosed":
		b.mu.Lock()
		if p := b.pages[session]; p != nil {
			p.dialog = nil
		}
		b.mu.Unlock()
	case "Page.loadEventFired":
		b.mu.Lock()
		if p := b.pages[session]; p != nil {
			signal(p.loaded)
		}
		b.mu.Unlock()
	case "Page.frameNavigated":
		var e struct {
			Frame struct{ ParentID string } `json:"frame"`
		}
		json.Unmarshal(params, &e)
		if e.Frame.ParentID == "" {
			b.mu.Lock()
			if p := b.pages[session]; p != nil {
				p.refs, p.outline = map[string]int64{}, nil
			}
			b.mu.Unlock()
		}
	}
}

func signal(c chan struct{}) {
	select {
	case c <- struct{}{}:
	default:
	}
}

// ready sets up a target Chrome attached, which waits for it: a page's events, and its user agent without
// "Headless", which sites refuse.
func (b *browser) ready(session string, isPage bool) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	if isPage {
		b.call(ctx, session, "Page.enable", nil)
		if b.ua != "" {
			b.call(ctx, session, "Network.setUserAgentOverride", map[string]any{"userAgent": b.ua})
		}
	}
	b.call(ctx, session, "Runtime.runIfWaitingForDebugger", nil)
}

// call sends a DevTools command, to a page's session or ("") the browser, and waits for its answer.
func (b *browser) call(ctx context.Context, session, method string, params any) (json.RawMessage, error) {
	c, err := b.send(session, method, params)
	if err != nil {
		return nil, err
	}
	select {
	case r := <-c:
		return r.result, r.err
	case <-ctx.Done():
		return nil, fmt.Errorf("%s: %w", method, ctx.Err())
	}
}

func (b *browser) send(session, method string, params any) (chan reply_, error) {
	id := b.next.Add(1)
	msg := map[string]any{"id": id, "method": method}
	if params != nil {
		msg["params"] = params
	}
	if session != "" {
		msg["sessionId"] = session
	}
	raw, _ := json.Marshal(msg)
	c := make(chan reply_, 1)
	b.mu.Lock()
	b.waiting[id] = c
	b.mu.Unlock()
	b.wMu.Lock()
	_, err := b.w.Write(append(raw, 0))
	b.wMu.Unlock()
	if err != nil {
		b.mu.Lock()
		delete(b.waiting, id)
		b.mu.Unlock()
		return nil, fmt.Errorf("Chrome closed: %w", err)
	}
	return c, nil
}

// errDialog is input that opened a dialog, which holds the page until it's answered.
var errDialog = errors.New("a dialog opened")

// input sends input to a page and waits for it to be taken, or for a dialog it opened: a dialog holds the input
// until it's answered (browser_dialog).
func (b *browser) input(ctx context.Context, p *page, method string, params any) (json.RawMessage, error) {
	select {
	case <-p.opened:
	default:
	}
	c, err := b.send(p.session, method, params)
	if err != nil {
		return nil, err
	}
	select {
	case r := <-c:
		return r.result, r.err
	case <-p.opened:
		return nil, errDialog
	case <-time.After(20 * time.Second):
		return nil, fmt.Errorf("%s: the page didn't take it in 20 seconds", method)
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}

func (b *browser) click(ctx context.Context, p *page, x, y float64, button string, count int) (json.RawMessage, error) {
	if _, err := b.input(ctx, p, "Input.dispatchMouseEvent", map[string]any{"type": "mouseMoved", "x": x, "y": y}); err != nil {
		return nil, err
	}
	for n := 1; n <= count; n++ {
		ev := map[string]any{"x": x, "y": y, "button": button, "clickCount": n}
		ev["type"] = "mousePressed"
		if _, err := b.input(ctx, p, "Input.dispatchMouseEvent", ev); err != nil {
			return nil, err
		}
		ev["type"] = "mouseReleased"
		if _, err := b.input(ctx, p, "Input.dispatchMouseEvent", ev); err != nil {
			return nil, err
		}
	}
	return nil, nil
}

func (b *browser) drag(ctx context.Context, p *page, x, y, toX, toY float64) (json.RawMessage, error) {
	steps := []map[string]any{{"type": "mouseMoved", "x": x, "y": y}, {"type": "mousePressed", "x": x, "y": y, "button": "left", "clickCount": 1}}
	for i := 1; i <= 10; i++ {
		f := float64(i) / 10
		steps = append(steps, map[string]any{"type": "mouseMoved", "x": x + (toX-x)*f, "y": y + (toY-y)*f, "button": "left", "buttons": 1})
	}
	steps = append(steps, map[string]any{"type": "mouseReleased", "x": toX, "y": toY, "button": "left", "clickCount": 1})
	for _, s := range steps {
		if _, err := b.input(ctx, p, "Input.dispatchMouseEvent", s); err != nil {
			return nil, err
		}
	}
	return nil, nil
}

// keys are the keys key takes by name, as DevTools names them, with their Windows key codes.
var keys = map[string]struct {
	key, code string
	vk        int
	text      string
}{
	"enter": {"Enter", "Enter", 13, "\r"}, "return": {"Enter", "Enter", 13, "\r"}, "tab": {"Tab", "Tab", 9, ""}, "escape": {"Escape", "Escape", 27, ""}, "esc": {"Escape", "Escape", 27, ""},
	"backspace": {"Backspace", "Backspace", 8, ""}, "delete": {"Delete", "Delete", 46, ""}, "space": {" ", "Space", 32, " "},
	"up": {"ArrowUp", "ArrowUp", 38, ""}, "down": {"ArrowDown", "ArrowDown", 40, ""}, "left": {"ArrowLeft", "ArrowLeft", 37, ""}, "right": {"ArrowRight", "ArrowRight", 39, ""},
	"home": {"Home", "Home", 36, ""}, "end": {"End", "End", 35, ""}, "pageup": {"PageUp", "PageUp", 33, ""}, "pagedown": {"PageDown", "PageDown", 34, ""},
}

// key presses a key, or a combination like ctrl-a.
func (b *browser) key(ctx context.Context, p *page, combo string) (json.RawMessage, error) {
	parts := strings.FieldsFunc(strings.ToLower(combo), func(r rune) bool { return r == '-' || r == '+' })
	if len(parts) == 0 {
		return nil, errors.New("press which key?")
	}
	mods := 0
	for _, m := range parts[:len(parts)-1] {
		switch m {
		case "alt", "option":
			mods |= 1
		case "ctrl", "control":
			mods |= 2
		case "cmd", "meta", "win", "super":
			mods |= 4
		case "shift":
			mods |= 8
		default:
			return nil, fmt.Errorf("no modifier %q: there's ctrl, alt, shift and cmd", m)
		}
	}
	name := parts[len(parts)-1]
	k, ok := keys[name]
	if !ok {
		r := []rune(name)
		if len(r) != 1 {
			return nil, fmt.Errorf("no key %q", name)
		}
		up := strings.ToUpper(name)
		k.key, k.text, k.vk = name, name, int(r[0])
		if r[0] >= 'a' && r[0] <= 'z' {
			k.code, k.vk = "Key"+up, int(up[0])
		} else if r[0] >= '0' && r[0] <= '9' {
			k.code = "Digit" + name
		}
	}
	down := map[string]any{"type": "keyDown", "key": k.key, "code": k.code, "windowsVirtualKeyCode": k.vk, "modifiers": mods}
	if k.text != "" && mods&(2|4) == 0 {
		down["text"] = k.text
	} else {
		down["type"] = "rawKeyDown"
	}
	if mods&(2|4) != 0 && name == "a" {
		down["commands"] = []string{"selectAll"}
	}
	if _, err := b.input(ctx, p, "Input.dispatchKeyEvent", down); err != nil {
		return nil, err
	}
	return b.input(ctx, p, "Input.dispatchKeyEvent", map[string]any{"type": "keyUp", "key": k.key, "code": k.code, "windowsVirtualKeyCode": k.vk, "modifiers": mods})
}

// navigate opens url in a page and waits for it to load, up to 20 seconds.
func (b *browser) navigate(ctx context.Context, p *page, url string) error {
	select {
	case <-p.loaded:
	default:
	}
	raw, err := b.input(ctx, p, "Page.navigate", map[string]any{"url": url})
	if errors.Is(err, errDialog) {
		return nil
	}
	if err != nil {
		return err
	}
	var nav struct {
		ErrorText string `json:"errorText"`
		LoaderID  string `json:"loaderId"`
	}
	json.Unmarshal(raw, &nav)
	if nav.ErrorText != "" {
		return fmt.Errorf("couldn't open %s: %s", url, nav.ErrorText)
	}
	if nav.LoaderID == "" {
		return nil // the same document, at another #fragment
	}
	select {
	case <-p.loaded:
	case <-p.opened:
	case <-time.After(20 * time.Second):
	case <-ctx.Done():
	}
	return nil
}

// A tab is a page as the browser tools list them.
type tab struct {
	Active bool   `json:"active"`
	ID     string `json:"tab_id"`
	Title  string `json:"title"`
	URL    string `json:"url"`
}

// tabs are the browser's pages, the current one marked active.
func (b *browser) tabs(ctx context.Context) ([]tab, error) {
	raw, err := b.call(ctx, "", "Target.getTargets", nil)
	if err != nil {
		return nil, err
	}
	var got struct {
		TargetInfos []struct{ TargetID, Type, Title, URL string } `json:"targetInfos"`
	}
	json.Unmarshal(raw, &got)
	b.mu.Lock()
	current := b.current
	b.mu.Unlock()
	out := []tab{}
	for _, t := range got.TargetInfos {
		if t.Type == "page" {
			out = append(out, tab{Active: t.TargetID == current, ID: tabID(t.TargetID), Title: t.Title, URL: t.URL})
		}
	}
	return out, nil
}

func tabID(target string) string { return "tab-" + strings.ToLower(target) }

// pageOf is the page with a tab id, which becomes the current one.
func (b *browser) pageOf(ctx context.Context, id string) (*page, error) {
	if id == "" {
		return b.currentPage(ctx)
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	for _, p := range b.pages {
		if tabID(p.target) == strings.ToLower(id) {
			b.current = p.target
			return p, nil
		}
	}
	return nil, fmt.Errorf("no tab %s: it closed, or is another device's; get_browser_state lists the tabs", id)
}

// currentPage is the page the person sees, a new one if every tab closed. Chrome attaches each page as it opens.
func (b *browser) currentPage(ctx context.Context) (*page, error) {
	for range 50 {
		b.mu.Lock()
		var any_ *page
		for _, p := range b.pages {
			if p.target == b.current {
				b.mu.Unlock()
				return p, nil
			}
			if any_ == nil || p.target < any_.target {
				any_ = p
			}
		}
		if any_ != nil {
			b.current = any_.target
			b.mu.Unlock()
			return any_, nil
		}
		b.mu.Unlock()
		if !b.alive() {
			return nil, errors.New("Chrome closed")
		}
		if _, err := b.call(ctx, "", "Target.getTargets", nil); err == nil {
			// None attached yet: a new tab if none is open.
			if tabs, _ := b.tabs(ctx); len(tabs) == 0 {
				b.call(ctx, "", "Target.createTarget", map[string]any{"url": "about:blank"})
			}
		}
		select {
		case <-time.After(100 * time.Millisecond):
		case <-ctx.Done():
			return nil, ctx.Err()
		}
	}
	return nil, errors.New("Chrome has no tab to use")
}

// chromeGB is the memory a chrome device is taken to need, in GB.
const chromeGB = 1

// borrowChrome lends a new chrome device here, its Chrome started with a copy of the browser profile in, if any.
func (ds *devices) borrowChrome(ctx context.Context, in map[string]any) (any, error) {
	text := func(k string) string { s, _ := in[k].(string); return strings.TrimSpace(s) }
	why := text("for")
	if why == "" {
		return nil, errors.New("say what it's for, in a few words: the person sees it")
	}
	if no := ds.chromeWhyNot(ctx); no != "" {
		return nil, fmt.Errorf("none of your devices runs chrome devices now: %s", no)
	}
	if !roomFor(chromeGB) {
		return nil, errors.New("this machine has no memory to spare for another Chrome now: give back one you're done with")
	}
	p, err := profileIn(in)
	if err != nil {
		return nil, err
	}
	// A profile is a Linux Chrome's, its sign-ins sealed with Linux's basic key, which a Mac's Chrome can't open.
	skipped := ""
	if p != nil && runtime.GOOS == "darwin" {
		skipped, p = p.Name, nil
	}
	v, err := ds.makeVM(ctx, chromeKind{}, newPoolName(), nil)
	if err != nil {
		return nil, err
	}
	l := loan{By: cmpOr(text("by"), caller(ctx)), For: why, NoScreen: in["screen"] == false, Since: time.Now().UTC()}
	if p != nil {
		l.Profile, l.Item = p.Name, p.Item
	}
	ds.setLoan(v.Name, &l)
	ds.hold(v.Name, p)
	ds.touch(v.Name)
	fail := func(err error) (any, error) {
		ds.deleteVM(context.WithoutCancel(ctx), v)
		ds.setLoan(v.Name, nil)
		return nil, err
	}
	if p != nil {
		if err := ds.unpackProfile(ctx, v, p); err != nil {
			return fail(err)
		}
	}
	if err := ds.awake(ctx, v); err != nil {
		return fail(err)
	}
	b, err := liveBrowser(v)
	if err != nil {
		return fail(err)
	}
	pg, err := b.currentPage(ctx)
	if err != nil {
		return fail(err)
	}
	me, _, _ := ds.hosts(ctx)
	log.Printf("%s borrowed %s (chrome) for %q, profile %q", l.By, v.Name, l.For, l.Profile)
	out := map[string]any{"device": v.Name, "on": me, "kind": chromeName, "target_id": targetOf(v), "tab_id": tabID(pg.target)}
	msg := fmt.Sprintf("Borrowed %s, Chrome on %s, deleted with everything in it when you give it back. Pass device %q to the browser tools, with target_id %q and tab_id %q: its Chrome is running, no browser_prepare needed.",
		v.Name, me, v.Name, targetOf(v), tabID(pg.target))
	if p != nil {
		out["profile"] = p.Name
		msg += fmt.Sprintf(" It started with a copy of the %s profile.", p.Name)
	}
	if skipped != "" {
		msg += fmt.Sprintf(" It started without the %s profile: a profile's sign-ins open in Chrome on Linux only, not on a Mac.", skipped)
	}
	out["text"] = msg + " Give it back with give_back when done; it's given back for you after 15 minutes without a call."
	return out, nil
}
