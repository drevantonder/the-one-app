// Package skills is the built-in plugin keeping the user's agent skills (the
// Agent Skills format: a folder with a SKILL.md) the same on every device:
// it writes each skill to every installed agent's user-level skills folder,
// and records hand edits to the folders it made as revisions.
package skills

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/drevantonder/the-one-app/sdk"
)

// revision is one version of a skill; nil Files means it was removed.
type revision struct {
	Entry string
	Time  string
	Files map[string]string
}

type setIn struct {
	Name  string            `json:"name" jsonschema:"the skill's name, also its folder's: lowercase letters, digits and hyphens"`
	Files map[string]string `json:"files" jsonschema:"every file of the skill, by path relative to its folder (SKILL.md at least), as text; replaces the whole skill"`
}

type nameIn struct {
	Name string `json:"name" jsonschema:"the skill's name, as listed by list"`
}

type getIn struct {
	Name     string `json:"name" jsonschema:"the skill's name, as listed by list"`
	Revision int    `json:"revision,omitempty" jsonschema:"an earlier revision number, as listed by history; the current one if empty"`
}

type importIn struct {
	Path string `json:"path" jsonschema:"absolute path (or ~/...) of a skill folder, holding SKILL.md, on the device the call runs on"`
	Name string `json:"name,omitempty" jsonschema:"the skill's name; the folder's name if empty"`
}

type noIn struct{}

var Plugin = sdk.Plugin{ID: "skills", Name: "Skills", Version: "0.1.0", Setup: setup}

