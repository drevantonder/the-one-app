package skills

import (
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
	"time"
)

type tool struct {
	name   string
	dir    string // the tool's folder; its skills are written only if it exists
	skills string
}

type target struct {
	tools []string
	dir   string
}

// targets are the user-level skills folders of this device's installed
// agent tools, each once, so no tool reads a skill twice from them.
func targets(home string) []target {
	codex := os.Getenv("CODEX_HOME")
	if codex == "" {
		codex = filepath.Join(home, ".codex")
	}
	config := filepath.Join(home, ".config")
	shared := filepath.Join(home, ".agents", "skills")
	tools := []tool{
		// Claude Code: ~/.claude/skills/<name>/SKILL.md; it doesn't read
		// ~/.agents/skills. https://code.claude.com/docs/en/skills
		{"Claude Code", filepath.Join(home, ".claude"), filepath.Join(home, ".claude", "skills")},
		// Codex: $HOME/.agents/skills is its only user-level folder.
		// https://developers.openai.com/codex/skills
		{"Codex", codex, shared},
		// OpenCode reads ~/.config/opencode/skills, ~/.claude/skills and
		// ~/.agents/skills; the shared one is used. With Claude Code also
		// installed it sees the same skill twice, identical.
		// https://opencode.ai/docs/skills/
		{"OpenCode", filepath.Join(config, "opencode"), shared},
		// Amp reads ~/.config/agents/skills, ~/.agents/skills,
		// ~/.config/amp/skills, then ~/.claude/skills; the first skill with a
		// name wins. https://ampcode.com/manual/agent-skills
		{"Amp", filepath.Join(config, "amp"), shared},
	}
	var out []target
	at := map[string]int{}
	for _, t := range tools {
		if fi, err := os.Stat(t.dir); err != nil || !fi.IsDir() {
			continue
		}
		if i, ok := at[t.skills]; ok {
			out[i].tools = append(out[i].tools, t.name)
			continue
		}
		at[t.skills] = len(out)
		out = append(out, target{[]string{t.name}, t.skills})
	}
	return out
}

// files syncs the skills with this device's skills folders. state is the
// plugin's own folder, which remembers what each skill folder last held when
// it matched its skill, so a difference from that is a hand edit.
type files struct {
	home  string // the user's home; os.UserHomeDir if empty
	state string
}

// sync writes each skill in want to every skills folder, except where a
// folder has been edited by hand since it was last in sync, and deletes the
// plugin's folders of the skills in gone. It returns the newest hand edit of
// each skill, for the caller to record as a revision (then sync again).
// lines say what was done, by skill; those under "" aren't about one skill.
func (f *files) sync(want map[string]map[string]string, gone []string) (edits map[string]map[string]string, lines map[string][]string, err error) {
	edits, lines = map[string]map[string]string{}, map[string][]string{}
	home := f.home
	if home == "" {
		if home, err = os.UserHomeDir(); err != nil {
			return nil, nil, err
		}
	}
	synced := map[string]string{}
	statePath := filepath.Join(f.state, "synced.json")
	if raw, err := os.ReadFile(statePath); err == nil {
		_ = json.Unmarshal(raw, &synced)
	}
	newest := map[string]time.Time{}
	var errs []error
	for _, t := range targets(home) {
		who := strings.Join(t.tools, ", ")
		if link := symlinked(home, t.dir); link != "" {
			lines[""] = append(lines[""], fmt.Sprintf("%s: %s is a symlink, left alone", who, link))
			continue
		}
		for _, name := range gone {
			dir := filepath.Join(t.dir, name)
			if ours(dir) {
				if err := os.RemoveAll(dir); err != nil {
					errs = append(errs, err)
					continue
				}
			}
			delete(synced, dir)
		}
		for _, name := range sorted(want) {
			dir := filepath.Join(t.dir, name)
			say := func(format string, args ...any) {
				lines[name] = append(lines[name], fmt.Sprintf("%s: %s ", who, dir)+fmt.Sprintf(format, args...))
			}
			fi, err := os.Lstat(dir)
			switch {
			case errors.Is(err, fs.ErrNotExist):
			case err != nil:
				errs = append(errs, err)
				continue
			case fi.Mode()&fs.ModeSymlink != 0:
				say("is a symlink, left alone")
				continue
			case !fi.IsDir():
				say("isn't a folder, left alone")
				continue
			case !ours(dir):
				say("isn't the One App's (no %s in it), left alone", marker)
				continue
			default:
				cur, at, err := readSkill(name, dir)
				if errors.Is(err, errSymlink) {
					say("has %v, left alone", err)
					continue
				}
				if err == nil && same(cur, want[name]) {
					synced[dir] = hash(cur)
					say("is current")
					continue
				}
				last, known := synced[dir]
				if _, ok := cur["SKILL.md"]; known && ok && hash(cur) != last {
					if err != nil {
						// Left for the user to fix, rather than lost.
						say("was edited by hand, but %v; left alone", err)
						continue
					}
					say("was edited by hand")
					if at.After(newest[name]) {
						newest[name], edits[name] = at, cur
					}
					continue
				}
			}
			if err := writeSkill(dir, want[name]); err != nil {
				errs = append(errs, err)
				continue
			}
			synced[dir] = hash(want[name])
			say("written")
		}
	}
	raw, _ := json.Marshal(synced)
	if err := os.MkdirAll(f.state, 0o700); err != nil {
		errs = append(errs, err)
	} else if err := write(statePath, string(raw)); err != nil {
		errs = append(errs, err)
	}
	return edits, lines, errors.Join(errs...)
}

