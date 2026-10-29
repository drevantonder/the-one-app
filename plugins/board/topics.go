package board

import (
	"fmt"
	"regexp"
	"sort"
	"strings"
	"time"

	"github.com/drevantonder/the-one-app/sdk/optmem"
)

const (
	TopLines   = optmem.WakeLines // lines a read of the top prints
	TopicLines = 48               // lines a read of any other topic prints
	stamp      = "2006-01-02 15:04Z"
)

type topic struct {
	path, desc string
	last       time.Time // the latest post, or the topic's creation
	log        *optmem.Log
}

type ref struct {
	topic *topic
	i     int
}

// Board is the state rebuilt from the plugin's log.
type Board struct {
	topics map[string]*topic
	posts  []ref // every post, in log order
}

func New() *Board {
	b := &Board{topics: map[string]*topic{}}
	b.ensure("")
	return b
}

var segment = regexp.MustCompile(`^[a-z0-9]+(-[a-z0-9]+)*$`)

// ParsePath checks a topic path; "" is the top.
func ParsePath(s string) (string, error) {
	s = strings.Trim(strings.TrimSpace(s), "/")
	if s == "" {
		return "", nil
	}
	for seg := range strings.SplitSeq(s, "/") {
		if !segment.MatchString(seg) {
			return "", fmt.Errorf("%q is not a topic path: use lowercase segments of letters, digits and hyphens, joined by /, like projects/the-one-app", s)
		}
	}
	return s, nil
}

func parent(path string) string {
	i := strings.LastIndex(path, "/")
	if i < 0 {
		return ""
	}
	return path[:i]
}

func base(path string) string { return path[strings.LastIndex(path, "/")+1:] }

func label(path string) string {
	if path == "" {
		return "the top"
	}
	return path
}

// show names a topic in lists, where the top has no path.
func show(path string) string {
	if path == "" {
		return "(top)"
	}
	return path
}

func words(path string) optmem.Words {
	arg, cli := "", ""
	if path != "" {
		arg, cli = fmt.Sprintf(`topic "%s", `, path), "--topic "+path+" "
	}
	w := optmem.Words{
		One: "post", Many: "posts in " + path, Log: "the topic", Write: "post", Title: "Topic " + path,
		Empty: "No posts yet. Post the first with board_post.",
		Again: "call board_read again",
		Nap: func(b string) string {
			return fmt.Sprintf(`board_nap with %sblock "%s" and text "<your line>" (CLI: one board nap %s--block %s --text "<your line>")`, arg, b, cli, b)
		},
		Next: func(part, at int) string {
			return fmt.Sprintf("Call board_read with %spart %d and at %d (CLI: one board read %s--part %d --at %d)", arg, part, at, cli, part, at)
		},
	}
	if path == "" {
		w.Many, w.Log, w.Title = "posts at the top", "the top", "The top of the board"
	} else {
		w.Again = fmt.Sprintf(`call board_read with topic "%s" again`, path)
	}
	return w
}

// ensure returns the topic at path, adding it if it is new.
func (b *Board) ensure(path string) *topic {
	if t, ok := b.topics[path]; ok {
		return t
	}
	t := &topic{path: path, log: optmem.New(words(path))}
	t.log.Budget = TopicLines
	if path == "" {
		t.log.Budget = TopLines
	}
	b.topics[path] = t
	return t
}

// when is a post's time in UTC: devices stamp posts in their own zones, and
// every device must print them alike.
func when(s string) time.Time {
	t, _ := time.Parse(time.RFC3339, s)
	return t.UTC()
}

func (t *topic) touch(at time.Time) {
	if at.After(t.last) {
		t.last = at
	}
}

// Create adds a topic. A topic created twice keeps its first description.
func (b *Board) Create(path, desc, at string) {
	_, existed := b.topics[path]
	t := b.ensure(path)
	if !existed || t.desc == "" {
		t.desc = desc
	}
	t.touch(when(at))
}

// Post adds a post to its topic, which a post from a merged log may reach
// before the topic's creation does.
func (b *Board) Post(id string, p postEntry) {
	t := b.ensure(p.Topic)
	at := when(p.Time)
	t.touch(at)
	text := p.Text
	if p.Thread != "" {
		text += " (thread: " + p.Thread + ")"
	}
	who := p.Device
	if p.Where != "" {
		who += " in " + p.Where
	}
	if who != "" {
		text = strings.TrimSpace(who) + ": " + text
	}
	t.log.Add(id, at.Format(stamp), text)
	b.posts = append(b.posts, ref{t, t.log.T() - 1})
}

// find returns an existing topic.
func (b *Board) find(path string) (*topic, error) {
	path, err := ParsePath(path)
	if err != nil {
		return nil, err
	}
	if t, ok := b.topics[path]; ok {
		return t, nil
	}
	near := parent(path)
	for b.topics[near] == nil {
		near = parent(near)
	}
	return nil, fmt.Errorf("no topic %s: create it with board_create, or use %s", path, label(near))
}

