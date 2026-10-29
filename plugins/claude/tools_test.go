package claude

import (
	"context"
	"os"
	"path/filepath"
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
