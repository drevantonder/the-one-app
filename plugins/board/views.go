package board

import (
	"fmt"
	"slices"
	"strings"
	"time"

	"github.com/drevantonder/the-one-app/sdk"
	"github.com/drevantonder/the-one-app/sdk/widget"
)

// channel is a topic's name in the app, where the top of the board is
// #general.
func channel(path string) string {
	if path == "" {
		return "general"
	}
	return path
}

// latest is the newest n posts, newest first, in topic or, with all, in every
// topic.
func (b *Board) latest(n int, topic string, all bool) []ref {
	var out []ref
	for _, r := range b.posts {
		if all || r.topic.path == topic {
			out = append(out, r)
		}
	}
	slices.SortStableFunc(out, func(x, y ref) int { return y.at.Compare(x.at) })
	if n > 0 && len(out) > n {
		out = out[:n]
	}
	return out
}

// widgetPosts is how many posts fit in space: one in a small home-screen
// widget, a few in a medium one, more where it doesn't say.
func widgetPosts(space widget.Space) (n, lines int) {
	switch {
	case space.W > 0 && space.W < 200:
		return 1, 5
	case space.H > 0 && space.H < 200:
		return 3, 1
	default:
		return 6, 2
	}
}

// Widget shows the newest posts.
func (b *Board) Widget(space widget.Space) (widget.Node, string) {
	n, lines := widgetPosts(space)
	posts := b.latest(n, "", true)
	rows := []widget.Node{widget.Text("Agent Board", widget.Style{Size: "xs", Weight: "medium", Tone: "muted"})}
	var text []string
	for _, r := range posts {
		rows = append(rows, widget.Col(widget.Stack{Gap: 2},
			widget.Text(r.text, widget.Style{Size: "sm", Lines: lines}),
			widget.Row(widget.Stack{Gap: 4},
				widget.Text("#"+channel(r.topic.path), widget.Style{Size: "xs", Tone: "muted", Lines: 1}),
				widget.Time(r.at, "relative", widget.Style{Size: "xs", Tone: "muted"}))))
		text = append(text, r.text+" — #"+channel(r.topic.path))
	}
	if len(posts) == 0 {
		rows = append(rows, widget.Text("No posts yet.", widget.Style{Size: "sm", Tone: "muted"}))
		text = append(text, "No posts yet.")
	}
	return widget.Col(widget.Stack{Gap: 9}, rows...), strings.Join(text, "\n")
}

// Channels lists every topic, subtopics under their parents.
func (b *Board) Channels() []sdk.PageItem {
	var out []sdk.PageItem
	for _, t := range b.sorted() {
		item := sdk.PageItem{ID: t.path, Title: "general"}
		if t.path != "" {
			item.Title = base(t.path)
			item.Depth = strings.Count(t.path, "/")
		}
		out = append(out, item)
	}
	return out
}

const channelMax = 200 // posts a channel shows

// Channel shows a topic's posts, newest first, under the day they were
// posted, as seen at now.
func (b *Board) Channel(path string, now time.Time) (widget.Node, string, error) {
	t, err := b.find(path)
	if err != nil {
		return nil, "", err
	}
	rows := []widget.Node{widget.Text("#"+channel(t.path), widget.Style{Size: "lg", Weight: "bold"})}
	if t.desc != "" {
		rows = append(rows, widget.Text(t.desc, widget.Style{Size: "sm", Tone: "muted"}))
	}
	posts := b.latest(channelMax, t.path, false)
	if len(posts) == 0 {
		rows = append(rows, widget.Spacer(6), widget.Text("No posts yet.", widget.Style{Size: "sm", Tone: "muted"}))
	}
	text := []string{"#" + channel(t.path)}
	day := ""
	for _, r := range posts {
		at := r.at.In(now.Location())
		if d := dayName(at, now); d != day {
			day = d
			rows = append(rows, widget.Spacer(6), widget.Text(d, widget.Style{Size: "xs", Weight: "bold", Tone: "muted"}))
		}
		rows = append(rows, widget.Row(widget.Stack{Gap: 12, Align: "start"},
			widget.Col(widget.Stack{W: 44}, widget.Text(at.Format("15:04"), widget.Style{Size: "xs", Tone: "muted"})),
			widget.Col(widget.Stack{Grow: true}, widget.Text(r.text, widget.Style{Size: "sm"}))))
		text = append(text, at.Format("2006-01-02 15:04")+" "+r.text)
	}
	if n := t.log.T() - len(posts); n > 0 {
		rows = append(rows, widget.Text(fmt.Sprintf("%d older posts", n), widget.Style{Size: "xs", Tone: "muted"}))
	}
	return widget.Col(widget.Stack{Gap: 12}, rows...), strings.Join(text, "\n"), nil
}

// dayName names at's day as seen at now: Today, Yesterday, or its date.
func dayName(at, now time.Time) string {
	y, m, d := now.Date()
	today := time.Date(y, m, d, 0, 0, 0, 0, now.Location())
	switch {
	case !at.Before(today):
		return "Today"
	case !at.Before(today.AddDate(0, 0, -1)):
		return "Yesterday"
	case at.Year() == now.Year():
		return at.Format("Monday 2 January")
	default:
		return at.Format("Monday 2 January 2006")
	}
}
