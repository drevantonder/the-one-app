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
	choice string             // the judge's pick of the news
	roast  []string
	plain  []string
	asked  map[string]any
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
		f.asked = in.(map[string]any)
		answers := map[string]any{}
		for q, v := range f.judge {
			answers[q] = map[string]any{"noul": v}
		}
		if f.choice != "" {
			answers["news"] = map[string]any{"choice": f.choice}
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

// written is the writer's answer: headlines as emoji and text, in pairs.
func written(pairs ...string) string {
	var hs []map[string]string
	for i := 0; i+1 < len(pairs); i += 2 {
		hs = append(hs, map[string]string{"emoji": pairs[i], "headline": pairs[i+1]})
	}
	raw, _ := json.Marshal(map[string]any{"headlines": hs})
	return string(raw)
}

var (
	roasts = written("🙈", "No peeking, agents", "🔐", "Keys go to the vault", "🤡", "Agents locked out of everything ever")
	plains = written("🔐", "Secrets kept by the core", "🔑", "Your keys stay secret")
	good   = map[string]float64{"worth": 0.9, "r1_true": 0.9, "r1_funny": 0.72, "r2_true": 0.85, "r2_funny": 0.78, "n1_true": 0.9, "n2_true": 0.9}
)

func with(kv ...any) map[string]float64 {
	m := maps.Clone(good)
	for i := 0; i+1 < len(kv); i += 2 {
		m[kv[i].(string)] = kv[i+1].(float64)
	}
	return m
}

func TestNext(t *testing.T) {
	f := &fake{news: news, judge: good, roast: []string{roasts}, plain: []string{plains}}
	h, err := (&plugin{}).next(f.invoke)
	if err != nil || h == nil || h.line() != "🔐 Keys go to the vault" || h.Kind != "roast" || h.Time == "" {
		t.Fatalf("the funniest true roast wins: %+v, %v", h, err)
	}
	state := f.asked["state"].(map[string]any)
	if state["roast 3"] != nil || state["roast 2"] != "🔐 Keys go to the vault" || state["news 2"] != "🔑 Your keys stay secret" {
		t.Fatalf("the judge sees every take that fits, by name: %v", state)
	}
	if c := f.chats["roast"][0][0]["content"]; !strings.Contains(c, "Core secrets: agents never read them.") || strings.Contains(c, "fca0b23") {
		t.Fatalf("prompt: %s", c)
	}
}

func TestPick(t *testing.T) {
	for name, c := range map[string]struct {
		judge  map[string]float64
		choice string
		roast  string
		want   string
	}{
		"funniest true roast":     {good, "", roasts, "Keys go to the vault"},
		"funniest invents":        {with("r2_true", 0.5), "", roasts, "Secrets kept by the core"},
		"none funny enough":       {with("r2_funny", 0.7), "", roasts, "Secrets kept by the core"},
		"the judge's news":        {with("r2_funny", 0.7), "news 2", roasts, "Your keys stay secret"},
		"its news is wrong":       {with("r2_funny", 0.7, "n2_true", 0.4), "news 2", roasts, "No peeking, agents"},
		"a pick that isn't there": {with("r2_funny", 0.7), "news 9", roasts, "Secrets kept by the core"},
		"nothing right":           {with("r1_true", 0.5, "r2_true", 0.5, "n1_true", 0.5), "", roasts, ""},
		"not worth it":            {with("worth", 0.3), "", roasts, ""},
		"judge offline":           {nil, "", roasts, "Secrets kept by the core"},
		"no roasts written":       {good, "", written(), "Secrets kept by the core"},
	} {
		f := &fake{news: news, judge: c.judge, choice: c.choice, roast: []string{c.roast}, plain: []string{plains}}
		h, err := (&plugin{}).next(f.invoke)
		if got := ""; err != nil || (h == nil) != (c.want == "") || h != nil && h.Text != c.want {
			if h != nil {
				got = h.Text
			}
			t.Errorf("%s: got %q, %v; want %q", name, got, err, c.want)
		}
		if h != nil && (h.Kind == "news") != strings.HasPrefix(c.want, "S") && (h.Kind == "news") != strings.HasPrefix(c.want, "Y") {
			t.Errorf("%s: kind %q", name, h.Kind)
		}
	}
}

func TestTakes(t *testing.T) {
	recent := []Headline{{Emoji: "🔑", Text: "Your keys stay secret"}}
	f := &fake{news: news, judge: with("r2_funny", 0.7), roast: []string{written("🙈", "No peeking, agents", "🙈", "no peeking, agents")}, plain: []string{plains}}
	(&plugin{headlines: recent}).next(f.invoke)
	state := f.asked["state"].(map[string]any)
	if state["roast 2"] != nil || state["news 2"] != nil || state["news 1"] != "🔐 Secrets kept by the core" {
		t.Fatalf("repeats, of each other or of the ticker, are dropped: %v", state)
	}
	long := written("📰", "This one is far too long to fit", "📰", "And so is this one, also too long")
	f = &fake{news: news, judge: good, roast: []string{long, written("🙈", "No peeking, agents")}, plain: []string{long, long}}
	h, err := (&plugin{}).next(f.invoke)
	if err != nil || h == nil || h.Text != "No peeking, agents" || !strings.Contains(f.chats["roast"][1][2]["content"], "Every one is over 24 characters") {
		t.Fatalf("all too long: told once, then dropped: %+v, %v", h, err)
	}
}

func TestNoHeadline(t *testing.T) {
	for name, f := range map[string]*fake{
		"nothing new":    {news: "Nothing new."},
		"writers say no": {news: news, judge: good, roast: []string{written()}, plain: []string{written()}},
	} {
		if h, err := (&plugin{}).next(f.invoke); h != nil || err != nil {
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
	f := &fake{news: news, judge: with("r1_true", 0.0, "r2_true", 0.0), plain: []string{written("💍", "💍 ✨ Ring looks expensive")}}
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
