// The One App's iPhone client as a MyGo app (spike, egoist/mygo PR #940), on the core the desktop shares: the Rust
// client linked in statically (../client with -tags cgoffi), sign-in (../signin) in the system's sign-in sheet, the
// device's key and the refresh token in the Keychain (mygo.Secrets), and app screens in WKWebView answering the `one`
// bridge (../bridge), untrusted, at screen://<app>/ as on the desktop.
//
// Spike only: the address and the first screen come from the environment (ONE_ADDRESS, ONE_SCREEN), and ONE_PROBE
// makes the screen send bridge messages by itself, as nobody taps the simulator.
package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"log"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/drevantonder/the-one-app/mygo/bridge"
	"github.com/drevantonder/the-one-app/mygo/client"
	"github.com/drevantonder/the-one-app/mygo/ios/shell"
	"github.com/drevantonder/the-one-app/mygo/signin"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/transfer"
)

func env(name, fallback string) string {
	if v := os.Getenv(name); v != "" {
		return v
	}
	return fallback
}

var secrets = mygo.Secrets.Service("one")

// keychain is the iPhone's Keychain, as sign-in's Store.
type keychain struct{}

func (keychain) Get(k string) ([]byte, error) { return secrets.Get(safe(k)) }
func (keychain) Set(k string, v []byte) error { return secrets.Set(safe(k), v) }
func (keychain) Delete(k string) error        { return secrets.Delete(safe(k)) }

// safe is a key as mygo.Secrets takes it: sign-in's keys start with the address.
func safe(k string) string {
	sum := sha256.Sum256([]byte(k))
	return "signin-" + hex.EncodeToString(sum[:8])
}

var (
	home   *mygo.Window
	device *client.ThisDevice
	data   = filepath.Join(os.Getenv("HOME"), "Library", "Application Support")

	screensMu sync.Mutex
	screens   = map[*mygo.Page]*screen{}
)

func main() {
	mygo.Protocol.HandleFunc("screen", serveScreen)
	mygo.App.WhenReady(func() {
		home = mygo.NewWindow(mygo.WindowOptions{Title: "The One App"})
		say("Signing in…")
		go func() {
			if err := start(); err != nil {
				log.Printf("one: %v", err)
				say(err.Error())
			}
		}()
	})
	if err := mygo.App.Run(); err != nil {
		log.Fatal(err)
	}
}

// say shows one grey line, the shell's own page while no screen shows.
func say(text string) {
	home.Page().LoadHTML(`<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1"><body style="font:17px -apple-system;color:#888;display:grid;place-items:center;height:90vh;margin:0">`+
		strings.ReplaceAll(text, "<", "&lt;"), "")
}

func start() error {
	auth := &signin.Auth{Address: strings.TrimRight(env("ONE_ADDRESS", "http://127.0.0.1:43181"), "/"), Name: "iPhone (MyGo)", Store: keychain{}}
	secret, err := secrets.Get("device")
	if errors.Is(err, mygo.ErrSecretNotFound) {
		secret = client.NewDeviceSecret()
		err = secrets.Set("device", secret)
		log.Printf("one: made this device's key, kept in the Keychain")
	} else if err == nil {
		log.Printf("one: this device's key is from the Keychain")
	}
	if err != nil {
		return err
	}
	if auth.SignedIn() {
		log.Printf("one: signed in already, by the refresh token in the Keychain")
	} else {
		join, err := client.DeviceJoin(secret)
		if err != nil {
			return err
		}
		// Sign-in comes back to a loopback port (signin's redirect), which the sheet loads; then the sheet goes.
		err = auth.SignIn(context.Background(), join, shell.OpenSignIn)
		shell.CloseSignIn()
		if err != nil {
			return err
		}
		log.Printf("one: signed in in the system's sign-in sheet")
	}
	sum := sha256.Sum256(secret)
	device, err = client.NewThisDevice(secret, auth.Address, filepath.Join(data, "client", hex.EncodeToString(sum[:8])))
	if err != nil {
		return err
	}
	device.On(hex.EncodeToString(sum[:]))
	in, err := device.Connect()
	if err != nil && !errors.Is(err, client.ErrNotLetIn) {
		return err
	}
	if !in {
		token, err := auth.Token()
		if err != nil {
			return err
		}
		if err := device.Ask(token, "iPhone (MyGo)", nil); err != nil {
			return err
		}
		code := device.Code()
		log.Printf("one: waiting to be let in, code %s", code)
		say("Approve this iPhone on one of your devices: " + code)
		for {
			token, err := auth.Token()
			if err != nil {
				return err
			}
			if ok, err := device.Wait(token); err != nil {
				return err
			} else if ok {
				break
			}
			time.Sleep(2 * time.Second)
		}
	}
	log.Printf("one: let in")
	app, name, _ := strings.Cut(env("ONE_SCREEN", "device-activity/main"), "/")
	return show(bridge.Route{App: app, Screen: name, Params: "{}"})
}

