package spaces

import (
	"encoding/base64"
	"fmt"
	"slices"
	"strings"
	"sync"
	"time"

	"github.com/drevantonder/the-one-app/protocol"
	"github.com/drevantonder/the-one-app/sdk"
	"github.com/drevantonder/the-one-app/sdk/widget"
)

// The Spaces page in the app: every Space as a picture of its desktop that
// opens it live, and a free one, or why there is none. Its settings hold
// the logins and backups; there is nothing to set.

type tile struct {
	name, title, who, host string
	since                  time.Time
	free                   bool
	order                  int // borrowed, signing in, the user's own
}

func setupPage(c *sdk.Ctx, p *pool) {
	p.urls = map[string]viewLink{}
	c.Page("page", "Spaces", "macwindow.on.rectangle", "Draw the Spaces page: every Space's desktop, and whether one is free.", nil,
		func(call *sdk.Call, _ string, _ *time.Location) (widget.Node, string, error) {
			return p.grid(call, 220, true)
		})
	c.PageSettings("page", func(call *sdk.Call, _ *time.Location) (widget.Node, string, error) {
		return p.settings(call)
	})
	sdk.Op(c, "menubar", "Draw the Spaces in use, for the menu bar.", sdk.Read, func(call *sdk.Call, _ struct{}) (sdk.Result, error) {
		root, text, err := p.grid(call, 120, false)
		if err != nil {
			return sdk.Result{}, err
		}
		now := time.Now()
		return sdk.Result{Text: text, Data: widget.Description{V: widget.Version, App: c.ID(), Widget: "menubar", Title: "Spaces",
			At: now.UnixMilli(), Next: now.Add(10 * time.Second).UnixMilli(), Root: widget.Col(widget.Stack{Gap: 6},
				widget.Text("Spaces", widget.Style{Size: "xs", Tone: "muted"}), root)}}, nil
	})
	sdk.Op(c, "finish_sign_in", "Save the sites signed in to in every open sign-in, and close them.", sdk.Write, func(call *sdk.Call, _ struct{}) (sdk.Result, error) {
		var spaces []string
		for _, t := range p.tiles() {
			if t.order == 1 {
				spaces = append(spaces, t.name)
			}
		}
		for _, s := range spaces {
			if _, err := p.giveBackOp(call, giveBackIn{Space: s}); err != nil {
				return sdk.Result{}, err
			}
		}
		return sdk.Text(fmt.Sprintf("Saved %d sign-ins.", len(spaces))), nil
	})
	c.Menubar(func(items []sdk.MenubarItem) []sdk.MenubarItem {
		return append(items, sdk.MenubarItem{ID: "spaces", Tool: "spaces_menubar"})
	})
	// A link that opens a browser to sign in to sites in, for the app to
	// open in a window of its own.
	c.Hook("sign_in", func(call *sdk.Call, _ protocol.HookRequest) (protocol.HookResponse, error) {
		res, err := p.signIn(call, signInIn{})
		if err != nil {
			return protocol.HookResponse{Status: 500, ContentType: "text/plain", Body: err.Error()}, nil
		}
		url, _ := res.Data.(map[string]any)["url"].(string)
		return protocol.HookResponse{ContentType: "text/html", Body: fmt.Sprintf(`<meta http-equiv="refresh" content="0;url=%s">`, url)}, nil
	})
}

// finish saves the sign-ins when their windows close.
var finish = widget.Action{Op: "finish_sign_in"}

type viewLink struct {
	url   string
	until time.Time
}

