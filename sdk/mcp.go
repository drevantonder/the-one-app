package sdk

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"os/exec"
	"regexp"
	"strings"
	"sync"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// MCPServer is a remote MCP server (URL) or a local one started as a
// command, whose tools a plugin imports as its own operations.
type MCPServer struct {
	URL     string            `json:"url,omitempty"`
	Command []string          `json:"command,omitempty"`
	Headers map[string]string `json:"headers,omitempty"`
}

// ImportMCP makes the server's tools the plugin's operations. With
// accounts, it connects once signed in, as each account; with several, each
// operation takes an account.
func ImportMCP(c *Ctx, server MCPServer, accounts *Accounts) {
	im := &mcpImport{ctx: c, server: server, accounts: accounts, sessions: map[string]*mcp.ClientSession{}}
	c.OnReady(func(call *Call) { im.refresh(call.Context) })
	if accounts != nil {
		accounts.OnChange(func(call *Call) {
			if im.listed() {
				im.register()
			} else {
				im.refresh(call.Context)
			}
		})
	}
	Op(c, "refresh_tools", "Reconnect to the MCP server and reload its tools.", Write, func(call *Call, _ noInput) (Result, error) {
		if err := im.refresh(call.Context); err != nil {
			return Result{}, err
		}
		return Text(fmt.Sprintf("%d tools.", len(im.names))), nil
	})
}

type mcpImport struct {
	ctx      *Ctx
	server   MCPServer
	accounts *Accounts

	mu       sync.Mutex
	sessions map[string]*mcp.ClientSession // by account; "" without accounts
	tools    []*mcp.Tool
	names    []string
}

var toolName = regexp.MustCompile(`[^a-zA-Z0-9_-]+`)

// refresh lists the server's tools and swaps them in as operations.
func (im *mcpImport) refresh(ctx context.Context) error {
	account := ""
	if im.accounts != nil {
		names := im.accounts.Names()
		if len(names) == 0 {
			return errors.New("not signed in: use sign_in first")
		}
		account = names[0] // every account sees the same tools
	}
	session, err := im.connect(ctx, account, true)
	if err != nil {
		im.ctx.Logf("mcp: %v", err)
		return err
	}
	var tools []*mcp.Tool
	for t, err := range session.Tools(ctx, nil) {
		if err != nil {
			return err
		}
		tools = append(tools, t)
	}
	im.mu.Lock()
	im.tools = tools
	im.mu.Unlock()
	im.register()
	return nil
}

func (im *mcpImport) listed() bool {
	im.mu.Lock()
	defer im.mu.Unlock()
	return im.tools != nil
}

// register makes every listed tool an operation.
func (im *mcpImport) register() {
	im.mu.Lock()
	defer im.mu.Unlock()
	for _, name := range im.names {
		im.ctx.RemoveOp(name)
	}
	im.names = im.names[:0]
	for _, t := range im.tools {
		remote := t.Name
		name := strings.Trim(toolName.ReplaceAllString(remote, "_"), "_")
		var schema map[string]any
		raw, _ := json.Marshal(t.InputSchema)
		_ = json.Unmarshal(raw, &schema)
		if schema == nil {
			schema = map[string]any{"type": "object"}
		}
		props, _ := schema["properties"].(map[string]any)
		_, own := props["account"]
		raw, _ = json.Marshal(im.accounts.withAccount(schema))
		kind := Write
		if t.Annotations != nil && t.Annotations.ReadOnlyHint {
			kind = Read
		}
		im.ctx.OpRaw(name, t.Description, raw, kind, func(call *Call, input json.RawMessage) (Result, error) {
			return im.call(call.Context, remote, own, input)
		})
		im.names = append(im.names, name)
	}
	im.ctx.Changed()
}

func (im *mcpImport) connect(ctx context.Context, account string, fresh bool) (*mcp.ClientSession, error) {
	im.mu.Lock()
	defer im.mu.Unlock()
	if s := im.sessions[account]; s != nil {
		if !fresh {
			return s, nil
		}
		_ = s.Close()
		delete(im.sessions, account)
	}
	var transport mcp.Transport
	switch {
	case im.server.URL != "":
		transport = &mcp.StreamableClientTransport{Endpoint: im.server.URL, HTTPClient: &http.Client{Transport: &authTransport{im.accounts, account, im.server.Headers}}}
	case len(im.server.Command) > 0:
		transport = &mcp.CommandTransport{Command: exec.Command(im.server.Command[0], im.server.Command[1:]...)}
	default:
		return nil, errors.New("MCP server needs a url or a command")
	}
	client := mcp.NewClient(&mcp.Implementation{Name: "the-one-app/" + im.ctx.id, Version: "1"}, nil)
	session, err := client.Connect(ctx, transport, nil)
	if err != nil {
		return nil, err
	}
	im.sessions[account] = session
	return session, nil
}

func (im *mcpImport) call(ctx context.Context, tool string, ownAccount bool, input json.RawMessage) (Result, error) {
	args := map[string]any{}
	if len(input) > 0 && string(input) != "null" {
		if err := json.Unmarshal(input, &args); err != nil {
			return Result{}, fmt.Errorf("bad input: %w", err)
		}
	}
	account := ""
	if im.accounts != nil {
		var err error
		if account, err = im.accounts.Resolve(takeAccount(args, ownAccount)); err != nil {
			return Result{}, err
		}
	}
	var res *mcp.CallToolResult
	var err error
	for attempt := range 2 { // a dropped session is reconnected once
		var session *mcp.ClientSession
		if session, err = im.connect(ctx, account, attempt > 0); err != nil {
			continue
		}
		if res, err = session.CallTool(ctx, &mcp.CallToolParams{Name: tool, Arguments: args}); err == nil {
			break
		}
	}
	if err != nil {
		return Result{}, err
	}
	return MCPResult(res)
}

// MCPResult turns an MCP tool result into an operation's result.
func MCPResult(res *mcp.CallToolResult) (Result, error) {
	var parts []string
	var images []Image
	for _, c := range res.Content {
		switch v := c.(type) {
		case *mcp.TextContent:
			parts = append(parts, v.Text)
		case *mcp.ImageContent:
			images = append(images, Image{MIMEType: v.MIMEType, Data: v.Data})
		default:
			parts = append(parts, fmt.Sprintf("[%T]", c))
		}
	}
	text := strings.Join(parts, "\n")
	if res.IsError {
		return Result{}, errors.New(text)
	}
	return Result{Text: text, Data: res.StructuredContent, Images: images}, nil
}

// authTransport adds an account's token and any fixed headers.
type authTransport struct {
	accounts *Accounts
	account  string
	headers  map[string]string
}

func (t *authTransport) RoundTrip(req *http.Request) (*http.Response, error) {
	req = req.Clone(req.Context())
	for k, v := range t.headers {
		req.Header.Set(k, v)
	}
	if t.accounts != nil {
		token, err := t.accounts.Token(req.Context(), t.account)
		if err != nil {
			return nil, err
		}
		req.Header.Set("Authorization", "Bearer "+token)
	}
	return http.DefaultTransport.RoundTrip(req)
}
