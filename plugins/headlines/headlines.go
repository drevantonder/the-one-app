// Package headlines writes a news ticker for the user: every half minute it
// reads what's new on the Agent Board and, when something is worth it, has a
// model write one short headline with an emoji, for the menu bar and the
// lock screen.
package headlines

import (
	"encoding/json"
	"fmt"
	"regexp"
	"slices"
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

	gate     = "typesafe/jev-latest"
	worth    = 0.6 // how sure the gate must be that the news deserves a headline
	session  = "headlines"
	attempts = 3
)

// writers are tried in order: a local model costs nothing, a service is
// there for when its device is off.
var writers = []string{"gemma4:12b", "opencode-go/deepseek-v4.1-flash"}

type Headline struct {
	Emoji string `json:"emoji"`
	Text  string `json:"text"`
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
	if !worthIt(invoke, posts) {
		return nil, nil
	}
	var recent []string
	for _, h := range p.recent(5) {
		recent = append(recent, h.Text)
	}
	var errs []string
	for _, model := range writers {
		h, err := write(invoke, model, posts, recent)
		if err == nil {
			if h != nil {
				h.Time = time.Now().UTC().Format(time.RFC3339)
			}
			return h, nil
		}
		errs = append(errs, model+": "+err.Error())
	}
	return nil, fmt.Errorf("no model wrote a headline: %s", strings.Join(errs, "; "))
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

// worthIt asks the gate whether the posts deserve a headline; if it can't
// answer, the writer decides.
func worthIt(invoke invoker, posts []string) bool {
	res, err := invoke("models", "decide", map[string]any{
		"model": gate,
		"state": map[string]any{"posts": strings.Join(posts, "\n")},
		"questions": map[string]any{"news": map[string]any{"type": "noul",
			"instructions": "Would the person who owns these apps and agents find any of these posts worth a glance on a news ticker: something now works, was fixed or changed, or an agent started real work on something? No for rules for agents, notes on how someone likes things, compressions of old posts, and routine status."}},
	})
	if err != nil {
		return true
	}
	raw, _ := json.Marshal(res.Data)
	var d struct {
		Answers struct {
			News struct {
				Noul *float64 `json:"noul"`
			} `json:"news"`
		} `json:"answers"`
	}
	if json.Unmarshal(raw, &d) != nil || d.Answers.News.Noul == nil {
		return true
	}
	return *d.Answers.News.Noul >= worth
}

const style = `You write the headline for a one-line news ticker in the Mac menu bar of the person who owns these apps and agents. From the new posts, pick the one thing that matters most to them, done or just started, and say it in 3 or 4 plain words, like a newspaper headline: present tense, no code names, commit ids, version numbers or full stop. Only state what a post says; work just started is underway, not done.
Examples of the style, from other news: Bridge reopens to traffic, Rail strike called off, City bans night flights.
Also pick the one emoji, from all of Unicode, that best shows what this story is about.
The headline must be at most %d characters, spaces included. If none of the posts is news to them, give an empty headline.`

var answer = json.RawMessage(`{"type":"object","properties":{"emoji":{"type":"string"},"headline":{"type":"string"}},"required":["emoji","headline"]}`)

// write has model write a headline, telling it how far over the limit it
// went until it fits. An empty headline means the model found no news.
func write(invoke invoker, model string, posts, recent []string) (*Headline, error) {
	prompt := "New posts:\n" + strings.Join(posts, "\n")
	if len(recent) > 0 {
		prompt += "\n\nAlready on the ticker, don't repeat: " + strings.Join(recent, "; ")
	}
	msgs := []map[string]string{{"role": "user", "content": prompt}}
	var last string
	for range attempts {
		res, err := invoke("models", "chat", map[string]any{"model": model, "system": fmt.Sprintf(style, limit), "messages": msgs, "schema": answer})
		if err != nil {
			return nil, err
		}
		var a struct {
			Emoji    string `json:"emoji"`
			Headline string `json:"headline"`
		}
		if err := json.Unmarshal([]byte(res.Text), &a); err != nil {
			return nil, fmt.Errorf("not JSON: %q", res.Text)
		}
		text := strings.TrimRight(strings.TrimSpace(a.Headline), ".")
		if text == "" {
			return nil, nil
		}
		if n := utf8.RuneCountInString(text); n > limit {
			last = text
			msgs = append(msgs, map[string]string{"role": "assistant", "content": res.Text},
				map[string]string{"role": "user", "content": fmt.Sprintf("%q is %d characters, %d too many. Say the same thing in at most %d.", text, n, n-limit, limit)})
			continue
		}
		for _, h := range recent {
			if strings.EqualFold(h, text) {
				return nil, nil
			}
		}
		return &Headline{Emoji: emoji(a.Emoji), Text: text}, nil
	}
	return nil, fmt.Errorf("still too long after %d tries: %q", attempts, last)
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
