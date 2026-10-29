// Package agentsmd is the built-in plugin keeping the user's global
// AGENTS.md the same on every device: it writes the document to each agent's
// user-level AGENTS.md, and records hand edits to those files as revisions.
package agentsmd

import (
	"fmt"
	"strings"
	"sync"
	"time"

	"github.com/drevantonder/the-one-app/sdk"
)

type revision struct {
	Entry string
	Time  string
	Text  string
}

type setIn struct {
	Text string `json:"text" jsonschema:"the complete new AGENTS.md; replaces the current one"`
}

type restoreIn struct {
	Revision int `json:"revision" jsonschema:"revision number, as listed by history"`
}

type noIn struct{}

var Plugin = sdk.Plugin{ID: "agentsmd", Name: "AGENTS.md", Version: "0.1.0", Setup: setup}

func setup(c *sdk.Ctx) error {
	var (
		mu     sync.Mutex
		revs   []revision
		report []string
		ready  bool
	)
	kick := make(chan struct{}, 1)
	poke := func() {
		select {
		case kick <- struct{}{}:
		default:
		}
	}
	current := func() string {
		mu.Lock()
		defer mu.Unlock()
		if len(revs) == 0 {
			return ""
		}
		return revs[len(revs)-1].Text
	}
	count := func() int { mu.Lock(); defer mu.Unlock(); return len(revs) }

	c.Entry("set", sdk.Available)
	c.OnReset(func() { mu.Lock(); revs = nil; mu.Unlock() })
	sdk.Handle(c, "set", func(e sdk.Entry, in setIn) error {
		mu.Lock()
		revs = append(revs, revision{Entry: e.ID, Time: e.Time, Text: in.Text})
		mu.Unlock()
		poke()
		return nil
	})

	save := func(call *sdk.Call, text string) error {
		_, err := call.Append("set", setIn{Text: text})
		return err
	}

	sdk.Op(c, "get", "Show the user's global AGENTS.md, the agent instructions kept the same on every device, and which files it is written to on this device.", sdk.Read, func(*sdk.Call, noIn) (sdk.Result, error) {
		text := current()
		if text == "" {
			text = "No global AGENTS.md yet. Set it with agentsmd_set."
		}
		mu.Lock()
		lines := append([]string(nil), report...)
		mu.Unlock()
		if len(lines) > 0 {
			text += "\n\n---\nOn this device:\n" + strings.Join(lines, "\n")
		}
		return sdk.Text(text), nil
	})
	sdk.Op(c, "set", "Replace the user's global AGENTS.md with new text. Only do this when the user asks; earlier versions stay in history.", sdk.Write, func(call *sdk.Call, in setIn) (sdk.Result, error) {
		text := strings.TrimSpace(in.Text)
		if text == "" {
			return sdk.Result{}, fmt.Errorf("AGENTS.md cannot be empty")
		}
		if text == current() {
			return sdk.Text("Unchanged."), nil
		}
		if err := save(call, text); err != nil {
			return sdk.Result{}, err
		}
		return sdk.Text(fmt.Sprintf("Saved as revision %d.", count())), nil
	})
	sdk.Op(c, "history", "List every revision of the global AGENTS.md, newest first.", sdk.Read, func(*sdk.Call, noIn) (sdk.Result, error) {
		mu.Lock()
		defer mu.Unlock()
		if len(revs) == 0 {
			return sdk.Text("No revisions yet."), nil
		}
		var b strings.Builder
		for i := len(revs) - 1; i >= 0; i-- {
			first := strings.SplitN(revs[i].Text, "\n", 2)[0]
			if len(first) > 80 {
				first = first[:80] + "…"
			}
			fmt.Fprintf(&b, "%d  %s  %s\n", i+1, revs[i].Time, first)
		}
		return sdk.Text(strings.TrimRight(b.String(), "\n")), nil
	})
	sdk.Op(c, "restore", "Make an earlier revision of the global AGENTS.md current again (as a new revision).", sdk.Write, func(call *sdk.Call, in restoreIn) (sdk.Result, error) {
		mu.Lock()
		n := len(revs)
		var text string
		if in.Revision >= 1 && in.Revision <= n {
			text = revs[in.Revision-1].Text
		}
		mu.Unlock()
		if text == "" {
			return sdk.Result{}, fmt.Errorf("no revision %d; there are %d", in.Revision, n)
		}
		if text == current() {
			return sdk.Text("That revision is already current."), nil
		}
		if err := save(call, text); err != nil {
			return sdk.Result{}, err
		}
		return sdk.Text(fmt.Sprintf("Restored revision %d as revision %d.", in.Revision, count())), nil
	})

	// Files are written only from here, once the log has been replayed: on
	// ready, after each new entry, and every minute to catch hand edits.
	c.OnReady(func(call *sdk.Call) {
		mu.Lock()
		started := ready
		ready = true
		mu.Unlock()
		if started {
			return
		}
		dir, err := c.Dir()
		if err != nil {
			c.Logf("agentsmd: %v", err)
			return
		}
		f := &files{state: dir}
		call = call.Detached()
		go func() {
			tick := time.NewTicker(time.Minute)
			defer tick.Stop()
			warned := map[string]bool{}
			for {
				doc := current()
				edit, lines, err := f.sync(doc)
				if err != nil {
					c.Logf("agentsmd: %v", err)
				}
				for _, l := range lines {
					if strings.Contains(l, "symlink") && !warned[l] {
						warned[l] = true
						c.Logf("agentsmd: %s", l)
					}
				}
				mu.Lock()
				report = lines
				mu.Unlock()
				if edit != "" && edit != doc {
					err := save(call, edit)
					if err == nil {
						continue
					}
					c.Logf("agentsmd: recording a hand edit: %v", err)
				}
				select {
				case <-tick.C:
				case <-kick:
				}
			}
		}()
	})
	return nil
}
