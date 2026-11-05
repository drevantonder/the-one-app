// Package bridge answers src/screen/bridge.js's messages for a screen, as every shell does: what a screen asks of its
// app goes to the node as the person, and whatever a person touches (menus, alerts, files, links) goes to the
// platform's Shell. It names no UI toolkit, so a desktop (MyGo) and an iPhone share it, each giving its own Shell.
// Screens are untrusted: each acts only as its own app, and only the client's own screens (Route.Own) as the person.
package bridge

import (
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"

	"github.com/drevantonder/the-one-app/mygo/client"
)

// Route is a screen to show: an app's (App, Screen, its JSON Params, the Scope it's opened for) or one of the client's
// own (Own, by Screen).
type Route struct {
	App    string `json:"app,omitempty"`
	Screen string `json:"screen"`
	Params string `json:"params"`
	Scope  string `json:"scope,omitempty"`
	Own    bool   `json:"own,omitempty"`
}

// MenuItem is one of `one.menu`'s items.
type MenuItem struct {
	Title   string `json:"title"`
	Danger  bool   `json:"danger"`
	Checked bool   `json:"checked"`
	Icon    string `json:"icon,omitempty"`
	Row     bool   `json:"row,omitempty"`
	Section bool   `json:"section,omitempty"`
}

// Shell is what a platform does natively for a screen. Each method may block until the person answers; it's called
// from a goroutine of the screen's message.
type Shell interface {
	// Menu shows a menu at (x, y) in the page (or from `rect`, the element's bounds) → the index chosen, or -1.
	Menu(items []MenuItem, x, y float64, rect []float64) (int, error)
	// Alert shows title and text with OK; with `action`, Cancel and that → whether it was chosen.
	Alert(title, text, action string, danger bool) (bool, error)
	Copy(text string) error
	Link(url string) error
	// Title sets the bar's title (empty while the screen shows its own heading) and what the screen is called.
	Title(text, name string)
	// Open shows `to`: pushed over this screen, or in a sheet, giving what it closed with.
	Open(to Route, sheet bool) (json.RawMessage, error)
	// Close takes this screen away: a sheet gives `value` to who opened it.
	Close(value json.RawMessage)
	// Notify asks the person to let this client show notifications → "granted", "denied" or "default".
	Notify() (string, error)
	Save(name string, data []byte) error
	Preview(name string, data []byte) error
	SignOut() error
	// Device names this client: "Linux", "Mac", ….
	Device() string
}

// Host answers one screen's messages.
type Host struct {
	Device *client.ThisDevice
	Shell  Shell
	Route  Route
	// Kept is the folder own screens keep `one.cache` in.
	Kept string
}

var (
	icons     map[string]string
	iconsOnce sync.Once
	// IconsFile is Phosphor's icons (vendor/phosphor/icons.json), which a shell sets to where it ships them.
	IconsFile string
)

func iconPaths() map[string]string {
	iconsOnce.Do(func() {
		b, err := os.ReadFile(IconsFile)
		if err == nil {
			json.Unmarshal(b, &icons)
		}
		if icons == nil {
			icons = map[string]string{}
		}
	})
	return icons
}

// message is any of bridge.js's messages, each field what some type has.
type message struct {
	Type   string          `json:"type"`
	App    string          `json:"app"`
	Op     string          `json:"op"`
	Input  json.RawMessage `json:"input"`
	Screen string          `json:"screen"`
	Params json.RawMessage `json:"params"`
	Scope  string          `json:"scope"`
	Sheet  bool            `json:"sheet"`
	Value  json.RawMessage `json:"value"`
	Text   string          `json:"text"`
	Name   string          `json:"name"`
	Items  []MenuItem      `json:"items"`
	X      float64         `json:"x"`
	Y      float64         `json:"y"`
	Rect   []float64       `json:"rect"`
	Title  string          `json:"title"`
	Action string          `json:"action"`
	Danger bool            `json:"danger"`
	URL    string          `json:"url"`
	File   *file           `json:"file"`
	Names  []string        `json:"names"`
	Method string          `json:"method"`
	Path   string          `json:"path"`
	Body   json.RawMessage `json:"body"`
	Key    string          `json:"key"`
	On     *bool           `json:"on"`
	Icon   string          `json:"icon"`
	Seed   string          `json:"seed"`
	Color  string          `json:"color"`
	Image  string          `json:"image"`
	Kind   string          `json:"kind"`
	Need   string          `json:"need"`
}

