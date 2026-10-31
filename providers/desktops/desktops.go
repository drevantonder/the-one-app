package main

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"image"
	"image/jpeg"
	_ "image/png"
	"log"
	"os"
	"slices"
	"strconv"
	"strings"
	"sync"
	"time"
)

type desktops struct {
	cache string // the desktops' tools, kept for when none is running

	mu    sync.Mutex
	tools []tool

	connMu sync.Mutex
	conns  map[string]*conn // by desktop
}

func newDesktops(cache string) *desktops {
	ds := &desktops{cache: cache, conns: map[string]*conn{}}
	if b, err := os.ReadFile(cache); err == nil {
		json.Unmarshal(b, &ds.tools)
	}
	return ds
}

func (ds *desktops) describe() map[string]any {
	ops := ownOps()
	ds.mu.Lock()
	defer ds.mu.Unlock()
	for _, t := range ds.tools {
		if slices.Contains(browsing, t.Name) {
			ops = append(ops, map[string]any{"name": t.Name, "description": t.Description, "input": withDesktop(t.InputSchema)})
		}
	}
	return map[string]any{"title": "Desktops", "icon": "monitor", "instructions": instructions, "ops": ops, "widgets": []any{}}
}

// withDesktop adds the desktop argument to a tool's input.
func withDesktop(schema json.RawMessage) map[string]any {
	var s map[string]any
	json.Unmarshal(schema, &s)
	if s == nil {
		s = map[string]any{"type": "object"}
	}
	props, _ := s["properties"].(map[string]any)
	if props == nil {
		props = map[string]any{}
	}
	props["desktop"] = desktopArg
	s["properties"] = props
	return s
}

// refresh reads the tools from a running desktop: every desktop runs the
// same image, so any one will do.
func (ds *desktops) refresh(ctx context.Context) {
	all, err := list(ctx)
	if err != nil {
		log.Print(err)
		return
	}
	for _, d := range all {
		if d.State != "running" {
			continue
		}
		var tools []tool
		c, err := ds.conn(ctx, d)
		if err == nil {
			tools, err = c.tools(ctx)
		}
		if err != nil {
			log.Printf("reading %s's tools: %v", d.Name, err)
			continue
		}
		b, _ := json.Marshal(tools)
		ds.mu.Lock()
		ds.tools = tools
		ds.mu.Unlock()
		if err := os.WriteFile(ds.cache, b, 0o600); err != nil {
			log.Print(err)
		}
		return
	}
}

// conn is the desktop's session, made the first time it's needed. A browser
// an earlier session launched can't be used or closed by this one, and holds
// its profile, so those are closed first.
func (ds *desktops) conn(ctx context.Context, d desktop) (*conn, error) {
	ds.connMu.Lock()
	defer ds.connMu.Unlock()
	if c := ds.conns[d.Name]; c != nil {
		return c, nil
	}
	closeBrowsers(ctx, d)
	c, err := dial(ctx, d)
	if err != nil {
		return nil, err
	}
	ds.conns[d.Name] = c
	return c, nil
}

func (ds *desktops) drop(name string) {
	ds.connMu.Lock()
	delete(ds.conns, name)
	ds.connMu.Unlock()
}

// session is the label calls share unless they give their own: the browser
// tools refuse to act without one.
const session = "agents"

// call runs one of a desktop's tools, dialing again once if the connection is
// gone, as after the desktop restarted.
func (ds *desktops) call(ctx context.Context, d desktop, name string, args map[string]any) (result, error) {
	if _, ok := args["session"]; !ok && ds.takesSession(name) {
		args["session"] = session
	}
	var r result
	var err error
	for attempt := range 2 {
		if attempt > 0 {
			ds.drop(d.Name)
		}
		var c *conn
		if c, err = ds.conn(ctx, d); err != nil {
			continue
		}
		if r, err = c.call(ctx, name, args); err != nil {
			continue
		}
		if args["session"] == session && bytes.Contains(r.Structured, []byte(`"session_ended"`)) {
			// It ended, idle; only start_session revives a name.
			if _, err = c.call(ctx, "start_session", map[string]any{"session": session}); err != nil {
				break
			}
			r, err = c.call(ctx, name, args)
		}
		if name == "browser_prepare" && bytes.Contains(r.Structured, []byte("browser_endpoint_owner_mismatch")) {
			// The profile is open in a browser no session of ours owns.
			closeBrowsers(ctx, d)
			r, err = c.call(ctx, name, args)
		}
		break
	}
	return r, err
}

