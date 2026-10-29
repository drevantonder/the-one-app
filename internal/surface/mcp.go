package surface

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"sync"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/drevantonder/the-one-app/internal/core"
)

// mcpHub keeps every MCP server's tools and resources in step with the
// node. Each new session gets a server built with the current instructions;
// when plugins change, every server's tools and resources are replaced,
// which notifies clients.
type mcpHub struct {
	node    *core.Node
	version string

	mu      sync.Mutex
	servers map[*mcp.Server]installed
	latest  *mcp.Server
}

// MCP returns a streamable HTTP MCP handler for the node.
func MCP(n *core.Node, version string) http.Handler {
	h := &mcpHub{node: n, version: version, servers: map[*mcp.Server]installed{}}
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
	h.servers[s] = h.install(s, installed{})
	h.latest = s
	return s
}

func (h *mcpHub) refresh() {
	h.mu.Lock()
	defer h.mu.Unlock()
	for s, old := range h.servers {
		h.servers[s] = h.install(s, old)
	}
	// Instructions may have changed: the next session gets a new server.
	h.latest = nil
}

// installed is what a server currently has.
type installed struct{ tools, resources []string }

func (h *mcpHub) install(s *mcp.Server, old installed) installed {
	if len(old.tools) > 0 {
		s.RemoveTools(old.tools...)
	}
	if len(old.resources) > 0 {
		s.RemoveResources(old.resources...)
	}
	var now installed
	for _, r := range h.node.Resources() {
		res := &mcp.Resource{URI: r.URI, Name: r.Name, Description: r.Description, MIMEType: r.MIMEType}
		s.AddResource(res, func(_ context.Context, req *mcp.ReadResourceRequest) (*mcp.ReadResourceResult, error) {
			r, ok := h.node.Resource(req.Params.URI)
			if !ok {
				return nil, mcp.ResourceNotFoundError(req.Params.URI)
			}
			c := &mcp.ResourceContents{URI: r.URI, MIMEType: r.MIMEType, Text: r.Text}
			if len(r.Meta) > 0 {
				if err := json.Unmarshal(r.Meta, &c.Meta); err != nil {
					return nil, fmt.Errorf("%s: bad meta: %w", r.URI, err)
				}
			}
			return &mcp.ReadResourceResult{Contents: []*mcp.ResourceContents{c}}, nil
		})
		now.resources = append(now.resources, r.URI)
	}
	for _, op := range h.node.Operations() {
		op := op
		tool := &mcp.Tool{
			Name:        op.ToolName(),
			Description: op.Description,
			InputSchema: op.InputSchema,
			Annotations: &mcp.ToolAnnotations{ReadOnlyHint: op.ReadOnly},
		}
		if op.UI != nil {
			tool.Meta = mcp.Meta{"ui": op.UI}
		}
		if op.Widget {
			if tool.Meta == nil {
				tool.Meta = mcp.Meta{}
			}
			tool.Meta["dev.theoneapp/widget"] = true
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
			return &mcp.CallToolResult{Content: content, StructuredContent: structured(res.Data)}, nil
		})
		now.tools = append(now.tools, tool.Name)
	}
	return now
}

// structured is a result's data as MCP structured content, which must be a
// JSON object, for MCP Apps views to draw.
func structured(data any) any {
	b, err := json.Marshal(data)
	if err != nil || len(b) == 0 || b[0] != '{' {
		return nil
	}
	return json.RawMessage(b)
}
