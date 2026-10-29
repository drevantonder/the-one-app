package headlines

import (
	"encoding/json"
	"errors"
	"maps"
	"slices"
	"strings"
	"testing"

	"github.com/drevantonder/the-one-app/sdk"
	"github.com/drevantonder/the-one-app/sdk/widget"
)

const news = `New on the Agent Board since you last looked:
the-one-app #53 2026-10-29 05:30Z personal: Core secrets (protocol v9, fca0b23): agents never read them.
the-one-app #54 2026-10-29 05:33Z personal in /home/drevan/the-one-app: Starting: a board hook.
the-one-app/apple #58 2026-10-29 07:33Z mac in the-one-app/apple: Mac app reopens its window after an update.`

func TestNewsPosts(t *testing.T) {
	got := newsPosts(news)
	want := []string{"Core secrets: agents never read them.", "Starting: a board hook.", "Mac app reopens its window after an update."}
	if !slices.Equal(got, want) {
		t.Fatalf("%q", got)
	}
	if newsPosts("Nothing new.") != nil {
		t.Fatal("no news")
	}
}

func TestEmoji(t *testing.T) {
	for in, want := range map[string]string{"🔑": "🔑", " ✈️ ": "✈️", "🇦🇺": "🇦🇺", "key": "", "🔑 Keys": "", "": ""} {
		if got := emoji(in); got != want {
			t.Errorf("emoji(%q) = %q, want %q", in, got, want)
		}
	}
}

// fake is the board and the models: the judge's answers, and what the
// writer answers with each style, in turn.
type fake struct {
	news   string
	judge  map[string]float64 // nil: the judge is offline
	roast  []string
	plain  []string
	chats  map[string][][]map[string]string
	models []string
}

func (f *fake) invoke(plugin, op string, in any) (sdk.Result, error) {
	switch plugin + "_" + op {
	case "board_news":
		return sdk.Text(f.news), nil
	case "models_decide":
		if f.judge == nil {
			return sdk.Result{}, errors.New("offline")
		}
		answers := map[string]any{}
		for q, v := range f.judge {
			answers[q] = map[string]any{"noul": v}
		}
		return sdk.Result{Data: map[string]any{"answers": answers}}, nil
	case "models_chat":
		m := in.(map[string]any)
		f.models = append(f.models, m["model"].(string))
		kind, queue := "news", &f.plain
		if m["system"] == styles["roast"] {
			kind, queue = "roast", &f.roast
		}
		if f.chats == nil {
			f.chats = map[string][][]map[string]string{}
		}
		f.chats[kind] = append(f.chats[kind], m["messages"].([]map[string]string))
		if len(*queue) == 0 {
			return sdk.Result{}, errors.New("offline")
		}
		a := (*queue)[0]
		*queue = (*queue)[1:]
		return sdk.Text(a), nil
	}
	return sdk.Result{}, errors.New("unknown " + plugin + "_" + op)
}

func reply(emoji, text string) string { return `{"emoji":"` + emoji + `","headline":"` + text + `"}` }

var good = map[string]float64{"worth": 0.9, "roast_true": 0.9, "roast_funny": 0.78, "news_true": 0.9}

func with(q string, v float64) map[string]float64 {
	m := maps.Clone(good)
	m[q] = v
	return m
}

func TestNext(t *testing.T) {
	p := &plugin{}
	f := &fake{news: news, judge: good,
		roast: []string{reply("🙈", "No peeking, agents")},
		plain: []string{reply("🔐", "Agents can no longer read your secret keys"), reply("🔐", "Your keys stay secret.")},
	}
	h, err := p.next(f.invoke)
	if err != nil || h == nil || h.line() != "🙈 No peeking, agents" || h.Kind != "roast" || h.Time == "" {
		t.Fatalf("a true, funny roast wins: %+v, %v", h, err)
	}
	retry := f.chats["news"][1]
	if len(retry) != 3 || !strings.Contains(retry[2]["content"], "42 characters, 18 too many") {
		t.Fatalf("the writer should be told how far over it went: %v", retry)
	}
	if c := f.chats["roast"][0][0]["content"]; !strings.Contains(c, "Core secrets: agents never read them.") || strings.Contains(c, "fca0b23") {
		t.Fatalf("prompt: %s", c)
	}
}

