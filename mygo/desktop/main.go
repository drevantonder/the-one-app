// The One App's shell on a computer, as a MyGo app: it signs in, keeps this device's key in the OS's secret store,
// lists the person's apps on its home page (the app's own page, which calls Go through the `one` plugin), and opens
// each app's screen in a window of its own. A screen is untrusted: it's served at screen://<app>/ in a page MyGo
// trusts with nothing, and talks only by posting bridge.js's messages to its own origin's /one, which bridge.Host
// answers as that app alone.
package main

import (
	"context"
	"crypto/sha256"
	"embed"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"io/fs"
	"log"
	"net/http"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"time"

	"github.com/drevantonder/the-one-app/mygo/bridge"
	"github.com/drevantonder/the-one-app/mygo/client"
	"github.com/drevantonder/the-one-app/mygo/signin"
	"github.com/egoist/mygo"
)

//go:embed web
var web embed.FS

var (
	address   = flag.String("address", os.Getenv("ONE_ADDRESS"), "your One App's address")
	inWindow  = flag.Bool("sign-in-here", os.Getenv("ONE_SIGN_IN_HERE") != "", "sign in in a window of this app, not the browser (a throwaway network's sign-in, which needs no passkey)")
	forget    = flag.Bool("forget", false, "forget what this computer keeps for -address in the secret store (its device key and sign-in), then quit")
	openApp   = flag.String("open", os.Getenv("ONE_OPEN"), "an app whose main screen opens once signed in")
	started   = time.Now()
	dataDir   string
	auth      *signin.Auth
	device    *client.ThisDevice
	stateMu   sync.Mutex
	state     = State{Phase: "starting"}
	homeWin   *mygo.Window
	screensMu sync.Mutex
	screens   = map[*mygo.Page]*screen{}
	oneEvent  = mygo.NewEvent[json.RawMessage]("one-event")
	changed   = mygo.NewEvent[State]("one-state")
)

// State is what the home page shows: signing in, the code to approve, or the apps.
type State struct {
	Phase   string            `json:"phase"` // starting, signing-in, approve, ready, failed
	Code    string            `json:"code,omitempty"`
	Error   string            `json:"error,omitempty"`
	Apps    []json.RawMessage `json:"apps,omitempty"`
	Person  string            `json:"person,omitempty"`
	ReadyMs int64             `json:"readyMs,omitempty"`
}

func setState(f func(s *State)) {
	stateMu.Lock()
	f(&state)
	s := state
	stateMu.Unlock()
	changed.Broadcast(s)
}

// secrets is mygo.Secrets as the sign-in's Store.
type secrets struct{}

func (secrets) Get(k string) ([]byte, error) { return mygo.Secrets.Get(k) }
func (secrets) Set(k string, v []byte) error { return mygo.Secrets.Set(k, v) }
func (secrets) Delete(k string) error        { return mygo.Secrets.Delete(k) }

// Service is the `one` plugin: what the home page asks of Go.
type Service struct{}

// State is where signing in is, and the apps once it's done.
func (Service) State() State {
	stateMu.Lock()
	defer stateMu.Unlock()
	return state
}

// Open opens an app's main screen in a window of its own.
func (Service) Open(app string) {
	openScreen(bridge.Route{App: app, Screen: "main", Params: "{}"}, nil)
}

// SignOut signs this computer out.
func (Service) SignOut() error { return signOut() }

