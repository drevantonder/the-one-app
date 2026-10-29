package claude

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestRunFindsToolsOutsidePATH(t *testing.T) {
	home := t.TempDir()
	bin := filepath.Join(home, ".local", "bin")
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(bin, "claude"), []byte("#!/bin/sh\necho '9.9.9 (Claude Code)'\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("HOME", home)
	t.Setenv("PATH", "/usr/bin:/bin")
	if c, _ := versions(context.Background()); c != "9.9.9" {
		t.Fatalf("claude version %q", c)
	}
}

func TestConnectKeepsExistingServer(t *testing.T) {
	home := t.TempDir()
	bin := filepath.Join(home, ".local", "bin")
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(bin, "claude"), []byte("#!/bin/sh\necho \"$@\" >> \"$HOME/args\"\necho 'MCP server the-one-app already exists in user config'\nexit 1\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("HOME", home)
	if err := connect(context.Background()); err != nil {
		t.Fatal(err)
	}
	args, _ := os.ReadFile(filepath.Join(home, "args"))
	if !strings.HasPrefix(string(args), "mcp add --scope user the-one-app -- /") || !strings.HasSuffix(strings.TrimSpace(string(args)), " mcp") {
		t.Fatalf("claude %s", args)
	}
}