func TestPick(t *testing.T) {
	roast, plain := reply("🙈", "No peeking, agents"), reply("🔐", "Your keys stay secret")
	for name, c := range map[string]struct {
		judge        map[string]float64
		roast, plain string
		want         string
	}{
		"true and funny":       {good, roast, plain, "roast"},
		"invented":             {with("roast_true", 0.4), roast, plain, "news"},
		"not funny":            {with("roast_funny", 0.68), roast, plain, "news"},
		"news wrong too":       {map[string]float64{"worth": 0.9, "roast_true": 0.9, "roast_funny": 0.6, "news_true": 0.5}, roast, plain, "roast"},
		"nothing right":        {map[string]float64{"worth": 0.9, "roast_true": 0.5, "news_true": 0.5}, roast, plain, ""},
		"not worth it":         {with("worth", 0.3), roast, plain, ""},
		"no roast written":     {good, "", plain, "news"},
		"judge offline":        {nil, roast, plain, "news"},
		"judge offline, roast": {nil, roast, "", "roast"},
	} {
		f := &fake{news: news, judge: c.judge}
		if c.roast != "" {
			f.roast = []string{c.roast}
		}
		if c.plain != "" {
			f.plain = []string{c.plain}
		}
		h, err := (&plugin{}).next(f.invoke)
		if got := ""; err != nil || (h == nil) != (c.want == "") || h != nil && h.Kind != c.want {
			if h != nil {
				got = h.Kind
			}
			t.Errorf("%s: got %q, %v; want %q", name, got, err, c.want)
		}
	}
}

func TestNoHeadline(t *testing.T) {
	p := &plugin{headlines: []Headline{{Emoji: "🔐", Text: "Your keys stay secret"}}}
	for name, f := range map[string]*fake{
		"nothing new":    {news: "Nothing new."},
		"writers say no": {news: news, judge: good, roast: []string{reply("", "")}, plain: []string{reply("", "")}},
		"repeats":        {news: news, judge: good, roast: []string{reply("🔑", "your keys stay secret")}, plain: []string{reply("🔑", "Your keys stay secret")}},
	} {
		if h, err := p.next(f.invoke); h != nil || err != nil {
			t.Errorf("%s: %+v, %v", name, h, err)
		}
	}
}

func TestWriterOffline(t *testing.T) {
	f := &fake{news: news, judge: good}
	if _, err := (&plugin{}).next(f.invoke); err == nil || !slices.Equal(f.models, []string{writer, writer}) {
		t.Fatalf("only the local writer, and its error: %v, %v", f.models, err)
	}
}

func TestEmojiTwice(t *testing.T) {
	f := &fake{news: news, judge: with("roast_true", 0), plain: []string{reply("💍", "💍 ✨ Ring looks expensive")}}
	if h, _ := (&plugin{}).next(f.invoke); h == nil || h.line() != "💍 Ring looks expensive" {
		t.Fatalf("%+v", h)
	}
}

func TestWidget(t *testing.T) {
	p := &plugin{}
	if _, text := p.widget(sdkSpace(0, 0)); text != "" {
		t.Fatal("no headline, no text")
	}
	p.headlines = []Headline{{Emoji: "🔐", Text: "Your keys stay secret", Time: "2026-10-29T05:30:00Z"}, {Emoji: "🪟", Text: "Mac app remembers window", Time: "2026-10-29T05:40:00Z"}}
	root, text := p.widget(sdkSpace(0, 0))
	if text != "🪟 Mac app remembers window\n🔐 Your keys stay secret" {
		t.Fatalf("%q", text)
	}
	if raw, _ := json.Marshal(root); !strings.Contains(string(raw), "Mac app remembers window") {
		t.Fatal(string(raw))
	}
	if _, text := p.widget(sdkSpace(150, 70)); text != "🪟 Mac app remembers window" {
		t.Fatalf("a small widget shows the newest: %q", text)
	}
}

func sdkSpace(w, h float64) widget.Space { return widget.Space{W: w, H: h} }
