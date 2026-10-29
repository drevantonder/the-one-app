// Package spaces is the built-in plugin that gives agents computers of their
// own: Linux desktops, each in a container, with a browser and apps, that
// people watch and take over in a web viewer. It runs where Podman (Linux)
// or Apple's container (Mac) is installed; other devices reach it through
// the mesh.
//
// The desktops are Cua's (github.com/trycua/cua), driven by the server
// inside each one, cua-spacesd: its MCP tools become this plugin's
// operations, with a space argument once there are several.
package spaces

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	goruntime "runtime"
	"slices"
	"strings"
	"sync"
	"text/tabwriter"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/drevantonder/the-one-app/sdk"
)

var Plugin = sdk.Plugin{ID: "spaces", Name: "Spaces", Version: "0.1.0", Platforms: []string{"linux", "darwin"}, Setup: setup}

type nameIn struct {
	Name string `json:"name" jsonschema:"the Space's name"`
}

type backupIn struct {
	Name string `json:"name" jsonschema:"the Space's name"`
	On   bool   `json:"on" jsonschema:"true to back it up from now on, false to stop and delete its backups"`
}

type viewIn struct {
	Name    string `json:"name" jsonschema:"the Space's name"`
	Minutes int    `json:"minutes,omitempty" jsonschema:"how long the link works; default 60, at most 1440"`
}

// own are the plugin's operations; a Space's tools with these names are
// left out.
var own = []string{"create", "list", "start", "stop", "delete", "view", "backup", "release", "refresh_tools", "borrow", "sign_in", "give_back", "logins", "forget_login", "room", "screen", "page", "page_settings", "menubar", "finish_sign_in", "open_sign_in"}