// tiles are every device's Spaces, the ones in use first.
func (p *pool) tiles() []tile {
	names := p.t.names()
	p.mu.Lock()
	defer p.mu.Unlock()
	var out []tile
	for _, n := range names {
		t := tile{name: n, host: strings.Split(p.located(n), ".")[0]}
		b, lent := p.lent[n]
		switch {
		case strings.HasPrefix(n, poolPrefix) && !lent:
			t.free = true
		case lent && b.SignIn:
			t.title, t.who, t.since, t.order = "Signing in to sites", "You", b.Since, 1
		case lent:
			t.title, t.who, t.since = cmp(b.For, "Borrowed"), cmp(b.Agent, "An agent"), b.Since
		default:
			t.title, t.order = strings.ToUpper(n[:1])+n[1:], 2
		}
		out = append(out, t)
	}
	slices.SortStableFunc(out, func(a, b tile) int {
		if a.order != b.order {
			return a.order - b.order
		}
		return a.since.Compare(b.since)
	})
	return out
}

// link is a live view of the Space, made on its device and kept until
// shortly before it stops working.
func (p *pool) link(call *sdk.Call, t tile) string {
	p.mu.Lock()
	l, ok := p.urls[t.name]
	p.mu.Unlock()
	if ok && time.Until(l.until) > 10*time.Minute {
		return l.url
	}
	var url string
	if s, err := find(call.Context, t.name); err == nil {
		url, _ = viewURL(call.Context, s, 60)
	} else if res, err := call.Invoke("spaces", "view", map[string]any{"name": t.name, "minutes": 60, "device": t.host}); err == nil {
		url, _ = res.Data.(map[string]any)["url"].(string)
	}
	if url != "" {
		p.mu.Lock()
		p.urls[t.name] = viewLink{url, time.Now().Add(time.Hour)}
		p.mu.Unlock()
	}
	return url
}

// grid draws the Spaces as pictures at least min points wide; with all,
// the ones always on, a free one and what they all use too.
func (p *pool) grid(call *sdk.Call, min float64, all bool) (widget.Node, string, error) {
	tiles := p.tiles()
	rooms := p.roomsAll(call)
	uses := map[string]usage{}
	for _, r := range rooms {
		for n, u := range r.Spaces {
			uses[n] = u
		}
	}
	type shown struct {
		pic, url string
	}
	got := make([]shown, len(tiles))
	var wg sync.WaitGroup
	for i, t := range tiles {
		if t.free || !all && t.order == 2 {
			continue
		}
		wg.Go(func() {
			if img, err := p.t.screen(call, screenIn{Space: t.name, Width: 480}); err == nil {
				got[i].pic = "data:" + img.MIMEType + ";base64," + base64.StdEncoding.EncodeToString(img.Data)
			}
			got[i].url = p.link(call, t)
		})
	}
	wg.Wait()
	small := min < 200
	var cells []widget.Node
	var lines []string
	free := ""
	for i, t := range tiles {
		if t.free {
			free = t.host
			continue
		}
		if !all && t.order == 2 {
			continue
		}
		pic := widget.Img(got[i].pic, 0, 0, 16.0/10, 10)
		if got[i].pic == "" {
			pic = widget.Col(widget.Stack{Aspect: 16.0 / 10, Radius: 10, Border: "solid"})
		}
		var meta []string
		if t.who != "" {
			meta = append(meta, t.who)
		}
		if !small {
			meta = append(meta, t.host)
			if u, ok := uses[t.name]; ok {
				meta = append(meta, size(u.Mem), fmt.Sprintf("%.0f%% CPU", u.CPU))
			}
		}
		line := []widget.Node{widget.Text(strings.Join(meta, " · "), widget.Style{Size: "sm", Tone: "muted", Lines: 1})}
		if !t.since.IsZero() {
			line = append(line, widget.Time(t.since, "relative", widget.Style{Size: "sm", Tone: "muted"}))
		}
		titleSize := "md"
		if small {
			titleSize = "sm"
		}
		cell := widget.Col(widget.Stack{Gap: 2}, pic, widget.Spacer(6),
			widget.Text(t.title, widget.Style{Size: titleSize, Weight: "medium", Lines: 1}),
			widget.Row(widget.Stack{Gap: 6}, line...))
		if got[i].url != "" {
			var done *widget.Action
			if t.order == 1 {
				done = &finish
			}
			cell = widget.Popout(got[i].url, t.title, done, cell)
		}
		cells = append(cells, cell)
		lines = append(lines, fmt.Sprintf("%s: %s", t.title, t.who))
	}
	if all {
		cells = append(cells, p.freeTile(call, free))
	}
	if !all {
		if len(cells) == 0 {
			return widget.Text("No agent is using a Space.", widget.Style{Size: "sm", Tone: "muted"}), "No Spaces in use.", nil
		}
		return widget.Grid(min, 12, cells...), strings.Join(lines, "\n"), nil
	}
	head, text := summary(rooms, len(cells)-1)
	return widget.Col(widget.Stack{Gap: 28}, head, widget.Grid(min, 20, cells...)), strings.Join(append(lines, text), "\n"), nil
}

