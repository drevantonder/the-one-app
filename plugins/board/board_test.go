package board

import (
	"fmt"
	"strings"
	"testing"

	"github.com/drevantonder/the-one-app/sdk/optmem"
)

const at = "2026-10-29T14:05:00+11:00"

func create(t *testing.T, b *Board, path, desc string) {
	t.Helper()
	p, err := b.CanCreate(path, desc, false)
	if err != nil {
		t.Fatal(err)
	}
	b.Create(p, desc, at)
}

func post(b *Board, path, text string) {
	b.Post(fmt.Sprintf("mac:%d", len(b.posts)), postEntry{Topic: path, Text: text, Time: at, Device: "mac", Where: "the-one-app"})
}

// napAll answers every compression due in every topic.
func napAll(b *Board) {
	for _, t := range b.topics {
		for {
			todo := t.log.Pending(1)
			if len(todo) == 0 {
				break
			}
			t.log.PutSummary(todo[0], t.log.Digest(todo[0]), "summary of "+todo[0].String())
		}
	}
}

func TestPaths(t *testing.T) {
	for in, want := range map[string]string{"": "", " / ": "", "projects": "projects", "/projects/the-one-app/": "projects/the-one-app", "a1/b-2": "a1/b-2"} {
		if got, err := ParsePath(in); err != nil || got != want {
			t.Errorf("ParsePath(%q) = %q, %v; want %q", in, got, err, want)
		}
	}
	for _, bad := range []string{"Projects", "a//b", "a-", "-a", "a--b", "a b", "a_b", "a/./b"} {
		if _, err := ParsePath(bad); err == nil {
			t.Errorf("ParsePath(%q) should fail", bad)
		}
	}
}

func TestCreateNeedsAParentAndOneLine(t *testing.T) {
	b := New()
	if _, err := b.CanCreate("projects/the-one-app", "x", false); err == nil || !strings.Contains(err.Error(), "no topic projects") {
		t.Fatalf("missing parent: %v", err)
	}
	if _, err := b.CanCreate("", "x", false); err == nil {
		t.Fatal("the top can't be created")
	}
	if _, err := b.CanCreate("projects", "two\nlines", false); err == nil {
		t.Fatal("a description is one line")
	}
	create(t, b, "projects", "Work projects")
	create(t, b, "projects/the-one-app", "The One App")
	if _, err := b.CanCreate("projects", "again", true); err == nil || !strings.Contains(err.Error(), "already exists") {
		t.Fatalf("existing topic: %v", err)
	}
}

func TestDuplicateGuard(t *testing.T) {
	b := New()
	create(t, b, "projects", "Work projects")
	create(t, b, "projects/the-one-app", "The One App")
	create(t, b, "projects/the-one-app/whatsapp", "The WhatsApp plugin")
	create(t, b, "life", "The user's life")
	for _, path := range []string{"life/whatsapp", "whats-app", "life/whatsapps", "life/one-app", "life/whatsap"} {
		_, err := b.CanCreate(path, "x", false)
		if err == nil || !strings.Contains(err.Error(), "look like the same thing") || !strings.Contains(err.Error(), "force") {
			t.Errorf("%s: %v, want a refusal", path, err)
		}
	}
	_, err := b.CanCreate("life/whatsapp", "x", false)
	if !strings.Contains(err.Error(), "projects/the-one-app/whatsapp — The WhatsApp plugin") {
		t.Errorf("the refusal should show the existing topic: %v", err)
	}
	if p, err := b.CanCreate("life/whatsapp", "x", true); err != nil || p != "life/whatsapp" {
		t.Errorf("force: %q, %v", p, err)
	}
	for _, path := range []string{"life/health", "projects/app", "projects/what", "life/messages"} {
		if _, err := b.CanCreate(path, "x", false); err != nil {
			t.Errorf("%s: %v, want it created", path, err)
		}
	}
}

func TestTopicsTree(t *testing.T) {
	b := New()
	create(t, b, "projects", "Work projects")
	create(t, b, "projects/the-one-app", "The One App")
	post(b, "projects/the-one-app", "started the board")
	post(b, "", "the user lives in Sydney")
	want := "(top) — 1 post — active 2026-10-29 03:05Z — mac in the-one-app: the user lives in Sydney\n" +
		"  projects — Work projects — no posts — active 2026-10-29 03:05Z\n" +
		"    projects/the-one-app — The One App — 1 post — active 2026-10-29 03:05Z — mac in the-one-app: started the board"
	if got := b.Topics(); got != want {
		t.Fatalf("topics:\n%s\nwant:\n%s", got, want)
	}
}

func TestReadEndsWithSubtopics(t *testing.T) {
	b := New()
	if out, _ := b.Read("", 0, 0); out != "No posts yet. Post the first with board_post.\nNo subtopics." {
		t.Fatalf("empty top:\n%s", out)
	}
	create(t, b, "projects", "Work projects")
	create(t, b, "projects/the-one-app", "The One App")
	create(t, b, "projects/the-one-app/whatsapp", "The WhatsApp plugin")
	post(b, "", "read projects before working")
	post(b, "projects/the-one-app", "first")
	post(b, "projects/the-one-app", "second")

	top, _ := b.Read("", 0, 0)
	if !strings.HasPrefix(top, "#0 2026-10-29 03:05Z mac in the-one-app: read projects before working\n") ||
		!strings.HasSuffix(top, "Subtopics:\n  projects — Work projects — active 2026-10-29 03:05Z") {
		t.Fatalf("top:\n%s", top)
	}
	app, _ := b.Read("projects/the-one-app", 0, 0)
	for _, want := range []string{"projects/the-one-app: The One App\n", "#1 2026-10-29 03:05Z mac in the-one-app: second\n",
		"Subtopics:\n  projects/the-one-app/whatsapp — The WhatsApp plugin — active", "board_nap with topic \"projects/the-one-app\", block \"0-1\""} {
		if !strings.Contains(app, want) {
			t.Errorf("read is missing %q:\n%s", want, app)
		}
	}
	if strings.Contains(app, "Work projects") {
		t.Errorf("read shows more than its own posts and direct subtopics:\n%s", app)
	}
	if _, err := b.Read("projects/nope", 0, 0); err == nil || !strings.Contains(err.Error(), "or use projects") {
		t.Errorf("unknown topic: %v", err)
	}
}