func setup(c *sdk.Ctx) error {
	var err error
	if goruntime.GOOS == "darwin" {
		rt, err = newApple()
	} else {
		rt, err = newPodman()
	}
	if err != nil {
		return err
	}
	t := &tools{ctx: c, sessions: map[string]*mcp.ClientSession{}, cleaned: map[string]bool{}}
	var mu sync.Mutex
	hosts := map[string]string{} // backed-up Space → the device holding it
	holder := func(name string) string {
		mu.Lock()
		defer mu.Unlock()
		return hosts[name]
	}
	c.Entry("hosted", sdk.Available)
	c.Entry("released", sdk.Available)
	c.OnReset(func() { mu.Lock(); clear(hosts); mu.Unlock() })
	sdk.Handle(c, "hosted", func(_ sdk.Entry, in hosted) error { mu.Lock(); hosts[in.Name] = in.Host; mu.Unlock(); return nil })
	sdk.Handle(c, "released", func(_ sdk.Entry, in released) error { mu.Lock(); delete(hosts, in.Name); mu.Unlock(); return nil })
	host := func(call *sdk.Call, name string) error {
		me, err := tailnetName(call.Context)
		if err != nil {
			return err
		}
		if holder(name) != me {
			_, err = call.Append("hosted", hosted{Name: name, Host: me})
		}
		return err
	}
	held := func(name string) bool { // backed up: this device holds it
		me, err := tailnetName(context.Background())
		return err == nil && holder(name) == me
	}
	// Every Space is recorded with the device it is on, so any device can
	// list it and pass calls about it on to that device.
	where := map[string]string{} // Space → its device's tailnet name
	located := func(name string) string {
		mu.Lock()
		defer mu.Unlock()
		return where[name]
	}
	c.Entry("located", sdk.Available)
	c.Entry("gone", sdk.Available)
	c.OnReset(func() { mu.Lock(); clear(where); mu.Unlock() })
	sdk.Handle(c, "located", func(_ sdk.Entry, in hosted) error { mu.Lock(); where[in.Name] = in.Host; mu.Unlock(); return nil })
	sdk.Handle(c, "gone", func(_ sdk.Entry, in released) error { mu.Lock(); delete(where, in.Name); mu.Unlock(); return nil })
	// locate records this device's Spaces as they are.
	locate := func(call *sdk.Call) {
		me, err := tailnetName(call.Context)
		if err != nil {
			return
		}
		spaces, err := list(call.Context)
		if err != nil {
			return
		}
		here := map[string]bool{}
		for _, s := range spaces {
			here[s.Name] = true
			if located(s.Name) != me {
				_, _ = call.Append("located", hosted{Name: s.Name, Host: me})
			}
		}
		mu.Lock()
		var gone []string
		for name, h := range where {
			if h == me && !here[name] {
				gone = append(gone, name)
			}
		}
		mu.Unlock()
		for _, name := range gone {
			_, _ = call.Append("gone", released{Name: name})
		}
	}
	// elsewhere runs op on the device a Space is on, when that isn't this
	// one.
	t.elsewhere = func(call *sdk.Call, name, op string, in any) (sdk.Result, bool, error) {
		if _, err := find(call.Context, name); err == nil {
			return sdk.Result{}, false, nil
		}
		me, _ := tailnetName(call.Context)
		h := located(name)
		if h == "" || h == me {
			return sdk.Result{}, false, nil
		}
		raw, _ := json.Marshal(in)
		var args map[string]any
		_ = json.Unmarshal(raw, &args)
		if args == nil {
			args = map[string]any{}
		}
		args["device"] = strings.Split(h, ".")[0]
		res, err := call.Invoke("spaces", op, args)
		return res, true, err
	}
	t.names = func() []string {
		mu.Lock()
		defer mu.Unlock()
		var names []string
		for n := range where {
			names = append(names, n)
		}
		slices.Sort(names)
		return names
	}
	// Spaces stopped with stop stay stopped; others are started when the
	// node starts, as after a reboot.
	down := map[string]bool{}
	isDown := func(name string) bool { mu.Lock(); defer mu.Unlock(); return down[name] }
	c.Entry("stopped", sdk.Available)
	c.Entry("started", sdk.Available)
	c.OnReset(func() { mu.Lock(); clear(down); mu.Unlock() })
	sdk.Handle(c, "stopped", func(_ sdk.Entry, in released) error { mu.Lock(); down[in.Name] = true; mu.Unlock(); return nil })
	sdk.Handle(c, "started", func(_ sdk.Entry, in released) error { mu.Lock(); delete(down, in.Name); mu.Unlock(); return nil })
	up := func(call *sdk.Call, name string) error {
		if !isDown(name) {
			return nil
		}
		_, err := call.Append("started", released{Name: name})
		return err
	}
	p := setupPool(c, t, located, locate)
	c.OnReady(func(call *sdk.Call) {
		locate(call)
		_ = t.refresh(call.Context)
		go keepBackedUp(call.Detached(), held)
		go func(call *sdk.Call) {
			if restart(call, isDown) {
				locate(call)
				_ = t.refresh(call.Context)
			}
		}(call.Detached())
	})

	sdk.Op(c, "create", "Create a Space: a Linux desktop of its own, with a browser, for agents to use. Sign in to sites in it with view. A Space that has backups, from this device or another, is restored from the newest.", sdk.Write, func(call *sdk.Call, in nameIn) (sdk.Result, error) {
		if strings.HasPrefix(in.Name, poolPrefix) {
			return sdk.Result{}, fmt.Errorf("%s names are the pool's: borrow one with borrow", poolPrefix)
		}
		saved, err := backups(call, in.Name)
		if err != nil && !noBucket(err) {
			return sdk.Result{}, err
		}
		if len(saved) > 0 {
			me, err := tailnetName(call.Context)
			if err != nil {
				return sdk.Result{}, err
			}
			if h := holder(in.Name); h != "" && h != me {
				return sdk.Result{}, fmt.Errorf("Space %q is on %s: release it there first, which backs it up and frees it", in.Name, h)
			}
		}
		if h := located(in.Name); h != "" {
			if me, _ := tailnetName(call.Context); h != me {
				return sdk.Result{}, fmt.Errorf("Space %q is on %s", in.Name, strings.Split(h, ".")[0])
			}
		}
		s, err := create(call.Context, in.Name)
		if err != nil {
			return sdk.Result{}, err
		}
		if err := up(call, s.Name); err != nil {
			return sdk.Result{}, err
		}
		defer func() { locate(call); _ = t.refresh(call.Context) }()
		if len(saved) == 0 {
			return sdk.Text(fmt.Sprintf("Created Space %q. Open it with view; back it up with backup.", s.Name)), nil
		}
		if err := restore(call, s, saved[0]); err != nil {
			return sdk.Result{}, fmt.Errorf("created Space %q, but restoring its backup failed: %w", s.Name, err)
		}
		if err := host(call, s.Name); err != nil {
			return sdk.Result{}, err
		}
		return sdk.Text(fmt.Sprintf("Restored Space %q from its backup of %s. It is backed up as before.", s.Name, saved[0].Modified.Local().Format(time.DateTime))), nil
	})
	sdk.Op(c, "list", "List the Spaces on every device, and backed-up ones on none.", sdk.Read, func(call *sdk.Call, _ struct{}) (sdk.Result, error) {
		spaces, err := list(call.Context)
		if err != nil {
			return sdk.Result{}, err
		}
		saved, _ := call.Files("") // without a bucket there are no backups
		latest := map[string]sdk.FileInfo{}
		for _, f := range saved {
			name, _, _ := strings.Cut(f.Name, "/")
			if strings.HasPrefix(name, "@") {
				continue
			}
			if f.Name > latest[name].Name {
				latest[name] = f
			}
		}
		var b strings.Builder
		w := tabwriter.NewWriter(&b, 0, 0, 2, ' ', 0)
		for i, s := range spaces {
			used := "never"
			if !s.LastUsed.IsZero() {
				used = s.LastUsed.Local().Format(time.DateTime)
			}
			backed := "not backed up"
			if spaces[i].BackedUp = held(s.Name); spaces[i].BackedUp {
				backed = "backed up"
				if f, ok := latest[s.Name]; ok {
					backed = fmt.Sprintf("backed up %s (%s)", f.Modified.Local().Format(time.DateTime), size(f.Size))
				}
			}
			delete(latest, s.Name)
			if st := p.status(s.Name); st != "" {
				backed = st
			}
			fmt.Fprintf(w, "%s\t%s\tlast started %s\t%s\n", s.Name, s.State, used, backed)
		}
		me, _ := tailnetName(call.Context)
		mu.Lock()
		var others []space
		for name, h := range where {
			if h != me && !slices.ContainsFunc(spaces, func(s space) bool { return s.Name == name }) {
				others = append(others, space{Name: name, State: "on " + strings.Split(h, ".")[0]})
			}
		}
		mu.Unlock()
		for _, o := range others {
			if _, ok := latest[o.Name]; ok {
				o.BackedUp = true
				delete(latest, o.Name)
			}
			fmt.Fprintf(w, "%s\t%s\t\t%s\n", o.Name, o.State, p.status(o.Name))
			spaces = append(spaces, o)
		}
		for name, f := range latest {
			where := "not on this device"
			if h := holder(name); h != "" {
				where = "on " + strings.Split(h, ".")[0]
			}
			fmt.Fprintf(w, "%s\t%s\t\tbacked up %s (%s); create it to restore it here\n", name, where, f.Modified.Local().Format(time.DateTime), size(f.Size))
			spaces = append(spaces, space{Name: name, State: where, BackedUp: true})
		}
		w.Flush()
		if len(spaces) == 0 {
			return sdk.Result{Text: "No Spaces. Make one with create.", Data: []space{}}, nil
		}
		return sdk.Result{Text: strings.TrimSpace(b.String()), Data: spaces}, nil
	})
	sdk.Op(c, "start", "Start a stopped Space.", sdk.Write, func(call *sdk.Call, in nameIn) (sdk.Result, error) {
		if res, ok, err := t.elsewhere(call, in.Name, "start", in); ok {
			return res, err
		}
		s, err := find(call.Context, in.Name)
		if err != nil {
			return sdk.Result{}, err
		}
		if err := rt.start(call.Context, s.Name); err != nil {
			return sdk.Result{}, err
		}
		if err := up(call, s.Name); err != nil {
			return sdk.Result{}, err
		}
		if err := ready(call.Context, s); err != nil {
			return sdk.Result{}, err
		}
		return sdk.Text("Started."), nil
	})
	sdk.Op(c, "stop", "Stop a Space. Its files stay; start it again any time. A backed-up Space is backed up first.", sdk.Write, func(call *sdk.Call, in nameIn) (sdk.Result, error) {
		if res, ok, err := t.elsewhere(call, in.Name, "stop", in); ok {
			return res, err
		}
		s, err := find(call.Context, in.Name)
		if err != nil {
			return sdk.Result{}, err
		}
		if s.State == "running" && held(s.Name) {
			if _, err := backup(call, s); err != nil {
				return sdk.Result{}, fmt.Errorf("not stopped: backing up failed: %w", err)
			}
		}
		t.drop(in.Name)
		if err := rt.stop(call.Context, s.Name); err != nil {
			return sdk.Result{}, err
		}
		if _, err := call.Append("stopped", released{Name: s.Name}); err != nil {
			return sdk.Result{}, err
		}
		return sdk.Text("Stopped."), nil
	})
	sdk.Op(c, "delete", "Delete a Space and everything in it, including its sign-ins and backups. To move a Space to another device instead, use release.", sdk.Write, func(call *sdk.Call, in nameIn) (sdk.Result, error) {
		if res, ok, err := t.elsewhere(call, in.Name, "delete", in); ok {
			return res, err
		}
		s, err := find(call.Context, in.Name)
		if err != nil {
			return sdk.Result{}, err
		}
		t.drop(s.Name)
		if err := remove(call.Context, s); err != nil {
			return sdk.Result{}, err
		}
		locate(call)
		_ = t.refresh(call.Context)
		if err := dropBackups(call, s.Name); err != nil {
			return sdk.Result{}, fmt.Errorf("deleted Space %q, but not all its backups: %w", s.Name, err)
		}
		if held(s.Name) {
			if _, err := call.Append("released", released{Name: s.Name}); err != nil {
				return sdk.Result{}, err
			}
		}
		return sdk.Text(fmt.Sprintf("Deleted Space %q.", s.Name)), nil
	})
	sdk.Op(c, "backup", "Turn a Space's backups on or off. Backed up, its home folder (without caches) is stored in the bucket, encrypted, when it stops and every 6 hours while it runs, keeping the last 3; it can then be restored on any device. Turning backups off deletes them.", sdk.Write, func(call *sdk.Call, in backupIn) (sdk.Result, error) {
		if res, ok, err := t.elsewhere(call, in.Name, "backup", in); ok {
			return res, err
		}
		s, err := find(call.Context, in.Name)
		if err != nil {
			return sdk.Result{}, err
		}
		if !in.On {
			if err := dropBackups(call, s.Name); err != nil {
				return sdk.Result{}, err
			}
			if held(s.Name) {
				if _, err := call.Append("released", released{Name: s.Name}); err != nil {
					return sdk.Result{}, err
				}
			}
			return sdk.Text(fmt.Sprintf("Space %q is no longer backed up, and its backups are deleted.", s.Name)), nil
		}
		if s.State != "running" {
			return sdk.Result{}, fmt.Errorf("Space %q is %s: start it first", s.Name, s.State)
		}
		if err := host(call, s.Name); err != nil {
			return sdk.Result{}, err
		}
		f, err := backup(call, s)
		if err != nil {
			return sdk.Result{}, err
		}
		return sdk.Text(fmt.Sprintf("Space %q is backed up (%s so far).", s.Name, size(f.Size))), nil
	})
	sdk.Op(c, "release", "Free a backed-up Space for another device: back it up one last time and remove it here, keeping its backups. Then create it on the other device to restore it there.", sdk.Write, func(call *sdk.Call, in nameIn) (sdk.Result, error) {
		if res, ok, err := t.elsewhere(call, in.Name, "release", in); ok {
			return res, err
		}
		s, err := find(call.Context, in.Name)
		if err != nil {
			return sdk.Result{}, err
		}
		if !held(s.Name) {
			return sdk.Result{}, fmt.Errorf("Space %q isn't backed up: turn backups on first, or it would be lost", s.Name)
		}
		if s.State != "running" {
			return sdk.Result{}, fmt.Errorf("Space %q is %s: start it first, so its last changes are backed up", s.Name, s.State)
		}
		if _, err := backup(call, s); err != nil {
			return sdk.Result{}, err
		}
		t.drop(s.Name)
		if err := remove(call.Context, s); err != nil {
			return sdk.Result{}, err
		}
		locate(call)
		_ = t.refresh(call.Context)
		if _, err := call.Append("released", released{Name: s.Name}); err != nil {
			return sdk.Result{}, err
		}
		return sdk.Text(fmt.Sprintf("Released Space %q: create it on another device to restore it there.", s.Name)), nil
	})
	sdk.Op(c, "view", "Get a link to watch and use a Space's desktop in a browser, from any of your devices: to sign in to sites, or to see what an agent is doing.", sdk.Read, func(call *sdk.Call, in viewIn) (sdk.Result, error) {
		if res, ok, err := t.elsewhere(call, in.Name, "view", in); ok {
			return res, err
		}
		s, err := find(call.Context, in.Name)
		if err != nil {
			return sdk.Result{}, err
		}
		if s.State != "running" {
			return sdk.Result{}, fmt.Errorf("Space %q is %s: start it first", s.Name, s.State)
		}
		minutes := min(max(in.Minutes, 1), 1440)
		if in.Minutes == 0 {
			minutes = 60
		}
		url, err := viewURL(call.Context, s, minutes)
		if err != nil {
			return sdk.Result{}, err
		}
		return sdk.Result{Text: fmt.Sprintf("%s\nThe link works for %d minutes.", url, minutes), Data: map[string]any{"url": url, "minutes": minutes}}, nil
	})
	sdk.Op(c, "screen", "A small picture of a Space's desktop, as it is now.", sdk.Read, func(call *sdk.Call, in screenIn) (sdk.Result, error) {
		img, err := t.screen(call, in)
		if err != nil {
			return sdk.Result{}, err
		}
		return sdk.Result{Text: "The desktop of Space " + in.Space + ".", Images: []sdk.Image{img}}, nil
	})
	sdk.Op(c, "refresh_tools", "Reload the tools Spaces offer.", sdk.Write, func(call *sdk.Call, _ struct{}) (sdk.Result, error) {
		if err := t.refresh(call.Context); err != nil {
			return sdk.Result{}, err
		}
		return sdk.Text(fmt.Sprintf("%d tools.", len(t.ops))), nil
	})
	return nil
}

