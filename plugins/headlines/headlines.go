// Package headlines writes a news ticker for the user: every half minute it
// reads what's new on the Agent Board and, when something is worth it, has a
// model write one short headline with an emoji, for the menu bar and the
// lock screen.
package headlines

import (
	"encoding/json"
	"errors"
	"fmt"
	"regexp"
	"slices"
	"strconv"
	"strings"
	"sync"
	"time"
	"unicode"
	"unicode/utf8"

	"github.com/drevantonder/the-one-app/sdk"
	"github.com/drevantonder/the-one-app/sdk/widget"
)

var Plugin = sdk.Plugin{ID: "headlines", Name: "Headlines", Version: "0.1.0", Setup: setup}

const (
	limit = 24 // characters in a headline, spaces included, so it fits the menu bar
	every = 30 * time.Second
	keep  = 50 // headlines kept in memory

	judge    = "typesafe/jev-latest"
	worth    = 0.6  // how sure the judge must be that the posts deserve a headline
	honest   = 0.8  // that a roast doesn't invent
	funny    = 0.75 // that the best roast lands; Jev's answers here run 0.6 to 0.8
	accurate = 0.7  // that the news is right
	session  = "headlines"
	attempts = 2
	takes    = 6 // headlines written of each kind, for the judge to pick from
)

// writer is local, so a headline every half minute costs nothing; a
// subscription's allowance would run out.
const writer = "gemma4:12b"

type Headline struct {
	Emoji string `json:"emoji"`
	Text  string `json:"text"`
	Kind  string `json:"kind,omitempty"` // roast or news
	Time  string `json:"time"`
}

func (h Headline) line() string { return strings.TrimSpace(h.Emoji + " " + h.Text) }

type plugin struct {
	mu        sync.Mutex
	headlines []Headline
}

func setup(c *sdk.Ctx) error {
	p := &plugin{}
	c.Entry("headline", sdk.Available)
	c.OnReset(func() {
		p.mu.Lock()
		p.headlines = nil
		p.mu.Unlock()
	})
	sdk.Handle(c, "headline", func(_ sdk.Entry, h Headline) error {
		p.mu.Lock()
		p.headlines = append(p.headlines, h)
		if len(p.headlines) > keep {
			p.headlines = p.headlines[len(p.headlines)-keep:]
		}
		p.mu.Unlock()
		return nil
	})

	c.Every("check", every, func(call *sdk.Call) error {
		h, err := p.next(call.Invoke)
		if err != nil || h == nil {
			return err
		}
		_, err = call.Append("headline", h)
		return err
	})

	c.Widget("widget", "Headlines", "Show the latest headline about what's going on, as a widget; its text's first line is the headline.", time.Minute, func(_ *sdk.Call, space widget.Space) (widget.Node, string, error) {
		root, text := p.widget(space)
		return root, text, nil
	})
	c.Menubar(func(items []sdk.MenubarItem) []sdk.MenubarItem {
		return append(items, sdk.MenubarItem{ID: "headlines", Tool: "headlines_widget", Bar: true})
	})
	return nil
}

func (p *plugin) recent(n int) []Headline {
	p.mu.Lock()
	defer p.mu.Unlock()
	out := slices.Clone(p.headlines[max(0, len(p.headlines)-n):])
	slices.Reverse(out)
	return out
}

func (p *plugin) widget(space widget.Space) (widget.Node, string) {
	n := 1
	if space.H == 0 || space.H > 200 {
		n = 5
	}
	hs := p.recent(n)
	if len(hs) == 0 {
		return widget.Text("No news yet.", widget.Style{Size: "sm", Tone: "muted"}), ""
	}
	var rows []widget.Node
	var text []string
	for i, h := range hs {
		size, weight := "sm", ""
		if i == 0 {
			size, weight = "md", "medium"
		}
		at, _ := time.Parse(time.RFC3339, h.Time)
		rows = append(rows, widget.Col(widget.Stack{Gap: 2},
			widget.Text(h.line(), widget.Style{Size: size, Weight: weight, Lines: 2}),
			widget.Time(at, "relative", widget.Style{Size: "xs", Tone: "muted"})))
		text = append(text, h.line())
	}
	return widget.Col(widget.Stack{Gap: 10}, rows...), strings.Join(text, "\n")
}

type invoker func(plugin, op string, in any) (sdk.Result, error)

// next reads the board's news and, if any of it deserves one, writes a
// headline. Nothing new, or nothing worth it, is no headline and no error.
func (p *plugin) next(invoke invoker) (*Headline, error) {
	res, err := invoke("board", "news", map[string]any{"session": session})
	if err != nil {
		return nil, err
	}
	posts := newsPosts(res.Text)
	if len(posts) == 0 {
		return nil, nil
	}
	var recent []string
	for _, h := range p.recent(5) {
		recent = append(recent, h.Text)
	}
	roasts, errRoast := write(invoke, writer, styles["roast"], posts, recent)
	news, errNews := write(invoke, writer, styles["news"], posts, recent)
	if len(roasts) == 0 && len(news) == 0 {
		return nil, errors.Join(errRoast, errNews)
	}
	for i := range roasts {
		roasts[i].Kind = "roast"
	}
	for i := range news {
		news[i].Kind = "news"
	}
	h := pick(invoke, posts, roasts, news)
	if h != nil {
		h.Time = time.Now().UTC().Format(time.RFC3339)
	}
	return h, nil
}