func setup(c *sdk.Ctx) error {
	var (
		mu     sync.Mutex
		skills = map[string][]revision{}
		report map[string][]string
		ready  bool
	)
	kick := make(chan struct{}, 1)
	poke := func() {
		select {
		case kick <- struct{}{}:
		default:
		}
	}
	current := func(name string) map[string]string {
		mu.Lock()
		defer mu.Unlock()
		if revs := skills[name]; len(revs) > 0 {
			return revs[len(revs)-1].Files
		}
		return nil
	}
	count := func(name string) int { mu.Lock(); defer mu.Unlock(); return len(skills[name]) }

	c.Entry("set", sdk.Available)
	c.Entry("remove", sdk.Available)
	c.OnReset(func() { mu.Lock(); skills = map[string][]revision{}; mu.Unlock() })
	sdk.Handle(c, "set", func(e sdk.Entry, in setIn) error {
		mu.Lock()
		skills[in.Name] = append(skills[in.Name], revision{Entry: e.ID, Time: e.Time, Files: in.Files})
		mu.Unlock()
		poke()
		return nil
	})
	sdk.Handle(c, "remove", func(e sdk.Entry, in nameIn) error {
		mu.Lock()
		skills[in.Name] = append(skills[in.Name], revision{Entry: e.ID, Time: e.Time})
		mu.Unlock()
		poke()
		return nil
	})

	save := func(call *sdk.Call, name string, files map[string]string) (string, error) {
		if err := check(name, files); err != nil {
			return "", err
		}
		if same(files, current(name)) {
			return "Unchanged.", nil
		}
		if _, err := call.Append("set", setIn{Name: name, Files: files}); err != nil {
			return "", err
		}
		return fmt.Sprintf("Saved %s (%d files) as revision %d.", name, len(files), count(name)), nil
	}
	onDevice := func(name string, all bool) []string {
		mu.Lock()
		defer mu.Unlock()
		var out []string
		for _, l := range report[name] {
			if all || !strings.HasSuffix(l, " is current") && !strings.HasSuffix(l, " written") {
				out = append(out, l)
			}
		}
		return out
	}

	sdk.Op(c, "list", "List the user's agent skills, kept the same on every device, with their descriptions.", sdk.Read, func(*sdk.Call, noIn) (sdk.Result, error) {
		mu.Lock()
		names := sorted(skills)
		mu.Unlock()
		var b strings.Builder
		for _, name := range names {
			files := current(name)
			if files == nil {
				continue
			}
			fmt.Fprintf(&b, "%s: %s\n", name, frontmatter(files["SKILL.md"])["description"])
			for _, l := range onDevice(name, false) {
				fmt.Fprintf(&b, "  %s\n", l)
			}
		}
		if b.Len() == 0 {
			b.WriteString("No skills yet. Add one with skills_import or skills_set.\n")
		}
		if lines := onDevice("", true); len(lines) > 0 {
			b.WriteString("\nOn this device:\n" + strings.Join(lines, "\n") + "\n")
		}
		return sdk.Text(strings.TrimRight(b.String(), "\n")), nil
	})
	sdk.Op(c, "get", "Show one of the user's agent skills: every file in it, and where it is written on this device.", sdk.Read, func(_ *sdk.Call, in getIn) (sdk.Result, error) {
		mu.Lock()
		revs := skills[in.Name]
		mu.Unlock()
		n := len(revs)
		if in.Revision == 0 {
			in.Revision = n
		}
		if n == 0 || in.Revision < 1 || in.Revision > n {
			return sdk.Result{}, fmt.Errorf("no skill %q revision %d; see skills_list and skills_history", in.Name, in.Revision)
		}
		files := revs[in.Revision-1].Files
		if files == nil {
			return sdk.Text(fmt.Sprintf("%s was removed in revision %d.", in.Name, in.Revision)), nil
		}
		var b strings.Builder
		fmt.Fprintf(&b, "%s, revision %d of %d\n", in.Name, in.Revision, n)
		for _, p := range sorted(files) {
			fmt.Fprintf(&b, "\n=== %s ===\n%s\n", p, strings.TrimRight(files[p], "\n"))
		}
		if lines := onDevice(in.Name, true); len(lines) > 0 && in.Revision == n {
			b.WriteString("\n---\nOn this device:\n" + strings.Join(lines, "\n"))
		}
		return sdk.Text(strings.TrimRight(b.String(), "\n")), nil
	})
	sdk.Op(c, "import", "Add or update a skill from a skill folder on the device the call runs on. Only do this when the user asks; earlier versions stay in history.", sdk.Write, func(call *sdk.Call, in importIn) (sdk.Result, error) {
		dir := in.Path
		if rest, ok := strings.CutPrefix(dir, "~/"); ok || dir == "~" {
			home, err := os.UserHomeDir()
			if err != nil {
				return sdk.Result{}, err
			}
			dir = filepath.Join(home, rest)
		}
		if !filepath.IsAbs(dir) {
			return sdk.Result{}, fmt.Errorf("%q: give the folder's absolute path", in.Path)
		}
		name := in.Name
		if name == "" {
			name = filepath.Base(filepath.Clean(dir))
		}
		if err := checkName(name); err != nil {
			return sdk.Result{}, err
		}
		real, err := filepath.EvalSymlinks(dir)
		if err != nil {
			return sdk.Result{}, err
		}
		files, _, err := readSkill(name, real)
		if err != nil {
			return sdk.Result{}, fmt.Errorf("%s: %w", dir, err)
		}
		text, err := save(call, name, files)
		return sdk.Text(text), err
	})
	sdk.Op(c, "set", "Replace one of the user's agent skills, or add one, with every file given. Only do this when the user asks; earlier versions stay in history.", sdk.Write, func(call *sdk.Call, in setIn) (sdk.Result, error) {
		text, err := save(call, in.Name, in.Files)
		return sdk.Text(text), err
	})
	sdk.Op(c, "remove", "Remove one of the user's agent skills, and the folders made for it, from every device. Only do this when the user asks; it stays in history.", sdk.Write, func(call *sdk.Call, in nameIn) (sdk.Result, error) {
		if current(in.Name) == nil {
			return sdk.Result{}, fmt.Errorf("no skill %q", in.Name)
		}
		if _, err := call.Append("remove", in); err != nil {
			return sdk.Result{}, err
		}
		return sdk.Text(fmt.Sprintf("Removed %s; set or import it again to bring it back.", in.Name)), nil
	})
	sdk.Op(c, "history", "List every revision of one of the user's agent skills, newest first.", sdk.Read, func(_ *sdk.Call, in nameIn) (sdk.Result, error) {
		mu.Lock()
		defer mu.Unlock()
		revs := skills[in.Name]
		if len(revs) == 0 {
			return sdk.Result{}, fmt.Errorf("no skill %q", in.Name)
		}
		var b strings.Builder
		for i := len(revs) - 1; i >= 0; i-- {
			what := "removed"
			if f := revs[i].Files; f != nil {
				size := 0
				for _, t := range f {
					size += len(t)
				}
				what = fmt.Sprintf("%d files, %d bytes", len(f), size)
			}
			fmt.Fprintf(&b, "%d  %s  %s\n", i+1, revs[i].Time, what)
		}
		return sdk.Text(strings.TrimRight(b.String(), "\n")), nil
	})

	// Folders are written only from here, once the log has been replayed: on
	// ready, after each new entry, and every minute to catch hand edits.
	c.OnReady(func(call *sdk.Call) {
		mu.Lock()
		started := ready
		ready = true
		mu.Unlock()
		if started {
			return
		}
		dir, err := c.Dir()
		if err != nil {
			c.Logf("skills: %v", err)
			return
		}
		f := &files{state: dir}
		call = call.Detached()
		go func() {
			tick := time.NewTicker(time.Minute)
			defer tick.Stop()
			warned := map[string]bool{}
			for {
				want := map[string]map[string]string{}
				var gone []string
				mu.Lock()
				for name, revs := range skills {
					if files := revs[len(revs)-1].Files; files != nil {
						want[name] = files
					} else {
						gone = append(gone, name)
					}
				}
				mu.Unlock()
				edits, lines, err := f.sync(want, gone)
				if err != nil {
					c.Logf("skills: %v", err)
				}
				for _, ls := range lines {
					for _, l := range ls {
						if strings.Contains(l, "left alone") && !warned[l] {
							warned[l] = true
							c.Logf("skills: %s", l)
						}
					}
				}
				mu.Lock()
				report = lines
				mu.Unlock()
				saved := false
				for _, name := range sorted(edits) {
					if same(edits[name], want[name]) {
						continue
					}
					if _, err := save(call, name, edits[name]); err != nil {
						c.Logf("skills: recording a hand edit of %s: %v", name, err)
						continue
					}
					saved = true
				}
				if saved {
					continue
				}
				select {
				case <-tick.C:
				case <-kick:
				}
			}
		}()
	})
	return nil
}
