package skills

import (
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"time"
	"unicode/utf8"
)

const (
	maxSize  = 256 << 10 // per skill, so the log stays small
	maxFiles = 100
	// marker is in every skill folder the plugin made, and only those.
	marker = ".theoneapp"
)

// The Agent Skills name rule: lowercase letters, digits and single hyphens.
var validName = regexp.MustCompile(`^[a-z0-9]+(-[a-z0-9]+)*$`)

var errSymlink = errors.New("a symlink in it")

func checkName(name string) error {
	if len(name) > 64 || !validName.MatchString(name) {
		return fmt.Errorf("%q isn't a skill name: use lowercase letters, digits and hyphens, at most 64", name)
	}
	return nil
}

// check refuses what doesn't belong in the log: binary files, odd paths and
// skills over the size cap.
func check(name string, files map[string]string) error {
	if err := checkName(name); err != nil {
		return err
	}
	if _, ok := files["SKILL.md"]; !ok {
		return fmt.Errorf("%s has no SKILL.md", name)
	}
	if len(files) > maxFiles {
		return fmt.Errorf("%s has %d files; the limit is %d", name, len(files), maxFiles)
	}
	size := 0
	for p, text := range files {
		if p == "" || path.IsAbs(p) || path.Clean(p) != p || p == ".." || strings.HasPrefix(p, "../") || strings.Contains(p, `\`) {
			return fmt.Errorf("%q isn't a relative path inside the skill", p)
		}
		for _, part := range strings.Split(p, "/") {
			if strings.HasPrefix(part, ".") {
				return fmt.Errorf("%q: skills don't keep hidden files or folders", p)
			}
		}
		if !utf8.ValidString(text) || strings.ContainsRune(text, 0) {
			return fmt.Errorf("%s isn't text; skills keep text files only", p)
		}
		size += len(text)
	}
	if size > maxSize {
		return fmt.Errorf("%s is %d KB; the limit is %d KB", name, (size+1023)/1024, maxSize/1024)
	}
	return nil
}

// readSkill reads a skill folder as check would accept it, and the newest
// change in it. Hidden files and folders, the marker among them, are skipped.
func readSkill(name, dir string) (map[string]string, time.Time, error) {
	files := map[string]string{}
	var newest time.Time
	size := 0
	err := filepath.WalkDir(dir, func(p string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		rel, _ := filepath.Rel(dir, p)
		rel = filepath.ToSlash(rel)
		if rel != "." && strings.HasPrefix(d.Name(), ".") {
			if d.IsDir() {
				return filepath.SkipDir
			}
			return nil
		}
		fi, err := d.Info()
		if err != nil {
			return err
		}
		if fi.ModTime().After(newest) {
			newest = fi.ModTime()
		}
		switch {
		case d.Type()&fs.ModeSymlink != 0:
			return fmt.Errorf("%w: %s", errSymlink, rel)
		case d.IsDir():
			return nil
		case !d.Type().IsRegular():
			return fmt.Errorf("%s isn't a regular file", rel)
		}
		if size += int(fi.Size()); size > maxSize {
			return fmt.Errorf("%s is over %d KB", name, maxSize/1024)
		}
		raw, err := os.ReadFile(p)
		if err != nil {
			return err
		}
		files[rel] = string(raw)
		return nil
	})
	if err != nil {
		return files, newest, err
	}
	return files, newest, check(name, files)
}

func hash(files map[string]string) string {
	h := sha256.New()
	for _, p := range sorted(files) {
		fmt.Fprintf(h, "%s\x00%s\x00", p, files[p])
	}
	return hex.EncodeToString(h.Sum(nil))
}

func same(a, b map[string]string) bool {
	if len(a) != len(b) {
		return false
	}
	for p, t := range a {
		if u, ok := b[p]; !ok || u != t {
			return false
		}
	}
	return true
}

func sorted[V any](m map[string]V) []string {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	return keys
}

// frontmatter reads a SKILL.md's top-level fields, enough for its name and
// description: plain, quoted and block (| or >) values, joined into a line.
func frontmatter(doc string) map[string]string {
	fields := map[string]string{}
	lines := strings.Split(strings.ReplaceAll(doc, "\r\n", "\n"), "\n")
	if len(lines) == 0 || strings.TrimSpace(lines[0]) != "---" {
		return fields
	}
	key := ""
	for _, l := range lines[1:] {
		if strings.TrimSpace(l) == "---" {
			break
		}
		if key != "" && (strings.HasPrefix(l, " ") || strings.HasPrefix(l, "\t")) {
			fields[key] = strings.TrimSpace(fields[key] + " " + strings.TrimSpace(l))
			continue
		}
		k, v, ok := strings.Cut(l, ":")
		if !ok || strings.HasPrefix(l, " ") {
			key = ""
			continue
		}
		key, v = strings.TrimSpace(k), strings.TrimSpace(v)
		if strings.HasPrefix(v, "|") || strings.HasPrefix(v, ">") {
			v = ""
		} else if len(v) >= 2 && (v[0] == '"' || v[0] == '\'') && v[len(v)-1] == v[0] {
			v = v[1 : len(v)-1]
		}
		fields[key] = v
	}
	return fields
}
