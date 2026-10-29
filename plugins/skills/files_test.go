package skills

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
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(text), 0o644); err != nil {
		t.Fatal(err)
	}
}

func skill(text string, more ...string) map[string]string {
	files := map[string]string{"SKILL.md": "---\nname: x\ndescription: " + text + "\n---\n" + text + "\n"}
	for i := 0; i+1 < len(more); i += 2 {
		files[more[i]] = more[i+1]
	}
	return files
}

func one(name string, files map[string]string) map[string]map[string]string {
	return map[string]map[string]string{name: files}
}

func TestTargetsEachFolderOnce(t *testing.T) {
	_, home := setupHome(t, ".claude", ".codex", ".config/opencode", ".config/amp")
	got := targets(home)
	if len(got) != 2 {
		t.Fatalf("%+v", got)
	}
	if got[0].dir != filepath.Join(home, ".claude", "skills") || got[1].dir != filepath.Join(home, ".agents", "skills") {
		t.Fatalf("%+v", got)
	}
	if strings.Join(got[1].tools, ",") != "Codex,OpenCode,Amp" {
		t.Fatalf("%+v", got[1].tools)
	}
}

func TestWritesOnlyInstalledTools(t *testing.T) {
	f, home := setupHome(t, ".codex")
	want := skill("v1", "scripts/run.sh", "echo hi\n")
	if _, _, err := f.sync(one("pdf", want), nil); err != nil {
		t.Fatal(err)
	}
	dir := filepath.Join(home, ".agents", "skills", "pdf")
	if got := read(t, filepath.Join(dir, "scripts", "run.sh")); got != "echo hi\n" {
		t.Fatalf("got %q", got)
	}
	if !ours(dir) {
		t.Fatal("no marker")
	}
	if _, err := os.Stat(filepath.Join(home, ".claude")); !os.IsNotExist(err) {
		t.Fatalf("wrote for a tool that isn't installed: %v", err)
	}
}

func TestNothingInstalledWritesNothing(t *testing.T) {
	f, home := setupHome(t)
	if _, _, err := f.sync(one("pdf", skill("v1")), nil); err != nil {
		t.Fatal(err)
	}
	if entries, _ := os.ReadDir(home); len(entries) != 0 {
		t.Fatalf("wrote %v", entries)
	}
}

func TestUpdateDropsOldFiles(t *testing.T) {
	f, home := setupHome(t, ".claude")
	dir := filepath.Join(home, ".claude", "skills", "pdf")
	f.sync(one("pdf", skill("v1", "ref/a.md", "a")), nil)
	put(t, filepath.Join(dir, ".DS_Store"), "x")
	if edits, _, err := f.sync(one("pdf", skill("v2")), nil); err != nil || len(edits) != 0 {
		t.Fatal(edits, err)
	}
	if _, err := os.Stat(filepath.Join(dir, "ref")); !os.IsNotExist(err) {
		t.Fatalf("old file kept: %v", err)
	}
	if !strings.Contains(read(t, filepath.Join(dir, "SKILL.md")), "v2") {
		t.Fatal("not updated")
	}
	if read(t, filepath.Join(dir, ".DS_Store")) != "x" {
		t.Fatal("hidden file touched")
	}
}

func TestOthersFoldersLeftAlone(t *testing.T) {
	f, home := setupHome(t, ".claude")
	root := filepath.Join(home, ".claude", "skills")
	put(t, filepath.Join(root, "pdf", "SKILL.md"), "mine")
	real := filepath.Join(home, "repo", "docx")
	put(t, filepath.Join(real, "SKILL.md"), "theirs")
	if err := os.Symlink(real, filepath.Join(root, "docx")); err != nil {
		t.Fatal(err)
	}
	want := map[string]map[string]string{"pdf": skill("v1"), "docx": skill("v1")}
	_, lines, err := f.sync(want, []string{"pdf", "docx"})
	if err != nil {
		t.Fatal(err)
	}
	_, lines, err = f.sync(want, nil)
	if err != nil {
		t.Fatal(err)
	}
	if read(t, filepath.Join(root, "pdf", "SKILL.md")) != "mine" || read(t, filepath.Join(real, "SKILL.md")) != "theirs" {
		t.Fatal("changed a folder that isn't the plugin's")
	}
	if fi, _ := os.Lstat(filepath.Join(root, "docx")); fi.Mode()&os.ModeSymlink == 0 {
		t.Fatal("symlink replaced")
	}
	if !strings.Contains(lines["pdf"][0], "isn't the One App's") || !strings.Contains(lines["docx"][0], "symlink") {
		t.Fatalf("not reported: %q", lines)
	}
}

func TestSymlinkedSkillsFolderLeftAlone(t *testing.T) {
	f, home := setupHome(t, ".claude")
	real := filepath.Join(home, "dotfiles", "skills")
	os.MkdirAll(real, 0o755)
	if err := os.Symlink(real, filepath.Join(home, ".claude", "skills")); err != nil {
		t.Fatal(err)
	}
	_, lines, err := f.sync(one("pdf", skill("v1")), nil)
	if err != nil {
		t.Fatal(err)
	}
	if entries, _ := os.ReadDir(real); len(entries) != 0 {
		t.Fatal("wrote through the symlink")
	}
	if len(lines[""]) != 1 || !strings.Contains(lines[""][0], "symlink") {
		t.Fatalf("not reported: %q", lines)
	}
}