func main() {
	flag.Parse()
	if p := os.Getenv("ONE_LOG"); p != "" {
		if f, err := os.OpenFile(p, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0o600); err == nil {
			log.SetOutput(f)
		}
	}
	mygo.App.SetName("The One App")
	if *forget {
		a := strings.TrimRight(*address, "/")
		for _, k := range []string{"device " + a, a + " refresh", a + " client"} {
			fmt.Printf("%s: %v\n", strings.SplitN(k, " ", 2)[0], mygo.Secrets.Delete(k))
		}
		return
	}
	if err := client.Load(""); err != nil {
		log.Fatal(err)
	}
	client.Client = "mygo-" + runtime.GOOS
	bridge.IconsFile = findIcons()
	base, _ := mygo.App.Path(mygo.PathUserData)
	dataDir = filepath.Join(base, "one")
	sub, _ := fs.Sub(web, "web")
	mygo.SetFrontend(sub)
	mygo.Use(mygo.Plugin{Name: "one", Service: Service{}})
	mygo.Protocol.HandleFunc("screen", serveScreen)
	probes()
	mygo.App.WhenReady(func() {
		homeWin = mygo.NewWindow(mygo.WindowOptions{Title: "The One App", URL: "/", Width: 900, Height: 640})
		homeWin.OnReadyToShow(func() { log.Printf("home window ready in %v", time.Since(started)) })
		go start()
	})
	if err := mygo.App.Run(); err != nil {
		log.Fatal(err)
	}
}

// findIcons is Phosphor's icons: beside the program, else in the checkout.
func findIcons() string {
	exe, _ := os.Executable()
	wd, _ := os.Getwd()
	for _, p := range []string{filepath.Join(filepath.Dir(exe), "icons.json"), filepath.Join(wd, "..", "vendor", "phosphor", "icons.json"), filepath.Join(wd, "vendor", "phosphor", "icons.json")} {
		if _, err := os.Stat(p); err == nil {
			return p
		}
	}
	return ""
}

func fail(err error) {
	log.Printf("failed: %v", err)
	setState(func(s *State) { s.Phase, s.Error = "failed", err.Error() })
}

// start signs in if it must, lets this device in, then lists the apps.
func start() {
	if *address == "" {
		fail(errors.New("No address: start it with -address <your One App's address>."))
		return
	}
	host, _ := os.Hostname()
	auth = &signin.Auth{Address: strings.TrimRight(*address, "/"), Name: "The One App on " + host, Store: secrets{}}
	secret, err := deviceSecret()
	if err != nil {
		fail(fmt.Errorf("the secret store: %w", err))
		return
	}
	if !auth.SignedIn() {
		setState(func(s *State) { s.Phase = "signing-in" })
		join, err := client.DeviceJoin(secret)
		if err != nil {
			fail(err)
			return
		}
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
		defer cancel()
		var win *mygo.Window
		err = auth.SignIn(ctx, join, func(u string) error {
			if !*inWindow {
				return mygo.Shell.OpenExternal(u)
			}
			// An untrusted page leaves its origin (sign-in's pages, then this machine's loopback callback) only while a
			// listener says it may: so the listener comes first, then the page.
			win = mygo.NewWindow(mygo.WindowOptions{Title: "Sign In", Width: 480, Height: 640, Page: mygo.PageOptions{Untrusted: true}})
			win.Page().OnWillNavigate(func(e *mygo.NavigateEvent) {
				if os.Getenv("ONE_DEBUG") != "" {
					log.Printf("debug sign-in: to %s", e.URL)
				}
			})
			win.Page().LoadURL(u)
			return nil
		})
		if win != nil {
			win.Close()
		}
		if err != nil {
			fail(err)
			return
		}
	}
	token, err := auth.Token()
	if err != nil {
		fail(err)
		return
	}
	sum := sha256.Sum256([]byte(auth.Address + " " + signin.Subject(token)))
	device, err = client.NewThisDevice(secret, auth.Address, filepath.Join(dataDir, "client", hex.EncodeToString(sum[:8])))
	if err != nil {
		fail(err)
		return
	}
	if on := onDevice(); on != "" {
		device.On(on)
	}
	in, err := device.Connect()
	if err != nil && !errors.Is(err, client.ErrNotLetIn) {
		fail(err)
		return
	}
	if !in {
		host, _ := os.Hostname()
		if err := device.Ask(token, host, nil); err != nil {
			fail(err)
			return
		}
		code := device.Code()
		log.Printf("approve this device: %s", code)
		setState(func(s *State) { s.Phase, s.Code = "approve", code })
		for {
			token, _ = auth.Token()
			ok, err := device.Wait(token)
			if err != nil {
				fail(err)
				return
			}
			if ok {
				break
			}
			time.Sleep(2 * time.Second)
		}
	}
	apps, err := device.Apps()
	if err != nil {
		fail(err)
		return
	}
	log.Printf("signed in and listed %d apps in %v", len(apps), time.Since(started))
	setState(func(s *State) { s.Phase, s.Apps, s.ReadyMs = "ready", apps, time.Since(started).Milliseconds() })
	go events()
	if *openApp != "" {
		openScreen(bridge.Route{App: *openApp, Screen: "main", Params: "{}"}, nil)
	}
}

