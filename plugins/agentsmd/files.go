package agentsmd

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
	"time"
)

type target struct {
	tool string
	dir  string // the tool's folder; written only if it exists
	file string
}

// targets are the user-level AGENTS.md files agents read. Each tool's own
// folder is the test of whether it is installed.
func targets(home string) []target {
	codex := os.Getenv("CODEX_HOME")
	if codex == "" {
		codex = filepath.Join(home, ".codex")
	}
	config := filepath.Join(home, ".config")
	return []target{
		// Codex: $CODEX_HOME/AGENTS.md, ~/.codex by default.
		{"Codex", codex, filepath.Join(codex, "AGENTS.md")},
		// Amp: ~/.config/amp/AGENTS.md. Amp also reads ~/.config/AGENTS.md,
		// which is left alone so Amp doesn't read the document twice.
		{"Amp", filepath.Join(config, "amp"), filepath.Join(config, "amp", "AGENTS.md")},
		// OpenCode: ~/.config/opencode/AGENTS.md.
		{"OpenCode", filepath.Join(config, "opencode"), filepath.Join(config, "opencode", "AGENTS.md")},
	}
}

// files syncs the document with this device's AGENTS.md files. state is the
// plugin's own folder, which remembers what each file last held when it
// matched the document, so a difference from that is a hand edit.
type files struct {
	home  string // the user's home; os.UserHomeDir if empty
	state string
}

func hash(s string) string {
	h := sha256.Sum256([]byte(s))
	return hex.EncodeToString(h[:])
}

// sync writes doc to every file that hasn't been edited by hand since it was
// last in sync, and returns the newest hand edit, if any, for the caller to
// record as a revision (then sync again). An empty doc, never set, leaves
// every file alone. lines say what was done with each file.
func (f *files) sync(doc string) (edit string, lines []string, err error) {
	if doc == "" {
		return "", nil, nil
	}
	home := f.home
	if home == "" {
		if home, err = os.UserHomeDir(); err != nil {
			return "", nil, err
		}
	}
	synced := map[string]string{}
	statePath := filepath.Join(f.state, "synced.json")
	if raw, err := os.ReadFile(statePath); err == nil {
		_ = json.Unmarshal(raw, &synced)
	}
	var newest time.Time
	var errs []error
	for _, t := range targets(home) {
		if fi, err := os.Stat(t.dir); err != nil || !fi.IsDir() {
			continue
		}
		fi, err := os.Lstat(t.file)
		if err == nil && fi.Mode()&fs.ModeSymlink != 0 {
			lines = append(lines, fmt.Sprintf("%s: %s is a symlink, left alone", t.tool, t.file))
			continue
		}
		if err == nil && !fi.Mode().IsRegular() {
			lines = append(lines, fmt.Sprintf("%s: %s isn't a regular file, left alone", t.tool, t.file))
			continue
		}
		var cur string
		if err == nil {
			raw, err := os.ReadFile(t.file)
			if err != nil {
				errs = append(errs, err)
				continue
			}
			cur = strings.TrimSpace(string(raw))
		}
		if cur == doc {
			synced[t.file] = hash(doc)
			lines = append(lines, fmt.Sprintf("%s: %s is current", t.tool, t.file))
			continue
		}
		last, known := synced[t.file]
		if known && cur != "" && hash(cur) != last {
			lines = append(lines, fmt.Sprintf("%s: %s was edited by hand", t.tool, t.file))
			if fi.ModTime().After(newest) {
				newest, edit = fi.ModTime(), cur
			}
			continue
		}
		if !known && cur != "" {
			// A file from before this plugin: keep a copy, once.
			backup := filepath.Join(f.state, "before", strings.ToLower(t.tool)+"-AGENTS.md")
			if _, err := os.Stat(backup); errors.Is(err, fs.ErrNotExist) {
				if err := write(backup, cur); err != nil {
					errs = append(errs, err)
					continue
				}
			}
		}
		if err := write(t.file, doc); err != nil {
			errs = append(errs, err)
			continue
		}
		synced[t.file] = hash(doc)
		lines = append(lines, fmt.Sprintf("%s: %s written", t.tool, t.file))
	}
	raw, _ := json.Marshal(synced)
	if err := write(statePath, string(raw)); err != nil {
		errs = append(errs, err)
	}
	return edit, lines, errors.Join(errs...)
}

// write replaces path with text through a temporary file, so an agent never
// reads half a file.
func write(path, text string) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return err
	}
	tmp, err := os.CreateTemp(filepath.Dir(path), ".agentsmd-*")
	if err != nil {
		return err
	}
	defer os.Remove(tmp.Name())
	if _, err := tmp.WriteString(text + "\n"); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	if err := os.Chmod(tmp.Name(), 0o644); err != nil {
		return err
	}
	return os.Rename(tmp.Name(), path)
}