var (
	newsLine = regexp.MustCompile(`^\S+ #\d+ \S+ \S+ [^\s:]+(?: in [^:]+)?: (.*)$`)
	aside    = regexp.MustCompile(`\s*\([^)]*\)`)
)

// newsPosts is the text of each post in board_news, without what only an
// agent needs: commit ids and asides in brackets.
func newsPosts(text string) []string {
	var out []string
	for _, l := range strings.Split(text, "\n") {
		m := newsLine.FindStringSubmatch(l)
		if m == nil {
			continue
		}
		out = append(out, strings.TrimSpace(aside.ReplaceAllString(m[1], "")))
	}
	return out
}

// pick has the judge say whether the posts deserve a headline and which of
// the candidates to show: the funniest roast that is true, if it's funny
// enough, else the news that best tells the main point, if it's right. A
// pick-one question ranks the news well, but would pick a roast that
// invents, so roasts are each asked whether they're true and funny. If the
// judge can't answer, the first news is shown.
func pick(invoke invoker, posts []string, roasts, news []Headline) *Headline {
	state := map[string]any{"posts": strings.Join(posts, "\n")}
	questions := map[string]any{"worth": map[string]any{"type": "noul", "instructions": "Are these posts worth a glance on Andre's news ticker: something now works, was fixed or changed, real work started, or a bug or blunder worth a laugh? No for rules for agents, notes on preferences, compressions and routine status."}}
	for i, h := range roasts {
		name := fmt.Sprintf("roast %d", i+1)
		state[name] = h.line()
		questions[fmt.Sprintf("r%d_true", i+1)] = map[string]any{"type": "noul", "instructions": "Is " + name + " true to what the posts say (a joke may exaggerate, but not invent)?"}
		questions[fmt.Sprintf("r%d_funny", i+1)] = map[string]any{"type": "noul", "instructions": "Would " + name + " make a tech person smile?"}
	}
	options := map[string]string{}
	for i, h := range news {
		name := fmt.Sprintf("news %d", i+1)
		state[name] = h.line()
		options[name] = name
		questions[fmt.Sprintf("n%d_true", i+1)] = map[string]any{"type": "noul", "instructions": "Is " + name + " accurate to the posts?"}
	}
	if len(news) > 1 {
		questions["news"] = map[string]any{"type": "choice", "instructions": "Which news headline best tells the main point of the posts, accurately?", "criteria": options}
	}
	first := func() *Headline {
		if len(news) > 0 {
			return &news[0]
		}
		return &roasts[0]
	}
	res, err := invoke("models", "decide", map[string]any{"model": judge, "state": state, "questions": questions})
	if err != nil {
		return first()
	}
	raw, _ := json.Marshal(res.Data)
	var d struct {
		Answers map[string]struct {
			Noul   float64 `json:"noul"`
			Choice string  `json:"choice"`
		} `json:"answers"`
	}
	if json.Unmarshal(raw, &d) != nil || len(d.Answers) == 0 {
		return first()
	}
	a := func(q string, i int) float64 { return d.Answers[fmt.Sprintf(q, i+1)].Noul }
	if d.Answers["worth"].Noul < worth {
		return nil
	}
	best := -1
	for i := range roasts {
		if a("r%d_true", i) >= honest && (best < 0 || a("r%d_funny", i) > a("r%d_funny", best)) {
			best = i
		}
	}
	if best >= 0 && a("r%d_funny", best) >= funny {
		return &roasts[best]
	}
	n := 0
	if c, ok := strings.CutPrefix(d.Answers["news"].Choice, "news "); ok {
		if i, err := strconv.Atoi(c); err == nil && i >= 1 && i <= len(news) {
			n = i - 1
		}
	}
	if n < len(news) && a("n%d_true", n) >= accurate {
		return &news[n]
	}
	if best >= 0 {
		return &roasts[best]
	}
	return nil
}

const ticker = "You write headlines for the one-line news ticker in the Mac menu bar of Andre, who builds The One App with a crew of AI agents that post to a shared board. "

var rules = fmt.Sprintf(`
From the new posts, take the one thing that matters most, done or just started; work just started is underway, not done.
Write %d different headlines, each its own take. Rules for each: 3 to 5 words, at most %d characters, spaces included; one emoji that fits; no code names or commit ids. If none of the posts is worth a glance, give no headlines.
Examples:
`, takes, limit)