// deviceSecret is this computer's device key, kept in the secret store, made the first time.
func deviceSecret() ([]byte, error) {
	key := "device " + strings.TrimRight(*address, "/")
	if b, err := mygo.Secrets.Get(key); err == nil {
		if s, err := base64.StdEncoding.DecodeString(string(b)); err == nil && len(s) == 32 {
			return s, nil
		}
	} else if !errors.Is(err, mygo.ErrSecretNotFound) {
		return nil, err
	}
	s := client.NewDeviceSecret()
	return s, mygo.Secrets.Set(key, []byte(base64.StdEncoding.EncodeToString(s)))
}

// onDevice is an id of this computer's own, hashed, that outlives signing out.
func onDevice() string {
	for _, p := range []string{"/etc/machine-id", "/var/lib/dbus/machine-id"} {
		if b, err := os.ReadFile(p); err == nil {
			sum := sha256.Sum256([]byte("this device " + strings.TrimSpace(string(b))))
			return hex.EncodeToString(sum[:])
		}
	}
	return ""
}

// events forwards the node's events to every open screen, as `one.__event`.
func events() {
	for {
		err := device.Events(context.Background(), func(e json.RawMessage) {
			screensMu.Lock()
			defer screensMu.Unlock()
			for p := range screens {
				oneEvent.EmitPage(p, e)
			}
		})
		log.Printf("events: %v", err)
		time.Sleep(3 * time.Second)
	}
}

func signOut() error {
	if device != nil {
		device.SignOut()
	}
	if auth != nil {
		auth.SignOut()
	}
	mygo.Secrets.Delete("device " + strings.TrimRight(*address, "/"))
	setState(func(s *State) { *s = State{Phase: "failed", Error: "Signed out. Start the app again to sign in."} })
	return nil
}

// screen is one open screen: its route, its window, and its page as the node gave it.
type screen struct {
	route  bridge.Route
	win    *mygo.Window
	html   string
	host   *bridge.Host
	closed chan json.RawMessage
	value  json.RawMessage
}

// screenHost is a screen's host name: letters, digits and dashes of its app's name.
func screenHost(r bridge.Route) string {
	if r.Own {
		return "own"
	}
	h := strings.Map(func(c rune) rune {
		if c >= 'a' && c <= 'z' || c >= '0' && c <= '9' || c == '-' {
			return c
		}
		return '-'
	}, strings.ToLower(r.App))
	if h = strings.Trim(h, "-"); h == "" {
		return "app"
	}
	return h
}