func (ds *desktops) takesSession(name string) bool {
	ds.mu.Lock()
	defer ds.mu.Unlock()
	for _, t := range ds.tools {
		if t.Name == name {
			var s struct{ Properties map[string]json.RawMessage }
			json.Unmarshal(t.InputSchema, &s)
			_, ok := s.Properties["session"]
			return ok
		}
	}
	return false
}

// closeBrowsers closes the browsers the desktop's tools launched, marking
// their profiles as closed normally, or they'd offer to restore their pages.
func closeBrowsers(ctx context.Context, d desktop) {
	err := sh(ctx, d, `profiles=`+home+`/.local/state/cua-driver/browser-profiles/
main() { # each browser's own process, not its helpers
	for p in $(pgrep -f -- "[-]-user-data-dir=$profiles"); do
		c=$(tr '\0' ' ' 2>/dev/null </proc/$p/cmdline) || continue
		case "$c" in *--type=*) ;; *) echo $p ;; esac
	done
}
pids=$(main)
[ -n "$pids" ] && kill $pids
for i in $(seq 50); do [ -z "$(main)" ] && break; sleep 0.1; done
sed -i 's/"exit_type":"Crashed"/"exit_type":"Normal"/' $profiles*/Default/Preferences 2>/dev/null
true`)
	if err != nil {
		log.Printf("closing %s's browsers: %v", d.Name, err)
	}
}

// content is a tool's answer for the node: its content, with its structured
// content as text too, since that often holds what its text leaves out.
func content(r result) (any, error) {
	items := r.Content
	if len(r.Structured) > 0 && string(r.Structured) != "null" {
		items = append(items, map[string]any{"type": "text", "text": string(r.Structured)})
	}
	if r.IsError {
		var texts []string
		for _, c := range items {
			if t, ok := c["text"].(string); ok {
				texts = append(texts, t)
			}
		}
		return nil, errors.New(strings.Join(texts, "\n"))
	}
	return map[string]any{"content": items}, nil
}

func (ds *desktops) run(ctx context.Context, op string, raw json.RawMessage) (any, error) {
	in := map[string]any{}
	if len(raw) > 0 && string(raw) != "null" {
		if err := json.Unmarshal(raw, &in); err != nil {
			return nil, fmt.Errorf("bad input: %w", err)
		}
	}
	name, _ := in["desktop"].(string)
	delete(in, "desktop")
	switch op {
	case "list":
		all, err := list(ctx)
		if err != nil || len(all) == 0 {
			return "No desktops. Make one with create.", err
		}
		var b strings.Builder
		for _, d := range all {
			started := ""
			if !d.Started.IsZero() {
				started = ", last started " + d.Started.Local().Format(time.DateTime)
			}
			fmt.Fprintf(&b, "%s: %s%s\n", d.Name, d.State, started)
		}
		return strings.TrimSpace(b.String()), nil
	case "create":
		d, err := create(ctx, name)
		if err != nil {
			return nil, err
		}
		ds.refresh(ctx)
		return fmt.Sprintf("Created desktop %s. Its browser is yours to sign in to sites with: open it with view.", d.Name), nil
	case "start":
		d, err := find(ctx, name)
		if err != nil || d.State == "running" {
			return "It's running.", err
		}
		ds.drop(d.Name)
		if err := start(ctx, d); err != nil {
			return nil, err
		}
		ds.refresh(ctx)
		return "Started.", nil
	case "stop":
		d, err := find(ctx, name)
		if err != nil {
			return nil, err
		}
		ds.drop(d.Name)
		return "Stopped. Its files and sign-ins stay.", stop(ctx, d)
	case "delete":
		d, err := find(ctx, name)
		if err != nil {
			return nil, err
		}
		ds.drop(d.Name)
		if err := remove(ctx, d); err != nil {
			return nil, err
		}
		return fmt.Sprintf("Deleted desktop %s and everything in it.", d.Name), nil
	case "view":
		d, err := running(ctx, name)
		if err != nil {
			return nil, err
		}
		minutes := 60
		if m, ok := in["minutes"].(float64); ok {
			minutes = min(max(int(m), 1), 1440)
		}
		url, err := viewURL(ctx, d, minutes)
		if err != nil {
			return nil, err
		}
		return fmt.Sprintf("%s\nThe link works for %d minutes, on the person's own devices.", url, minutes), nil
	case "screen":
		d, err := running(ctx, name)
		if err != nil {
			return nil, err
		}
		width := 640
		if w, ok := in["width"].(float64); ok {
			width = min(max(int(w), 160), 1920)
		}
		return ds.screen(ctx, d, width)
	case "tools":
		return ds.list(ctx, name, in["name"])
	case "use":
		tool, _ := in["tool"].(string)
		args, _ := in["args"].(map[string]any)
		return ds.use(ctx, name, tool, args)
	}
	if slices.Contains(browsing, op) {
		return ds.use(ctx, name, op, in)
	}
	return nil, fmt.Errorf("no operation %s", op)
}