// symlinked returns the first folder from home down to dir that is a
// symlink, if any.
func symlinked(home, dir string) string {
	rel, err := filepath.Rel(home, dir)
	if err != nil {
		return dir
	}
	p := home
	for _, part := range strings.Split(rel, string(filepath.Separator)) {
		p = filepath.Join(p, part)
		if fi, err := os.Lstat(p); err == nil && fi.Mode()&fs.ModeSymlink != 0 {
			return p
		}
	}
	return ""
}

// ours says whether dir is a real folder the plugin made.
func ours(dir string) bool {
	fi, err := os.Lstat(dir)
	if err != nil || !fi.IsDir() {
		return false
	}
	fi, err = os.Lstat(filepath.Join(dir, marker))
	return err == nil && fi.Mode().IsRegular()
}

// writeSkill makes dir hold exactly files, plus the marker and any hidden
// files already there. A new skill is put together beside it first, so an
// agent never sees half of one.
func writeSkill(dir string, files map[string]string) error {
	if _, err := os.Lstat(dir); errors.Is(err, fs.ErrNotExist) {
		if err := os.MkdirAll(filepath.Dir(dir), 0o755); err != nil {
			return err
		}
		tmp, err := os.MkdirTemp(filepath.Dir(dir), ".theoneapp-*")
		if err != nil {
			return err
		}
		defer os.RemoveAll(tmp)
		if err := os.Chmod(tmp, 0o755); err != nil {
			return err
		}
		if err := fill(tmp, files); err != nil {
			return err
		}
		return os.Rename(tmp, dir)
	}
	if err := fill(dir, files); err != nil {
		return err
	}
	// Drop files the skill no longer has, then emptied folders, deepest first.
	var stale, dirs []string
	err := filepath.WalkDir(dir, func(p string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		rel, _ := filepath.Rel(dir, p)
		if rel == "." {
			return nil
		}
		if strings.HasPrefix(d.Name(), ".") {
			if d.IsDir() {
				return filepath.SkipDir
			}
			return nil
		}
		if d.IsDir() {
			dirs = append(dirs, p)
		} else if _, ok := files[filepath.ToSlash(rel)]; !ok && d.Type().IsRegular() {
			stale = append(stale, p)
		}
		return nil
	})
	if err != nil {
		return err
	}
	for _, p := range stale {
		if err := os.Remove(p); err != nil {
			return err
		}
	}
	for i := len(dirs) - 1; i >= 0; i-- {
		_ = os.Remove(dirs[i]) // fails, as it should, unless empty
	}
	return nil
}

func fill(dir string, files map[string]string) error {
	if !ours(dir) {
		if err := write(filepath.Join(dir, marker), "Made by the One App's skills plugin, which keeps this skill the same on every device. Edits here are kept as a new revision; delete this file to take the folder over."); err != nil {
			return err
		}
	}
	for _, p := range sorted(files) {
		path := filepath.Join(dir, filepath.FromSlash(p))
		if fi, err := os.Lstat(path); err == nil && fi.Mode()&fs.ModeSymlink != 0 {
			return fmt.Errorf("%s is a symlink", path)
		}
		cur, err := os.ReadFile(path)
		if err == nil && string(cur) == files[p] {
			continue
		}
		if err := write(path, files[p]); err != nil {
			return err
		}
	}
	return nil
}

// write replaces path with text through a temporary file, so an agent never
// reads half a file.
func write(path, text string) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	tmp, err := os.CreateTemp(filepath.Dir(path), ".theoneapp-*")
	if err != nil {
		return err
	}
	defer os.Remove(tmp.Name())
	if _, err := tmp.WriteString(text); err != nil {
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
