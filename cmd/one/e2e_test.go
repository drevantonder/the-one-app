package main

import (
	"context"
	"fmt"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

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

	if ins := session.InitializeResult().Instructions; !strings.Contains(ins, "call `memory_wake`") || !strings.Contains(ins, "memory_note") {
		t.Fatalf("instructions missing the memory plugin's text:\n%s", ins)
	}
	tools, err := session.ListTools(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	var names []string
	for _, tool := range tools.Tools {
		names = append(names, tool.Name)
	}
	for _, want := range []string{"memory_wake", "instructions_set", "memory_note", "memory_nap", "plugins_list"} {
		if !slices.Contains(names, want) {
			t.Fatalf("tool %s missing from %v", want, names)
		}
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
		call("memory_note", map[string]any{"text": fmt.Sprintf("fact %d", i)})
	}
	if text, isErr := call("memory_note", map[string]any{"text": ""}); !isErr {
		t.Fatalf("empty note should be a tool error, got %q", text)
	}
	if text, _ := call("memory_nap", map[string]any{"block": "0-1", "text": "facts 0 and 1"}); !strings.Contains(text, "0-1 saved") {
		t.Fatalf("nap: %s", text)
	}
	wake, _ := call("memory_wake", nil)
	for _, want := range []string{"#2 ", "fact 2", "You are awake."} {
		if !strings.Contains(wake, want) {
			t.Fatalf("wake missing %q:\n%s", want, wake)
		}
	}

	// Disabling a plugin removes its tools and wake section; enabling it
	// again rebuilds everything from the log.
	call("plugins_disable", map[string]any{"id": "memory"})
	tools, _ = session.ListTools(ctx, nil)
	for _, tool := range tools.Tools {
		if strings.HasPrefix(tool.Name, "memory_") {
			t.Fatalf("memory tool %s still listed after disable", tool.Name)
		}
	}
	call("plugins_enable", map[string]any{"id": "memory"})
	if wake, _ := call("memory_wake", nil); !strings.Contains(wake, "fact 2") {
		t.Fatalf("memory not restored after enable:\n%s", wake)
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
	if ins := stdio.InitializeResult().Instructions; !strings.Contains(ins, "Keep answers short.") || !strings.Contains(ins, "memory_wake") {
		t.Fatalf("new session instructions:\n%s", ins)
	}
	res, err := stdio.CallTool(ctx, &mcp.CallToolParams{Name: "memory_wake"})
	if err != nil {
		t.Fatal(err)
	}
	if text := res.Content[0].(*mcp.TextContent).Text; !strings.Contains(text, "fact 2") {
		t.Fatalf("wake over stdio:\n%s", text)
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
