package claude

import (
	"cmp"
	"context"
	"encoding/json"
	"fmt"
	"slices"
	"strings"
	"sync"
	"time"

	"github.com/drevantonder/the-one-app/plugins/internal/nodes"
	"github.com/drevantonder/the-one-app/sdk"
)

type device struct {
	Name      string    `json:"name"`
	Online    bool      `json:"online"`
	Coding    bool      `json:"coding"`
	Claude    string    `json:"claude,omitempty"`
	Swap      string    `json:"claudeSwap,omitempty"`
	Accounts  []account `json:"accounts,omitempty"`
	SigningIn bool      `json:"signingIn,omitempty"`
	Error     string    `json:"error,omitempty"`
}

type usage struct {
	Pct float64 `json:"pct"`
}

type account struct {
	Email  string `json:"email"`
	Org    string `json:"organizationName,omitempty"`
	Alias  string `json:"alias,omitempty"`
	Active bool   `json:"active"`
	Usage  struct {
		FiveHour *usage `json:"fiveHour,omitempty"`
		SevenDay *usage `json:"sevenDay,omitempty"`
	} `json:"usage"`
}

func (p *plugin) here(call *sdk.Call) device {
	me := p.me(call)
	d := device{Name: me, Online: true, Coding: p.isCoding(me)}
	ctx, cancel := context.WithTimeout(call.Context, 30*time.Second)
	defer cancel()
	d.Claude, d.Swap = versions(ctx)
	if d.Swap != "" {
		var err error
		if d.Accounts, err = accounts(ctx); err != nil {
			d.Error = err.Error()
		}
	}
	p.mu.Lock()
	d.SigningIn = p.login != nil
	p.mu.Unlock()
	return d
}

// all asks every online device for its Claude Code setup at once.
func (p *plugin) all(call *sdk.Call) ([]device, error) {
	ds, err := nodes.List(call)
	if err != nil {
		return nil, err
	}
	out := make([]device, len(ds))
	var wg sync.WaitGroup
	for i, n := range ds {
		out[i] = device{Name: n.Name, Coding: p.isCoding(n.Name)}
		if !n.Online {
			continue
		}
		wg.Go(func() {
			if n.Self {
				out[i] = p.here(call)
				return
			}
			res, err := call.Invoke("claude", "here", map[string]any{"device": n.Name})
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

// missing is the accounts some device is signed in to that a coding device
// isn't, by device.
func missing(devices []device) map[string][]string {
	var all []string
	for _, d := range devices {
		for _, a := range d.Accounts {
			if !slices.Contains(all, a.Email) {
				all = append(all, a.Email)
			}
		}
	}
	slices.Sort(all)
	out := map[string][]string{}
	for _, d := range devices {
		if !d.Coding || !d.Online || d.Error != "" {
			continue
		}
		for _, e := range all {
			if !slices.ContainsFunc(d.Accounts, func(a account) bool { return a.Email == e }) {
				out[d.Name] = append(out[d.Name], e)
			}
		}
	}
	return out
}

func report(devices []device) string {
	gaps := missing(devices)
	var b strings.Builder
	for _, d := range devices {
		b.WriteString(d.Name)
		if d.Coding {
			b.WriteString(" (coding)")
		}
		switch {
		case !d.Online:
			b.WriteString(": offline\n")
			continue
		case d.Claude == "" && d.Swap == "":
			b.WriteString(": no Claude Code or claude-swap")
		default:
			fmt.Fprintf(&b, ": Claude Code %s, claude-swap %s", cmp.Or(d.Claude, "not installed"), cmp.Or(d.Swap, "not installed"))
		}
		if d.SigningIn {
			b.WriteString(", waiting for a sign-in code")
		}
		b.WriteString("\n")
		if d.Error != "" {
			fmt.Fprintf(&b, "  error: %s\n", d.Error)
		}
		for _, a := range d.Accounts {
			mark := " "
			if a.Active {
				mark = "*"
			}
			fmt.Fprintf(&b, "  %s %s", mark, a.Email)
			if a.Org != "" {
				fmt.Fprintf(&b, " [%s]", a.Org)
			}
			if u := a.Usage.FiveHour; u != nil {
				fmt.Fprintf(&b, " 5h %.0f%%", u.Pct)
			}
			if u := a.Usage.SevenDay; u != nil {
				fmt.Fprintf(&b, " 7d %.0f%%", u.Pct)
			}
			b.WriteString("\n")
		}
		for _, e := range gaps[d.Name] {
			fmt.Fprintf(&b, "    %s not signed in here\n", e)
		}
	}
	return strings.TrimRight(b.String(), "\n")
}