func TestSymlinkInsideLeftAlone(t *testing.T) {
	f, home := setupHome(t, ".claude")
	dir := filepath.Join(home, ".claude", "skills", "pdf")
	f.sync(one("pdf", skill("v1")), nil)
	outside := filepath.Join(home, "outside.md")
	put(t, outside, "outside")
	os.Remove(filepath.Join(dir, "SKILL.md"))
	os.Symlink(outside, filepath.Join(dir, "SKILL.md"))
	_, lines, err := f.sync(one("pdf", skill("v2")), nil)
	if err != nil {
		t.Fatal(err)
	}
	if read(t, outside) != "outside" {
		t.Fatal("wrote through the symlink")
	}
	if fi, _ := os.Lstat(filepath.Join(dir, "SKILL.md")); fi.Mode()&os.ModeSymlink == 0 {
		t.Fatal("symlink replaced")
	}
	if !strings.Contains(lines["pdf"][0], "symlink") {
		t.Fatalf("not reported: %q", lines)
	}
}

func TestRemoveDeletesOnlyOurs(t *testing.T) {
	f, home := setupHome(t, ".claude", ".codex")
	f.sync(one("pdf", skill("v1")), nil)
	mine := filepath.Join(home, ".claude", "skills", "docx")
	put(t, filepath.Join(mine, "SKILL.md"), "mine")
	if _, _, err := f.sync(nil, []string{"pdf", "docx"}); err != nil {
		t.Fatal(err)
	}
	for _, root := range []string{".claude/skills", ".agents/skills"} {
		if _, err := os.Stat(filepath.Join(home, root, "pdf")); !os.IsNotExist(err) {
			t.Fatalf("%s: not removed: %v", root, err)
		}
	}
	if read(t, filepath.Join(mine, "SKILL.md")) != "mine" {
		t.Fatal("removed a folder that isn't the plugin's")
	}
}

func TestHandEdit(t *testing.T) {
	f, home := setupHome(t, ".claude", ".codex")
	claude := filepath.Join(home, ".claude", "skills", "pdf")
	codex := filepath.Join(home, ".agents", "skills", "pdf")
	f.sync(one("pdf", skill("v1")), nil)
	put(t, filepath.Join(claude, "notes.md"), "by hand")
	edits, _, err := f.sync(one("pdf", skill("v1")), nil)
	if err != nil || edits["pdf"]["notes.md"] != "by hand" || len(edits["pdf"]) != 2 {
		t.Fatalf("edits %v, %v", edits, err)
	}
	if read(t, filepath.Join(claude, "notes.md")) != "by hand" {
		t.Fatal("hand edit overwritten")
	}
	// Recorded as a revision, the edit spreads.
	if edits, _, err := f.sync(edits, nil); err != nil || len(edits) != 0 {
		t.Fatal(edits, err)
	}
	if read(t, filepath.Join(codex, "notes.md")) != "by hand" {
		t.Fatal("edit didn't spread")
	}
	// A new revision from elsewhere reaches folders not edited since.
	if edits, _, _ := f.sync(one("pdf", skill("v3")), nil); len(edits) != 0 {
		t.Fatal(edits)
	}
	if _, err := os.Stat(filepath.Join(claude, "notes.md")); !os.IsNotExist(err) {
		t.Fatal("not replaced")
	}
}

func TestNewestHandEditWins(t *testing.T) {
	f, home := setupHome(t, ".claude", ".codex")
	claude := filepath.Join(home, ".claude", "skills", "pdf", "SKILL.md")
	codex := filepath.Join(home, ".agents", "skills", "pdf", "SKILL.md")
	f.sync(one("pdf", skill("v1")), nil)
	put(t, claude, "older")
	put(t, codex, "newer")
	old := time.Now().Add(-time.Hour)
	os.Chtimes(claude, old, old)
	if edits, _, _ := f.sync(one("pdf", skill("v1")), nil); edits["pdf"]["SKILL.md"] != "newer" {
		t.Fatalf("edits %v", edits)
	}
}

func TestBadHandEditLeftAlone(t *testing.T) {
	f, home := setupHome(t, ".claude")
	dir := filepath.Join(home, ".claude", "skills", "pdf")
	f.sync(one("pdf", skill("v1")), nil)
	put(t, filepath.Join(dir, "logo.png"), "\x89PNG\x00\x01")
	edits, lines, err := f.sync(one("pdf", skill("v1")), nil)
	if err != nil || len(edits) != 0 {
		t.Fatal(edits, err)
	}
	if !strings.Contains(lines["pdf"][0], "isn't text") {
		t.Fatalf("not reported: %q", lines)
	}
	if _, err := os.Stat(filepath.Join(dir, "logo.png")); err != nil {
		t.Fatal("user's file deleted")
	}
}

func TestMissingSkillMdIsRewritten(t *testing.T) {
	f, home := setupHome(t, ".claude")
	dir := filepath.Join(home, ".claude", "skills", "pdf")
	f.sync(one("pdf", skill("v1")), nil)
	os.Remove(filepath.Join(dir, "SKILL.md"))
	if edits, _, _ := f.sync(one("pdf", skill("v1")), nil); len(edits) != 0 {
		t.Fatalf("recorded %v", edits)
	}
	if !strings.Contains(read(t, filepath.Join(dir, "SKILL.md")), "v1") {
		t.Fatal("not rewritten")
	}
}
