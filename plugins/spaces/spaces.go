// Package spaces is the built-in plugin that gives agents computers of their
// own: Linux desktops, each in a container, with a browser and apps, that
// people watch and take over in a web viewer. It runs where Podman is
// installed; other devices reach it through the mesh.
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
	"slices"
	"strings"
	"sync"
	"text/tabwriter"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/drevantonder/the-one-app/sdk"
)

var Plugin = sdk.Plugin{ID: "spaces", Version: "0.1.0", Platforms: []string{"linux"}, Setup: setup}

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
var own = []string{"create", "list", "start", "stop", "delete", "view", "backup", "release", "refresh_tools"}

func setup(c *sdk.Ctx) error {
	var err error
	if rt, err = newPodman(); err != nil {
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
	c.OnReady(func(call *sdk.Call) {
		_ = t.refresh(call.Context)
		go keepBackedUp(call.Detached(), held)
	})

	sdk.Op(c, "create", "Create a Space: a Linux desktop of its own, with a browser, for agents to use. Sign in to sites in it with view. A Space that has backups, from this device or another, is restored from the newest.", sdk.Write, func(call *sdk.Call, in nameIn) (sdk.Result, error) {
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
		s, err := create(call.Context, in.Name)
		if err != nil {
			return sdk.Result{}, err
		}
		defer func() { _ = t.refresh(call.Context) }()
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
	sdk.Op(c, "list", "List the Spaces on this device.", sdk.Read, func(call *sdk.Call, _ struct{}) (sdk.Result, error) {
		spaces, err := list(call.Context)
		if err != nil {
			return sdk.Result{}, err
		}
		saved, _ := call.Files("") // without a bucket there are no backups
		latest := map[string]sdk.FileInfo{}
		for _, f := range saved {
			name, _, _ := strings.Cut(f.Name, "/")
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
			fmt.Fprintf(w, "%s\t%s\tlast started %s\t%s\n", s.Name, s.State, used, backed)
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
		s, err := find(call.Context, in.Name)
		if err != nil {
			return sdk.Result{}, err
		}
		if err := rt.start(call.Context, s.Name); err != nil {
			return sdk.Result{}, err
		}
		if err := ready(call.Context, s); err != nil {
			return sdk.Result{}, err
		}
		return sdk.Text("Started."), nil
	})
	sdk.Op(c, "stop", "Stop a Space. Its files stay; start it again any time. A backed-up Space is backed up first.", sdk.Write, func(call *sdk.Call, in nameIn) (sdk.Result, error) {
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
		return sdk.Text("Stopped."), nil
	})
	sdk.Op(c, "delete", "Delete a Space and everything in it, including its sign-ins and backups. To move a Space to another device instead, use release.", sdk.Write, func(call *sdk.Call, in nameIn) (sdk.Result, error) {
		s, err := find(call.Context, in.Name)
		if err != nil {
			return sdk.Result{}, err
		}
		t.drop(s.Name)
		if err := remove(call.Context, s); err != nil {
			return sdk.Result{}, err
		}
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
		_ = t.refresh(call.Context)
		if _, err := call.Append("released", released{Name: s.Name}); err != nil {
			return sdk.Result{}, err
		}
		return sdk.Text(fmt.Sprintf("Released Space %q: create it on another device to restore it there.", s.Name)), nil
	})
	sdk.Op(c, "view", "Get a link to watch and use a Space's desktop in a browser, from any of your devices: to sign in to sites, or to see what an agent is doing.", sdk.Read, func(call *sdk.Call, in viewIn) (sdk.Result, error) {
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
		path, err := viewerPath(call.Context, s, minutes*60)
		if err != nil {
			return sdk.Result{}, err
		}
		host, err := tailnetName(call.Context)
		if err != nil {
			return sdk.Result{}, err
		}
		url := fmt.Sprintf("https://%s:%d%s", host, s.port, path)
		return sdk.Result{Text: fmt.Sprintf("%s\nThe link works for %d minutes.", url, minutes), Data: map[string]any{"url": url, "minutes": minutes}}, nil
	})
	sdk.Op(c, "refresh_tools", "Reload the tools Spaces offer.", sdk.Write, func(call *sdk.Call, _ struct{}) (sdk.Result, error) {
		if err := t.refresh(call.Context); err != nil {
			return sdk.Result{}, err
		}
		return sdk.Text(fmt.Sprintf("%d tools.", len(t.names))), nil
	})
	return nil
}

// tools imports the MCP tools of the server in every Space. Each Space
// offers the same tools, so they are listed from any running one.
type tools struct {
	ctx *sdk.Ctx

	mu       sync.Mutex
	sessions map[string]*mcp.ClientSession // by Space
	cleaned  map[string]bool               // Spaces whose leftover browsers are closed
	names    []string
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
	var names []string
	for _, s := range spaces {
		names = append(names, s.Name)
	}
	t.register(listed, names)
	return nil
}

// register makes the tools operations. With no Spaces there are none; the
// space argument is required once there are several.
func (t *tools) register(listed []*mcp.Tool, spaces []string) {
	t.mu.Lock()
	defer t.mu.Unlock()
	for _, n := range t.names {
		t.ctx.RemoveOp(n)
	}
	t.names = nil
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
				return t.call(call.Context, name, input)
			})
			t.names = append(t.names, name)
		}
	}
	t.ctx.Changed()
}

func (t *tools) call(ctx context.Context, tool string, input json.RawMessage) (sdk.Result, error) {
	args := map[string]any{}
	if len(input) > 0 && string(input) != "null" {
		if err := json.Unmarshal(input, &args); err != nil {
			return sdk.Result{}, fmt.Errorf("bad input: %w", err)
		}
	}
	name, _ := args["space"].(string)
	delete(args, "space")
	if name == "" {
		spaces, err := list(ctx)
		if err != nil {
			return sdk.Result{}, err
		}
		if len(spaces) != 1 {
			return sdk.Result{}, errors.New("which Space? pass space")
		}
		name = spaces[0].Name
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
			closeBrowsers(ctx, s)
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

// closeBrowsers closes the browsers the Space's tools launched. Each belongs
// to the connection that launched it: once that connection is gone (this
// plugin restarted or reconnected), the tools can neither use nor close it,
// and it holds its profile.
func closeBrowsers(ctx context.Context, s space) {
	_ = rt.exec(ctx, s.Name, nil, nil, "pkill", "-f", "--", "--user-data-dir="+home+"/.local/state/cua-driver/browser-profiles/")
	time.Sleep(time.Second)
}

func (t *tools) connect(ctx context.Context, s space) (*mcp.ClientSession, error) {
	t.mu.Lock()
	defer t.mu.Unlock()
	if session := t.sessions[s.Name]; session != nil {
		return session, nil
	}
	if !t.cleaned[s.Name] {
		closeBrowsers(ctx, s)
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