func TestReadBudgets(t *testing.T) {
	b := New()
	create(t, b, "projects", "Work projects")
	for i := range 300 {
		post(b, "", fmt.Sprintf("top %d", i))
		post(b, "projects", fmt.Sprintf("projects %d", i))
	}
	if out, _ := b.Read("projects", 0, 0); !strings.HasPrefix(out, "projects: Work projects\nCannot wake") || !strings.Contains(out, `call board_read with topic "projects" again`) {
		t.Fatalf("uncompressed topic:\n%s", out)
	}
	napAll(b)
	for path, budget := range map[string]int{"": TopLines, "projects": TopicLines} {
		out, _ := b.Read(path, 0, 0)
		if n := strings.Count(out, "\n#"); n > budget || n < budget-8 {
			t.Errorf("%q printed %d lines, budget %d:\n%s", path, n, budget, out)
		}
	}
}

func TestNapGoesToTheBusiestTopic(t *testing.T) {
	b := New()
	create(t, b, "projects", "Work projects")
	create(t, b, "life", "The user's life")
	for i := range 4 {
		post(b, "projects", fmt.Sprint(i))
	}
	post(b, "life", "a")
	post(b, "life", "b")
	if nap := b.NextNap("life"); !strings.Contains(nap, `topic "life"`) {
		t.Fatalf("own topic first:\n%s", nap)
	}
	life := b.topics["life"].log
	life.PutSummary(optmem.Block{Lo: 0, Hi: 2}, life.Digest(optmem.Block{Lo: 0, Hi: 2}), "a and b")
	if nap := b.NextNap("life"); !strings.Contains(nap, `topic "projects", block "0-1"`) || !strings.Contains(nap, `one board nap --topic projects --block 0-1 --text "<your line>"`) {
		t.Fatalf("then the busiest:\n%s", nap)
	}
	napAll(b)
	if nap := b.NextNap(""); nap != "" {
		t.Fatalf("nothing due, got:\n%s", nap)
	}
}

func TestRecallAcrossTopics(t *testing.T) {
	b := New()
	create(t, b, "projects", "Work projects")
	post(b, "", "the user likes tea")
	post(b, "projects", "chose sqlite for the store")
	post(b, "projects", "tea break")
	all, _ := b.Recall("tea", "")
	want := "(top) #0 2026-10-29 03:05Z mac in the-one-app: the user likes tea\nprojects #1 2026-10-29 03:05Z mac in the-one-app: tea break\n2 matches."
	if all != want {
		t.Fatalf("recall:\n%s\nwant:\n%s", all, want)
	}
	if one, _ := b.Recall("tea", "projects"); one != "#1 2026-10-29 03:05Z mac in the-one-app: tea break\n1 match." {
		t.Fatalf("recall in projects:\n%s", one)
	}
	if hit, _ := b.Recall("^projects", ""); !strings.Contains(hit, "2 matches") {
		t.Fatalf("topic names are searchable:\n%s", hit)
	}
}

// Replaying the same entries in the same order must give the same board,
// even when a post arrives before its topic's creation.
func TestReplayIsDeterministic(t *testing.T) {
	replay := func() *Board {
		b := New()
		b.Post("linux:1", postEntry{Topic: "projects/x", Text: "early", Time: at, Device: "linux", Thread: "t3://1"})
		b.Create("projects", "Work projects", at)
		b.Create("projects/x", "X", "2026-10-28T09:00:00Z")
		b.Create("projects/x", "X again", at)
		for i := range 20 {
			post(b, "projects", fmt.Sprint(i))
		}
		napAll(b)
		return b
	}
	a, c := replay(), replay()
	for _, path := range []string{"", "projects", "projects/x"} {
		ra, _ := a.Read(path, 0, 0)
		rc, _ := c.Read(path, 0, 0)
		if ra != rc {
			t.Fatalf("%q reads differ:\n%s\n---\n%s", path, ra, rc)
		}
	}
	if a.Topics() != c.Topics() {
		t.Fatalf("topics differ:\n%s\n---\n%s", a.Topics(), c.Topics())
	}
	x, _ := a.Read("projects/x", 0, 0)
	if !strings.HasPrefix(x, "projects/x: X\n#0 2026-10-29 03:05Z linux: early (thread: t3://1)") {
		t.Fatalf("projects/x:\n%s", x)
	}
}

func TestPostTimesShareOneZone(t *testing.T) {
	b := New()
	b.Post("a", postEntry{Text: "from the mac", Time: "2026-10-29T12:45:00+10:00", Device: "mac"})
	b.Post("b", postEntry{Text: "from personal", Time: "2026-10-29T02:46:00Z", Device: "personal"})
	s, err := b.Read("", 0, 0)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(s, "2026-10-29 02:45Z mac") || !strings.Contains(s, "2026-10-29 02:46Z personal") {
		t.Fatalf("times not in UTC:\n%s", s)
	}
}