type screen struct {
	route bridge.Route
	html  string
	host  *bridge.Host
	win   *mygo.Window
}

// show opens a screen's page, as its node makes it, in a window of its own: on a phone, over the one before.
func show(r bridge.Route) error {
	html, err := device.Screen(r.App, r.Screen, r.Params, r.Scope)
	if err != nil {
		return err
	}
	if probe := os.Getenv("ONE_PROBE"); probe != "" {
		html = strings.Replace(html, "</body>", "<script>addEventListener('load',()=>setTimeout(()=>{"+probe+"},1500))</script></body>", 1)
	}
	s := &screen{route: r, html: html}
	s.host = &bridge.Host{Device: device, Route: r, Shell: &iphone{s: s}, Kept: filepath.Join(data, "kept")}
	s.win = mygo.NewWindow(mygo.WindowOptions{
		Title: r.App, URL: "screen://" + r.App + "/", Hidden: true,
		Page: mygo.PageOptions{Untrusted: true, PreloadScript: bridge.Shim},
	})
	screensMu.Lock()
	screens[s.win.Page()] = s
	screensMu.Unlock()
	s.win.Show()
	log.Printf("one: opened %s/%s (%d bytes)", r.App, r.Screen, len(html))
	return nil
}

// serveScreen is screen://<app>/: its page at /, and its bridge at /one.
func serveScreen(w http.ResponseWriter, r *http.Request) {
	screensMu.Lock()
	s := screens[mygo.CallerPage(r.Context())]
	screensMu.Unlock()
	if s == nil || !strings.HasPrefix(r.Host, s.route.App) {
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
			var m struct{ Type string }
			json.Unmarshal(body, &m)
			log.Printf("one: bridge %s", m.Type)
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

// iphone is bridge.Shell on an iPhone.
type iphone struct{ s *screen }

func (i *iphone) Menu(items []bridge.MenuItem, x, y float64, rect []float64) (int, error) {
	return shell.Menu(items)
}

func (i *iphone) Alert(title, text, action string, danger bool) (bool, error) {
	buttons := []string{"OK"}
	if action != "" {
		buttons = []string{"Cancel", action}
	}
	opts := mygo.MessageOptions{Parent: i.s.win, Title: title, Message: text, Buttons: buttons}
	if danger {
		opts.DestructiveButtons = []int{1}
	}
	r, err := mygo.Dialog.Message(opts)
	return err == nil && (action == "" || r.Button == 1), err
}

func (i *iphone) Copy(text string) error  { return mygo.Clipboard.Write(transfer.TextData(text)) }
func (i *iphone) Link(url string) error   { return mygo.Shell.OpenExternal(url) }
func (i *iphone) Title(text, name string) { i.s.win.SetTitle(text) }
func (i *iphone) Open(to bridge.Route, sheet bool) (json.RawMessage, error) {
	return nil, errors.New("open isn't in this spike")
}
func (i *iphone) Close(value json.RawMessage)            {}
func (i *iphone) Notify() (string, error)                { return "default", nil }
func (i *iphone) Save(name string, data []byte) error    { return errors.New("save isn't in this spike") }
func (i *iphone) Preview(name string, data []byte) error { return errors.New("preview isn't in this spike") }
func (i *iphone) SignOut() error                         { return errors.New("sign out isn't in this spike") }
func (i *iphone) Device() string                         { return "iPhone" }