// styles are the writer's prompts, one for each kind of headline, so each
// commits: a roast is all joke, news is all fact. Both are written for every
// update, several takes of each, and the judge picks.
var styles = map[string]string{
	"roast": ticker + `Find the hook in these posts, a bug, a repeat, a self-own, an irony, and make fun of it, like a cheeky tech tabloid. Commit to the joke; no plain summary. Moves that work: a pun, a deadpan 'take three' or 'thrice' for a repeat, 'works on my machine', a bug as a person (a zombie plugin, a stray VM squatting), a grand claim undercut. Mean to code and agents, never to Andre. The joke must be about what the posts really say.` + rules + `Post: Hex dictation picked the closed MacBook's mic, not the Beats.
Headline: 🎙️ Mic listens to shut lid
Post: Retired the memory plugin: the Agent Board is agents' shared memory now.
Headline: 🪦 Memory plugin forgotten
Post: Widgets checked everywhere. Bug: the RAM column header wraps to RA/M.
Headline: 🐏 RAM shrinks to RA/M
Post: Fixed two copies of the app after each update: every copy relaunched itself.
Headline: 👯 Mac app quits twinning
Post: The node restarts a plugin whose process dies, at once, then with backoff.
Headline: 🧟 Dead plugins now rise
Post: The Mac app had no main menu, so Cmd-V reached no view: added Edit and Window menus.
Headline: 📋 Mac app learns to paste`,
	"news": ticker + `Say plainly and exactly what changed for Andre, like a wire-service headline: present tense, the most useful fact, no jokes. Only state what the posts say.` + rules + `Post: New claude plugin: coding devices get Claude Code kept installed.
Headline: 🤖 Claude Code on every box
Post: Secrets: plugins read the ones they name; agents never read them.
Headline: 🔐 Keys hidden from agents
Post: Devices is now a plugin: per-node RAM ring, CPU and GPU sparklines, disk bar.
Headline: 📊 Devices get sparklines
Post: Brand: The One App. Icon: a 3D gold One Ring inscribed with device and agent icons.
Headline: 💍 App gets a One Ring icon`,
}

var answers = json.RawMessage(fmt.Sprintf(`{"type":"object","properties":{"headlines":{"type":"array","maxItems":%d,"items":{"type":"object","properties":{"emoji":{"type":"string"},"headline":{"type":"string"}},"required":["emoji","headline"]}}},"required":["headlines"]}`, takes))

// write has model write takes headlines in one style, and keeps those that
// fit and aren't already on the ticker. If none fit, it is told so once.
func write(invoke invoker, model, style string, posts, recent []string) ([]Headline, error) {
	prompt := "New posts:\n" + strings.Join(posts, "\n")
	if len(recent) > 0 {
		prompt += "\n\nAlready on the ticker, don't repeat: " + strings.Join(recent, "; ")
	}
	msgs := []map[string]string{{"role": "user", "content": prompt}}
	for range attempts {
		res, err := invoke("models", "chat", map[string]any{"model": model, "system": style, "messages": msgs, "schema": answers})
		if err != nil {
			return nil, err
		}
		var a struct {
			Headlines []struct {
				Emoji    string `json:"emoji"`
				Headline string `json:"headline"`
			} `json:"headlines"`
		}
		if err := json.Unmarshal([]byte(res.Text), &a); err != nil {
			return nil, fmt.Errorf("not JSON: %q", res.Text)
		}
		if len(a.Headlines) == 0 {
			return nil, nil
		}
		var out []Headline
		long := 0
		seen := map[string]bool{}
		for _, h := range recent {
			seen[strings.ToLower(h)] = true
		}
		for _, h := range a.Headlines {
			text := strings.TrimRight(strings.TrimLeftFunc(h.Headline, notWord), ".")
			if utf8.RuneCountInString(text) > limit {
				long++
				continue
			}
			if text == "" || seen[strings.ToLower(text)] {
				continue
			}
			seen[strings.ToLower(text)] = true
			out = append(out, Headline{Emoji: emoji(h.Emoji), Text: text})
		}
		if len(out) > 0 || long < len(a.Headlines) {
			return out, nil
		}
		msgs = append(msgs, map[string]string{"role": "assistant", "content": res.Text},
			map[string]string{"role": "user", "content": fmt.Sprintf("Every one is over %d characters. Write %d again, each at most %d.", limit, takes, limit)})
	}
	return nil, fmt.Errorf("every headline over %d characters, %d times", limit, attempts)
}

// notWord is what a model sometimes puts before a headline's first word:
// space, or the emoji again.
func notWord(r rune) bool {
	return !unicode.IsLetter(r) && !unicode.IsDigit(r) && r != '\'' && r != '"'
}

// emoji keeps s if it is one emoji, or a short sequence like a flag or
// family, and drops it otherwise.
func emoji(s string) string {
	s = strings.TrimSpace(s)
	if s == "" || utf8.RuneCountInString(s) > 8 {
		return ""
	}
	for _, r := range s {
		if unicode.IsLetter(r) || unicode.IsDigit(r) || unicode.IsSpace(r) || unicode.IsPunct(r) {
			return ""
		}
	}
	return s
}
