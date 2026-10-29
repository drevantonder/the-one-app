package skills

import (
	"strings"
	"testing"
)

func TestCheck(t *testing.T) {
	ok := map[string]string{"SKILL.md": "hi", "scripts/a.py": "print(1)"}
	if err := check("pdf-tools", ok); err != nil {
		t.Fatal(err)
	}
	for _, c := range []struct {
		name  string
		files map[string]string
		want  string
	}{
		{"PDF", ok, "isn't a skill name"},
		{"a--b", ok, "isn't a skill name"},
		{"pdf", map[string]string{"README.md": "x"}, "no SKILL.md"},
		{"pdf", map[string]string{"SKILL.md": "x", "../a": "x"}, "isn't a relative path"},
		{"pdf", map[string]string{"SKILL.md": "x", "/a": "x"}, "isn't a relative path"},
		{"pdf", map[string]string{"SKILL.md": "x", ".theoneapp": "x"}, "hidden"},
		{"pdf", map[string]string{"SKILL.md": "x", "a.png": "\x89PNG\x00"}, "isn't text"},
		{"pdf", map[string]string{"SKILL.md": strings.Repeat("x", maxSize+1)}, "limit is 256 KB"},
	} {
		if err := check(c.name, c.files); err == nil || !strings.Contains(err.Error(), c.want) {
			t.Errorf("%s %v: %v, want %q", c.name, c.files, err, c.want)
		}
	}
}

func TestFrontmatter(t *testing.T) {
	doc := "---\nname: pdf\ndescription: >-\n  Fill PDF forms.\n  Use for PDFs.\nlicense: \"MIT\"\n---\n# PDF\ndescription: not this\n"
	f := frontmatter(doc)
	if f["name"] != "pdf" || f["description"] != "Fill PDF forms. Use for PDFs." || f["license"] != "MIT" {
		t.Fatalf("%q", f)
	}
	if len(frontmatter("# no frontmatter")) != 0 {
		t.Fatal("read fields without frontmatter")
	}
}