// viewURL is a link to the Space's viewer that works for minutes.
func viewURL(ctx context.Context, s space, minutes int) (string, error) {
	path, err := viewerPath(ctx, s, minutes*60)
	if err != nil {
		return "", err
	}
	host, err := tailnetName(ctx)
	if err != nil {
		return "", err
	}
	return fmt.Sprintf("https://%s:%d%s", host, s.port, path), nil
}

// tools imports the MCP tools of the server in every Space. Each Space
// offers the same tools, so they are listed from any running one.
type tools struct {
	ctx *sdk.Ctx
	// elsewhere passes a call on to the device a Space is on; names are
	// every device's Spaces.
	elsewhere func(call *sdk.Call, name, op string, in any) (sdk.Result, bool, error)
	names     func() []string
	used      func(name string) // a Space's tool was called on this device

	mu       sync.Mutex
	sessions map[string]*mcp.ClientSession // by Space
	cleaned  map[string]bool               // Spaces whose leftover browsers are closed
	ops      []string
}

func (t *tools) refresh(ctx context.Context) error {
	spaces, err := list(ctx)
	if err != nil {
		return err
	}
	var listed []*mcp.Tool
	for _, s := range spaces {
		if s.State != "running" {
			continue
		}
		session, err := t.connect(ctx, s)
		if err != nil {
			t.ctx.Logf("space %s: %v", s.Name, err)
			continue
		}
		for tool, err := range session.Tools(ctx, nil) {
			if err != nil {
				return err
			}
			listed = append(listed, tool)
		}
		break
	}
	names := t.names()
	for _, s := range spaces {
		if !slices.Contains(names, s.Name) {
			names = append(names, s.Name)
		}
	}
	t.register(listed, names)
	return nil
}