// openScreen fetches a screen's page, then shows it in a window of its own (a sheet: a modal window over `over`).
func openScreen(r bridge.Route, over *screen) *screen {
	t := time.Now()
	var html string
	var err error
	if r.Own {
		html = client.ClientScreen(r.Screen, r.Params)
	} else {
		html, err = device.Screen(r.App, r.Screen, r.Params, r.Scope)
	}
	if err != nil {
		mygo.Dialog.Error("Can’t Open This", err.Error())
		return nil
	}
	s := &screen{route: r, html: html, closed: make(chan json.RawMessage, 1)}
	s.host = &bridge.Host{Device: device, Route: r, Shell: &shell{s: s}, Kept: filepath.Join(dataDir, "kept")}
	opts := mygo.WindowOptions{
		Title: r.App, URL: "screen://" + screenHost(r) + "/", Width: 820, Height: 680,
		Page: mygo.PageOptions{Untrusted: true, PreloadScript: bridge.Shim, DevTools: mygo.DevToolsEnabled},
	}
	if over != nil {
		opts.Parent, opts.Modal, opts.Width, opts.Height = over.win, true, 560, 600
	}
	s.win = mygo.NewWindow(opts)
	debugPage(r.App+"/"+r.Screen, s.win)
	screensMu.Lock()
	screens[s.win.Page()] = s
	screensMu.Unlock()
	s.win.OnReadyToShow(func() { log.Printf("screen %s/%s shown %v after it was asked for", r.App, r.Screen, time.Since(t)) })
	// Anything that would take the screen away goes to the browser; its own page stays.
	s.win.Page().OnWillNavigate(func(e *mygo.NavigateEvent) {
		if strings.HasPrefix(e.URL, "screen://") || strings.HasPrefix(e.URL, "http://"+screenHost(r)+".screen.localhost") {
			return
		}
		e.PreventDefault()
		outside(e.URL)
	})
	s.win.Page().SetWindowOpenHandler(func(req mygo.WindowOpenRequest) *mygo.WindowOptions {
		outside(req.URL)
		return nil
	})
	s.win.OnClosed(func() {
		screensMu.Lock()
		delete(screens, s.win.Page())
		screensMu.Unlock()
		select {
		case s.closed <- s.value:
		default:
		}
	})
	return s
}

func outside(u string) {
	for _, p := range []string{"https://", "http://", "mailto:", "tel:"} {
		if strings.HasPrefix(u, p) {
			go mygo.Shell.OpenExternal(u)
			return
		}
	}
}

// serveScreen is screen://<app>/: its page at /, and its bridge at /one, for the page that asks alone.
func serveScreen(w http.ResponseWriter, r *http.Request) {
	p := mygo.CallerPage(r.Context())
	screensMu.Lock()
	s := screens[p]
	screensMu.Unlock()
	if s == nil || !strings.HasPrefix(r.Host, screenHost(s.route)) {
		http.NotFound(w, r)
		return
	}
	switch {
	case r.URL.Path == "/" && r.Method == http.MethodGet:
		w.Header().Set("content-type", "text/html; charset=utf-8")
		io.WriteString(w, s.html)
	case r.URL.Path == "/one" && r.Method == http.MethodPost:
		body, err := io.ReadAll(http.MaxBytesReader(w, r.Body, 32<<20))
		reply := map[string]any{}
		if err == nil {
			var ok any
			ok, err = s.host.Answer(body)
			reply["ok"] = ok
		}
		if err != nil {
			reply = map[string]any{"error": err.Error()}
		}
		w.Header().Set("content-type", "application/json")
		json.NewEncoder(w).Encode(reply)
	default:
		http.NotFound(w, r)
	}
}

// debugPage logs where a window's page is and what it says, every few seconds while ONE_DEBUG is set.
func debugPage(name string, win *mygo.Window) {
	if os.Getenv("ONE_DEBUG") == "" {
		return
	}
	win.Page().OnWillNavigate(func(e *mygo.NavigateEvent) { log.Printf("debug %s: to %s", name, e.URL) })
	go func() {
		for range 20 {
			time.Sleep(3 * time.Second)
			v, err := win.Page().Eval("return location.href + ' | ' + (document.body ? document.body.innerText.slice(0, 200) : '(no body)')")
			log.Printf("debug %s: %v %v", name, v, err)
		}
	}()
}
