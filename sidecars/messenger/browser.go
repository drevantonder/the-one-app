package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"slices"
	"strings"
	"sync"
	"time"
)

// browser is the browser in a device borrowed from the agent-devices sidecar,
// through this device's node, starting with a copy of a browser profile
// signed in to messenger.com.
type browser struct {
	node    string
	profile string // the browser profile

	mu       sync.Mutex
	device string // the borrowed device; given back when idle
	session  string // the device's tools keep a browser in a session
	target   string
	tab      string
	used     time.Time
	dirty    bool // off the chat list: a conversation, or search results
}

const (
	home = "https://www.messenger.com/"
	// idle is how long a borrowed device is kept without a call.
	idle = 5 * time.Minute
)

var errNotSignedIn = errors.New("Messenger isn't signed in on that browser profile. Ask the person to sign in again: messenger_sign_in with the profile gives them a link, where they sign in at messenger.com (with Keep me signed in), then save the profile")

// devices runs one of Agent Devices' operations: those with a browser
// profile (whose Vault item only the app's calls may use) the app's, the rest
// its sidecar's, wherever it runs.
func (b *browser) devices(op string, input map[string]any) (any, error) {
	call := map[string]any{"app": "agent-devices", "op": op, "input": input}
	if !slices.Contains([]string{"borrow", "give_back", "profiles", "sign_in", "save_profile", "add_profile"}, op) {
		call["sidecar"] = "agent-devices"
	}
	body, _ := json.Marshal(call)
	client := &http.Client{Timeout: 3 * time.Minute, Transport: toNode}
	resp, err := client.Post(b.node+"/v1/call", "application/json", bytes.NewReader(body))
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	raw, _ := io.ReadAll(resp.Body)
	var reply struct {
		OK    any    `json:"ok"`
		Error string `json:"error"`
	}
	if err := json.Unmarshal(raw, &reply); err != nil {
		return nil, fmt.Errorf("agent devices %s: %s", op, raw)
	}
	if reply.Error != "" || resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("agent devices %s: %s", op, reply.Error)
	}
	return reply.OK, nil
}

// borrow takes a device with the profile, unless one is borrowed: one with
// no screen, since nobody watches it read.
func (b *browser) borrow() error {
	if b.device != "" {
		return nil
	}
	got, err := b.devices("borrow", map[string]any{"for": "Reading Messenger", "by": "messenger", "profile": b.profile, "screen": false})
	if err != nil {
		return err
	}
	m, _ := got.(map[string]any)
	b.device, _ = m["device"].(string)
	b.target, b.tab = "", ""
	b.session = fmt.Sprintf("messenger-%d", time.Now().UnixNano())
	if b.device == "" {
		return errors.New("couldn't borrow a device")
	}
	return nil
}

// giveBack returns the device once it's been idle a while; run under mu.
func (b *browser) giveBack() {
	if b.device == "" || time.Since(b.used) < idle {
		return
	}
	b.devices("give_back", map[string]any{"device": b.device})
	b.device = ""
}

// tool runs one of the device's tools and returns its structured answer,
// borrowing a device again if the last one was given back.
func (b *browser) tool(name string, in map[string]any) (map[string]any, error) {
	b.used = time.Now()
	out, err := b.try(name, in)
	// The device was given back, as after 15 minutes without a call.
	if gone := b.device; err != nil && gone != "" && strings.Contains(err.Error(), gone) {
		b.device = ""
		if name != "browser_prepare" {
			if err := b.open(); err != nil {
				return nil, err
			}
		}
		out, err = b.try(name, in)
	}
	return out, err
}

func (b *browser) try(name string, in map[string]any) (map[string]any, error) {
	if err := b.borrow(); err != nil {
		return nil, err
	}
	in["session"] = b.session
	var got any
	var err error
	if name == "list_windows" {
		got, err = b.devices("use", map[string]any{"device": b.device, "tool": name, "args": in})
	} else {
		in["device"] = b.device
		if b.target != "" && name != "browser_prepare" {
			in["target_id"], in["tab_id"] = b.target, b.tab
		}
		got, err = b.devices(name, in)
	}
	if err != nil {
		return nil, err
	}
	data := structured(got)
	if s, _ := data["status"].(string); s == "refused" {
		msg, _ := data["message"].(string)
		return nil, fmt.Errorf("%s refused: %s", name, msg)
	}
	return data, nil
}

// structured is a tool's structured answer: the agent-devices sidecar gives it as
// its content's last text.
func structured(got any) map[string]any {
	m, _ := got.(map[string]any)
	items, _ := m["content"].([]any)
	for i := len(items) - 1; i >= 0; i-- {
		item, _ := items[i].(map[string]any)
		text, _ := item["text"].(string)
		var out map[string]any
		if json.Unmarshal([]byte(text), &out) == nil {
			return out
		}
	}
	return map[string]any{}
}

// open starts the device's browser on the profile and attaches to its tab.
func (b *browser) open() error {
	b.target, b.tab = "", ""
	prepared, err := b.tool("browser_prepare", map[string]any{"allow_launch": true, "profile": map[string]any{"mode": "isolated_named", "name": b.profile}})
	if err != nil {
		return fmt.Errorf("starting the browser: %w", err)
	}
	pid, _ := prepared["prepared_pid"].(float64)
	windows, err := b.tool("list_windows", map[string]any{"pid": int(pid)})
	if err != nil {
		return err
	}
	ws, _ := windows["windows"].([]any)
	if len(ws) == 0 {
		return errors.New("the browser has no window")
	}
	w, _ := ws[0].(map[string]any)
	bound, err := b.tool("get_browser_state", map[string]any{"pid": int(pid), "window_id": w["window_id"]})
	if err != nil {
		return err
	}
	b.target, _ = bound["target_id"].(string)
	tabs, _ := bound["tabs"].([]any)
	for _, t := range tabs { // the active tab, or else the first
		if t, _ := t.(map[string]any); t["active"] == true || b.tab == "" {
			b.tab, _ = t["tab_id"].(string)
		}
	}
	if b.target == "" || b.tab == "" {
		return errors.New("couldn't attach to the browser")
	}
	b.dirty = true
	return nil
}