// register makes the tools operations. With no Spaces there are none; the
// space argument is required once there are several.
func (t *tools) register(listed []*mcp.Tool, spaces []string) {
	t.mu.Lock()
	defer t.mu.Unlock()
	for _, n := range t.ops {
		t.ctx.RemoveOp(n)
	}
	t.ops = nil
	if len(spaces) > 0 {
		for _, tool := range listed {
			if slices.Contains(own, tool.Name) {
				continue
			}
			var schema map[string]any
			raw, _ := json.Marshal(tool.InputSchema)
			_ = json.Unmarshal(raw, &schema)
			if schema == nil {
				schema = map[string]any{"type": "object"}
			}
			props, _ := schema["properties"].(map[string]any)
			if props == nil {
				props = map[string]any{}
				schema["properties"] = props
			}
			props["space"] = map[string]any{"type": "string", "enum": spaces, "description": "the Space to use"}
			if len(spaces) > 1 {
				req, _ := schema["required"].([]any)
				schema["required"] = append(req, "space")
			}
			raw, _ = json.Marshal(schema)
			kind := sdk.Write
			if tool.Annotations != nil && tool.Annotations.ReadOnlyHint {
				kind = sdk.Read
			}
			name := tool.Name
			t.ctx.OpRaw(name, "In a Space: "+tool.Description, raw, kind, func(call *sdk.Call, input json.RawMessage) (sdk.Result, error) {
				return t.call(call, name, input)
			})
			t.ops = append(t.ops, name)
		}
	}
	t.ctx.Changed()
}

