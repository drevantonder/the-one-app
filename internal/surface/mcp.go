package surface

import (
	"context"
	"encoding/json"
	"net/http"
	"sync"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/drevantonder/the-one-app/internal/core"
)

// mcpHub keeps every MCP server's tools in step with the node. Each new
// session gets a server built with the current instructions; when plugins
// change, every server's tools are replaced, which notifies clients.
type mcpHub struct {
	node    *core.Node
	version string

	mu      sync.Mutex
	servers map[*mcp.Server][]string // server → tool names it currently has
	latest  *mcp.Server
}

// MCP returns a streamable HTTP MCP handler for the node.
func MCP(n *core.Node, version string) http.Handler {
	h := &mcpHub{node: n, version: version, servers: map[*mcp.Server][]string{}}
	n.OnChange(h.refresh)
	return mcp.NewStreamableHTTPHandler(h.server, nil)
}

// server returns the server for a new session, building a fresh one if the
// instructions have changed since the last.
func (h *mcpHub) server(*http.Request) *mcp.Server {
	h.mu.Lock()
	defer h.mu.Unlock()
	if h.latest != nil {
		return h.latest
	}
	s := mcp.NewServer(&mcp.Implementation{Name: "the-one-app", Version: h.version}, &mcp.ServerOptions{
		Instructions: h.node.Instructions(context.Background()),
	})
	h.servers[s] = h.install(s, nil)
	h.latest = s
	return s
}

func (h *mcpHub) refresh() {
	h.mu.Lock()
	defer h.mu.Unlock()
	for s, names := range h.servers {
		h.servers[s] = h.install(s, names)
	}
	// Instructions may have changed: the next session gets a new server.
	h.latest = nil
}

func (h *mcpHub) install(s *mcp.Server, old []string) []string {
	if len(old) > 0 {
		s.RemoveTools(old...)
	}
	var names []string
	for _, op := range h.node.Operations() {
		op := op
		tool := &mcp.Tool{
			Name:        op.ToolName(),
			Description: op.Description,
			InputSchema: op.InputSchema,
			Annotations: &mcp.ToolAnnotations{ReadOnlyHint: op.ReadOnly},
		}
		s.AddTool(tool, func(ctx context.Context, req *mcp.CallToolRequest) (*mcp.CallToolResult, error) {
			args := req.Params.Arguments
			if len(args) == 0 {
				args = json.RawMessage("{}")
			}
			res, err := h.node.Call(ctx, op.Plugin, op.Name, args, mcpCaller())
			if err != nil {
				return &mcp.CallToolResult{IsError: true, Content: []mcp.Content{&mcp.TextContent{Text: errText(err)}}}, nil
			}
			var content []mcp.Content
			if text := res.Text; text != "" || res.Data != nil || len(res.Images) == 0 {
				if text == "" {
					b, _ := json.MarshalIndent(res.Data, "", "  ")
					text = string(b)
				}
				content = append(content, &mcp.TextContent{Text: text})
			}
			for _, img := range res.Images {
				content = append(content, &mcp.ImageContent{MIMEType: img.MIMEType, Data: img.Data})
			}
			return &mcp.CallToolResult{Content: content}, nil
		})
		names = append(names, tool.Name)
	}
	return names
}
