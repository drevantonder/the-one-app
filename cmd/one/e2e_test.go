package main

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"slices"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// nodeEnv runs a test node with its own home, with a Claude Code folder, so
// its plugins write to agents' settings there and not the user's.
func nodeEnv(t *testing.T, home string) []string {
	if err := os.MkdirAll(filepath.Join(home, ".claude"), 0o755); err != nil {
		t.Fatal(err)
	}
	out, err := exec.Command("go", "env", "GOCACHE", "GOMODCACHE").Output()
	if err != nil {
		t.Fatal(err)
	}
	cache := strings.Fields(string(out))
	return append(os.Environ(), "HOME="+home, "CLAUDE_CONFIG_DIR=", "CODEX_HOME=", "XDG_CONFIG_HOME=", "PI_CODING_AGENT_DIR=", "GOCACHE="+cache[0], "GOMODCACHE="+cache[1])
}

// TestNodeOverMCP builds the real binary, runs a node, and drives it with the
// official MCP client, the way an agent would.
func TestNodeOverMCP(t *testing.T) {
	if testing.Short() {
		t.Skip("builds and runs the binary")
	}
	dir := t.TempDir()
	bin := filepath.Join(dir, "one")
	if out, err := exec.Command("go", "build", "-o", bin, ".").CombinedOutput(); err != nil {
		t.Fatalf("build: %v\n%s", err, out)
	}
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	addr := ln.Addr().String()
	ln.Close()

	serve := exec.Command(bin, "serve", "--local", "--addr", addr, "--data", filepath.Join(dir, "data"))
	serve.Stderr = os.Stderr
	serve.Dir = dir // like launchd or systemd: not inside a Go module
	home := filepath.Join(dir, "home")
	serve.Env = nodeEnv(t, home)
	if err := serve.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = serve.Process.Signal(os.Interrupt); _ = serve.Wait() })
	waitFor(t, "http://"+addr+"/healthz")

	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	client := mcp.NewClient(&mcp.Implementation{Name: "test", Version: "1"}, nil)
	session, err := client.Connect(ctx, &mcp.StreamableClientTransport{Endpoint: "http://" + addr + "/mcp"}, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer session.Close()

	if ins := session.InitializeResult().Instructions; !strings.Contains(ins, "## Agent Board") || !strings.Contains(ins, "board_post") {
		t.Fatalf("instructions missing the board's text:\n%s", ins)
	}
	tools, err := session.ListTools(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	var names []string
	for _, tool := range tools.Tools {
		names = append(names, tool.Name)
	}
	for _, want := range []string{"instructions_set", "skills_list", "plugins_list",
		"board_topics", "board_create", "board_read", "board_post", "board_nap", "board_zoom", "board_recall", "board_news"} {
		if !slices.Contains(names, want) {
			t.Fatalf("tool %s missing from %v", want, names)
		}
	}

	// MCP Apps: the menu bar lists views, each a ui:// resource showing a
	// tool's structured result.
	bar, err := session.CallTool(ctx, &mcp.CallToolParams{Name: "menubar", Arguments: map[string]any{}})
	if err != nil || bar.IsError {
		t.Fatalf("menubar: %v %+v", err, bar)
	}
	var doc struct {
		Items []struct {
			ID, Tool, Resource string
			Bar                bool
		}
	}
	raw, _ := json.Marshal(bar.StructuredContent)
	if err := json.Unmarshal(raw, &doc); err != nil || len(doc.Items) < 3 || len(doc.Items) > 4 || len(doc.Items) == 4 && doc.Items[3].ID != "spaces" || doc.Items[0].ID != "devices" || doc.Items[1].ID != "one.plugins" || doc.Items[2].ID != "board" || !doc.Items[2].Bar {
		t.Fatalf("menubar items: %s", raw)
	}
	for _, item := range doc.Items {
		// An item with no view is a widget, which the apps draw natively.
		if item.Resource != "" {
			view, err := session.ReadResource(ctx, &mcp.ReadResourceParams{URI: item.Resource})
			if err != nil || view.Contents[0].MIMEType != "text/html;profile=mcp-app" || !strings.Contains(view.Contents[0].Text, "ui/initialize") {
				t.Fatalf("view %s: %v", item.Resource, err)
			}
		}
		res, err := session.CallTool(ctx, &mcp.CallToolParams{Name: item.Tool, Arguments: map[string]any{}})
		if err != nil || res.IsError || res.StructuredContent == nil {
			t.Fatalf("%s: no structured result: %v %+v", item.Tool, err, res)
		}
	}
	// Pages: a plugin's page lists items, and draws each as a widget.
	pages, err := session.CallTool(ctx, &mcp.CallToolParams{Name: "pages", Arguments: map[string]any{}})
	if err != nil || pages.IsError {
		t.Fatalf("pages: %v %+v", err, pages)
	}
	var pdoc struct {
		Pages []struct{ ID, Title, Tool, Items, Settings string }
	}
	raw, _ = json.Marshal(pages.StructuredContent)
	if err := json.Unmarshal(raw, &pdoc); err != nil || len(pdoc.Pages) < 1 || pdoc.Pages[0].Tool != "board_channel" || pdoc.Pages[0].Items != "board_channel_items" {
		t.Fatalf("pages: %s", raw)
	}
	// Spaces, where Podman is, has a page with no items, only its widget,
	// and settings.
	if len(pdoc.Pages) > 1 {
		p := pdoc.Pages[1]
		if p.Tool != "spaces_page" || p.Items != "" || p.Settings != "spaces_page_settings" {
			t.Fatalf("pages: %s", raw)
		}
		for _, tool := range []string{p.Tool, p.Settings} {
			res, err := session.CallTool(ctx, &mcp.CallToolParams{Name: tool, Arguments: map[string]any{}})
			if raw, _ := json.Marshal(res.StructuredContent); err != nil || res.IsError || !strings.Contains(string(raw), `"root"`) {
				t.Fatalf("%s: %v %s", tool, err, raw)
			}
		}
	}
	items, err := session.CallTool(ctx, &mcp.CallToolParams{Name: "board_channel_items", Arguments: map[string]any{}})
	if raw, _ = json.Marshal(items.StructuredContent); err != nil || !strings.Contains(string(raw), `"title":"general"`) {
		t.Fatalf("board_channel_items: %v %s", err, raw)
	}
	channel, err := session.CallTool(ctx, &mcp.CallToolParams{Name: "board_channel", Arguments: map[string]any{"item": ""}})
	if raw, _ = json.Marshal(channel.StructuredContent); err != nil || channel.IsError || !strings.Contains(string(raw), "#general") {
		t.Fatalf("board_channel: %v %s", err, raw)
	}

	for _, tool := range tools.Tools {
		if ui, _ := tool.Meta["ui"].(map[string]any); tool.Name == "plugins_list" && ui["resourceUri"] != "ui://one/plugins" {
			t.Fatalf("plugins_list _meta: %v", tool.Meta)
		}
	}

	// A widget is an MCP tool: its structured result is the widget, which the
	// core's widget view draws in MCP Apps hosts.
	var widgetTool *mcp.Tool
	for _, tool := range tools.Tools {
		if tool.Name == "devices_widget" {
			widgetTool = tool
		}
	}
	if widgetTool == nil || widgetTool.Meta["dev.theoneapp/widget"] != true {
		t.Fatalf("devices_widget missing or not marked as a widget: %+v", widgetTool)
	}
	if ui, _ := widgetTool.Meta["ui"].(map[string]any); ui["resourceUri"] != "ui://one/widget" {
		t.Fatalf("devices_widget _meta.ui: %v", widgetTool.Meta)
	}
	drawn, err := session.CallTool(ctx, &mcp.CallToolParams{Name: "devices_widget", Arguments: map[string]any{}})
	if err != nil || drawn.IsError {
		t.Fatalf("devices_widget: %v %+v", err, drawn)
	}
	raw, _ = json.Marshal(drawn.StructuredContent)
	var desc struct {
		V    int
		App  string
		Root struct{ T string }
	}
	if json.Unmarshal(raw, &desc) != nil || desc.V != 1 || desc.App != "devices" || desc.Root.T != "col" {
		t.Fatalf("devices_widget description: %s", raw)
	}
	if view, err := session.ReadResource(ctx, &mcp.ReadResourceParams{URI: "ui://one/widget"}); err != nil || !strings.Contains(view.Contents[0].Text, "ui/initialize") {
		t.Fatalf("widget view: %v", err)
	}

	call := func(name string, args map[string]any) (string, bool) {
		t.Helper()
		res, err := session.CallTool(ctx, &mcp.CallToolParams{Name: name, Arguments: args})
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		return res.Content[0].(*mcp.TextContent).Text, res.IsError
	}

	if out, _ := call("shell_run", map[string]any{"command": "echo hello from the shell"}); !strings.Contains(out, "exit 0") || !strings.Contains(out, "hello from the shell") {
		t.Fatalf("shell_run: %s", out)
	}
	if out, _ := call("shell_history", nil); !strings.Contains(out, "echo hello from the shell") {
		t.Fatalf("shell_history: %s", out)
	}
	call("instructions_set", map[string]any{"text": "Keep answers short."})
	for i := range 3 {
		call("board_post", map[string]any{"text": fmt.Sprintf("fact %d", i)})
	}
	if text, isErr := call("board_post", map[string]any{"text": ""}); !isErr {
		t.Fatalf("empty post should be a tool error, got %q", text)
	}
	if text, _ := call("board_nap", map[string]any{"block": "0-1", "text": "facts 0 and 1"}); !strings.Contains(text, "0-1 saved") {
		t.Fatalf("nap: %s", text)
	}
	if top, _ := call("board_read", nil); !strings.Contains(top, "#2 ") || !strings.Contains(top, "fact 2") {
		t.Fatalf("board_read after posts:\n%s", top)
	}

	call("board_create", map[string]any{"topic": "projects", "description": "Work projects"})
	if text, isErr := call("board_create", map[string]any{"topic": "projects/the-one-app/whatsapp", "description": "x"}); !isErr {
		t.Fatalf("create without a parent should be a tool error, got %q", text)
	}
	call("board_post", map[string]any{"topic": "projects", "text": "started the board", "where": "the-one-app"})
	if text, _ := call("board_post", map[string]any{"text": "the user lives in Sydney"}); !strings.Contains(text, "Posted as #3 in the top.") {
		t.Fatalf("board_post: %s", text)
	}
	if top, _ := call("board_read", nil); !strings.Contains(top, "the user lives in Sydney") || !strings.Contains(top, "Subtopics:\n  projects — ") {
		t.Fatalf("board_read:\n%s", top)
	}
	// Agents' prompt hook: a session's first prompt marks its place, and
	// later prompts get only what has been posted since.
	if settings, err := os.ReadFile(filepath.Join(home, ".claude", "settings.json")); err != nil || !strings.Contains(string(settings), "http://"+addr+"/hooks/board/news") {
		t.Fatalf("prompt hook not installed: %v\n%s", err, settings)
	}
	hook := func() string {
		res, err := http.Post("http://"+addr+"/hooks/board/news", "application/json", strings.NewReader(`{"session_id":"s1","hook_event_name":"UserPromptSubmit"}`))
		if err != nil {
			t.Fatal(err)
		}
		defer res.Body.Close()
		b, _ := io.ReadAll(res.Body)
		return string(b)
	}
	if first := hook(); !strings.Contains(first, "news will appear here") {
		t.Fatalf("first hook call: %q", first)
	}
	call("board_post", map[string]any{"topic": "projects", "text": "hook news"})
	if news := hook(); !strings.Contains(news, "projects #1 ") || !strings.Contains(news, "hook news") || strings.Contains(news, "Sydney") {
		t.Fatalf("hook news:\n%s", news)
	}
	if again := hook(); again != "" {
		t.Fatalf("hook repeated news: %q", again)
	}

	if hits, _ := call("board_recall", map[string]any{"pattern": "board|sydney"}); !strings.Contains(hits, "projects #0 ") || !strings.Contains(hits, "(top) #3 ") {
		t.Fatalf("board_recall:\n%s", hits)
	}

	// Disabling a plugin removes its tools and wake section; enabling it
	// again rebuilds everything from the log.
	call("plugins_disable", map[string]any{"id": "board"})
	tools, _ = session.ListTools(ctx, nil)
	for _, tool := range tools.Tools {
		if strings.HasPrefix(tool.Name, "board_") {
			t.Fatalf("board tool %s still listed after disable", tool.Name)
		}
	}
	call("plugins_enable", map[string]any{"id": "board"})
	if top, _ := call("board_read", nil); !strings.Contains(top, "fact 2") {
		t.Fatalf("board not restored after enable:\n%s", top)
	}
	// A plugin whose process dies is started again, without a reload.
	if runtime.GOOS != "windows" {
		out, err := exec.Command("pgrep", "-P", strconv.Itoa(serve.Process.Pid), "-f", "plugin-host board").Output()
		if err != nil {
			t.Fatalf("finding the board's process: %v", err)
		}
		pid, _ := strconv.Atoi(strings.Fields(string(out))[0])
		if p, err := os.FindProcess(pid); err != nil || p.Kill() != nil {
			t.Fatalf("killing the board's process %d: %v", pid, err)
		}
		deadline := time.Now().Add(10 * time.Second)
		for {
			res, err := session.CallTool(ctx, &mcp.CallToolParams{Name: "board_read"})
			if err == nil && !res.IsError && strings.Contains(res.Content[0].(*mcp.TextContent).Text, "fact 2") {
				break
			}
			if time.Now().After(deadline) {
				t.Fatalf("board not restarted after its process was killed: %v %+v", err, res)
			}
			time.Sleep(100 * time.Millisecond)
		}
	}

	// A plugin with no code: an MCP server's tools become its operations.
	upstream := mcp.NewServer(&mcp.Implementation{Name: "upstream", Version: "1"}, nil)
	type echoIn struct {
		Text string `json:"text"`
	}
	mcp.AddTool(upstream, &mcp.Tool{Name: "echo", Description: "Echo text back."}, func(_ context.Context, _ *mcp.CallToolRequest, in echoIn) (*mcp.CallToolResult, any, error) {
		return &mcp.CallToolResult{Content: []mcp.Content{&mcp.TextContent{Text: "echo: " + in.Text}}}, nil, nil
	})
	mcp.AddTool(upstream, &mcp.Tool{Name: "picture", Description: "Return an image."}, func(context.Context, *mcp.CallToolRequest, struct{}) (*mcp.CallToolResult, any, error) {
		return &mcp.CallToolResult{Content: []mcp.Content{&mcp.ImageContent{MIMEType: "image/png", Data: []byte("png bytes")}}}, nil, nil
	})
	upstreamSrv := httptest.NewServer(mcp.NewStreamableHTTPHandler(func(*http.Request) *mcp.Server { return upstream }, nil))
	defer func() { upstreamSrv.CloseClientConnections(); upstreamSrv.Close() }() // the plugin keeps a live connection
	add := exec.Command(bin, "plugins", "add", "echoer", "--mcp", upstreamSrv.URL)
	add.Env = append(os.Environ(), "ONE_ADDR="+addr)
	if out, err := add.CombinedOutput(); err != nil {
		t.Fatalf("plugins add: %v\n%s", err, out)
	}
	deadline := time.Now().Add(10 * time.Second)
	for {
		res, err := session.CallTool(ctx, &mcp.CallToolParams{Name: "echoer_echo", Arguments: map[string]any{"text": "hi"}})
		if err == nil && !res.IsError && res.Content[0].(*mcp.TextContent).Text == "echo: hi" {
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("echoer_echo never worked: %v %+v", err, res)
		}
		time.Sleep(200 * time.Millisecond)
	}
	// Reloading a plugin with no code restarts it; there is nothing to build.
	if res, err := session.CallTool(ctx, &mcp.CallToolParams{Name: "plugins_reload", Arguments: map[string]any{"id": "echoer"}}); err != nil || res.IsError {
		t.Fatalf("plugins_reload echoer: %v %+v", err, res)
	}
	// Images, such as screenshots, come through as images.
	res, err := session.CallTool(ctx, &mcp.CallToolParams{Name: "echoer_picture", Arguments: map[string]any{}})
	if err != nil || len(res.Content) != 1 {
		t.Fatalf("echoer_picture: %v %+v", err, res)
	}
	if img, ok := res.Content[0].(*mcp.ImageContent); !ok || img.MIMEType != "image/png" || string(img.Data) != "png bytes" {
		t.Fatalf("echoer_picture: want the image, got %+v", res.Content[0])
	}

	// `one mcp` relays stdio to the same node, with the same instructions and tools.
	relay := exec.Command(bin, "mcp")
	relay.Env = append(os.Environ(), "ONE_ADDR="+addr)
	stdio, err := client.Connect(ctx, &mcp.CommandTransport{Command: relay}, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer stdio.Close()
	// A new session's instructions include the user's instructions, set above.
	if ins := stdio.InitializeResult().Instructions; !strings.Contains(ins, "Keep answers short.") || !strings.Contains(ins, "board_read") {
		t.Fatalf("new session instructions:\n%s", ins)
	}
	res, err = stdio.CallTool(ctx, &mcp.CallToolParams{Name: "board_read"})
	if err != nil {
		t.Fatal(err)
	}
	if text := res.Content[0].(*mcp.TextContent).Text; !strings.Contains(text, "fact 2") {
		t.Fatalf("board_read over stdio:\n%s", text)
	}
	if view, err := stdio.ReadResource(ctx, &mcp.ReadResourceParams{URI: "ui://one/widget"}); err != nil || !strings.Contains(view.Contents[0].Text, "ui/initialize") {
		t.Fatalf("widget view over stdio: %v", err)
	}
}

func waitFor(t *testing.T, url string) {
	t.Helper()
	deadline := time.Now().Add(20 * time.Second)
	for time.Now().Before(deadline) {
		if resp, err := http.Get(url); err == nil {
			resp.Body.Close()
			return
		}
		time.Sleep(100 * time.Millisecond)
	}
	t.Fatalf("node did not start at %s", url)
}
