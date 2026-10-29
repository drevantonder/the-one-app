package agentsmd

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func setupHome(t *testing.T, tools ...string) (*files, string) {
	t.Helper()
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("CODEX_HOME", "")
	for _, d := range tools {
		if err := os.MkdirAll(filepath.Join(home, d), 0o755); err != nil {
			t.Fatal(err)
		}
	}
	return &files{home: home, state: t.TempDir()}, home
}

func read(t *testing.T, path string) string {
	t.Helper()
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return string(raw)
}

func put(t *testing.T, path, text string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(text), 0o644); err != nil {
		t.Fatal(err)
	}
}

func TestUnsetLeavesFilesAlone(t *testing.T) {
	f, home := setupHome(t, ".codex")
	codex := filepath.Join(home, ".codex", "AGENTS.md")
	put(t, codex, "mine")
	if edit, _, err := f.sync(""); err != nil || edit != "" {
		t.Fatal(edit, err)
	}
	if got := read(t, codex); got != "mine" {
		t.Fatalf("file changed: %q", got)
	}
}

func TestWritesOnlyInstalledTools(t *testing.T) {
	f, home := setupHome(t, ".codex", ".config/opencode")
	if _, _, err := f.sync("be kind"); err != nil {
		t.Fatal(err)
	}
	for _, p := range []string{".codex/AGENTS.md", ".config/opencode/AGENTS.md"} {
		if got := read(t, filepath.Join(home, p)); got != "be kind\n" {
			t.Fatalf("%s: %q", p, got)
		}
	}
	if _, err := os.Stat(filepath.Join(home, ".config", "amp")); !os.IsNotExist(err) {
		t.Fatalf("created a folder for a tool that isn't installed: %v", err)
	}
}

func TestFirstWriteKeepsACopy(t *testing.T) {
	f, home := setupHome(t, ".codex")
	codex := filepath.Join(home, ".codex", "AGENTS.md")
	put(t, codex, "old\n")
	if edit, _, err := f.sync("merged"); err != nil || edit != "" {
		t.Fatal(edit, err)
	}
	if got := read(t, codex); got != "merged\n" {
		t.Fatalf("got %q", got)
	}
	if got := read(t, filepath.Join(f.state, "before", "codex-AGENTS.md")); got != "old\n" {
		t.Fatalf("backup %q", got)
	}
}

func TestHandEdit(t *testing.T) {
	f, home := setupHome(t, ".codex", ".config/amp")
	codex := filepath.Join(home, ".codex", "AGENTS.md")
	amp := filepath.Join(home, ".config", "amp", "AGENTS.md")
	if _, _, err := f.sync("v1"); err != nil {
		t.Fatal(err)
	}
	put(t, codex, "v2 by hand\n")
	edit, _, err := f.sync("v1")
	if err != nil || edit != "v2 by hand" {
		t.Fatalf("edit %q, %v", edit, err)
	}
	if got := read(t, codex); got != "v2 by hand\n" {
		t.Fatalf("hand edit overwritten: %q", got)
	}
	// Recorded as a revision, the edit spreads.
	if edit, _, err := f.sync("v2 by hand"); err != nil || edit != "" {
		t.Fatal(edit, err)
	}
	if got := read(t, amp); got != "v2 by hand\n" {
		t.Fatalf("amp %q", got)
	}
	// A new revision from elsewhere reaches files not edited since.
	if edit, _, _ := f.sync("v3"); edit != "" {
		t.Fatal(edit)
	}
	if got := read(t, codex); got != "v3\n" {
		t.Fatalf("got %q", got)
	}
}

func TestNewestHandEditWins(t *testing.T) {
	f, home := setupHome(t, ".codex", ".config/amp")
	codex := filepath.Join(home, ".codex", "AGENTS.md")
	amp := filepath.Join(home, ".config", "amp", "AGENTS.md")
	f.sync("v1")
	put(t, codex, "older")
	put(t, amp, "newer")
	old := time.Now().Add(-time.Hour)
	os.Chtimes(codex, old, old)
	if edit, _, _ := f.sync("v1"); edit != "newer" {
		t.Fatalf("edit %q", edit)
	}
}

func TestEmptiedFileIsRewritten(t *testing.T) {
	f, home := setupHome(t, ".codex")
	codex := filepath.Join(home, ".codex", "AGENTS.md")
	f.sync("v1")
	put(t, codex, "")
	if edit, _, _ := f.sync("v1"); edit != "" {
		t.Fatalf("recorded an empty edit %q", edit)
	}
	if got := read(t, codex); got != "v1\n" {
		t.Fatalf("got %q", got)
	}
}

func TestSymlinksLeftAlone(t *testing.T) {
	f, home := setupHome(t, ".codex")
	real := filepath.Join(home, "repo", "AGENTS.global.md")
	os.MkdirAll(filepath.Dir(real), 0o755)
	put(t, real, "theirs\n")
	codex := filepath.Join(home, ".codex", "AGENTS.md")
	if err := os.Symlink(real, codex); err != nil {
		t.Fatal(err)
	}
	edit, lines, err := f.sync("doc")
	if err != nil || edit != "" {
		t.Fatal(edit, err)
	}
	if got := read(t, real); got != "theirs\n" {
		t.Fatalf("wrote through the symlink: %q", got)
	}
	if fi, _ := os.Lstat(codex); fi.Mode()&os.ModeSymlink == 0 {
		t.Fatal("symlink replaced")
	}
	if len(lines) != 1 || !strings.Contains(lines[0], "symlink") {
		t.Fatalf("not reported: %q", lines)
	}
}
