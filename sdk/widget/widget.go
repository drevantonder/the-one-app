// Package widget builds widgets: small views of a plugin's data that every
// renderer draws with its own parts (SwiftUI in the apps, the menu bar and
// home screens; HTML in MCP Apps hosts). The format is the PTC App's widget
// description v1, with gauge, spark, icon and dot added, and w, dim and color
// on a few nodes. Renderers skip node types and fields they don't know.
//
// Sizes are points, times are epoch milliseconds. Colors are names: green,
// orange, red, blue, purple, gray, or muted. Links are https URLs.
package widget

import "time"

// Version is the description version renderers read.
const Version = 1

// A Node is one part of a widget, as JSON.
type Node = map[string]any

// Description is a rendered widget: what a widget operation returns as its
// result data.
type Description struct {
	V      int     `json:"v"`
	App    string  `json:"app"`
	Widget string  `json:"widget"`
	Title  string  `json:"title"`
	At     int64   `json:"at"`
	Next   int64   `json:"next"`
	W      float64 `json:"w,omitempty"`
	H      float64 `json:"h,omitempty"`
	Link   string  `json:"link,omitempty"`
	Root   Node    `json:"root"`
}

// Space is the room a renderer has for the widget, in points; zero when it
// doesn't say.
type Space struct {
	W float64 `json:"w,omitempty" jsonschema:"width in points the widget is drawn at"`
	H float64 `json:"h,omitempty" jsonschema:"height in points the widget is drawn at"`
}

// Stack lays out a row or column. Grow takes the free space along the
// parent's axis; W fixes the width; Dim draws it faded, as for something
// offline.
//
// Aspect fixes its width over height; Border (solid or dashed) outlines it,
// with corners of Radius.
type Stack struct {
	Gap, Pad, W    float64
	Align, Justify string // align: start center end stretch; justify: start center end between
	Grow, Dim      bool
	Aspect, Radius float64
	Border         string
}

// Style is how text looks. Size is xl (28), lg (20), md (15), sm (13) or
// xs (10);
// Weight regular, medium or bold; Tone default or muted.
type Style struct {
	Size, Weight, Tone string
	Lines              int
	Align              string // start, center, end
}

// Item is a list row.
type Item struct {
	Title, Sub string
	End        Node // shown at the trailing edge: Text or Time
	Link       string
}

// Point is one value on a chart.
type Point struct {
	Value float64 `json:"value"`
	Label string  `json:"label,omitempty"`
}

// Action runs one of the widget's plugin's operations, then the widget is
// drawn again.
type Action struct {
	Op    string         `json:"op"`
	Input map[string]any `json:"input,omitempty"`
}

func put(n Node, key string, v any) Node {
	switch x := v.(type) {
	case string:
		if x == "" {
			return n
		}
	case float64:
		if x == 0 {
			return n
		}
	case int:
		if x == 0 {
			return n
		}
	case bool:
		if !x {
			return n
		}
	case nil:
		return n
	}
	n[key] = v
	return n
}

func stack(t string, s Stack, children []Node) Node {
	n := Node{"t": t, "c": nonNil(children)}
	put(n, "gap", s.Gap)
	put(n, "pad", s.Pad)
	put(n, "w", s.W)
	put(n, "align", s.Align)
	put(n, "justify", s.Justify)
	put(n, "grow", s.Grow)
	put(n, "aspect", s.Aspect)
	put(n, "radius", s.Radius)
	put(n, "border", s.Border)
	return put(n, "dim", s.Dim)
}

func nonNil(c []Node) []Node {
	if c == nil {
		return []Node{}
	}
	return c
}

func Row(s Stack, children ...Node) Node { return stack("row", s, children) }
func Col(s Stack, children ...Node) Node { return stack("col", s, children) }

// Grid lays its children out in as many columns as fit, each at least min
// points wide.
func Grid(min, gap float64, children ...Node) Node {
	return put(Node{"t": "grid", "min": min, "c": nonNil(children)}, "gap", gap)
}