// CanCreate checks a new topic, returning its clean path.
func (b *Board) CanCreate(path, desc string, force bool) (string, error) {
	path, err := ParsePath(path)
	if err != nil {
		return "", err
	}
	if path == "" {
		return "", fmt.Errorf("the top always exists: name the topic, like projects/the-one-app")
	}
	if desc = strings.TrimSpace(desc); desc == "" || strings.ContainsAny(desc, "\r\n") || len(desc) > optmem.EntryChars {
		return "", fmt.Errorf("the description must be one line of at most %d bytes", optmem.EntryChars)
	}
	if t, ok := b.topics[path]; ok {
		return "", fmt.Errorf("%s already exists: %s", path, t.desc)
	}
	if p := parent(path); b.topics[p] == nil {
		return "", fmt.Errorf("no topic %s: create it first", p)
	}
	if force {
		return path, nil
	}
	if like := b.Similar(path); len(like) > 0 {
		lines := []string{"Not created: these topics look like the same thing."}
		for _, t := range like {
			lines = append(lines, "  "+t.path+" — "+t.desc)
		}
		lines = append(lines, "Use one of them, or call board_create again with force true if yours is different.")
		return "", fmt.Errorf("%s", strings.Join(lines, "\n"))
	}
	return path, nil
}

// Similar lists topics anywhere whose last segment is the same as path's,
// or close to it.
func (b *Board) Similar(path string) []*topic {
	var out []*topic
	for _, t := range b.sorted() {
		if t.path != "" && t.path != path && alike(base(t.path), base(path)) {
			out = append(out, t)
		}
	}
	return out
}

// alike says whether two segments name the same thing: one's words run
// inside the other's, as one-app does in the-one-app, or they are the same
// but for hyphens, a plural or a typo.
func alike(a, b string) bool {
	wa, wb := "-"+a+"-", "-"+b+"-"
	norm := func(s string) string { return strings.TrimSuffix(strings.ReplaceAll(s, "-", ""), "s") }
	a, b = norm(a), norm(b)
	short := min(len(a), len(b))
	switch {
	case a == b:
		return true
	case short >= 4 && (strings.Contains(wa, wb) || strings.Contains(wb, wa)):
		return true
	case short >= 8:
		return distance(a, b) <= 2
	case short >= 4:
		return distance(a, b) <= 1
	}
	return false
}

// distance is the Levenshtein distance between a and b.
func distance(a, b string) int {
	prev := make([]int, len(b)+1)
	for j := range prev {
		prev[j] = j
	}
	for i := 1; i <= len(a); i++ {
		cur := make([]int, len(b)+1)
		cur[0] = i
		for j := 1; j <= len(b); j++ {
			cost := 1
			if a[i-1] == b[j-1] {
				cost = 0
			}
			cur[j] = min(prev[j]+1, cur[j-1]+1, prev[j-1]+cost)
		}
		prev = cur
	}
	return prev[len(b)]
}

func (b *Board) sorted() []*topic {
	out := make([]*topic, 0, len(b.topics))
	for _, t := range b.topics {
		out = append(out, t)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].path < out[j].path })
	return out
}

func active(t *topic) string {
	if t.last.IsZero() {
		return "never active"
	}
	return "active " + t.last.Format(stamp)
}

// Topics prints every topic as a tree.
func (b *Board) Topics() string {
	var lines []string
	for _, t := range b.sorted() {
		parts := []string{show(t.path)}
		if t.desc != "" {
			parts = append(parts, t.desc)
		}
		switch n := t.log.T(); n {
		case 0:
			parts = append(parts, "no posts")
		case 1:
			parts = append(parts, "1 post")
		default:
			parts = append(parts, fmt.Sprintf("%d posts", n))
		}
		parts = append(parts, active(t))
		if h := t.log.Headline(); h != "" {
			parts = append(parts, h)
		}
		depth := 0
		if t.path != "" {
			depth = strings.Count(t.path, "/") + 1
		}
		lines = append(lines, strings.Repeat("  ", depth)+strings.Join(parts, " — "))
	}
	return strings.Join(lines, "\n")
}

// Directory lists a topic's direct subtopics, one line each.
func (b *Board) Directory(path string) string {
	var lines []string
	for _, t := range b.sorted() {
		if t.path == "" || parent(t.path) != path {
			continue
		}
		about := t.log.Headline()
		if about == "" {
			about = t.desc
		}
		lines = append(lines, "  "+t.path+" — "+about+" — "+active(t))
	}
	if len(lines) == 0 {
		return "No subtopics."
	}
	return "Subtopics:\n" + strings.Join(lines, "\n")
}

// Read is a topic's wake, ending with its subtopics.
func (b *Board) Read(path string, part, at int) (string, error) {
	t, err := b.find(path)
	if err != nil {
		return "", err
	}
	s, err := t.log.Wake(part, at, b.Directory(t.path))
	if err != nil {
		return "", err
	}
	if t.path != "" {
		s = t.path + ": " + t.desc + "\n" + s
	}
	return s, nil
}

// NextNap asks for the next compression in path, or else in the topic
// with the most of them due.
func (b *Board) NextNap(path string) string {
	if t, ok := b.topics[path]; ok {
		if nap := t.log.NextNap(); nap != "" {
			return nap
		}
	}
	var busiest *topic
	most := 0
	for _, t := range b.sorted() {
		if n := len(t.log.Pending(0)); n > most {
			busiest, most = t, n
		}
	}
	if busiest == nil {
		return ""
	}
	return busiest.log.NextNap()
}

// Recall searches one topic, or every topic, naming the topic of each hit.
func (b *Board) Recall(pattern, path string) (string, error) {
	if strings.TrimSpace(path) != "" {
		t, err := b.find(path)
		if err != nil {
			return "", err
		}
		return t.log.Recall(pattern)
	}
	return optmem.Search(pattern, func(yield func(string) bool) {
		for _, r := range b.posts {
			if !yield(show(r.topic.path) + " " + r.topic.log.Line(r.i)) {
				return
			}
		}
	})
}
