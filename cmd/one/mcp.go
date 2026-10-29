package main

import (
	"context"
	"fmt"
	"os"
	"sync"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// mcpStdio serves MCP over stdin/stdout by relaying to this device's node, so
// agents are configured the same way on every device: `one mcp`, no URL.
// Tools, resources and instructions mirror the node's, and follow it when
// plugins change.
func mcpStdio() error {
	ctx := context.Background()
	endpoint := "http://" + defaultAddr() + "/mcp"

	var (
		mu     sync.Mutex
		server *mcp.Server
		names  []string
		uris   []string
		remote *mcp.ClientSession
	)
	sync_ := func() {
		mu.Lock()
		defer mu.Unlock()
		if server == nil || remote == nil {
			return
		}
		var tools []*mcp.Tool
		for t, err := range remote.Tools(ctx, nil) {
			if err != nil {
				fmt.Fprintln(os.Stderr, "one mcp: list tools:", err)
				return
			}
			tools = append(tools, t)
		}
		if len(names) > 0 {
			server.RemoveTools(names...)
		}
		names = names[:0]
		for _, t := range tools {
			name := t.Name
			server.AddTool(t, func(ctx context.Context, req *mcp.CallToolRequest) (*mcp.CallToolResult, error) {
				return remote.CallTool(ctx, &mcp.CallToolParams{Name: name, Arguments: req.Params.Arguments})
			})
			names = append(names, name)
		}
		var resources []*mcp.Resource
		for r, err := range remote.Resources(ctx, nil) {
			if err != nil {
				fmt.Fprintln(os.Stderr, "one mcp: list resources:", err)
				return
			}
			resources = append(resources, r)
		}
		if len(uris) > 0 {
			server.RemoveResources(uris...)
		}
		uris = uris[:0]
		for _, r := range resources {
			server.AddResource(r, func(ctx context.Context, req *mcp.ReadResourceRequest) (*mcp.ReadResourceResult, error) {
				return remote.ReadResource(ctx, &mcp.ReadResourceParams{URI: req.Params.URI})
			})
			uris = append(uris, r.URI)
		}
	}

	client := mcp.NewClient(&mcp.Implementation{Name: "one-mcp", Version: version}, &mcp.ClientOptions{
		ToolListChangedHandler:     func(context.Context, *mcp.ToolListChangedRequest) { go sync_() },
		ResourceListChangedHandler: func(context.Context, *mcp.ResourceListChangedRequest) { go sync_() },
	})
	var err error
	for attempt := 0; ; attempt++ {
		remote, err = client.Connect(ctx, &mcp.StreamableClientTransport{Endpoint: endpoint}, nil)
		if err == nil {
			break
		}
		if attempt >= 20 { // the node may still be starting, e.g. right after login
			return fmt.Errorf("cannot reach the node at %s (is `one serve` running?): %w", endpoint, err)
		}
		time.Sleep(500 * time.Millisecond)
	}
	defer remote.Close()

	mu.Lock()
	server = mcp.NewServer(&mcp.Implementation{Name: "the-one-app", Version: version}, &mcp.ServerOptions{
		Instructions: remote.InitializeResult().Instructions,
	})
	mu.Unlock()
	sync_()
	return server.Run(ctx, &mcp.StdioTransport{})
}