// Popout opens url, a live web page, in a window of its own titled title
// when its children are tapped: floating over other windows where the
// renderer can.
func Popout(url, title string, children ...Node) Node {
	return Node{"t": "popout", "url": url, "title": title, "c": nonNil(children)}
}

// Img shows a picture from src, an https or data URL. With aspect (width
// over height) it fills the width it is given; otherwise it is w by h.
func Img(src string, w, h, aspect, radius float64) Node {
	n := Node{"t": "img", "src": src}
	put(n, "w", w)
	put(n, "h", h)
	put(n, "aspect", aspect)
	return put(n, "radius", radius)
}

// Link opens to when its children are tapped.
func Link(to string, children ...Node) Node {
	return Node{"t": "link", "to": to, "c": nonNil(children)}
}

func styled(n Node, s Style) Node {
	put(n, "size", s.Size)
	put(n, "weight", s.Weight)
	put(n, "tone", s.Tone)
	put(n, "lines", s.Lines)
	return put(n, "align", s.Align)
}

func Text(text string, s Style) Node { return styled(Node{"t": "text", "text": text}, s) }

// Time shows at as relative ("5 min ago"), timer, date or clock.
func Time(at time.Time, style string, s Style) Node {
	return styled(Node{"t": "time", "at": at.UnixMilli(), "style": style}, s)
}

func Stat(value, label string) Node { return put(Node{"t": "stat", "value": value}, "label", label) }

// Spacer takes size points, or the free space when size is 0.
func Spacer(size float64) Node { return put(Node{"t": "spacer"}, "size", size) }

// List shows as many items as fit, then how many more there are.
func List(grow bool, items ...Item) Node {
	c := make([]Node, len(items))
	for i, it := range items {
		c[i] = Node{"t": "item", "title": it.Title}
		put(c[i], "sub", it.Sub)
		put(c[i], "link", it.Link)
		if it.End != nil {
			c[i]["end"] = it.End
		}
	}
	return put(Node{"t": "list", "c": c}, "grow", grow)
}

// Chart is a bar or line chart; hl is the index of the one point drawn in
// full contrast, or -1.
func Chart(kind string, data []Point, h float64, hl int) Node {
	n := Node{"t": "chart", "kind": kind, "data": data}
	if hl >= 0 {
		n["hl"] = hl
	}
	return put(n, "h", h)
}

// Progress is a bar filled to value (0 to 1). W fixes its width; color names
// its fill.
func Progress(value float64, label string, w float64, color string) Node {
	n := Node{"t": "progress", "value": value}
	put(n, "label", label)
	put(n, "w", w)
	return put(n, "color", color)
}

// Gauge is a ring filled to value (0 to 1), size points across.
func Gauge(value float64, color string, size float64) Node {
	return put(Node{"t": "gauge", "value": value, "color": color}, "size", size)
}

// Spark is a small line with the area under it shaded, of values from 0 to
// max, w by h points.
func Spark(values []float64, max float64, color string, w, h float64) Node {
	if values == nil {
		values = []float64{}
	}
	return Node{"t": "spark", "data": values, "max": max, "color": color, "w": w, "h": h}
}

// Icon draws an SVG path (a 24-point viewBox), outlined with stroke or
// filled, size points across. A badge, also a path, is filled at its bottom
// trailing corner, with the icon cut away around it.
func Icon(path string, stroke bool, size float64, badge string) Node {
	n := Node{"t": "icon", "path": path, "size": size}
	put(n, "stroke", stroke)
	return put(n, "badge", badge)
}

// Dot is a small status dot.
func Dot(color string) Node { return Node{"t": "dot", "color": color} }

func Button(label string, a Action, primary bool) Node {
	return put(Node{"t": "button", "label": label, "action": a}, "primary", primary)
}

func Toggle(label string, on bool, a Action) Node {
	return Node{"t": "toggle", "label": label, "on": on, "action": a}
}
