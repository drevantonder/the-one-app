package headlines

import (
	"encoding/json"
	"errors"
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

// fake is the board and the models: the gate's answer, and the writer's
// answers in turn.
type fake struct {
	news    string
	gate    float64
	answers []string
	chats   [][]map[string]string
	models  []string
}

func (f *fake) invoke(plugin, op string, in any) (sdk.Result, error) {
	switch plugin + "_" + op {
	case "board_news":
		return sdk.Text(f.news), nil
	case "models_decide":
		return sdk.Result{Data: map[string]any{"answers": map[string]any{"news": map[string]any{"noul": f.gate}}}}, nil
	case "models_chat":
		m := in.(map[string]any)
		f.models = append(f.models, m["model"].(string))
		f.chats = append(f.chats, m["messages"].([]map[string]string))
		if len(f.answers) == 0 {
			return sdk.Result{}, errors.New("offline")
		}
		a := f.answers[0]
		f.answers = f.answers[1:]
		return sdk.Text(a), nil
	}
	return sdk.Result{}, errors.New("unknown " + plugin + "_" + op)
}

func TestNext(t *testing.T) {
	p := &plugin{}
	f := &fake{news: news, gate: 0.9, answers: []string{
		`{"emoji":"🔐","headline":"Agents can no longer read your secret keys"}`,
		`{"emoji":"🔐","headline":"Your keys stay secret."}`,
	}}
	h, err := p.next(f.invoke)
	if err != nil || h == nil {
		t.Fatal(h, err)
	}
	if h.line() != "🔐 Your keys stay secret" || h.Time == "" {
		t.Fatalf("%+v", h)
	}
	retry := f.chats[1]
	if len(retry) != 3 || !strings.Contains(retry[2]["content"], "42 characters, 18 too many") {
		t.Fatalf("the writer should be told how far over it went: %v", retry)
	}
	if !strings.Contains(f.chats[0][0]["content"], "Core secrets: agents never read them.") || strings.Contains(f.chats[0][0]["content"], "fca0b23") {
		t.Fatalf("prompt: %s", f.chats[0][0]["content"])
	}
}

func TestNoHeadline(t *testing.T) {
	p := &plugin{headlines: []Headline{{Emoji: "🔐", Text: "Your keys stay secret"}}}
	for name, f := range map[string]*fake{
		"nothing new":    {news: "Nothing new."},
		"gate says no":   {news: news, gate: 0.3, answers: []string{`{"emoji":"🔐","headline":"Keys"}`}},
		"writer says no": {news: news, gate: 0.9, answers: []string{`{"emoji":"","headline":""}`}},
		"a repeat":       {news: news, gate: 0.9, answers: []string{`{"emoji":"🔑","headline":"Your keys stay secret"}`}},
	} {
		if h, err := p.next(f.invoke); h != nil || err != nil {
			t.Errorf("%s: %+v, %v", name, h, err)
		}
	}
}

func TestWriterOffline(t *testing.T) {
	p := &plugin{}
	f := &fake{news: news, gate: 0.9}
	if _, err := p.next(f.invoke); err == nil || !slices.Equal(f.models, []string{writer}) {
		t.Fatalf("only the local writer, and its error: %v, %v", f.models, err)
	}
}

func TestEmojiTwice(t *testing.T) {
	p := &plugin{}
	f := &fake{news: news, gate: 0.9, answers: []string{`{"emoji":"💍","headline":"💍 ✨ Ring looks expensive"}`}}
	if h, _ := p.next(f.invoke); h == nil || h.line() != "💍 Ring looks expensive" {
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
