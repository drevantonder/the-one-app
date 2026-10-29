package t3code

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"slices"
	"strings"
	"sync"
	"time"

	"github.com/drevantonder/the-one-app/plugins/internal/nodes"
	"github.com/drevantonder/the-one-app/sdk"
)

type host struct {
	Name    string    `json:"name"`
	Online  bool      `json:"online"`
	Host    bool      `json:"host"`
	Version string    `json:"version,omitempty"`
	Runs    string    `json:"runs,omitempty"` // how the server runs
	Apps    []session `json:"apps,omitempty"`
	Error   string    `json:"error,omitempty"`
}

func (p *plugin) here(call *sdk.Call) host {
	me, _ := nodes.Self(call)
	h := host{Name: me, Online: true, Host: p.isHost(me)}
	if _, err := t3(); err != nil {
		return h
	}
	ctx, cancel := context.WithTimeout(call.Context, 30*time.Second)
	defer cancel()
	if out, err := runT3(ctx, "--version"); err == nil {
		h.Version = strings.TrimPrefix(strings.TrimSpace(out), "t3 ")
	}
	if app := desktopApp(); app != "" {
		h.Runs = "in the desktop app"
	} else if out, err := runT3(ctx, "service", "status"); err == nil {
		for line := range strings.Lines(out) {
			if st, ok := strings.CutPrefix(strings.TrimSpace(line), "Status: "); ok {
				h.Runs = "background service: " + st
			}
		}
	}
	out, err := runT3(ctx, "auth", "session", "list")
	if err != nil {
		h.Error = err.Error()
		return h
	}
	h.Apps = apps(sessions(out))
	return h
}

// all asks every online device for its T3 Code server at once.
func (p *plugin) all(call *sdk.Call) ([]host, error) {
	ns, err := nodes.List(call)
	if err != nil {
		return nil, err
	}
	out := make([]host, len(ns))
	var wg sync.WaitGroup
	for i, n := range ns {
		out[i] = host{Name: n.Name, Host: p.isHost(n.Name)}
		if !n.Online {
			continue
		}
		wg.Go(func() {
			if n.Self {
				out[i] = p.here(call)
				return
			}
			res, err := call.Invoke("t3code", "here", map[string]any{"device": n.Name})
			if err == nil {
				raw, _ := json.Marshal(res.Data)
				err = json.Unmarshal(raw, &out[i])
			}
			if err != nil {
				out[i].Online, out[i].Error = true, err.Error()
			}
		})
	}
	wg.Wait()
	return out, nil
}

const soon = 7 * 24 * time.Hour

func report(hs []host, now time.Time) string {
	var b strings.Builder
	for _, h := range hs {
		b.WriteString(h.Name)
		if h.Host {
			b.WriteString(" (host)")
		}
		switch {
		case !h.Online:
			b.WriteString(": offline\n")
			continue
		case h.Version == "":
			b.WriteString(": no T3 Code\n")
		default:
			fmt.Fprintf(&b, ": T3 Code %s", h.Version)
			if h.Runs != "" {
				b.WriteString(", " + h.Runs)
			}
			b.WriteString("\n")
		}
		if h.Error != "" {
			fmt.Fprintf(&b, "  error: %s\n", h.Error)
		}
		for _, a := range h.Apps {
			fmt.Fprintf(&b, "  %s", a.Client)
			if !a.LastConnected.IsZero() {
				fmt.Fprintf(&b, ", last used %s", a.LastConnected.Local().Format("Jan 2 15:04"))
			}
			if !a.Expires.IsZero() {
				left := a.Expires.Sub(now)
				switch {
				case left <= 0:
					b.WriteString(", expired: pair again")
				case left < soon:
					fmt.Fprintf(&b, ", expires in %d days: pair again soon", int(left.Hours()/24)+1)
				default:
					fmt.Fprintf(&b, ", expires %s", a.Expires.Local().Format("Jan 2"))
				}
			}
			b.WriteString("\n")
		}
	}
	return strings.TrimRight(b.String(), "\n")
}

func (p *plugin) pair(call *sdk.Call, in pairIn) (sdk.Result, error) {
	if strings.TrimSpace(in.For) == "" {
		return sdk.Result{}, errors.New("say which app to pair, e.g. iPhone")
	}
	ns, err := nodes.List(call)
	if err != nil {
		return sdk.Result{}, err
	}
	var targets []nodes.Node
	for _, n := range ns {
		if len(in.Devices) > 0 && slices.Contains(in.Devices, n.Name) || len(in.Devices) == 0 && p.isHost(n.Name) {
			targets = append(targets, n)
		}
	}
	if len(targets) == 0 {
		return sdk.Result{}, errors.New("no T3 Code hosts: choose them with hosts")
	}
	links := make([]string, len(targets))
	var wg sync.WaitGroup
	for i, n := range targets {
		wg.Go(func() {
			var url string
			var err error
			switch {
			case !n.Online:
				err = errors.New("offline")
			case n.Self:
				url, err = pairLink(call.Context, in.For)
			default:
				var res sdk.Result
				res, err = call.Invoke("t3code", "pair_here", map[string]any{"device": n.Name, "for": in.For})
				url = strings.TrimSpace(res.Text)
			}
			if err != nil {
				links[i] = fmt.Sprintf("%s: couldn't make a link: %v", n.Name, err)
				return
			}
			links[i] = n.Name + ": " + url
		})
	}
	wg.Wait()
	return sdk.Text(fmt.Sprintf("Paste each link into Add environment in T3 Code on the %s, or open it there. Each works once, for 15 minutes.\n\n%s", in.For, strings.Join(links, "\n"))), nil
}