func (t *tools) call(call *sdk.Call, tool string, input json.RawMessage) (sdk.Result, error) {
	return t.callAs(call, tool, input, true)
}

// callAs runs a Space's tool; one that isn't use, like a thumbnail, leaves
// the Space as idle as it was.
func (t *tools) callAs(call *sdk.Call, tool string, input json.RawMessage, use bool) (sdk.Result, error) {
	ctx := call.Context
	args := map[string]any{}
	if len(input) > 0 && string(input) != "null" {
		if err := json.Unmarshal(input, &args); err != nil {
			return sdk.Result{}, fmt.Errorf("bad input: %w", err)
		}
	}
	name, _ := args["space"].(string)
	if name == "" {
		names := t.names()
		if len(names) != 1 {
			return sdk.Result{}, errors.New("which Space? pass space")
		}
		name = names[0]
		args["space"] = name
	}
	if !use {
		if _, err := find(ctx, name); err != nil {
			raw, _ := json.Marshal(map[string]any{"space": name, "width": args["max_image_dimension"]})
			if res, ok, err := t.elsewhere(call, name, "screen", json.RawMessage(raw)); ok {
				return res, err
			}
		}
	}
	if res, ok, err := t.elsewhere(call, name, tool, args); ok {
		return res, err
	}
	delete(args, "space")
	if t.used != nil && use {
		t.used(name)
	}
	s, err := find(ctx, name)
	if err != nil {
		return sdk.Result{}, err
	}
	if s.State != "running" {
		return sdk.Result{}, fmt.Errorf("Space %q is %s: start it first", s.Name, s.State)
	}
	var res *mcp.CallToolResult
	for attempt := range 2 { // a dropped session, as after a restart, is reconnected once
		if attempt > 0 {
			t.drop(s.Name)
		}
		var session *mcp.ClientSession
		if session, err = t.connect(ctx, s); err != nil {
			continue
		}
		if res, err = session.CallTool(ctx, &mcp.CallToolParams{Name: tool, Arguments: args}); err != nil {
			continue
		}
		if tool == "browser_prepare" && attempt == 0 && strings.Contains(fmt.Sprint(res.StructuredContent), "browser_endpoint_owner_mismatch") {
			// The profile is open in a browser left by an earlier connection.
			closeBrowsers(ctx, s, false)
			if res, err = session.CallTool(ctx, &mcp.CallToolParams{Name: tool, Arguments: args}); err != nil {
				continue
			}
		}
		break
	}
	if err != nil {
		return sdk.Result{}, err
	}
	return sdk.MCPResult(res)
}

