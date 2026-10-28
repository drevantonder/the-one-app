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

// ImportMCP makes the server's tools the plugin's operations. With an
// account, it connects once signed in and sends the account's token.
func ImportMCP(c *Ctx, server MCPServer, account *Account) {
	im := &mcpImport{ctx: c, server: server, account: account}
	c.OnReady(func(call *Call) { im.refresh(call.Context) })
	if account != nil {
		account.OnSignIn(func(call *Call) { im.refresh(call.Context) })
	}
	Op(c, "refresh_tools", "Reconnect to the MCP server and reload its tools.", Write, func(call *Call, _ noInput) (Result, error) {
		if err := im.refresh(call.Context); err != nil {
			return Result{}, err
		}
		return Text(fmt.Sprintf("%d tools.", len(im.names))), nil
	})
}

type mcpImport struct {
	ctx     *Ctx
	server  MCPServer
	account *Account

	mu      sync.Mutex
	session *mcp.ClientSession
	names   []string
}

var toolName = regexp.MustCompile(`[^a-zA-Z0-9_-]+`)

// refresh lists the server's tools and swaps them in as operations.
func (im *mcpImport) refresh(ctx context.Context) error {
	if im.account != nil && !im.account.SignedIn() {
		return errors.New("not signed in: use sign_in first")
	}
	session, err := im.connect(ctx, true)
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
	for _, name := range im.names {
		im.ctx.RemoveOp(name)
	}
	im.names = im.names[:0]
	for _, t := range tools {
		remote := t.Name
		name := strings.Trim(toolName.ReplaceAllString(remote, "_"), "_")
		schema, _ := json.Marshal(t.InputSchema)
		kind := Write
		if t.Annotations != nil && t.Annotations.ReadOnlyHint {
			kind = Read
		}
		im.ctx.OpRaw(name, t.Description, schema, kind, func(call *Call, input json.RawMessage) (Result, error) {
			return im.call(call.Context, remote, input)
		})
		im.names = append(im.names, name)
	}
	im.ctx.Changed()
	return nil
}

func (im *mcpImport) connect(ctx context.Context, fresh bool) (*mcp.ClientSession, error) {
	im.mu.Lock()
	defer im.mu.Unlock()
	if im.session != nil && !fresh {
		return im.session, nil
	}
	if im.session != nil {
		_ = im.session.Close()
		im.session = nil
	}
	var transport mcp.Transport
	switch {
	case im.server.URL != "":
		transport = &mcp.StreamableClientTransport{Endpoint: im.server.URL, HTTPClient: &http.Client{Transport: &authTransport{im.account, im.server.Headers}}}
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
	im.session = session
	return session, nil
}

func (im *mcpImport) call(ctx context.Context, tool string, input json.RawMessage) (Result, error) {
	var args map[string]any
	if len(input) > 0 && string(input) != "null" {
		if err := json.Unmarshal(input, &args); err != nil {
			return Result{}, fmt.Errorf("bad input: %w", err)
		}
	}
	var res *mcp.CallToolResult
	var err error
	for attempt := range 2 { // a dropped session is reconnected once
		var session *mcp.ClientSession
		if session, err = im.connect(ctx, attempt > 0); err != nil {
			continue
		}
		if res, err = session.CallTool(ctx, &mcp.CallToolParams{Name: tool, Arguments: args}); err == nil {
			break
		}
	}
	if err != nil {
		return Result{}, err
	}
	var parts []string
	for _, c := range res.Content {
		switch v := c.(type) {
		case *mcp.TextContent:
			parts = append(parts, v.Text)
		default:
			parts = append(parts, fmt.Sprintf("[%T]", c))
		}
	}
	text := strings.Join(parts, "\n")
	if res.IsError {
		return Result{}, errors.New(text)
	}
	return Result{Text: text, Data: res.StructuredContent}, nil
}

// authTransport adds the account's token and any fixed headers.
type authTransport struct {
	account *Account
	headers map[string]string
}

func (t *authTransport) RoundTrip(req *http.Request) (*http.Response, error) {
	req = req.Clone(req.Context())
	for k, v := range t.headers {
		req.Header.Set(k, v)
	}
	if t.account != nil {
		token, err := t.account.Token(req.Context())
		if err != nil {
			return nil, err
		}
		req.Header.Set("Authorization", "Bearer "+token)
	}
	return http.DefaultTransport.RoundTrip(req)
}