// freeTile is the Space ready for the next agent, or why there is none.
func (p *pool) freeTile(call *sdk.Call, host string) widget.Node {
	title, sub := "Free", host
	if host == "" {
		if best := p.best(call); best != nil {
			sub = "starting on " + strings.Split(best.Host, ".")[0]
		} else {
			title, sub = "No room", "every device is short of memory"
		}
	}
	return widget.Col(widget.Stack{Gap: 2},
		widget.Col(widget.Stack{Aspect: 16.0 / 10, Radius: 10, Border: "well"}), widget.Spacer(6),
		widget.Text(title, widget.Style{Size: "md", Weight: "medium", Tone: "muted"}),
		widget.Text(sub, widget.Style{Size: "sm", Tone: "muted"}))
}

// summary is what the Spaces use together, and how full each device that
// runs them is.
func summary(rooms []room, running int) (widget.Node, string) {
	var mem int64
	var cpu float64
	measured := false
	var bars []widget.Node
	for _, r := range rooms {
		if len(r.Spaces) == 0 || r.Total == 0 {
			continue
		}
		measured = true
		for _, u := range r.Spaces {
			mem += u.Mem
			cpu += u.CPU
		}
		inUse := r.Total - r.Free
		bars = append(bars, widget.Row(widget.Stack{Gap: 10},
			widget.Text(strings.Split(r.Host, ".")[0], widget.Style{Size: "sm", Tone: "muted"}),
			widget.Progress(float64(inUse)/float64(r.Total), "", 180, "blue"),
			widget.Text(fmt.Sprintf("%s of %s", size(inUse), size(r.Total)), widget.Style{Size: "sm", Tone: "muted"})))
	}
	stats := []widget.Node{widget.Stat(fmt.Sprint(running), "Running")}
	text := fmt.Sprintf("%d running", running)
	if measured {
		stats = append(stats, widget.Stat(size(mem), "Memory"), widget.Stat(fmt.Sprintf("%.0f%%", cpu), "CPU"))
		text += fmt.Sprintf("; using %s and %.0f%% CPU", size(mem), cpu)
	}
	return widget.Col(widget.Stack{Gap: 14}, append([]widget.Node{widget.Row(widget.Stack{Gap: 40}, stats...)}, bars...)...), text
}