type ref struct {
	Ref  string `json:"ref"`
	Role string `json:"role"`
	Name string `json:"name"`
}

type page struct {
	url   string
	nodes []node
	parts [][]node // the page comes in parts; a row cut between two shows in both
	refs  []ref
}

func (p page) ref(role, name string) string {
	for _, r := range p.refs {
		if r.Role == role && r.Name == name {
			return r.Ref
		}
	}
	return ""
}

// signedOut says whether the page is Messenger's sign-in page.
func (p page) signedOut() bool {
	return p.ref("button", "Log In") != "" || p.ref("textbox", "Password") != "" || strings.Contains(p.url, "/login")
}

// chats are the chat list's rows, each once: a row cut between two parts of
// the page shows in both, each missing what the other has.
func (p page) chats() []Chat {
	var out []Chat
	at := map[string]int{}
	for _, part := range p.parts {
		for _, c := range parseChats(part) {
			i, seen := at[c.Name]
			if !seen {
				at[c.Name] = len(out)
				out = append(out, c)
				continue
			}
			link := cmpOr(out[i].link, c.link)
			if out[i].Time == "" && c.Time != "" {
				out[i] = c
			}
			out[i].link = link
		}
	}
	return out
}

func cmpOr(a, b string) string {
	if a != "" {
		return a
	}
	return b
}

// snapshot reads the page, or the parts matching query; it attaches again
// once if the browser closed (it does after a few idle minutes).
func (b *browser) snapshot(query string) (page, error) {
	var err error
	for attempt := range 2 {
		if b.target == "" || attempt > 0 {
			if err = b.open(); err != nil {
				return page{}, err
			}
		}
		in := map[string]any{"snapshot_format": "semantic_v2"}
		if query != "" {
			in["query"] = query
		}
		var p page
		outline := ""
		for range 6 {
			var data map[string]any
			if data, err = b.tool("get_browser_state", in); err != nil {
				break
			}
			raw, _ := json.Marshal(data)
			var part struct {
				Outline  string               `json:"outline"`
				Refs     []ref                `json:"refs"`
				Page     struct{ URL string } `json:"page"`
				Snapshot struct {
					Continuation string `json:"continuation"`
				} `json:"snapshot"`
			}
			json.Unmarshal(raw, &part)
			outline += part.Outline + "\n"
			p.parts = append(p.parts, parseOutline(part.Outline))
			p.refs = append(p.refs, part.Refs...)
			p.url = part.Page.URL
			if part.Snapshot.Continuation == "" {
				break
			}
			in["continuation"] = part.Snapshot.Continuation
		}
		if err == nil {
			p.nodes = parseOutline(outline)
			return p, nil
		}
	}
	return page{}, err
}

// list reads the chat list, loading it unless it's showing. Anything else
// is loaded over rather than read: the browser tools drop a page whose DOM
// nests too deeply, as some conversations' do, and are slow to say so.
func (b *browser) list() (page, error) {
	if b.target != "" && !b.dirty {
		if p, err := b.snapshot(""); err == nil && p.url == home && len(p.chats()) > 0 {
			return p, nil
		}
	}
	var p page
	for range 2 { // if the list doesn't show, load it once more
		if err := b.navigate(home); err != nil {
			return page{}, err
		}
		b.dirty = false
		for range 30 { // a browser that just started can take a while
			time.Sleep(500 * time.Millisecond)
			var err error
			if p, err = b.snapshot(""); err != nil {
				return page{}, err
			}
			if p.signedOut() {
				// Its copy of the profile stays signed out: borrow afresh next time, after the person signed in.
				b.devices("give_back", map[string]any{"device": b.device})
				b.device = ""
				return page{}, errNotSignedIn
			}
			if len(p.chats()) > 0 {
				return p, nil
			}
		}
	}
	return p, errors.New("Messenger's chat list didn't show")
}

// navigate loads url, attaching again once if the browser closed.
func (b *browser) navigate(url string) error {
	if b.target == "" {
		if err := b.open(); err != nil {
			return err
		}
	}
	if _, err := b.tool("browser_navigate", map[string]any{"url": url}); err == nil {
		return nil
	}
	if err := b.open(); err != nil {
		return err
	}
	_, err := b.tool("browser_navigate", map[string]any{"url": url})
	return err
}

func (b *browser) click(p page, role, name string) error {
	r := p.ref(role, name)
	if r == "" {
		return fmt.Errorf("%s %q isn't on the page", role, name)
	}
	_, err := b.tool("browser_click", map[string]any{"ref": r, "input_route": "dom_event"})
	return err
}

// markUnread marks a chat unread again from its menu in the chat list.
func (b *browser) markUnread(chat string) error {
	p, err := b.snapshot(menuPrefix + chat)
	if err != nil {
		return err
	}
	if err := b.click(p, "button", menuPrefix+chat); err != nil {
		return err
	}
	for range 10 {
		time.Sleep(300 * time.Millisecond)
		if p, err = b.snapshot("Mark as unread"); err != nil {
			return err
		}
		if p.ref("menuitem", "Mark as unread") != "" {
			return b.click(p, "menuitem", "Mark as unread")
		}
	}
	return errors.New("its menu had no Mark as unread")
}
