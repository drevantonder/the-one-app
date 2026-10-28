// Package spaces is the built-in plugin that gives agents computers of their
// own: Linux desktops, each in a container, with a browser and apps, that
// people watch and take over in a web viewer. It runs where Incus is
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
	"os/exec"
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

type viewIn struct {
	Name    string `json:"name" jsonschema:"the Space's name"`
	Minutes int    `json:"minutes,omitempty" jsonschema:"how long the link works; default 60, at most 1440"`
}

// own are the plugin's operations; a Space's tools with these names are
// left out.
var own = []string{"create", "list", "start", "stop", "delete", "view", "refresh_tools"}

func setup(c *sdk.Ctx) error {
	if _, err := exec.LookPath("incus"); err != nil {
		return errors.New("needs Incus (https://linuxcontainers.org/incus)")
	}
	t := &tools{ctx: c, sessions: map[string]*mcp.ClientSession{}}
	c.OnReady(func(call *sdk.Call) { _ = t.refresh(call.Context) })

	sdk.Op(c, "create", "Create a Space: a Linux desktop of its own, with a browser, for agents to use. Sign in to sites in it with view.", sdk.Write, func(call *sdk.Call, in nameIn) (sdk.Result, error) {
		s, err := create(call.Context, in.Name)
		if err != nil {
			return sdk.Result{}, err
		}
		_ = t.refresh(call.Context)
		return sdk.Text(fmt.Sprintf("Created Space %q. Open it with view.", s.Name)), nil
	})
	sdk.Op(c, "list", "List the Spaces on this device.", sdk.Read, func(call *sdk.Call, _ struct{}) (sdk.Result, error) {
		spaces, err := list(call.Context)
		if err != nil {
			return sdk.Result{}, err
		}
		if len(spaces) == 0 {
			return sdk.Result{Text: "No Spaces. Make one with create.", Data: []space{}}, nil
		}
		var b strings.Builder
		w := tabwriter.NewWriter(&b, 0, 0, 2, ' ', 0)
		for _, s := range spaces {
			used := "never"
			if !s.LastUsed.IsZero() {
				used = s.LastUsed.Local().Format(time.DateTime)
			}
			fmt.Fprintf(w, "%s\t%s\tlast started %s\n", s.Name, s.State, used)
		}
		w.Flush()
		return sdk.Result{Text: strings.TrimSpace(b.String()), Data: spaces}, nil
	})
	sdk.Op(c, "start", "Start a stopped Space.", sdk.Write, func(call *sdk.Call, in nameIn) (sdk.Result, error) {
		s, err := find(call.Context, in.Name)
		if err != nil {
			return sdk.Result{}, err
		}
		if _, err := incus(call.Context, "start", prefix+s.Name); err != nil && !strings.Contains(err.Error(), "already running") {
			return sdk.Result{}, err
		}
		if err := ready(call.Context, s); err != nil {
			return sdk.Result{}, err
		}
		return sdk.Text("Started."), nil
	})
	sdk.Op(c, "stop", "Stop a Space. Its files stay; start it again any time.", sdk.Write, func(call *sdk.Call, in nameIn) (sdk.Result, error) {
		if _, err := find(call.Context, in.Name); err != nil {
			return sdk.Result{}, err
		}
		t.drop(in.Name)
		if _, err := incus(call.Context, "stop", prefix+in.Name); err != nil && !strings.Contains(err.Error(), "already stopped") {
			return sdk.Result{}, err
		}
		return sdk.Text("Stopped."), nil
	})
	sdk.Op(c, "delete", "Delete a Space and everything in it, including its sign-ins.", sdk.Write, func(call *sdk.Call, in nameIn) (sdk.Result, error) {
		s, err := find(call.Context, in.Name)
		if err != nil {
			return sdk.Result{}, err
		}
		t.drop(s.Name)
		if err := remove(call.Context, s); err != nil {
			return sdk.Result{}, err
		}
		_ = t.refresh(call.Context)
		return sdk.Text(fmt.Sprintf("Deleted Space %q.", s.Name)), nil
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
		if res, err = session.CallTool(ctx, &mcp.CallToolParams{Name: tool, Arguments: args}); err == nil {
			break
		}
	}
	if err != nil {
		return sdk.Result{}, err
	}
	return sdk.MCPResult(res)
}

func (t *tools) connect(ctx context.Context, s space) (*mcp.ClientSession, error) {
	t.mu.Lock()
	defer t.mu.Unlock()
	if session := t.sessions[s.Name]; session != nil {
		return session, nil
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

func (t *tools) drop(name string) {
	t.mu.Lock()
	defer t.mu.Unlock()
	if s := t.sessions[name]; s != nil {
		_ = s.Close()
		delete(t.sessions, name)
	}
}

type bearer string

func (b bearer) RoundTrip(req *http.Request) (*http.Response, error) {
	req = req.Clone(req.Context())
	req.Header.Set("Authorization", "Bearer "+string(b))
	return http.DefaultTransport.RoundTrip(req)
}