type file struct {
	Name   string `json:"name"`
	Type   string `json:"type"`
	Base64 string `json:"base64"`
	Blob   string `json:"blob"`
}

// ownOnly are the messages only the client's own screens may send.
var ownOnly = map[string]bool{"api": true, "signOut": true, "appearance": true, "device": true, "request": true, "cache": true, "developer": true}

// Answer is a message's reply, as JSON, or the error the screen's promise fails with.
func (h *Host) Answer(raw []byte) (any, error) {
	var m message
	if err := json.Unmarshal(raw, &m); err != nil {
		return nil, err
	}
	own := h.Route.Own
	if ownOnly[m.Type] && !own {
		return nil, errors.New("Only Settings can do that.")
	}
	d := h.Device
	switch m.Type {
	case "call":
		if own {
			return nil, errors.New("Settings has no app to call.")
		}
		return d.Call(h.Route.App, m.Op, m.Input)
	case "kept":
		if own {
			return nil, nil
		}
		input := m.Input
		if len(input) == 0 {
			input = json.RawMessage("{}")
		}
		k := d.Kept(h.Route.App, m.Op, input)
		if k == nil {
			return nil, nil
		}
		return map[string]any{"answer": json.RawMessage(k.Answer), "at": k.At}, nil
	case "open":
		params := string(m.Params)
		if params == "" || params == "null" {
			params = "{}"
		}
		to := Route{App: h.Route.App, Screen: firstOf(m.Screen, "main"), Params: params, Scope: firstOf(m.Scope, h.Route.Scope)}
		switch {
		case m.App != "":
			to.App = m.App
		case own:
			to = Route{Own: true, Screen: m.Screen, Params: params}
		}
		return h.Shell.Open(to, m.Sheet)
	case "close":
		h.Shell.Close(m.Value)
		return nil, nil
	case "title":
		h.Shell.Title(m.Text, m.Name)
		return nil, nil
	case "menu":
		if len(m.Items) == 0 {
			return nil, nil
		}
		i, err := h.Shell.Menu(m.Items, m.X, m.Y, m.Rect)
		if err != nil || i < 0 {
			return nil, err
		}
		return i, nil
	case "alert":
		return h.Shell.Alert(m.Title, m.Text, m.Action, m.Danger)
	case "copy":
		return nil, h.Shell.Copy(m.Text)
	case "link":
		return nil, h.Shell.Link(m.URL)
	case "save", "preview":
		if m.File == nil {
			return nil, errors.New("No file to show.")
		}
		name, data, err := h.fileOf(m.File)
		if err != nil {
			return nil, err
		}
		if m.Type == "save" {
			return nil, h.Shell.Save(name, data)
		}
		return nil, h.Shell.Preview(name, data)
	case "notify":
		return h.Shell.Notify()
	case "scan":
		return nil, errors.New("Scanning documents needs the iPhone, iPad or Android app. Choose the files instead.")
	case "vault", "request":
		return nil, errors.New("The Vault's sheet isn’t in this app yet.")
	case "icons":
		all := iconPaths()
		out := map[string]string{}
		for _, n := range m.Names {
			if d, ok := all[n]; ok {
				out[n] = d
			}
		}
		return out, nil
	case "icon":
		return tile(m), nil
	case "haptic":
		return nil, nil
	case "traces":
		if t := d.Traces(); t != nil {
			return json.RawMessage(*t), nil
		}
		return nil, nil
	case "api":
		return d.API(firstOf(m.Method, "GET"), m.Path, m.Body)
	case "signOut":
		return nil, h.Shell.SignOut()
	case "appearance":
		return "automatic", nil
	case "device":
		return map[string]any{"name": h.Shell.Device(), "signOut": true}, nil
	case "developer":
		if m.On != nil {
			d.SetDeveloper(*m.On)
		}
		return d.Developer(), nil
	case "cache":
		return h.cache(m)
	}
	return nil, fmt.Errorf("Unknown message")
}