// closeBrowsers closes the browsers the Space's tools launched, and with
// signIns the one a sign-in opened too. Each of the tools' belongs to the
// connection that launched it: once that connection is gone (this plugin
// restarted or reconnected), the tools can neither use nor close it, and it
// holds its profile. The profiles are then marked as closed normally (the
// tools end a connection's browsers the same way), or the browsers would
// offer to restore their pages when next started.
func closeBrowsers(ctx context.Context, s space, signIns bool) {
	const profiles = home + "/.local/state/cua-driver/browser-profiles/"
	keep := signInClass
	if signIns {
		keep = "--matches-no-browser"
	}
	_ = rt.exec(ctx, s.Name, nil, nil, "sh", "-c", `
main() { # each browser's own process, not its helpers
	for p in $(pgrep -f -- '[-]-user-data-dir=`+profiles+`'); do
		c=$(tr '\0' ' ' </proc/$p/cmdline 2>/dev/null) || continue
		case "$c" in *--type=*|*`+keep+`*) ;; *) echo $p ;; esac
	done
}
pids=$(main)
[ -n "$pids" ] && kill $pids
for i in $(seq 50); do [ -z "$(main)" ] && break; sleep 0.1; done
sed -i 's/"exit_type":"Crashed"/"exit_type":"Normal"/' `+profiles+`*/Default/Preferences`)
}