// settings lists the sites agents are signed in to, and the backups.
func (p *pool) settings(call *sdk.Call) (widget.Node, string, error) {
	me, _ := tailnetName(call.Context)
	head := func(s string) widget.Node {
		return widget.Text(strings.ToUpper(s), widget.Style{Size: "xs", Weight: "bold", Tone: "muted"})
	}
	var rows []widget.Node
	var lines []string
	rows = append(rows, head("Logins"))
	p.mu.Lock()
	logins := make(map[string]loginSites, len(p.sites))
	for k, v := range p.sites {
		logins[k] = v
	}
	p.mu.Unlock()
	names := slices.Sorted(func(yield func(string) bool) {
		for n := range logins {
			if !yield(n) {
				return
			}
		}
	})
	for _, n := range names {
		l := logins[n]
		if len(names) > 1 {
			rows = append(rows, widget.Text(n, widget.Style{Size: "sm", Weight: "bold"}))
		}
		for _, s := range l.Sites {
			state, color := "signed in", "green"
			if !s.Until.IsZero() && s.Until.Before(time.Now()) {
				state, color = "signed out", "orange"
			}
			rows = append(rows, widget.Row(widget.Stack{Gap: 8}, widget.Dot(color),
				widget.Text(s.Site, widget.Style{Size: "sm"}), widget.Spacer(0), widget.Text(state, widget.Style{Size: "xs", Tone: "muted"})))
			lines = append(lines, fmt.Sprintf("%s: %s", s.Site, state))
		}
		if l.Others > 0 {
			rows = append(rows, widget.Text(fmt.Sprintf("and cookies from %d other sites", l.Others), widget.Style{Size: "xs", Tone: "muted"}))
		}
		rows = append(rows, widget.Text("saved "+l.At.Local().Format("2 Jan 15:04"), widget.Style{Size: "xs", Tone: "muted"}))
	}
	if len(names) == 0 {
		rows = append(rows, widget.Text("No sites yet.", widget.Style{Size: "sm", Tone: "muted"}))
	}
	open := 0
	for _, t := range p.tiles() {
		if t.order != 1 {
			continue
		}
		open++
		if url := p.link(call, t); url != "" {
			rows = append(rows, widget.Row(widget.Stack{Gap: 8},
				widget.Dot("orange"), widget.Text("A sign-in is open", widget.Style{Size: "sm"}), widget.Spacer(0),
				widget.Popout(url, "Signing in to sites", &finish, widget.Text("Open", widget.Style{Size: "sm", Weight: "medium"}))))
		}
	}
	if open > 0 {
		rows = append(rows, widget.Button("Save and close", finish, false))
	} else {
		rows = append(rows, widget.Popout(fmt.Sprintf("http://%s:4747/hooks/spaces/sign_in", me), "Signing in to sites", &finish,
			widget.Row(widget.Stack{Pad: 6, Radius: 7, Border: "solid"}, widget.Text("Sign in to a site…", widget.Style{Size: "sm", Weight: "medium"}))))
	}

	rows = append(rows, widget.Spacer(10), head("Backups"))
	files, err := call.Files("")
	if err != nil {
		rows = append(rows, widget.Text("Backups need a bucket.", widget.Style{Size: "sm", Tone: "muted"}))
		return widget.Col(widget.Stack{Gap: 6}, rows...), strings.Join(lines, "\n"), nil
	}
	latest := map[string]sdk.FileInfo{}
	count := map[string]int{}
	for _, f := range files {
		name, _, _ := strings.Cut(f.Name, "/")
		if strings.HasPrefix(name, "@") {
			continue
		}
		count[name]++
		if f.Name > latest[name].Name {
			latest[name] = f
		}
	}
	here := map[string]bool{}
	for _, n := range p.t.names() {
		here[n] = true
	}
	for _, n := range slices.Sorted(func(yield func(string) bool) {
		for n := range latest {
			if !yield(n) {
				return
			}
		}
	}) {
		f := latest[n]
		row := []widget.Node{widget.Text(n, widget.Style{Size: "sm"}), widget.Spacer(0),
			widget.Text(fmt.Sprintf("%s · %s · %d kept", f.Modified.Local().Format("2 Jan 15:04"), size(f.Size), count[n]), widget.Style{Size: "xs", Tone: "muted"})}
		if !here[n] {
			row = append(row, widget.Button("Restore", widget.Action{Op: "create", Input: map[string]any{"name": n}}, false))
		}
		rows = append(rows, widget.Row(widget.Stack{Gap: 8}, row...))
		lines = append(lines, fmt.Sprintf("backup %s: %s", n, f.Modified.Local().Format(time.DateTime)))
	}
	if len(latest) == 0 {
		rows = append(rows, widget.Text("No Space is backed up.", widget.Style{Size: "sm", Tone: "muted"}))
	}
	return widget.Col(widget.Stack{Gap: 6}, rows...), strings.Join(lines, "\n"), nil
}