func firstOf(s ...string) string {
	for _, v := range s {
		if v != "" {
			return v
		}
	}
	return ""
}

func (h *Host) fileOf(f *file) (string, []byte, error) {
	name := firstOf(filepath.Base(f.Name), "file")
	if f.Base64 != "" {
		b, err := base64.StdEncoding.DecodeString(f.Base64)
		return name, b, err
	}
	if f.Blob != "" {
		r, err := h.Device.Send("GET", "/v1/blobs/"+f.Blob, nil, nil, "")
		if err != nil {
			return "", nil, err
		}
		b, err := r.Bytes()
		return name, b, err
	}
	return "", nil, errors.New("No file to show.")
}

func (h *Host) cache(m message) (any, error) {
	key := m.Key
	if len(key) == 0 || len(key) > 40 || strings.ContainsFunc(key, func(r rune) bool {
		return !(r == '-' || r >= '0' && r <= '9' || r >= 'a' && r <= 'z' || r >= 'A' && r <= 'Z')
	}) {
		return nil, nil
	}
	path := filepath.Join(h.Kept, key+".json")
	if m.Value == nil {
		b, err := os.ReadFile(path)
		if err != nil {
			return nil, nil
		}
		return json.RawMessage(b), nil
	}
	os.MkdirAll(h.Kept, 0o700)
	return nil, os.WriteFile(path, m.Value, 0o600)
}

// tile is an app's icon as the shells draw it: a finished icon as it is, a logo inset on white, or its icon filled
// (else its first letter) in white on its colour.
func tile(m message) string {
	if m.Image != "" && m.Kind == "icon" {
		return m.Image
	}
	esc := func(s string) string {
		return strings.NewReplacer("&", "&amp;", "<", "&lt;", ">", "&gt;", `"`, "&quot;").Replace(s)
	}
	var svg string
	if m.Image != "" {
		svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256"><rect width="256" height="256" rx="58" fill="#fff"/><image href="` + esc(m.Image) + `" x="36" y="36" width="184" height="184"/></svg>`
	} else {
		color := m.Color
		if color == "" {
			palette := []string{"#5e5ce6", "#ff9f0a", "#30b0c7", "#ff375f", "#34c759", "#af52de", "#0a84ff", "#a2845e"}
			var h uint32
			for _, c := range m.Seed {
				h = h*31 + uint32(c)
			}
			color = palette[h%uint32(len(palette))]
		}
		face := ""
		if d, ok := iconPaths()[m.Icon+"-fill"]; ok && m.Icon != "" {
			face = `<path transform="translate(64 64) scale(0.5)" fill="#fff" d="` + esc(d) + `"/>`
		} else {
			letter := "?"
			if m.Seed != "" {
				letter = strings.ToUpper(m.Seed[:1])
			}
			face = `<text x="128" y="128" dy="0.35em" text-anchor="middle" font-family="system-ui" font-weight="600" font-size="118" fill="#fff">` + esc(letter) + `</text>`
		}
		svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256"><rect width="256" height="256" rx="58" fill="` + esc(color) + `"/>` + face + `</svg>`
	}
	return "data:image/svg+xml;base64," + base64.StdEncoding.EncodeToString([]byte(svg))
}

// Shim is what a page needs before bridge.js runs, for a shell whose screens post each message to their own origin's
// `/one` and get its reply as the response (a custom scheme's handler calls Host.Answer), and hear node events by the
// shell's `on(name, fn)`: `window.__oneShell`, as Android's and Windows' web views put one.
const Shim = `(() => {
  window.__oneShell = { postMessage: async (message) => {
    const r = await fetch("/one", { method: "POST", body: JSON.stringify(message) });
    const m = await r.json();
    if (m.error !== undefined) throw new Error(m.error);
    return m.ok;
  } };
  window.mygo?.on?.("one-event", (e) => window.one?.__event(e));
})();`