// signInClass marks the browser a sign-in opens, which the tools didn't.
const signInClass = "--class=one-sign-in"

func (t *tools) connect(ctx context.Context, s space) (*mcp.ClientSession, error) {
	t.mu.Lock()
	defer t.mu.Unlock()
	if session := t.sessions[s.Name]; session != nil {
		return session, nil
	}
	if !t.cleaned[s.Name] {
		closeBrowsers(ctx, s, false)
		t.cleaned[s.Name] = true
	}
	transport := &mcp.StreamableClientTransport{Endpoint: s.local() + "/mcp", HTTPClient: &http.Client{Transport: bearer(s.token)}}
	client := mcp.NewClient(&mcp.Implementation{Name: "the-one-app/spaces", Version: "1"}, nil)
	session, err := client.Connect(ctx, transport, nil)
	if err != nil {
		return nil, err
	}
	t.sessions[s.Name] = session
	return session, nil
}

// drop forgets a Space's connection; the next one closes the browsers
// this one launched.
func (t *tools) drop(name string) {
	t.mu.Lock()
	defer t.mu.Unlock()
	if s := t.sessions[name]; s != nil {
		_ = s.Close()
		delete(t.sessions, name)
	}
	delete(t.cleaned, name)
}

type bearer string

func (b bearer) RoundTrip(req *http.Request) (*http.Response, error) {
	req = req.Clone(req.Context())
	req.Header.Set("Authorization", "Bearer "+string(b))
	return http.DefaultTransport.RoundTrip(req)
}