func (ds *desktops) use(ctx context.Context, name, tool string, args map[string]any) (any, error) {
	if tool == "" {
		return nil, errors.New("which tool? desktops_tools lists them")
	}
	d, err := running(ctx, name)
	if err != nil {
		return nil, err
	}
	if args == nil {
		args = map[string]any{}
	}
	r, err := ds.call(ctx, d, tool, args)
	if err != nil {
		return nil, err
	}
	return content(r)
}

// list is the desktop's tools other than the browser's, or one in full.
func (ds *desktops) list(ctx context.Context, desktop string, name any) (any, error) {
	ds.mu.Lock()
	tools := ds.tools
	ds.mu.Unlock()
	if len(tools) == 0 {
		if _, err := running(ctx, desktop); err != nil {
			return nil, err
		}
		ds.refresh(ctx)
		ds.mu.Lock()
		tools = ds.tools
		ds.mu.Unlock()
	}
	if n, _ := name.(string); n != "" {
		for _, t := range tools {
			if t.Name == n {
				return t.Description + "\n\nInput: " + string(t.InputSchema), nil
			}
		}
		return nil, fmt.Errorf("no tool %s", n)
	}
	var b strings.Builder
	for _, t := range tools {
		if !slices.Contains(browsing, t.Name) {
			fmt.Fprintf(&b, "%s: %s\n", t.Name, gist(t.Description))
		}
	}
	return strings.TrimSpace(b.String()), nil
}

// gist is a description's first sentence.
func gist(s string) string {
	s, _, _ = strings.Cut(s, "\n")
	if i := strings.Index(s, ". "); i >= 0 {
		s = s[:i+1]
	}
	return s
}

// screen is a small JPEG of the desktop.
func (ds *desktops) screen(ctx context.Context, d desktop, width int) (any, error) {
	r, err := ds.call(ctx, d, "get_desktop_state", map[string]any{"max_image_dimension": width})
	if err != nil {
		return nil, err
	}
	for _, c := range r.Content {
		data, _ := c["data"].(string)
		if c["type"] != "image" || data == "" {
			continue
		}
		raw, err := base64.StdEncoding.DecodeString(data)
		if err != nil {
			return nil, err
		}
		img, _, err := image.Decode(bytes.NewReader(raw))
		if err != nil {
			return nil, err
		}
		var b bytes.Buffer
		if err := jpeg.Encode(&b, img, &jpeg.Options{Quality: 70}); err != nil {
			return nil, err
		}
		return map[string]any{"content": []map[string]any{
			{"type": "image", "mimeType": "image/jpeg", "data": base64.StdEncoding.EncodeToString(b.Bytes())},
			{"type": "text", "text": "Desktop " + d.Name + " now."},
		}}, nil
	}
	return nil, errors.New("the desktop sent no picture")
}

// viewURL is a link to the desktop's viewer that works for minutes, served
// over HTTPS on the tailnet: browsers only play its video on secure pages.
func viewURL(ctx context.Context, d desktop, minutes int) (string, error) {
	port := strconv.Itoa(d.port)
	if _, err := tailscale(ctx, "serve", "--bg", "--https="+port, "http://127.0.0.1:"+port); err != nil {
		return "", err
	}
	out, err := tailscale(ctx, "status", "--json")
	if err != nil {
		return "", err
	}
	var st struct{ Self struct{ DNSName string } }
	if err := json.Unmarshal([]byte(out), &st); err != nil {
		return "", err
	}
	path, err := viewerPath(ctx, d, minutes*60)
	if err != nil {
		return "", err
	}
	return fmt.Sprintf("https://%s:%s%s", strings.TrimSuffix(st.Self.DNSName, "."), port, path), nil
}
